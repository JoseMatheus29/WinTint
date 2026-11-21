//! WinTint: personalizes Windows through independent modules, each with its own
//! section of the settings file: the dock and the colors of Windows itself.

mod color;
mod config;
mod dock;
mod win;
mod windows_theme;

use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, Wry};

use config::AppConfig;
use dock::DockState;

const SETTINGS_WINDOW: &str = "settings";

/// State that belongs to the app as a whole rather than to one module.
#[derive(Default)]
struct AppState {
    start_with_windows: AtomicBool,
    dock_toggle: Mutex<Option<CheckMenuItem<Wry>>>,
}

#[tauri::command]
fn get_config(app: AppHandle) -> Result<AppConfig, String> {
    config::load(&app)
}

#[tauri::command]
fn save_config(config: AppConfig, app: AppHandle) -> Result<(), String> {
    let config = config::store(&app, config)?;
    let state = app.state::<AppState>();
    let autostart = config.start_with_windows;
    if state.start_with_windows.swap(autostart, Ordering::Relaxed) != autostart {
        win::set_autostart(autostart)?;
    }
    apply_config(&app, config);
    Ok(())
}

// Runs off the main thread because extracting an icon can touch the disk.
#[tauri::command(async)]
fn get_icon(path: String) -> Option<String> {
    win::file_icon(&path)
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    request_settings(&app);
}

/// Paints the settings window's title bar to match its theme.
#[tauri::command]
fn set_settings_colors(app: AppHandle, caption: String, text: String, dark: bool) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW)
        && let Ok(hwnd) = window.hwnd()
    {
        win::set_title_bar_colors(hwnd.0 as isize, &caption, &text, dark);
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn open_config_file(app: AppHandle) -> Result<(), String> {
    let path = config::path(&app)?;
    Command::new("notepad.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Hands a stored config to every module, the tray and the open windows.
fn apply_config(app: &AppHandle, config: AppConfig) {
    dock::apply(app, &config.dock);
    windows_theme::apply(app, &config);
    if let Some(toggle) = app.state::<AppState>().dock_toggle.lock().unwrap().as_ref() {
        let _ = toggle.set_checked(config.dock.enabled);
    }
    let _ = app.emit("config-changed", config);
}

fn toggle_dock(app: &AppHandle) {
    let enabled = !dock::is_enabled(app);
    // A broken file still lets the dock be toggled, just not remembered.
    match config::load(app) {
        Ok(mut config) => {
            config.dock.enabled = enabled;
            let _ = config::save(app, &config);
            apply_config(app, config);
        }
        Err(_) => dock::set_enabled(app, enabled),
    }
}

/// Creating a window from the main thread deadlocks on Windows, and menu events
/// run there, so the window is always opened from its own thread.
fn request_settings(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let window = match app.get_webview_window(SETTINGS_WINDOW) {
            Some(window) => {
                let _ = window.unminimize();
                window
            }
            None => {
                let built = WebviewWindowBuilder::new(
                    &app,
                    SETTINGS_WINDOW,
                    WebviewUrl::App("settings.html".into()),
                )
                .title("WinTint")
                .inner_size(980.0, 680.0)
                .min_inner_size(780.0, 540.0)
                .center()
                .build();
                let Ok(window) = built else {
                    return;
                };
                window
            }
        };
        if let Ok(hwnd) = window.hwnd() {
            let _ = win::focus_window(hwnd.0 as isize);
        }
    });
}

fn build_tray(app: &AppHandle, dock_enabled: bool) -> tauri::Result<()> {
    let toggle =
        CheckMenuItem::with_id(app, "dock", "Dock ativa", true, dock_enabled, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Personalizar...", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &settings, &quit])?;
    *app.state::<AppState>().dock_toggle.lock().unwrap() = Some(toggle);

    let mut tray = TrayIconBuilder::new()
        .tooltip("WinTint")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "dock" => toggle_dock(app),
            "settings" => request_settings(app),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Opening the program again brings up the settings, the only way back
        // while every module is off and the tray icon is out of reach.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            request_settings(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(DockState::default())
        .setup(|app| {
            let handle = app.handle();
            let config = config::load(handle).ok();
            win::remove_legacy_autostart();
            if let Some(config) = &config {
                let state = app.state::<AppState>();
                state
                    .start_with_windows
                    .store(config.start_with_windows, Ordering::Relaxed);
                // Written again so the entry follows the program if it was moved.
                if config.start_with_windows {
                    let _ = win::set_autostart(true);
                }
            }
            let dock_config = config.as_ref().map(|config| &config.dock);
            build_tray(handle, dock_config.is_none_or(|dock| dock.enabled))?;
            dock::start(handle, dock_config);
            windows_theme::start(handle, config.as_ref());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_icon,
            open_settings,
            set_settings_colors,
            open_config_file,
            quit_app,
            dock::save_dock_order,
            dock::set_dragging,
            dock::get_windows,
            dock::load_bar_image,
            dock::set_layout,
            dock::launch_app,
            dock::focus_window,
            dock::minimize_window,
            dock::open_start_menu,
            dock::open_quick_settings,
            dock::open_calendar,
            dock::open_task_manager,
            dock::get_widget_data,
            dock::set_volume,
            dock::media_action,
            windows_theme::get_windows_look
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // A disabled dock leaves no windows open, which must not end the program.
            RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            RunEvent::Exit => dock::shutdown(app),
            _ => {}
        });
}
