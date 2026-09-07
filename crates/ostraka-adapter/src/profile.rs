//! Adapter profiles: a vendor CLI described as data.

use crate::capability::Capabilities;
use crate::isolation::Isolation;
use crate::usage::UsageSpec;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How the adapter's output should be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventFormat {
    /// Plain output; the whole run yields one message event.
    #[default]
    None,
    /// One JSON object per line.
    Jsonl,
    /// The whole of stdout is a single JSON document, and the reply is at
    /// `event_text` within it. Used by CLIs whose structured mode reports both
    /// the answer and what it cost in one object.
    Json,
}

/// Which side of a run a profile is being invoked for.
///
/// The distinction is a safety property, not a convenience: an author has to be
/// able to edit the worktree, and a reviewer must not be. A reviewer holding
/// write permission can make a rejected change pass by editing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Author,
    Review,
}

fn default_probe_args() -> Vec<String> {
    vec!["--version".to_string()]
}

/// A vendor CLI, described entirely in configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub command: String,
    /// Invocation that writes a change. Must contain `{{prompt}}`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Invocation used when this profile reviews someone else's change.
    ///
    /// Absent means the author invocation is reused. Any profile whose author
    /// args grant tool or write permission should set this to a read-only form:
    /// the reviewer is handed the diff in its prompt and needs nothing else.
    #[serde(default)]
    pub review_args: Option<Vec<String>>,
    /// Appended only when a run names a model, so that a profile can express an
    /// optional flag without emitting an empty argument when no model is given.
    #[serde(default)]
    pub model_args: Vec<String>,
    /// How to ask whether this CLI is usable here. `--version` by default.
    #[serde(default = "default_probe_args")]
    pub probe_args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Variables this vendor needs from the launching environment.
    ///
    /// A vendor process is started with a built environment, not an inherited
    /// one — see [`crate::environment`]. Everything outside a generic
    /// operating-system baseline is dropped, so a CLI that authenticates by
    /// environment variable names that variable here. It belongs in the profile
    /// rather than in the runtime for the same reason the command does: the
    /// runtime cannot know which variable belongs to which vendor.
    #[serde(default)]
    pub inherit_env: Vec<String>,
    /// How this vendor is kept from reading the operator's own setup.
    ///
    /// Only for the part that no flag can reach: a CLI whose entire per-user
    /// directory is named by an environment variable. Everything a flag can
    /// suppress belongs in `args` and `review_args`, where it is visible in the
    /// command line the run record shows.
    #[serde(default)]
    pub isolation: Option<Isolation>,
    /// Where this vendor reports what a run cost, if it reports it at all.
    ///
    /// Configuration rather than code because the three CLIs shipped today use
    /// three different shapes on two different streams, and one of them rounds.
    #[serde(default)]
    pub usage: Option<UsageSpec>,
    #[serde(default)]
    pub event_format: EventFormat,
    /// RFC 6901 pointer to the reply, when `event_format` is `json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_text: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self> {
        let profile: Profile = toml::from_str(text).map_err(|e| Error::Profile {
            id: "<unparsed>".to_string(),
            message: e.to_string(),
        })?;
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "id must not be empty".to_string(),
            });
        }
        if self.command.trim().is_empty() {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "command must not be empty".to_string(),
            });
        }
        let carries_prompt = |args: &[String]| args.iter().any(|a| a.contains("{{prompt}}"));
        if !carries_prompt(&self.args) {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "args must place the task somewhere: no {{prompt}} placeholder found"
                    .to_string(),
            });
        }
        // A review invocation that drops the prompt drops the diff, and a
        // reviewer with no diff rejects everything for the wrong reason.
        if self
            .review_args
            .as_deref()
            .is_some_and(|r| !carries_prompt(r))
        {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "review_args must place the task somewhere: no {{prompt}} placeholder \
                          found"
                    .to_string(),
            });
        }
        if let Some(isolation) = &self.isolation {
            isolation.validate(&self.id)?;
            // Both would set the same variable and one would silently win. A
            // profile that contradicts itself should say so at parse time.
            if self.env.contains_key(&isolation.home_env) {
                return Err(Error::Profile {
                    id: self.id.clone(),
                    message: format!(
                        "env sets {:?}, which isolation.home_env also sets; remove one",
                        isolation.home_env
                    ),
                });
            }
        }
        if self.event_format == EventFormat::Json && self.event_text.is_none() {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "event_format is \"json\", so event_text must point at the reply inside \
                          the document; without it the run has no output at all"
                    .to_string(),
            });
        }
        if self.probe_args.is_empty() {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "probe_args must not be empty; the runtime has no way to ask whether \
                          this CLI is usable"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// The argument template for one side of a run.
    fn template(&self, role: Role) -> &[String] {
        match role {
            Role::Author => &self.args,
            Role::Review => self.review_args.as_deref().unwrap_or(&self.args),
        }
    }

    /// Fills the profile's argument template for one run.
    ///
    /// Substitution is literal and non-recursive: a value that itself looks like
    /// a placeholder is inserted as text, never expanded again.
    pub fn render_args(
        &self,
        role: Role,
        prompt: &str,
        model: Option<&str>,
        worktree: &str,
    ) -> Vec<String> {
        let render = |arg: &String| render_one(arg, prompt, model.unwrap_or(""), worktree);
        let mut out: Vec<String> = self.template(role).iter().map(render).collect();
        // Omitted rather than rendered empty: `--model ""` is not the same
        // request as leaving the vendor on its default model.
        if model.is_some() {
            out.extend(self.model_args.iter().map(render));
        }
        out
    }
}

