//! The desktop: its wallpaper and which icons it shows.

use serde_json::Value;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::Win32::UI::Shell::{
    DESKTOP_WALLPAPER_POSITION, DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH,
    DWPOS_TILE, DesktopWallpaper, IDesktopWallpaper, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
    SHChangeNotify,
};
use windows::core::{HSTRING, PCWSTR};

use super::{Values, flag, text};
use crate::win;

const HIDDEN_ICONS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel";
/// The icons Windows offers in its desktop icon settings, with the value that
/// hides each one and whether it shows while Windows has no value for it.
const ICONS: [(&str, &str, bool); 5] = [
    ("iconComputer", "{20D04FE0-3AEA-1069-A2D8-08002B30309D}", false),
    ("iconUserFiles", "{59031a47-3f72-44a7-89c5-5595fe6b30ee}", false),
    ("iconNetwork", "{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}", false),
    ("iconRecycleBin", "{645FF040-5081-101B-9F08-00AA002F954E}", true),
    ("iconControlPanel", "{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}", false),
];
const FITS: [(&str, DESKTOP_WALLPAPER_POSITION); 6] = [
    ("fill", DWPOS_FILL),
    ("fit", DWPOS_FIT),
    ("stretch", DWPOS_STRETCH),
    ("tile", DWPOS_TILE),
    ("center", DWPOS_CENTER),
    ("span", DWPOS_SPAN),
];
const PREVIEW_WIDTH: i32 = 320;
const PREVIEW_HEIGHT: i32 = 180;

pub fn read(values: &mut Values) {
    values.insert("desktopIcons".into(), win::desktop_icons_visible().into());
    for (name, id, shown_by_default) in ICONS {
        let hidden = win::read_user_value(HIDDEN_ICONS, id).and_then(|value| value.as_dword());
        let shown = hidden.map_or(shown_by_default, |hidden| hidden == 0);
        values.insert(name.into(), shown.into());
    }
    let (wallpaper, fit) = with_wallpaper(|desktop| unsafe {
        let path = desktop.GetWallpaper(PCWSTR::null()).ok().map(|path| {
            let text = path.to_string().unwrap_or_default();
            CoTaskMemFree(Some(path.0 as *const _));
            text
        });
        let fit = desktop.GetPosition().ok();
        Ok((path, fit))
    })
    .unwrap_or((None, None));
    // An empty path means each monitor has its own picture, or there is none.
    let wallpaper = wallpaper.filter(|path| !path.is_empty());
    let fit = fit.and_then(|fit| FITS.iter().find(|(_, position)| *position == fit));
    values.insert("wallpaper".into(), wallpaper.into());
    values.insert("wallpaperFit".into(), fit.map(|(name, _)| *name).into());
}

pub fn set(name: &str, value: &Value) -> Option<Result<(), String>> {
    let result = match name {
        "desktopIcons" => flag(value).and_then(set_desktop_icons),
        "wallpaper" => text(value).and_then(set_wallpaper),
        "wallpaperFit" => text(value).and_then(set_fit),
        _ => {
            let (_, id, _) = ICONS.iter().find(|(icon, _, _)| *icon == name)?;
            flag(value).and_then(|shown| set_icon(id, shown))
        }
    };
    Some(result)
}

pub fn preview() -> Option<String> {
    let path = with_wallpaper(|desktop| unsafe {
        let path = desktop.GetWallpaper(PCWSTR::null())?;
        let text = path.to_string().unwrap_or_default();
        CoTaskMemFree(Some(path.0 as *const _));
        Ok(text)
    })
    .ok()?;
    win::file_thumbnail(&path, PREVIEW_WIDTH, PREVIEW_HEIGHT)
}

fn set_desktop_icons(visible: bool) -> Result<(), String> {
    if win::desktop_icons_visible() == Some(visible) {
        return Ok(());
    }
    win::toggle_desktop_icons()
}

fn set_icon(id: &str, shown: bool) -> Result<(), String> {
    win::set_user_dword(HIDDEN_ICONS, id, !shown as u32)?;
    // Makes the desktop read the setting again, as the Settings dialog does.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

fn set_wallpaper(path: &str) -> Result<(), String> {
    with_wallpaper(|desktop| unsafe { desktop.SetWallpaper(PCWSTR::null(), &HSTRING::from(path)) })
        .map_err(|error| format!("Não foi possível usar esta imagem: {error}"))
}

fn set_fit(name: &str) -> Result<(), String> {
    let (_, position) = FITS
        .iter()
        .find(|(fit, _)| *fit == name)
        .ok_or_else(|| format!("O ajuste de imagem {name} não existe."))?;
    with_wallpaper(|desktop| unsafe { desktop.SetPosition(*position) })
        .map_err(|error| format!("Não foi possível mudar o ajuste da imagem: {error}"))
}

/// Runs `action` with the desktop wallpaper object, the same one the Settings
/// app uses, which covers every monitor.
fn with_wallpaper<T>(
    action: impl FnOnce(&IDesktopWallpaper) -> windows::core::Result<T>,
) -> Result<T, String> {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = CoCreateInstance::<_, IDesktopWallpaper>(&DesktopWallpaper, None, CLSCTX_ALL)
            .and_then(|desktop| action(&desktop));
        if com.is_ok() {
            CoUninitialize();
        }
        result.map_err(|error| error.message())
    }
}
