//! Global push-to-talk hotkey handler.
//!
//! Phase 1 (T1.5 / T2.6) wires:
//!   - On press: start audio capture.
//!   - On release: stop capture, transcribe, inject via clipboard paste.
//!
//! The hook also drives the pill widget's state machine via Tauri events.
//! Every transition (idle → listening → processing → success/error → idle)
//! emits a `pill-state` event on the "pill" window so the floating widget
//! can re-render accordingly. Errors during transcription are surfaced as
//! `pill-state` "error" so the pill can show the message inline.
//!
//! ## Hotkey rebinding
//!
//! The shortcut spec is persisted via `crate::config` and parsed at startup
//! (`register_initial`). Users can re-bind at runtime through the Settings
//! hotkey text field (`set_shortcut` IPC). The currently-active spec is
//! also reachable from the front-end via `current_shortcut_string()`.

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Current recording mode, lazily initialized from persisted settings.
///   - `"push-to-talk"`: hold the hotkey to record; release stops + transcribes.
///   - `"toggle"`: first press starts recording, the next press stops it.
///   - `"vad"`: press starts capture; auto-stops after silence hold-off and transcribes.
static MODE: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(crate::config::load_mode()));

/// Canonical display string for the currently registered shortcut, e.g.
/// `"Ctrl+Shift+Space"`. Updated whenever `set_shortcut` succeeds so the
/// tray / dashboard / onboarding all read the same value.
static CURRENT_SPEC: Lazy<RwLock<String>> = Lazy::new(|| {
    RwLock::new(crate::config::load_hotkey())
});

/// The actual `Shortcut` struct currently registered with the OS. Kept in a
/// `Mutex` so `set_shortcut` can swap it atomically.
static CURRENT_SHORTCUT: Lazy<Mutex<Shortcut>> = Lazy::new(|| {
    let spec = crate::config::load_hotkey();
    Mutex::new(
        parse_shortcut(&spec).unwrap_or_else(|_| {
            // Fall back to the historical default if parsing fails — better
            // than panicking at startup.
            parse_shortcut("Ctrl+Shift+Space").expect("default hotkey must parse")
        }),
    )
});

/// Whether we have ever successfully performed an OS-level registration.
/// Used to avoid spurious "unregister" warnings on the very first startup
/// (the Lazy CURRENT_SHORTCUT has a struct but no OS binding exists yet).
static EVER_REGISTERED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// Shared, long-lived tokio runtime used for STT calls from synchronous
/// threads (the transcription worker spawned by `on_release`). Building a
/// fresh runtime per transcription paid ~5-15 ms of construction overhead
/// for every press-and-release cycle.
static TRANSCRIBE_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("DevWhisp: failed to build shared transcription tokio runtime")
});

/// Set the active recording mode and persist it. Accepts "toggle", "vad", or
/// "push-to-talk" (case-insensitive); anything else normalizes to push-to-talk.
pub fn set_mode(mode: &str) {
    let normalized = if mode.eq_ignore_ascii_case("toggle") {
        "toggle"
    } else if mode.eq_ignore_ascii_case("vad")
        || mode.eq_ignore_ascii_case("voice-activity")
        || mode.eq_ignore_ascii_case("auto")
    {
        "vad"
    } else {
        "push-to-talk"
    };
    *MODE.write() = normalized.to_string();
    crate::config::save_mode(normalized);
    log::info!("recording mode set to: {normalized}");
}

/// The active recording mode ("push-to-talk" | "toggle" | "vad").
pub fn get_mode() -> String {
    MODE.read().clone()
}

/// Canonical display form of the currently registered hotkey
/// (e.g. `"Ctrl+Shift+Space"`).
pub fn current_shortcut_string() -> String {
    CURRENT_SPEC.read().clone()
}

/// Run an async transcription future on the shared tokio runtime. Cheap;
/// reuses one Runtime for the whole app lifetime.
pub fn block_on_transcribe<F>(fut: F) -> F::Output
where
    F: std::future::Future,
{
    TRANSCRIBE_RUNTIME.block_on(fut)
}

