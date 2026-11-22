//! Thin wrappers over the Win32 calls the modules need.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::sync::Mutex;

use base64::Engine;
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_CLOAKED, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE, DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetMonitorInfoW, GetObjectW, HBITMAP, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_BINARY, REG_DWORD, REG_EXPAND_SZ,
    REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY, RegCloseKey, RegDeleteKeyValueW, RegDeleteTreeW,
    RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    SendInput, VIRTUAL_KEY, VK_A, VK_LWIN, VK_N,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, QUNS_RUNNING_D3D_FULL_SCREEN, SHCreateItemFromParsingName,
    SHLoadIndirectString, SHQueryUserNotificationState, SIIGBF, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, FindWindowExW, FindWindowW, GW_OWNER, GWL_EXSTYLE,
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, HWND_BROADCAST, IsIconic, IsWindow, IsWindowVisible,
    IsZoomed, MONITORINFOF_PRIMARY, PostMessageW, SMTO_ABORTIFHUNG, SPI_GETWORKAREA, SPI_SETCURSORS,
    SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SW_HIDE, SendMessageTimeoutW, WM_COMMAND, WM_SETTINGCHANGE,
    SW_MINIMIZE, SW_RESTORE, SW_SHOWNA, SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetForegroundWindow, SetWindowLongPtrW, ShowWindow, ShowWindowAsync, SystemParametersInfoW,
    WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use windows::core::{BOOL, HRESULT, HSTRING, PCSTR, PCWSTR, PWSTR, w};

const ICON_SIZE: i32 = 128;
const MAX_IMAGE_BYTES: u64 = 3 * 1024 * 1024;
const SHELL_CLASSES: [&str; 3] = ["Progman", "WorkerW", "Shell_TrayWnd"];
/// Shell surfaces that can span the screen, such as the desktop and Alt+Tab.
const NOT_FULLSCREEN_APPS: [&str; 6] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "XamlExplorerHostIslandWindow",
    "MultitaskingViewFrame",
    "Windows.UI.Core.CoreWindow",
];
const AUTOSTART_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const AUTOSTART_VALUE: PCWSTR = w!("WinTint");
/// The entry written while the app was called Dock JM.
const LEGACY_AUTOSTART_VALUE: PCWSTR = w!("Dock JM");

static ICON_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, PartialEq, Serialize)]
pub struct AppWindow {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
    pub active: bool,
}

fn to_hwnd(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

/// Usable area of the primary monitor (the screen minus a visible taskbar).
pub fn work_area() -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    Some(rect)
}

/// Keeps the dock out of Alt+Tab and toggles whether clicks pass through it.
/// Tauri's own toggle rewrites the whole extended style, which would undo the
/// tool window flag, so the style is managed here instead.
/// Turns "#rrggbb" into the 0x00BBGGRR layout Win32 expects.
fn colorref(color: &str) -> Option<u32> {
    let rgb = u32::from_str_radix(color.strip_prefix('#')?, 16).ok()?;
    Some((rgb & 0xff) << 16 | (rgb & 0xff00) | (rgb >> 16) & 0xff)
}

/// Windows 10 ignores the caption colors and only applies the dark mode.
pub fn set_title_bar_colors(hwnd: isize, caption: &str, text: &str, dark: bool) {
    let window = to_hwnd(hwnd);
    let set = |attribute: DWMWINDOWATTRIBUTE, value: u32| unsafe {
        let _ = DwmSetWindowAttribute(
            window,
            attribute,
            &value as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        );
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE, dark as u32);
    if let Some(color) = colorref(caption) {
        set(DWMWA_CAPTION_COLOR, color);
        set(DWMWA_BORDER_COLOR, color);
    }
    if let Some(color) = colorref(text) {
        set(DWMWA_TEXT_COLOR, color);
    }
}

pub fn set_dock_style(hwnd: isize, click_through: bool) {
    let hwnd = to_hwnd(hwnd);
    let pass_through = WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0;
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let base = (current & !WS_EX_APPWINDOW.0) | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0;
        let style = if click_through {
            base | pass_through
        } else {
            base & !pass_through
        };
        if style != current {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
        }
    }
}

/// Adds or removes the program from the apps Windows opens at sign-in.
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let result = unsafe {
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command: Vec<u16> = format!("\"{}\"", exe.display())
                .encode_utf16()
                .chain(Some(0))
                .collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                AUTOSTART_KEY,
                AUTOSTART_VALUE,
                REG_SZ.0,
                Some(command.as_ptr() as *const c_void),
                (command.len() * size_of::<u16>()) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, AUTOSTART_KEY, AUTOSTART_VALUE)
        }
    };
    // Removing an entry that is already gone is not a failure.
    if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(format!("Não foi possível alterar o início automático ({})", result.0))
    }
}

