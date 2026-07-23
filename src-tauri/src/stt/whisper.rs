//! Whisper STT runner (whisper.cpp via `whisper-rs`).
//!
//! Phase 1 — real implementation. Lazy-initializes a `WhisperContext` on
//! first use, then reuses it across calls. Audio format: 16 kHz mono f32 PCM,
//! which is what `super::audio` produces.

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

/// Lazily-initialized whisper context. Created on the first call to
/// `transcribe`, then reused for the lifetime of the app.
static CONTEXT: once_cell::sync::Lazy<Mutex<Option<Arc<WhisperContext>>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

static STATE: once_cell::sync::Lazy<Mutex<Option<WhisperState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// Pre-load the model context + decode state in the background so the very
/// first real transcription doesn't pay the (~1 s) load cost. Best-effort:
/// called once at startup after the model is staged. No-op if already loaded
/// or the model isn't present yet.
pub fn warm() -> Result<()> {
    let model_path = match super::model_manager::active_model_path() {
        Some(p) if p.is_file() => p,
        _ => return Ok(()),
    };
    let ctx = {
        let mut guard = CONTEXT.lock();
        if guard.is_none() {
            let c = WhisperContext::new_with_params(
                model_path.to_string_lossy().as_ref(),
                WhisperContextParameters::default(),
            )
            .map_err(|e| anyhow!("warm: failed to load model: {:?}", e))?;
            *guard = Some(Arc::new(c));
        }
        Arc::clone(guard.as_ref().unwrap())
    };
    {
        let mut state_guard = STATE.lock();
        if state_guard.is_none() {
            *state_guard = Some(
                ctx.create_state()
                    .map_err(|e| anyhow!("warm: failed to create state: {:?}", e))?,
            );
        }
    }
    log::info!("whisper warmed (model + state preloaded)");
    Ok(())
}

/// Transcribe a buffer of 16 kHz mono f32 PCM samples.
///
/// Returns the recognized text. Empty string if no speech was detected or
/// the buffer is too short (< 100 ms).
pub fn transcribe(samples: &[f32]) -> Result<String> {
    if samples.len() < 1600 {
        return Ok(String::new());
    }

    let model_path = super::model_manager::active_model_path()
        .ok_or_else(|| anyhow!("no model downloaded; run download() in model_manager first"))?;

    // Pre-flight: ensure the model file is actually on disk and non-empty.
    if !model_path.is_file() {
        return Err(anyhow!(
            "active model path {:?} is not a file; the model may not have downloaded correctly",
            model_path
        ));
    }
    let meta = std::fs::metadata(&model_path)
        .with_context(|| format!("stat {:?}", model_path))?;
    if meta.len() < 1_000_000 {
        return Err(anyhow!(
            "model file {:?} is only {} bytes — download looks incomplete; re-download the model",
            model_path,
            meta.len()
        ));
    }
    log::info!(
        "loading whisper model from {:?} ({} MB)",
        model_path,
        meta.len() / 1_000_000
    );

    let started = std::time::Instant::now();

    // Initialize context + state on first call. Previously this took two
    // separate locks (CONTEXT, then STATE) which left a window where a
    // concurrent `reset()` could clear both, panicking on `unwrap()` below.
    // Now both refs are taken from a single critical section via a OnceCell
    // pattern: STATE owns the WhisperState and CONTEXT only holds the shared
    // Arc<WhisperContext>. CONTEXT is initialized first (independent of
    // STATE) and STATE is initialized once, in one guard.
    let ctx = {
        let mut guard = CONTEXT.lock();
        if guard.is_none() {
            let ctx = WhisperContext::new_with_params(
                model_path.to_string_lossy().as_ref(),
                WhisperContextParameters::default(),
            )
            .map_err(|e| anyhow!("failed to load whisper model: {:?}", e))?;
            *guard = Some(Arc::new(ctx));
        }
        Arc::clone(guard.as_ref().unwrap())
    };

    // Single lock acquisition for STATE — initializes on first call and
    // returns a guard that lives for the entire transcription. No second
    // lock, no race window with `reset()`.
    let mut state_guard = STATE.lock();
    if state_guard.is_none() {
        log::info!("creating whisper state");
        *state_guard = Some(
            ctx.create_state()
                .map_err(|e| anyhow!("failed to create whisper state: {:?}", e))?,
        );
    }
    let state = state_guard.as_mut().unwrap();

    // Build the full-transcription parameters. Greedy decoding for speed.
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // English-only checkpoints always use "en". Multilingual (large-v3, distil)
    // auto-detect so non-English dictation still works.
    let lang = match super::model_manager::active_variant() {
        Some(v) if v.is_english_only() => Some("en"),
        Some(_) => None, // auto-detect
        None => Some("en"),
    };
    params.set_language(lang);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_single_segment(true);

    // Physical cores only: hyper-threads share execution units and don't help
    // ALU-bound decoder work. Cap at 8 for sanity on HEDT.
    let n_threads = num_cpus::get_physical().clamp(2, 8) as i32;
    params.set_n_threads(n_threads);

    // The whisper encoder is a *fixed* cost: it always processes a 30 s mel
    // window (1500 frames), even for a 2 s clip — that's why short utterances
    // had a high real-time factor. Shrink the encoder context to the actual
    // utterance length (+ a margin) so we don't burn cycles on padded silence.
    // This is the single biggest CPU win for typical short voice commands.
    // Clips >= 24 s keep the full context (0 = default 1500).
    let secs = samples.len() as f32 / 16_000.0;
    let audio_ctx: i32 = if secs >= 24.0 {
        0
    } else {
        // ~50 encoder frames per second of audio, + ~1.6 s margin, floored so
        // very short clips keep enough context for accuracy.
        (((secs * 50.0) as i32) + 80).clamp(256, 1500)
    };
    params.set_audio_ctx(audio_ctx);

    log::info!(
        "whisper: transcribing {} samples ({:.2}s @ 16 kHz) with {} threads, audio_ctx={}",
        samples.len(),
        secs,
        n_threads,
        audio_ctx
    );
    state
        .full(params, samples)
        .map_err(|e| anyhow!("whisper full() failed: {:?}", e))?;

    let n = state.full_n_segments();
    let mut text = String::new();
    for i in 0..n {
        let segment = state
            .get_segment(i)
            .ok_or_else(|| anyhow!("segment {} missing", i))?;
        let segment_text = segment
            .to_str()
            .map_err(|e| anyhow!("segment text decode error: {:?}", e))?;
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(segment_text.trim());
    }
    let elapsed = started.elapsed();
    let audio_secs = samples.len() as f32 / 16_000.0;
    log::info!(
        "whisper: {} segments, {:.2}s of audio in {:.2}s (RTF={:.2})",
        n,
        audio_secs,
        elapsed.as_secs_f32(),
        elapsed.as_secs_f32() / audio_secs
    );
    Ok(text)
}