/// Register the user's persisted shortcut at app startup. Must be called
/// after `app.global_shortcut()` is available (i.e. inside the setup hook).
pub fn register_initial<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let spec = crate::config::load_hotkey();
    let parsed = parse_shortcut(&spec)?;
    register_parsed(app, parsed, &spec)?;
    *CURRENT_SPEC.write() = spec;
    Ok(())
}

/// Swap the registered shortcut at runtime. Parses the spec, unregisters
/// the old shortcut, registers the new one, and persists the spec.
/// Returns the canonical display form on success. On parse failure the
/// existing registration is preserved.
pub fn set_shortcut<R: Runtime>(app: &AppHandle<R>, spec: &str) -> Result<String> {
    let parsed = parse_shortcut(spec)?;
    register_parsed(app, parsed, spec)?;
    *CURRENT_SPEC.write() = spec.to_string();
    crate::config::save_hotkey(spec);
    Ok(canonicalize(spec))
}

fn register_parsed<R: Runtime>(app: &AppHandle<R>, new: Shortcut, spec: &str) -> Result<()> {
    let old = *CURRENT_SHORTCUT.lock();
    // Unregister the previous only if we actually registered something before.
    // On the absolute first call (startup) the Lazy holds a struct but the OS
    // has nothing yet — attempting unregister would just warn.
    if *EVER_REGISTERED.lock() {
        if let Err(e) = app.global_shortcut().unregister(old) {
            log::warn!("failed to unregister previous hotkey: {e}");
        }
    }
    app.global_shortcut()
        .on_shortcut(new, move |_app, _scut, event| {
            let app_handle = _app.clone();
            let pressed = event.state == ShortcutState::Pressed;
            log::info!("hotkey {}", if pressed { "pressed" } else { "released" });
            if let Err(e) = on_hotkey(&app_handle, pressed) {
                log::error!("hotkey handler error: {e:?}");
            }
        })
        .map_err(|e| anyhow!("failed to register hotkey '{spec}': {e}"))?;
    *CURRENT_SHORTCUT.lock() = new;
    *EVER_REGISTERED.lock() = true;
    log::info!("hotkey registered: {spec}");
    Ok(())
}

/// Parse a shortcut spec string like `"Ctrl+Shift+Space"` into a `Shortcut`.
///
/// Recognized modifiers (case-insensitive, also accepts common aliases):
///   - Ctrl | Control
///   - Shift
///   - Alt  | Option
///   - Meta | Win | Cmd | Super
///
/// Recognized keys (case-insensitive):
///   - F1..F24, Space, Enter | Return, Tab, Escape | Esc, Backspace,
///     CapsLock, ScrollLock, Insert, Delete, Home, End, PageUp, PageDown,
///     ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Backquote (`)
///   - single character (A–Z, 0–9, punctuation)
pub fn parse_shortcut(spec: &str) -> Result<Shortcut> {
    let raw = spec.trim();
    if raw.is_empty() {
        return Err(anyhow!("hotkey is empty"));
    }
    let mut modifiers = Modifiers::empty();
    let mut key: Option<Code> = None;
    for tok in raw.split('+') {
        let t = tok.trim();
        if t.is_empty() {
            return Err(anyhow!("hotkey has an empty segment in '{spec}'"));
        }
        match normalize_modifier(t) {
            Some(m) => {
                modifiers |= m;
                continue;
            }
            None => {}
        }
        if key.is_some() {
            return Err(anyhow!(
                "hotkey '{spec}' has more than one non-modifier key (only one is allowed)"
            ));
        }
        key = Some(parse_key(t)?);
    }
    let code = key.ok_or_else(|| {
        anyhow!("hotkey '{spec}' has no non-modifier key (press a regular key like F8 or Space)")
    })?;
    Ok(Shortcut::new(Some(modifiers), code))
}

fn normalize_modifier(s: &str) -> Option<Modifiers> {
    match s.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifiers::CONTROL),
        "shift" => Some(Modifiers::SHIFT),
        "alt" | "option" => Some(Modifiers::ALT),
        "meta" | "win" | "cmd" | "super" => Some(Modifiers::META),
        _ => None,
    }
}