/// Deletes the entry left by Dock JM, so the app never starts twice.
pub fn remove_legacy_autostart() {
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, AUTOSTART_KEY, LEGACY_AUTOSTART_VALUE);
    }
}

/// True when the foreground app takes over the whole primary monitor, where
/// the dock lives. A maximized window also covers it when the taskbar is
/// hidden, but that is a normal window the dock should stay over.
pub fn is_fullscreen_app_active(own_pid: u32) -> bool {
    unsafe {
        if SHQueryUserNotificationState().is_ok_and(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN) {
            return true;
        }

        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid()
            || window_pid(hwnd) == own_pid
            || IsZoomed(hwnd).as_bool()
            || NOT_FULLSCREEN_APPS.contains(&window_class(hwnd).as_str())
        {
            return false;
        }

        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut rect = RECT::default();
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool()
            || info.dwFlags & MONITORINFOF_PRIMARY == 0
            || GetWindowRect(hwnd, &mut rect).is_err()
        {
            return false;
        }
        let screen = info.rcMonitor;
        rect.left <= screen.left
            && rect.top <= screen.top
            && rect.right >= screen.right
            && rect.bottom >= screen.bottom
    }
}

/// Foreground window, or `None` when it belongs to the dock itself.
pub fn foreground_window(own_pid: u32) -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid() && window_pid(hwnd) != own_pid).then_some(hwnd.0 as isize)
}

/// Top-level windows a user would switch to, topmost first.
pub fn list_windows(own_pid: u32, active: Option<isize>) -> Vec<AppWindow> {
    let mut handles: Vec<HWND> = Vec::new();
    let _ = unsafe {
        EnumWindows(
            Some(collect_hwnd),
            LPARAM(&mut handles as *mut Vec<HWND> as isize),
        )
    };

    handles
        .into_iter()
        .filter(|&hwnd| window_pid(hwnd) != own_pid && is_app_window(hwnd))
        .filter_map(|hwnd| {
            Some(AppWindow {
                hwnd: hwnd.0 as isize,
                title: window_title(hwnd),
                exe: window_exe(hwnd)?,
                active: active == Some(hwnd.0 as isize),
            })
        })
        .collect()
}

unsafe extern "system" fn collect_hwnd(hwnd: HWND, list: LPARAM) -> BOOL {
    unsafe { (*(list.0 as *mut Vec<HWND>)).push(hwnd) };
    true.into()
}

fn is_app_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok() {
            return false;
        }

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let hidden = ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0;
        if hidden && ex_style & WS_EX_APPWINDOW.0 == 0 {
            return false;
        }

        // Suspended store apps stay "visible" but are cloaked by the compositor.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return false;
        }

        !SHELL_CLASSES.contains(&window_class(hwnd).as_str()) && !window_title(hwnd).is_empty()
    }
}

fn window_class(hwnd: HWND) -> String {
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    String::from_utf16_lossy(&class[..len])
}

fn window_title(hwnd: HWND) -> String {
    let mut title = [0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd, &mut title) } as usize;
    String::from_utf16_lossy(&title[..len])
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

fn window_exe(hwnd: HWND) -> Option<String> {
    let pid = window_pid(hwnd);
    let exe = process_path(pid)?;
    if !exe.to_lowercase().ends_with("applicationframehost.exe") {
        return Some(exe);
    }

    // Store apps are hosted by a frame process; the real app is a child window.
    let mut host = (pid, None::<u32>);
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(find_hosted_pid),
            LPARAM(&mut host as *mut (u32, Option<u32>) as isize),
        );
    }
    host.1.and_then(process_path).or(Some(exe))
}

unsafe extern "system" fn find_hosted_pid(child: HWND, host: LPARAM) -> BOOL {
    let host = unsafe { &mut *(host.0 as *mut (u32, Option<u32>)) };
    let pid = window_pid(child);
    if pid != host.0 {
        host.1 = Some(pid);
        return false.into();
    }
    true.into()
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

pub fn focus_window(hwnd: isize) -> Result<(), String> {
    let hwnd = to_hwnd(hwnd);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err("A janela não existe mais".into());
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        }
        if !SetForegroundWindow(hwnd).as_bool() {
            // Windows refuses focus changes from background processes. Empty
            // input makes this process the last one to have received input.
            let input = INPUT {
                r#type: INPUT_MOUSE,
                ..Default::default()
            };
            SendInput(&[input], size_of::<INPUT>() as i32);
            if !SetForegroundWindow(hwnd).as_bool() {
                return Err("O Windows não permitiu trocar de janela".into());
            }
        }
    }
    Ok(())
}

