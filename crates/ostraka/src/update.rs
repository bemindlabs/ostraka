//! `ostraka update` — replace this binary with the current release.
//!
//! Two things make this more than a download, and both of them are refusals.
//!
//! **It will not replace a binary something else owns.** A copy installed by
//! Homebrew, by cargo or by npm is tracked by that tool: it has a manifest
//! saying which version is on this machine, and overwriting the file behind its
//! back leaves that manifest lying. The next `brew upgrade` then has nothing to
//! do, and the version it believes in is not the one that runs. So the install
//! is identified from the path this executable is running from, and where
//! somebody else owns it the command says whose it is and what to type instead.
//!
//! **A checksum that cannot be verified is a refusal, not a warning.**
//! `scripts/install.sh` prints a warning and carries on when the machine has no
//! `sha256sum`, which is a defensible trade for a script that is asking to be
//! piped into a shell anyway. It is not defensible here: this writes over the
//! binary that is running, and there is no version of "probably the right
//! bytes" that justifies doing that. The digest is computed in this process, so
//! it does not depend on which of three checksum tools a machine happens to
//! have.
//!
//! Nothing here parses HTML or trusts a redirect: the release is asked for by
//! name over the same URL shape the installer, the formula and the npm shim all
//! use, and that name is a contract `check-hygiene.sh` holds the four of them
//! to.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

type Failure = Box<dyn std::error::Error>;

const REPO: &str = "bemindlabs/ostraka";

/// Overridable so this can be exercised against a directory of locally built
/// artifacts rather than only ever by cutting a release and watching what
/// happens to other people — the same reason `install.sh` takes one.
const BASE_URL_ENV: &str = "OSTRAKA_BASE_URL";

/// Which tag to move to, when something other than the release feed decides.
///
/// The pair with `OSTRAKA_BASE_URL`, and the same pair `install.sh` takes, for
/// the same reason: without them the only way to find out whether this command
/// works is to cut a release and watch what happens to other people. With them
/// the whole path — fetch, verify, unpack, replace — runs against a directory
/// of locally built artifacts.
const VERSION_ENV: &str = "OSTRAKA_VERSION";

/// Who put this binary here.
///
/// Determined from the path it is running from, because that is the only thing
/// available: nothing is written at install time saying how, and a marker file
/// beside the binary would be one more thing to get out of step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// Nothing else is tracking this file, so this command may replace it.
    Ours,
    /// A package manager installed it and still believes it knows the version.
    Managed { name: &'static str, command: String },
}

impl Owner {
    /// What `exe` says about who owns it.
    ///
    /// Path-shaped rather than clever. Each of these is the layout the tool in
    /// question actually uses, and a false negative is the safe direction: it
    /// means the command offers to replace a file it should have left alone,
    /// which is why the check is made against several spellings rather than one.
    pub fn of(exe: &Path) -> Self {
        let path = exe.to_string_lossy().replace('\\', "/");
        let has = |needle: &str| path.contains(needle);

        if has("/Cellar/ostraka/") || has("/homebrew/") || has("/linuxbrew/") {
            return Owner::Managed {
                name: "Homebrew",
                command: "brew upgrade ostraka".to_string(),
            };
        }
        if has("/.cargo/bin/") || has("/.rustup/") {
            return Owner::Managed {
                name: "cargo",
                command: "cargo install ostraka --force".to_string(),
            };
        }
        if has("/node_modules/") || has("/npm/") || has("/.npm-global/") {
            return Owner::Managed {
                name: "npm",
                command: "npm update -g ostraka".to_string(),
            };
        }
        Owner::Ours
    }
}

/// The release target triple for the machine this is running on.
///
/// The five the release workflow builds, and nothing inferred: a platform that
/// is not in that matrix has no artifact to fetch, and saying so is better than
/// requesting a URL that will 404.
pub fn target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    })
}

/// Three numbers, compared as numbers.
///
/// `"1.10.0"` is newer than `"1.9.0"` and sorts before it as a string, which is
/// the whole reason this is not a string comparison. Anything that does not
/// parse compares as older than everything, so a tag this build cannot read
/// never triggers an update — the safe direction, since the alternative is
/// replacing a working binary on the strength of a version nobody understood.
fn parts(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next().unwrap_or(v);
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

/// Whether `latest` is a version worth moving to from `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parts(current), parts(latest)) {
        (Some(now), Some(new)) => new > now,
        _ => false,
    }
}

