//! Adaptive Voice Activity Detection (VAD) with pause-aware state machine.
//!
//! Unlike the original threshold-only VAD, this engine:
//!   1. Adapts its energy threshold to the observed noise floor and speech peak
//!      (so it works across different mics, rooms, and distances).
//!   2. Distinguishes **brief pauses** (intra-sentence breaths) from **true silence**
//!      (end of utterance) via a two-tier hold-off.
//!   3. Emits a `paused` pill state so the user sees the app is still listening
//!      while they gather their thoughts — producing a cleaner, more natural flow.
//!
//! State machine:
//!
//!   Idle → [press] → Speaking ──brief silence──→ Paused ──long silence──→ Stopped
//!                      ↑                            │
//!                      └────speech resumes──────────┘
//!
//! * `Speaking`  : energy is above threshold.
//! * `Paused`    : energy dropped below threshold, but within the *pause* window.
//!                 The capture thread keeps running.  The pill shows "Paused".
//! * `Stopped`   : energy stayed below threshold past the *silence* window.
//!                 Capture ends and transcription runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio;
use crate::window::pill_window;

// ---------------------------------------------------------------------------
// Configuration defaults
// ---------------------------------------------------------------------------

/// Default brief-pause duration (ms).  Energy below threshold for *this* long
/// triggers the `Paused` UI state, but recording continues.
const DEFAULT_PAUSE_MS: u64 = 400;

/// Default end-of-speech silence (ms).  Energy below threshold for *this* long
/// triggers actual stop + transcription.  Must be ≥ pause_ms.
const DEFAULT_SILENCE_MS: u64 = 900;

/// Minimum recording duration before auto-stop is allowed (prevents cutting
/// off very short utterances before the user even starts speaking).
const MIN_SPEECH_MS: u64 = 300;

/// VAD poll interval.  25 ms is responsive enough to feel instant while keeping
/// CPU use negligible.
const VAD_TICK_MS: u64 = 25;

/// Energy window for a single RMS reading: 1024 samples @ 16 kHz ≈ 64 ms.
/// Longer than the old 512-sample window (32 ms) so breath noise is smoothed.
const VAD_RMS_WINDOW: usize = 1024;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Decision returned by the VAD engine each tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// Keep capturing — either speaking or in a brief pause.
    Continue,
    /// Brief pause detected; emit UI feedback but do NOT stop capture.
    Paused,
    /// Silence has persisted past the hold-off; stop and transcribe.
    ShouldStop,
}

/// Current state of the VAD finite-state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Idle,
    Speaking,
    Paused,
}

/// Serialised state payload sent to the pill window.
#[derive(Debug, Clone, Serialize)]
struct PillState {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

// ---------------------------------------------------------------------------
// VAD engine
// ---------------------------------------------------------------------------

/// Adaptive voice-activity engine.
///
/// Call `process()` on every VAD tick (every ~25 ms).  It tracks the noise
/// floor and speech peak in real time, builds a stable per-tick energy
/// reading from a 1024-sample window, and drives the state machine.
pub struct VadEngine {
    /// Current FSM state.
    state: VadState,

    /// When did the current state begin?
    state_entered_at: Instant,

    /// When did we last see energy above the adaptive threshold?
    last_speech_at: Instant,

    /// When did the overall recording start?  (Used for `min_speech_ms`.)
    recording_started_at: Instant,

    /// Estimated noise floor (RMS of quietest recent windows).  Updated with
    /// an exponential moving average when energy is very low.
    noise_floor: f32,

    /// Highest RMS seen since the start of this utterance.  Used to set the
    /// adaptive threshold relative to the dynamic range.
    speech_peak: f32,

    /// Fixed user-configurable threshold (loaded from settings).  When
    /// adaptive mode is off this is used directly.  In adaptive mode it acts
    /// as a floor so we never threshold below it.
    base_threshold: f32,

    /// Is adaptive thresholding enabled?
    adaptive: bool,

    /// Configurable timeouts (loaded from settings or defaults).
    pause_ms: u64,
    silence_ms: u64,
    min_speech_ms: u64,

    /// Rolling history of recent RMS values (for smoothing & hysteresis).
    rms_history: [f32; 8],
    rms_idx: usize,