fn parse_key(s: &str) -> Result<Code> {
    // Function keys: F1..F24
    if let Some(rest) = s.to_ascii_lowercase().strip_prefix('f') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=24).contains(&n) {
                return Ok(function_key_code(n));
            }
        }
    }
    match s.to_ascii_lowercase().as_str() {
        "space" => Ok(Code::Space),
        "enter" | "return" => Ok(Code::Enter),
        "tab" => Ok(Code::Tab),
        "escape" | "esc" => Ok(Code::Escape),
        "backspace" => Ok(Code::Backspace),
        "delete" | "del" => Ok(Code::Delete),
        "insert" => Ok(Code::Insert),
        "home" => Ok(Code::Home),
        "end" => Ok(Code::End),
        "pageup" => Ok(Code::PageUp),
        "pagedown" => Ok(Code::PageDown),
        "arrowup" | "up" => Ok(Code::ArrowUp),
        "arrowdown" | "down" => Ok(Code::ArrowDown),
        "arrowleft" | "left" => Ok(Code::ArrowLeft),
        "arrowright" | "right" => Ok(Code::ArrowRight),
        "capslock" | "caps" => Ok(Code::CapsLock),
        "scrolllock" => Ok(Code::ScrollLock),
        "numlock" => Ok(Code::NumLock),
        "printscreen" => Ok(Code::PrintScreen),
        "pause" | "break" => Ok(Code::Pause),
        "backquote" | "backtick" | "`" => Ok(Code::Backquote),
        _ => {
            // Single-character keys: A–Z, 0–9, common punctuation.
            // CRITICAL: we must return the actual `Code` for the typed
            // character — previously this returned `Code::KeyA` as a
            // placeholder and was never rewritten, so `Ctrl+G`,
            // `Ctrl+Shift+B`, etc. all silently registered as `Ctrl+A`.
            // See https://github.com/AEmad99/DevWhisp — 0.1.2 hotkey bug.
            let bytes = s.as_bytes();
            if bytes.len() == 1 {
                let c = bytes[0];
                if c.is_ascii_alphabetic() || c.is_ascii_digit() || c.is_ascii_punctuation() {
                    return Ok(letter_to_code(c));
                }
            }
            Err(anyhow!("unknown key '{s}' in hotkey"))
        }
    }
}

/// Map a single ASCII character to the corresponding `tauri_plugin_global_shortcut::Code`.
/// Letters A–Z map to `KeyA..KeyZ`, digits 0–9 map to `Digit0..Digit9`,
/// and a curated set of punctuation marks map to their named variants.
fn letter_to_code(c: u8) -> Code {
    match c {
        b'A' | b'a' => Code::KeyA,
        b'B' | b'b' => Code::KeyB,
        b'C' | b'c' => Code::KeyC,
        b'D' | b'd' => Code::KeyD,
        b'E' | b'e' => Code::KeyE,
        b'F' | b'f' => Code::KeyF,
        b'G' | b'g' => Code::KeyG,
        b'H' | b'h' => Code::KeyH,
        b'I' | b'i' => Code::KeyI,
        b'J' | b'j' => Code::KeyJ,
        b'K' | b'k' => Code::KeyK,
        b'L' | b'l' => Code::KeyL,
        b'M' | b'm' => Code::KeyM,
        b'N' | b'n' => Code::KeyN,
        b'O' | b'o' => Code::KeyO,
        b'P' | b'p' => Code::KeyP,
        b'Q' | b'q' => Code::KeyQ,
        b'R' | b'r' => Code::KeyR,
        b'S' | b's' => Code::KeyS,
        b'T' | b't' => Code::KeyT,
        b'U' | b'u' => Code::KeyU,
        b'V' | b'v' => Code::KeyV,
        b'W' | b'w' => Code::KeyW,
        b'X' | b'x' => Code::KeyX,
        b'Y' | b'y' => Code::KeyY,
        b'Z' | b'z' => Code::KeyZ,
        b'0' => Code::Digit0,
        b'1' => Code::Digit1,
        b'2' => Code::Digit2,
        b'3' => Code::Digit3,
        b'4' => Code::Digit4,
        b'5' => Code::Digit5,
        b'6' => Code::Digit6,
        b'7' => Code::Digit7,
        b'8' => Code::Digit8,
        b'9' => Code::Digit9,
        b'`' => Code::Backquote,
        b'-' => Code::Minus,
        b'=' => Code::Equal,
        b'[' => Code::BracketLeft,
        b']' => Code::BracketRight,
        b'\\' => Code::Backslash,
        b';' => Code::Semicolon,
        b'\'' => Code::Quote,
        b',' => Code::Comma,
        b'.' => Code::Period,
        b'/' => Code::Slash,
        _ => Code::KeyA, // unreachable — caller already filtered
    }
}

