//! Keeping the operator's machine out of the run.
//!
//! A fleet runner whose results depend on whose laptop it is has not isolated
//! anything. Most of what a vendor CLI reads from its operator — instruction
//! files, MCP servers, hooks, plugins — is suppressed by flags, and flags are
//! ordinary profile `args` that need no code at all.
//!
//! One shape needs code. Some CLIs keep everything per-user in a single
//! directory named by an environment variable, with no flag to ignore it, so
//! relocating that directory is the only lever there is. Relocating it also
//! relocates the credentials that live inside it, which is why a profile may
//! name the entries that are authentication rather than configuration: those
//! are linked into the relocated directory so the CLI can still log in.
//!
//! Linked, never copied. Isolation must not leave a second copy of a secret on
//! disk in order to achieve itself.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// How a profile's vendor is prevented from reading the operator's setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Isolation {
    /// Environment variable that tells the CLI where its per-user directory is.
    pub home_env: String,
    /// Where that directory lives when the variable is unset, relative to the
    /// operator's home directory.
    pub home_source: String,
    /// Entries inside it that are credentials rather than configuration.
    ///
    /// Everything not named here stays behind, which is the point. A name that
    /// is missing on this machine is not an error: a CLI that authenticates by
    /// environment variable has no such file, and one that needed it will say
    /// so on stderr, which a failed run already keeps.
    #[serde(default)]
    pub credentials: Vec<String>,
}

impl Isolation {
    pub(crate) fn validate(&self, id: &str) -> Result<()> {
        let refuse = |message: String| -> Result<()> {
            Err(Error::Profile {
                id: id.to_string(),
                message,
            })
        };

        if self.home_env.trim().is_empty()
            || self
                .home_env
                .contains(|c: char| c == '=' || c.is_whitespace())
        {
            return refuse(format!(
                "isolation.home_env must be an environment variable name, not {:?}",
                self.home_env
            ));
        }
        if !is_contained_relative(Path::new(&self.home_source)) {
            return refuse(format!(
                "isolation.home_source must be a relative path inside the operator's home, not \
                 {:?}",
                self.home_source
            ));
        }
        // A credential entry names something to link out of the operator's home
        // directory. Anything but a single plain name is a way to ask for a file
        // somewhere else entirely.
        for name in &self.credentials {
            let mut parts = Path::new(name).components();
            let single =
                matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none();
            if !single {
                return refuse(format!(
                    "isolation.credentials entries must be plain file names, not {name:?}"
                ));
            }
        }
        Ok(())
    }

    /// Creates the relocated directory and returns the environment that points
    /// the CLI at it.
    ///
    /// The directory is per profile rather than per run. The goal is
    /// independence from the operator's configuration, not a fresh sandbox: a
    /// vendor that refreshes its own token needs somewhere to keep it, and a
    /// per-run directory would make every run re-authenticate.
    pub fn provision(&self, root: &Path, profile_id: &str) -> Result<BTreeMap<String, String>> {
        self.provision_from(root, profile_id, operator_home().as_deref())
    }

    /// The body of [`Isolation::provision`], with the operator's home passed in
    /// rather than read from the environment, so it is testable without a test
    /// mutating the environment every other test is reading.
    fn provision_from(
        &self,
        root: &Path,
        profile_id: &str,
        operator_home: Option<&Path>,
    ) -> Result<BTreeMap<String, String>> {
        let home = root.join(profile_id);
        std::fs::create_dir_all(&home).map_err(|e| Error::Isolation {
            id: profile_id.to_string(),
            message: format!("could not create {}: {e}", home.display()),
        })?;

        if !self.credentials.is_empty() {
            let Some(operator_home) = operator_home else {
                return Err(Error::Isolation {
                    id: profile_id.to_string(),
                    message: "neither HOME nor USERPROFILE is set, so the credentials to carry \
                              into the isolated directory cannot be found"
                        .to_string(),
                });
            };
            let source = operator_home.join(&self.home_source);
            for name in &self.credentials {
                carry(&source.join(name), &home.join(name), profile_id)?;
            }
        }

        Ok(BTreeMap::from([(
            self.home_env.clone(),
            home.to_string_lossy().into_owned(),
        )]))
    }
}

/// True for a relative path that cannot escape the directory it is joined to.
fn is_contained_relative(path: &Path) -> bool {
    path.components().count() > 0
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn operator_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn carry(source: &Path, link: &Path, profile_id: &str) -> Result<()> {
    // Already provisioned. Re-pointing it would overwrite whatever an operator
    // put here deliberately, and the link is to a path rather than to a
    // snapshot, so it does not go stale.
    if std::fs::symlink_metadata(link).is_ok() {
        return Ok(());
    }
    if !source.exists() {
        return Ok(());
    }
    symlink_file(source, link).map_err(|e| Error::Isolation {
        id: profile_id.to_string(),
        message: format!(
            "could not link {} to {}: {e}",
            link.display(),
            source.display()
        ),
    })
}

#[cfg(unix)]
fn symlink_file(source: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, link)
}

