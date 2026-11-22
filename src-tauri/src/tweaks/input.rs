//! The mouse pointer, its speed and the keyboard repeat.

use serde_json::Value;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SPI_GETMOUSE, SPI_GETMOUSESPEED,
    SPI_GETWHEELSCROLLLINES, SPI_SETKEYBOARDDELAY, SPI_SETKEYBOARDSPEED, SPI_SETMOUSE,
    SPI_SETMOUSESPEED, SPI_SETWHEELSCROLLLINES, SYSTEM_PARAMETERS_INFO_ACTION,
};

use super::{Choice, Values, as_param, flag, number, spi_get, spi_set, text};
use crate::win;

const CURSORS: &str = r"Control Panel\Cursors";
const ACCESSIBILITY: &str = r"Software\Microsoft\Accessibility";
const SCHEMES: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Control Panel\Cursors\Schemes";
/// The pointer roles, in the order every scheme lists its files.
const CURSOR_ROLES: [&str; 17] = [
    "Arrow",
    "Help",
    "AppStarting",
    "Wait",
    "Crosshair",
    "IBeam",
    "NWPen",
    "No",
    "SizeNS",
    "SizeWE",
    "SizeNWSE",
    "SizeNESW",
    "SizeAll",
    "UpArrow",
    "Hand",
    "Pin",
    "Person",
];
/// Marks a scheme as one of the system's, not one saved by the user.
const SYSTEM_SCHEME: u32 = 2;
/// The pointer sizes of the Settings app go from 1 to 15, adding 16 pixels
/// to the normal 32 at each step.
const CURSOR_SIZES: u32 = 15;
const CURSOR_BASE: u32 = 32;
const CURSOR_STEP: u32 = 16;
/// Not in the documentation: what the Settings app calls to resize the
/// pointer at once. The new size in pixels goes in the pointer argument.
const SPI_SETCURSORSIZE: SYSTEM_PARAMETERS_INFO_ACTION = SYSTEM_PARAMETERS_INFO_ACTION(0x2029);
/// The thresholds Windows uses for "Enhance pointer precision".
const PRECISION_ON: [i32; 3] = [6, 10, 1];
const MAX_MOUSE_SPEED: u32 = 20;
const MAX_SCROLL_LINES: u32 = 100;
const MAX_KEYBOARD_DELAY: u32 = 3;
const MAX_KEYBOARD_SPEED: u32 = 31;

struct Scheme {
    id: String,
    label: String,
    files: Vec<String>,
}

/// After the 17 files, a scheme may carry the resource of its localized name.
fn schemes() -> Vec<Scheme> {
    win::machine_strings(SCHEMES)
        .into_iter()
        .map(|(id, value)| {
            let parts: Vec<&str> = value.split(',').collect();
            let label = parts
                .get(CURSOR_ROLES.len()..CURSOR_ROLES.len() + 2)
                .map(|resource| resource.join(","))
                .and_then(|resource| win::load_indirect_string(&resource))
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| id.clone());
            let files = parts.iter().take(CURSOR_ROLES.len()).map(|file| file.to_string()).collect();
            Scheme { id, label, files }
        })
        .collect()
}

pub fn cursor_schemes() -> Vec<Choice> {
    schemes()
        .into_iter()
        .map(|scheme| Choice {
            id: scheme.id,
            label: scheme.label,
        })
        .collect()
}

fn same_file(a: &str, b: &str) -> bool {
    win::expand_env(a).eq_ignore_ascii_case(&win::expand_env(b))
}

/// The scheme whose pointer matches the current one, if any.
fn current_scheme() -> Option<String> {
    let arrow = win::read_user_string(CURSORS, "Arrow").unwrap_or_default();
    schemes()
        .into_iter()
        .find(|scheme| scheme.files.first().is_some_and(|file| same_file(file, &arrow)))
        .map(|scheme| scheme.id)
}

fn cursor_size() -> u32 {
    let base = win::read_user_value(CURSORS, "CursorBaseSize")
        .and_then(|value| value.as_dword())
        .unwrap_or(CURSOR_BASE);
    (base.saturating_sub(CURSOR_BASE) / CURSOR_STEP + 1).clamp(1, CURSOR_SIZES)
}

