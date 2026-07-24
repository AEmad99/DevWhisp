//! DevWhisp IPC commands (frontend → backend bridge).
//!
//! Each `#[tauri::command]` here is callable from the Svelte frontend via
//! `invoke("name", { ...args })`. Keep this layer thin — actual logic lives
//! in `audio/`, `stt/`, `inject/`, `history/`, `formatter/`, `dictionary/`.

use crate::dictionary::{self, DictEntry};
use crate::formatter::{self, FormatOptions};
use crate::history::{self, TranscriptionRow};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;

/// Lightweight liveness check. Returns "pong".
#[tauri::command]
pub fn ping() -> &'static str {
    "pong"
}

/// Static metadata about the running app.
#[derive(Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub phase: &'static str,
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: "DevWhisp",
        version: env!("CARGO_PKG_VERSION"),
        phase: "M1 — feasibility spike (end-to-end PTT)",
    }
}

/// Start a recording session. Returns once audio capture is live.
#[tauri::command]
pub fn start_listening() -> Result<(), String> {
    crate::audio::start().map_err(|e| e.to_string())
}

/// Stop the current recording session and return the captured audio.
#[tauri::command]
pub fn stop_listening() -> Result<Vec<f32>, String> {
    crate::audio::stop_and_drain().map_err(|e| e.to_string())
}

/// Whether a recording session is currently active.
#[tauri::command]
pub fn is_listening() -> bool {
    crate::audio::is_active()
}

/// The active recording mode ("push-to-talk" | "toggle" | "vad").
#[tauri::command]
pub fn get_recording_mode() -> String {
    crate::hotkey::get_mode()
}

/// Set the recording mode ("push-to-talk" | "toggle" | "vad") and persist it.
#[tauri::command]
pub fn set_recording_mode(mode: String) -> Result<(), String> {
    // Defense in depth: even though hotkey::set_mode normalizes unknown values
    // to "push-to-talk", we surface an explicit error so the UI never silently
    // persists a typo or attacker-supplied string into settings.json.
    let normalized = mode.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "push-to-talk" | "toggle" | "vad" => {
            crate::hotkey::set_mode(&normalized);
            Ok(())
        }
        _ => Err(format!(
            "recording mode must be one of: push-to-talk, toggle, vad (got '{mode}')"
        )),
    }
}

/// VAD silence hold-off in milliseconds (default 600). After this much
/// continuous low-energy audio the utterance ends automatically.
#[tauri::command]
pub fn get_vad_silence_ms() -> u32 {
    crate::config::load_vad_silence_ms()
}

#[tauri::command]
pub fn set_vad_silence_ms(ms: u32) -> Result<(), String> {
    let clamped = ms.clamp(100, 5000);
    crate::config::save_vad_silence_ms(clamped);
    Ok(())
}

/// Whether adaptive VAD thresholding is enabled (default true).
#[tauri::command]
pub fn get_vad_adaptive() -> bool {
    crate::config::load_vad_adaptive()
}

#[tauri::command]
pub fn set_vad_adaptive(adaptive: bool) -> Result<(), String> {
    crate::config::save_vad_adaptive(adaptive);
    Ok(())
}

/// VAD brief-pause threshold in milliseconds (default 400).  Silence below this
/// keeps the pill in "Paused" but does not stop recording.
#[tauri::command]
pub fn get_vad_pause_ms() -> u32 {
    crate::config::load_vad_pause_ms()
}

#[tauri::command]
pub fn set_vad_pause_ms(ms: u32) -> Result<(), String> {
    let clamped = ms.clamp(100, 2000);
    crate::config::save_vad_pause_ms(clamped);
    Ok(())
}

/// VAD minimum-speech duration in milliseconds (default 300).  Auto-stop is
/// suppressed until the recording has lasted at least this long.
#[tauri::command]
pub fn get_vad_min_speech_ms() -> u32 {
    crate::config::load_vad_min_speech_ms()
}

