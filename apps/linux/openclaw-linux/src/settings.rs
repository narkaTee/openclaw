use crate::config::resolve_state_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const CURRENT_ONBOARDING_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub paused: bool,
    pub canvas_enabled: bool,
    pub onboarding_seen: bool,
    pub onboarding_version: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            paused: false,
            canvas_enabled: true,
            onboarding_seen: false,
            onboarding_version: 0,
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let path = Self::path();
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(_) => return Self::default(),
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|err| err.to_string())?;
        fs::write(path, bytes).map_err(|err| err.to_string())
    }

    fn path() -> PathBuf {
        resolve_state_dir().join("linux-app.json")
    }
}
