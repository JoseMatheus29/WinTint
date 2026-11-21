//! Windows settings changed on the spot, like the switches in the Settings app:
//! transparency effects, desktop icons and the mouse pointer scheme. Windows
//! keeps them after WinTint closes, so nothing here needs restoring.

use serde::Serialize;

use crate::win;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const CURSORS: &str = r"Control Panel\Cursors";
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorScheme {
    id: String,
    /// The name Windows shows in its own language, when the scheme has one.
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tweaks {
    transparency: bool,
    /// None when the desktop window cannot be found.
    desktop_icons: Option<bool>,
    cursor_schemes: Vec<CursorScheme>,
    /// The scheme whose pointer matches the current one, if any.
    cursor_scheme: Option<String>,
}

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

fn same_file(a: &str, b: &str) -> bool {
    win::expand_env(a).eq_ignore_ascii_case(&win::expand_env(b))
}

#[tauri::command]
pub fn get_windows_tweaks() -> Tweaks {
    let schemes = schemes();
    let arrow = win::read_user_string(CURSORS, "Arrow").unwrap_or_default();
    let cursor_scheme = schemes
        .iter()
        .find(|scheme| scheme.files.first().is_some_and(|file| same_file(file, &arrow)))
        .map(|scheme| scheme.id.clone());
    let transparency = win::read_user_value(PERSONALIZE, "EnableTransparency")
        .and_then(|value| value.as_dword())
        .is_none_or(|value| value != 0);
    Tweaks {
        transparency,
        desktop_icons: win::desktop_icons_visible(),
        cursor_schemes: schemes
            .into_iter()
            .map(|scheme| CursorScheme {
                id: scheme.id,
                label: scheme.label,
            })
            .collect(),
        cursor_scheme,
    }
}

#[tauri::command(async)]
pub fn set_transparency(enabled: bool) -> Result<(), String> {
    win::set_user_dword(PERSONALIZE, "EnableTransparency", enabled as u32)?;
    win::broadcast_color_change();
    Ok(())
}

#[tauri::command]
pub fn set_desktop_icons(visible: bool) -> Result<(), String> {
    if win::desktop_icons_visible() == Some(visible) {
        return Ok(());
    }
    win::toggle_desktop_icons()
}

#[tauri::command(async)]
pub fn set_cursor_scheme(id: String) -> Result<(), String> {
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
