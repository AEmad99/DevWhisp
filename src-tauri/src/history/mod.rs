//! Transcription history: SQLite-backed CRUD.
//!
//! Public API (all return `Result<T, anyhow::Error>`):
//!   - `insert(text, duration_ms, source)` -> row id
//!   - `list(limit, offset)` -> newest-first `Vec<TranscriptionRow>`
//!   - `search(query, limit)` -> case-insensitive `LIKE` match
//!   - `delete(id)` -> bool (true if a row was removed)
//!   - `clear()` -> count of removed rows
//!
//! The IPC layer (`crate::ipc`) is responsible for converting these into
//! `Vec<HistoryEntry>` and string-keyed error types.

pub mod db;
pub mod types;

pub use db::{clear, delete, insert, list, search};
pub use types::TranscriptionRow;