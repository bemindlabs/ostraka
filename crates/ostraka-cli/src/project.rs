//! Loading a project: its config and its adapter profiles.

use ostraka_adapter::Profile;
use ostraka_core::config::Config;
use std::path::Path;

type Loaded<T> = Result<T, Box<dyn std::error::Error>>;

pub fn load_config(project: &Path) -> Loaded<Config> {
    let path = project.join("ostraka.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Config::parse(&text)?)
}

/// Reads every `*.toml` in `adapters/`, in sorted order.
pub fn load_profiles(project: &Path) -> Loaded<Vec<Profile>> {
    let dir = project.join("adapters");
    if !dir.is_dir() {
        return Err(format!("no adapters directory at {}", dir.display()).into());
    }

    let mut paths: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();

    let mut profiles = Vec::with_capacity(paths.len());
    for path in paths {
        let text = std::fs::read_to_string(&path)?;
        profiles.push(Profile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    Ok(profiles)
}
