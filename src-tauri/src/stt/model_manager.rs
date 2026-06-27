//! Model manager: download and activate STT model files.
//!
//! Phase 1 — Whisper (ggml format from `ggerganov/whisper.cpp`).
//! Phase 2+ — Moonshine (ONNX format from `UsefulSensors/moonshine`).
//!
//! Models live at `~/.devwhisp/models/<variant>/`. The active variant is
//! tracked by an `active.txt` file in that directory.

#![allow(dead_code)]

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelVariant {
    /// Whisper tiny.en (English only) — ~75 MB, fastest, baseline accuracy.
    WhisperTinyEn,
    /// Moonshine Tiny (English) — ~50 MB, faster + more accurate than Whisper tiny.
    MoonshineTiny,
}

impl ModelVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WhisperTinyEn => "whisper-tiny-en",
            Self::MoonshineTiny => "moonshine-tiny",
        }
    }

    pub fn approx_size_mb(&self) -> u32 {
        match self {
            Self::WhisperTinyEn => 75,
            Self::MoonshineTiny => 50,
        }
    }

    /// Primary model file (the file whisper/onnx loads directly).
    /// This is the path the STT runner passes to its backend.
    pub fn model_file(&self, dir: &Path) -> PathBuf {
        match self {
            Self::WhisperTinyEn => dir.join("ggml-tiny.en.bin"),
            Self::MoonshineTiny => dir.join("onnx").join("encoder.onnx"),
        }
    }

    /// Files that this variant needs on disk (relative to the model dir).
    pub fn files(&self) -> &'static [&'static str] {
        match self {
            Self::WhisperTinyEn => &["ggml-tiny.en.bin"],
            Self::MoonshineTiny => &["onnx/encoder.onnx", "onnx/decoder.onnx", "tokenizer.json"],
        }
    }

    /// Remote URL to download the variant's files from. The first entry is
    /// the source URL; the rest are derived filenames.
    pub fn source_url(&self) -> &'static str {
        match self {
            Self::WhisperTinyEn => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
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
    let variant = match trimmed {
        "whisper-tiny-en" => Some(ModelVariant::WhisperTinyEn),
        "moonshine-tiny" => Some(ModelVariant::MoonshineTiny),
        _ => None,
    }?;
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
        ModelVariant::WhisperTinyEn => {
            let url = variant.source_url();
            let filename = variant.files()[0];
            let target = dest.join(filename);
            if target.exists() {
                log::info!("{} already present at {:?}", filename, target);
            } else {
                log::info!("downloading {} ({} MB) from {}", filename, variant.approx_size_mb(), url);
                let app1 = app.clone();
                let vname = variant.as_str().to_string();
                download_file(url, &target, move |pct, dl, tot| {
                    log::debug!("progress {:.1}%", pct);
                    if let Some(ref a) = app1 {
                        let _ = a.emit("model-download-progress", serde_json::json!({
                            "variant": vname,
                            "pct": pct,
                            "downloadedMB": dl / 1_000_000,
                            "totalMB": if tot > 0 { tot / 1_000_000 } else { variant.approx_size_mb() as u64 }
                        }));
                    }
                }).await?;
            }
        }
        ModelVariant::MoonshineTiny => {
            // Use the community ONNX export which is reliably available
            // https://huggingface.co/onnx-community/moonshine-tiny-ONNX
            let repo = "onnx-community/moonshine-tiny-ONNX";
            let base = format!("https://huggingface.co/{}/resolve/main", repo);

            // Map remote file -> local expected name (to match moonshine.rs loader)
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
                download_file(&url, &target, move |pct, dl, tot| {
                    log::debug!("Moonshine {} progress {:.1}%", local_for_log, pct);
                    if let Some(ref a) = app2 {
                        let _ = a.emit("model-download-progress", serde_json::json!({
                            "variant": vname2,
                            "pct": pct,
                            "downloadedMB": dl / 1_000_000,
                            "totalMB": if tot > 0 { tot / 1_000_000 } else { variant.approx_size_mb() as u64 }
                        }));
                    }
                }).await?;
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
/// advertised / accumulated size exceeds `max_bytes`. The hard ceiling is
/// 2× the largest known model variant to leave headroom for ONNX bundles
/// and tokenizer metadata.
async fn download_file<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<()>
where
    F: FnMut(f64, u64, u64) + Send + 'static,
{
    use futures_util::StreamExt;

    // Hard ceiling: 2× the largest variant we ship. Moonshine total is ~150 MB
    // across three files, but each individual download is smaller — 250 MB
    // gives comfortable headroom for any reasonable bundle.
    const MAX_DOWNLOAD_BYTES: u64 = 250 * 1_000_000;

    let client = reqwest::Client::builder()
        .user_agent("DevWhisp/0.1")
        .build()
        .context("failed to build reqwest client")?;

    let resp = client
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

    let mut stream = resp.bytes_stream();
    let mut file = std::fs::File::create(dest).context("failed to create destination file")?;
    let mut downloaded: u64 = 0;
    let mut last_logged: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download stream error")?;
        downloaded += chunk.len() as u64;
        if downloaded > MAX_DOWNLOAD_BYTES {
            return Err(anyhow!(
                "download exceeded maximum {} bytes mid-stream; aborting",
                MAX_DOWNLOAD_BYTES
            ));
        }
        std::io::Write::write_all(&mut file, &chunk).context("failed to write to file")?;

        let pct = if total > 0 { (downloaded as f64 / total as f64) * 100.0 } else { 0.0 };
        on_progress(pct, downloaded, total);

        if total > 0 && downloaded - last_logged >= 3_000_000 {
            log::info!(
                "  download progress: {:.1}% ({:.1} MB / {:.1} MB)",
                pct,
                downloaded as f64 / 1_000_000.0,
                total as f64 / 1_000_000.0
            );
            last_logged = downloaded;
        }
    }
    log::info!("download complete: {:?}", dest);
    Ok(())
}

// Bundled model support has been removed per the in-app download strategy.
// The app now requires an explicit download via the UI (first-run or Settings).
// These functions are kept as no-ops / deprecated for a clean transition.
// Remove calls from lib.rs and callers.

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
