//! Windows settings changed on the spot, like the switches in the Settings app.
//! They are read from Windows every time instead of being kept in the config,
//! and Windows keeps them after WinTint closes, so nothing needs restoring.
//!
//! Each area of Windows has its own file. All of them answer to the same two
//! commands: one reads every setting by name, the other changes one of them.

mod desktop;
mod effects;
mod input;
mod shell;
mod sounds;

use std::ffi::c_void;

use serde::Serialize;
use serde_json::{Map, Value};
use windows::Win32::UI::WindowsAndMessaging::{
    SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SYSTEM_PARAMETERS_INFO_ACTION,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};

/// Every setting by the name the settings page uses for it. A setting Windows
/// will not report is null.
type Values = Map<String, Value>;

#[derive(Serialize)]
pub struct Choice {
    id: String,
    /// The name Windows shows in its own language, when there is one.
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tweaks {
    values: Values,
    cursor_schemes: Vec<Choice>,
    sound_schemes: Vec<Choice>,
}

#[tauri::command(async)]
pub fn get_windows_tweaks() -> Tweaks {
    let mut values = Values::new();
    effects::read(&mut values);
    desktop::read(&mut values);
    shell::read(&mut values);
    input::read(&mut values);
    sounds::read(&mut values);
    Tweaks {
        values,
        cursor_schemes: input::cursor_schemes(),
        sound_schemes: sounds::schemes(),
    }
}

#[tauri::command(async)]
pub fn set_windows_tweak(name: String, value: Value) -> Result<(), String> {
    effects::set(&name, &value)
        .or_else(|| desktop::set(&name, &value))
        .or_else(|| shell::set(&name, &value))
        .or_else(|| input::set(&name, &value))
        .or_else(|| sounds::set(&name, &value))
        .unwrap_or_else(|| Err(format!("O ajuste {name} não existe.")))
}

#[tauri::command(async)]
pub fn get_wallpaper_preview() -> Option<String> {
    desktop::preview()
}

#[tauri::command(async)]
pub fn restart_explorer() -> Result<(), String> {
    shell::restart_explorer()
}

fn flag(value: &Value) -> Result<bool, String> {
    value.as_bool().ok_or_else(|| format!("Esperava ligado ou desligado, veio {value}."))
}

fn text(value: &Value) -> Result<&str, String> {
    value.as_str().ok_or_else(|| format!("Esperava um texto, veio {value}."))
}

fn number(value: &Value, min: u32, max: u32) -> Result<u32, String> {
    value
        .as_u64()
        .filter(|number| (min as u64..=max as u64).contains(number))
        .map(|number| number as u32)
        .ok_or_else(|| format!("Esperava um número de {min} a {max}, veio {value}."))
}

/// Reads a system parameter that Windows writes into `T`.
fn spi_get<T: Default>(action: SYSTEM_PARAMETERS_INFO_ACTION) -> Option<T> {
    let mut value = T::default();
    unsafe {
        SystemParametersInfoW(
            action,
            0,
            Some(&mut value as *mut T as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    Some(value)
}

/// Changes a system parameter, saves it for the next sign-in and tells the
/// open windows. Some parameters take the new value in `ui_param` and others
/// in the pointer itself, as each one's documentation says.
fn spi_set(action: SYSTEM_PARAMETERS_INFO_ACTION, ui_param: u32, pv_param: *mut c_void) -> Result<(), String> {
    unsafe { SystemParametersInfoW(action, ui_param, Some(pv_param), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) }
        .map_err(|error| format!("O Windows recusou a mudança: {}", error.message()))
}

/// A number passed in the pointer argument, as several parameters expect.
fn as_param(value: u32) -> *mut c_void {
    value as usize as *mut c_void
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name the settings page binds a control to.
    const NAMES: [&str; 33] = [
        "transparency",
        "animations",
        "windowShadows",
        "dragFullWindows",
        "snapWindows",
        "desktopIcons",
        "iconComputer",
        "iconUserFiles",
        "iconNetwork",
        "iconRecycleBin",
        "iconControlPanel",
        "wallpaper",
        "wallpaperFit",
        "taskbarAlignment",
        "taskbarSearch",
        "taskbarCombine",
        "taskbarTaskView",
        "taskbarSeconds",
        "taskbarAutoHide",
        "startMostUsed",
        "startRecentFiles",
        "explorerOpensTo",
        "fileExtensions",
        "hiddenFiles",
        "classicContextMenu",
        "cursorScheme",
        "cursorSize",
        "mouseSpeed",
        "mousePrecision",
        "scrollLines",
        "keyboardDelay",
        "keyboardSpeed",
        "soundScheme",
    ];

    /// Reads the real settings without changing anything.
    #[test]
    fn reads_every_setting() {
        let tweaks = get_windows_tweaks();
        for name in NAMES {
            assert!(tweaks.values.contains_key(name), "{name} is missing");
        }
        assert_eq!(tweaks.values.len(), NAMES.len());
        assert!(!tweaks.sound_schemes.is_empty());
    }

    #[test]
    fn refuses_unknown_names_and_wrong_values() {
        assert!(set_windows_tweak("nope".into(), Value::Bool(true)).is_err());
        assert!(set_windows_tweak("mouseSpeed".into(), Value::from(99)).is_err());
        assert!(set_windows_tweak("animations".into(), Value::from("yes")).is_err());
        assert!(set_windows_tweak("taskbarAlignment".into(), Value::from("top")).is_err());
    }
}