#[tauri::command]
pub fn set_vad_min_speech_ms(ms: u32) -> Result<(), String> {
    let clamped = ms.clamp(50, 2000);
    crate::config::save_vad_min_speech_ms(clamped);
    Ok(())
}

/// Re-inject (paste again) previously-transcribed text into the focused app.
/// Used by the History view's "Paste again" action.
#[tauri::command]
pub fn reinject_text(text: String) -> Result<(), String> {
    // 32 KB hard cap — anything beyond this almost certainly wasn't produced by
    // STT and is either a bug or an IPC abuse attempt.
    const MAX_PASTE_BYTES: usize = 32 * 1024;
    if text.len() > MAX_PASTE_BYTES {
        return Err(format!(
            "text too large to paste ({} bytes; max {MAX_PASTE_BYTES})",
            text.len()
        ));
    }
    crate::inject::inject(&text).map_err(|e| e.to_string())
}

/// Text-formatting options the transcription pipeline applies before pasting.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatSettings {
    pub auto_capitalize: bool,
    pub append_space: bool,
    pub paste_uppercase: bool,
}

/// Read the persisted formatting options (so Settings reflects real state).
#[tauri::command]
pub fn get_format_options() -> FormatSettings {
    FormatSettings {
        auto_capitalize: crate::config::load_bool("capitalize_first", true),
        append_space: crate::config::load_bool("append_space", true),
        paste_uppercase: crate::config::load_bool("paste_uppercase", false),
    }
}

/// Persist the formatting options. The hotkey + tray transcription paths read
/// these on every transcription, so toggling them takes effect immediately.
#[tauri::command]
pub fn set_format_options(
    auto_capitalize: bool,
    append_space: bool,
    paste_uppercase: bool,
) -> Result<(), String> {
    crate::config::save_bool("capitalize_first", auto_capitalize);
    crate::config::save_bool("append_space", append_space);
    crate::config::save_bool("paste_uppercase", paste_uppercase);
    Ok(())
}

/// Open an https URL in the user's default browser. Used by the About
/// "Project page" link so it doesn't navigate the app's own webview away.
///
/// Security: HTTPS-only, and the host must not resolve to a private/loopback
/// address. `cmd /C start "" "<url>"` on Windows happily launches any
/// registered URL handler (mailto:, file:, ms-settings:, javascript:, etc.),
/// so we tighten the validator before handing the string to the OS.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") {
        return Err("only https:// URLs are allowed".into());
    }
    let host = trimmed
        .strip_prefix("https://")
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.split(':').next())
        .unwrap_or("");
    if host.is_empty() {
        return Err("URL is missing a host".into());
    }
    if is_blocked_host(host) {
        return Err(format!("host '{host}' is not allowed (private/loopback)"));
    }
    launch_external(trimmed);
    Ok(())
}

/// Shared launch helper used by both the IPC command and the tray "Help" item.
/// The tray passes a known-safe constant; the IPC command path validates first.
pub(crate) fn launch_external(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Reject obvious private/loopback hostnames. Cheap, conservative check — the
/// alternative is a full DNS resolve at IPC time, which is overkill for a
/// "Project page" link. Callers must check this before launching any URL.
pub(crate) fn is_blocked_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h == "127.0.0.1"
        || h == "::1"
        || h == "0.0.0.0"
        || h.starts_with("127.")
        || h.starts_with("10.")
        || h.starts_with("192.168.")
        || h.starts_with("169.254.")
        || (h.starts_with("172.") && h.split('.').nth(1).and_then(|s| s.parse::<u8>().ok()).map(|n| (16..=31).contains(&n)).unwrap_or(false))
        || h.starts_with("fc")
        || h.starts_with("fd")
        || h.starts_with("fe80:")
}