/// Shows or hides the Windows taskbar on every monitor. It only touches the
/// windows, so nothing outlives Explorer restarting.
pub fn set_taskbar_visible(visible: bool) {
    let command = if visible { SW_SHOWNA } else { SW_HIDE };
    let apply = |hwnd: HWND| unsafe {
        if IsWindowVisible(hwnd).as_bool() != visible {
            let _ = ShowWindow(hwnd, command);
        }
    };

    unsafe {
        if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            apply(primary);
        }
        let mut previous = None;
        while let Ok(secondary) =
            FindWindowExW(None, previous, w!("Shell_SecondaryTrayWnd"), PCWSTR::null())
        {
            apply(secondary);
            previous = Some(secondary);
        }
    }
}

/// Presses the keys in order and releases them in reverse, like a shortcut.
fn press_keys(keys: &[VIRTUAL_KEY]) {
    let event = |key, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let down = keys.iter().map(|&key| event(key, KEYBD_EVENT_FLAGS(0)));
    let up = keys.iter().rev().map(|&key| event(key, KEYEVENTF_KEYUP));
    let inputs: Vec<INPUT> = down.chain(up).collect();
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

/// The Windows key opens the Start menu, or closes it if open.
pub fn open_start_menu() {
    press_keys(&[VK_LWIN]);
}

/// Win+A: the Wi-Fi, volume and battery panel.
pub fn open_quick_settings() {
    press_keys(&[VK_LWIN, VK_A]);
}

/// Win+N: notifications and the calendar.
pub fn open_calendar() {
    press_keys(&[VK_LWIN, VK_N]);
}

pub fn minimize_window(hwnd: isize) {
    unsafe {
        let _ = ShowWindowAsync(to_hwnd(hwnd), SW_MINIMIZE);
    }
}

pub fn launch(path: &str) -> Result<(), String> {
    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success with any value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("Não foi possível abrir {path}"))
    }
}

/// Reads a picture chosen by the user as a data URL. The size is capped because
/// it travels inside the config on every change.
pub fn read_image(path: &str) -> Result<String, String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_lowercase);
    let mime = match extension.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => return Err("Escolha uma imagem PNG, JPG, WEBP ou GIF".into()),
    };
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_IMAGE_BYTES {
        return Err("A imagem precisa ter no máximo 3 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// Icon of a file as a PNG data URL.
pub fn file_icon(path: &str) -> Option<String> {
    let size = SIZE {
        cx: ICON_SIZE,
        cy: ICON_SIZE,
    };
    shell_image(path, size, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK)
}

/// A small copy of a picture, as Explorer shows it, as a PNG data URL.
pub fn file_thumbnail(path: &str, width: i32, height: i32) -> Option<String> {
    let size = SIZE {
        cx: width,
        cy: height,
    };
    shell_image(path, size, SIIGBF_RESIZETOFIT)
}

fn shell_image(path: &str, size: SIZE, flags: SIIGBF) -> Option<String> {
    if !Path::new(path).exists() {
        return None;
    }

    // Parallel calls fail while the shell is still loading at startup.
    let _guard = ICON_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let image = render_shell_image(path, size, flags);
        if com.is_ok() {
            CoUninitialize();
        }
        image
    }
}

unsafe fn render_shell_image(path: &str, size: SIZE, flags: SIIGBF) -> Option<String> {
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let bitmap = factory.GetImage(size, flags).ok()?;
        let pixels = read_bitmap(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let (width, height, pixels) = pixels?;
        encode_png(width, height, &pixels)
    }
}

/// Copies a bitmap out as straight-alpha RGBA, top row first.
unsafe fn read_bitmap(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = BITMAP::default();
        let read = GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut info as *mut BITMAP as *mut c_void),
        );
        if read == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
            return None;
        }
        let (width, height) = (info.bmWidth, info.bmHeight);

        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut c_void),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }

        // Icons without an alpha channel come back fully transparent.
        let opaque = pixels.chunks_exact(4).all(|pixel| pixel[3] == 0);
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = if opaque { 255 } else { pixel[3] as u32 };
            let unpremultiply = |value: u8| match alpha {
                0 | 255 => value,
                _ => (value as u32 * 255 / alpha).min(255) as u8,
            };
            let (blue, green, red) = (pixel[0], pixel[1], pixel[2]);
            pixel[0] = unpremultiply(red);
            pixel[1] = unpremultiply(green);
            pixel[2] = unpremultiply(blue);
            pixel[3] = alpha as u8;
        }
        Some((width as u32, height as u32, pixels))
    }
}

fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Option<String> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(pixels).ok()?;
    writer.finish().ok()?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:image/png;base64,{encoded}"))
}

/// A raw registry value, kept so it can be written back exactly as it was.
#[derive(Clone, Serialize, Deserialize)]
pub struct RegValue {
    pub kind: u32,
    pub data: Vec<u8>,
}

impl RegValue {
    pub fn as_dword(&self) -> Option<u32> {
        let bytes: [u8; 4] = self.data.as_slice().try_into().ok()?;
        (self.kind == REG_DWORD.0).then(|| u32::from_le_bytes(bytes))
    }
}

pub fn read_user_value(key: &str, name: &str) -> Option<RegValue> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    unsafe {
        let probe = RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_ANY,
            Some(&mut kind as *mut _),
            None,
            Some(&mut size as *mut _),
        );
        if probe != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        let read = RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_ANY,
            Some(&mut kind as *mut _),
            Some(data.as_mut_ptr() as *mut c_void),
            Some(&mut size as *mut _),
        );
        if read != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
        Some(RegValue { kind: kind.0, data })
    }
}

/// Creates the key when it is missing.
pub fn write_user_value(key: &str, name: &str, value: &RegValue) -> Result<(), String> {
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            value.kind,
            Some(value.data.as_ptr() as *const c_void),
            value.data.len() as u32,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(r"Não foi possível gravar {key}\{name} ({})", result.0))
    }
}

pub fn set_user_dword(key: &str, name: &str, value: u32) -> Result<(), String> {
    let data = value.to_le_bytes().to_vec();
    write_user_value(key, name, &RegValue { kind: REG_DWORD.0, data })
}

pub fn set_user_binary(key: &str, name: &str, data: &[u8]) -> Result<(), String> {
    let data = data.to_vec();
    write_user_value(key, name, &RegValue { kind: REG_BINARY.0, data })
}

pub fn delete_user_value(key: &str, name: &str) {
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name));
    }
}

/// What dwmapi.dll keeps for the accent shown in title bars.
#[repr(C)]
#[derive(Default)]
struct ColorizationParams {
    color: u32,
    afterglow: u32,
    color_balance: u32,
    afterglow_balance: u32,
    blur_balance: u32,
    glass_reflection_intensity: u32,
    opaque_blend: u32,
}

/// Repaints title bars in the new accent at once. dwmapi.dll exports these two
/// functions by number only: 127 reads the colors and 131 sets them.
pub fn set_dwm_colorization(argb: u32) {
    type GetParams = unsafe extern "system" fn(*mut ColorizationParams) -> HRESULT;
    type SetParams = unsafe extern "system" fn(*const ColorizationParams, u32) -> HRESULT;
    unsafe {
        // Already loaded, since the dock calls into it.
        let Ok(module) = GetModuleHandleW(w!("dwmapi.dll")) else {
            return;
        };
        let get = GetProcAddress(module, PCSTR(127 as *const u8));
        let set = GetProcAddress(module, PCSTR(131 as *const u8));
        let (Some(get), Some(set)) = (get, set) else {
            return;
        };
        let get: GetParams = std::mem::transmute(get);
        let set: SetParams = std::mem::transmute(set);
        let mut params = ColorizationParams::default();
        if get(&mut params).is_err() {
            return;
        }
        params.color = argb;
        params.afterglow = argb;
        let _ = set(&params, 0);
    }
}

/// Tells every window that the colors changed, as the Settings app does.
pub fn broadcast_color_change() {
    broadcast_setting_change("ImmersiveColorSet");
}

/// Tells every window that a group of settings changed, so they read it again.
pub fn broadcast_setting_change(area: &str) {
    let area = HSTRING::from(area);
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(area.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
}

/// Removes a key that only tests create.
#[cfg(test)]
pub fn delete_test_key(key: &str) {
    use windows::Win32::System::Registry::RegDeleteKeyW;
    unsafe {
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(key));
    }
}

fn utf16_string(data: &[u8]) -> String {
    let wide: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}

