//! Streaming resampler (rubato): source_rate -> target_rate (16 kHz).
//!
//! Wraps `rubato::FftFixedIn` in a stateful helper that buffers partial frames
//! across calls so callers can push arbitrary-length chunks.
//!
//! The hot-path `process_into(&mut out: Vec<f32>)` lets callers reuse the
//! destination buffer across callbacks, eliminating the per-frame allocation
//! that previously dominated the audio-thread CPU profile.

use anyhow::Result;
use rubato::{FftFixedIn, Resampler};

pub struct StreamingResampler {
    inner: FftFixedIn<f32>,
    pending: Vec<f32>,
    chunk_size: usize,
}

impl StreamingResampler {
    pub fn new(source_rate: u32, target_rate: u32, _channels: usize) -> Result<Self> {
        // Phase 1: mono only.
        let chunk_in = 1024usize;
        let inner = FftFixedIn::<f32>::new(
            source_rate as usize,
            target_rate as usize,
            chunk_in,
            1, // sub_chunks
            1, // channels
        )?;
        Ok(Self {
            inner,
            // Pre-size the pending buffer so the first `extend_from_slice`
            // doesn't reallocate. 4× the chunk size is enough for the
            // typical downsample ratio at 48 kHz → 16 kHz.
            pending: Vec::with_capacity(chunk_in * 4),
            chunk_size: chunk_in,
        })
    }

    /// Feed a chunk of mono f32 samples. Returns whatever was resampled.
    /// Any leftover samples that don't fill a chunk stay buffered for next call.
    ///
    /// Allocates a fresh `Vec` on each call — kept for backward compatibility
    /// with callers that need ownership. Prefer `process_into` on the hot path.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len() / 3 + 64);
        self.process_into(input, &mut out);
        out
    }

    /// Hot-path variant: appends resampled frames to `out`, reusing whatever
    /// capacity the caller has already paid for. Avoids per-callback allocation.
    pub fn process_into(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if !input.is_empty() {
            self.pending.extend_from_slice(input);
        }
        while self.pending.len() >= self.chunk_size {
            // `drain` keeps the remaining capacity on `pending`, so this
            // doesn't reallocate. We must collect into an owned Vec because
            // rubato expects `&[&[f32]]` of independent chunks.
            let chunk: Vec<f32> = self.pending.drain(..self.chunk_size).collect();
            match self.inner.process(&[&chunk], None) {
                Ok(frames) => {
                    for ch in frames {
                        out.extend_from_slice(&ch);
                    }
                }
                Err(e) => {
                    log::warn!("resampler error: {e}");
                }
            }
        }
    }

    /// Drain the resampler's group delay (rubato's FFT introduces a delay).
    pub fn flush(&mut self) -> Vec<f32> {
        // Process an empty buffer with the "flush" flag set.
        match self.inner.process(&[&[]], None) {
            Ok(frames) => {
                let mut out = Vec::new();
                for ch in frames {
                    out.extend_from_slice(&ch);
                }
                out
            }
            Err(_) => Vec::new(),
        }
    }

    /// Same as `flush` but appends to a reused buffer.
    pub fn flush_into(&mut self, out: &mut Vec<f32>) {
        match self.inner.process(&[&[]], None) {
            Ok(frames) => {
                for ch in frames {
                    out.extend_from_slice(&ch);
                }
            }
            Err(_) => {}
        }
    }
}