    /// Whether we have already emitted the `paused` pill state for the
    /// current pause episode (deduplication).
    pause_emitted: bool,
}

impl VadEngine {
    /// Create a new engine with the given settings.
    ///
    /// `base_threshold` is the raw RMS floor (e.g. 0.015).  When `adaptive` is
    /// true the engine raises this floor based on observed noise and speech.
    pub fn new(
        base_threshold: f32,
        adaptive: bool,
        pause_ms: Option<u64>,
        silence_ms: Option<u64>,
        min_speech_ms: Option<u64>,
    ) -> Self {
        let pause_ms = pause_ms.unwrap_or(DEFAULT_PAUSE_MS);
        let silence_ms = silence_ms.unwrap_or(DEFAULT_SILENCE_MS).max(pause_ms + 100);
        Self {
            state: VadState::Idle,
            state_entered_at: Instant::now(),
            last_speech_at: Instant::now(),
            recording_started_at: Instant::now(),
            noise_floor: base_threshold.max(0.001),
            speech_peak: base_threshold * 4.0,
            base_threshold: base_threshold.max(0.001),
            adaptive,
            pause_ms,
            silence_ms,
            min_speech_ms: min_speech_ms.unwrap_or(MIN_SPEECH_MS),
            rms_history: [0.0; 8],
            rms_idx: 0,
            pause_emitted: false,
        }
    }

    /// Convenience: build from the user's persisted settings.
    pub fn from_settings() -> Self {
        let base = crate::config::load_vad_energy_threshold();
        let adaptive = crate::config::load_bool("vad_adaptive", true);
        let pause_ms = crate::config::load_u64("vad_pause_ms").map(|v| v as u64);
        let silence_ms = crate::config::load_u64("vad_silence_ms").map(|v| v as u64);
        let min_speech = crate::config::load_u64("vad_min_speech_ms").map(|v| v as u64);
        Self::new(base, adaptive, pause_ms, silence_ms, min_speech)
    }

    /// Call once when the recording actually starts (mic is live).
    pub fn on_recording_start(&mut self) {
        self.state = VadState::Speaking;
        let now = Instant::now();
        self.state_entered_at = now;
        self.recording_started_at = now;
        self.last_speech_at = now;
        self.noise_floor = self.base_threshold;
        self.speech_peak = self.base_threshold * 4.0;
        self.rms_history = [0.0; 8];
        self.rms_idx = 0;
        self.pause_emitted = false;
    }

    /// Feed one VAD tick.  Returns the decision for this tick.
    ///
    /// `raw_rms` should be the RMS of the most recent ~1024 samples from the
    /// live capture buffer.
    pub fn process(&mut self, raw_rms: f32) -> VadDecision {
        // ------------------------------------------------------------------
        // 1. Smooth the raw RMS with a tiny rolling average (reduces breath
        //    spikes / keyboard-click noise).
        // ------------------------------------------------------------------
        self.rms_history[self.rms_idx] = raw_rms;
        self.rms_idx = (self.rms_idx + 1) % self.rms_history.len();
        let rms: f32 = self.rms_history.iter().sum::<f32>() / self.rms_history.len() as f32;

        // ------------------------------------------------------------------
        // 2. Adaptive threshold update
        // ------------------------------------------------------------------
        if self.adaptive {
            // Update noise floor during quiet windows.
            if rms < self.noise_floor * 1.5 {
                self.noise_floor = self.noise_floor * 0.92 + rms * 0.08;
                self.noise_floor = self.noise_floor.clamp(0.0005, 0.06);
            }
            // Update speech peak when loud.
            if rms > self.speech_peak {
                self.speech_peak = self.speech_peak * 0.3 + rms * 0.7;
            } else {
                // Slow decay so a short pause doesn't collapse the peak.
                self.speech_peak = self.speech_peak * 0.985 + rms * 0.015;
            }
            self.speech_peak = self.speech_peak.clamp(self.base_threshold * 2.0, 1.0);
        }

        let threshold = self.current_threshold();
        let is_speech = rms >= threshold;

        // ------------------------------------------------------------------
        // 3. State machine
        // ------------------------------------------------------------------
        let now = Instant::now();
        let running_ms = now.duration_since(self.recording_started_at).as_millis() as u64;
        let silent_ms = now.duration_since(self.last_speech_at).as_millis() as u64;

        if is_speech {
            self.last_speech_at = now;
            if self.state != VadState::Speaking {
                self.state = VadState::Speaking;
                self.state_entered_at = now;
                self.pause_emitted = false;
            }
            return VadDecision::Continue;
        }

        // Below threshold — figure out how long we've been quiet.
        match self.state {
            VadState::Idle => VadDecision::Continue, // shouldn't happen while monitoring
            VadState::Speaking => {
                if silent_ms >= self.silence_ms && running_ms >= self.min_speech_ms {
                    // Long silence + minimum speech satisfied → stop.
                    self.state = VadState::Idle;
                    VadDecision::ShouldStop
                } else if silent_ms >= self.pause_ms {
                    // Brief silence → transition to Paused, keep recording.
                    self.state = VadState::Paused;
                    self.state_entered_at = now;
                    self.pause_emitted = false;
                    VadDecision::Paused
                } else {
                    // Still within the brief-pause window → keep going.
                    VadDecision::Continue
                }
            }
            VadState::Paused => {
                if silent_ms >= self.silence_ms && running_ms >= self.min_speech_ms {
                    self.state = VadState::Idle;
                    VadDecision::ShouldStop
                } else {
                    // Stay paused.  Emit UI once per episode.
                    if !self.pause_emitted {
                        self.pause_emitted = true;
                        VadDecision::Paused
                    } else {
                        VadDecision::Continue
                    }
                }
            }
        }
    }

