//! The brightness section of the settings file.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MIN_BRIGHTNESS: u32 = 50;
pub const FULL_BRIGHTNESS: u32 = 100;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DisplayConfig {
    /// Percent for each screen, by GDI device name. Missing screens are at 100.
    pub brightness: BTreeMap<String, u32>,
}

impl DisplayConfig {
    pub(crate) fn normalize(&mut self) {
        for value in self.brightness.values_mut() {
            *value = (*value).clamp(MIN_BRIGHTNESS, FULL_BRIGHTNESS);
        }
    }

    pub fn brightness(&self, device: &str) -> u32 {
        self.brightness.get(device).copied().unwrap_or(FULL_BRIGHTNESS)
    }
}
