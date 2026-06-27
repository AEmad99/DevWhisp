//! Text formatting: capitalize, dictionary replace, append space.
//!
//! Phase 1 (T2.7) — implemented in a later task. For now this is a stub that
//! trims and returns the input as-is.

#![allow(dead_code)]

/// Apply DevWhisp's user-configurable text transformations to a transcription.
pub fn format(raw: &str, capitalize_first: bool, append_space: bool) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut s = trimmed.to_string();
    if capitalize_first {
        if let Some(first) = s.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
    }
    if append_space && !s.ends_with(char::is_whitespace) {
        s.push(' ');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_basic() {
        assert_eq!(format("hello world", false, false), "hello world");
        assert_eq!(format("  hello  ", true, false), "Hello");
        assert_eq!(format("hello", false, true), "hello ");
    }
}
