//! The Windows module: light or dark mode and the accent color, taken from the
//! dock's theme or chosen by hand.
//!
//! Windows has no public API for the accent color, so this writes the same
//! registry values the Settings app does and then tells running programs.
//! Before the first change it saves the original values, and turning the
//! module off writes them back.

pub mod config;

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::color::Rgb;
use crate::config::AppConfig;
use crate::dock::config::Theme;
use crate::win::{self, RegValue};
use config::{DEFAULT_ACCENT, Mode};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const DWM: &str = r"Software\Microsoft\Windows\DWM";
const ACCENT: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
const DESKTOP: &str = r"Control Panel\Desktop";

/// Every value this module may change, saved before its first change.
const TOUCHED: [(&str, &str); 11] = [
    (PERSONALIZE, "AppsUseLightTheme"),
    (PERSONALIZE, "SystemUsesLightTheme"),
    (PERSONALIZE, "ColorPrevalence"),
    (DWM, "AccentColor"),
    (DWM, "ColorizationColor"),
    (DWM, "ColorizationAfterglow"),
    (DWM, "ColorPrevalence"),
    (ACCENT, "AccentPalette"),
    (ACCENT, "AccentColorMenu"),
    (ACCENT, "StartColorMenu"),
    (DESKTOP, "AutoColorization"),
];

const SNAPSHOT_FILE: &str = "windows-original.json";
/// Bars brighter than this get the light mode, as in the settings window.
const LIGHT_BAR_LUMINANCE: f64 = 0.5;
/// Opacity Windows itself uses for the colorization color.
const COLORIZATION_ALPHA: u32 = 0xc4;
/// How much more colorful a detail must be to stand for the theme over the bar.
const SIGNATURE_MARGIN: f64 = 0.1;
/// Keeps the accent usable behind white or black text.
const ACCENT_LIGHTNESS: (f64, f64) = (0.2, 0.75);

/// What Windows should look like.
#[derive(Clone, Copy, PartialEq)]
struct Target {
    dark: bool,
    accent: Rgb,
    title_bars: bool,
    start: bool,
}

#[derive(Serialize)]
pub struct Look {
    dark: bool,
    accent: String,
}

#[derive(Serialize, Deserialize)]
struct SavedValue {
    key: String,
    name: String,
    /// None when Windows did not have the value at all.
    value: Option<RegValue>,
}

/// None asks for the original look back.
pub struct WindowsThemeState(Mutex<Sender<Option<Target>>>);

/// What the module would apply for a config, shown in the settings window.
#[tauri::command]
pub fn get_windows_look(config: AppConfig) -> Look {
    let target = target(&config);
    Look {
        dark: target.dark,
        accent: target.accent.hex(),
    }
}

/// How much color there is: 0 for grays, black and white, 1 for pure hues.
fn chroma(color: Rgb) -> f64 {
    let (_, s, l) = color.hsl();
    s * (1.0 - (2.0 * l - 1.0).abs())
}

/// The color that identifies a theme: the most colorful one, looking at the
/// bar first since it covers the most area. A detail color only takes over
/// when it is clearly more colorful, as on a gray or black bar.
pub fn signature_color(theme: &Theme) -> Option<Rgb> {
    let mut candidates = vec![&theme.bar_color];
    if theme.use_gradient {
        candidates.push(&theme.bar_color2);
    }
    if theme.glow_size > 0.0 {
        candidates.push(&theme.glow_color);
    }
    candidates.extend([&theme.border_color, &theme.active_color, &theme.dot_color]);
    candidates
        .into_iter()
        .filter_map(|hex| Rgb::parse(hex))
        .fold(None, |best, color| match best {
            Some(best) if chroma(color) <= chroma(best) + SIGNATURE_MARGIN => Some(best),
            _ => Some(color),
        })
}

fn from_dock(theme: &Theme) -> (bool, Rgb) {
    let luminance = |hex: &str| Rgb::parse(hex).map_or(0.0, Rgb::luminance);
    let brightness = if theme.use_gradient {
        (luminance(&theme.bar_color) + luminance(&theme.bar_color2)) / 2.0
    } else {
        luminance(&theme.bar_color)
    };
    let accent = signature_color(theme).unwrap_or_else(|| Rgb::parse(DEFAULT_ACCENT).unwrap());
    (brightness <= LIGHT_BAR_LUMINANCE, accent)
}

