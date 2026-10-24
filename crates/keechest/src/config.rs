//! Non-secret preferences, stored as JSON in the user's config directory.

use std::fs;
use std::path::PathBuf;

use keechest_core::GeneratorOptions;
use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 5;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Recently opened vaults, most recent first.
    pub recent: Vec<PathBuf>,
    /// `None` follows the system theme.
    pub dark: Option<bool>,
    /// Lock after this many minutes without input; 0 never.
    pub autolock_minutes: u32,
    /// Clear copied secrets from the clipboard after this many seconds; 0 never.
    pub clipboard_seconds: u32,
    pub lock_on_minimize: bool,
    pub generator: GeneratorOptions,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            dark: None,
            autolock_minutes: 5,
            clipboard_seconds: 20,
            lock_on_minimize: true,
            generator: GeneratorOptions::default(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "KeeChest").map(|d| d.config_dir().join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    /// Where a new vault goes by default.
    pub fn default_vault_path() -> PathBuf {
        let dir = directories::UserDirs::new()
            .and_then(|u| {
                u.document_dir()
                    .map(PathBuf::from)
                    .or(Some(u.home_dir().into()))
            })
            .unwrap_or_default();
        dir.join(format!("KeeChest.{}", keechest_core::format::EXTENSION))
    }
}
