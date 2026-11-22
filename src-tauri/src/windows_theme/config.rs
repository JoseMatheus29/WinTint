//! The Windows module's section of the settings file.

use serde::{Deserialize, Serialize};

use crate::color::Rgb;

pub const DEFAULT_ACCENT: &str = "#0078d4";

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    #[default]
    Dark,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowsConfig {
    /// While on, WinTint sets the mode and accent color of Windows. Turning it
    /// off puts back what Windows had before.
    pub enabled: bool,
    /// Takes the mode and accent from the dock's theme instead of the two below.
    pub follow_dock: bool,
    pub mode: Mode,
    pub accent: String,
    pub accent_on_title_bars: bool,
    pub accent_on_start: bool,
}

impl Default for WindowsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            follow_dock: true,
            mode: Mode::Dark,
            accent: DEFAULT_ACCENT.into(),
            accent_on_title_bars: true,
            accent_on_start: false,
        }
    }
}

impl WindowsConfig {
    pub(crate) fn normalize(&mut self) {
        if Rgb::parse(&self.accent).is_none() {
            self.accent = DEFAULT_ACCENT.into();
        }
    }
}
