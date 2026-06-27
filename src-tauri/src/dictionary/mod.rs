//! Persistent user dictionary.
//!
//! Stored as JSON at `~/.devwhisp/dictionary.json`. Schema:
//!
//! ```json
//! { "replacements": [{"from": "ai", "to": "AI"}, ...] }
//! ```
//!
//! Atomic writes: serialise to `<path>.tmp`, then `rename` over the
//! target. On Windows, `rename` over an existing file is allowed since
//! Rust 1.5+ via `std::fs::rename` (it uses `MoveFileEx` with the
//! `MOVEFILE_REPLACE_EXISTING` flag).

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One replacement entry as it lives on disk and over the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DictEntry {
    pub from: String,
    pub to: String,
}

/// On-disk schema. Keeps a top-level object so we can add other fields
/// later (e.g. enabled, last_used_at) without breaking parsers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DictFile {
    #[serde(default)]
    replacements: Vec<DictEntry>,
}

impl Default for DictFile {
    fn default() -> Self {
        Self {
            replacements: Vec::new(),
        }
    }
}

/// Process-wide cache of the last-loaded list. Avoids re-reading the
/// file on every IPC call.
static CACHE: once_cell::sync::Lazy<Mutex<Vec<DictEntry>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(Vec::new()));

/// Test-only override for the dictionary path. Same reasoning as
/// `TEST_DB_PATH_OVERRIDE` in `history::db`: `dirs::home_dir()` on
/// Windows uses the registry, not env vars, so we need an explicit
/// path hook for test isolation.
static TEST_DICT_PATH_OVERRIDE: once_cell::sync::Lazy<Mutex<Option<PathBuf>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// Set or clear the test-only dictionary path override. Production
/// code never calls this.
#[cfg(test)]
pub fn _set_test_dict_path(p: Option<PathBuf>) {
    *TEST_DICT_PATH_OVERRIDE.lock() = p;
}

/// Absolute path to the dictionary JSON file.
pub fn dictionary_path() -> PathBuf {
    if let Some(p) = TEST_DICT_PATH_OVERRIDE.lock().clone() {
        return p;
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".devwhisp")
        .join("dictionary.json")
}

/// Load the dictionary from disk (or return the default empty list).
/// Result is cached in `CACHE` for the rest of the process lifetime;
/// every mutation (`add` / `remove` / `save`) keeps the cache in sync.
pub fn load() -> Result<Vec<(String, String)>> {
    let entries = read_from_disk(&dictionary_path())?;
    let pairs: Vec<(String, String)> = entries
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    *CACHE.lock() = entries;
    mark_cache_initialised();
    Ok(pairs)
}

/// Persist `entries` to disk atomically and refresh the cache.
pub fn save(entries: &[DictEntry]) -> Result<()> {
    atomic_write(&dictionary_path(), &DictFile {
        replacements: entries.to_vec(),
    })?;
    *CACHE.lock() = entries.to_vec();
    mark_cache_initialised();
    Ok(())
}

/// Add (or update) a `(from, to)` pair. If `from` already exists its
/// `to` value is overwritten. Returns the new full list of entries.
pub fn add(from: &str, to: &str) -> Result<Vec<DictEntry>> {
    if from.is_empty() {
        anyhow::bail!("dictionary entry `from` cannot be empty");
    }
    let mut current = read_from_disk(&dictionary_path())?;
    if let Some(existing) = current.iter_mut().find(|e| e.from == from) {
        existing.to = to.to_string();
    } else {
        current.push(DictEntry {
            from: from.to_string(),
            to: to.to_string(),
        });
    }
    save(&current)?;
    Ok(current)
}

/// Remove the entry whose `from` matches `from`. Returns the new full
/// list of entries.
pub fn remove(from: &str) -> Result<Vec<DictEntry>> {
    let mut current = read_from_disk(&dictionary_path())?;
    let before = current.len();
    current.retain(|e| e.from != from);
    if current.len() == before {
        // Nothing was removed; still a no-op success.
        return Ok(current);
    }
    save(&current)?;
    Ok(current)
}

/// Return the current dictionary. Reads from the cache if populated,
/// otherwise loads from disk.
pub fn list() -> Result<Vec<DictEntry>> {
    if cached_path_was_initialised() {
        return Ok(CACHE.lock().clone());
    }
    let entries = read_from_disk(&dictionary_path())?;
    *CACHE.lock() = entries.clone();
    mark_cache_initialised();
    Ok(entries)
}

/// Read + parse the dictionary file. Missing file -> empty list.
fn read_from_disk(path: &Path) -> Result<Vec<DictEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read dictionary file {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: DictFile = serde_json::from_str(&raw)
        .with_context(|| format!("parse dictionary file {}", path.display()))?;
    Ok(parsed.replacements)
}