pub fn read(values: &mut Values) {
    let precision = spi_get::<[i32; 3]>(SPI_GETMOUSE).map(|mouse| mouse[2] != 0);
    values.insert("cursorScheme".into(), current_scheme().into());
    values.insert("cursorSize".into(), cursor_size().into());
    values.insert("mouseSpeed".into(), spi_get::<u32>(SPI_GETMOUSESPEED).into());
    values.insert("mousePrecision".into(), precision.into());
    values.insert("scrollLines".into(), spi_get::<u32>(SPI_GETWHEELSCROLLLINES).into());
    values.insert("keyboardDelay".into(), spi_get::<u32>(SPI_GETKEYBOARDDELAY).into());
    values.insert("keyboardSpeed".into(), spi_get::<u32>(SPI_GETKEYBOARDSPEED).into());
}

pub fn set(name: &str, value: &Value) -> Option<Result<(), String>> {
    let result = match name {
        "cursorScheme" => text(value).and_then(set_cursor_scheme),
        "cursorSize" => number(value, 1, CURSOR_SIZES).and_then(set_cursor_size),
        "mouseSpeed" => number(value, 1, MAX_MOUSE_SPEED)
            .and_then(|speed| spi_set(SPI_SETMOUSESPEED, 0, as_param(speed))),
        "mousePrecision" => flag(value).and_then(set_precision),
        "scrollLines" => number(value, 1, MAX_SCROLL_LINES)
            .and_then(|lines| spi_set(SPI_SETWHEELSCROLLLINES, lines, std::ptr::null_mut())),
        "keyboardDelay" => number(value, 0, MAX_KEYBOARD_DELAY)
            .and_then(|delay| spi_set(SPI_SETKEYBOARDDELAY, delay, std::ptr::null_mut())),
        "keyboardSpeed" => number(value, 0, MAX_KEYBOARD_SPEED)
            .and_then(|speed| spi_set(SPI_SETKEYBOARDSPEED, speed, std::ptr::null_mut())),
        _ => return None,
    };
    Some(result)
}

fn set_cursor_scheme(id: &str) -> Result<(), String> {
    let scheme = schemes()
        .into_iter()
        .find(|scheme| scheme.id == id)
        .ok_or_else(|| format!("O esquema de cursor {id} não existe neste Windows."))?;
    for (role, file) in CURSOR_ROLES.iter().zip(scheme.files.iter().chain(std::iter::repeat(&String::new()))) {
        win::set_user_expand_string(CURSORS, role, file)?;
    }
    // The name Windows shows for the scheme in the mouse settings.
    win::set_user_expand_string(CURSORS, "", &scheme.label)?;
    win::set_user_dword(CURSORS, "Scheme Source", SYSTEM_SCHEME)?;
    win::reload_cursors()
}

fn set_cursor_size(size: u32) -> Result<(), String> {
    let pixels = CURSOR_BASE + (size - 1) * CURSOR_STEP;
    // Where the Settings app shows the size it has.
    win::set_user_dword(ACCESSIBILITY, "CursorSize", size)?;
    if spi_set(SPI_SETCURSORSIZE, 0, as_param(pixels)).is_ok() {
        return Ok(());
    }
    // Builds without that call still load the pointers at the saved size.
    win::set_user_dword(CURSORS, "CursorBaseSize", pixels)?;
    win::reload_cursors()
}

fn set_precision(on: bool) -> Result<(), String> {
    let mut mouse = if on { PRECISION_ON } else { [0; 3] };
    spi_set(SPI_SETMOUSE, 0, mouse.as_mut_ptr() as *mut _)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_variables_like_windows() {
        let root = std::env::var("SystemRoot").unwrap();
        assert_eq!(win::expand_env(r"%SystemRoot%\cursors\a.cur"), format!(r"{root}\cursors\a.cur"));
        assert_eq!(win::expand_env(r"%systemroot%\x"), format!(r"{root}\x"));
        assert_eq!(win::expand_env("no vars"), "no vars");
        assert_eq!(win::expand_env("%NOPE_NOT_SET%x"), "%NOPE_NOT_SET%x");
        assert_eq!(win::expand_env("50%"), "50%");
    }

    /// Reads the real schemes without changing anything.
    #[test]
    fn reads_the_installed_schemes() {
        let schemes = schemes();
        assert!(!schemes.is_empty());
        for scheme in &schemes {
            assert!(scheme.files.len() <= CURSOR_ROLES.len(), "{}", scheme.id);
            assert!(!scheme.label.is_empty());
        }
    }
}
