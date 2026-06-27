//! SQLite-backed transcription history.
//!
//! Storage: a single SQLite database at `~/.devwhisp/history.db`, opened
//! lazily on first call and shared through a `Mutex<Connection>`. We use
//! `parking_lot::Mutex` (already a project dep) for speed; the connection
//! is only ever touched from synchronous IPC handlers and the audio
//! injection path on the hotkey release thread, so contention is not a
//! concern.
//!
//! Schema lives in `init_schema`. We use `CREATE TABLE IF NOT EXISTS` so
//! the file is created on first run with no separate migration step.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Arc;

use super::types::TranscriptionRow;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS transcriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    duration_ms INTEGER,
    source TEXT,
    language TEXT
);
CREATE INDEX IF NOT EXISTS idx_created_at
    ON transcriptions(created_at DESC);
"#;

/// Shared DB handle. Initialised lazily by `handle()`.
static DB: once_cell::sync::Lazy<Arc<Mutex<Option<Connection>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(None)));

/// Test-only override for the database path. When `Some(p)`, `db_path()`
/// returns `p` directly instead of consulting `dirs::home_dir()`. This
/// is the only way to isolate tests on Windows: `dirs::home_dir()`
/// reads the registry via SHGetFolderPathW and ignores `HOME` /
/// `USERPROFILE` env vars, so env-var tricks don't work in CI.
static TEST_DB_PATH_OVERRIDE: once_cell::sync::Lazy<Mutex<Option<PathBuf>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// Set or clear the test-only DB path override. Production code never
/// calls this — the override defaults to `None` at process start and
/// stays that way unless a unit test flips it.
#[cfg(test)]
pub fn _set_test_db_path(p: Option<PathBuf>) {
    *TEST_DB_PATH_OVERRIDE.lock() = p;
}

/// Absolute path to the SQLite file. Created on first DB call.
pub fn db_path() -> PathBuf {
    if let Some(p) = TEST_DB_PATH_OVERRIDE.lock().clone() {
        return p;
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".devwhisp")
        .join("history.db")
}

/// Run a closure with mutable access to the DB connection.
///
/// The lazy-static holds an `Option<Connection>` behind a
/// `parking_lot::Mutex`. On first call we open the file, create the
/// schema, then run the caller's closure. Subsequent calls reuse the
/// same handle.
fn with_conn<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&mut Connection) -> Result<T>,
{
    let mut guard = DB.lock();
    if guard.is_none() {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create history db dir {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("open history db at {}", path.display()))?;
        conn.execute_batch(SCHEMA_SQL)
            .context("initialise history db schema")?;
        log::info!("history db opened at {}", path.display());
        *guard = Some(conn);
    }
    let conn = guard.as_mut().expect("connection initialised above");
    f(conn)
}

/// Insert a new transcription. Returns the new row id.
///
/// `source` is stored as-is; the IPC layer passes `"ptt"` for push-to-talk
/// recordings. `language` defaults to `"en"` when the caller doesn't know.
pub fn insert(text: &str, duration_ms: Option<i64>, source: Option<&str>) -> Result<i64> {
    let now_ms = now_ms();
    with_conn(|conn| {
        conn.execute(
            "INSERT INTO transcriptions (text, created_at, duration_ms, source, language)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![text, now_ms, duration_ms, source, "en"],
        )?;
        Ok(conn.last_insert_rowid())
    })
}

