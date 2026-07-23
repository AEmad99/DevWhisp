//! Always-on-top floating "pill" window.
//!
//! The pill is the centerpiece UX surface: a small, frameless, transparent,
//! draggable widget that visualizes STT state (idle / listening / processing
//! / error) and the live audio level. It opens automatically on app startup
//! and is re-shown from the tray via the "Show Pill" menu item.
//!
//! Position is persisted to `~/.devwhisp/pill-position.json` so the pill lands
//! where the user last left it. First launch defaults to bottom-right of the
//! primary monitor.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, WebviewUrl,
    WebviewWindowBuilder,
};

/// Window label the rest of the app (event emitters, tray) addresses the pill by.
pub const PILL_LABEL: &str = "pill";

/// Snap the pill to a screen edge when released within this many logical
/// pixels of it (Plan §2.2 "snap zones").
const SNAP_THRESHOLD: f64 = 80.0;

/// Drag-settle bookkeeping: each `Moved` event bumps the sequence; a debounced
/// worker only snaps if its sequence is still the latest (i.e. the drag has
/// stopped). `SNAPPING` guards the programmatic move we make while snapping so
/// it doesn't re-trigger the debounce.
static MOVE_SEQ: AtomicU64 = AtomicU64::new(0);
static SNAPPING: AtomicBool = AtomicBool::new(false);

/// Default size when no saved position exists. The pill is intentionally
/// compact — the user can grow it from Settings (size sliders) if they want.
/// Listening/processing state adds a bit of width but the underlying window
/// stays at the chosen size; the pill's content auto-grows the window via
/// `set_size` if it needs more room (handled by `set_pill_size` IPC).
// Larger than the visual capsule so the webview can inset the pill body
// (safe padding for AA + glow + brand mark) without clipping the left edge.
const DEFAULT_WIDTH: f64 = 196.0;
const DEFAULT_HEIGHT: f64 = 48.0;

/// Margin from the bottom-right corner of the primary monitor on first launch.
const DEFAULT_MARGIN: f64 = 24.0;

/// On-disk record for the saved window position. Stored as logical pixels
/// so it stays valid when the user later changes DPI / monitor scaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedPosition {
    x: f64,
    y: f64,
}

/// Resolved path to `~/.devwhisp/pill-position.json`.
fn position_file() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".devwhisp");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("pill-position.json"))
}

/// Read the saved position, if one exists and parses cleanly.
fn load_saved_position() -> Option<SavedPosition> {
    let path = position_file()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<SavedPosition>(&raw).ok()
}

/// Write the saved position. Best-effort: errors are logged, never fatal.
pub fn save_position(x: f64, y: f64) {
    let Some(path) = position_file() else {
        return;
    };
    let payload = SavedPosition { x, y };
    match serde_json::to_string_pretty(&payload) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("failed to save pill position: {e}");
            }
        }
        Err(e) => log::warn!("failed to serialize pill position: {e}"),
    }
}

/// Compute the default bottom-right position on the primary monitor.
fn bottom_right_default<R: tauri::Runtime>(app: &AppHandle<R>) -> (f64, f64) {
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let size = monitor.size();
        let scale = monitor.scale_factor();
        // Convert physical monitor size to logical pixels so the math below
        // matches what Tauri uses for window positions.
        let logical_w = size.width as f64 / scale;
        let logical_h = size.height as f64 / scale;
        let x = (logical_w - DEFAULT_WIDTH - DEFAULT_MARGIN).max(0.0);
        let y = (logical_h - DEFAULT_HEIGHT - DEFAULT_MARGIN).max(0.0);
        return (x, y);
    }
    // No monitor info → fall back to a sensible "near top-right" spot so
    // the pill is still visible on most setups.
    (200.0, 200.0)
}

