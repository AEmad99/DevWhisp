//! DevWhisp settings: read/write via tauri-plugin-store (KV).
//!
//! Phase 1 (T2.8) is a thin facade that wraps a single store file at
//! `~/.devwhisp/settings.json`. Real schema and per-key helpers land in
//! subsequent tasks.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub model: String,
    pub hotkey: String,
    pub mode: String,
    pub theme: String,
    pub language: String,
    pub auto_start: bool,
    pub capitalize_first: bool,
    pub append_space: bool,
    pub paste_uppercase: bool,
    pub dictionary: Vec<DictEntry>,
    pub vad_silence_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub spoken: String,
    pub replacement: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: "tiny".to_string(),
            hotkey: "Ctrl+Shift+Space".to_string(),
            mode: "push-to-talk".to_string(),
            theme: "dark".to_string(),
            language: "en".to_string(),
            auto_start: false,
            capitalize_first: true,
            append_space: false,
            paste_uppercase: false,
            dictionary: vec![
                DictEntry { spoken: "next js".into(), replacement: "Next.js".into() },
                DictEntry { spoken: "typescript".into(), replacement: "TypeScript".into() },
                DictEntry { spoken: "tauri".into(), replacement: "Tauri".into() },
            ],
            vad_silence_ms: 600,
        }
    }
}

/// Path to the persistent settings file.
pub fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".devwhisp")
        .join("settings.json")
}

/// Read the persisted recording mode ("push-to-talk" | "toggle").
/// Best-effort: returns the default ("push-to-talk") when the settings file
/// is missing or unreadable.
pub fn load_mode() -> String {
    match std::fs::read_to_string(settings_path()) {
        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get("mode").and_then(|m| m.as_str()).map(str::to_string))
            .unwrap_or_else(|| Settings::default().mode),
        Err(_) => Settings::default().mode,
    }
}

/// Persist the recording mode, merging into the existing settings file so the
/// other keys are preserved. Best-effort; errors are logged, never fatal.
pub fn save_mode(mode: &str) {
    save_value("mode", serde_json::Value::String(mode.to_string()));
}

/// Read persisted VAD silence hold-off (ms). Default 600.
pub fn load_vad_silence_ms() -> u32 {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("vad_silence_ms").and_then(|m| m.as_u64()).map(|u| u as u32))
        .unwrap_or(600)
}

/// Persist VAD silence hold-off ms.
pub fn save_vad_silence_ms(ms: u32) {
    save_value("vad_silence_ms", serde_json::Value::Number(ms.into()));
}

/// Read persisted VAD energy threshold (raw RMS). Default ~0.015.
pub fn load_vad_energy_threshold() -> f32 {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("vad_silence_threshold").and_then(|x| x.as_f64()).map(|f| f as f32))
        .unwrap_or(0.015)
}

/// Persist VAD energy (RMS) threshold.
pub fn save_vad_energy_threshold(th: f32) {
    if let Some(num) = serde_json::Number::from_f64(th as f64) {
        save_value("vad_silence_threshold", serde_json::Value::Number(num));
    }
}

/// Read the persisted hotkey spec (e.g. "Ctrl+Shift+Space"). Default
/// `"Ctrl+Shift+Space"` if missing/unreadable. The string is validated by
/// `crate::hotkey::parse_shortcut` before use; this function only returns it.
pub fn load_hotkey() -> String {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("hotkey").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| "Ctrl+Shift+Space".to_string())
}

/// Persist the hotkey spec verbatim. Caller is responsible for validation —
/// this just stores the string.
pub fn save_hotkey(spec: &str) {
    save_value("hotkey", serde_json::Value::String(spec.to_string()));
}

/// Read an unsigned integer setting from settings.json, defaulting when absent/unreadable.
pub fn load_u64(key: &str) -> Option<u64> {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get(key).and_then(|x| x.as_u64())
            .or_else(|| v.get(key).and_then(|x| x.as_i64()).map(|i| i as u64)))
}

/// Persist an unsigned integer setting.
pub fn save_u64(key: &str, value: u64) {
    save_value(key, serde_json::Value::Number(value.into()));
}

/// Read persisted adaptive-VAD flag (default true).
pub fn load_vad_adaptive() -> bool {
    load_bool("vad_adaptive", true)
}

/// Persist adaptive-VAD flag.
pub fn save_vad_adaptive(adaptive: bool) {
    save_bool("vad_adaptive", adaptive);
}

