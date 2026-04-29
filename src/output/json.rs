//! Newline-delimited JSON renderer.
//!
//! One event per line with the `event` tag set by [`super::Event`]'s serde
//! attributes (e.g. `{"event": "status", "kind": "...", ...}`). Streams as it
//! goes — `finalize()` is a no-op.

use std::io::Write;

use super::{DoctorCheck, Event, Output, OutputError};

/// Newline-delimited JSON output. One event per line:
/// `{"event": "status", "kind": "...", ...}` or
/// `{"event": "applied_action", "kind": "...", ...}`.
pub struct JsonOutput<'w> {
    out: &'w mut dyn Write,
}

impl<'w> JsonOutput<'w> {
    /// Wrap a writer (typically a locked stdout). Each event is flushed as
    /// a single line on `push`; the writer is not buffered internally.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Self { out }
    }
}

impl Output for JsonOutput<'_> {
    fn push(&mut self, event: Event) -> Result<(), OutputError> {
        // Bare section headers are a human-rendering construct — JSON
        // consumers reconstruct sections from `Check { section, ... }`
        // events directly.
        if matches!(event, Event::DoctorCheck(DoctorCheck::SectionHeader { .. })) {
            return Ok(());
        }
        serde_json::to_writer(&mut *self.out, &event)?;
        writeln!(self.out)?;
        Ok(())
    }
}