/// Build the pill WebviewWindow and show it. Idempotent: if a "pill" window
/// already exists (e.g. user re-invoked from the tray), it is unhidden and
/// focused instead of being recreated.
///
/// Errors are logged, never propagated — a missing pill must not stop
/// the main app from booting.
pub fn create_pill_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<()> {
    // If the window already exists, just reveal it. Cheaper than recreating
    // the webview and matches user expectation that tray → "Show Pill" is
    // an idempotent toggle.
    if let Some(existing) = app.get_webview_window(PILL_LABEL) {
        let _ = existing.show();
        let _ = existing.unminimize();
        log::info!("pill window already exists — showing existing instance");
        return Ok(());
    }

    // Decide where to put it. Saved position wins if present and sane;
    // otherwise we land bottom-right of the primary monitor.
    let (x, y) = load_saved_position()
        .map(|p| (p.x, p.y))
        .unwrap_or_else(|| bottom_right_default(app));

    let url = WebviewUrl::App("pill.html".into());

    let window = WebviewWindowBuilder::new(app, PILL_LABEL, url)
        .title("DevWhisp Pill")
        .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .visible(true)
        .shadow(false)
        .build()
        .context("failed to build pill window")?;

    // Position can drift slightly during build on some platforms; re-apply.
    let _ = window.set_position(LogicalPosition::new(x, y));
    let _ = window.set_size(LogicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT));
    // Don't grab focus on first show — the pill is a passive widget.
    let _ = window.set_focus();

    log::info!(
        "pill window created at logical ({x:.0}, {y:.0}) size {DEFAULT_WIDTH}x{DEFAULT_HEIGHT}"
    );
    Ok(())
}

/// Hide the pill window without closing it. Used by the tray "Hide Pill"
/// item and by Pill.svelte's close button.
pub fn hide_pill_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<()> {
    if let Some(window) = app.get_webview_window(PILL_LABEL) {
        // Persist the current position before hiding so the next "Show Pill"
        // restores exactly where the user left it.
        if let Ok(pos) = window.outer_position() {
            if let Some(monitor) = window.current_monitor().ok().flatten() {
                let scale = monitor.scale_factor();
                let logical_x = pos.x as f64 / scale;
                let logical_y = pos.y as f64 / scale;
                save_position(logical_x, logical_y);
            } else {
                save_position(pos.x as f64, pos.y as f64);
            }
        }
        window.hide().context("failed to hide pill window")?;
    }
    Ok(())
}

/// Show the pill (no-op if it doesn't exist yet — caller should ensure
/// `create_pill_window` has run at least once).
pub fn show_pill_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<()> {
    if let Some(window) = app.get_webview_window(PILL_LABEL) {
        window.show().context("failed to show pill window")?;
        let _ = window.unminimize();
    } else {
        // Lazily create if absent.
        create_pill_window(app)?;
    }
    Ok(())
}

/// Toggle pill visibility (used by tray quick-toggles).
pub fn toggle_pill_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<()> {
    match app.get_webview_window(PILL_LABEL) {
        Some(window) if window.is_visible().unwrap_or(false) => hide_pill_window(app),
        _ => show_pill_window(app),
    }
}

/// Persist the pill's current position. Called from the frontend drag handler
/// via a small IPC bridge (`pill_save_position`).
pub fn persist_position<R: tauri::Runtime>(app: &AppHandle<R>, x: f64, y: f64) -> Result<()> {
    save_position(x, y);
    if let Some(window) = app.get_webview_window(PILL_LABEL) {
        // Best-effort sync — the frontend will also call set_position.
        let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
    }
    Ok(())
}

/// Called from `lib.rs`'s window-event hook on every pill `Moved` event.
/// Debounces so the snap only runs once the drag has settled.
pub fn on_pill_moved<R: tauri::Runtime>(window: &tauri::Window<R>) {
    if SNAPPING.load(Ordering::SeqCst) {
        return; // ignore the move we just made while snapping
    }
    let seq = MOVE_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let win = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(220));
        // Only act if no newer move arrived — i.e. the drag has stopped.
        if MOVE_SEQ.load(Ordering::SeqCst) == seq {
            snap_to_edge(&win);
        }
    });
}