fn target(config: &AppConfig) -> Target {
    let windows = &config.windows;
    let (dark, accent) = if windows.follow_dock {
        from_dock(&config.dock.theme)
    } else {
        let accent = Rgb::parse(&windows.accent).unwrap_or_else(|| Rgb::parse(DEFAULT_ACCENT).unwrap());
        (windows.mode == Mode::Dark, accent)
    };
    let (h, s, l) = accent.hsl();
    let (low, high) = ACCENT_LIGHTNESS;
    Target {
        dark,
        accent: Rgb::from_hsl(h, s, l.clamp(low, high)),
        title_bars: windows.accent_on_title_bars,
        start: windows.accent_on_start,
    }
}

/// The eight shades Windows keeps for an accent, from lightest to darkest. The
/// last one is unused by Windows 11.
fn shades(accent: Rgb) -> [Rgb; 8] {
    [
        accent.mix(Rgb::WHITE, 0.7),
        accent.mix(Rgb::WHITE, 0.5),
        accent.mix(Rgb::WHITE, 0.25),
        accent,
        accent.mix(Rgb::BLACK, 0.15),
        accent.mix(Rgb::BLACK, 0.3),
        accent.mix(Rgb::BLACK, 0.45),
        accent.mix(Rgb::BLACK, 0.6),
    ]
}

/// Windows stores most accent colors as 0xAABBGGRR.
fn abgr(color: Rgb) -> u32 {
    0xff00_0000 | (color.b as u32) << 16 | (color.g as u32) << 8 | color.r as u32
}

fn argb(color: Rgb, alpha: u32) -> u32 {
    alpha << 24 | (color.r as u32) << 16 | (color.g as u32) << 8 | color.b as u32
}

fn write(target: Target) -> Result<(), String> {
    let light = !target.dark as u32;
    let shades = shades(target.accent);
    let palette: Vec<u8> = shades
        .iter()
        .enumerate()
        .flat_map(|(i, c)| [c.r, c.g, c.b, if i < 7 { 0xff } else { 0 }])
        .collect();
    let colorization = argb(target.accent, COLORIZATION_ALPHA);

    // Without this, Windows would pick the accent from the wallpaper again.
    win::set_user_dword(DESKTOP, "AutoColorization", 0)?;
    win::set_user_dword(PERSONALIZE, "AppsUseLightTheme", light)?;
    win::set_user_dword(PERSONALIZE, "SystemUsesLightTheme", light)?;
    win::set_user_dword(PERSONALIZE, "ColorPrevalence", target.start as u32)?;
    win::set_user_binary(ACCENT, "AccentPalette", &palette)?;
    win::set_user_dword(ACCENT, "AccentColorMenu", abgr(target.accent))?;
    win::set_user_dword(ACCENT, "StartColorMenu", abgr(shades[5]))?;
    win::set_user_dword(DWM, "AccentColor", abgr(target.accent))?;
    win::set_user_dword(DWM, "ColorizationColor", colorization)?;
    win::set_user_dword(DWM, "ColorizationAfterglow", colorization)?;
    win::set_user_dword(DWM, "ColorPrevalence", target.title_bars as u32)?;
    win::set_dwm_colorization(colorization);
    win::broadcast_color_change();
    Ok(())
}

fn read_originals(values: &[(&str, &str)]) -> Vec<SavedValue> {
    values
        .iter()
        .map(|(key, name)| SavedValue {
            key: key.to_string(),
            name: name.to_string(),
            value: win::read_user_value(key, name),
        })
        .collect()
}

/// Writes saved values back, deleting the ones Windows did not have.
fn put_back(saved: &[SavedValue]) -> Result<(), String> {
    for value in saved {
        match &value.value {
            Some(original) => win::write_user_value(&value.key, &value.name, original)?,
            None => win::delete_user_value(&value.key, &value.name),
        }
    }
    Ok(())
}

fn snapshot_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(SNAPSHOT_FILE))
}