/// True iff `load()` (or any mutation) has populated the cache for this
/// process lifetime. We track this with a dedicated flag because the
/// cache itself is an empty `Vec` when the dictionary is empty — we'd
/// otherwise re-read the file on every `list()` call.
static CACHE_INITIALISED: once_cell::sync::Lazy<Mutex<bool>> =
    once_cell::sync::Lazy::new(|| Mutex::new(false));

fn cached_path_was_initialised() -> bool {
    *CACHE_INITIALISED.lock()
}

fn mark_cache_initialised() {
    *CACHE_INITIALISED.lock() = true;
}

/// Atomic write: serialise to `<path>.tmp`, fsync, then rename.
fn atomic_write(path: &Path, file: &DictFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dict dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(file).context("serialise dictionary")?;
    std::fs::write(&tmp, json.as_bytes())
        .with_context(|| format!("write dictionary tmp {}", tmp.display()))?;
    // Best-effort fsync for crash safety; ignored on platforms where
    // File::sync_all isn't meaningful.
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Module-global lock that serialises every dictionary test.
    /// Same reason as in `history::db::tests`: `CACHE`,
    /// `CACHE_INITIALISED`, and `TEST_DICT_PATH_OVERRIDE` are
    /// process-global, so parallel tests would stomp each other.
    static TEST_LOCK: once_cell::sync::Lazy<parking_lot::Mutex<()>> =
        once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(()));

    /// Each test gets a fresh temp dictionary file under a unique
    /// tempdir so the real ~/.devwhisp/dictionary.json is never
    /// touched. Uses the `_set_test_dict_path` override because
    /// `dirs::home_dir()` on Windows ignores env vars.
    fn with_temp_home<F: FnOnce()>(f: F) {
        let _guard = TEST_LOCK.lock();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("devwhisp-dict-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dictionary.json");
        _set_test_dict_path(Some(path));

        // Reset caches so the new path takes effect.
        CACHE.lock().clear();
        *CACHE_INITIALISED.lock() = false;

        f();

        let _ = std::fs::remove_dir_all(&dir);
        _set_test_dict_path(None);
        CACHE.lock().clear();
        *CACHE_INITIALISED.lock() = false;
    }

    #[test]
    fn empty_when_no_file() {
        with_temp_home(|| {
            let entries = load().unwrap();
            assert!(entries.is_empty());
        });
    }

    #[test]
    fn add_then_list_round_trip() {
        with_temp_home(|| {
            let entries = add("ai", "AI").unwrap();
            assert_eq!(entries, vec![DictEntry { from: "ai".into(), to: "AI".into() }]);
            let listed = list().unwrap();
            assert_eq!(listed, entries);
            // On-disk file exists and contains our entry.
            let raw = std::fs::read_to_string(dictionary_path()).unwrap();
            assert!(raw.contains("\"from\": \"ai\""));
            assert!(raw.contains("\"to\": \"AI\""));
        });
    }

    #[test]
    fn add_updates_existing() {
        with_temp_home(|| {
            add("ai", "AI").unwrap();
            let entries = add("ai", "A.I.").unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].to, "A.I.");
        });
    }

    #[test]
    fn remove_existing() {
        with_temp_home(|| {
            add("ai", "AI").unwrap();
            add("ts", "TypeScript").unwrap();
            let after = remove("ai").unwrap();
            assert_eq!(after.len(), 1);
            assert_eq!(after[0].from, "ts");
        });
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        with_temp_home(|| {
            add("ai", "AI").unwrap();
            let after = remove("ghost").unwrap();
            assert_eq!(after.len(), 1);
            assert_eq!(after[0].from, "ai");
        });
    }

    #[test]
    fn add_rejects_empty_from() {
        with_temp_home(|| {
            let err = add("", "X").unwrap_err();
            assert!(err.to_string().contains("`from`"));
        });
    }

    #[test]
    fn save_writes_valid_json() {
        with_temp_home(|| {
            save(&[
                DictEntry { from: "ai".into(), to: "AI".into() },
                DictEntry { from: "ts".into(), to: "TypeScript".into() },
            ])
            .unwrap();
            // Re-load from disk and confirm we got the same data back.
            CACHE.lock().clear();
            *CACHE_INITIALISED.lock() = false;
            let loaded = load().unwrap();
            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded[0].0, "ai");
            assert_eq!(loaded[1].0, "ts");
        });
    }

    #[test]
    fn atomic_write_leaves_no_tmp_on_success() {
        with_temp_home(|| {
            save(&[DictEntry { from: "x".into(), to: "y".into() }]).unwrap();
            let tmp = dictionary_path().with_extension("json.tmp");
            assert!(!tmp.exists(), "tmp file should be renamed away: {}", tmp.display());
            assert!(dictionary_path().exists());
        });
    }
}