/// Snap the pill to the nearest screen edge if it was dropped within
/// `SNAP_THRESHOLD` of it; otherwise persist wherever the user left it.
/// All math is done in physical pixels relative to the pill's current
/// monitor, then persisted as logical pixels (matching `save_position`).
fn snap_to_edge<R: tauri::Runtime>(window: &tauri::Window<R>) {
    let Ok(pos) = window.outer_position() else {
        return;
    };
    let Some(monitor) = window.current_monitor().ok().flatten() else {
        return;
    };
    let scale = monitor.scale_factor();
    let m_pos = monitor.position();
    let m_size = monitor.size();
    let win_size = window.outer_size().unwrap_or_else(|_| {
        tauri::PhysicalSize::new(
            (DEFAULT_WIDTH * scale) as u32,
            (DEFAULT_HEIGHT * scale) as u32,
        )
    });

    let threshold = (SNAP_THRESHOLD * scale) as i32;
    let margin = (DEFAULT_MARGIN * scale) as i32;
    let rel_x = pos.x - m_pos.x;
    let rel_y = pos.y - m_pos.y;
    let max_x = (m_size.width as i32 - win_size.width as i32).max(0);
    let max_y = (m_size.height as i32 - win_size.height as i32).max(0);

    let mut nx = rel_x;
    let mut ny = rel_y;
    if rel_x <= threshold {
        nx = margin;
    } else if rel_x >= max_x - threshold {
        nx = (max_x - margin).max(0);
    }
    if rel_y <= threshold {
        ny = margin;
    } else if rel_y >= max_y - threshold {
        ny = (max_y - margin).max(0);
    }
    nx = nx.clamp(0, max_x);
    ny = ny.clamp(0, max_y);

    if nx != rel_x || ny != rel_y {
        SNAPPING.store(true, Ordering::SeqCst);
        let _ = window.set_position(PhysicalPosition::new(m_pos.x + nx, m_pos.y + ny));
        save_position((m_pos.x + nx) as f64 / scale, (m_pos.y + ny) as f64 / scale);
        // Release the guard after the programmatic move has been processed.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(250));
            SNAPPING.store(false, Ordering::SeqCst);
        });
    } else {
        // Not near an edge — persist where the user dropped it (covers the
        // native-drag path, where the frontend doesn't report the position).
        save_position(pos.x as f64 / scale, pos.y as f64 / scale);
    }
}

// ---- Pill size + position-preset IPC support --------------------------

/// Size bounds for the user-controlled pill. The range is intentionally
/// compact so the pill never becomes a large overlay. Min size leaves room
/// for the safe inset used by Pill.svelte (avoids left-edge clipping).
pub const MIN_WIDTH: f64 = 160.0;
pub const MAX_WIDTH: f64 = 360.0;
pub const MIN_HEIGHT: f64 = 36.0;
pub const MAX_HEIGHT: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PositionPreset {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

impl PositionPreset {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "top-left" | "topleft" => Some(Self::TopLeft),
            "top-right" | "topright" => Some(Self::TopRight),
            "bottom-left" | "bottomleft" => Some(Self::BottomLeft),
            "bottom-right" | "bottomright" => Some(Self::BottomRight),
            "center" | "centre" | "middle" => Some(Self::Center),
            _ => None,
        }
    }
}

/// Resize the pill window. Dimensions are clamped to safe bounds so a
/// frontend bug can't accidentally make the pill invisible.
pub fn set_pill_size<R: tauri::Runtime>(
    app: &AppHandle<R>,
    width: f64,
    height: f64,
) -> Result<()> {
    let w = width.clamp(MIN_WIDTH, MAX_WIDTH);
    let h = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    if let Some(window) = app.get_webview_window(PILL_LABEL) {
        window
            .set_size(LogicalSize::new(w, h))
            .context("failed to set pill size")?;
    } else {
        create_pill_window(app)?;
        if let Some(window) = app.get_webview_window(PILL_LABEL) {
            window
                .set_size(LogicalSize::new(w, h))
                .context("failed to set pill size after create")?;
        }
    }
    // Broadcast to the pill webview so the CSS `--pill-w` / `--pill-h` can
    // re-sync. The pill webview reads this on a `pill-size` event listener.
    let _ = app.emit(
        "pill-size",
        serde_json::json!({ "width": w, "height": h }),
    );
    Ok(())
}

