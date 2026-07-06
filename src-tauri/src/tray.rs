//! System tray icon + menu.
//!
//! T2.9 — the tray is the always-present control point. It offers recording
//! start/stop, quick navigation to the History and Settings views, pill
//! visibility toggles, a recording-mode switch (push-to-talk vs toggle), and
//! Help/Quit. Navigation items reveal the main window and emit a `navigate`
//! event the frontend (App.svelte) routes to the right view.

use anyhow::Result;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Reveal the main window and ask the frontend to switch to `view`
/// ("dashboard" | "history" | "settings" | "help").
fn navigate_main<R: Runtime>(app: &AppHandle<R>, view: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.emit("navigate", view);
    }
}

/// Project page opened by the tray "Help" item.
const HELP_URL: &str = "https://github.com/AEmad99/devwhisp";

/// Open a URL in the user's default browser, best-effort and cross-platform.
fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Human-readable label for the current recording mode.
fn mode_label() -> String {
    match crate::hotkey::get_mode().as_str() {
        "toggle" => "Toggle (tap to start/stop)".to_string(),
        "vad" => "VAD (auto-stop on silence)".to_string(),
        _ => "Push-to-Talk (hold)".to_string(),
    }
}

pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let start = MenuItem::with_id(app, "start", "Start Recording", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop Recording", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    // Informational (disabled) row showing the active hotkey + mode.
    let info = MenuItem::with_id(
        app,
        "info_hotkey",
        format!("{} · {}", crate::hotkey::current_shortcut_string(), mode_label()),
        false,
        None::<&str>,
    )?;
    // Recording-mode switch (click to cycle push-to-talk / toggle / vad).
    let is_toggle = crate::hotkey::get_mode().eq_ignore_ascii_case("toggle");
    let mode_toggle = CheckMenuItem::with_id(
        app,
        "mode_toggle",
        "Cycle recording mode",
        true,
        is_toggle,
        None::<&str>,
    )?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let open_history = MenuItem::with_id(app, "open_history", "Open History", true, None::<&str>)?;
    let open_settings = MenuItem::with_id(app, "open_settings", "Settings…", true, None::<&str>)?;
    let show_pill = MenuItem::with_id(app, "show_pill", "Show Pill", true, None::<&str>)?;
    let hide_pill = MenuItem::with_id(app, "hide_pill", "Hide Pill", true, None::<&str>)?;
    let sep3 = PredefinedMenuItem::separator(app)?;

    let help = MenuItem::with_id(app, "help", "Help", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open DevWhisp", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit DevWhisp", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &start, &stop, &sep1, &info, &mode_toggle, &sep2, &open_history, &open_settings,
            &show_pill, &hide_pill, &sep3, &help, &open, &quit,
        ],
    )?;

    let app_for_menu = app.clone();
    // Clone the menu items the handler mutates/refreshes so it can update their
    // text/checked state when the mode changes.
    let mode_toggle_for_handler = mode_toggle.clone();
    let info_for_handler = info.clone();

    let _tray = TrayIconBuilder::with_id("devwhisp-tray")
        .tooltip("DevWhisp — voice-to-text pill")
        .icon(app.default_window_icon().cloned().unwrap_or_else(|| {
            tauri::image::Image::new_owned(vec![0u8; 4], 1, 1)
        }))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "start" => {
                log::info!("tray: start");
                let _ = crate::audio::start();
            }
            "stop" => {
                log::info!("tray: stop");
                let _ = crate::hotkey::stop_and_transcribe(&app);
            }
            "mode_toggle" => {
                // Cycle push-to-talk -> toggle -> vad -> ptt (supports new VAD mode).
                let current = crate::hotkey::get_mode();
                let next = if current.eq_ignore_ascii_case("push-to-talk") {
                    "toggle"
                } else if current.eq_ignore_ascii_case("toggle") {
                    "vad"
                } else {
                    "push-to-talk"
                };
                crate::hotkey::set_mode(next);
                let now_toggle = next.eq_ignore_ascii_case("toggle");
                let _ = mode_toggle_for_handler.set_checked(now_toggle);
                let _ = info_for_handler.set_text(format!(
                    "{} · {}",
                    crate::hotkey::current_shortcut_string(),
                    mode_label()
                ));
                log::info!("tray: recording mode -> {next}");
            }
            "open_history" => {
                log::info!("tray: open history");
                navigate_main(&app_for_menu, "history");
            }
            "open_settings" => {
                log::info!("tray: open settings");
                navigate_main(&app_for_menu, "settings");
            }
            "help" => {
                log::info!("tray: help -> opening project page");
                open_url(HELP_URL);
            }
            "show_pill" => {
                log::info!("tray: show pill");
                if let Err(e) = crate::window::pill_window::show_pill_window(&app_for_menu) {
                    log::warn!("tray show_pill failed: {e:?}");
                }
            }
            "hide_pill" => {
                log::info!("tray: hide pill");
                if let Err(e) = crate::window::pill_window::hide_pill_window(&app_for_menu) {
                    log::warn!("tray hide_pill failed: {e:?}");
                }
            }
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                log::info!("tray: quit");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|_tray, event| {
            // Left-click opens the main window.
            if let tauri::tray::TrayIconEvent::Click { button, .. } = event {
                if matches!(button, tauri::tray::MouseButton::Left) {
                    if let Some(window) = _tray.app_handle().get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    log::info!("system tray built");
    Ok(())
}
