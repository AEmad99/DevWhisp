//! Transcription history: SQLite-backed CRUD.
//!
//! Public API (all return `Result<T, anyhow::Error>`):
//!   - `insert(text, duration_ms, source)` -> row id
//!   - `list(limit, offset)` -> newest-first `Vec<TranscriptionRow>`
//!   - `search(query, limit)` -> case-insensitive `LIKE` match
//!   - `delete(id)` -> bool (true if a row was removed)
//!   - `delete_older_than(cutoff_ms)` -> count of rows removed (auto-prune)
//!   - `prune_if_needed()` -> count removed, honoring the retention setting
//!   - `clear()` -> count of removed rows
//!
//! The IPC layer (`crate::ipc`) is responsible for converting these into
//! `Vec<HistoryEntry>` and string-keyed error types.

pub mod db;
pub mod types;

pub use db::{clear, delete, delete_older_than, insert, list, search};
pub use types::TranscriptionRow;

/// Number of milliseconds in one day.
const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// Delete history rows older than the configured retention window.
///
/// Reads the retention from settings:
///   - **Absent** → use the default (2 days).
///   - **0 / "Never"** → pruning disabled, returns `Ok(0)` without touching
///     the DB.
///   - **`n` days** → delete rows with `created_at < now - n days`.
///
/// Returns the number of rows removed. Best-effort: callers log warnings on
/// error rather than failing the surrounding operation (startup, post-insert).
pub fn prune_if_needed() -> anyhow::Result<i64> {
    // Absent key → fresh install → apply the default. Explicit `Some(0)` →
    // user-disabled ("Never") → no-op.
    let days = match crate::config::load_history_retention_days() {
        None => crate::config::DEFAULT_HISTORY_RETENTION_DAYS,
        Some(0) => return Ok(0),
        Some(n) => n,
    };
    let now_ms = now_epoch_ms();
    let cutoff = now_ms - (days as i64) * MS_PER_DAY;
    let removed = db::delete_older_than(cutoff)?;
    if removed > 0 {
        log::info!(
            "history auto-prune: removed {removed} rows older than {days} day(s)"
        );
    }
    Ok(removed)
}

/// Unix epoch in milliseconds.
fn now_epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}