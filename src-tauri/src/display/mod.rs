//! Brightness of each screen. Windows itself only controls laptop screens, so
//! WinTint dims through the gamma ramp Windows sends to every screen instead.
//! That works on any monitor, but only darkens, and Windows refuses ramps that
//! dim too much, so the floor is about half of what the monitor shows.
//!
//! A screen may already have its own curve, from a calibration or the video
//! driver. WinTint keeps that curve and only scales it, and saves it to a file
//! before the first change so it comes back even after a crash.

pub mod config;
mod screens;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use config::{DisplayConfig, FULL_BRIGHTNESS, MIN_BRIGHTNESS};
use screens::Ramp;

/// How often a dimmed screen is checked, since sleep, a display change or
/// another program can reset its ramp.
const KEEP_INTERVAL: Duration = Duration::from_secs(3);
/// How much brighter to try when Windows refuses a ramp.
const REFUSED_STEP: u32 = 2;
/// Difference, in ramp units, still counted as the same curve.
const RAMP_TOLERANCE: u16 = 512;
const ORIGINALS_FILE: &str = "display-original.json";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInfo {
    id: String,
    name: String,
    primary: bool,
    brightness: u32,
}

#[derive(Default)]
struct Dimming {
    /// The curve each dimmed screen had before WinTint.
    originals: BTreeMap<String, Ramp>,
    /// Brightness WinTint has set, by device, for screens it dimmed.
    percent: BTreeMap<String, u32>,
}

pub struct DisplayState {
    dimming: Mutex<Dimming>,
    originals_path: Option<PathBuf>,
}

#[tauri::command]
pub fn get_screens(app: AppHandle) -> Vec<ScreenInfo> {
    let config = crate::config::load(&app).map(|config| config.display).unwrap_or_default();
    screens::list()
        .into_iter()
        .map(|screen| ScreenInfo {
            brightness: config.brightness(&screen.device),
            id: screen.device,
            name: screen.name,
            primary: screen.primary,
        })
        .collect()
}

/// Changes one screen right away while a slider moves. Returns the brightness
/// that took effect, which is higher when Windows refused the one asked for.
#[tauri::command]
pub fn set_brightness(id: String, percent: u32, state: State<DisplayState>) -> Result<u32, String> {
    set(&state, &id, percent)
}

fn set(state: &DisplayState, device: &str, percent: u32) -> Result<u32, String> {
    let mut dimming = state.dimming.lock().unwrap();
    let percent = percent.clamp(MIN_BRIGHTNESS, FULL_BRIGHTNESS);

    if percent == FULL_BRIGHTNESS {
        if let Some(original) = dimming.originals.remove(device) {
            screens::set_ramp(device, &original);
            dimming.percent.remove(device);
            state.save_originals(&dimming.originals);
        }
        return Ok(FULL_BRIGHTNESS);
    }

    if !dimming.originals.contains_key(device) {
        let original = screens::get_ramp(device)
            .ok_or_else(|| format!("O Windows não deixou ler o brilho de {device}."))?;
        dimming.originals.insert(device.to_string(), original);
        state.save_originals(&dimming.originals);
    }
    let original = dimming.originals[device];
    let mut applied = percent;
    while !screens::set_ramp(device, &screens::scaled(&original, applied as f64 / 100.0)) {
        if applied == FULL_BRIGHTNESS {
            return Err(format!("O Windows não aceitou mudar o brilho de {device}."));
        }
        applied = (applied + REFUSED_STEP).min(FULL_BRIGHTNESS);
    }
    dimming.percent.insert(device.to_string(), applied);
    Ok(applied)
}

impl DisplayState {
    fn save_originals(&self, originals: &BTreeMap<String, Ramp>) {
        let Some(path) = &self.originals_path else {
            return;
        };
        if originals.is_empty() {
            let _ = fs::remove_file(path);
            return;
        }
        let saved: BTreeMap<&String, Vec<Vec<u16>>> = originals
            .iter()
            .map(|(device, ramp)| (device, ramp.iter().map(|channel| channel.to_vec()).collect()))
            .collect();
        if let Ok(json) = serde_json::to_string(&saved) {
            let _ = fs::write(path, json);
        }
    }
}

/// Brings every screen to the brightness in the config.
pub fn apply(app: &AppHandle, config: &DisplayConfig) {
    let state = app.state::<DisplayState>();
    for screen in screens::list() {
        let wanted = config.brightness(&screen.device);
        let current = state
            .dimming
            .lock()
            .unwrap()
            .percent
            .get(&screen.device)
            .copied()
            .unwrap_or(FULL_BRIGHTNESS);
        if wanted != current {
            let _ = set(&state, &screen.device, wanted);
        }
    }
}

/// Puts back curves saved by a WinTint that did not close properly.
fn restore_saved_originals(path: &PathBuf) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    if let Ok(saved) = serde_json::from_str::<BTreeMap<String, Vec<Vec<u16>>>>(&text) {
        for (device, channels) in saved {
            let ramp: Option<Ramp> = channels
                .iter()
                .map(|channel| <[u16; 256]>::try_from(channel.as_slice()).ok())
                .collect::<Option<Vec<_>>>()
                .and_then(|channels| channels.try_into().ok());
            if let Some(ramp) = ramp {
                screens::set_ramp(&device, &ramp);
            }
        }
    }
    let _ = fs::remove_file(path);
}

fn spawn_keeper(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(KEEP_INTERVAL);
        let expected: Vec<(String, Ramp)> = {
            let state = app.state::<DisplayState>();
            let dimming = state.dimming.lock().unwrap();
            dimming
                .percent
                .iter()
                .map(|(device, percent)| {
                    let original = &dimming.originals[device];
                    (device.clone(), screens::scaled(original, *percent as f64 / 100.0))
                })
                .collect()
        };
        for (device, expected) in expected {
            let drifted = screens::get_ramp(&device).is_none_or(|current| {
                current
                    .iter()
                    .flatten()
                    .zip(expected.iter().flatten())
                    .any(|(a, b)| a.abs_diff(*b) > RAMP_TOLERANCE)
            });
            if drifted {
                screens::set_ramp(&device, &expected);
            }
        }
    });
}

pub fn start(app: &AppHandle, config: Option<&DisplayConfig>) {
    let originals_path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(ORIGINALS_FILE));
    if let Some(path) = &originals_path {
        restore_saved_originals(path);
    }
    app.manage(DisplayState {
        dimming: Mutex::new(Dimming::default()),
        originals_path,
    });
    if let Some(config) = config {
        apply(app, config);
    }
    spawn_keeper(app.clone());
}

/// Puts every dimmed screen back to its own curve as the program ends.
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<DisplayState>();
    let mut dimming = state.dimming.lock().unwrap();
    for (device, original) in &dimming.originals {
        screens::set_ramp(device, original);
    }
    dimming.originals.clear();
    dimming.percent.clear();
    state.save_originals(&dimming.originals);
}