/// Return up to `limit` rows starting at `offset`, newest first.
pub fn list(limit: i64, offset: i64) -> Result<Vec<TranscriptionRow>> {
    let lim = limit.max(0);
    let off = offset.max(0);
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, text, created_at, duration_ms, source, language
             FROM transcriptions
             ORDER BY created_at DESC, id DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt
            .query_map(params![lim, off], TranscriptionRow::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Case-insensitive substring search over `text`. Returns up to `limit`
/// matches, newest first.
pub fn search(query: &str, limit: i64) -> Result<Vec<TranscriptionRow>> {
    let lim = limit.max(0);
    let pattern = format!("%{}%", query);
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, text, created_at, duration_ms, source, language
             FROM transcriptions
             WHERE LOWER(text) LIKE LOWER(?1)
             ORDER BY created_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![pattern, lim], TranscriptionRow::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Delete a single row by id. Returns `true` if a row was removed.
pub fn delete(id: i64) -> Result<bool> {
    with_conn(|conn| {
        let n = conn.execute("DELETE FROM transcriptions WHERE id = ?1", params![id])?;
        Ok(n > 0)
    })
}

/// Wipe every row. Returns the number of rows that were deleted.
pub fn clear() -> Result<i64> {
    with_conn(|conn| {
        let n = conn.execute("DELETE FROM transcriptions", [])?;
        Ok(n as i64)
    })
}

/// Unix epoch in milliseconds — central helper so tests can mock it later
/// if we ever need to.
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counter so each test gets its own temp DB file.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Module-global lock that serialises every DB test. We need it
    /// because the DB path is process-global (the lazy `DB` static
    /// and `TEST_DB_PATH_OVERRIDE` are both shared across threads),
    /// so running tests in parallel would have them stomp each
    /// other's DB files.
    static TEST_LOCK: once_cell::sync::Lazy<parking_lot::Mutex<()>> =
        once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(()));

    /// Run `f` against a fresh temporary DB and clean up afterwards.
    ///
    /// Acquires the module-global `TEST_LOCK` so tests can't race on
    /// the shared `DB` static and `TEST_DB_PATH_OVERRIDE`.
    fn with_temp_db<F: FnOnce()>(f: F) {
        let _guard = TEST_LOCK.lock();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("devwhisp-history-test-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        // Install override + reset the connection so the new path takes effect.
        _set_test_db_path(Some(path.clone()));
        *DB.lock() = None;

        f();

        // Best-effort cleanup.
        let _ = std::fs::remove_dir_all(&dir);
        _set_test_db_path(None);
        *DB.lock() = None;
    }

    #[test]
    fn insert_and_list_round_trip() {
        with_temp_db(|| {
            let id1 = insert("hello world", Some(1200), Some("ptt")).unwrap();
            let id2 = insert("second entry", None, Some("dictation")).unwrap();
            assert!(id1 > 0 && id2 > id1);

            let rows = list(10, 0).unwrap();
            assert_eq!(rows.len(), 2);
            // Newest first.
            assert_eq!(rows[0].text, "second entry");
            assert_eq!(rows[0].source.as_deref(), Some("dictation"));
            assert_eq!(rows[1].text, "hello world");
            assert_eq!(rows[1].duration_ms, Some(1200));
        });
    }

    #[test]
    fn search_is_case_insensitive() {
        with_temp_db(|| {
            insert("AI is great", Some(500), Some("ptt")).unwrap();
            insert("the cat sat", Some(300), Some("ptt")).unwrap();
            insert("another Ai mention", None, Some("ptt")).unwrap();

            let hits = search("ai", 10).unwrap();
            assert_eq!(hits.len(), 2, "expected 2 AI hits, got {}", hits.len());
            for h in &hits {
                assert!(
                    h.text.to_lowercase().contains("ai"),
                    "search returned non-matching row: {:?}",
                    h.text
                );
            }
        });
    }

    #[test]
    fn delete_returns_true_only_when_row_exists() {
        with_temp_db(|| {
            let id = insert("to delete", Some(100), Some("ptt")).unwrap();
            assert!(delete(id).unwrap());
            assert!(!delete(id).unwrap());
            assert!(!delete(999_999).unwrap());
            assert_eq!(list(10, 0).unwrap().len(), 0);
        });
    }

    #[test]
    fn clear_returns_count() {
        with_temp_db(|| {
            insert("a", Some(10), Some("ptt")).unwrap();
            insert("b", Some(20), Some("ptt")).unwrap();
            insert("c", None, Some("ptt")).unwrap();
            assert_eq!(clear().unwrap(), 3);
            assert_eq!(list(10, 0).unwrap().len(), 0);
            // Idempotent: clearing an empty table is fine.
            assert_eq!(clear().unwrap(), 0);
        });
    }

    #[test]
    fn list_respects_limit_and_offset() {
        with_temp_db(|| {
            for i in 0..5 {
                insert(&format!("row {i}"), Some(i * 100), Some("ptt")).unwrap();
                // Sleep just enough so created_at ticks a millisecond
                // apart and ORDER BY is deterministic.
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let page1 = list(2, 0).unwrap();
            let page2 = list(2, 2).unwrap();
            let page3 = list(2, 4).unwrap();
            assert_eq!(page1.len(), 2);
            assert_eq!(page2.len(), 2);
            assert_eq!(page3.len(), 1);
            // Newest-first: page1 should hold rows 4 and 3.
            assert!(page1[0].text.contains('4'));
            assert!(page1[1].text.contains('3'));
        });
    }

    #[test]
    fn db_path_is_under_devwhisp_dir() {
        // Sanity check that we land under ~/.devwhisp, not the temp dir.
        let p: PathBuf = db_path();
        assert!(
            p.components().any(|c| c.as_os_str() == ".devwhisp"),
            "expected .devwhisp in path, got {}",
            p.display()
        );
    }
}