/// Substitutes placeholders in a single pass.
///
/// Chained `replace` calls would re-scan text that was just substituted in, so a
/// prompt containing `{{model}}` would have it expanded — letting task text
/// reach into the argument list. One pass, left to right, cannot do that.
fn render_one(template: &str, prompt: &str, model: &str, worktree: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(end) = after.find("}}") else {
            // No closing delimiter: the rest is literal text.
            break;
        };
        let name = &after[2..end];
        match name {
            "prompt" => out.push_str(prompt),
            "model" => out.push_str(model),
            "worktree" => out.push_str(worktree),
            // An unknown placeholder is left exactly as written, so a typo is
            // visible in the command line rather than silently blanked.
            _ => out.push_str(&after[..end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        id = "example"
        command = "example-cli"
        args = ["--print", "{{prompt}}"]
        event_format = "jsonl"

        [capabilities]
        headless = true
    "#;

    #[test]
    fn parses_a_profile() {
        let p = Profile::parse(SAMPLE).expect("parses");
        assert_eq!(p.id, "example");
        assert_eq!(p.event_format, EventFormat::Jsonl);
        assert!(p.capabilities.headless);
        assert_eq!(p.probe_args, ["--version"]);
    }

    #[test]
    fn a_profile_that_never_passes_the_prompt_is_refused() {
        let text = r#"
            id = "broken"
            command = "example-cli"
            args = ["--help"]
        "#;
        assert!(Profile::parse(text).is_err());
    }

    #[test]
    fn substitution_is_literal_and_not_recursive() {
        let p = Profile::parse(SAMPLE).expect("parses");
        let rendered = p.render_args(Role::Author, "say {{model}} please", Some("m1"), "/wt");
        assert_eq!(rendered, ["--print", "say {{model}} please"]);
    }

    #[test]
    fn an_unknown_placeholder_is_left_visible() {
        let text = r#"
            id = "u"
            command = "c"
            args = ["{{prompt}}", "{{nonsense}}"]
        "#;
        let p = Profile::parse(text).expect("parses");
        assert_eq!(
            p.render_args(Role::Author, "go", None, "/wt"),
            ["go", "{{nonsense}}"]
        );
    }

    #[test]
    fn a_model_flag_is_omitted_entirely_when_no_model_is_named() {
        let text = r#"
            id = "m"
            command = "c"
            args = ["{{prompt}}"]
            model_args = ["--model", "{{model}}"]
        "#;
        let p = Profile::parse(text).expect("parses");
        assert_eq!(p.render_args(Role::Author, "go", None, "/wt"), ["go"]);
        assert_eq!(
            p.render_args(Role::Author, "go", Some("m1"), "/wt"),
            ["go", "--model", "m1"]
        );
    }

    #[test]
    fn a_reviewer_gets_the_read_only_invocation_when_one_is_declared() {
        let text = r#"
            id = "r"
            command = "c"
            args = ["--write", "{{prompt}}"]
            review_args = ["--read-only", "{{prompt}}"]
        "#;
        let p = Profile::parse(text).expect("parses");
        assert_eq!(
            p.render_args(Role::Author, "go", None, "/wt"),
            ["--write", "go"]
        );
        assert_eq!(
            p.render_args(Role::Review, "go", None, "/wt"),
            ["--read-only", "go"]
        );
    }

    #[test]
    fn a_profile_with_no_review_args_reuses_the_author_invocation() {
        let p = Profile::parse(SAMPLE).expect("parses");
        assert_eq!(
            p.render_args(Role::Review, "go", None, "/wt"),
            p.render_args(Role::Author, "go", None, "/wt")
        );
    }

    #[test]
    fn review_args_that_drop_the_prompt_are_refused() {
        let text = r#"
            id = "r"
            command = "c"
            args = ["{{prompt}}"]
            review_args = ["--help"]
        "#;
        let err = Profile::parse(text).expect_err("must refuse");
        assert!(err.to_string().contains("review_args"));
    }

    #[test]
    fn a_profile_that_both_isolates_and_unsets_its_own_isolation_is_refused() {
        // Both would set the same variable and one would silently win.
        let text = r#"
            id = "i"
            command = "c"
            args = ["{{prompt}}"]

            [env]
            VENDOR_HOME = "/home/someone/.vendor"

            [isolation]
            home_env = "VENDOR_HOME"
            home_source = ".vendor"
        "#;
        let err = Profile::parse(text).expect_err("must refuse");
        assert!(err.to_string().contains("VENDOR_HOME"), "{err}");
    }

    #[test]
    fn an_unaskable_profile_is_refused() {
        let text = r#"
            id = "p"
            command = "c"
            args = ["{{prompt}}"]
            probe_args = []
        "#;
        let err = Profile::parse(text).expect_err("must refuse");
        assert!(err.to_string().contains("probe_args"));
    }
}