/// Return the pill's current logical size, falling back to defaults if the
/// window isn't open.
pub fn get_pill_size<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<[f64; 2]> {
    if let Some(window) = app.get_webview_window(PILL_LABEL) {
        let s = window.inner_size().context("failed to read pill size")?;
        let monitor = window.current_monitor().ok().flatten();
        let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
        Ok([s.width as f64 / scale, s.height as f64 / scale])
    } else {
        Ok([DEFAULT_WIDTH, DEFAULT_HEIGHT])
    }
}

/// Snap the pill to one of the 5 preset positions against the primary
/// monitor. The position is persisted via `save_position` so the next
/// launch restores it.
pub fn set_pill_position_preset<R: tauri::Runtime>(
    app: &AppHandle<R>,
    preset: &str,
) -> Result<()> {
    let parsed = PositionPreset::parse(preset)
        .ok_or_else(|| anyhow!("unknown position preset '{preset}'"))?;
    let monitor = app
        .primary_monitor()
        .context("no primary monitor")?
        .context("no primary monitor")?;
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let logical_w = size.width as f64 / scale;
    let logical_h = size.height as f64 / scale;
    // Use current size if available so the chosen preset respects the
    // user's chosen pill dimensions; fall back to defaults otherwise.
    let (pill_w, pill_h) = match app.get_webview_window(PILL_LABEL) {
        Some(w) => {
            let s = w.inner_size().unwrap_or(tauri::PhysicalSize::new(
                (DEFAULT_WIDTH * scale) as u32,
                (DEFAULT_HEIGHT * scale) as u32,
            ));
            (s.width as f64 / scale, s.height as f64 / scale)
        }
        None => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    };
    let margin = DEFAULT_MARGIN;
    let (x, y) = match parsed {
        PositionPreset::TopLeft => (margin, margin),
        PositionPreset::TopRight => {
            (logical_w - pill_w - margin, margin)
        }
        PositionPreset::BottomLeft => (margin, logical_h - pill_h - margin),
        PositionPreset::BottomRight => (
            logical_w - pill_w - margin,
            logical_h - pill_h - margin,
        ),
        PositionPreset::Center => (
            (logical_w - pill_w) / 2.0,
            (logical_h - pill_h) / 2.0,
        ),
    };
    let (x, y) = (x.max(0.0), y.max(0.0));

    if let Some(window) = app.get_webview_window(PILL_LABEL) {
        SNAPPING.store(true, Ordering::SeqCst);
        window
            .set_position(LogicalPosition::new(x, y))
            .context("failed to set pill position")?;
        // Release the snap guard after the programmatic move settles so the
        // Moved-event hook doesn't re-snap and undo us.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(250));
            SNAPPING.store(false, Ordering::SeqCst);
        });
    } else {
        // Lazily create at the requested position so a hide+preset still
        // works even if the pill was never shown.
        let _ = create_pill_window_at(app, x, y);
    }
    save_position(x, y);
    Ok(())
}

/// Build the pill at a specific position. Used by `set_pill_position_preset`
/// when the pill doesn't exist yet.
fn create_pill_window_at<R: tauri::Runtime>(
    app: &AppHandle<R>,
    x: f64,
    y: f64,
) -> Result<()> {
    if app.get_webview_window(PILL_LABEL).is_some() {
        return Ok(());
    }
    let url = WebviewUrl::App("pill.html".into());
    let window = WebviewWindowBuilder::new(app, PILL_LABEL, url)
        .title("DevWhisp Pill")
        .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .visible(true)
        .shadow(false)
        .build()
        .context("failed to build pill window at preset")?;
    let _ = window.set_position(LogicalPosition::new(x, y));
    Ok(())
}