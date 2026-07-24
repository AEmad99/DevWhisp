//! Text formatter: post-transcription cleanup.
//!
//! Pipeline (in order):
//!   1. `trim` whitespace off both ends.
//!   2. Apply the `(from, to)` dictionary pairs in one pass, longest source
//!      first, case-insensitive and whole-word. Every hit is replaced with
//!      the literal `to` value, so "ai is cool" with `("ai", "AI")` becomes
//!      "AI is cool", and `("next js", "Next.js")` wins over `("js", ...)`.
//!   3. Capitalise the first character if `auto_capitalize` is true.
//!   4. Append a single trailing space if `append_space` is true and the
//!      string doesn't already end with whitespace.
//!
//! The dict replacement uses a regex-free whole-word matcher driven by
//! char boundaries. We treat any non-alphanumeric character as a word
//! boundary, matching the behaviour users expect for voice-driven dict
//! replacements ("ai" should match inside "ai is" but not inside "said").
//!
//! This module is intentionally independent of `inject::formatter` (which
//! is a thin trim/capitalise/space helper that already existed in
//! `src-tauri/src/inject/formatter.rs`). The inject-side helper stays as
//! a low-level utility; this one is the user-facing formatter that
//! consumes the persisted dictionary and runs on every transcription.

use serde::Deserialize;

/// User-tunable options for `format_transcript`.
#[derive(Debug, Clone, Deserialize)]
pub struct FormatOptions {
    #[serde(default = "default_true")]
    pub auto_capitalize: bool,
    #[serde(default = "default_true")]
    pub append_space: bool,
    #[serde(default = "default_false")]
    pub paste_uppercase: bool,
    /// Replacement pairs applied left-to-right, case-insensitive, whole-word.
    #[serde(default)]
    pub dict: Vec<(String, String)>,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            auto_capitalize: default_true(),
            append_space: default_true(),
            paste_uppercase: default_false(),
            dict: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

/// Run the full formatting pipeline on `text`.
pub fn format_transcript(text: &str, options: &FormatOptions) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut s = trimmed.to_string();
    if !options.dict.is_empty() {
        s = apply_dictionary(&s, &options.dict);
    }

    if options.auto_capitalize {
        capitalize_first(&mut s);
    }

    if options.append_space && !s.ends_with(char::is_whitespace) {
        s.push(' ');
    }

    // Top-tier auto punctuation (lightweight rules)
    if !s.trim().is_empty() && !s.trim_end().ends_with(|c: char| ".!?,".contains(c)) {
        let word_count = s.split_whitespace().count();
        if word_count >= 4 {
            s = s.trim_end().to_string() + ".";
        }
    }

    if options.paste_uppercase {
        s = s.to_uppercase();
    }

    s
}

