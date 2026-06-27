//! DevWhisp — small, tray-resident, push-to-talk voice-to-text desktop app.
//!
//! Phase 1 (M1) wires the foundation: Tauri shell, plugins, system tray, global
//! hotkey, and the audio capture / STT / inject module skeletons.
//!
//! The full end-to-end flow (hotkey → record → transcribe → cursor-paste)
//! lands in tasks T1.4 + T1.5.

mod audio;
mod config;
mod dictionary;
mod formatter;
mod history;
mod hotkey;
mod inject;
mod ipc;
mod pill;
mod stt;
mod tray;
mod window;

use tauri::{Manager, WindowEvent};

/// Tauri app entry point. Wires plugins, tray, global hotkey, and IPC commands.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    log::info!("DevWhisp starting up...");

    tauri::Builder::default()
        // --- Plugins ---
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Focus the existing window when a second instance is launched.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
                let _ = window.show();
            }
        }))
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        // Start-at-login. The frontend toggles it via @tauri-apps/plugin-autostart
        // (enable/disable/isEnabled); the desktop launcher is registered here.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        // tauri-plugin-updater is wired in T4.3 once we have signing keys
        // and a release endpoint. For Phase 1 it's omitted to avoid the
        // required config block.
        // --- Setup hook: tray + hotkey + initial state ---
        .setup(|app| {
            log::info!("DevWhisp setup hook running");

            // Model download is now fully in-app (no bundling). On first use the UI
            // (onboarding / dashboard / settings) will prompt the user to download
            // their chosen model. We no longer stage anything at startup.
            // The hotkey and other flows will surface clear "model required" guidance.

            // Hand the AppHandle to the audio module so the capture thread
            // can emit `audio-level` events without it being threaded
            // through every callsite.
            audio::set_app_handle(app.handle().clone());

            // Spawn the floating pill window. Errors are logged inside
            // create_pill_window and must not abort startup.
            if let Err(e) = window::pill_window::create_pill_window(app.handle()) {
                log::warn!("pill window failed to create: {e:?}");
            }

            // Build system tray
            tray::build_tray(app.handle())?;

            // Register the global hotkey from the persisted spec (default
            // "Ctrl+Shift+Space"). Dispatch through hotkey::on_hotkey so the
            // configured recording mode is honored — push-to-talk records
            // while held; toggle starts on the first press and stops on the
            // next (key-up ignored); VAD auto-ends on silence.
            //
            // CRITICAL: the global-shortcut plugin's `on_shortcut` internally
            // dispatches the actual OS registration onto the main thread via
            // `run_on_main_thread` and then blocks on a channel for the
            // result. The setup hook ALSO runs on the main thread, so calling
            // `register_initial` synchronously here would deadlock (the main
            // thread blocks waiting for a task it can never run because it's
            // busy blocking). We defer registration to the next iteration of
            // the event loop so the setup hook can return first.
            let reg_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Yield once so the setup hook returns and the main-thread
                // event loop is free to process the registration task.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if let Err(e) = hotkey::register_initial(&reg_app) {
                    log::error!("failed to register initial hotkey: {e:?}");
                }
            });

            // Pre-warm the whisper model in the background so the very first
            // press-and-release doesn't pay the model-load cost (~1 s on
            // tiny.en). No-op if the model isn't downloaded yet.
            let warm_app = app.handle().clone();
            std::thread::Builder::new()
                .name("devwhisp-whisper-warm".to_string())
                .spawn(move || {
                    if let Err(e) = stt::whisper::warm() {
                        log::info!("whisper warm skipped: {e:?}");
                    } else {
                        log::info!("whisper warmed for {warm_app:?}");
                    }
                })
                .ok();

            log::info!("DevWhisp setup complete");
            Ok(())
        })
        // --- IPC commands (frontend -> backend) ---
        .invoke_handler(tauri::generate_handler![
            ipc::ping,
            ipc::get_app_info,
            ipc::start_listening,
            ipc::stop_listening,
            ipc::is_listening,
            ipc::transcribe_buffer,
            ipc::get_model_status,
            ipc::download_model,
            ipc::list_history,
            ipc::search_history,
            ipc::delete_history_entry,
            ipc::clear_history,
            ipc::get_dictionary,
            ipc::add_dictionary_entry,
            ipc::remove_dictionary_entry,
            ipc::get_recording_mode,
            ipc::set_recording_mode,
            ipc::get_vad_silence_ms,
            ipc::set_vad_silence_ms,
            ipc::reinject_text,
            ipc::get_format_options,
            ipc::set_format_options,
            ipc::open_external,
            ipc::get_acceleration_info,
            ipc::set_acceleration_mode,
            ipc::list_audio_devices,
            ipc::get_selected_audio_device,
            ipc::set_selected_audio_device,
            ipc::set_active_model,
            ipc::list_model_statuses,
            ipc::get_hotkey,
            ipc::set_hotkey,
            ipc::list_predefined_hotkeys,
            pill::ipc::show_pill,
            pill::ipc::hide_pill,
            pill::ipc::toggle_pill,
            pill::ipc::save_pill_position,
            pill::ipc::trigger_hotkey,
            pill::ipc::set_pill_size,
            pill::ipc::get_pill_size,
            pill::ipc::set_pill_position_preset,
        ])
        // --- Close-to-tray: intercept close requests on the main window so
        // clicking X hides the window instead of quitting. The user can
        // bring it back via the tray menu ("Open DevWhisp") or the global
        // hotkey. Quit is reserved for the tray menu's "Quit DevWhisp"
        // item, which calls `app.exit(0)` directly.
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    log::info!("main window close requested -> hiding to tray");
                    api.prevent_close();
                    let _ = window.hide();
                }
            } else if window.label() == window::pill_window::PILL_LABEL {
                // Snap the pill to a screen edge once a drag settles.
                if let WindowEvent::Moved(_) = event {
                    window::pill_window::on_pill_moved(window);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running DevWhisp");
}
