//! Pill widget: IPC handlers for the floating pill window.
//!
//! Splitting these out of `crate::ipc` keeps the IPC module focused on the
//! history / formatter / dictionary handlers owned by other tracks. The
//! pill widget is a self-contained subsystem with its own state machine
//! and persistence, so its IPC surface lives alongside its window code.

pub mod ipc;