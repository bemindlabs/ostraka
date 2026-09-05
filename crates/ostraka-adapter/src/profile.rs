//! Adapter profiles: a vendor CLI described as data.

use crate::capability::Capabilities;
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
}

/// A vendor CLI, described entirely in configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub event_format: EventFormat,
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
        if !self.args.iter().any(|a| a.contains("{{prompt}}")) {
            return Err(Error::Profile {
                id: self.id.clone(),
                message: "args must place the task somewhere: no {{prompt}} placeholder found"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Fills the profile's argument template for one run.
    ///
    /// Substitution is literal and non-recursive: a value that itself looks like
    /// a placeholder is inserted as text, never expanded again.
    pub fn render_args(&self, prompt: &str, model: Option<&str>, worktree: &str) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| render_one(arg, prompt, model.unwrap_or(""), worktree))
            .collect()
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
        let rendered = p.render_args("say {{model}} please", Some("m1"), "/wt");
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
        assert_eq!(p.render_args("go", None, "/wt"), ["go", "{{nonsense}}"]);
    }

    #[test]
    fn an_absent_model_renders_empty_not_the_placeholder() {
        let text = r#"
            id = "m"
            command = "c"
            args = ["{{prompt}}", "--model={{model}}"]
        "#;
        let p = Profile::parse(text).expect("parses");
        assert_eq!(p.render_args("go", None, "/wt"), ["go", "--model="]);
    }
}