/// Map F1..F24 to the right `Code` variant. `tauri_plugin_global_shortcut`
/// exposes F1..F12 directly; F13..F24 use the `F13`..`F24` variants.
fn function_key_code(n: u8) -> Code {
    match n {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        13 => Code::F13,
        14 => Code::F14,
        15 => Code::F15,
        16 => Code::F16,
        17 => Code::F17,
        18 => Code::F18,
        19 => Code::F19,
        20 => Code::F20,
        21 => Code::F21,
        22 => Code::F22,
        23 => Code::F23,
        _ => Code::F24,
    }
}

/// Canonicalize a hotkey spec for display: title-case modifiers, preserve
/// the user's letter-case for keys. Always returns something like
/// `"Ctrl+Shift+Space"`.
pub fn canonicalize(spec: &str) -> String {
    let mut out = String::new();
    for tok in spec.split('+') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('+');
        }
        match normalize_modifier(t) {
            Some(m) => {
                out.push_str(modifier_label(m));
            }
            None => {
                // Preserve original case for single-character keys; title-case
                // for named keys.
                if t.len() == 1 {
                    out.push_str(t);
                } else if t.eq_ignore_ascii_case("space") {
                    out.push_str("Space");
                } else if t.eq_ignore_ascii_case("escape") || t.eq_ignore_ascii_case("esc") {
                    out.push_str("Escape");
                } else if t.eq_ignore_ascii_case("enter") || t.eq_ignore_ascii_case("return") {
                    out.push_str("Enter");
                } else if t.eq_ignore_ascii_case("tab") {
                    out.push_str("Tab");
                } else if t.eq_ignore_ascii_case("backspace") {
                    out.push_str("Backspace");
                } else if t.eq_ignore_ascii_case("delete") || t.eq_ignore_ascii_case("del") {
                    out.push_str("Delete");
                } else if t.eq_ignore_ascii_case("insert") {
                    out.push_str("Insert");
                } else if t.eq_ignore_ascii_case("home") {
                    out.push_str("Home");
                } else if t.eq_ignore_ascii_case("end") {
                    out.push_str("End");
                } else if t.eq_ignore_ascii_case("pageup") {
                    out.push_str("PageUp");
                } else if t.eq_ignore_ascii_case("pagedown") {
                    out.push_str("PageDown");
                } else if t.eq_ignore_ascii_case("capslock") || t.eq_ignore_ascii_case("caps") {
                    out.push_str("CapsLock");
                } else if t.eq_ignore_ascii_case("scrolllock") {
                    out.push_str("ScrollLock");
                } else {
                    out.push_str(t);
                }
            }
        }
    }
    out
}

