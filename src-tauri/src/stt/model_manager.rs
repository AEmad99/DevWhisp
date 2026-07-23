//! Model manager: download and activate STT model files.
//!
//! Primary engine: Whisper via whisper.cpp (`whisper-rs`), matching BridgeVoice's
//! local model ladder (Tiny → Large-v3 + Distil-Large).
//! Optional: Moonshine Tiny (ONNX) behind the `moonshine` cargo feature.
//!
//! Models live at `~/.devwhisp/models/<variant>/`. The active variant is
//! tracked by an `active.txt` file in that directory.

#![allow(dead_code)]

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

static HTTP_CLIENT: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent("DevWhisp/0.1")
        .build()
        .expect("failed to build reqwest client")
});

/// Hard ceiling for a single download. Large-v3 is ~3.1 GB; leave headroom.
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelVariant {
    /// Whisper tiny.en — ~75 MB, fastest, basic accuracy.
    WhisperTinyEn,
    /// Whisper base.en — ~142 MB, BridgeVoice's recommended default.
    WhisperBaseEn,
    /// Whisper small.en — ~466 MB, better accuracy for longer dictation.
    WhisperSmallEn,
    /// Whisper medium.en — ~1.5 GB, high accuracy.
    WhisperMediumEn,
    /// Whisper large-v3 (multilingual) — ~3.1 GB, maximum Whisper accuracy.
    WhisperLargeV3,
    /// Distil-Whisper large-v3 — ~1.5 GB, best speed-to-accuracy ratio.
    WhisperDistilLargeV3,
    /// Moonshine Tiny (English) — ~50 MB, ONNX edge model (feature-gated).
    MoonshineTiny,
}