/// Transcribe a buffer of 16 kHz mono PCM samples.
///
/// Pipeline:
///   1. STT (Whisper) -> raw text.
///   2. If `format` is true (default), run the formatter (trim, dict
///      replace, capitalize first, append trailing space).
///   3. Insert the formatted text into the history DB with
///      `source = "ptt"` and the audio duration in milliseconds.
///   4. Return the (possibly formatted) text.
///
/// The `format` flag defaults to true so the existing App.svelte wiring
/// keeps working unchanged — it just calls `transcribe_buffer({ samples })`.
#[tauri::command]
pub async fn transcribe_buffer(
    samples: Vec<f32>,
    format: Option<bool>,
    auto_cap: Option<bool>,
    append_space: Option<bool>,
    paste_uppercase: Option<bool>,
) -> Result<String, String> {
    // 30 s @ 16 kHz mono is the largest sensible PTT utterance. Beyond that
    // it's almost certainly an IPC abuse / DoS attempt.
    const MAX_SAMPLES: usize = 30 * 16_000;
    if samples.len() > MAX_SAMPLES {
        return Err(format!(
            "audio buffer too large ({} samples; max {MAX_SAMPLES})",
            samples.len()
        ));
    }
    let start_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let raw = crate::stt::transcribe_pcm_16k(&samples)
        .await
        .map_err(|e| e.to_string())?;

    let end_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let duration_ms = if start_ms > 0 && end_ms >= start_ms {
        Some(end_ms - start_ms)
    } else {
        None
    };

		    let should_format = format.unwrap_or(true);
		    let out = if should_format {
		        // Use the cached dictionary (`list()`) instead of `load()` to
		        // avoid a disk read on every transcription. The cache is
		        // populated at startup and kept in sync by add/remove.
		        let pairs = dictionary::list()
		            .map(|entries| {
		                entries
		                    .into_iter()
		                    .map(|e| (e.from, e.to))
		                    .collect::<Vec<_>>()
		            })
		            .unwrap_or_default();
		        // Default to true; frontend can override via the next two optional args.
		        let auto_cap = auto_cap.unwrap_or(true);
		        let append = append_space.unwrap_or(true);
		        let uppercase = paste_uppercase.unwrap_or_else(|| crate::config::load_bool("paste_uppercase", false));
		        let opts = FormatOptions {
		            auto_capitalize: auto_cap,
		            append_space: append,
		            paste_uppercase: uppercase,
		            dict: pairs,
		        };
		        formatter::format_transcript(&raw, &opts)
		    } else {
		        raw.trim().to_string()
		    };

    if !out.is_empty() {
        if let Err(e) = history::insert(&out, duration_ms, Some("ptt")) {
            log::warn!("history insert failed: {e:?}");
        } else {
            // Keep history bounded: prune old rows now that we've just grown
            // it. Best-effort; a failure here must not lose the transcription.
            if let Err(e) = history::prune_if_needed() {
                log::warn!("history post-insert prune failed: {e:?}");
            }
        }
    }

    Ok(out)
}

/// Status of an STT model variant (active or listed).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub variant: String,
    /// Short UI label (e.g. "Base", "Distil-Large").
    pub display_name: String,
    /// One-line description for the model picker.
    pub description: String,
    pub ready: bool,
    pub path: String,
    pub file_size_mb: u64,
    pub expected_size_mb: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccelerationInfo {
    pub mode: String, // "auto" | "cpu" | "gpu"
    pub detected: String, // e.g. "cpu", "vulkan", "directml", "cuda"
    pub in_use: String,
}

#[tauri::command]
pub fn get_acceleration_info() -> AccelerationInfo {
    let (mode, detected, in_use) = probe_acceleration();
    AccelerationInfo { mode, detected, in_use }
}