/// Saves the original values once. A file left from an earlier run already
/// holds them, and reading the registry now would only find WinTint's colors.
fn save_snapshot(app: &AppHandle) -> Result<(), String> {
    let path = snapshot_path(app)?;
    if path.exists() {
        return Ok(());
    }
    let json = serde_json::to_string_pretty(&read_originals(&TOUCHED)).map_err(|e| e.to_string())?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

fn restore(app: &AppHandle) -> Result<(), String> {
    let path = snapshot_path(app)?;
    if !path.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let saved: Vec<SavedValue> = serde_json::from_str(&text)
        .map_err(|e| format!("Erro em {}: {e}", path.display()))?;
    put_back(&saved)?;
    let colorization = saved
        .iter()
        .find(|value| value.key == DWM && value.name == "ColorizationColor")
        .and_then(|value| value.value.as_ref())
        .and_then(RegValue::as_dword);
    if let Some(colorization) = colorization {
        win::set_dwm_colorization(colorization);
    }
    win::broadcast_color_change();
    fs::remove_file(&path).map_err(|e| e.to_string())
}

/// Runs every change in order on one thread: writing to the registry and
/// telling every window can take a moment, and two changes must never mix.
fn spawn_worker(app: AppHandle, jobs: Receiver<Option<Target>>) {
    thread::spawn(move || {
        let mut applied: Option<Target> = None;
        while let Ok(mut wanted) = jobs.recv() {
            // Only the latest request matters after a burst of changes.
            while let Ok(newer) = jobs.try_recv() {
                wanted = newer;
            }
            let result = match wanted {
                None => restore(&app).map(|_| applied = None),
                Some(target) if applied != Some(target) => save_snapshot(&app)
                    .and_then(|_| write(target))
                    .map(|_| applied = Some(target)),
                Some(_) => Ok(()),
            };
            if let Err(error) = result {
                let _ = app.emit("windows-theme-error", error);
            }
        }
    });
}

pub fn apply(app: &AppHandle, config: &AppConfig) {
    let wanted = config.windows.enabled.then(|| target(config));
    let state = app.state::<WindowsThemeState>();
    let _ = state.0.lock().unwrap().send(wanted);
}

/// Also puts back the original look if the app was closed with the module off
/// before it finished doing so.
pub fn start(app: &AppHandle, config: Option<&AppConfig>) {
    let (sender, jobs) = mpsc::channel();
    app.manage(WindowsThemeState(Mutex::new(sender)));
    spawn_worker(app.clone(), jobs);
    if let Some(config) = config {
        apply(app, config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_match_the_registry_layout() {
        // The values Windows 11 itself stores for its default blue.
        let blue = Rgb::parse("#0078d4").unwrap();
        assert_eq!(abgr(blue), 0xffd4_7800);
        assert_eq!(argb(blue, COLORIZATION_ALPHA), 0xc400_78d4);
    }

    #[test]
    fn follows_the_dock_theme() {
        let mut config = AppConfig::default();
        config.windows.enabled = true;

        config.dock.theme.bar_color = "#f97316".into();
        config.dock.theme.bar_color2 = "#1a1a1a".into();
        config.dock.theme.use_gradient = true;
        config.dock.theme.active_color = "#facc15".into();
        let ninja = target(&config);
        assert!(ninja.dark);
        // The orange bar, not the yellow dot under the focused app.
        assert_eq!(ninja.accent, Rgb::parse("#f97316").unwrap());

        config.dock.theme.bar_color = "#ffc2dc".into();
        config.dock.theme.bar_color2 = "#fff1f7".into();
        config.dock.theme.active_color = "#e6004c".into();
        let pink = target(&config);
        assert!(!pink.dark);
        // A pastel bar loses to a clearly more colorful detail.
        assert_eq!(pink.accent, Rgb::parse("#e6004c").unwrap());

        config.dock.theme.bar_color = "#18181c".into();
        config.dock.theme.use_gradient = false;
        config.dock.theme.border_color = "#ffffff".into();
        config.dock.theme.active_color = "#6ee7f9".into();
        let graphite = target(&config);
        assert!(graphite.dark);
        assert_eq!(graphite.accent, Rgb::parse("#6ee7f9").unwrap());
    }

    /// Uses a key of its own, so the real colors of Windows are never touched.
    #[test]
    fn puts_back_exactly_what_was_there() {
        const KEY: &str = r"Software\WinTintTest";
        win::set_user_dword(KEY, "Existing", 7).unwrap();
        win::delete_user_value(KEY, "Missing");
        let values = [(KEY, "Existing"), (KEY, "Missing")];

        let saved = read_originals(&values);
        let json = serde_json::to_string(&saved).unwrap();
        win::set_user_dword(KEY, "Existing", 99).unwrap();
        win::set_user_binary(KEY, "Missing", &[1, 2, 3]).unwrap();

        let saved: Vec<SavedValue> = serde_json::from_str(&json).unwrap();
        put_back(&saved).unwrap();
        let existing = win::read_user_value(KEY, "Existing");
        let missing = win::read_user_value(KEY, "Missing");
        win::delete_user_value(KEY, "Existing");
        win::delete_test_key(KEY);

        assert_eq!(existing.and_then(|value| value.as_dword()), Some(7));
        assert!(missing.is_none());
    }

    #[test]
    fn keeps_the_accent_readable() {
        let mut config = AppConfig::default();
        config.windows.follow_dock = false;
        config.windows.accent = "#ffffff".into();
        let (_, _, l) = target(&config).accent.hsl();
        assert!(l <= ACCENT_LIGHTNESS.1 + 0.01);
    }
}