/// Fetches a URL to bytes, via curl.
///
/// A subprocess rather than an HTTP client, and that is a deliberate trade
/// rather than a shortcut. Every HTTP crate brings a TLS stack, and the ones
/// that are async bring a runtime — which is the dependency this project
/// refuses everywhere else, on the grounds that a binary you can curl stops
/// being one the moment it needs something installed. This binary's entire job
/// is running other people's programs; running one more is in character, and
/// `install.sh` already requires the same one.
fn fetch(url: &str) -> Result<Vec<u8>, Failure> {
    let out = Command::new("curl")
        .args(["-fsSL", "--proto", "=https,file", url])
        .output()
        .map_err(|e| -> Failure {
            format!(
                "could not run curl, which is how this fetches a release: {e}\n\
                 install curl, or download the release by hand from \
                 https://github.com/{REPO}/releases"
            )
            .into()
        })?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(format!("could not fetch {url}: {}", said.trim()).into());
    }
    Ok(out.stdout)
}

/// The tag of the newest published release.
pub fn latest_tag() -> Result<String, Failure> {
    if let Ok(pinned) = std::env::var(VERSION_ENV) {
        if !pinned.is_empty() {
            return Ok(pinned);
        }
    }
    let body = fetch(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?;
    let doc: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| -> Failure { format!("the release listing did not parse: {e}").into() })?;
    doc.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|t| t.to_string())
        .ok_or_else(|| "the release listing named no tag".into())
}

fn base_url(tag: &str) -> String {
    match std::env::var(BASE_URL_ENV) {
        Ok(base) if !base.is_empty() => base.trim_end_matches('/').to_string(),
        _ => format!("https://github.com/{REPO}/releases/download/{tag}"),
    }
}

/// Lowercase hex, which is how the sidecar writes it.
fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The first field of a `sha256sum`-shaped line.
pub fn expected_digest(sidecar: &str) -> Option<String> {
    let word = sidecar.split_whitespace().next()?;
    let looks_right = word.len() == 64
        && word
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase());
    looks_right.then(|| word.to_string())
}

/// Downloads the release archive and returns its verified bytes.
///
/// Verification is not optional and not delegated. A mismatch, a sidecar that
/// is missing, and a sidecar that does not parse are all the same answer — this
/// does not run — because each of them means the same thing: nobody can say
/// these are the right bytes, and the next step overwrites the running binary.
fn verified_archive(tag: &str, target: &str) -> Result<Vec<u8>, Failure> {
    let name = format!("ostraka-{tag}-{target}");
    let base = base_url(tag);
    let archive = fetch(&format!("{base}/{name}.tar.gz"))?;

    let sidecar = fetch(&format!("{base}/{name}.tar.gz.sha256")).map_err(|e| -> Failure {
        format!(
            "{name}.tar.gz has no readable checksum beside it ({e}), so these bytes \
             cannot be verified — and this command overwrites the binary that is \
             running, which is not something to do on unverified bytes"
        )
        .into()
    })?;
    let sidecar = String::from_utf8_lossy(&sidecar);
    let expected = expected_digest(&sidecar).ok_or_else(|| -> Failure {
        format!("the checksum published for {name}.tar.gz is not a sha256 digest").into()
    })?;

    let actual = digest(&archive);
    if actual != expected {
        return Err(format!(
            "checksum mismatch for {name}.tar.gz\n  published {expected}\n  downloaded {actual}"
        )
        .into());
    }
    Ok(archive)
}

/// Unpacks the archive and returns the path of the binary inside it.
///
/// `tar` as a subprocess for the same reason as curl, and with the same
/// precedent: `install.sh` requires it, and Windows has shipped it since 10.
fn unpack(archive: &[u8], tag: &str, target: &str, into: &Path) -> Result<PathBuf, Failure> {
    let name = format!("ostraka-{tag}-{target}");
    let tarball = into.join(format!("{name}.tar.gz"));
    std::fs::write(&tarball, archive)?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(into)
        .status()
        .map_err(|e| -> Failure {
            format!("could not run tar to unpack the release: {e}").into()
        })?;
    if !status.success() {
        return Err(format!("tar could not unpack {name}.tar.gz").into());
    }
    let binary = into.join(&name).join(if cfg!(windows) {
        "ostraka.exe"
    } else {
        "ostraka"
    });
    if !binary.is_file() {
        return Err(format!("{name}.tar.gz did not contain an ostraka binary").into());
    }
    Ok(binary)
}