#[tauri::command]
pub fn set_acceleration_mode(mode: String) -> Result<(), String> {
    if !["auto", "cpu", "gpu"].contains(&mode.as_str()) {
        return Err("acceleration_mode must be one of: auto, cpu, gpu".to_string());
    }
    crate::config::save_string("acceleration_mode", &mode);
    // Moonshine sessions are provider-specific; reset them so the next
    // transcription rebuilds with the newly-selected acceleration mode.
    #[cfg(feature = "moonshine")]
    {
        crate::stt::moonshine::reset();
    }
    Ok(())
}

// Helper used by moonshine loader and probe (avoids duplication).
pub(crate) fn get_acceleration_mode() -> String {
    crate::config::load_string("acceleration_mode").unwrap_or_else(|| "auto".to_string())
}

/// Runtime probe (task spec):
/// - try ort providers if moonshine feature
/// - env (DEVWHISP_ACCEL, WGPU_*, ONNX_*)
/// - simple wgpu or cpal hints
/// - whisper feature state (cuda/vulkan)
fn probe_acceleration() -> (String, String, String) {
    let mode = get_acceleration_mode();

    let mut detected = "cpu".to_string();

    // whisper feature state (baked at build)
    let whisper_gpu = cfg!(feature = "cuda") || cfg!(feature = "vulkan");
    if whisper_gpu {
        detected = "gpu (whisper build)".to_string();
    }

    // env hints
    if let Ok(v) = std::env::var("DEVWHISP_ACCEL") {
        let vl = v.to_lowercase();
        if vl.contains("gpu") {
            if detected == "cpu" { detected = "gpu (env)".to_string(); }
        } else if vl.contains("cpu") {
            detected = "cpu (env)".to_string();
        }
    }
    if std::env::var("WGPU_BACKEND").is_ok() || std::env::var("ONNX_GPU").is_ok() {
        if detected == "cpu" { detected = "gpu (hint)".to_string(); }
    }

    // ort providers probe when moonshine enabled (uses current ort config path)
    #[cfg(feature = "moonshine")]
    {
        use ort::ep;
        use ort::ep::ExecutionProvider;
        if ep::CUDA::default().is_available().unwrap_or(false) {
            detected = "cuda".to_string();
        } else if cfg!(target_os = "windows") && ep::DirectML::default().is_available().unwrap_or(false) {
            detected = "directml".to_string();
        } else if ep::WebGPU::default().is_available().unwrap_or(false) {
            detected = "webgpu".to_string();
        }
    }

    // decide in_use
    let in_use = match mode.as_str() {
        "cpu" => "cpu".to_string(),
        "gpu" => {
            if detected != "cpu" && !detected.starts_with("cpu") { detected.clone() } else { "cpu".to_string() }
        }
        _ => { // auto
            if detected != "cpu" && !detected.starts_with("cpu") {
                detected.clone()
            } else if whisper_gpu {
                detected.clone()
            } else {
                "cpu".to_string()
            }
        }
    };

    (mode, detected, in_use)
}

/// List available audio input devices (for top-tier audio control).
#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<String>, String> {
    #[allow(unused_imports)]
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let mut devices = vec![];
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(desc) = d.description() {
                devices.push(desc.name().to_string());
            } else {
                devices.push("Unknown device".to_string());
            }
        }
    }
    if devices.is_empty() {
        devices.push("Default".to_string());
    }
    Ok(devices)
}

/// Persisted selected audio input device name (or None for default).
#[tauri::command]
pub fn get_selected_audio_device() -> Option<String> {
    crate::config::load_string("audio_device")
}

/// Set (and persist) the selected input device name. Use "Default" or None-equivalent to reset.
#[tauri::command]
pub fn set_selected_audio_device(name: String) -> Result<(), String> {
    const MAX_DEVICE_NAME_BYTES: usize = 128;
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "Default" {
        // clear to use default (remove key or set special)
        crate::config::save_string("audio_device", "");
    } else if trimmed.len() > MAX_DEVICE_NAME_BYTES {
        return Err(format!(
            "device name too long ({} bytes; max {MAX_DEVICE_NAME_BYTES})",
            trimmed.len()
        ));
    } else {
        crate::config::save_string("audio_device", trimmed);
    }
    Ok(())
}