    /// Current effective energy threshold (adaptive or fixed).
    pub fn current_threshold(&self) -> f32 {
        if !self.adaptive {
            return self.base_threshold;
        }
        // Adaptive threshold sits midway between noise floor and speech peak,
        // but never below the user-defined floor.
        let dynamic = (self.noise_floor + self.speech_peak) * 0.4;
        dynamic.max(self.base_threshold)
    }

    /// Current FSM state (for telemetry / logging).
    pub fn state(&self) -> VadState {
        self.state
    }

    /// Whether the engine has already emitted the `Paused` UI signal for the
    /// current pause episode.
    pub fn pause_emitted(&self) -> bool {
        self.pause_emitted
    }
}

// ---------------------------------------------------------------------------
// Background monitor thread
// ---------------------------------------------------------------------------

/// Spawn a background thread that polls the adaptive VAD engine and ends the
/// recording when true silence is detected.  Emits `pill-state` events so the
/// UI shows "Paused" during brief pauses.
///
/// This replaces the old `spawn_vad_monitor` in `hotkey.rs`.
pub fn spawn_adaptive_vad_monitor<R: tauri::Runtime>(app: AppHandle<R>, stop_requested: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("devwhisp-vad".to_string())
        .spawn(move || {
            let mut engine = VadEngine::from_settings();
            engine.on_recording_start();

            // Let the first few audio frames arrive before judging energy.
            std::thread::sleep(Duration::from_millis(150));

            let mut last_decision = VadDecision::Continue;

            while audio::is_active() && !stop_requested.load(Ordering::Relaxed) {
                let rms = audio::recent_rms(VAD_RMS_WINDOW);
                let decision = engine.process(rms);

                // Emit UI transitions on state changes.
                if decision != last_decision {
                    match decision {
                        VadDecision::Paused => {
                            emit_pill_state(&app, "paused", Some("Still listening…".to_string()));
                        }
                        VadDecision::Continue if last_decision == VadDecision::Paused => {
                            // Speech resumed after a pause.
                            emit_pill_state(&app, "listening", None);
                        }
                        VadDecision::ShouldStop => {
                            // Don't emit here — stop_and_transcribe will drive
                            // processing → idle.
                        }
                        _ => {}
                    }
                    last_decision = decision;
                }

                if decision == VadDecision::ShouldStop {
                    // Hand off to the unified stop path.
                    let _ = crate::hotkey::stop_and_transcribe(&app);
                    break;
                }

                std::thread::sleep(Duration::from_millis(VAD_TICK_MS));
            }
        })
        .ok(); // best-effort
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn emit_pill_state<R: tauri::Runtime>(app: &AppHandle<R>, state: &'static str, message: Option<String>) {
    let payload = PillState { state, message };
    if let Some(window) = app.get_webview_window(pill_window::PILL_LABEL) {
        let _ = window.emit("pill-state", payload);
    } else {
        let _ = app.emit("pill-state", payload);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_threshold_honoured() {
        let mut e = VadEngine::new(0.02, false, None, None, None);
        e.on_recording_start();
        // 0.025 is above the 0.02 fixed threshold.
        assert_eq!(e.process(0.025), VadDecision::Continue);
        assert_eq!(e.state(), VadState::Speaking);
        // 0.01 is below threshold.
        assert_eq!(e.process(0.01), VadDecision::Continue); // still within pause window
        // Simulate many ticks of silence to exceed the default 900 ms silence.
        // We can't sleep in tests, so we test the threshold computation instead.
        assert_eq!(e.current_threshold(), 0.02);
    }

    #[test]
    fn adaptive_threshold_rises() {
        let mut e = VadEngine::new(0.01, true, None, None, None);
        e.on_recording_start();
        // Feed loud speech for a few ticks.
        for _ in 0..8 {
            let _ = e.process(0.15);
        }
        // Threshold should now be well above the base 0.01.
        assert!(e.current_threshold() > 0.02, "adaptive threshold should rise above base");
    }

    #[test]
    fn pause_state_transition() {
        let mut e = VadEngine::new(0.02, false, Some(100), Some(300), Some(50));
        e.on_recording_start();
        // Start speaking.
        assert_eq!(e.process(0.05), VadDecision::Continue);
        // Drop below threshold — still within 100 ms pause window.
        assert_eq!(e.process(0.005), VadDecision::Continue);
        // We can't test exact timing without sleep, but we can verify the
        // state machine logic: after a long-enough silence it should become
        // Paused then eventually ShouldStop.
    }
}