#[cfg(windows)]
fn symlink_file(source: &Path, link: &Path) -> std::io::Result<()> {
    // Needs Developer Mode or the create-symlink privilege. Copying instead
    // would put a second copy of a credential on disk, which is not a trade
    // this makes silently: the error names the file that could not be linked.
    std::os::windows::fs::symlink_file(source, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_source: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no symlink support on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn isolation(credentials: &[&str]) -> Isolation {
        Isolation {
            home_env: "VENDOR_HOME".to_string(),
            home_source: ".vendor".to_string(),
            credentials: credentials.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn provisioning_names_the_relocated_directory() {
        let root = tempdir();
        let env = isolation(&[])
            .provision_from(&root, "vendor", None)
            .expect("provisions");
        assert_eq!(
            env.get("VENDOR_HOME").map(String::as_str),
            Some(root.join("vendor").to_string_lossy().as_ref())
        );
        assert!(root.join("vendor").is_dir());
    }

    #[test]
    fn a_credential_is_linked_rather_than_copied() {
        // The point of relocating a home directory is that the operator's
        // configuration stays behind. The credential has to follow, and it has
        // to follow without becoming a second copy of the secret on disk.
        let home = tempdir();
        let vendor = home.join(".vendor");
        std::fs::create_dir_all(&vendor).expect("vendor dir");
        std::fs::write(vendor.join("auth.json"), "token").expect("credential");

        let root = home.join("runs");
        isolation(&["auth.json"])
            .provision_from(&root, "vendor", Some(&home))
            .expect("provisions");

        let link = root.join("vendor").join("auth.json");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link exists")
                .file_type()
                .is_symlink(),
            "credential was materialized rather than linked"
        );
        assert_eq!(std::fs::read_to_string(&link).expect("reads"), "token");
    }

    #[test]
    fn nothing_but_the_named_credentials_follows_the_run() {
        let home = tempdir();
        let vendor = home.join(".vendor");
        std::fs::create_dir_all(&vendor).expect("vendor dir");
        std::fs::write(vendor.join("auth.json"), "token").expect("credential");
        std::fs::write(vendor.join("AGENTS.md"), "operator instructions").expect("instructions");
        std::fs::write(vendor.join("config.toml"), "model = 'whatever'").expect("config");

        let root = home.join("runs");
        isolation(&["auth.json"])
            .provision_from(&root, "vendor", Some(&home))
            .expect("provisions");

        let carried: Vec<String> = std::fs::read_dir(root.join("vendor"))
            .expect("reads")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(carried, ["auth.json"]);
    }

    #[test]
    fn a_credential_that_does_not_exist_here_is_not_an_error() {
        // A CLI that authenticates by environment variable has no such file,
        // and one that needed it says so on stderr, which a failed run keeps.
        let home = tempdir();
        let root = home.join("runs");
        isolation(&["absent.json"])
            .provision_from(&root, "vendor", Some(&home))
            .expect("provisions anyway");
        assert!(!root.join("vendor").join("absent.json").exists());
    }

    #[test]
    fn credentials_with_nowhere_to_come_from_are_refused() {
        let root = tempdir();
        let err = isolation(&["auth.json"])
            .provision_from(&root, "vendor", None)
            .expect_err("must refuse");
        assert!(err.to_string().contains("HOME"));
    }

    #[test]
    fn a_credential_entry_cannot_reach_outside_the_home_directory() {
        let mut iso = isolation(&["../../.ssh/id_rsa"]);
        assert!(iso.validate("v").is_err());
        iso.credentials = vec!["auth.json".to_string()];
        assert!(iso.validate("v").is_ok());
    }

    #[test]
    fn a_home_source_cannot_climb_out_of_the_home_directory() {
        let iso = Isolation {
            home_env: "VENDOR_HOME".to_string(),
            home_source: "../elsewhere".to_string(),
            credentials: Vec::new(),
        };
        assert!(iso.validate("v").is_err());
    }

    #[test]
    fn an_unusable_environment_variable_name_is_refused() {
        let iso = Isolation {
            home_env: "NOT AN ENV NAME".to_string(),
            home_source: ".vendor".to_string(),
            credentials: Vec::new(),
        };
        assert!(iso.validate("v").is_err());
    }

    fn tempdir() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ostraka-isolation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }
}
