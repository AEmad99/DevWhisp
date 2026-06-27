//! IPC commands for the pill widget.
//!
//! Frontend calls (`Pill.svelte` and the tray menu) → these thin handlers.
//! The actual logic lives in `crate::window::pill_window` and
//! `crate::hotkey`. Keep this layer anemic — just plumbing.

use tauri::AppHandle;

/// Show the floating pill window. Lazily creates it on first call.
#[tauri::command]
pub fn show_pill(app: AppHandle) -> Result<(), String> {
    crate::window::pill_window::show_pill_window(&app).map_err(|e| e.to_string())
}

/// Hide the floating pill window. Persists its current position so the
/// next `show_pill` restores exactly where the user left it.
#[tauri::command]
pub fn hide_pill(app: AppHandle) -> Result<(), String> {
    crate::window::pill_window::hide_pill_window(&app).map_err(|e| e.to_string())
}

/// Toggle the pill between hidden and visible.
#[tauri::command]
pub fn toggle_pill(app: AppHandle) -> Result<(), String> {
    crate::window::pill_window::toggle_pill_window(&app).map_err(|e| e.to_string())
}

/// Persist the pill's logical-pixel position. Called from the frontend
/// drag handler so the pill lands where the user last left it.
#[tauri::command]
pub fn save_pill_position(app: AppHandle, x: f64, y: f64) -> Result<(), String> {
    crate::window::pill_window::persist_position(&app, x, y).map_err(|e| e.to_string())
}

/// Programmatically fire the hotkey. Used by Pill.svelte's "click to talk"
/// affordance so the same audio path runs whether the user pressed
/// Ctrl+Shift+Space or clicked the pill. Routed through `on_hotkey` so a
/// click respects the configured mode (in toggle mode a click starts, the
/// next click stops).
#[tauri::command]
pub fn trigger_hotkey(app: AppHandle) -> Result<(), String> {
    crate::hotkey::on_hotkey(&app, true).map_err(|e| e.to_string())
}

/// Resize the pill window. Width is clamped to `[150.0, 420.0]`, height to
/// `[36.0, 96.0]`. The pill's CSS uses `--pill-w` / `--pill-h` so the
/// content reflows to match the chosen size.
#[tauri::command]
pub fn set_pill_size(
    app: AppHandle,
    width: f64,
    height: f64,
) -> Result<(), String> {
    crate::window::pill_window::set_pill_size(&app, width, height)
        .map_err(|e| e.to_string())
}

/// Return the pill's current logical size as `[width, height]`. Falls back
/// to defaults from `pill_window` if the window isn't open.
#[tauri::command]
pub fn get_pill_size(app: AppHandle) -> Result<[f64; 2], String> {
    crate::window::pill_window::get_pill_size(&app).map_err(|e| e.to_string())
}

/// Snap the pill to one of 5 preset positions. Accepted values:
/// `"top-left"`, `"top-right"`, `"bottom-left"`, `"bottom-right"`, `"center"`.
/// Position is persisted so it survives a restart.
#[tauri::command]
pub fn set_pill_position_preset(
    app: AppHandle,
    preset: String,
) -> Result<(), String> {
    crate::window::pill_window::set_pill_position_preset(&app, &preset)
        .map_err(|e| e.to_string())
}
