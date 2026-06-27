//! cpal-based microphone capture with on-the-fly resampling to 16 kHz mono f32.
//!
//! Pipeline:
//!   1. Open the default input stream at the device's native rate (usually 48 kHz).
//!   2. Convert each frame to mono f32 in [-1, 1] (cpal `Sample` trait).
//!   3. Push into a `rubato` resampler that targets 16 kHz.
//!   4. Append resampled frames into the shared captured buffer.
//!
//! While recording, a side-channel ticker thread computes the RMS audio level
//! every ~33 ms (30 fps) on the most recent `LEVEL_WINDOW_SAMPLES` samples
//! and emits an `audio-level` event to the pill window so it can drive its
//! live visualizer. Emits are throttled by a "changed enough" OR "long
//! enough ago" rule so the IPC channel never floods.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, StreamConfig};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::resampler::StreamingResampler;
use super::visualizer::{LevelTracker, LEVEL_WINDOW_SAMPLES};

const TARGET_RATE: u32 = 16_000;

/// Tick interval for the audio-level emitter — ~60 fps.
const LEVEL_TICK_MS: u64 = 16;

/// Minimum change vs. the last emitted value that triggers an immediate emit.
/// Smaller changes are still emitted once `LEVEL_MAX_STALENESS_MS` passes.
const LEVEL_DELTA_THRESHOLD: f32 = 0.006;

/// Hard ceiling on how stale an "unchanged enough" reading can get before
/// we force an emit anyway. Keeps the UI animation feeling alive.
const LEVEL_MAX_STALENESS_MS: u64 = 50;

/// Payload for the `audio-level` event.
#[derive(Debug, Clone, Serialize)]
struct AudioLevelPayload {
    level: f32,
}

/// Resolve preferred device (from settings "audio_device") or default.
/// Falls back gracefully if named device not present.
fn get_input_device() -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = crate::config::load_string("audio_device") {
        if !name.is_empty() && name != "Default" {
            if let Ok(mut devs) = host.input_devices() {
                for d in devs.by_ref() {
                    if let Ok(desc) = d.description() {
                        if desc.name() == name {
                            return Ok(d);
                        }
                    }
                }
            }
            log::warn!("preferred audio device '{name}' not found; using default");
        }
    }
    host.default_input_device()
        .ok_or_else(|| anyhow!("no default input device available"))
}