fn modifier_label(m: Modifiers) -> &'static str {
    if m.contains(Modifiers::CONTROL) {
        "Ctrl"
    } else if m.contains(Modifiers::SHIFT) {
        "Shift"
    } else if m.contains(Modifiers::ALT) {
        "Alt"
    } else if m.contains(Modifiers::META) {
        "Meta"
    } else {
        ""
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

/// Physical key press state flag to suppress OS auto-repeat events.
static HOTKEY_DOWN: AtomicBool = AtomicBool::new(false);

/// Unified hotkey dispatch honoring the configured mode. In push-to-talk the
/// press starts and the release stops; in toggle the press flips recording
/// on/off and key-releases are ignored.
/// In "vad": press starts capture (auto VAD monitor ends on silence); releases
/// and extra presses while active are ignored (manual stop via tray/keyboard
/// still works).
pub fn on_hotkey<R: Runtime>(app: &AppHandle<R>, pressed: bool) -> Result<()> {
    if pressed {
        // Suppress OS key auto-repeat events while the shortcut key is held down
        if HOTKEY_DOWN.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
    } else {
        HOTKEY_DOWN.store(false, Ordering::SeqCst);
    }

    let mode = MODE.read().clone();
    if mode.eq_ignore_ascii_case("toggle") {
        if pressed {
            if crate::audio::is_active() {
                return on_release(app);
            }
            return on_press(app);
        }
        // Ignore key-up in toggle mode.
        Ok(())
    } else if mode.eq_ignore_ascii_case("vad") {
        if pressed {
            if crate::audio::is_active() {
                // Allow re-press to force early stop in VAD mode (convenience).
                return on_release(app);
            }
            let r = on_press(app);
            spawn_vad_monitor(app.clone());
            return r;
        }
        // Ignore releases in VAD (auto handles stop).
        Ok(())
    } else if pressed {
        on_press(app)
    } else {
        on_release(app)
    }
}

/// State payload sent to the pill window.
#[derive(Debug, Clone, Serialize)]
struct PillState {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// Payload for live (or final) partial transcripts shown in the pill.


fn emit_state<R: Runtime>(app: &AppHandle<R>, state: &'static str, message: Option<String>) {
    let payload = PillState { state, message };
    // emit_to targets only the "pill" window. Fall back to a global emit if
    // the window isn't registered yet — keeps first-launch behavior sane
    // (the pill window listens on this same event name).
    if let Some(window) = app.get_webview_window(crate::window::pill_window::PILL_LABEL) {
        if let Err(e) = window.emit("pill-state", payload) {
            log::warn!("failed to emit pill-state to pill window: {e}");
        }
    } else if let Err(e) = app.emit("pill-state", payload) {
        log::warn!("failed to emit pill-state globally: {e}");
    }
}



/// Called when the push-to-talk hotkey is first pressed down.
pub fn on_press<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    emit_state(app, "listening", None);
    crate::audio::start()?;
    Ok(())
}

/// Serializes the stop-capture → transcribe → inject sequence across the
/// hotkey release path, VAD auto-end, and tray stop. Prevents races where two
/// stop signals process the same audio buffer twice.
static STOP_TX_MUTEX: Mutex<()> = Mutex::new(());

/// Stop capture and run the transcription/inject path. Call this instead of
/// calling `stop_and_drain` + `transcribe_and_inject` directly.
pub fn stop_and_transcribe<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let _guard = STOP_TX_MUTEX.lock();
    let samples = crate::audio::stop_and_drain()?;
    transcribe_and_inject(app.clone(), samples);
    Ok(())
}

/// Called when the push-to-talk hotkey is released.
pub fn on_release<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    stop_and_transcribe(app)
}

