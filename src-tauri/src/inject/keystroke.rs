//! Keystroke simulation primitives (enigo wrapper).
//!
//! Used as a fallback if clipboard paste doesn't work in a specific app.
//! Slower than clipboard paste but doesn't require clipboard round-tripping.

#![allow(dead_code)]

use anyhow::Result;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

/// Type a sequence of characters one at a time.
pub fn type_text(text: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    for ch in text.chars() {
        enigo.key(Key::Unicode(ch), Direction::Click)?;
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Ok(())
}