fn model_status_for(v: crate::stt::model_manager::ModelVariant) -> ModelStatus {
    use crate::stt::model_manager::models_root;
    let dir = models_root().map(|r| r.join(v.as_str()));
    let expected = v.expected_size_mb();
    let (file_size_mb, ready) = if let Some(d) = &dir {
        let f = v.model_file(d);
        if f.is_file() {
            if let Ok(m) = std::fs::metadata(&f) {
                let mb = m.len() / 1_000_000;
                // Within ~15% of expected size (or at least 1 MB for tiny models).
                let tol = (expected.max(10) as i64 * 15) / 100;
                let ready = mb > 0 && (mb as i64 - expected as i64).abs() <= tol.max(5);
                (mb, ready)
            } else {
                (0, false)
            }
        } else {
            (0, false)
        }
    } else {
        (0, false)
    };
    ModelStatus {
        variant: v.as_str().to_string(),
        display_name: v.display_name().to_string(),
        description: v.description().to_string(),
        ready,
        path: dir
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default(),
        file_size_mb,
        expected_size_mb: expected,
    }
}

#[tauri::command]
pub fn get_model_status() -> ModelStatus {
    use crate::stt::model_manager::ModelVariant;
    if let Some(v) = crate::stt::model_manager::active_variant() {
        return model_status_for(v);
    }
    // No active model yet — return an empty status (UI prompts download).
    ModelStatus {
        variant: String::new(),
        display_name: String::new(),
        description: String::new(),
        ready: false,
        path: String::new(),
        file_size_mb: 0,
        expected_size_mb: ModelVariant::WhisperBaseEn.expected_size_mb(),
    }
}

/// Download a model variant by name. Returns the path to the model dir.
///
/// BridgeVoice-style Whisper ladder:
/// `whisper-tiny-en`, `whisper-base-en`, `whisper-small-en`,
/// `whisper-medium-en`, `whisper-large-v3`, `whisper-distil-large-v3`,
/// plus optional `moonshine-tiny`.
///
/// Emits "model-download-progress" events with
/// `{ variant, pct, downloadedMB, totalMB }`.
#[tauri::command]
pub async fn download_model(app: tauri::AppHandle, variant: String) -> Result<String, String> {
    use crate::stt::model_manager::ModelVariant;
    let v = ModelVariant::from_id(&variant)
        .ok_or_else(|| format!("unknown model variant: {variant}"))?;

    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({
            "variant": variant,
            "pct": 0,
            "downloadedMB": 0,
            "totalMB": v.approx_size_mb()
        }),
    );

    let total_mb = v.approx_size_mb() as u64;
    let download_res = crate::stt::model_manager::download(v, Some(app.clone())).await;
    if let Err(ref e) = download_res {
        let _ = app.emit(
            "model-download-progress",
            serde_json::json!({
                "variant": variant,
                "pct": 0,
                "downloadedMB": 0,
                "totalMB": total_mb,
                "error": e.to_string()
            }),
        );
    }
    let path: PathBuf = download_res.map_err(|e| e.to_string())?;

    // Model switch: drop any previously loaded whisper context.
    crate::stt::whisper::reset();
    #[cfg(feature = "moonshine")]
    {
        crate::stt::moonshine::reset();
    }

    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({
            "variant": variant,
            "pct": 100,
            "downloadedMB": total_mb,
            "totalMB": total_mb
        }),
    );

    Ok(path.to_string_lossy().to_string())
}

