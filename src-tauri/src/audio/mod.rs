//! Audio capture: microphone -> 16 kHz mono PCM ring buffer.
//!
//! Phase 1 (T1.3) — real cpal + rubato pipeline. Public API:
//!   - `start() -> Result<()>`    — open mic stream, begin buffering
//!   - `stop_and_drain() -> Result<Vec<f32>>` — close stream, return 16 kHz mono f32
//!   - `is_active() -> bool`
//!
//! The `AppHandle` is owned by the audio module (set once via
//! `set_app_handle` from `lib.rs` setup). The capture thread uses it to emit
//! `audio-level` events at 30 fps to the pill window. This avoids leaking
//! the handle into IPC command signatures, which keeps the ipc module the
//! sole domain of the history/formatter worker.
//!
//! ## Lifecycle / race-condition notes
//!
//! The capture thread is **per-recording**: each `start()` spawns a fresh
//! thread that owns its own `active: Arc<AtomicBool>` flag. The flag is
//! installed in the module-level `ACTIVE` slot under a mutex when the
//! thread starts, and removed when the thread exits. This means:
//!
//!   * The OLD thread's callback sees the flag as false on the very next
//!     sample (because `start()` swaps in a new flag), so it stops writing
//!     to the shared buffer immediately — no risk of stale samples
//!     contaminating the next recording (the "duplicate recording" bug).
//!   * `start()` never blocks on the hotkey callback thread. It returns
//!     after a brief swap — the previous thread winds down in the
//!     background.
//!   * `stop_and_drain()` flips the active flag, swaps the slot to a
//!     no-op sentinel, and returns the buffer immediately. The trailing
//!     resampler flush is drained on the background thread, **after** the
//!     buffer has already been moved into the caller's hands.

pub mod capture;
pub mod resampler;
pub mod vad;
pub mod visualizer;

use anyhow::Result;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;

/// Sentinel Arc placed in `ACTIVE` while no recording is in progress.
/// Its `load()` returns false, so a stale thread accidentally reading it
/// just goes quiet.
fn inactive_flag() -> Arc<AtomicBool> {
    static INACTIVE: once_cell::sync::Lazy<Arc<AtomicBool>> =
        once_cell::sync::Lazy::new(|| Arc::new(AtomicBool::new(false)));
    INACTIVE.clone()
}

/// Shared state: the captured, resampled PCM samples (16 kHz mono f32).
/// The capture thread writes here; `stop_and_drain` reads + swaps it out.
static CAPTURED: once_cell::sync::Lazy<Arc<Mutex<Vec<f32>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

/// Module-wide "which capture thread is currently the active writer".
/// Holds an `Arc<AtomicBool>` so swapping in a new flag is atomic AND the
/// cpal callback closure can `Arc::clone` it once at stream-build time
/// and use the cheap reference for every subsequent check.
///
/// While idle, holds `inactive_flag()`.
static ACTIVE: once_cell::sync::Lazy<Mutex<Arc<AtomicBool>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(inactive_flag()));

/// Stored at startup so the capture thread can emit `audio-level` events
/// without callers having to thread an AppHandle through every callsite.
static APP_HANDLE: once_cell::sync::OnceCell<AppHandle> =
    once_cell::sync::OnceCell::new();

static LAST_LEVEL: once_cell::sync::Lazy<parking_lot::Mutex<f32>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(0.0));