impl ModelVariant {
    /// All variants shown in the model picker (BridgeVoice-style ladder + Moonshine).
    pub const ALL: &'static [ModelVariant] = &[
        Self::WhisperTinyEn,
        Self::WhisperBaseEn,
        Self::WhisperSmallEn,
        Self::WhisperMediumEn,
        Self::WhisperLargeV3,
        Self::WhisperDistilLargeV3,
        Self::MoonshineTiny,
    ];

    /// Parse a variant id string (as stored in `active.txt` / IPC).
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "whisper-tiny-en" => Some(Self::WhisperTinyEn),
            "whisper-base-en" => Some(Self::WhisperBaseEn),
            "whisper-small-en" => Some(Self::WhisperSmallEn),
            "whisper-medium-en" => Some(Self::WhisperMediumEn),
            "whisper-large-v3" => Some(Self::WhisperLargeV3),
            "whisper-distil-large-v3" => Some(Self::WhisperDistilLargeV3),
            "moonshine-tiny" => Some(Self::MoonshineTiny),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WhisperTinyEn => "whisper-tiny-en",
            Self::WhisperBaseEn => "whisper-base-en",
            Self::WhisperSmallEn => "whisper-small-en",
            Self::WhisperMediumEn => "whisper-medium-en",
            Self::WhisperLargeV3 => "whisper-large-v3",
            Self::WhisperDistilLargeV3 => "whisper-distil-large-v3",
            Self::MoonshineTiny => "moonshine-tiny",
        }
    }

    /// Short UI label (BridgeVoice-style names).
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::WhisperTinyEn => "Tiny",
            Self::WhisperBaseEn => "Base",
            Self::WhisperSmallEn => "Small",
            Self::WhisperMediumEn => "Medium",
            Self::WhisperLargeV3 => "Large",
            Self::WhisperDistilLargeV3 => "Distil-Large",
            Self::MoonshineTiny => "Moonshine Tiny",
        }
    }

    /// One-line description for the settings picker.
    pub fn description(&self) -> &'static str {
        match self {
            Self::WhisperTinyEn => "Fastest · basic accuracy · quick notes & commands",
            Self::WhisperBaseEn => "Fast · good accuracy · general dictation (recommended)",
            Self::WhisperSmallEn => "Moderate · better accuracy · longer dictation",
            Self::WhisperMediumEn => "Slower · great accuracy · detailed transcription",
            Self::WhisperLargeV3 => "Slowest · best Whisper accuracy · multilingual",
            Self::WhisperDistilLargeV3 => "Fast for its size · great accuracy · best speed/quality ratio",
            Self::MoonshineTiny => "Edge ONNX model · very fast on CPU (needs moonshine build)",
        }
    }

    pub fn approx_size_mb(&self) -> u32 {
        self.expected_size_mb()
    }

    /// Expected on-disk size in MB for readiness validation.
    pub fn expected_size_mb(&self) -> u32 {
        match self {
            Self::WhisperTinyEn => 75,
            Self::WhisperBaseEn => 142,
            Self::WhisperSmallEn => 466,
            Self::WhisperMediumEn => 1536,
            Self::WhisperLargeV3 => 3100,
            Self::WhisperDistilLargeV3 => 1520,
            Self::MoonshineTiny => 50,
        }
    }

    /// True for English-only Whisper checkpoints (`.en`).
    pub fn is_english_only(&self) -> bool {
        matches!(
            self,
            Self::WhisperTinyEn
                | Self::WhisperBaseEn
                | Self::WhisperSmallEn
                | Self::WhisperMediumEn
                | Self::MoonshineTiny
        )
    }

    /// Whether this is a whisper.cpp / ggml model.
    pub fn is_whisper(&self) -> bool {
        !matches!(self, Self::MoonshineTiny)
    }

    /// Primary model file (the file whisper/onnx loads directly).
    pub fn model_file(&self, dir: &Path) -> PathBuf {
        match self {
            Self::WhisperTinyEn => dir.join("ggml-tiny.en.bin"),
            Self::WhisperBaseEn => dir.join("ggml-base.en.bin"),
            Self::WhisperSmallEn => dir.join("ggml-small.en.bin"),
            Self::WhisperMediumEn => dir.join("ggml-medium.en.bin"),
            Self::WhisperLargeV3 => dir.join("ggml-large-v3.bin"),
            Self::WhisperDistilLargeV3 => dir.join("ggml-distil-large-v3.bin"),
            Self::MoonshineTiny => dir.join("onnx").join("encoder.onnx"),
        }
    }

    /// Files that this variant needs on disk (relative to the model dir).
    pub fn files(&self) -> &'static [&'static str] {
        match self {
            Self::WhisperTinyEn => &["ggml-tiny.en.bin"],
            Self::WhisperBaseEn => &["ggml-base.en.bin"],
            Self::WhisperSmallEn => &["ggml-small.en.bin"],
            Self::WhisperMediumEn => &["ggml-medium.en.bin"],
            Self::WhisperLargeV3 => &["ggml-large-v3.bin"],
            Self::WhisperDistilLargeV3 => &["ggml-distil-large-v3.bin"],
            Self::MoonshineTiny => &["onnx/encoder.onnx", "onnx/decoder.onnx", "tokenizer.json"],
        }
    }

    /// Remote URL for single-file Whisper downloads.
    pub fn source_url(&self) -> &'static str {
        match self {
            Self::WhisperTinyEn => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
            }
            Self::WhisperBaseEn => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
            }
            Self::WhisperSmallEn => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin"
            }
            Self::WhisperMediumEn => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin"
            }
            Self::WhisperLargeV3 => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin"
            }
            Self::WhisperDistilLargeV3 => {
                "https://huggingface.co/distil-whisper/distil-large-v3-ggml/resolve/main/ggml-distil-large-v3.bin"
            }
            Self::MoonshineTiny => "https://huggingface.co/onnx-community/moonshine-tiny-ONNX",
        }
    }
}

