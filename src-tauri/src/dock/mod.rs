//! The dock: a bar on a screen edge with apps and widgets, which can stand in
//! for the Windows taskbar.

pub mod config;
mod widgets;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
    WebviewWindowBuilder,
};
use windows::Win32::Foundation::RECT;

use crate::win::{self, AppWindow};
use config::{DockConfig, Edge, PinnedApp, WidgetId};
use widgets::WidgetData;

const CURSOR_POLL: Duration = Duration::from_millis(40);
/// How many cursor polls pass between window list refreshes (about 500 ms).
const TICKS_PER_SNAPSHOT: u32 = 12;
const DOCK_WINDOW: &str = "dock";
const DOCK_RETRY: Duration = Duration::from_secs(1);
/// Grace period for reaching an auto-hidden dock after touching the screen edge.
const DOCK_HIDE_DELAY: Duration = Duration::from_millis(500);
const WIDGET_POLL: Duration = Duration::from_millis(100);
const WIDGET_REFRESH: Duration = Duration::from_secs(1);

/// Dock geometry measured by the frontend, in CSS pixels. `start` and `length`
/// run along the screen edge, `thickness` and `gap` away from it.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    edge: Edge,
    start: f64,
    length: f64,
    thickness: f64,
    gap: f64,
    /// How far the magnified icons can reach outside the bar at rest.
    extra: f64,
    window_thickness: f64,
}

#[derive(Default)]
pub struct DockState {
    enabled: AtomicBool,
    hide_taskbar: AtomicBool,
    auto_hide: AtomicBool,
    layout: Mutex<Option<Layout>>,
    windows: Mutex<Vec<AppWindow>>,
    dock_on_screen: AtomicBool,
    widgets: Mutex<Vec<WidgetId>>,
    widget_data: Mutex<WidgetData>,
    /// Asks the widget thread to read again without waiting for its next turn.
    widgets_stale: AtomicBool,
    dragging: AtomicBool,
}

/// Stores an order set by dragging icons. It reads the file again so a change
/// made in the settings window at the same time is not lost.
#[tauri::command]
pub fn save_dock_order(
    apps: Vec<PinnedApp>,
    app_order: Vec<String>,
    app: AppHandle,
) -> Result<(), String> {
    let mut config = crate::config::load(&app)?;
    config.dock.apps = apps;
    config.dock.app_order = app_order;
    crate::config::save(&app, &config)?;
    let _ = app.emit("dock-order-changed", config);
    Ok(())
}