pub fn run_capture_loop(
    captured: Arc<Mutex<Vec<f32>>>,
    active_flag: Arc<AtomicBool>,
) -> Result<()> {
    let device = get_input_device()?;
    let device_name = device
        .description()
        .ok()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    log::info!("audio input device: {device_name}");

    let cfg = device
        .default_input_config()
        .context("failed to query default input config")?;
    let sample_format = cfg.sample_format();
    let source_rate: u32 = cfg.sample_rate();
    let channels = cfg.channels() as usize;
    let stream_config: StreamConfig = cfg.into();
    log::info!(
        "audio input config: rate={source_rate} Hz, channels={channels}, format={sample_format:?}"
    );

    // Resampler is shared across the audio callback (mutable) and the
    // post-capture flush (also mutable). Wrap in Arc<Mutex<>> so both can
    // access. The cpal callback is real-time-ish; parking_lot::Mutex is
    // non-poisoning and reasonably fast for short critical sections.
    let resampler: Arc<Mutex<StreamingResampler>> =
        Arc::new(Mutex::new(StreamingResampler::new(source_rate, TARGET_RATE, channels)?));

    let err = |e: cpal::Error| log::error!("audio stream error: {e}");

    // --- Spawn the audio-level ticker thread (30 fps) -------------------
    //
    // Runs alongside the capture stream. Reads the most-recent
    // `LEVEL_WINDOW_SAMPLES` from the shared buffer, computes RMS, and
    // emits the throttled payload to the pill window. The thread exits
    // when `active_flag` flips off. The AppHandle is fetched from the
    // module-level static set in lib.rs setup — no need to thread it
    // through every call site.
    let level_captured = Arc::clone(&captured);
    let level_active = Arc::clone(&active_flag);
    let level_thread = std::thread::Builder::new()
        .name("devwhisp-audio-level".to_string())
        .spawn(move || {
            level_ticker_loop(level_captured, level_active);
        })?;

    // Build the stream with the appropriate typed callback for this device's
    // sample format. Each callback downmixes, resamples, and appends to the
    // shared buffer. All closures capture Arc clones (Send + 'static).
    //
    // Per-callback allocation strategy:
    //   * `mono_buf` and `resampled_buf` are thread-local, swapped between
    //     calls so the cpal callback never allocates.
    //   * `resampled_buf` is then moved into the captured buffer (one
    //     allocation per callback for the extend, which `extend_from_slice`
    //     amortizes — Vec grows geometrically).
    let stream = match sample_format {
        SampleFormat::F32 => {
            let captured = Arc::clone(&captured);
            let active_flag = Arc::clone(&active_flag);
            let resampler = Arc::clone(&resampler);
            let ch = channels;
            device.build_input_stream(
                stream_config,
                move |data: &[f32], _ci: &cpal::InputCallbackInfo| {
                    if !active_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut mono_buf: Vec<f32> = Vec::with_capacity(data.len());
                    if ch == 1 {
                        mono_buf.extend_from_slice(data);
                    } else {
                        for frame in data.chunks_exact(ch) {
                            let sum: f32 = frame.iter().copied().sum();
                            mono_buf.push(sum / ch as f32);
                        }
                    }
                    let mut resampled_buf: Vec<f32> =
                        Vec::with_capacity(mono_buf.len() / 3 + 64);
                    resampler.lock().process_into(&mono_buf, &mut resampled_buf);
                    captured.lock().extend_from_slice(&resampled_buf);
                },
                err,
                None,
            )?
        }
        SampleFormat::I16 => {
            let captured = Arc::clone(&captured);
            let active_flag = Arc::clone(&active_flag);
            let resampler = Arc::clone(&resampler);
            let ch = channels;
            device.build_input_stream(
                stream_config,
                move |data: &[i16], _ci: &cpal::InputCallbackInfo| {
                    if !active_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut mono_buf: Vec<f32> = Vec::with_capacity(data.len());
                    if ch == 1 {
                        for s in data {
                            mono_buf.push(s.to_sample::<f32>());
                        }
                    } else {
                        for frame in data.chunks_exact(ch) {
                            let mut sum = 0.0_f32;
                            for s in frame {
                                sum += s.to_sample::<f32>();
                            }
                            mono_buf.push(sum / ch as f32);
                        }
                    }
                    let mut resampled_buf: Vec<f32> =
                        Vec::with_capacity(mono_buf.len() / 3 + 64);
                    resampler.lock().process_into(&mono_buf, &mut resampled_buf);
                    captured.lock().extend_from_slice(&resampled_buf);
                },
                err,
                None,
            )?
        }
        SampleFormat::U16 => {
            let captured = Arc::clone(&captured);
            let active_flag = Arc::clone(&active_flag);
            let resampler = Arc::clone(&resampler);
            let ch = channels;
            device.build_input_stream(
                stream_config,
                move |data: &[u16], _ci: &cpal::InputCallbackInfo| {
                    if !active_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut mono_buf: Vec<f32> = Vec::with_capacity(data.len());
                    if ch == 1 {
                        for s in data {
                            mono_buf.push(s.to_sample::<f32>());
                        }
                    } else {
                        for frame in data.chunks_exact(ch) {
                            let mut sum = 0.0_f32;
                            for s in frame {
                                sum += s.to_sample::<f32>();
                            }
                            mono_buf.push(sum / ch as f32);
                        }
                    }
                    let mut resampled_buf: Vec<f32> =
                        Vec::with_capacity(mono_buf.len() / 3 + 64);
                    resampler.lock().process_into(&mono_buf, &mut resampled_buf);
                    captured.lock().extend_from_slice(&resampled_buf);
                },
                err,
                None,
            )?
        }
        // Other formats (I8/U8/I24/U24/F64) are uncommon on consumer mics.
        other => {
            log::warn!("non-standard sample format: {other:?}; aborting capture");
            return Err(anyhow!("unsupported sample format: {other:?}"));
        }
    };

    stream.play().context("failed to start audio stream")?;
    log::info!("audio stream playing");

    // Keep the stream alive until the active flag flips off.
    while active_flag.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(stream);
    log::info!("audio stream stopped");

    // Wait for the level ticker to wind down so its final ticks don't
    // outlive the stream.
    let _ = level_thread.join();

    // Discard the resampler's trailing samples (rubato's FFT group delay).
    // We deliberately do NOT append them to `CAPTURED` here because:
    //
    //   1. By the time we reach this point, `stop_and_drain` has already
    //      swapped the active flag and returned the buffer to the caller.
    //      Anything we append now lands in a stale buffer.
    //   2. If the caller immediately calls `start()` for a new recording,
    //      `start()` clears the buffer — but the OLD trailing flush here
    //      could still race with `start()`'s clear and contaminate it,
    //      producing the "duplicate recording" bug.
    //   3. The trailing group delay is at most ~250 samples (~16 ms at
    //      16 kHz), well below Whisper's 100 ms minimum input floor, so
    //      dropping them has no perceptible impact on transcription.
    let mut trailing: Vec<f32> = Vec::new();
    resampler.lock().flush_into(&mut trailing);
    if !trailing.is_empty() {
        log::debug!(
            "discarded {} trailing resampler samples from dying capture thread",
            trailing.len()
        );
    }

    Ok(())
}

