//! Speech-to-text dispatch.
//!
//! Phase 1 — delegates to `whisper::transcribe` (whisper.cpp via whisper-rs).
//! Phase 2+ — will add `moonshine::transcribe` behind the `moonshine` cargo
//! feature; same public API, swappable model.

pub mod model_manager;
pub mod moonshine;
pub mod whisper;

use anyhow::Result;

/// Transcribe a buffer of 16 kHz mono PCM samples (f32 in [-1, 1]).
///
/// Returns the recognized text. Empty string if no speech was detected.
///
/// Phase 1 routes through Whisper. To swap to Moonshine later, change the
/// body to `moonshine::transcribe(samples).await` behind the `moonshine`
/// feature flag.
pub async fn transcribe_pcm_16k(samples: &[f32]) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }

    if crate::stt::model_manager::active_model_path().is_none() {
        return Err(anyhow::anyhow!(
            "No STT model downloaded yet. Open Settings → Models and download one (Base recommended)."
        ));
    }

    let variant_name = crate::stt::model_manager::active_model_dir()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();

    log::info!(
        "transcribing {} samples ({:.2}s @ 16 kHz) with {}",
        samples.len(),
        samples.len() as f32 / 16_000.0,
        variant_name
    );

    if variant_name.contains("moonshine") {
        #[cfg(feature = "moonshine")]
        {
            moonshine::transcribe(samples).await
        }
        #[cfg(not(feature = "moonshine"))]
        {
            // Do not hard-fail the whole transcription. Return a stub with guidance
            // so the user sees output instead of generic "transcription failed".
            log::warn!(
                "Moonshine model is active, but this build does not include the `moonshine` feature (ort). \
                 Using placeholder. For real inference: `npm run tauri:dev:moonshine` or build with --features moonshine."
            );
            Ok("[Moonshine requires --features moonshine build]".to_string())
        }
    } else {
        // Default / Whisper path (sync wrapped)
        whisper::transcribe(samples)
    }
}
