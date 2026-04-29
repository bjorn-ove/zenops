//! Structured-event channel for command output.
//!
//! Every command emits through the [`Output`] trait. The renderer chosen by
//! `--output` decides formatting: [`TerminalRenderer`] for human-readable,
//! column-aligned text with optional ANSI color, or [`JsonOutput`] for
//! newline-delimited JSON. Both write to stdout.
//!
//! This is distinct from `log::*!`. `log::*!` is the runtime-event channel
//! (debug breadcrumbs, gated behind `RUST_LOG`); the [`Output`] trait is the
//! structured-output channel (the actual command result). They don't bridge.
//! The stdout/stderr split — `Output` to stdout, `log::*!` and fatal errors
//! to stderr — keeps `--output json` parseable even with `RUST_LOG=debug`.
//!
//! Each command uses a subset of the trait surface: `apply` and `status`
//! emit [`Status`] and [`AppliedAction`]; `pkg` emits [`PkgEntry`]; `doctor`
//! emits [`DoctorCheck`]; `init` (without `--apply`) emits [`InitSummary`].
//! The trait is intentionally polymorphic — multiple impls (and shapes like
//! `JsonSchema` derives on the event types) are part of the contract; don't
//! collapse it into a concrete formatter.

mod event;
mod json;
mod terminal;

#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::Serialize;

pub use event::{
    AppliedAction, BootstrapSummary, DoctorCheck, DoctorSection, DoctorSeverity, FileStatus,
    InitSummary, PkgEntry, PkgEntryState, PkgInstallHints, PkgStatus, ResolvedConfigFilePath,
    Status, SymlinkStatus,
};
pub use json::JsonOutput;
pub use terminal::TerminalRenderer;

/// Errors surfaced by [`Output`] implementations. `Io` comes from
/// `writeln!` / `write!` on the backing `Write`; `Json` from
/// `serde_json::to_writer` in `JsonOutput`. Both variants use
/// `#[error(transparent)]` so the user sees the underlying message
/// verbatim.
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    /// I/O error from the backing `Write`.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Serde JSON error from `JsonOutput`.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// One structured output event. The renderer match-dispatches; the JSON
/// renderer serialises this directly with the `event` tag (e.g. `{"event":
/// "status", ...}`).
#[derive(Serialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// One row of the per-file desired-state report (used by `apply`'s
    /// pre-change pass and by `status`).
    Status(Status),
    /// One row per change `apply` actually wrote to disk.
    AppliedAction(AppliedAction),
    /// One row of `zenops pkg`.
    PkgEntry(PkgEntry),
    /// One row (or section marker) of `zenops doctor`.
    DoctorCheck(DoctorCheck),
    /// Outcome of `zenops init <url>` without `--apply`.
    InitSummary(InitSummary),
    /// Outcome of `zenops init` bootstrap (no URL).
    BootstrapSummary(BootstrapSummary),
}

/// The structured-output channel a command emits through. See the
/// [module-level docs](self) for how this relates to `log::*!` and how the
/// renderer is chosen.
///
/// One method, [`push`](Output::push), takes any [`Event`] variant; the
/// renderer match-dispatches internally. [`finalize`](Output::finalize) is
/// called once at the end of a command — `JsonOutput` streams as it goes
/// and treats it as a no-op; `TerminalRenderer` accumulates rows so it can
/// column-align them and flushes here.
pub trait Output {
    /// Emit one structured event. Variants of [`Event`] cover every
    /// command's output surface: the per-file `Status` / `AppliedAction`
    /// stream from `apply` and `status`, `PkgEntry` from `pkg`,
    /// `DoctorCheck` from `doctor`, and the one-shot `InitSummary` /
    /// `BootstrapSummary` from `init`.
    fn push(&mut self, event: Event) -> Result<(), OutputError>;
    /// Emit any buffered output. `JsonOutput` streams as it goes and has a
    /// no-op `finalize`; `TerminalRenderer` accumulates lines so it can
    /// column-align them and flushes here. Safe to call zero or one time at
    /// the end of a command.
    fn finalize(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}