/// Puts `new` where `exe` is, as atomically as the platform allows.
///
/// On Unix a rename over a running executable is fine: the running process
/// keeps the old inode and the next one gets the new file.
///
/// Windows needs one more step, and it is renaming rather than deleting that
/// makes it possible. The loader holds the running image open in a way that
/// refuses a delete and permits a rename, so the running file is moved aside
/// first and the new one takes its place. The `.old` is left behind because it
/// cannot be removed while it is running; the next update sweeps it up. A file
/// nobody deleted is untidy, and it is the cheaper of the two outcomes.
///
/// Every failure after that move puts the original back. Raised in review, and
/// it is the one path where getting it wrong leaves somebody with no binary at
/// all — which is worse than every other failure here, all of which leave the
/// working one exactly where it was.
fn place(new: &Path, exe: &Path) -> Result<(), Failure> {
    // Onto the same filesystem as the target: a rename across a device
    // boundary is a copy that can fail halfway, which is the one thing this
    // step exists to rule out.
    let staged = exe.with_extension("new");
    std::fs::copy(new, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }

    let aside = exe.with_extension("old");
    let moved_aside = if cfg!(windows) {
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(exe, &aside).inspect_err(|_| {
            let _ = std::fs::remove_file(&staged);
        })?;
        true
    } else {
        false
    };

    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::remove_file(&staged);
        if moved_aside {
            // The window this closes: the original has been moved and the
            // replacement has not landed, so `exe` names nothing. Putting it
            // back is the only outcome in which the operator still has the
            // program they started with.
            if let Err(back) = std::fs::rename(&aside, exe) {
                return Err(format!(
                    "could not install the new binary ({e}), and could not put the old \
                     one back either ({back}) — it is at {}, and moving it to {} by hand \
                     restores what was there",
                    aside.display(),
                    exe.display()
                )
                .into());
            }
        }
        return Err(format!("could not install the new binary: {e}").into());
    }
    Ok(())
}

/// What `update` found, before it decides what to do about it.
pub struct Standing {
    pub current: String,
    pub latest: String,
    pub newer: bool,
    pub owner: Owner,
    pub exe: PathBuf,
}

