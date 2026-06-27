//! Audio level analysis for the pill visualizer.
//!
//! Primitives:
//!   - [`rms_level`] — sustained energy in a window
//!   - [`peak_level`] — loudest sample (fast attack)
//!   - [`LevelTracker`] — noise gate + peak envelope + session AGC for
//!     quiet mics, mapped into `0.0..1.0` for the UI

/// Number of recent samples the audio-level emitter considers each tick.
///
/// 256 samples @ 16 kHz ≈ 16 ms — short enough to feel instant on speech
/// transients while still filtering single-sample spikes.
pub const LEVEL_WINDOW_SAMPLES: usize = 256;

/// Root-mean-square amplitude of `samples`, normalized into `0.0..1.0`.
///
/// * `silence` → `0.0`
/// * full-scale sine (`+/- 1.0`) → `~0.707` (the RMS of a sine is its
///   amplitude divided by √2; we leave that ratio alone and let the UI
///   apply any further scaling it wants)
/// * DC / clipping (`+/- 1.0`) → `~1.0`
///
/// Returns `0.0` for an empty buffer (no signal beats no data).
pub fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    // Accumulate in f64 to dodge catastrophic cancellation on long buffers.
    let mut sum_sq = 0.0_f64;
    for &s in samples {
        let v = s as f64;
        sum_sq += v * v;
    }
    let mean_sq = sum_sq / samples.len() as f64;
    let rms = mean_sq.sqrt();
    // Clamp to the documented range. Floating-point noise from the sqrt
    // can poke a hair above 1.0 when every sample is exactly 1.0.
    rms.clamp(0.0, 1.0) as f32
}

/// Peak absolute amplitude in `samples`, in `0.0..1.0`.
pub fn peak_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples
        .iter()
        .map(|s| s.abs())
        .fold(0.0_f32, f32::max)
        .clamp(0.0, 1.0)
}

/// Stateful mapper that boosts quiet microphones into a reactive `0..1` range.
///
/// Pipeline per tick:
///   1. Track a noise floor while the input is quiet.
///   2. Maintain a peak envelope (fast attack, slower decay).
///   3. Normalize against a rolling session peak (AGC).
///   4. Apply a soft-knee curve so whispers still move the visualizer.
#[derive(Debug, Clone)]
pub struct LevelTracker {
    noise_floor: f32,
    envelope: f32,
    session_peak: f32,
}

impl Default for LevelTracker {
    fn default() -> Self {
        Self {
            noise_floor: 0.0,
            envelope: 0.0,
            session_peak: 0.04,
        }
    }
}