#[tauri::command]
pub fn set_dragging(dragging: bool, state: State<DockState>) {
    state.dragging.store(dragging, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_windows(state: State<DockState>) -> Vec<AppWindow> {
    state.windows.lock().unwrap().clone()
}

#[tauri::command(async)]
pub fn load_bar_image(path: String) -> Result<String, String> {
    win::read_image(&path)
}

#[tauri::command]
pub fn set_layout(layout: Layout, state: State<DockState>) {
    *state.layout.lock().unwrap() = Some(layout);
}

#[tauri::command]
pub fn launch_app(path: String) -> Result<(), String> {
    win::launch(&path)
}

#[tauri::command]
pub fn focus_window(hwnd: isize) -> Result<(), String> {
    win::focus_window(hwnd)
}

#[tauri::command]
pub fn minimize_window(hwnd: isize) {
    win::minimize_window(hwnd);
}

#[tauri::command]
pub fn open_start_menu() {
    win::open_start_menu();
}

#[tauri::command]
pub fn open_quick_settings() {
    win::open_quick_settings();
}

#[tauri::command]
pub fn open_calendar() {
    win::open_calendar();
}

#[tauri::command]
pub fn open_task_manager() -> Result<(), String> {
    win::launch("taskmgr.exe")
}

#[tauri::command]
pub fn get_widget_data(state: State<DockState>) -> WidgetData {
    state.widgets_stale.store(true, Ordering::Relaxed);
    state.widget_data.lock().unwrap().clone()
}

#[tauri::command(async)]
pub fn set_volume(level: f32, state: State<DockState>) -> Result<(), String> {
    widgets::set_volume(level)?;
    state.widgets_stale.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command(async)]
pub fn media_action(action: String, state: State<DockState>) -> Result<(), String> {
    widgets::media_action(&action)?;
    state.widgets_stale.store(true, Ordering::Relaxed);
    Ok(())
}

/// Takes in the dock's settings. Without a readable config the dock still runs.
pub fn apply(app: &AppHandle, config: &DockConfig) {
    let state = app.state::<DockState>();
    state.enabled.store(config.enabled, Ordering::Relaxed);
    state.hide_taskbar.store(config.hide_taskbar, Ordering::Relaxed);
    state.auto_hide.store(config.auto_hide, Ordering::Relaxed);
    *state.widgets.lock().unwrap() = config.enabled_widgets();
    state.widgets_stale.store(true, Ordering::Relaxed);
}

pub fn is_enabled(app: &AppHandle) -> bool {
    app.state::<DockState>().enabled.load(Ordering::Relaxed)
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    app.state::<DockState>().enabled.store(enabled, Ordering::Relaxed);
}

pub fn start(app: &AppHandle, config: Option<&DockConfig>) {
    match config {
        Some(config) => apply(app, config),
        None => set_enabled(app, true),
    }
    spawn_monitor(app.clone());
    spawn_widget_poller(app.clone());
}

/// Brings the Windows taskbar back before the program ends.
pub fn shutdown(app: &AppHandle) {
    // Stops the monitor first, or it could hide the taskbar again.
    app.state::<DockState>()
        .hide_taskbar
        .store(false, Ordering::Relaxed);
    thread::sleep(CURSOR_POLL * 2);
    win::set_taskbar_visible(true);
}

/// Pins the window to an edge of the work area, spanning that edge entirely.
fn place_window(
    window: &WebviewWindow,
    area: RECT,
    edge: Edge,
    thickness: i32,
) -> tauri::Result<()> {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    let (size, position) = match edge {
        Edge::Bottom => ((width, thickness), (area.left, area.bottom - thickness)),
        Edge::Top => ((width, thickness), (area.left, area.top)),
        Edge::Left => ((thickness, height), (area.left, area.top)),
        Edge::Right => ((thickness, height), (area.right - thickness, area.top)),
    };
    window.set_size(PhysicalSize::new(size.0, size.1))?;
    window.set_position(PhysicalPosition::new(position.0, position.1))
}

/// The cursor as distance along the edge and distance away from it.
fn edge_coords((x, y): (i32, i32), area: RECT, edge: Edge) -> (f64, f64) {
    let (along, depth) = match edge {
        Edge::Bottom => (x - area.left, area.bottom - 1 - y),
        Edge::Top => (x - area.left, y - area.top),
        Edge::Left => (y - area.top, x - area.left),
        Edge::Right => (y - area.top, area.right - 1 - x),
    };
    (along as f64, depth as f64)
}

fn is_over_dock(
    cursor: (i32, i32),
    area: RECT,
    layout: Layout,
    scale: f64,
    hovering: bool,
    auto_hide: bool,
) -> bool {
    let (along, depth) = edge_coords(cursor, area, layout.edge);
    let reach = if hovering { layout.extra * scale } else { 0.0 };
    let start = layout.start * scale - reach;
    let end = (layout.start + layout.length) * scale + reach;
    // The cursor arrives from the screen edge, so the gap must not count as leaving.
    let near = if auto_hide { 0.0 } else { layout.gap * scale };
    let far = (layout.gap + layout.thickness) * scale + reach;
    along >= start && along <= end && depth >= near && depth <= far
}

fn is_at_edge(cursor: (i32, i32), area: RECT, edge: Edge) -> bool {
    let (along, depth) = edge_coords(cursor, area, edge);
    let length = match edge {
        Edge::Bottom | Edge::Top => area.right - area.left,
        Edge::Left | Edge::Right => area.bottom - area.top,
    };
    along >= 0.0 && along < length as f64 && depth <= 0.0
}

/// Decides when an auto-hidden dock is on screen.
#[derive(Default)]
struct AutoHide {
    revealed: bool,
    left_at: Option<Instant>,
}

impl AutoHide {
    /// Returns true when the dock has to slide in or out.
    fn update(&mut self, at_edge: bool, over_dock: bool) -> bool {
        if !self.revealed {
            self.revealed = at_edge;
            self.left_at = None;
            return at_edge;
        }
        if at_edge || over_dock {
            self.left_at = None;
            return false;
        }
        let left_at = *self.left_at.get_or_insert_with(Instant::now);
        if left_at.elapsed() < DOCK_HIDE_DELAY {
            return false;
        }
        self.reset();
        true
    }

    fn reset(&mut self) {
        self.revealed = false;
        self.left_at = None;
    }
}

/// Reads widget data only while the dock is on screen, so an auto-hidden dock
/// costs nothing, and refreshes the moment it slides in.
fn spawn_widget_poller(app: AppHandle) {
    thread::spawn(move || {
        widgets::init_thread();
        let mut collector = widgets::Collector::default();
        let mut last_read: Option<Instant> = None;

        loop {
            thread::sleep(WIDGET_POLL);

            let state = app.state::<DockState>();
            if !state.dock_on_screen.load(Ordering::Relaxed) {
                last_read = None;
                continue;
            }
            let stale = state.widgets_stale.swap(false, Ordering::Relaxed);
            if !stale && last_read.is_some_and(|read| read.elapsed() < WIDGET_REFRESH) {
                continue;
            }

            let wanted = state.widgets.lock().unwrap().clone();
            let data = collector.collect(&wanted);
            last_read = Some(Instant::now());

            let mut current = state.widget_data.lock().unwrap();
            if *current != data {
                *current = data.clone();
                let _ = app.emit("widgets-changed", data);
            }
        }
    });
}

/// Background loop that does what the page cannot: a click-through window
/// receives no mouse events, so the cursor has to be tracked from here.
fn spawn_monitor(app: AppHandle) {
    thread::spawn(move || {
        let own_pid = std::process::id();
        let mut dock: Option<(WebviewWindow, isize)> = None;
        let mut shown = false;
        let mut hovering = false;
        let mut tick = 0u32;
        let mut active = None;
        let mut placed = None;
        let mut fullscreen = false;
        let mut auto_hide_state = AutoHide::default();
        // Starts true so a taskbar left hidden by a killed dock comes back.
        let mut taskbar_hidden = true;

        loop {
            thread::sleep(CURSOR_POLL);
            tick = tick.wrapping_add(1);

            let state = app.state::<DockState>();
            let enabled = state.enabled.load(Ordering::Relaxed);

            // Checked on every poll because opening Start brings the taskbar back.
            if enabled && state.hide_taskbar.load(Ordering::Relaxed) {
                win::set_taskbar_visible(false);
                taskbar_hidden = true;
            } else if taskbar_hidden {
                win::set_taskbar_visible(true);
                taskbar_hidden = false;
            }

            if !enabled {
                state.dock_on_screen.store(false, Ordering::Relaxed);
                // Closing the page, not just hiding it, is what frees the
                // memory held by WebView2. It is created again when enabled.
                if let Some((window, _)) = dock.take() {
                    let _ = window.destroy();
                    *state.layout.lock().unwrap() = None;
                    state.dragging.store(false, Ordering::Relaxed);
                    shown = false;
                    hovering = false;
                    placed = None;
                    fullscreen = false;
                    auto_hide_state.reset();
                }
                continue;
            }

            if dock.is_none() {
                // The previous window may still be on its way out.
                if app.get_webview_window(DOCK_WINDOW).is_some() {
                    continue;
                }
                let created = create_dock_window(&app).and_then(|window| {
                    let hwnd = window.hwnd()?.0 as isize;
                    Ok((window, hwnd))
                });
                let Ok((window, hwnd)) = created else {
                    thread::sleep(DOCK_RETRY);
                    continue;
                };
                win::set_dock_style(hwnd, true);
                dock = Some((window, hwnd));
            }
            let Some((window, hwnd)) = dock.clone() else {
                continue;
            };

            if tick % TICKS_PER_SNAPSHOT == 0 {
                fullscreen = win::is_fullscreen_app_active(own_pid);
            }
            if fullscreen {
                state.dock_on_screen.store(false, Ordering::Relaxed);
                if shown {
                    let _ = window.hide();
                    shown = false;
                    hovering = false;
                    win::set_dock_style(hwnd, true);
                    let _ = app.emit("dock-hover", false);
                    auto_hide_state.reset();
                    let _ = app.emit("dock-reveal", false);
                }
                continue;
            }

            let Some(layout) = *state.layout.lock().unwrap() else {
                continue;
            };
            let Some(area) = win::work_area() else {
                continue;
            };
            let scale = window.scale_factor().unwrap_or(1.0);

            let thickness = (layout.window_thickness * scale).ceil() as i32;
            let target = (area, layout.edge, thickness);
            if placed != Some(target)
                && place_window(&window, area, layout.edge, thickness).is_ok()
            {
                placed = Some(target);
            }
            if !shown && placed.is_some() {
                let _ = window.show();
                win::set_dock_style(hwnd, true);
                shown = true;
            }

            if let Some(cursor) = win::cursor_pos() {
                let auto_hide = state.auto_hide.load(Ordering::Relaxed);
                if !auto_hide && auto_hide_state.revealed {
                    auto_hide_state.reset();
                    let _ = app.emit("dock-reveal", false);
                }

                // A dock that is slid away must let every click through.
                let on_screen = !auto_hide || auto_hide_state.revealed;
                state.dock_on_screen.store(on_screen, Ordering::Relaxed);
                // An icon being dragged may leave the bar and must keep the mouse.
                let over = state.dragging.load(Ordering::Relaxed)
                    || (on_screen
                        && is_over_dock(cursor, area, layout, scale, hovering, auto_hide));
                if over != hovering {
                    hovering = over;
                    win::set_dock_style(hwnd, !over);
                    let _ = app.emit("dock-hover", over);
                }

                if auto_hide && auto_hide_state.update(is_at_edge(cursor, area, layout.edge), over)
                {
                    let _ = app.emit("dock-reveal", auto_hide_state.revealed);
                }
            }

            if tick % TICKS_PER_SNAPSHOT == 0 {
                // Showing or moving the window can make Tauri restore its styles.
                win::set_dock_style(hwnd, !hovering);

                // Clicking the dock must not make the app the user was in
                // look inactive, so the dock's own focus is skipped here.
                active = win::foreground_window(own_pid).or(active);
                let windows = win::list_windows(own_pid, active);
                let mut current = state.windows.lock().unwrap();
                if *current != windows {
                    *current = windows.clone();
                    let _ = app.emit("windows-changed", windows);
                }
            }
        }
    });
}

fn create_dock_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == DOCK_WINDOW)
        .expect("the dock window is declared in tauri.conf.json");
    WebviewWindowBuilder::from_config(app, config)?.build()
}