/// Switch the active model among already downloaded variants.
/// Updates active.txt and resets in-memory STT contexts so the change takes
/// effect on the next transcription (no redownload).
#[tauri::command]
pub fn set_active_model(variant: String) -> Result<(), String> {
    use crate::stt::model_manager::ModelVariant;
    let v = ModelVariant::from_id(&variant)
        .ok_or_else(|| format!("unknown model variant: {variant}"))?;

    // Refuse to activate a model that isn't on disk yet.
    let status = model_status_for(v);
    if !status.ready {
        return Err(format!(
            "model '{}' is not downloaded yet — download it first",
            v.display_name()
        ));
    }

    crate::stt::model_manager::set_active(v).map_err(|e| e.to_string())?;

    crate::stt::whisper::reset();
    #[cfg(feature = "moonshine")]
    {
        crate::stt::moonshine::reset();
    }
    Ok(())
}

/// Return status for every known model so the UI can show a BridgeVoice-style
/// picker without affecting the currently active one.
#[tauri::command]
pub fn list_model_statuses() -> Result<Vec<ModelStatus>, String> {
    use crate::stt::model_manager::ModelVariant;
    Ok(ModelVariant::ALL.iter().map(|v| model_status_for(*v)).collect())
}

// ---------- History IPC ---------------------------------------------------
//
// The 7 IPC commands below are referenced exclusively from
// `tauri::generate_handler![...]` in `lib.rs`. Rust's `dead_code` lint
// doesn't see through that macro, so we silence it here. Without this
// attribute the lib still builds but emits 8 spurious dead-code
// warnings.
#[allow(dead_code)]

/// History entry delivered to the frontend. Alias of `TranscriptionRow`;
/// kept separate so we can reshape the wire format independently.
pub type HistoryEntry = TranscriptionRow;