impl LevelTracker {
    /// Reset transient state when a new recording session starts.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Compute the next UI level from the latest sample window.
    pub fn update(&mut self, samples: &[f32]) -> f32 {
        let peak = peak_level(samples);
        let rms = rms_level(samples);

        // Learn ambient noise while the window stays near the floor.
        if peak < self.noise_floor * 1.8 + 0.002 {
            self.noise_floor = self.noise_floor * 0.96 + peak * 0.04;
        }
        self.noise_floor = self.noise_floor.clamp(0.0, 0.12);

        // Peak envelope — attack quickly, decay slowly so the UI "hangs" on speech.
        if peak > self.envelope {
            self.envelope = self.envelope * 0.25 + peak * 0.75;
        } else {
            self.envelope = self.envelope * 0.78 + peak * 0.22;
        }

        let gated_peak = (self.envelope - self.noise_floor).max(0.0);
        let gated_rms = (rms - self.noise_floor).max(0.0);
        let raw = gated_peak * 0.65 + gated_rms * 0.35;

        // Session AGC: quiet mics get boosted; loud speech stays bounded.
        let observed = gated_peak.max(gated_rms);
        self.session_peak = self.session_peak * 0.985 + observed * 0.015;
        self.session_peak = self.session_peak.clamp(0.015, 1.0);

        let agc_gain = (0.22 / self.session_peak).clamp(2.0, 18.0);
        let scaled = raw * agc_gain;

        // Soft knee: expand low levels, compress near full scale.
        let out = 1.0 - (-scaled * 5.0).exp();
        out.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_returns_zero() {
        let buf = vec![0.0_f32; 512];
        let level = rms_level(&buf);
        assert!(
            level.abs() < 1e-6,
            "expected silence → 0.0, got {level}"
        );
    }

    #[test]
    fn full_scale_dc_returns_one() {
        // A constant +1.0 signal has RMS = |+1.0| = 1.0.
        let buf = vec![1.0_f32; 512];
        let level = rms_level(&buf);
        assert!(
            (level - 1.0).abs() < 1e-4,
            "expected full-scale DC → ~1.0, got {level}"
        );

        let buf_neg = vec![-1.0_f32; 512];
        let level_neg = rms_level(&buf_neg);
        assert!(
            (level_neg - 1.0).abs() < 1e-4,
            "expected full-scale DC (negative) → ~1.0, got {level_neg}"
        );
    }

    #[test]
    fn small_random_samples_land_in_known_range() {
        // Hand-built deterministic pseudo-random samples in [-0.5, 0.5].
        // RMS of a uniform distribution on [-a, a] is a / sqrt(3) — for
        // a=0.5 that's ≈ 0.2887. We give it a generous ±15% window so
        // the test stays robust if the implementation ever switches to a
        // slightly different normalization.
        let mut buf = Vec::with_capacity(512);
        let mut state: u32 = 0xC0FFEE;
        for _ in 0..512 {
            // xorshift32 — cheap, deterministic.
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            // Map low 24 bits to [-0.5, 0.5].
            let frac = (state & 0x00FF_FFFF) as f32 / 0x0100_0000 as f32; // [0, 1)
            buf.push(frac - 0.5);
        }
        let level = rms_level(&buf);
        // Loose bound — well within the theoretical envelope.
        assert!(
            level > 0.20 && level < 0.40,
            "expected RMS in (0.20, 0.40) for ±0.5 uniform noise, got {level}"
        );
        // Definitely quieter than full-scale.
        assert!(level < 0.5, "small samples should not saturate, got {level}");
    }

    #[test]
    fn empty_buffer_returns_zero() {
        let level = rms_level(&[]);
        assert_eq!(level, 0.0);
    }

    #[test]
    fn quiet_speech_gets_boosted_by_tracker() {
        let mut tracker = LevelTracker::default();
        // Simulate a quiet mic: RMS ~0.02, peak ~0.05.
        let quiet: Vec<f32> = (0..256)
            .map(|i| ((i as f32) * 0.31).sin() * 0.05)
            .collect();

        let mut last = 0.0_f32;
        for _ in 0..8 {
            last = tracker.update(&quiet);
        }
        assert!(
            last > 0.25,
            "expected quiet speech to map above 0.25 for UI, got {last}"
        );
        assert!(last < 1.0, "AGC should not saturate on quiet input, got {last}");
    }

    #[test]
    fn tracker_resets_between_sessions() {
        let mut tracker = LevelTracker::default();
        let loud = vec![0.8_f32; 256];
        let _ = tracker.update(&loud);
        tracker.reset();
        let silent = vec![0.0_f32; 256];
        let level = tracker.update(&silent);
        assert!(level < 0.05, "reset session should start near silence, got {level}");
    }

    #[test]
    fn sine_wave_matches_analytic_rms() {
        // RMS of a +/-1.0 sine is 1/sqrt(2) ≈ 0.7071.
        let n = 1024;
        let mut buf = Vec::with_capacity(n);
        for i in 0..n {
            let phase = (i as f32) * std::f32::consts::TAU / 64.0;
            buf.push(phase.sin());
        }
        let level = rms_level(&buf);
        let expected = 1.0 / 2.0_f32.sqrt();
        assert!(
            (level - expected).abs() < 0.02,
            "expected sine RMS ≈ {expected}, got {level}"
        );
    }
}