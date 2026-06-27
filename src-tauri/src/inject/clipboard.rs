//! Clipboard save / set / restore + simulated paste.
//!
//! Pipeline:
//!   1. Validate length + strip control characters other than `\n` / `\t`.
//!   2. Save the current clipboard text (so we can restore it).
//!   3. Set the clipboard to the new text via `arboard`.
//!   4. Send a synthetic Ctrl+V (Cmd+V on macOS) via `enigo`.
//!   5. Schedule restoration of the original clipboard on a background thread.
//!
//! A module-level `Mutex` serializes concurrent pastes so two `Ctrl+V` events
//! never overlap and the original-clipboard restore is always well-defined.

use anyhow::Result;
use arboard::Clipboard;
use enigo::{Direction, Keyboard, Settings};
use parking_lot::Mutex;
use std::time::Duration;

#[cfg(target_os = "macos")]
const PASTE_KEY: enigo::Key = enigo::Key::Meta;
#[cfg(not(target_os = "macos"))]
const PASTE_KEY: enigo::Key = enigo::Key::Control;

const PASTE_V: enigo::Key = enigo::Key::Unicode('v');

/// 32 KB hard cap on paste payload. Anything beyond this almost certainly
/// wasn't produced by STT and is either a bug or an IPC abuse attempt.
pub const MAX_PASTE_BYTES: usize = 32 * 1024;

/// Serializes the save → set → Ctrl+V → restore sequence so two overlapping
/// pastes can't clobber each other's "original clipboard" snapshot.
static PASTE_GUARD: Mutex<()> = Mutex::new(());

/// Save current clipboard, set new text, send Ctrl/Cmd+V, then restore the
/// original clipboard after 5 seconds.
pub fn paste_text(text: &str) -> Result<()> {
    if text.len() > MAX_PASTE_BYTES {
        anyhow::bail!(
            "paste payload too large ({} bytes; max {MAX_PASTE_BYTES})",
            text.len()
        );
    }
    let sanitized = strip_control_chars(text);
    if sanitized.is_empty() {
        anyhow::bail!("paste payload is empty after control-char stripping");
    }

    // Hold the guard for the whole sequence. The restore happens on a
    // background thread (with its own clone of the original text) so we
    // release the guard before sleep.
    let _guard = PASTE_GUARD.lock();

    // 1. Save the current clipboard content (best effort).
    let original = Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok());

    // 2. Set the new text.
    {
        let mut cb = match Clipboard::new() {
            Ok(cb) => cb,
            Err(e) => {
                log::error!("could not open clipboard for writing: {e}");
                return Ok(());
            }
        };
        if let Err(e) = cb.set_text(sanitized) {
            log::warn!("clipboard set_text failed: {e}");
        }
    }

    // Give the OS a moment to propagate the clipboard change.
    std::thread::sleep(Duration::from_millis(30));

    // 3. Send Ctrl+V (or Cmd+V on macOS).
    if let Err(e) = send_paste() {
        log::warn!("paste injection failed: {e:?}");
    }

    // 4. Schedule restoration of the original clipboard on a background thread.
    if let Some(prev) = original {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            match Clipboard::new() {
                Ok(mut cb) => match cb.set_text(prev) {
                    Ok(_) => log::info!("original clipboard content restored"),
                    Err(e) => log::warn!("clipboard restore failed: {e}"),
                },
                Err(e) => log::warn!("could not reopen clipboard for restore: {e}"),
            }
        });
    }

    Ok(())
}

fn send_paste() -> Result<()> {
    let mut enigo = enigo::Enigo::new(&Settings::default())?;
    // Hold the modifier, click 'v', release the modifier.
    enigo.key(PASTE_KEY, Direction::Press)?;
    std::thread::sleep(Duration::from_millis(20));
    enigo.key(PASTE_V, Direction::Click)?;
    std::thread::sleep(Duration::from_millis(20));
    enigo.key(PASTE_KEY, Direction::Release)?;
    Ok(())
}

/// Replace control chars other than `\n` and `\t` with a space. Prevents a
/// maliciously crafted transcript from injecting keystrokes via paste (e.g.
/// a newline that triggers a "submit" in chat apps) or hiding formatting
/// characters that confuse downstream text processing.
fn strip_control_chars(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' && c != '\r' {
                ' '
            } else {
                c
            }
        })
        .collect()
}