/// List transcriptions newest-first. `limit` defaults to 50, `offset`
/// to 0 when omitted.
#[tauri::command]
pub fn list_history(
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<HistoryEntry>, String> {
    history::list(limit.unwrap_or(50), offset.unwrap_or(0))
        .map_err(|e| e.to_string())
}

/// Case-insensitive substring search over transcription text.
#[tauri::command]
pub fn search_history(
    query: String,
    limit: Option<i64>,
) -> Result<Vec<HistoryEntry>, String> {
    history::search(&query, limit.unwrap_or(50)).map_err(|e| e.to_string())
}

/// Delete one row. Returns true if a row was removed.
#[tauri::command]
pub fn delete_history_entry(id: i64) -> Result<bool, String> {
    history::delete(id).map_err(|e| e.to_string())
}

/// Wipe the history. Returns the number of rows removed.
#[tauri::command]
pub fn clear_history() -> Result<i64, String> {
    history::clear().map_err(|e| e.to_string())
}

/// History auto-prune retention window in days.
///
/// - `Some(n)` for `n >= 1` → rows older than `n` days are auto-deleted.
/// - `None` or `0` → "Never" (auto-prune disabled).
///
/// When the key is unset in settings (fresh install), the backend returns
/// `None` and the frontend surfaces its default (2 days).
#[tauri::command]
pub fn get_history_retention_days() -> Option<u32> {
    crate::config::load_history_retention_days()
}

/// Persist the history retention window. `None` or `0` disables auto-prune.
#[tauri::command]
pub fn set_history_retention_days(days: Option<u32>) -> Result<(), String> {
    let n = match days {
        None => 0,
        Some(0) => 0,
        Some(v) => {
            // Reject nonsensical windows: keep a hard cap so a typo (or an IPC
            // abuse attempt) can't pin the DB to "keep everything for 10 years".
            if !(1..=365).contains(&v) {
                return Err(format!(
                    "history_retention_days must be between 1 and 365 (or 0/None to disable); got {v}"
                ));
            }
            v
        }
    };
    crate::config::save_history_retention_days(n);
    Ok(())
}

// ---------- Dictionary IPC ------------------------------------------------

/// Return the current dictionary.
#[tauri::command]
pub fn get_dictionary() -> Result<Vec<DictEntry>, String> {
    dictionary::list().map_err(|e| e.to_string())
}

/// Add or update a `(from, to)` entry. Returns the new full list.
#[tauri::command]
pub fn add_dictionary_entry(from: String, to: String) -> Result<Vec<DictEntry>, String> {
    dictionary::add(&from, &to).map_err(|e| e.to_string())
}

/// Remove the entry whose `from` matches. Returns the new full list.
#[tauri::command]
pub fn remove_dictionary_entry(from: String) -> Result<Vec<DictEntry>, String> {
    dictionary::remove(&from).map_err(|e| e.to_string())
}

// ---------- Hotkey IPC ---------------------------------------------------

/// Canonical display form of the currently registered hotkey, e.g.
/// `"Ctrl+Shift+Space"`. Always returns a valid string — falls back to
/// the default if the persisted value is unparseable.
#[tauri::command]
pub fn get_hotkey() -> String {
    crate::hotkey::current_shortcut_string()
}

/// Set the global hotkey from a spec like `"F8"` or `"Ctrl+Alt+Space"`.
/// On success returns the canonical display form. On failure the existing
/// binding is preserved and an error message is returned.
#[tauri::command]
pub fn set_hotkey(
    app: tauri::AppHandle,
    spec: String,
) -> Result<String, String> {
    // Clamp spec length to avoid abuse. 64 chars is more than enough for any
    // realistic hotkey (longest realistic: "Ctrl+Shift+Alt+Meta+F24" = 27 chars).
    if spec.len() > 64 {
        return Err(format!("hotkey spec too long ({} chars; max 64)", spec.len()));
    }
    crate::hotkey::set_shortcut(&app, &spec).map_err(|e| e.to_string())
}

/// One predefined hotkey the user can pick from in the Settings UI.
/// `spec` is the parseable form (e.g. "Ctrl+Shift+Space"); `label` is the
/// display form for the picker (e.g. "Ctrl + Shift + Space").
#[derive(Debug, Clone, Serialize)]
pub struct PredefinedHotkey {
    pub spec: String,
    pub label: String,
    pub description: String,
}

/// Return the curated list of predefined hotkeys. We keep this list short
/// and hand-picked — these are the bindings that almost always work, rarely
/// conflict with other apps, and cover the common modes (function keys,
/// Ctrl-combos, single modifier + Space/Fn). Free-form text input was
/// removed in 0.1.3 because the underlying `parse_key` was incorrectly
/// mapping every single character to `KeyA`, so a user typing `Ctrl+G`
/// would end up with `Ctrl+A` registered.
#[tauri::command]
pub fn list_predefined_hotkeys() -> Vec<PredefinedHotkey> {
    use crate::hotkey::canonicalize;
    fn item(spec: &str, description: &str) -> PredefinedHotkey {
        PredefinedHotkey {
            spec: spec.to_string(),
            label: canonicalize(spec),
            description: description.to_string(),
        }
    }
    vec![
        item("Ctrl+Shift+Space", "Recommended — almost never conflicts."),
        item("Ctrl+Space",        "Quick — two-key combo."),
        item("Alt+Space",         "Classic — same combo as many window managers."),
        item("Ctrl+Alt+Space",    "Heavy — hard to press by accident."),
        item("Ctrl+Shift+F8",     "Hard to conflict — function keys are rarely hotkeys."),
        item("Ctrl+Shift+F9",     "Hard to conflict — function keys are rarely hotkeys."),
        item("Ctrl+Shift+F10",    "Hard to conflict — function keys are rarely hotkeys."),
        item("Ctrl+Shift+F11",    "Hard to conflict — function keys are rarely hotkeys."),
        item("Ctrl+Shift+F12",    "Hard to conflict — function keys are rarely hotkeys."),
        item("F8",                "Bare function key — may conflict with some apps."),
        item("F9",                "Bare function key — may conflict with some apps."),
        item("F10",               "Bare function key — may conflict with some apps."),
    ]
}