//! Shared row type for transcription history.
//!
//! Used both as the in-memory return value from the DB layer and as the
//! serde-serialized shape delivered over IPC. `HistoryEntry` (in `ipc.rs`)
//! is just a re-export of this struct.

use rusqlite::Row;
use serde::Serialize;

/// One persisted transcription.
///
/// `source` is `"ptt"` for push-to-talk, or `"dictation"` for free-form
/// dictation mode (the dictation flow lands in a later task; we keep the
/// field now so the schema doesn't have to migrate).
#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionRow {
    pub id: i64,
    pub text: String,
    /// Unix epoch milliseconds.
    pub created_at: i64,
    pub duration_ms: Option<i64>,
    pub source: Option<String>,
    pub language: Option<String>,
}

impl TranscriptionRow {
    /// Map a rusqlite row into our struct. Column order matches the
    /// `SELECT id, text, created_at, duration_ms, source, language` query
    /// used in `db.rs`.
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            text: row.get(1)?,
            created_at: row.get(2)?,
            duration_ms: row.get(3)?,
            source: row.get(4)?,
            language: row.get(5)?,
        })
    }
}