pub fn read_user_string(key: &str, name: &str) -> Option<String> {
    let value = read_user_value(key, name)?;
    (value.kind == REG_SZ.0 || value.kind == REG_EXPAND_SZ.0).then(|| utf16_string(&value.data))
}

/// Writes a string, leaving %VARIABLES% for Windows to expand when it reads it.
pub fn set_user_expand_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    let data: Vec<u8> = value
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    write_user_value(key, name, &RegValue { kind: REG_EXPAND_SZ.0, data })
}

pub fn set_user_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    let data: Vec<u8> = value
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    write_user_value(key, name, &RegValue { kind: REG_SZ.0, data })
}

pub fn user_key_exists(key: &str) -> bool {
    let mut handle = HKEY::default();
    unsafe {
        let opened = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut handle);
        if opened == ERROR_SUCCESS {
            let _ = RegCloseKey(handle);
        }
        opened == ERROR_SUCCESS
    }
}

/// Deletes a key of HKEY_CURRENT_USER with everything under it.
pub fn delete_user_tree(key: &str) -> Result<(), String> {
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
    // A key that is already gone is what was asked for.
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!(r"Não foi possível apagar {key} ({})", result.0))
    }
}

/// The names of the keys right under a key of HKEY_CURRENT_USER.
pub fn user_subkeys(key: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut handle = HKEY::default();
    unsafe {
        let opened = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut handle);
        if opened != ERROR_SUCCESS {
            return names;
        }
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            let read = RegEnumKeyExW(
                handle,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            );
            if read != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&name[..name_len as usize]));
        }
        let _ = RegCloseKey(handle);
    }
    names
}

/// The string values under a key of HKEY_LOCAL_MACHINE, as stored.
pub fn machine_strings(key: &str) -> Vec<(String, String)> {
    let mut values = Vec::new();
    let mut handle = HKEY::default();
    unsafe {
        let opened = RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), None, KEY_READ, &mut handle);
        if opened != ERROR_SUCCESS {
            return values;
        }
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            let mut kind = 0u32;
            let mut data = vec![0u8; 8192];
            let mut data_len = data.len() as u32;
            let read = RegEnumValueW(
                handle,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            );
            if read != ERROR_SUCCESS {
                break;
            }
            if kind == REG_SZ.0 || kind == REG_EXPAND_SZ.0 {
                values.push((
                    String::from_utf16_lossy(&name[..name_len as usize]),
                    utf16_string(&data[..data_len as usize]),
                ));
            }
        }
        let _ = RegCloseKey(handle);
    }
    values
}

/// Replaces %NAME% with that environment variable, as Windows does when it
/// reads an expandable registry string.
pub fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match std::env::var(name) {
            Ok(value) => out.push_str(&value),
            Err(_) => out.push_str(&rest[start..start + end + 2]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Reads a localized string reference such as `@main.cpl,-1020`.
pub fn load_indirect_string(source: &str) -> Option<String> {
    let mut buffer = [0u16; 256];
    unsafe { SHLoadIndirectString(&HSTRING::from(source), &mut buffer, None) }.ok()?;
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

/// Makes Windows load the cursors written under `Control Panel\Cursors`.
pub fn reload_cursors() -> Result<(), String> {
    unsafe {
        SystemParametersInfoW(SPI_SETCURSORS, 0, None, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)
    }
    .map_err(|e| e.to_string())
}

/// The command Explorer runs for View > Show desktop icons.
const TOGGLE_DESKTOP_ICONS: usize = 0x7402;

/// The window that holds the desktop icons. Windows 11 keeps it in Progman,
/// older versions in one of the WorkerW windows.
fn desktop_view() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        if let Ok(view) = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
            return Some(view);
        }
        let mut worker = None;
        loop {
            let next = FindWindowExW(None, worker, w!("WorkerW"), PCWSTR::null()).ok()?;
            if let Ok(view) = FindWindowExW(Some(next), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
                return Some(view);
            }
            worker = Some(next);
        }
    }
}

pub fn desktop_icons_visible() -> Option<bool> {
    let view = desktop_view()?;
    let list = unsafe { FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null()) }.ok()?;
    Some(unsafe { IsWindowVisible(list) }.as_bool())
}

/// Explorer remembers the choice itself, as when done from its own menu.
pub fn toggle_desktop_icons() -> Result<(), String> {
    let view = desktop_view().ok_or("Não foi possível achar a área de trabalho.")?;
    unsafe { PostMessageW(Some(view), WM_COMMAND, WPARAM(TOGGLE_DESKTOP_ICONS), LPARAM(0)) }
        .map_err(|e| e.to_string())
}