/// Resolved path to the currently-active model FILE (the file the STT
/// backend opens directly). Returns `None` if no model is downloaded.
pub fn active_model_path() -> Option<PathBuf> {
    let root = models_root()?;
    let name = std::fs::read_to_string(root.join("active.txt")).ok()?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let variant = ModelVariant::from_id(trimmed)?;
    let dir = root.join(trimmed);
    if !dir.is_dir() {
        return None;
    }
    let file = variant.model_file(&dir);
    if file.is_file() {
        Some(file)
    } else {
        None
    }
}

/// Currently active variant, if `active.txt` is valid.
pub fn active_variant() -> Option<ModelVariant> {
    let root = models_root()?;
    let name = std::fs::read_to_string(root.join("active.txt")).ok()?;
    ModelVariant::from_id(name.trim())
}

/// Resolved path to the currently-active model DIRECTORY (for display in
/// the UI). Returns `None` if no model is downloaded.
pub fn active_model_dir() -> Option<PathBuf> {
    let root = models_root()?;
    let name = std::fs::read_to_string(root.join("active.txt")).ok()?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = root.join(trimmed);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

/// Root directory for downloaded models: `~/.devwhisp/models/`.
pub fn models_root() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let p = home.join(".devwhisp").join("models");
    if p.exists() {
        Some(p)
    } else {
        let _ = std::fs::create_dir_all(&p);
        Some(p)
    }
}

/// Mark a model variant as the active one.
pub fn set_active(variant: ModelVariant) -> Result<()> {
    let root = models_root().ok_or_else(|| anyhow!("no home dir"))?;
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("active.txt"), variant.as_str())?;
    log::info!("active model set to: {}", variant.as_str());
    Ok(())
}

/// Download a model variant. Streams the file(s) into the models dir with
/// a simple progress callback. Returns the path to the model directory.
/// Optional app handle enables real-time "model-download-progress" events.
pub async fn download(variant: ModelVariant, app: Option<tauri::AppHandle>) -> Result<PathBuf> {
    let root = models_root().ok_or_else(|| anyhow!("no home dir"))?;
    let dest = root.join(variant.as_str());
    std::fs::create_dir_all(&dest)?;

    match variant {
        ModelVariant::MoonshineTiny => {
            // Community ONNX export — https://huggingface.co/onnx-community/moonshine-tiny-ONNX
            let repo = "onnx-community/moonshine-tiny-ONNX";
            let base = format!("https://huggingface.co/{}/resolve/main", repo);

            let downloads: &[(&str, &str)] = &[
                ("onnx/encoder_model.onnx", "onnx/encoder.onnx"),
                ("onnx/decoder_model_merged_quantized.onnx", "onnx/decoder.onnx"),
                ("tokenizer.json", "tokenizer.json"),
            ];

            for (remote, local) in downloads {
                let target = if local.contains('/') {
                    let parent = dest.join(local).parent().unwrap().to_path_buf();
                    std::fs::create_dir_all(&parent)?;
                    dest.join(local)
                } else {
                    dest.join(local)
                };
                if target.exists() {
                    log::info!("Moonshine file already present: {:?}", target);
                    continue;
                }
                let url = format!("{}/{}", base, remote);
                log::info!("Downloading Moonshine {} from {}", local, url);
                let local_for_log = local.to_string();
                let app2 = app.clone();
                let vname2 = variant.as_str().to_string();
                let approx = variant.approx_size_mb() as u64;
                download_file(&url, &target, move |pct, dl, tot| {
                    log::debug!("Moonshine {} progress {:.1}%", local_for_log, pct);
                    if let Some(ref a) = app2 {
                        let _ = a.emit(
                            "model-download-progress",
                            serde_json::json!({
                                "variant": vname2,
                                "pct": pct,
                                "downloadedMB": dl / 1_000_000,
                                "totalMB": if tot > 0 { tot / 1_000_000 } else { approx }
                            }),
                        );
                    }
                })
                .await?;
            }
        }
        // All Whisper / Distil-Whisper ggml single-file downloads.
        _ => {
            let url = variant.source_url();
            let filename = variant.files()[0];
            let target = dest.join(filename);
            if target.exists() {
                log::info!("{} already present at {:?}", filename, target);
            } else {
                log::info!(
                    "downloading {} ({} MB) from {}",
                    filename,
                    variant.approx_size_mb(),
                    url
                );
                let app1 = app.clone();
                let vname = variant.as_str().to_string();
                let approx = variant.approx_size_mb() as u64;
                download_file(url, &target, move |pct, dl, tot| {
                    log::debug!("progress {:.1}%", pct);
                    if let Some(ref a) = app1 {
                        let _ = a.emit(
                            "model-download-progress",
                            serde_json::json!({
                                "variant": vname,
                                "pct": pct,
                                "downloadedMB": dl / 1_000_000,
                                "totalMB": if tot > 0 { tot / 1_000_000 } else { approx }
                            }),
                        );
                    }
                })
                .await?;
            }
        }
    }

    set_active(variant)?;
    Ok(dest)
}