/// ~60 fps audio-level ticker.
///
/// Computes a gain-normalized reactive level over the most recent
/// `LEVEL_WINDOW_SAMPLES` and emits `audio-level` to the pill window.
/// Throttling: emit immediately if the level moved by ≥
/// `LEVEL_DELTA_THRESHOLD`, or every `LEVEL_MAX_STALENESS_MS` at most —
/// whichever fires first.
fn level_ticker_loop(captured: Arc<Mutex<Vec<f32>>>, active_flag: Arc<AtomicBool>) {
    let tick = Duration::from_millis(LEVEL_TICK_MS);
    let staleness = Duration::from_millis(LEVEL_MAX_STALENESS_MS);

    let mut tracker = LevelTracker::default();
    let mut last_emitted_level: f32 = 0.0;
    let mut last_emit_at = Instant::now() - staleness; // first tick emits unconditionally

    while active_flag.load(Ordering::Relaxed) {
        // Pull just the window we need. Copying a 256-sample slice per tick
        // is cheap and keeps the lock held for only a couple of microseconds.
        let window: Vec<f32> = {
            let buf = captured.lock();
            let start = buf.len().saturating_sub(LEVEL_WINDOW_SAMPLES);
            buf[start..].to_vec()
        };

        let level = tracker.update(&window);
        let changed_enough = (level - last_emitted_level).abs() >= LEVEL_DELTA_THRESHOLD;
        let stale_enough = last_emit_at.elapsed() >= staleness;

        if changed_enough || stale_enough {
            if let Some(app) = super::app_handle() {
                emit_audio_level(&app, level);
            }
            *super::LAST_LEVEL.lock() = level;
            last_emitted_level = level;
            last_emit_at = Instant::now();
        }

        std::thread::sleep(tick);
    }

    tracker.reset();

    // Final "back to silence" tick so the pill wave drops on stop.
    if let Some(app) = super::app_handle() {
        emit_audio_level(&app, 0.0);
    }
}

/// Send a single `audio-level` payload to the pill window, falling back to
/// a global emit if the window isn't registered yet.
fn emit_audio_level(app: &AppHandle, level: f32) {
    let payload = AudioLevelPayload { level };
    if let Some(window) = app.get_webview_window(crate::window::pill_window::PILL_LABEL) {
        if let Err(e) = window.emit("audio-level", payload) {
            log::debug!("audio-level emit to pill window failed: {e}");
        }
    } else if let Err(e) = app.emit("audio-level", payload) {
        log::debug!("audio-level global emit failed: {e}");
    }
}