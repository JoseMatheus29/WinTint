//! Screens as Windows sees them, and the gamma ramps it sends to them.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};
use windows::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::{BOOL, HSTRING, PCWSTR};

/// Red, green and blue curves, 256 steps each, as GDI expects them.
pub type Ramp = [[u16; 256]; 3];

pub struct Screen {
    /// The name GDI knows the screen by, like `\\.\DISPLAY1`.
    pub device: String,
    pub name: String,
    pub primary: bool,
}

fn wide(text: &[u16]) -> String {
    let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..len])
}

/// Every connected screen, the primary one first and the rest from left to right.
pub fn list() -> Vec<Screen> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let found = unsafe { &mut *(data.0 as *mut Vec<(Screen, RECT)>) };
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
            let screen = Screen {
                device: wide(&info.szDevice),
                name: String::new(),
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            };
            found.push((screen, info.monitorInfo.rcMonitor));
        }
        true.into()
    }

    let mut found: Vec<(Screen, RECT)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut found as *mut _ as isize));
    }
    found.sort_by_key(|(screen, rect)| (!screen.primary, rect.left, rect.top));
    let names = friendly_names();
    found
        .into_iter()
        .enumerate()
        .map(|(index, (mut screen, _))| {
            screen.name = names
                .get(&screen.device)
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Monitor {}", index + 1));
            screen
        })
        .collect()
}

/// The monitor names Windows shows in its own settings, by GDI device name.
fn friendly_names() -> HashMap<String, String> {
    let mut names = HashMap::new();
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    unsafe {
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            != ERROR_SUCCESS
        {
            return names;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let queried = QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        );
        if queried != ERROR_SUCCESS {
            return names;
        }
        for path in &paths[..path_count as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) == 0
                && DisplayConfigGetDeviceInfo(&mut target.header) == 0
            {
                names.insert(
                    wide(&source.viewGdiDeviceName),
                    wide(&target.monitorFriendlyDeviceName),
                );
            }
        }
    }
    names
}

/// The curve a screen had before WinTint, darkened to `scale` without
/// changing its shape, so a calibration or a driver setting survives.
pub fn scaled(original: &Ramp, scale: f64) -> Ramp {
    original.map(|channel| channel.map(|value| (value as f64 * scale).round() as u16))
}

fn with_dc<T>(device: &str, action: impl FnOnce(HDC) -> T) -> Option<T> {
    let name = HSTRING::from(device);
    let dc = unsafe { CreateDCW(&name, PCWSTR::null(), PCWSTR::null(), None) };
    if dc.is_invalid() {
        return None;
    }
    let result = action(dc);
    unsafe {
        let _ = DeleteDC(dc);
    }
    Some(result)
}

/// False when Windows refuses the ramp, which it does for curves that dim too much.
pub fn set_ramp(device: &str, ramp: &Ramp) -> bool {
    with_dc(device, |dc| unsafe { SetDeviceGammaRamp(dc, ramp.as_ptr() as *const c_void) }.as_bool())
        .unwrap_or(false)
}

pub fn get_ramp(device: &str) -> Option<Ramp> {
    let mut ramp = [[0u16; 256]; 3];
    let read = with_dc(device, |dc| unsafe {
        GetDeviceGammaRamp(dc, ramp.as_mut_ptr() as *mut c_void)
    })?;
    read.as_bool().then_some(ramp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_curve_shape() {
        let mut original = [[0u16; 256]; 3];
        for step in 0..256 {
            for channel in &mut original {
                channel[step] = ((step as f64 / 255.0).powf(0.8) * 65535.0) as u16;
            }
        }
        let half = scaled(&original, 0.5);
        assert_eq!(scaled(&original, 1.0), original);
        assert_eq!(half[1][255], 32768);
        assert_eq!(half[2][128], (original[2][128] as f64 * 0.5).round() as u16);
    }
}
