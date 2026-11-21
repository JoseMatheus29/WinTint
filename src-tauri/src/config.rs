//! The settings file, with one section for each module.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::dock::config::DockConfig;
use crate::windows_theme::config::WindowsConfig;

const FILE: &str = "config.json";
const VERSION: u32 = 2;
/// Where the app kept its settings while it was called Dock JM.
const LEGACY_DIR: &str = "com.jmatheus.dockjm";
const LEGACY_FILE: &str = "dock.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub version: u32,
    pub start_with_windows: bool,
    pub dock: DockConfig,
    pub windows: WindowsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: VERSION,
            start_with_windows: false,
            dock: DockConfig::default(),
            windows: WindowsConfig::default(),
        }
    }
}

impl AppConfig {
    fn normalize(&mut self) {
        self.version = VERSION;
        self.dock.normalize();
        self.windows.normalize();
    }
}

pub fn path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE))
}

fn legacy_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    Some(dir.parent()?.join(LEGACY_DIR).join(LEGACY_FILE))
}

/// Dock JM kept everything in one flat file, which is now the dock's section.
fn migrate(text: &str) -> Result<AppConfig, serde_json::Error> {
    let legacy: Value = serde_json::from_str(text)?;
    let start_with_windows = legacy
        .get("startWithWindows")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(AppConfig {
        version: VERSION,
        start_with_windows,
        dock: serde_json::from_value(legacy)?,
        windows: WindowsConfig::default(),
    })
}

/// Reads the config, creating it on first run from Dock JM's file when there is
/// one. That file is left as it was. A broken file is reported and never
/// overwritten, so the user's edits are not lost.
pub fn load(app: &AppHandle) -> Result<AppConfig, String> {
    let path = path(app)?;
    if !path.exists() {
        let mut config = match legacy_path(app).filter(|legacy| legacy.exists()) {
            Some(legacy) => {
                let text = fs::read_to_string(&legacy).map_err(|e| e.to_string())?;
                migrate(&text)
                    .map_err(|e| format!("Erro ao importar {}: {e}", legacy.display()))?
            }
            None => AppConfig::default(),
        };
        config.normalize();
        save(app, &config)?;
        return Ok(config);
    }

    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut config: AppConfig =
        serde_json::from_str(&text).map_err(|e| format!("Erro no config.json: {e}"))?;
    config.normalize();
    Ok(config)
}

pub fn save(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = path(app)?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

/// Validates a config coming from the settings window before it is stored.
pub fn store(app: &AppHandle, mut config: AppConfig) -> Result<AppConfig, String> {
    config.normalize();
    save(app, &config)?;
    Ok(config)
}
