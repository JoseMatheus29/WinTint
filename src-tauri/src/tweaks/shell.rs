//! Explorer's own options: the taskbar, Start and the file windows.

use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, FILETIME};
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_UNIQUE_PROCESS, RmEndSession, RmForceShutdown, RmRegisterResources,
    RmRestart, RmShutdown, RmStartSession,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{
    ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage, SHELLSTATEA,
    SHGetSetSettings, SSF_MASK, SSF_SHOWALLOBJECTS, SSF_SHOWEXTENSIONS,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
use windows::core::{PCWSTR, PWSTR, w};

use super::{Values, flag, text};
use crate::win;

const ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
const SEARCH: &str = r"Software\Microsoft\Windows\CurrentVersion\Search";
/// Present when Windows 11 opens the old right-click menu straight away.
const CLASSIC_MENU: &str = r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}";
const CLASSIC_MENU_SERVER: &str =
    r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32";
const EXPLORER_RESTART_TIMEOUT: Duration = Duration::from_secs(10);
const EXPLORER_POLL: Duration = Duration::from_millis(200);

/// An option Explorer keeps as a number in the registry.
struct RegistryOption {
    name: &'static str,
    key: &'static str,
    value: &'static str,
    /// What Windows uses while the value does not exist.
    default: u32,
    /// Names for the numbers. Without them, the option is a switch.
    choices: &'static [(&'static str, u32)],
}

const OPTIONS: [RegistryOption; 8] = [
    RegistryOption {
        name: "taskbarAlignment",
        key: ADVANCED,
        value: "TaskbarAl",
        default: 1,
        choices: &[("left", 0), ("center", 1)],
    },
    RegistryOption {
        name: "taskbarSearch",
        key: SEARCH,
        value: "SearchboxTaskbarMode",
        default: 2,
        choices: &[("hidden", 0), ("icon", 1), ("label", 3), ("box", 2)],
    },
    RegistryOption {
        name: "taskbarCombine",
        key: ADVANCED,
        value: "TaskbarGlomLevel",
        default: 0,
        choices: &[("always", 0), ("full", 1), ("never", 2)],
    },
    RegistryOption {
        name: "taskbarTaskView",
        key: ADVANCED,
        value: "ShowTaskViewButton",
        default: 1,
        choices: &[],
    },
    RegistryOption {
        name: "taskbarSeconds",
        key: ADVANCED,
        value: "ShowSecondsInSystemClock",
        default: 0,
        choices: &[],
    },
    RegistryOption {
        name: "startMostUsed",
        key: ADVANCED,
        value: "Start_TrackProgs",
        default: 1,
        choices: &[],
    },
    RegistryOption {
        name: "startRecentFiles",
        key: ADVANCED,
        value: "Start_TrackDocs",
        default: 1,
        choices: &[],
    },
    RegistryOption {
        name: "explorerOpensTo",
        key: ADVANCED,
        value: "LaunchTo",
        default: 2,
        choices: &[("thisPc", 1), ("home", 2)],
    },
];

pub fn read(values: &mut Values) {
    for option in &OPTIONS {
        let number = win::read_user_value(option.key, option.value)
            .and_then(|value| value.as_dword())
            .unwrap_or(option.default);
        let value = if option.choices.is_empty() {
            Value::from(number != 0)
        } else {
            option
                .choices
                .iter()
                .find(|(_, choice)| *choice == number)
                .map_or(Value::Null, |(name, _)| Value::from(*name))
        };
        values.insert(option.name.into(), value);
    }
    let state = shell_state(SSF_SHOWALLOBJECTS | SSF_SHOWEXTENSIONS);
    values.insert("hiddenFiles".into(), (state & SHOW_ALL_OBJECTS != 0).into());
    values.insert("fileExtensions".into(), (state & SHOW_EXTENSIONS != 0).into());
    values.insert("taskbarAutoHide".into(), taskbar_auto_hides().into());
    values.insert("classicContextMenu".into(), win::user_key_exists(CLASSIC_MENU_SERVER).into());
}

pub fn set(name: &str, value: &Value) -> Option<Result<(), String>> {
    let result = match name {
        "hiddenFiles" => flag(value).map(|on| set_shell_state(SSF_SHOWALLOBJECTS, SHOW_ALL_OBJECTS, on)),
        "fileExtensions" => flag(value).map(|on| set_shell_state(SSF_SHOWEXTENSIONS, SHOW_EXTENSIONS, on)),
        "taskbarAutoHide" => flag(value).and_then(set_taskbar_auto_hide),
        "classicContextMenu" => flag(value).and_then(set_classic_menu),
        _ => {
            let option = OPTIONS.iter().find(|option| option.name == name)?;
            set_option(option, value)
        }
    };
    Some(result)
}