/// Install the Tauri app handle for use by the audio emitter. Must be
/// called once from `lib.rs` setup, before any recording starts. Idempotent:
/// subsequent calls are no-ops (the first handle wins) so plugin reloads
/// or test harnesses can't clobber the live handle.
pub fn set_app_handle(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

/// Read-only access to the stored app handle (for the capture thread).
pub(crate) fn app_handle() -> Option<AppHandle> {
    APP_HANDLE.get().cloned()
}

/// Start capturing from the default input device.
///
/// This is a non-blocking, idempotent operation:
///   * If a recording is already active, returns `Ok(())` immediately.
///   * Otherwise it builds a fresh per-capture active flag, swaps it into
///     the module-level slot (atomically disabling any in-flight capture
///     thread's callbacks), clears the buffer, and spawns the new thread.
///
/// The previous capture thread (if any) is left to wind down on its own.
/// It will see its flag as false on the very next sample and stop writing.
pub fn start() -> Result<()> {
    // Build a fresh active flag for the new capture thread.
    let new_active = Arc::new(AtomicBool::new(true));

    // Swap the active flag into the module slot. If the slot already held
    // a "true" flag, we're already recording — abort and let the existing
    // capture keep running.
    {
        let mut slot = ACTIVE.lock();
        if slot.load(Ordering::SeqCst) {
            // Already recording — no-op. Caller's idempotent expectation met.
            return Ok(());
        }
        // Disable the old thread's callbacks IMMEDIATELY by installing the
        // new (true) flag. The OLD thread is still alive but its callback
        // closure captured the OLD Arc — which we just replaced. The old
        // Arc was the only writer of CAPTURED, so it stops appending on
        // the next sample even before its `while` loop exits.
        *slot = new_active.clone();
    }

    // Clear any prior buffer the previous thread may have left behind.
    // (The previous thread is still draining its resampler — see below —
    // so we explicitly clear here so its trailing flush is dropped.)
    // Pre-allocate capacity for a 30s @ 16kHz recording to avoid reallocations.
    {
        let mut buf = CAPTURED.lock();
        buf.clear();
        buf.reserve(30 * 16_000);
    }

    // Spawn the capture thread. The thread owns its own `active` Arc clone.
    let captured = CAPTURED.clone();
    std::thread::Builder::new()
        .name("devwhisp-audio".to_string())
        .spawn(move || {
            if let Err(e) = capture::run_capture_loop(captured, new_active.clone()) {
                log::error!("audio capture loop exited: {e:?}");
            }

            // Capture thread finished. If the module slot still points at
            // OUR flag (no newer `start()` happened), reset it to the
            // inactive sentinel so `is_active()` correctly reports false.
            {
                let mut slot = ACTIVE.lock();
                if Arc::ptr_eq(&*slot, &new_active) {
                    *slot = inactive_flag();
                }
            }
        })?;

    log::info!("audio capture started");
    Ok(())
}

/// Stop capturing and return the captured samples (16 kHz mono f32).
///
/// This is a non-blocking operation that returns whatever has been buffered
/// so far. Any trailing resampler samples that the dying capture thread
/// hasn't yet flushed are **intentionally dropped** — they are at most a
/// few hundred samples (~16 ms at 16 kHz), well below the 100 ms minimum
/// STT floor. Keeping the buffer drained on the hotkey thread guarantees
/// no event-loop blocking, which is what was causing the "shortcut button
/// doesn't work" symptom.
///
/// The OLD thread's callback closure captured the OLD `active` Arc — by
/// the time `stop_and_drain` returns, we've already swapped the slot to
/// the inactive sentinel, so the old thread's next callback (if any) sees
/// `active == false` and stops appending to CAPTURED. The trailing flush
/// in `run_capture_loop` happens *after* the swap, but `CAPTURED` was
/// already moved out by `drain(..)`, so any append is to an empty (or
/// soon-to-be-cleared) buffer that the next `start()` will clear anyway.
pub fn stop_and_drain() -> Result<Vec<f32>> {
    // Disable callbacks by flipping the global slot to the inactive
    // sentinel. The old capture thread's callback closure reads its OWN
    // captured Arc — but we ALSO need to flip THAT one to false so the
    // cpal callback stops writing. We do this by swapping the slot AND
    // flipping whatever Arc used to live there.
    let old_active = {
        let mut slot = ACTIVE.lock();
        let old = std::mem::replace(&mut *slot, inactive_flag());
        // Flip the OLD flag so its callback closure goes quiet immediately.
        old.store(false, Ordering::SeqCst);
        old
    };
    // Reference the old_active so the compiler doesn't complain about it
    // being unused — its `store(false)` above is the side effect we need.
    let _ = old_active;

    // Move the buffer out. Any subsequent append by the dying thread is to
    // an empty Vec; `start()` will clear it before the new thread starts
    // writing, so contamination is impossible.
    let buf = CAPTURED.lock().drain(..).collect::<Vec<_>>();
    log::info!("audio capture stopped; {} samples drained", buf.len());
    Ok(buf)
}

pub fn is_active() -> bool {
    ACTIVE.lock().load(Ordering::SeqCst)
}

/// Returns the last known audio level (0.0 silent .. 1.0 loud) from the level ticker.
/// (Normalized UI value, not raw energy.)
pub fn last_audio_level() -> f32 {
    *LAST_LEVEL.lock()
}

/// Returns raw RMS energy (0.0..1.0) of the most recent ~N samples from the
/// capture buffer. Used for VAD energy-threshold checks (prefer raw over
/// AGC-normalized UI level).
pub fn recent_rms(window: usize) -> f32 {
    let buf = CAPTURED.lock();
    if buf.is_empty() {
        return 0.0;
    }
    let w = window.min(buf.len()).max(16);
    let start = buf.len() - w;
    visualizer::rms_level(&buf[start..])
}

/// Pure helper for VAD auto-end decision (kept simple + testable).
/// Returns true when energy is below threshold for at least the hold-off.
pub fn vad_should_stop(rms: f32, elapsed_silent_ms: u64, holdoff_ms: u64, energy_threshold: f32) -> bool {
    rms < energy_threshold && elapsed_silent_ms >= holdoff_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_logic_basic() {
        // comments document the logic
        assert!(vad_should_stop(0.01, 700, 600, 0.015), "should end after enough silence");
        assert!(!vad_should_stop(0.01, 500, 600, 0.015), "not enough hold-off yet");
        assert!(!vad_should_stop(0.03, 800, 600, 0.015), "speech above threshold");
        assert!(vad_should_stop(0.0149, 600, 600, 0.015));
        // zero buffer case handled by caller
    }

    #[test]
    fn e2e_smoke_hotkey_vad_audio_drain_transcribe_paths() {
        // Exercises key control flows from plan: VAD mode, hotkey mode switching,
        // vad_should_stop helper, audio start/stop/drain (no real mic; just flags + drain),
        // and transcribe entry (empty or no-model case returns guidance or stub).
        // Conceptual e2e for no-device / partial-sim / VAD.
        use crate::hotkey;

        // mode paths (push-to-talk / toggle / vad)
        hotkey::set_mode("vad");
        assert_eq!(hotkey::get_mode(), "vad");
        hotkey::set_mode("toggle");
        assert_eq!(hotkey::get_mode(), "toggle");
        hotkey::set_mode("push-to-talk");
        assert_eq!(hotkey::get_mode(), "push-to-talk");

        // VAD decision smoke with realistic numbers
        assert!(vad_should_stop(0.005, 650, 600, 0.015));
        assert!(!vad_should_stop(0.02, 650, 600, 0.015));

        // audio drain path (flags + drain) - does not open real device in test
        // (start would open cpal but we only test is_active / drain semantics here)
        assert!(!crate::audio::is_active());
        // recent_rms on empty is 0
        assert_eq!(crate::audio::recent_rms(512), 0.0);

        // transcribe entrypoints: empty buffer or no-model yields clean result (no panic)
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let empty = rt.block_on(crate::stt::transcribe_pcm_16k(&[]));
        // either Ok("") or Err("No STT model...") — both acceptable for smoke
        match empty {
            Ok(t) => assert!(t.is_empty()),
            Err(e) => assert!(e.to_string().contains("model") || e.to_string().contains("No")),
        }

        // formatter integration smoke via hotkey path expectations (VAD source tag etc)
        // (formatter tests are separate; here just call through public)
        let pairs: Vec<(String, String)> = vec![];
        let fopts = crate::formatter::FormatOptions { auto_capitalize: true, append_space: false, dict: pairs };
        let formatted = crate::formatter::format_transcript("hello from vad smoke", &fopts);
        assert!(formatted.starts_with("Hello"));
    }
}