/// Reset the loaded context and state so that the next transcription will reload
/// based on the current active model (supports model switching without full restart).
pub fn reset() {
    let mut ctx_guard = CONTEXT.lock();
    *ctx_guard = None;
    let mut state_guard = STATE.lock();
    *state_guard = None;
    log::info!("whisper context reset for model switch");
}

#[cfg(test)]
mod bench {
    use super::*;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    /// Content-agnostic test signal — the encoder cost depends on frame count
    /// (audio_ctx) and threads, not on what's being said, so this is a fair
    /// proxy for measuring the encoder speedup.
    fn gen_audio(secs: f32) -> Vec<f32> {
        let n = (secs * 16_000.0) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                0.03 * ((t * 180.0 * std::f32::consts::TAU).sin()
                    + 0.5 * (t * 350.0 * std::f32::consts::TAU).sin())
            })
            .collect()
    }

    fn time_one(ctx: &WhisperContext, samples: &[f32], threads: i32, audio_ctx: i32) -> f32 {
        let mut st = ctx.create_state().unwrap();
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_language(Some("en"));
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        p.set_no_context(true);
        p.set_single_segment(true);
        p.set_n_threads(threads);
        p.set_audio_ctx(audio_ctx);
        let t0 = std::time::Instant::now();
        st.full(p, samples).unwrap();
        t0.elapsed().as_secs_f32()
    }

    /// Manual benchmark: old (4 threads, full 30 s context) vs new (cores,
    /// audio-length-scaled context). Run with:
    ///   cargo test --lib bench_old_vs_new -- --ignored --nocapture
    #[test]
    #[ignore = "loads the 77 MB model and runs inference"]
    fn bench_old_vs_new() {
        let path = match crate::stt::model_manager::active_model_path() {
            Some(p) if p.is_file() => p,
            _ => {
                eprintln!("SKIP: no model on disk");
                return;
            }
        };
        let ctx = WhisperContext::new_with_params(
            path.to_string_lossy().as_ref(),
            WhisperContextParameters::default(),
        )
        .unwrap();
        let logical = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let new_threads = num_cpus::get_physical().clamp(2, 8) as i32;
        for secs in [3.0_f32, 6.0, 11.0] {
            let audio = gen_audio(secs);
            let _ = time_one(&ctx, &audio, 4, 0); // warm caches
            let old = time_one(&ctx, &audio, 4, 0);
            let new_ac = (((secs * 50.0) as i32) + 80).clamp(256, 1500);
            let new = time_one(&ctx, &audio, new_threads, new_ac);
            eprintln!(
                "BENCH {:>4.0}s: OLD(4t,full)={:.2}s (RTF {:.2})  NEW({}t,ac{})={:.2}s (RTF {:.2})  -> {:.1}x faster",
                secs, old, old / secs, new_threads, new_ac, new, new / secs, old / new
            );
        }
    }
}