fn set_option(option: &RegistryOption, value: &Value) -> Result<(), String> {
    let number = if option.choices.is_empty() {
        flag(value)? as u32
    } else {
        let name = text(value)?;
        option
            .choices
            .iter()
            .find(|(choice, _)| *choice == name)
            .map(|(_, number)| *number)
            .ok_or_else(|| format!("A opção {name} não existe."))?
    };
    win::set_user_dword(option.key, option.value, number).map_err(|error| {
        // Windows guards a few of these values from other programs.
        format!("O Windows não deixou mudar esta opção. {error}")
    })?;
    // The taskbar reads its options again on this message, as after Settings.
    win::broadcast_setting_change("TraySettings");
    Ok(())
}

/// The first bits of the shell state, in the order of the SHELLSTATE fields.
const SHOW_ALL_OBJECTS: i32 = 1 << 0;
const SHOW_EXTENSIONS: i32 = 1 << 1;

fn shell_state(mask: SSF_MASK) -> i32 {
    let mut state = SHELLSTATEA::default();
    unsafe { SHGetSetSettings(Some(&mut state), mask, false) };
    state._bitfield1
}

/// Explorer saves the change and refreshes its open windows by itself.
fn set_shell_state(mask: SSF_MASK, bit: i32, on: bool) {
    let mut state = SHELLSTATEA::default();
    unsafe { SHGetSetSettings(Some(&mut state), mask, false) };
    // A plain assignment, since the struct is packed.
    let bits = state._bitfield1;
    state._bitfield1 = if on { bits | bit } else { bits & !bit };
    unsafe { SHGetSetSettings(Some(&mut state), mask, true) };
}

fn appbar_data() -> APPBARDATA {
    APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        ..Default::default()
    }
}

fn taskbar_auto_hides() -> bool {
    let mut data = appbar_data();
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) };
    state as u32 & ABS_AUTOHIDE != 0
}

fn set_taskbar_auto_hide(on: bool) -> Result<(), String> {
    let mut data = appbar_data();
    data.hWnd = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
        .map_err(|_| "Não foi possível achar a barra de tarefas.".to_string())?;
    data.lParam.0 = if on { ABS_AUTOHIDE as isize } else { 0 };
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut data) };
    Ok(())
}

/// Explorer only reads this when it starts, so it needs a restart to show.
fn set_classic_menu(on: bool) -> Result<(), String> {
    if on {
        win::set_user_string(CLASSIC_MENU_SERVER, "", "")
    } else {
        win::delete_user_tree(CLASSIC_MENU)
    }
}

/// The process behind the taskbar and the desktop.
fn shell_process() -> Option<u32> {
    let tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

fn start_time(pid: u32) -> Option<FILETIME> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut created = FILETIME::default();
        let [mut exited, mut kernel, mut user] = [FILETIME::default(); 3];
        let read = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        read.ok().map(|_| created)
    }
}

/// Closes and reopens Explorer the way Task Manager does, through the Restart
/// Manager, which lets it save its state and start again on its own. Returns
/// once the taskbar is back.
pub fn restart_explorer() -> Result<(), String> {
    let pid = shell_process().ok_or("Não foi possível achar o Explorador.")?;
    let process = RM_UNIQUE_PROCESS {
        dwProcessId: pid,
        ProcessStartTime: start_time(pid).ok_or("Não foi possível ler o Explorador.")?,
    };
    let mut session = 0;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    unsafe {
        if RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
            return Err("O Windows não deixou reiniciar o Explorador.".into());
        }
        let registered = RmRegisterResources(session, None, Some(&[process]), None);
        let shut = registered == ERROR_SUCCESS
            && RmShutdown(session, RmForceShutdown.0 as u32, None) == ERROR_SUCCESS;
        let restarted = shut && RmRestart(session, None, None) == ERROR_SUCCESS;
        let _ = RmEndSession(session);
        if !shut {
            return Err("O Windows não deixou fechar o Explorador.".into());
        }
        if !restarted {
            // Without a shell running, a new Explorer becomes the shell.
            win::launch("explorer.exe")?;
        }
    }

    let started = Instant::now();
    while started.elapsed() < EXPLORER_RESTART_TIMEOUT {
        if shell_process().is_some_and(|new| new != pid) {
            return Ok(());
        }
        thread::sleep(EXPLORER_POLL);
    }
    Err("O Explorador demorou para voltar. Se a barra não aparecer, abra explorer.exe pelo Gerenciador de Tarefas.".into())
}