/// Fast, stable fingerprint for a PCM buffer. Used to suppress duplicate
/// transcriptions when the same audio is stopped twice (race / duplicate event).
fn audio_fingerprint(samples: &[f32]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let n = samples.len();
    if n == 0 {
        return 0;
    }
    // Hash a sparse set of samples plus length. Exact match catches the
    // duplicate-buffer case without hashing the entire (potentially long) clip.
    let mut hasher = DefaultHasher::new();
    n.hash(&mut hasher);
    const SAMPLES: usize = 32;
    for i in 0..SAMPLES {
        let idx = (n * i) / SAMPLES;
        samples[idx].to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Core transcription + inject path shared by manual release (ptt/toggle) and
/// VAD auto-end. Emits states, runs STT off-thread, formats, history, inject.
/// Uses current mode to pick source tag ("vad" vs "ptt").
pub(crate) fn transcribe_and_inject<R: Runtime>(app: AppHandle<R>, samples: Vec<f32>) {
    if samples.is_empty() {
        emit_state(&app, "idle", None);
        return;
    }

    // Defensive dedupe: the same audio buffer should never be transcribed twice,
    // but races between manual stop, VAD stop, and duplicate shortcut events can
    // occasionally surface the same utterance twice. Skip if we saw this exact
    // audio fingerprint very recently.
    static LAST_AUDIO: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
    {
        let mut last = LAST_AUDIO.lock();
        let fp = audio_fingerprint(&samples);
        if let Some((prev_fp, when)) = &*last {
            if *prev_fp == fp && when.elapsed() < Duration::from_millis(1500) {
                log::info!("skipping duplicate transcription (same audio fingerprint)");
                emit_state(&app, "idle", None);
                return;
            }
        }
        *last = Some((fp, Instant::now()));
    }

    emit_state(&app, "processing", None);

    // Use the shared long-lived runtime instead of building a fresh one per
    // transcription. Saves the runtime construction cost (5-15 ms) on every
    // press-and-release.
    let app_for_thread = app.clone();
    std::thread::spawn(move || {
        let duration_ms = (samples.len() as i64 * 1000) / 16_000;
        let raw = block_on_transcribe(async {
            crate::stt::transcribe_pcm_16k(&samples).await
        });
        match raw {
            Ok(t) if !t.is_empty() => {
                // Use the cached dictionary (`list()`) — no disk I/O on the
                // hot path. The cache is populated once at startup and kept
                // in sync via dictionary::add / dictionary::remove.
                let pairs = crate::dictionary::list()
                    .map(|entries| {
                        entries
                            .into_iter()
                            .map(|e| (e.from, e.to))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let opts = crate::formatter::FormatOptions {
                    auto_capitalize: crate::config::load_bool("capitalize_first", true),
                    append_space: crate::config::load_bool("append_space", true),
                    paste_uppercase: crate::config::load_bool("paste_uppercase", false),
                    dict: pairs,
                };
                let formatted = crate::formatter::format_transcript(&t, &opts);
                let source = if crate::hotkey::get_mode().eq_ignore_ascii_case("vad") {
                    "vad"
                } else {
                    "ptt"
                };
                if let Err(e) = crate::history::insert(&formatted, Some(duration_ms), Some(source)) {
                    log::warn!("history insert failed: {e:?}");
                }

                // Auto-inject AFTER hiding the pill (if visible). This returns
                // focus to whatever the user last clicked so the paste lands in
                // the intended text field/app. The pill is re-shown briefly for
                // feedback. (No transcript text is ever rendered inside the pill.)
                if let Some(pw) = app_for_thread.get_webview_window(crate::window::pill_window::PILL_LABEL) {
                    let _ = pw.hide();
                }
                std::thread::sleep(std::time::Duration::from_millis(70));

                let pasted = crate::inject::inject(&formatted).is_ok();
                if !pasted {
                    log::error!("inject failed");
                    emit_state(
                        &app_for_thread,
                        "error",
                        Some("Copied to clipboard (couldn't auto-paste)".to_string()),
                    );
                }

                // Re-show pill for the brief success/error indicator feedback.
                if let Some(pw) = app_for_thread.get_webview_window(crate::window::pill_window::PILL_LABEL) {
                    let _ = pw.show();
                }

                if pasted {
                    emit_state(&app_for_thread, "success", None);
                    let app_for_reset = app_for_thread.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                        emit_state(&app_for_reset, "idle", None);
                    });
                } else {
                    // brief error, then idle
                    let app_for_reset = app_for_thread.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(1200));
                        emit_state(&app_for_reset, "idle", None);
                    });
                }
            }
            Ok(_) => {
                log::info!("transcription empty, nothing to inject");
                emit_state(&app_for_thread, "idle", None);
            }
            Err(e) => {
                log::error!("transcription failed: {e:?}");
                emit_state(
                    &app_for_thread,
                    "error",
                    Some(format!("transcription failed: {e}")),
                );
            }
        }
    });
}

/// Spawn the adaptive VAD background monitor.  Replaces the old threshold-only
/// loop with a two-tier pause-aware engine that keeps recording through brief
/// silences and only stops on true end-of-utterance silence.
fn spawn_vad_monitor<R: Runtime>(app: AppHandle<R>) {
    use crate::audio::vad;
    let stop_flag = Arc::new(AtomicBool::new(false));
    vad::spawn_adaptive_vad_monitor(app, stop_flag);
}