/// Apply every dictionary pair to `input` in a single left-to-right pass,
/// trying **longest source first** at each position (plan §6 risk
/// mitigation: "Apply longest-match-first, case-insensitive matching").
///
/// Matching is case-insensitive and whole-word — whitespace, punctuation,
/// and string boundaries count as word boundaries, so `"ai"` matches inside
/// `"ai is"` but not inside `"said"`. Because we advance past the matched
/// *source* span and emit the replacement verbatim, a replacement's output
/// is never re-scanned: a short entry (`js`) can't clobber a longer one
/// (`next js` → `Next.js`), and replacement text can't trigger further
/// substitutions. Char-based so we avoid a regex dependency.
fn apply_dictionary(input: &str, dict: &[(String, String)]) -> String {
    // Order by source length, longest first. Stable, so equal-length entries
    // keep their insertion order.
    let mut pairs: Vec<(&str, &str)> = dict
        .iter()
        .filter(|(from, _)| !from.is_empty())
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect();
    pairs.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));

    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let mut hit = false;
        if is_word_boundary(&chars, i, true) {
            for (from, to) in &pairs {
                let from_count = from.chars().count();
                if i + from_count <= chars.len()
                    && is_word_boundary(&chars, i + from_count, false)
                {
                    let matched = from.chars().enumerate().all(|(offset, fc)| {
                        let actual = chars[i + offset].to_lowercase().next().unwrap_or('\0');
                        let expected = fc.to_lowercase().next().unwrap_or('\0');
                        actual == expected
                    });
                    if matched {
                        out.push_str(to);
                        i += from_count;
                        hit = true;
                        break;
                    }
                }
            }
        }
        if !hit {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Returns `true` if position `idx` in `chars` is at a word boundary.
/// `start` controls whether we treat the very beginning of the string as
/// a boundary (it always is when checking the left side of a match).
fn is_word_boundary(chars: &[char], idx: usize, start: bool) -> bool {
    if start {
        if idx == 0 {
            return true;
        }
        let prev = chars[idx - 1];
        return !prev.is_alphanumeric();
    } else {
        if idx >= chars.len() {
            return true;
        }
        let next = chars[idx];
        return !next.is_alphanumeric();
    }
}

/// Upper-case the first character of `s` in place. No-op on empty input.
/// Handles multi-byte first characters (e.g. accented letters) correctly.
fn capitalize_first(s: &mut String) {
    if s.is_empty() {
        return;
    }
    // Find the first char boundary.
    let first_char_len = s.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
    if first_char_len == 0 {
        return;
    }
    // SAFETY: we just measured the first char's byte length.
    let head = s[..first_char_len].to_string();
    let upper: String = head.chars().flat_map(|c| c.to_uppercase()).collect();
    s.replace_range(..first_char_len, &upper);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(auto_cap: bool, append: bool, uppercase: bool, dict: Vec<(&str, &str)>) -> FormatOptions {
        FormatOptions {
            auto_capitalize: auto_cap,
            append_space: append,
            paste_uppercase: uppercase,
            dict: dict
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    #[test]
    fn capitalizes_hello_world_dot() {
        let s = format_transcript("hello world.", &opts(true, false, false, vec![]));
        assert_eq!(s, "Hello world.");
    }

    #[test]
    fn appends_trailing_space() {
        let s = format_transcript("hello", &opts(false, true, false, vec![]));
        assert_eq!(s, "hello ");
        // Don't double-append when input already ends with whitespace.
        let s2 = format_transcript("hello. ", &opts(false, true, false, vec![]));
        assert_eq!(s2, "hello. ");
    }

    #[test]
    fn no_trailing_space_when_disabled() {
        let s = format_transcript("hello", &opts(false, false, false, vec![]));
        assert_eq!(s, "hello");
    }

    #[test]
    fn dict_case_insensitive_whole_word_ai() {
        let s = format_transcript(
            "ai is cool",
            &opts(false, false, false, vec![("ai", "AI")]),
        );
        assert_eq!(s, "AI is cool");
        // Mixed case input also triggers.
        let s2 = format_transcript(
            "Ai is cooler than AI but said ai",
            &opts(false, false, false, vec![("ai", "AI")]),
        );
        // "said" must not be mangled (whole-word rule).
        assert_eq!(s2, "AI is cooler than AI but said AI.");
    }

    #[test]
    fn dict_skips_substring_hits() {
        // "ai" should NOT match inside "said" or "tail".
        let s = format_transcript(
            "she said no to the tail",
            &opts(false, false, false, vec![("ai", "AI")]),
        );
        assert_eq!(s, "she said no to the tail.");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(format_transcript("", &opts(true, true, false, vec![])), "");
        assert_eq!(format_transcript("   ", &opts(true, true, false, vec![])), "");
        assert_eq!(format_transcript("\n\t", &opts(true, true, false, vec![])), "");
    }

    #[test]
    fn multiple_consecutive_dict_hits() {
        let s = format_transcript(
            "ts and js and ts and js",
            &opts(false, false, false, vec![("ts", "TypeScript"), ("js", "JavaScript")]),
        );
        assert_eq!(
            s,
            "TypeScript and JavaScript and TypeScript and JavaScript."
        );
    }

    #[test]
    fn capitalization_with_dict() {
        let s = format_transcript(
            "ai is great",
            &opts(true, false, false, vec![("ai", "AI")]),
        );
        assert_eq!(s, "AI is great");
    }

    #[test]
    fn trim_before_processing() {
        let s = format_transcript("   hello world  ", &opts(true, false, false, vec![]));
        assert_eq!(s, "Hello world");
    }

    #[test]
    fn empty_dict_pair_is_skipped() {
        // Should not infinite-loop or panic.
        let s = format_transcript("hello", &opts(false, false, false, vec![("", "X")]));
        assert_eq!(s, "hello");
    }

    #[test]
    fn dict_applies_longest_match_first() {
        // The longer multi-word entry must win over the shorter one even when
        // the shorter one is listed first (plan §6: longest-match-first).
        let s = format_transcript(
            "next js rocks",
            &opts(false, false, false, vec![("js", "JavaScript"), ("next js", "Next.js")]),
        );
        assert_eq!(s, "Next.js rocks");
        // Replacement output is never re-scanned: the "js" inside "Next.js"
        // is not substituted again.
    }

    #[test]
    fn dict_longest_match_independent_of_order() {
        // Same result regardless of insertion order.
        let a = format_transcript(
            "deploy to k8s now",
            &opts(false, false, false, vec![("k8s", "Kubernetes"), ("deploy to k8s", "ship it")]),
        );
        assert_eq!(a, "ship it now");
    }

    #[test]
    fn converts_to_all_uppercase_when_enabled() {
        let s = format_transcript("hello world", &opts(false, false, true, vec![]));
        assert_eq!(s, "HELLO WORLD");
        let s2 = format_transcript("hello world", &opts(true, true, true, vec![]));
        assert_eq!(s2, "HELLO WORLD ");
    }
}