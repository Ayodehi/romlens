//! Settings kept between runs, in `$XDG_CONFIG_HOME/romlens/settings.json`.
//! The macOS shell keeps the same switches in `UserDefaults`. Missing or
//! unreadable files give the defaults: a setting is never worth refusing to
//! start for.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// View › Show Explanations, stored inverted so the default is "shown".
    pub hide_explanations: bool,
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("romlens")
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        // Failing to remember a switch is not worth an error dialog.
        let _ = self.save_to(&Self::path());
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Written beside and renamed, so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("romlens-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_and_garbled_files_give_defaults() {
        let dir = scratch("missing");
        assert_eq!(
            Settings::load_from(&dir.join("settings.json")),
            Settings::default()
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), b"{not json").unwrap();
        assert_eq!(
            Settings::load_from(&dir.join("settings.json")),
            Settings::default()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trips_and_ignores_unknown_keys() {
        let dir = scratch("round");
        let path = dir.join("nested").join("settings.json");
        let s = Settings {
            hide_explanations: true,
        };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        std::fs::write(&path, br#"{"hide_explanations":true,"from_the_future":1}"#).unwrap();
        assert!(Settings::load_from(&path).hide_explanations);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_shown_default_survives_an_empty_object() {
        let dir = scratch("empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), b"{}").unwrap();
        assert!(!Settings::load_from(&dir.join("settings.json")).hide_explanations);
        let _ = std::fs::remove_dir_all(dir);
    }
}