/// Stream a single file from a URL into a destination path with progress logging and callback for UI.
///
/// Security: refuses to download if the server doesn't advertise a
/// `Content-Length` header (defeats naive streaming attacks) or if the
/// advertised / accumulated size exceeds `MAX_DOWNLOAD_BYTES`.
async fn download_file<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<()>
where
    F: FnMut(f64, u64, u64) + Send + 'static,
{
    use futures_util::StreamExt;

    let resp = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .context("download request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow!("download failed with HTTP {}", resp.status()));
    }

    let total = match resp.content_length() {
        Some(n) if n > 0 => n,
        _ => {
            return Err(anyhow!(
                "server did not provide Content-Length; refusing to stream an unbounded download from {url}"
            ));
        }
    };
    if total > MAX_DOWNLOAD_BYTES {
        return Err(anyhow!(
            "advertised content length {} exceeds maximum {}",
            total,
            MAX_DOWNLOAD_BYTES
        ));
    }

    // Write to a temp file next to the destination, then rename on success so
    // a cancelled/partial download never leaves a "ready-looking" model.
    let tmp = dest.with_extension("bin.partial");
    if tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }

    let mut stream = resp.bytes_stream();
    let mut file = std::fs::File::create(&tmp).context("failed to create destination file")?;
    let mut downloaded: u64 = 0;
    let mut last_logged: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download stream error")?;
        downloaded += chunk.len() as u64;
        if downloaded > MAX_DOWNLOAD_BYTES {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!(
                "download exceeded maximum {} bytes mid-stream; aborting",
                MAX_DOWNLOAD_BYTES
            ));
        }
        std::io::Write::write_all(&mut file, &chunk).context("failed to write to file")?;

        let pct = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        on_progress(pct, downloaded, total);

        if total > 0 && downloaded - last_logged >= 5_000_000 {
            log::info!(
                "  download progress: {:.1}% ({:.1} MB / {:.1} MB)",
                pct,
                downloaded as f64 / 1_000_000.0,
                total as f64 / 1_000_000.0
            );
            last_logged = downloaded;
        }
    }

    // Flush and promote partial → final.
    std::io::Write::flush(&mut file).ok();
    drop(file);
    std::fs::rename(&tmp, dest).with_context(|| {
        format!(
            "failed to rename partial download {:?} → {:?}",
            tmp, dest
        )
    })?;

    log::info!("download complete: {:?}", dest);
    Ok(())
}

// Bundled model support has been removed per the in-app download strategy.
// The app now requires an explicit download via the UI (first-run or Settings).

#[deprecated(note = "Bundling removed. Use in-app download via download() + UI progress.")]
pub fn bundled_model_path(_app: &AppHandle) -> Option<PathBuf> {
    log::debug!("bundled_model_path called but bundling is disabled");
    None
}

#[deprecated(note = "Bundling removed. Models are downloaded inside the app.")]
pub fn ensure_bundled_model(_app: &AppHandle) -> Result<()> {
    log::info!("ensure_bundled_model: bundling disabled — in-app download is the only path");
    Ok(())
}