/// Read persisted brief-pause threshold (default 400 ms).
pub fn load_vad_pause_ms() -> u32 {
    load_u64("vad_pause_ms")
        .map(|v| v.clamp(100, 2000) as u32)
        .unwrap_or(400)
}

/// Persist brief-pause threshold.
pub fn save_vad_pause_ms(ms: u32) {
    save_u64("vad_pause_ms", ms.clamp(100, 2000) as u64);
}

/// Read persisted minimum-speech duration (default 300 ms).
pub fn load_vad_min_speech_ms() -> u32 {
    load_u64("vad_min_speech_ms")
        .map(|v| v.clamp(50, 2000) as u32)
        .unwrap_or(300)
}

/// Persist minimum-speech duration.
pub fn save_vad_min_speech_ms(ms: u32) {
    save_u64("vad_min_speech_ms", ms.clamp(50, 2000) as u64);
}

/// Read a boolean setting from settings.json, defaulting when absent/unreadable.
pub fn load_bool(key: &str, default: bool) -> bool {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get(key).and_then(serde_json::Value::as_bool))
        .unwrap_or(default)
}

/// Persist a boolean setting, merging into the existing settings file.
pub fn save_bool(key: &str, value: bool) {
    save_value(key, serde_json::Value::Bool(value));
}

pub fn load_string(key: &str) -> Option<String> {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get(key).and_then(|x| x.as_str().map(|s| s.to_string())))
}

pub fn save_string(key: &str, value: &str) {
    save_value(key, serde_json::Value::String(value.to_string()));
}

/// Settings key holding the history retention window in days. Absent or `0`
/// means "Never" (auto-prune disabled). Default for a fresh install is 2 days,
/// surfaced via `DEFAULT_HISTORY_RETENTION_DAYS` rather than written to disk,
/// so existing users' settings.json is never disturbed.
pub const HISTORY_RETENTION_DAYS_KEY: &str = "history_retention_days";

/// Retention (days) applied on a fresh install when the key is absent.
pub const DEFAULT_HISTORY_RETENTION_DAYS: u32 = 2;

/// Read the persisted history retention window in days.
///
/// Three states, kept distinct so callers can tell "fresh install" apart from
/// "user turned pruning off":
///   - **key absent** → `None`: caller falls back to
///     `DEFAULT_HISTORY_RETENTION_DAYS` (fresh install = 2 days).
///   - **key = 0** → `Some(0)`: explicitly disabled ("Never").
///   - **key = n ≥ 1** → `Some(n)`: keep `n` days.
pub fn load_history_retention_days() -> Option<u32> {
    let raw = std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(HISTORY_RETENTION_DAYS_KEY).and_then(|x| x.as_u64()))?;
    Some(raw as u32)
}

/// Persist the history retention window in days. Pass `0` to disable
/// auto-pruning ("Never"). Best-effort; errors are logged, never fatal.
pub fn save_history_retention_days(days: u32) {
    save_value(
        HISTORY_RETENTION_DAYS_KEY,
        serde_json::Value::Number((days as u64).into()),
    );
}

/// Merge a single key/value into settings.json (creating it from defaults if
/// absent or malformed). Best-effort; errors are logged, never fatal.
///
/// Writes are atomic: serialize → write to `<path>.tmp` → fsync → rename.
/// If the process dies mid-write, the next read still sees either the old
/// file or the new file — never a half-written corruption.
fn save_value(key: &str, value: serde_json::Value) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| {
            serde_json::to_value(Settings::default()).unwrap_or_else(|_| serde_json::json!({}))
        });
    if let Some(obj) = root.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
    let json = match serde_json::to_string_pretty(&root) {
        Ok(j) => j,
        Err(e) => {
            log::warn!("failed to serialize settings: {e}");
            return;
        }
    };
    atomic_write(&path, json.as_bytes());
}

/// Write bytes to `path` atomically: tmp file in the same directory → fsync
/// → rename over the target. Survives a mid-write crash without corrupting
/// the original file.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write;
    let parent = match path.parent() {
        Some(p) => p,
        None => {
            log::warn!("settings path has no parent dir; falling back to non-atomic write");
            let _ = std::fs::write(path, bytes);
            return;
        }
    };
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings")
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_result {
        log::warn!("atomic settings write to tmp failed: {e}");
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        log::warn!("atomic settings rename failed: {e}");
        let _ = std::fs::remove_file(&tmp);
    }
}
