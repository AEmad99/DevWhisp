//! Text injection: clipboard save/set/paste + simulated keystroke.
//!
//! Phase 1 (T1.5) — real `arboard` + `enigo` wiring:
//!   - `inject(text)` -> `clipboard::paste_text(text)` (clipboard round-trip + Ctrl+V)
//!   - `keystroke::type_text(text)` (direct keystroke fallback, slower)

pub mod clipboard;
pub mod formatter;
pub mod keystroke;

use anyhow::Result;

/// Inject `text` into the currently-focused app via clipboard paste.
pub fn inject(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    log::info!(
        "injecting text: len={} preview={:?}",
        text.len(),
        preview(text)
    );
    clipboard::paste_text(text)
}

fn preview(s: &str) -> String {
    if s.len() <= 60 {
        s.to_string()
    } else {
        format!("{}…", &s[..60])
    }
}