/// `ostraka update`.
pub fn run(check_only: bool, json: bool) -> Result<bool, Failure> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let exe = std::env::current_exe()?;
    // Through any symlink, because `~/.local/bin/ostraka` pointing into a
    // Homebrew Cellar is exactly the case where replacing what the link names
    // would go behind Homebrew's back while looking like it did not.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let owner = Owner::of(&exe);

    let latest = latest_tag()?;
    let newer = is_newer(&current, &latest);
    let standing = Standing {
        current: current.clone(),
        latest: latest.clone(),
        newer,
        owner: owner.clone(),
        exe: exe.clone(),
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "current": standing.current,
                "latest": standing.latest,
                "update_available": standing.newer,
                "path": standing.exe.display().to_string(),
                "managed_by": match &standing.owner {
                    Owner::Ours => serde_json::Value::Null,
                    Owner::Managed { name, .. } => serde_json::Value::String(name.to_string()),
                },
                "command": match &standing.owner {
                    Owner::Ours => serde_json::Value::Null,
                    Owner::Managed { command, .. } => serde_json::Value::String(command.clone()),
                },
            }))?
        );
        return Ok(true);
    }

    if !newer {
        println!("ostraka {current} is current (latest release is {latest})");
        return Ok(true);
    }
    println!("ostraka {current} → {latest} available");

    if let Owner::Managed { name, command } = &owner {
        // Not a failure. The answer to "is there an update" is yes, and the
        // way to take it is one line further down.
        println!("  {} installed this, at {}", name, exe.display());
        println!("  take it with:  {command}");
        return Ok(true);
    }
    if check_only {
        println!("  run `ostraka update` to take it");
        return Ok(true);
    }

    let Some(target) = target() else {
        return Err(format!(
            "no release is published for {}-{}, so there is nothing to update to",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
        .into());
    };

    println!("  downloading ostraka-{latest}-{target}");
    let archive = verified_archive(&latest, target)?;
    println!("  checksum ok");

    let staging = std::env::temp_dir().join(format!("ostraka-update-{}", std::process::id()));
    std::fs::create_dir_all(&staging)?;
    let outcome = unpack(&archive, &latest, target, &staging).and_then(|new| place(&new, &exe));
    let _ = std::fs::remove_dir_all(&staging);
    outcome?;

    println!("  installed {latest} to {}", exe.display());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_a_package_manager_owns_is_not_ours_to_replace() {
        // Each of these is a real layout, and getting one wrong means writing
        // over a file whose manager still believes it knows the version — so
        // the next upgrade has nothing to do and reports a version that is not
        // the one running.
        for (path, manager) in [
            ("/opt/homebrew/Cellar/ostraka/1.0.0/bin/ostraka", "Homebrew"),
            ("/home/linuxbrew/.linuxbrew/bin/ostraka", "Homebrew"),
            ("/home/x/.cargo/bin/ostraka", "cargo"),
            ("/usr/lib/node_modules/ostraka/bin/ostraka", "npm"),
            ("C:/Users/x/.cargo/bin/ostraka.exe", "cargo"),
        ] {
            match Owner::of(Path::new(path)) {
                Owner::Managed { name, command } => {
                    assert_eq!(name, manager, "{path}");
                    assert!(!command.is_empty(), "{path} named no way to update");
                }
                Owner::Ours => panic!("{path} was treated as ours to overwrite"),
            }
        }
    }

    #[test]
    fn a_binary_nothing_is_tracking_is_ours() {
        for path in [
            "/home/x/.local/bin/ostraka",
            "/usr/local/bin/ostraka",
            "/home/x/bin/ostraka",
        ] {
            assert_eq!(Owner::of(Path::new(path)), Owner::Ours, "{path}");
        }
    }

    #[test]
    fn versions_compare_as_numbers_and_an_unreadable_one_never_updates() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "v1.0.1"));
        // The whole reason this is not a string comparison.
        assert!(is_newer("1.9.0", "1.10.0"));
        assert!(!is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        // A tag this build cannot read must not cause it to overwrite a
        // working binary, so anything unparseable is "no update".
        assert!(!is_newer("1.0.0", "nightly"));
        assert!(!is_newer("1.0.0", "1.0"));
        assert!(!is_newer("1.0.0", "1.0.0.1"));
        assert!(!is_newer("not-a-version", "2.0.0"));
    }

    #[test]
    fn a_prerelease_tag_is_read_by_its_numbers() {
        // The release matrix has never published one, and if it ever does, the
        // three numbers are what decides. Nothing here invents an ordering
        // between two prereleases of the same version.
        assert!(is_newer("1.0.0", "1.0.1-rc.1"));
        assert!(!is_newer("1.0.1", "1.0.1-rc.1"));
    }

    #[test]
    fn a_checksum_sidecar_is_read_strictly() {
        // sha256 of the empty string: sixty-four lowercase hex characters,
        // which is the only shape a sidecar is allowed to be.
        let good = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(good.len(), 64, "the fixture is not a digest");

        assert_eq!(
            expected_digest(&format!("{good}  ostraka-v1.0.0-x86_64.tar.gz")).as_deref(),
            Some(good)
        );
        assert_eq!(expected_digest(good).as_deref(), Some(good), "no filename");

        // Everything that is not plainly a digest is refused rather than
        // guessed at, because the next thing this does is overwrite a binary.
        assert!(expected_digest("").is_none());
        assert!(expected_digest("   ").is_none());
        assert!(expected_digest("not-a-digest  file").is_none());
        assert!(
            expected_digest(&good[..63]).is_none(),
            "sixty-three characters is not a digest"
        );
        assert!(
            expected_digest(&format!("{good}ab")).is_none(),
            "sixty-six characters is not a digest either"
        );
        assert!(
            expected_digest(&good.to_uppercase()).is_none(),
            "uppercase is not what the sidecar writes, so it is not assumed to be one"
        );
    }

    #[test]
    fn the_notice_names_the_way_out_that_matches_who_owns_the_binary() {
        // The same finding as `update` itself: telling somebody whose copy
        // Homebrew installed to run `ostraka update` sends them to a command
        // that will refuse. It has to name theirs.
        let ours = Path::new("/home/x/.local/bin/ostraka");
        let brewed = Path::new("/opt/homebrew/Cellar/ostraka/1.0.0/bin/ostraka");
        assert_eq!(
            notice::line("9.9.9", ours),
            None,
            "a current version was offered an update"
        );
        // Without a cache there is nothing to say, which is the state every
        // test runs in and is itself the assertion worth making: this must
        // never reach the network to answer.
        assert_eq!(notice::line("0.0.1", brewed), notice::line("0.0.1", brewed));
    }

    #[test]
    fn this_machine_has_a_release_target() {
        // Not a tautology: it fails on a platform the release matrix does not
        // build, which is the moment somebody needs to be told rather than
        // sent to a URL that 404s.
        assert!(
            target().is_some(),
            "no release target for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
}

/// Noticing a release without being asked.
///
/// Three rules, and they are what makes this acceptable rather than rude.
///
/// **It never costs a command any time.** The notice is read from a cache and
/// only from a cache; the refresh is a detached process nothing waits on. So a
/// run that happens to be the first one after the cache went stale prints
/// nothing new — the *next* one does. Blocking a run to ask GitHub about a
/// version would put a network round trip in front of work somebody is paying a
/// vendor for, which no amount of freshness is worth.
///
/// **It stays out of the way of anything reading the output.** Nothing is
/// printed unless stderr is a terminal, so a pipeline, a script and every
/// `--json` caller see exactly what they saw before.
///
/// **It can be turned off, and being off is remembered by nothing.** One
/// environment variable, checked every time.
pub mod notice {
    use super::{Owner, is_newer};
    use std::io::IsTerminal;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    /// Set to anything at all to stop this checking, permanently and everywhere.
    pub const OFF: &str = "OSTRAKA_NO_UPDATE_CHECK";

    /// How long a cached answer is worth believing.
    ///
    /// A day. The thing being watched changes a few times a year, and the cost
    /// of being a day late to hear about it is nil.
    const STALE_AFTER: Duration = Duration::from_secs(60 * 60 * 24);

    /// Where the cached answer lives.
    ///
    /// The user's cache directory, not the workspace: this is a fact about the
    /// machine, and a file about GitHub releases appearing inside somebody's
    /// project would be litter. Deleting it costs a day of staleness.
    fn cache() -> Option<PathBuf> {
        let base = match std::env::var_os("XDG_CACHE_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
        };
        Some(base.join("ostraka"))
    }

    fn enabled() -> bool {
        std::env::var_os(OFF).is_none()
    }

    /// The tag in the cache, and whether it is old enough to refresh.
    fn cached() -> (Option<String>, bool) {
        let Some(file) = cache().map(|d| d.join("latest.json")) else {
            return (None, false);
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return (None, true);
        };
        let tag = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|d| d.get("tag_name")?.as_str().map(str::to_string));
        // A file that did not parse is a half-written download, and its mtime
        // is fresh — so freshness is judged on having an answer, not on having
        // a file. Otherwise one interrupted refresh buys a day of silence.
        let fresh = tag.is_some()
            && std::fs::metadata(&file)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age < STALE_AFTER);
        (tag, !fresh)
    }

    /// Starts a refresh that nothing waits on.
    ///
    /// Detached deliberately: a thread would be killed when the process exits,
    /// and joining it would be the blocking this exists to avoid. curl writes
    /// the file; a partial one simply fails to parse next time and is treated
    /// as stale, which makes an interrupted refresh self-healing rather than a
    /// day of silence.
    fn refresh() {
        let Some(dir) = cache() else { return };
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let file = dir.join("latest.json");
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            super::REPO
        );
        let _ = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "20", "-o"])
            .arg(&file)
            .arg(&url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    /// The line to print, if there is one. Reads the cache; never the network.
    pub fn line(current: &str, exe: &std::path::Path) -> Option<String> {
        let (tag, _) = cached();
        let latest = tag?;
        if !is_newer(current, &latest) {
            return None;
        }
        let how = match Owner::of(exe) {
            Owner::Ours => "ostraka update".to_string(),
            Owner::Managed { command, .. } => command,
        };
        Some(format!(
            "ostraka {current} → {latest} is available. `{how}` to take it, \
             or set {OFF} to stop saying so."
        ))
    }

    /// Says so if there is something to say, and starts the next refresh.
    ///
    /// Called once, on the way out, after whatever was asked for has already
    /// been printed — so the notice is the last thing on the screen rather than
    /// something in front of the answer.
    pub fn offer(json: bool) {
        if json || !enabled() || !std::io::stderr().is_terminal() {
            return;
        }
        let current = env!("CARGO_PKG_VERSION");
        let exe = std::env::current_exe()
            .and_then(|p| std::fs::canonicalize(&p).or(Ok(p)))
            .unwrap_or_default();
        if let Some(said) = line(current, &exe) {
            eprintln!("\n{said}");
        }
        let (_, stale) = cached();
        if stale {
            refresh();
        }
    }
}
