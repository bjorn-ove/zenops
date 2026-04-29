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

use similar::{ChangeTag, TextDiff};
use smol_str::SmolStr;
use std::{
    fmt,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use schemars::JsonSchema;
use serde::Serialize;

use crate::{
    ansi::Styler,
    config_files::{ConfigFileDirs, ConfigFilePath},
    git::GitFileStatus,
};

// ---- Per-command event types ----------------------------------------------
//
// `Status` and `AppliedAction` cover the apply/status event stream — each
// variant maps to a marker + colored row in the column-aligned block.
// `pkg`, `doctor`, and `init` produce different output shapes (pkg-list
// rows, labeled environment checks, single-shot init summary) that don't
// fit that table, so they get their own event types pushed through
// dedicated trait methods.

/// One row of `zenops pkg`. `AggregateInstall` and `NoPackageManagerDetected`
/// cover the surrounding context (footer + pre-block warning) so JSON
/// consumers see every line as a structured event.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PkgEntry {
    /// One per visible package. `name` is the display label (`pkg.name`
    /// override or the map key); `key` is always the original map key so
    /// scripts can correlate even when `name` differs.
    Pkg {
        /// Display label (`pkg.name` override, or the map key).
        name: SmolStr,
        /// Original `[pkg.<key>]` map key, kept distinct from `name` so
        /// scripts can correlate even when the user overrides the label.
        key: SmolStr,
        /// Free-text description from `pkg.description`, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// Computed install state on this host.
        state: PkgEntryState,
        /// The matching detect strategy when `--verbose` is on AND a strategy
        /// matched on the current host.
        #[serde(skip_serializing_if = "Option::is_none")]
        matched_detect: Option<String>,
        /// Install commands grouped by package manager.
        install_hints: PkgInstallHints,
    },
    /// "To install all missing via brew: brew install foo bar" footer.
    /// Emitted at most once per `pkg` invocation, after all `Pkg` entries.
    AggregateInstall {
        /// Package manager that produced the aggregate command (e.g. `"brew"`).
        pkg_manager: String,
        /// The full ready-to-run shell command.
        command: String,
        /// Packages aggregated into the command, in the order they appear.
        packages: Vec<String>,
    },
    /// One-shot warning emitted when no supported package manager is on PATH.
    /// `supported` lets future managers land without a serde-tag rename.
    NoPackageManagerDetected {
        /// Package managers zenops knows how to detect.
        supported: Vec<String>,
    },
}

/// State a configured pkg is in on the current host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PkgEntryState {
    /// `enable = "disabled"` in the config; not expected to be present.
    Disabled,
    /// At least one detect strategy matched.
    Installed,
    /// Expected (`enable = "on"`) but no detect strategy matched.
    Missing,
}

/// Per-manager install hints. Mirrors `InstallHint` — extend in lockstep
/// when adding a new package manager so `--all-hints` stays complete.
#[derive(Debug, Clone, PartialEq, Default, Serialize, JsonSchema)]
pub struct PkgInstallHints {
    /// Homebrew packages this pkg installs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub brew: Vec<String>,
}

/// One row of `zenops doctor`. Two variants:
/// - `Check`: a labeled environment check (key/value/severity, optional
///   actionable hint, optional multi-line detail body).
/// - `SectionHeader`: marker for sections that have no rows of their own
///   (currently only `Packages`, which is followed by `Status::Pkg`
///   events from `push_pkg_health`). The terminal renderer prints a bold
///   section title; `JsonOutput` skips these — JSON consumers don't need
///   them since each `DoctorCheck::Check` carries its own `section` and
///   pkg health speaks for itself via `Status::Pkg`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DoctorCheck {
    /// A labeled environment check.
    Check {
        /// Section this row belongs to.
        section: DoctorSection,
        /// "os:", "git:", "remote:", … the existing left-column label.
        label: SmolStr,
        /// Severity that drives the marker / colour in the human renderer.
        severity: DoctorSeverity,
        /// Right-hand value: "found on PATH", "missing", a path, a remote URL.
        value: String,
        /// Dim-rendered actionable phrasing on the same line as the value.
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        /// Multi-line detail body, currently only used by the parse-error
        /// branch of `load_config_or_report`. One element per line, no
        /// trailing newline; renderer indents each under the row.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        detail: Vec<String>,
    },
    /// Marker for sections that have no rows of their own (currently just
    /// `Packages`, whose rows are pushed as `Status::Pkg` events from
    /// `push_pkg_health`). Skipped by `JsonOutput`.
    SectionHeader {
        /// Which section is starting.
        section: DoctorSection,
    },
}

/// Logical grouping for `doctor` rows. Section transitions trigger a
/// section header in the human renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSection {
    /// OS / kernel basics.
    System,
    /// Zenops config repo state.
    Repo,
    /// `config.toml` location and parse health.
    Config,
    /// Detected package manager(s).
    PkgManager,
    /// User identity (name, email, GitHub username).
    User,
    /// Configured shell and its rc files.
    Shell,
    /// Per-package install state — populated via `Status::Pkg`.
    Packages,
}

/// Severity badge attached to a [`DoctorCheck::Check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    /// Healthy.
    Ok,
    /// Informational; no action needed.
    Info,
    /// Degraded but functional; user should look at it.
    Warn,
    /// Broken; the user almost certainly needs to fix this.
    Bad,
}

/// Result of a successful `zenops init` clone (with a URL, no `--apply`).
/// When `--apply` is set, `init` clones then recurses into `Apply` and the
/// apply event stream is the contract — no `InitSummary` is emitted in
/// that case. Bootstrap (no URL) emits [`BootstrapSummary`] instead.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct InitSummary {
    /// Where the repo was cloned to (always `~/.config/zenops` today).
    pub clone_path: PathBuf,
    /// Remote URL recorded in the cloned repo, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// "bash" or "zsh" when the cloned config configures a shell, `None`
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Number of `[pkg.<name>]` entries in the cloned `config.toml`.
    pub pkg_count: usize,
}

/// Result of a successful `zenops init` bootstrap (no URL). Reports the
/// fresh repo path plus whatever identity the user chose at the prompts;
/// each identity field is `None` when the user accepted the empty default.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct BootstrapSummary {
    /// Where the new repo was created (always `~/.config/zenops` today).
    pub repo_path: PathBuf,
    /// "bash" or "zsh" when the user picked one at the shell prompt;
    /// `None` if they declined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Name written to `[user]` in the fresh `config.toml`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Email written to `[user]` in the fresh `config.toml`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Where a managed symlink stands relative to its desired target.
#[derive(Debug, PartialEq, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SymlinkStatus {
    /// Symlink exists and points to the right place.
    Ok,
    /// Symlink exists but points somewhere other than the desired target.
    WrongLink(PathBuf),
    /// The symlink does not exist and must be created
    New,
    /// The path is a file and not a symlink
    IsFile,
    /// The path is a directory and not a symlink
    IsDir,
    /// The path exists but is neither a regular file, directory, nor symlink
    /// (e.g. FIFO, socket, device node). zenops refuses to clobber it; the
    /// user must remove it manually.
    IsOther,
    /// The symlink exists and points to the correct location, but the source does not exist.
    RealPathIsMissing,
    /// The directory that should contain the symlink is missing.
    /// `dir` is the parent path that needs to be created before the symlink
    /// can land.
    DstDirIsMissing {
        /// Parent directory of the symlink that needs to be created.
        dir: ResolvedConfigFilePath,
    },
}

/// Where a generated file stands relative to its desired contents.
#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// On-disk content matches the generated body byte-for-byte.
    Ok,
    /// File exists but content differs from the generated body.
    Modified,
    /// File does not exist yet.
    New,
}

/// Per-package install state used by `Status::Pkg`.
#[derive(Debug, PartialEq, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum PkgStatus {
    /// Pkg is installed.
    Ok,
    /// A pkg the user expects to be present (`enable = "on"`) whose detect
    /// strategies don't match on the current host. `install_command` is the
    /// ready-to-run shell line (`"brew install python"`) when a package
    /// manager with a non-empty hint is detected, `None` otherwise.
    Missing {
        /// Ready-to-run install command, or `None` if no usable install
        /// hint exists for the detected package manager.
        install_command: Option<String>,
    },
}

/// A [`ConfigFilePath`] paired with its already-resolved absolute path.
/// Cached on construction so the renderer can format paths without
/// re-resolving.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ResolvedConfigFilePath {
    /// The symbolic path (root + relative tail).
    pub path: ConfigFilePath,
    /// The resolved absolute filesystem path.
    pub full: Arc<Path>,
}

impl ResolvedConfigFilePath {
    /// Resolve `path` against `dirs` and capture both forms.
    pub fn resolve(path: ConfigFilePath, dirs: &ConfigFileDirs) -> Self {
        let full = Arc::from(path.resolved(dirs));
        Self { path, full }
    }

    /// The parent path in the same root, with `full` adjusted in lock-step.
    /// Returns `None` if either side is already at the top.
    pub fn parent(&self) -> Option<Self> {
        Some(Self {
            path: self.path.parent()?,
            full: self.full.parent().map(Arc::from)?,
        })
    }
}

impl fmt::Display for ResolvedConfigFilePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.path.human_path(), f)
    }
}

/// One row of the desired-state report. Carries enough context for
/// either the human renderer (status marker + path + colored description)
/// or a JSON consumer to reconstruct a useful summary.
#[derive(Debug, PartialEq, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    /// A managed file whose content is generated by zenops.
    Generated {
        /// Body zenops wants on disk.
        want_content: Arc<str>,
        /// Body currently on disk, if any (used to render diffs).
        cur_content: Option<String>,
        /// Where the file lives.
        path: ResolvedConfigFilePath,
        /// Comparison verdict.
        status: FileStatus,
    },
    /// A managed symlink.
    Symlink {
        /// Target the symlink should point at (a file in the zenops repo).
        real: ResolvedConfigFilePath,
        /// Where the symlink lives.
        symlink: ResolvedConfigFilePath,
        /// Comparison verdict.
        status: SymlinkStatus,
    },
    /// One file's git state inside the zenops config repo. Emitted when
    /// the repo is dirty so the user sees what's uncommitted.
    Git {
        /// The zenops config repo (always the same value; carried so JSON
        /// consumers can see it on every row).
        repo: ResolvedConfigFilePath,
        /// Per-file status from `git status`.
        status: GitFileStatus,
    },
    /// Emitted when the zenops config repo has no uncommitted changes. The
    /// dirty case is covered per-file by `Git`.
    GitRepoClean {
        /// The zenops config repo path.
        repo: ResolvedConfigFilePath,
    },
    /// Install state for a configured pkg.
    Pkg {
        /// Pkg key from `[pkg.<key>]`.
        pkg: SmolStr,
        /// Computed install state on this host.
        status: PkgStatus,
    },
}

/// One change actually written to disk during `apply`.
#[derive(Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppliedAction {
    /// Existing generated file overwritten with new content.
    UpdatedFile(ResolvedConfigFilePath),
    /// Generated file created (path didn't exist before).
    CreatedFile(ResolvedConfigFilePath),
    /// Symlink created at `symlink` pointing at `real`.
    CreatedSymlink {
        /// Target the symlink points at.
        real: ResolvedConfigFilePath,
        /// Where the symlink was created.
        symlink: ResolvedConfigFilePath,
    },
    /// Symlink replaced at `symlink`: it previously pointed somewhere else
    /// and now points at `real`.
    ReplacedSymlink {
        /// New target the symlink points at.
        real: ResolvedConfigFilePath,
        /// Where the (replaced) symlink lives.
        symlink: ResolvedConfigFilePath,
    },
    /// Parent directory created so a managed file or symlink could land.
    CreatedDir(ResolvedConfigFilePath),
}

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

/// Newline-delimited JSON output. One event per line:
/// `{"event": "status", "kind": "...", ...}` or
/// `{"event": "applied_action", "kind": "...", ...}`.
pub struct JsonOutput<'w> {
    out: &'w mut dyn Write,
}

impl<'w> JsonOutput<'w> {
    /// Wrap a writer (typically a locked stdout). Each event is flushed as
    /// a single line on its `push_*` call; the writer is not buffered
    /// internally.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Self { out }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Default,
    Dim,
    ExtraDim,
    Bold,
    Red,
    Green,
    Yellow,
    Cyan,
    Magenta,
    BoldYellow,
}

impl Style {
    fn open(self, s: &Styler) -> &'static str {
        match self {
            Style::Default => "",
            Style::Dim => s.dim(),
            Style::ExtraDim => s.extra_dim(),
            Style::Bold => s.bold(),
            Style::Red => s.red(),
            Style::Green => s.green(),
            Style::Yellow => s.yellow(),
            Style::Cyan => s.cyan(),
            Style::Magenta => s.magenta(),
            Style::BoldYellow => s.bold_yellow(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    text: String,
    style: Style,
}

impl Segment {
    fn new(style: Style, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

#[derive(Debug)]
struct Line {
    marker: char,
    marker_style: Style,
    path: Vec<Segment>,
    description: Vec<Segment>,
}

/// Human-readable text renderer. Status/applied-action events are buffered
/// into a single column-aligned block flushed on transition to a different
/// event category or on `finalize`. Pkg-entry events are similarly buffered
/// (so the name column can pad to the widest visible row). Doctor checks
/// and init summaries render eagerly. Honors `color` for ANSI escapes,
/// `show_diffs` for whether `Status::Generated` variants emit a unified diff
/// after the summary block, and `show_clean` for whether clean-state events
/// (e.g. `FileStatus::Ok`, `PkgStatus::Ok`, `GitRepoClean`) get a visible
/// line or are dropped.
pub struct TerminalRenderer<'w> {
    out: &'w mut dyn Write,
    styler: Styler,
    show_diffs: bool,
    show_clean: bool,
    pending: Pending,
    /// Status/applied-action block accumulator.
    lines: Vec<Line>,
    diffs: Vec<PendingDiff>,
    /// Pkg block accumulator. The aggregate footer (when present) renders
    /// after all `Pkg` rows in the same block.
    pkg_rows: Vec<PkgRow>,
    pkg_aggregate: Option<PkgAggregate>,
    /// Tracks the currently-rendered doctor section so a transition triggers
    /// a section header + blank-line separator.
    last_doctor_section: Option<DoctorSection>,
    finalized: bool,
}

/// Which kind of block, if any, is currently buffered. Status, action, and
/// pkg events buffer until flushed; doctor and init render inline (no
/// buffering needed for those).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    StatusBlock,
    PkgBlock,
}

#[derive(Debug)]
struct PendingDiff {
    path: ResolvedConfigFilePath,
    cur_content: Option<String>,
    want_content: Arc<str>,
}

#[derive(Debug)]
struct PkgRow {
    /// Name column display label (post-`name`-override).
    name: SmolStr,
    description: Option<String>,
    state: PkgEntryState,
    matched_detect: Option<String>,
    install_hints: PkgInstallHints,
}

#[derive(Debug)]
struct PkgAggregate {
    pkg_manager: String,
    command: String,
}

impl<'w> TerminalRenderer<'w> {
    /// Wrap a writer (typically a locked stdout). `color` toggles ANSI
    /// escapes; `show_diffs` controls whether `Status::Generated` entries
    /// trail their unified diff after the summary block; `show_clean`
    /// toggles whether already-matching entries (`FileStatus::Ok`,
    /// `PkgStatus::Ok`, `GitRepoClean`) print a row or are dropped.
    pub fn new(out: &'w mut dyn Write, color: bool, show_diffs: bool, show_clean: bool) -> Self {
        Self {
            out,
            styler: Styler::new(color),
            show_diffs,
            show_clean,
            pending: Pending::None,
            lines: Vec::new(),
            diffs: Vec::new(),
            pkg_rows: Vec::new(),
            pkg_aggregate: None,
            last_doctor_section: None,
            finalized: false,
        }
    }

    fn write_line(&mut self, line: &Line, path_width: usize) -> Result<(), OutputError> {
        let s = &self.styler;
        let reset = s.reset();
        let marker_open = line.marker_style.open(s);
        let path_chars: usize = line.path.iter().map(|seg| seg.text.chars().count()).sum();
        let pad = path_width.saturating_sub(path_chars);
        write!(
            self.out,
            "{marker_open}{marker}{reset}  ",
            marker = line.marker,
        )?;
        for seg in &line.path {
            let open = seg.style.open(s);
            write!(self.out, "{open}{text}{reset}", text = seg.text)?;
        }
        write!(self.out, "{spaces}", spaces = " ".repeat(pad))?;
        if !line.description.is_empty() {
            write!(self.out, "  ")?;
            for seg in &line.description {
                let open = seg.style.open(s);
                write!(self.out, "{open}{text}{reset}", text = seg.text)?;
            }
        }
        writeln!(self.out)?;
        Ok(())
    }

    fn write_diff(&mut self, diff: &PendingDiff) -> Result<(), OutputError> {
        let old = diff.cur_content.as_deref().unwrap_or("");
        if diff.cur_content.is_some() {
            writeln!(self.out, "--- {} (current)", diff.path)?;
        } else {
            writeln!(self.out, "--- /dev/null")?;
        }
        writeln!(self.out, "+++ {} (generated)", diff.path)?;
        let text_diff = TextDiff::from_lines(old, &*diff.want_content);
        let s = &self.styler;
        let reset = s.reset();
        for change in text_diff.iter_all_changes() {
            let (prefix, open) = match change.tag() {
                ChangeTag::Delete => ("-", s.red()),
                ChangeTag::Insert => ("+", s.green()),
                ChangeTag::Equal => (" ", s.dim()),
            };
            write!(self.out, "{open}{prefix}{change}{reset}")?;
        }
        Ok(())
    }
}

fn ok_line(path: Vec<Segment>, description: &'static str) -> Line {
    Line {
        marker: '✓',
        marker_style: Style::Green,
        path,
        description: vec![Segment::new(Style::Green, description)],
    }
}

/// Styled segments for a single resolved path. `Zenops`-rooted paths with a
/// non-empty relative tail split into an extra-dim `~/.config/zenops` prefix
/// (fades below the regular dim used for e.g. the left side of a symlink
/// row) and a default-weight remainder so the identifying tail stands out.
/// Other paths (including the zenops root itself, like `GitRepoClean`)
/// render as one dim segment.
fn path_segments(path: &ResolvedConfigFilePath) -> Vec<Segment> {
    if let ConfigFilePath::Zenops(rel) = &path.path
        && !rel.as_str().is_empty()
    {
        return vec![
            Segment::new(Style::ExtraDim, "~/.config/zenops"),
            Segment::new(Style::Default, format!("/{rel}")),
        ];
    }
    vec![Segment::new(Style::Dim, path.to_string())]
}

/// Path segments for a `{symlink} → {real}` row: left side all dim, bold
/// ` → `, right side via `path_segments` (so a zenops-rooted `real` splits
/// into dim prefix + default-weight tail).
fn symlink_path_segments(
    symlink: &ResolvedConfigFilePath,
    real: &ResolvedConfigFilePath,
) -> Vec<Segment> {
    let mut segs = vec![
        Segment::new(Style::Dim, symlink.to_string()),
        Segment::new(Style::Bold, " → "),
    ];
    segs.extend(path_segments(real));
    segs
}

/// Path segments for a git row: `{repo}/{sub}`. `repo` is the zenops root
/// (extra-dim — same "shared prefix" treatment as right-side symlink paths),
/// `sub` is the file path inside the repo (default weight).
fn git_path_segments(repo: &ResolvedConfigFilePath, sub: String) -> Vec<Segment> {
    vec![
        Segment::new(Style::ExtraDim, repo.to_string()),
        Segment::new(Style::Default, format!("/{sub}")),
    ]
}

fn raw_dim_path(s: impl Into<String>) -> Vec<Segment> {
    vec![Segment::new(Style::Dim, s)]
}

fn status_to_line(status: &Status, show_clean: bool) -> Option<Line> {
    match status {
        Status::Generated {
            path,
            status: FileStatus::Ok,
            ..
        } => show_clean.then(|| ok_line(path_segments(path), "ok")),
        Status::Generated {
            path,
            status: FileStatus::Modified,
            ..
        } => Some(Line {
            marker: '~',
            marker_style: Style::Yellow,
            path: path_segments(path),
            description: vec![Segment::new(Style::Yellow, "modified")],
        }),
        Status::Generated {
            path,
            status: FileStatus::New,
            ..
        } => Some(Line {
            marker: '+',
            marker_style: Style::Yellow,
            path: path_segments(path),
            description: vec![Segment::new(Style::Yellow, "missing")],
        }),
        Status::Symlink {
            real,
            symlink,
            status: SymlinkStatus::Ok,
        } => show_clean.then(|| ok_line(symlink_path_segments(symlink, real), "ok")),
        Status::Symlink {
            real,
            symlink,
            status: SymlinkStatus::WrongLink(actual),
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: symlink_path_segments(symlink, real),
            description: vec![
                Segment::new(Style::Red, "wrong target"),
                Segment::new(Style::Dim, format!(" {}", actual.display())),
            ],
        }),
        Status::Symlink {
            real,
            symlink,
            status: SymlinkStatus::New,
        } => Some(Line {
            marker: '+',
            marker_style: Style::Yellow,
            path: symlink_path_segments(symlink, real),
            description: vec![Segment::new(Style::Yellow, "missing")],
        }),
        Status::Symlink {
            symlink,
            status: SymlinkStatus::IsFile,
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: path_segments(symlink),
            description: vec![
                Segment::new(Style::Red, "is a file"),
                Segment::new(Style::Dim, ", expected symlink"),
            ],
        }),
        Status::Symlink {
            symlink,
            status: SymlinkStatus::IsDir,
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: path_segments(symlink),
            description: vec![
                Segment::new(Style::Red, "is a dir"),
                Segment::new(Style::Dim, ", expected symlink"),
            ],
        }),
        Status::Symlink {
            symlink,
            status: SymlinkStatus::IsOther,
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: path_segments(symlink),
            description: vec![
                Segment::new(Style::Red, "is not a regular file"),
                Segment::new(Style::Dim, ", expected symlink"),
            ],
        }),
        Status::Symlink {
            real,
            status: SymlinkStatus::RealPathIsMissing,
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: path_segments(real),
            description: vec![Segment::new(Style::Red, "symlink source missing")],
        }),
        Status::Symlink {
            symlink,
            status: SymlinkStatus::DstDirIsMissing { .. },
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: path_segments(symlink),
            description: vec![Segment::new(Style::Red, "parent directory missing")],
        }),
        Status::Git { repo, status } => match status {
            GitFileStatus::Modified(p) => Some(Line {
                marker: 'M',
                marker_style: Style::Yellow,
                path: git_path_segments(repo, p.to_string()),
                description: vec![Segment::new(Style::Yellow, "modified")],
            }),
            GitFileStatus::Added(p) => Some(Line {
                marker: 'A',
                marker_style: Style::Yellow,
                path: git_path_segments(repo, p.to_string()),
                description: vec![Segment::new(Style::Yellow, "added")],
            }),
            GitFileStatus::Deleted(p) => Some(Line {
                marker: 'D',
                marker_style: Style::Red,
                path: git_path_segments(repo, p.to_string()),
                description: vec![Segment::new(Style::Red, "deleted")],
            }),
            GitFileStatus::Untracked(p) => Some(Line {
                marker: '?',
                marker_style: Style::Cyan,
                path: git_path_segments(repo, p.to_string()),
                description: vec![Segment::new(Style::Cyan, "untracked")],
            }),
            GitFileStatus::Other { code, path } => Some(Line {
                marker: '!',
                marker_style: Style::Magenta,
                path: git_path_segments(repo, path.to_string()),
                description: vec![
                    Segment::new(Style::Dim, "status "),
                    Segment::new(Style::Magenta, code.to_string()),
                ],
            }),
        },
        Status::Pkg {
            pkg,
            status: PkgStatus::Missing {
                install_command: Some(cmd),
            },
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: raw_dim_path(pkg.to_string()),
            description: vec![
                Segment::new(Style::Red, "missing"),
                Segment::new(Style::Dim, " — install: "),
                Segment::new(Style::BoldYellow, cmd.clone()),
            ],
        }),
        Status::Pkg {
            pkg,
            status: PkgStatus::Missing {
                install_command: None,
            },
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: raw_dim_path(pkg.to_string()),
            description: vec![Segment::new(Style::Red, "missing")],
        }),
        Status::Pkg {
            pkg,
            status: PkgStatus::Ok,
        } => show_clean.then(|| ok_line(raw_dim_path(pkg.to_string()), "ok")),
        Status::GitRepoClean { repo } => show_clean.then(|| ok_line(path_segments(repo), "clean")),
    }
}

fn action_to_line(action: &AppliedAction) -> Line {
    match action {
        AppliedAction::UpdatedFile(path) => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: path_segments(path),
            description: vec![Segment::new(Style::Green, "updated")],
        },
        AppliedAction::CreatedFile(path) => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: path_segments(path),
            description: vec![Segment::new(Style::Green, "created")],
        },
        AppliedAction::CreatedSymlink { real, symlink } => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: symlink_path_segments(symlink, real),
            description: vec![Segment::new(Style::Green, "linked")],
        },
        AppliedAction::ReplacedSymlink { real, symlink } => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: symlink_path_segments(symlink, real),
            description: vec![Segment::new(Style::Green, "relinked")],
        },
        AppliedAction::CreatedDir(path) => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: path_segments(path),
            description: vec![Segment::new(Style::Green, "mkdir")],
        },
    }
}

impl TerminalRenderer<'_> {
    /// Open or stay in `kind`. If a different block is currently buffered,
    /// flush it first. Inline event categories (doctor, init) call this
    /// with `Pending::None` so a buffered block flushes before they render.
    fn enter(&mut self, kind: Pending) -> Result<(), OutputError> {
        if self.pending != kind {
            self.flush_pending()?;
            self.pending = kind;
        }
        Ok(())
    }

    fn flush_pending(&mut self) -> Result<(), OutputError> {
        match self.pending {
            Pending::None => {}
            Pending::StatusBlock => self.flush_status_block()?,
            Pending::PkgBlock => self.flush_pkg_block()?,
        }
        self.pending = Pending::None;
        Ok(())
    }

    fn flush_status_block(&mut self) -> Result<(), OutputError> {
        if self.lines.is_empty() && self.diffs.is_empty() {
            return Ok(());
        }
        let path_width = self
            .lines
            .iter()
            .map(|l| l.path.iter().map(|seg| seg.text.chars().count()).sum())
            .max()
            .unwrap_or(0);
        let lines = std::mem::take(&mut self.lines);
        for line in &lines {
            self.write_line(line, path_width)?;
        }
        let diffs = std::mem::take(&mut self.diffs);
        for diff in &diffs {
            writeln!(self.out)?;
            self.write_diff(diff)?;
        }
        Ok(())
    }

    fn flush_pkg_block(&mut self) -> Result<(), OutputError> {
        let rows = std::mem::take(&mut self.pkg_rows);
        let aggregate = self.pkg_aggregate.take();
        if rows.is_empty() && aggregate.is_none() {
            return Ok(());
        }
        let name_width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
        let s = &self.styler;
        let reset = s.reset();
        // Two-space marker prefix + name column + two-space gap before
        // description. Continuation lines (detect, hints) align under the
        // description column.
        let indent = " ".repeat(2 + name_width + 2);
        for row in &rows {
            let (status_color, marker) = match row.state {
                PkgEntryState::Disabled => (s.dim(), "-"),
                PkgEntryState::Installed => (s.green(), "\u{2713}"),
                PkgEntryState::Missing => (s.red(), "\u{2717}"),
            };
            write!(
                self.out,
                "{status_color}{marker}{reset} {bold}{name:<name_width$}{reset}",
                bold = s.bold(),
                name = row.name,
            )?;
            if let Some(desc) = &row.description {
                write!(self.out, "  {dim}{desc}{reset}", dim = s.dim())?;
            }
            writeln!(self.out)?;

            if let Some(detect) = &row.matched_detect {
                writeln!(
                    self.out,
                    "{indent}{dim}detect: {detect}{reset}",
                    dim = s.dim(),
                )?;
            }
            for hint_line in pkg_hint_lines(row) {
                writeln!(
                    self.out,
                    "{indent}{hint}{hint_line}{reset}",
                    hint = s.bold_yellow(),
                )?;
            }
        }
        if let Some(agg) = aggregate {
            writeln!(self.out)?;
            writeln!(
                self.out,
                "{hint}To install all missing via {mgr}: {cmd}{reset}",
                hint = s.bold_yellow(),
                mgr = agg.pkg_manager,
                cmd = agg.command,
            )?;
        }
        Ok(())
    }

    fn render_doctor_section_header(&mut self, section: DoctorSection) -> Result<(), OutputError> {
        if self.last_doctor_section == Some(section) {
            return Ok(());
        }
        if self.last_doctor_section.is_some() {
            writeln!(self.out)?;
        }
        let title = doctor_section_title(section);
        let s = &self.styler;
        writeln!(self.out, "{}{}{}", s.bold(), title, s.reset())?;
        self.last_doctor_section = Some(section);
        Ok(())
    }

    fn render_doctor_check(&mut self, check: &DoctorCheck) -> Result<(), OutputError> {
        match check {
            DoctorCheck::SectionHeader { section } => {
                self.render_doctor_section_header(*section)?;
            }
            DoctorCheck::Check {
                section,
                label,
                severity,
                value,
                hint,
                detail,
            } => {
                self.render_doctor_section_header(*section)?;
                let s = &self.styler;
                let reset = s.reset();
                let color_open = match severity {
                    DoctorSeverity::Ok => s.green(),
                    DoctorSeverity::Info => "",
                    DoctorSeverity::Warn => s.yellow(),
                    DoctorSeverity::Bad => s.red(),
                };
                if let Some(hint) = hint {
                    writeln!(
                        self.out,
                        "  {label:<14} {color_open}{value}{reset}  {dim}{hint}{reset}",
                        dim = s.dim(),
                        label = label.as_str(),
                    )?;
                } else {
                    writeln!(
                        self.out,
                        "  {label:<14} {color_open}{value}{reset}",
                        label = label.as_str(),
                    )?;
                }
                for line in detail {
                    writeln!(self.out, "    {line}")?;
                }
            }
        }
        Ok(())
    }

    fn render_init_summary(&mut self, summary: &InitSummary) -> Result<(), OutputError> {
        writeln!(self.out, "Cloned into {}", summary.clone_path.display())?;
        if let Some(remote) = &summary.remote {
            writeln!(self.out, "  remote: {remote}")?;
        }
        match &summary.shell {
            Some(shell) => writeln!(self.out, "  shell:  {shell}")?,
            None => writeln!(self.out, "  shell:  (none configured)")?,
        }
        writeln!(self.out, "  pkgs:   {}", summary.pkg_count)?;
        writeln!(
            self.out,
            "Next: run `zenops apply` to realize this config on your system."
        )?;
        Ok(())
    }

    fn render_bootstrap_summary(&mut self, summary: &BootstrapSummary) -> Result<(), OutputError> {
        writeln!(
            self.out,
            "Initialized fresh zenops repo at {}",
            summary.repo_path.display()
        )?;
        match &summary.shell {
            Some(shell) => writeln!(self.out, "  shell:  {shell}")?,
            None => writeln!(self.out, "  shell:  (none configured)")?,
        }
        match &summary.name {
            Some(name) => writeln!(self.out, "  name:   {name}")?,
            None => writeln!(self.out, "  name:   (not set)")?,
        }
        match &summary.email {
            Some(email) => writeln!(self.out, "  email:  {email}")?,
            None => writeln!(self.out, "  email:  (not set)")?,
        }
        writeln!(self.out, "Next: edit config.toml, then run `zenops apply`.")?;
        Ok(())
    }
}

impl Output for TerminalRenderer<'_> {
    fn push(&mut self, event: Event) -> Result<(), OutputError> {
        match event {
            Event::Status(status) => {
                self.enter(Pending::StatusBlock)?;
                if self.show_diffs
                    && let Status::Generated {
                        want_content,
                        cur_content,
                        path,
                        status: FileStatus::Modified | FileStatus::New,
                    } = &status
                {
                    self.diffs.push(PendingDiff {
                        path: path.clone(),
                        cur_content: cur_content.clone(),
                        want_content: Arc::clone(want_content),
                    });
                }
                if let Some(line) = status_to_line(&status, self.show_clean) {
                    self.lines.push(line);
                }
                Ok(())
            }
            Event::AppliedAction(action) => {
                self.enter(Pending::StatusBlock)?;
                self.lines.push(action_to_line(&action));
                Ok(())
            }
            Event::PkgEntry(entry) => {
                match entry {
                    PkgEntry::NoPackageManagerDetected { supported } => {
                        // One-shot pre-block warning. Renders inline so the
                        // user sees it before any pkg rows; doesn't open a
                        // PkgBlock so the column-padding only considers
                        // actual rows.
                        self.enter(Pending::None)?;
                        writeln!(
                            self.out,
                            "note: no known package manager detected on PATH; install \
                             guidance will be hidden. Supported managers: {}.",
                            supported.join(", "),
                        )?;
                    }
                    PkgEntry::Pkg {
                        name,
                        key: _,
                        description,
                        state,
                        matched_detect,
                        install_hints,
                    } => {
                        self.enter(Pending::PkgBlock)?;
                        self.pkg_rows.push(PkgRow {
                            name,
                            description,
                            state,
                            matched_detect,
                            install_hints,
                        });
                    }
                    PkgEntry::AggregateInstall {
                        pkg_manager,
                        command,
                        packages: _,
                    } => {
                        self.enter(Pending::PkgBlock)?;
                        self.pkg_aggregate = Some(PkgAggregate {
                            pkg_manager,
                            command,
                        });
                    }
                }
                Ok(())
            }
            Event::DoctorCheck(check) => {
                self.enter(Pending::None)?;
                self.render_doctor_check(&check)
            }
            Event::InitSummary(summary) => {
                self.enter(Pending::None)?;
                self.render_init_summary(&summary)
            }
            Event::BootstrapSummary(summary) => {
                self.enter(Pending::None)?;
                self.render_bootstrap_summary(&summary)
            }
        }
    }

    fn finalize(&mut self) -> Result<(), OutputError> {
        if self.finalized {
            return Ok(());
        }
        self.finalized = true;
        self.flush_pending()
    }
}

fn doctor_section_title(section: DoctorSection) -> &'static str {
    match section {
        DoctorSection::System => "System",
        DoctorSection::Repo => "Config repo (~/.config/zenops)",
        DoctorSection::Config => "Config (~/.config/zenops/config.toml)",
        DoctorSection::PkgManager => "Package manager",
        DoctorSection::User => "User",
        DoctorSection::Shell => "Shell",
        DoctorSection::Packages => "Packages",
    }
}

/// One install-hint line per populated package manager. Used by both the
/// terminal renderer (each line gets the bold-yellow hint color) and (with
/// future managers) any caller wanting a textual rollup.
fn pkg_hint_lines(row: &PkgRow) -> Vec<String> {
    if !matches!(row.state, PkgEntryState::Missing) {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if !row.install_hints.brew.is_empty() {
        lines.push(format!("brew: {}", row.install_hints.brew.join(" ")));
    }
    // Extend in lockstep with PkgInstallHints / DetectedPackageManager when
    // adding a new manager.
    lines
}

#[cfg(test)]
mod tests {
    use similar_asserts::assert_eq;
    use zenops_safe_relative_path::{SafeRelativePath, SafeRelativePathBuf, srpath};

    use super::*;
    use crate::config_files::ConfigFilePath;

    fn home_path(rel: &str) -> ResolvedConfigFilePath {
        let srp = SafeRelativePath::from_relative_path(rel).unwrap();
        ResolvedConfigFilePath {
            path: ConfigFilePath::in_home(srp),
            full: Arc::from(Path::new("/home/test").join(rel)),
        }
    }

    fn render_status(status: Status, color: bool, show_diffs: bool) -> String {
        render_status_full(status, color, show_diffs, false)
    }

    fn render_status_full(
        status: Status,
        color: bool,
        show_diffs: bool,
        show_clean: bool,
    ) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, color, show_diffs, show_clean);
            r.push(Event::Status(status)).unwrap();
            r.finalize().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    fn render_action(action: AppliedAction) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.push(Event::AppliedAction(action)).unwrap();
            r.finalize().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    fn generated(cur: Option<&str>, want: &str, rel: &str, status: FileStatus) -> Status {
        Status::Generated {
            want_content: Arc::from(want),
            cur_content: cur.map(String::from),
            path: home_path(rel),
            status,
        }
    }

    #[test]
    fn generated_ok_emits_nothing() {
        let s = generated(Some("x\n"), "x\n", "a.toml", FileStatus::Ok);
        assert_eq!(render_status(s, false, false), "");
    }

    #[test]
    fn generated_ok_with_show_clean_renders_checkmark_line() {
        let s = generated(Some("x\n"), "x\n", "a.toml", FileStatus::Ok);
        assert_eq!(
            render_status_full(s, false, false, true),
            "✓  ~/a.toml  ok\n"
        );
    }

    #[test]
    fn generated_modified_renders_tilde_marker_and_modified_word() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        assert_eq!(render_status(s, false, false), "~  ~/a.toml  modified\n",);
    }

    #[test]
    fn generated_new_renders_plus_marker_and_missing_word() {
        let s = generated(None, "x\n", "a.toml", FileStatus::New);
        assert_eq!(render_status(s, false, false), "+  ~/a.toml  missing\n");
    }

    #[test]
    fn generated_modified_with_diff_renders_summary_then_blank_then_diff() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        let got = render_status(s, false, true);
        assert!(got.starts_with("~  ~/a.toml  modified\n"), "{got:?}");
        assert!(
            got.contains("\n--- ~/a.toml (current)\n+++ ~/a.toml (generated)\n"),
            "{got:?}",
        );
        assert!(got.contains("-a\n"), "{got:?}");
        assert!(got.contains("+b\n"), "{got:?}");
    }

    #[test]
    fn generated_new_with_diff_labels_dev_null() {
        let s = generated(None, "x\n", "a.toml", FileStatus::New);
        let got = render_status(s, false, true);
        assert!(got.starts_with("+  ~/a.toml  missing\n"), "{got:?}");
        assert!(
            got.contains("--- /dev/null\n+++ ~/a.toml (generated)\n"),
            "{got:?}",
        );
        assert!(got.contains("+x\n"), "{got:?}");
    }

    #[test]
    fn generated_with_diff_color_off_contains_no_ansi_escapes() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        let got = render_status(s, false, true);
        assert!(!got.contains('\x1b'), "unexpected ANSI escape: {got:?}");
    }

    #[test]
    fn generated_with_diff_color_on_emits_ansi_escapes() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        let got = render_status(s, true, true);
        assert!(got.contains("\x1b[31m-a\n\x1b[0m"), "{got:?}");
        assert!(got.contains("\x1b[32m+b\n\x1b[0m"), "{got:?}");
    }

    #[test]
    fn generated_empty_current_still_labels_as_current() {
        let s = generated(Some(""), "hi\n", "empty.toml", FileStatus::Modified);
        let got = render_status(s, false, true);
        assert!(got.contains("--- ~/empty.toml (current)\n"), "{got:?}");
    }

    fn symlink(real: &str, sym: &str, status: SymlinkStatus) -> Status {
        Status::Symlink {
            real: home_path(real),
            symlink: home_path(sym),
            status,
        }
    }

    #[test]
    fn symlink_ok_emits_nothing() {
        let s = symlink("src", "dst", SymlinkStatus::Ok);
        assert_eq!(render_status(s, false, false), "");
    }

    #[test]
    fn symlink_ok_with_show_clean_renders_checkmark_line() {
        let s = symlink("src", "dst", SymlinkStatus::Ok);
        assert_eq!(
            render_status_full(s, false, false, true),
            "✓  ~/dst → ~/src  ok\n",
        );
    }

    #[test]
    fn symlink_wrong_link_renders_arrow_and_actual_target() {
        let s = symlink(
            "src",
            "dst",
            SymlinkStatus::WrongLink(PathBuf::from("/other")),
        );
        assert_eq!(
            render_status(s, false, false),
            "✗  ~/dst → ~/src  wrong target /other\n",
        );
    }

    #[test]
    fn symlink_new_renders_plus_and_arrow() {
        assert_eq!(
            render_status(symlink("s", "d", SymlinkStatus::New), false, false),
            "+  ~/d → ~/s  missing\n",
        );
    }

    #[test]
    fn symlink_is_file_renders_cross_and_description() {
        assert_eq!(
            render_status(symlink("s", "d", SymlinkStatus::IsFile), false, false),
            "✗  ~/d  is a file, expected symlink\n",
        );
    }

    #[test]
    fn symlink_is_dir_renders_cross_and_description() {
        assert_eq!(
            render_status(symlink("s", "d", SymlinkStatus::IsDir), false, false),
            "✗  ~/d  is a dir, expected symlink\n",
        );
    }

    #[test]
    fn symlink_real_missing_reports_source_path() {
        assert_eq!(
            render_status(
                symlink("s", "d", SymlinkStatus::RealPathIsMissing),
                false,
                false,
            ),
            "✗  ~/s  symlink source missing\n",
        );
    }

    #[test]
    fn symlink_dst_dir_missing_reports_symlink_path() {
        assert_eq!(
            render_status(
                symlink(
                    "s",
                    "d",
                    SymlinkStatus::DstDirIsMissing { dir: home_path("") },
                ),
                false,
                false,
            ),
            "✗  ~/d  parent directory missing\n",
        );
    }

    fn zenops_path(rel: &str) -> ResolvedConfigFilePath {
        let srp = SafeRelativePath::from_relative_path(rel).unwrap();
        ResolvedConfigFilePath {
            path: ConfigFilePath::Zenops(Arc::from(srp)),
            full: Arc::from(Path::new("/home/test/.config/zenops").join(rel)),
        }
    }

    fn relpath(s: &str) -> SafeRelativePathBuf {
        srpath!("").safe_join(SafeRelativePath::from_relative_path(s).unwrap())
    }

    #[test]
    fn git_variants_render_expected_lines() {
        let repo = zenops_path("");
        let cases: Vec<(GitFileStatus, &str)> = vec![
            (
                GitFileStatus::Modified(relpath("a.toml")),
                "M  ~/.config/zenops/a.toml  modified\n",
            ),
            (
                GitFileStatus::Added(relpath("b.toml")),
                "A  ~/.config/zenops/b.toml  added\n",
            ),
            (
                GitFileStatus::Deleted(relpath("c.toml")),
                "D  ~/.config/zenops/c.toml  deleted\n",
            ),
            (
                GitFileStatus::Untracked(relpath("d.toml")),
                "?  ~/.config/zenops/d.toml  untracked\n",
            ),
            (
                GitFileStatus::Other {
                    code: SmolStr::new_static("UU"),
                    path: relpath("e.toml"),
                },
                "!  ~/.config/zenops/e.toml  status UU\n",
            ),
        ];
        for (status, want) in cases {
            let s = Status::Git {
                repo: repo.clone(),
                status,
            };
            assert_eq!(render_status(s, false, false), want);
        }
    }

    fn pkg_missing(pkg: &'static str, install_command: Option<&str>) -> Status {
        Status::Pkg {
            pkg: SmolStr::new_static(pkg),
            status: PkgStatus::Missing {
                install_command: install_command.map(String::from),
            },
        }
    }

    fn pkg_ok(pkg: &'static str) -> Status {
        Status::Pkg {
            pkg: SmolStr::new_static(pkg),
            status: PkgStatus::Ok,
        }
    }

    #[test]
    fn pkg_missing_with_install_command_includes_hint() {
        assert_eq!(
            render_status(
                pkg_missing("python", Some("brew install python")),
                false,
                false
            ),
            "✗  python  missing — install: brew install python\n",
        );
    }

    #[test]
    fn pkg_missing_without_install_command_is_terse() {
        assert_eq!(
            render_status(pkg_missing("python", None), false, false),
            "✗  python  missing\n",
        );
    }

    #[test]
    fn pkg_ok_without_show_clean_emits_nothing() {
        assert_eq!(render_status(pkg_ok("python"), false, false), "");
    }

    #[test]
    fn pkg_ok_with_show_clean_renders_checkmark_line() {
        assert_eq!(
            render_status_full(pkg_ok("python"), false, false, true),
            "✓  python  ok\n",
        );
    }

    #[test]
    fn git_repo_clean_without_show_clean_emits_nothing() {
        let s = Status::GitRepoClean {
            repo: zenops_path(""),
        };
        assert_eq!(render_status(s, false, false), "");
    }

    #[test]
    fn git_repo_clean_with_show_clean_renders_checkmark_line() {
        let s = Status::GitRepoClean {
            repo: zenops_path(""),
        };
        assert_eq!(
            render_status_full(s, false, false, true),
            "✓  ~/.config/zenops  clean\n",
        );
    }

    #[test]
    fn applied_actions_render_expected_lines() {
        assert_eq!(
            render_action(AppliedAction::UpdatedFile(home_path("a.toml"))),
            "✓  ~/a.toml  updated\n",
        );
        assert_eq!(
            render_action(AppliedAction::CreatedFile(home_path("a.toml"))),
            "✓  ~/a.toml  created\n",
        );
        assert_eq!(
            render_action(AppliedAction::CreatedSymlink {
                real: home_path("src"),
                symlink: home_path("dst"),
            }),
            "✓  ~/dst → ~/src  linked\n",
        );
        assert_eq!(
            render_action(AppliedAction::CreatedDir(home_path("subdir"))),
            "✓  ~/subdir  mkdir\n",
        );
    }

    #[test]
    fn multiple_lines_pad_path_column_to_widest() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.push(Event::Status(pkg_missing("py", None))).unwrap();
            r.push(Event::Status(generated(
                Some("a\n"),
                "b\n",
                "long/nested/path/file.toml",
                FileStatus::Modified,
            )))
            .unwrap();
            r.finalize().unwrap();
        }
        let got = String::from_utf8(buf).unwrap();
        let wide = "~/long/nested/path/file.toml".chars().count();
        let short = "py".chars().count();
        let pad = wide - short;
        let expected = format!(
            "✗  py{spaces}  missing\n~  ~/long/nested/path/file.toml  modified\n",
            spaces = " ".repeat(pad),
        );
        assert_eq!(got, expected);
    }

    #[test]
    fn finalize_with_no_events_emits_nothing() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.finalize().unwrap();
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "");
    }

    #[test]
    fn finalize_is_idempotent() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.push(Event::Status(pkg_missing("x", None))).unwrap();
            r.finalize().unwrap();
            r.finalize().unwrap();
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "✗  x  missing\n");
    }

    #[test]
    fn color_on_wraps_marker_path_and_description_with_expected_escapes() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        let got = render_status(s, true, false);
        // yellow marker
        assert!(got.contains("\x1b[33m~\x1b[0m"), "{got:?}");
        // dim path
        assert!(got.contains("\x1b[2m~/a.toml\x1b[0m"), "{got:?}");
        // yellow "modified"
        assert!(got.contains("\x1b[33mmodified\x1b[0m"), "{got:?}");
    }

    #[test]
    fn color_on_pkg_missing_install_command_is_bold_yellow() {
        let got = render_status(pkg_missing("py", Some("brew install py")), true, false);
        assert!(got.contains("\x1b[1;33mbrew install py\x1b[0m"), "{got:?}",);
    }

    #[test]
    fn ok_description_is_green_with_color_on() {
        let s = generated(Some("x\n"), "x\n", "a.toml", FileStatus::Ok);
        let got = render_status_full(s, true, false, true);
        assert!(got.contains("\x1b[32m✓\x1b[0m"), "{got:?}");
        assert!(got.contains("\x1b[32mok\x1b[0m"), "{got:?}");
    }

    #[test]
    fn clean_description_is_green_with_color_on() {
        let s = Status::GitRepoClean {
            repo: zenops_path(""),
        };
        let got = render_status_full(s, true, false, true);
        assert!(got.contains("\x1b[32m✓\x1b[0m"), "{got:?}");
        assert!(got.contains("\x1b[32mclean\x1b[0m"), "{got:?}");
    }

    #[test]
    fn symlink_ok_splits_zenops_prefix_and_bolds_arrow() {
        let s = Status::Symlink {
            real: zenops_path("configs/helix/config.toml"),
            symlink: home_path(".config/helix/config.toml"),
            status: SymlinkStatus::Ok,
        };
        let got = render_status_full(s, true, false, true);
        // left symlink path: dim
        assert!(
            got.contains("\x1b[2m~/.config/helix/config.toml\x1b[0m"),
            "{got:?}",
        );
        // arrow: bold
        assert!(got.contains("\x1b[1m → \x1b[0m"), "{got:?}");
        // right path zenops prefix: extra-dim (fades below the left-side dim)
        assert!(
            got.contains("\x1b[2;38;5;248m~/.config/zenops\x1b[0m"),
            "{got:?}",
        );
        // right path remainder: no opening escape, then reset
        assert!(got.contains("/configs/helix/config.toml\x1b[0m"), "{got:?}");
        // ok label: green
        assert!(got.contains("\x1b[32mok\x1b[0m"), "{got:?}");
    }

    #[test]
    fn git_row_splits_zenops_prefix() {
        let repo = zenops_path("");
        let s = Status::Git {
            repo,
            status: GitFileStatus::Modified(relpath("configs/helix/config.toml")),
        };
        let got = render_status(s, true, false);
        assert!(
            got.contains("\x1b[2;38;5;248m~/.config/zenops\x1b[0m"),
            "{got:?}",
        );
        assert!(got.contains("/configs/helix/config.toml\x1b[0m"), "{got:?}");
        // tail must not be wrapped in a dim open
        assert!(
            !got.contains("\x1b[2m/configs/helix/config.toml"),
            "tail should not be dim: {got:?}",
        );
    }

    #[test]
    fn path_column_padding_matches_visible_width_for_split_paths() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, true, false, false);
            // Short zenops-rooted path (splits): ~/.config/zenops/a.toml (22 chars visible)
            r.push(Event::Status(Status::Git {
                repo: zenops_path(""),
                status: GitFileStatus::Modified(relpath("a.toml")),
            }))
            .unwrap();
            // Longer home path (single dim segment)
            r.push(Event::Status(generated(
                Some("a\n"),
                "b\n",
                "long/nested/path/file.toml",
                FileStatus::Modified,
            )))
            .unwrap();
            r.finalize().unwrap();
        }
        let got = String::from_utf8(buf).unwrap();
        // Strip ANSI escapes to count visible chars per line.
        let stripped: String = {
            let mut out = String::new();
            let mut in_esc = false;
            for c in got.chars() {
                if in_esc {
                    if c == 'm' {
                        in_esc = false;
                    }
                    continue;
                }
                if c == '\x1b' {
                    in_esc = true;
                    continue;
                }
                out.push(c);
            }
            out
        };
        let lines: Vec<&str> = stripped.lines().collect();
        assert_eq!(lines.len(), 2, "{stripped:?}");
        let short_visible = "~/.config/zenops/a.toml".chars().count();
        let long_visible = "~/long/nested/path/file.toml".chars().count();
        let pad = long_visible - short_visible;
        let expected_short = format!("M  ~/.config/zenops/a.toml{}  modified", " ".repeat(pad));
        let expected_long = "~  ~/long/nested/path/file.toml  modified";
        assert_eq!(lines[0], expected_short, "{stripped:?}");
        assert_eq!(lines[1], expected_long, "{stripped:?}");
    }

    // ---- JsonOutput -------------------------------------------------------

    fn json_line_for_status(status: Status) -> serde_json::Value {
        let mut buf: Vec<u8> = Vec::new();
        JsonOutput::new(&mut buf)
            .push(Event::Status(status))
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.ends_with('\n'), "JSON line must end with newline: {s:?}");
        assert_eq!(
            s.matches('\n').count(),
            1,
            "expected exactly one line: {s:?}"
        );
        serde_json::from_str(s.trim_end()).unwrap()
    }

    fn json_line_for_action(action: AppliedAction) -> serde_json::Value {
        let mut buf: Vec<u8> = Vec::new();
        JsonOutput::new(&mut buf)
            .push(Event::AppliedAction(action))
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        serde_json::from_str(s.trim_end()).unwrap()
    }

    #[test]
    fn json_status_generated_tags_event_and_kind() {
        let v = json_line_for_status(generated(
            Some("a\n"),
            "b\n",
            "alpha.toml",
            FileStatus::Modified,
        ));
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "generated");
        assert_eq!(v["want_content"], "b\n");
        assert_eq!(v["cur_content"], "a\n");
        assert_eq!(v["status"], "modified");
    }

    #[test]
    fn json_status_symlink_wrong_link_preserves_target_path() {
        let v = json_line_for_status(symlink(
            "src",
            "dst",
            SymlinkStatus::WrongLink(PathBuf::from("/other")),
        ));
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "symlink");
        assert_eq!(v["status"]["kind"], "wrong_link");
        assert_eq!(v["status"]["data"], "/other");
    }

    #[test]
    fn json_status_git_tags_nested_git_status_kind() {
        let repo = zenops_path("");
        let v = json_line_for_status(Status::Git {
            repo,
            status: GitFileStatus::Untracked(relpath("x.toml")),
        });
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "git");
        assert_eq!(v["status"]["kind"], "untracked");
        assert_eq!(v["status"]["data"], "x.toml");
    }

    #[test]
    fn json_status_pkg_missing_preserves_install_command() {
        let v = json_line_for_status(pkg_missing("python", Some("brew install python")));
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "pkg");
        assert_eq!(v["pkg"], "python");
        assert_eq!(v["status"]["kind"], "missing");
        assert_eq!(
            v["status"]["data"]["install_command"],
            "brew install python"
        );
    }

    #[test]
    fn json_status_pkg_ok_tags_kind_ok() {
        let v = json_line_for_status(pkg_ok("python"));
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "pkg");
        assert_eq!(v["pkg"], "python");
        assert_eq!(v["status"]["kind"], "ok");
    }

    #[test]
    fn json_status_git_repo_clean_emits_event() {
        let repo = zenops_path("");
        let v = json_line_for_status(Status::GitRepoClean { repo });
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "git_repo_clean");
        assert_eq!(v["repo"]["path"]["path"], "");
    }

    #[test]
    fn json_applied_action_tags_event_and_kind() {
        let v = json_line_for_action(AppliedAction::CreatedFile(home_path("a.toml")));
        assert_eq!(v["event"], "applied_action");
        assert_eq!(v["kind"], "created_file");
    }

    #[test]
    fn json_multiple_events_produce_jsonl() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut out = JsonOutput::new(&mut buf);
            out.push(Event::Status(pkg_missing("python", None)))
                .unwrap();
            out.push(Event::AppliedAction(AppliedAction::CreatedDir(home_path(
                "d",
            ))))
            .unwrap();
        }
        let s = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2, "{s:?}");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first["event"], "status");
        assert_eq!(second["event"], "applied_action");
    }

    // ---- New event types: PkgEntry / DoctorCheck / InitSummary -----------

    fn pkg_entry_pkg(name: &'static str, state: PkgEntryState) -> PkgEntry {
        PkgEntry::Pkg {
            name: SmolStr::new_static(name),
            key: SmolStr::new_static(name),
            description: None,
            state,
            matched_detect: None,
            install_hints: PkgInstallHints::default(),
        }
    }

    fn render_pkg_entries(entries: Vec<PkgEntry>, color: bool) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, color, false, false);
            for e in entries {
                r.push(Event::PkgEntry(e)).unwrap();
            }
            r.finalize().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn pkg_entries_pad_name_column_to_widest() {
        let got = render_pkg_entries(
            vec![
                pkg_entry_pkg("py", PkgEntryState::Missing),
                pkg_entry_pkg("starship", PkgEntryState::Installed),
            ],
            false,
        );
        let lines: Vec<&str> = got.lines().collect();
        // "starship" (8 chars) is the widest name; "py" should pad to 8.
        assert_eq!(lines[0], "✗ py      ", "{got:?}");
        assert_eq!(lines[1], "✓ starship", "{got:?}");
    }

    #[test]
    fn pkg_entry_disabled_uses_dash_marker() {
        let got = render_pkg_entries(vec![pkg_entry_pkg("ghost", PkgEntryState::Disabled)], false);
        assert!(got.starts_with("- ghost"), "{got:?}");
    }

    #[test]
    fn pkg_entry_missing_with_brew_hint_renders_indented_hint_line() {
        let got = render_pkg_entries(
            vec![PkgEntry::Pkg {
                name: SmolStr::new_static("foo"),
                key: SmolStr::new_static("foo"),
                description: None,
                state: PkgEntryState::Missing,
                matched_detect: None,
                install_hints: PkgInstallHints {
                    brew: vec!["foo-formula".into()],
                },
            }],
            false,
        );
        assert!(got.contains("✗ foo"), "{got:?}");
        assert!(got.contains("brew: foo-formula"), "{got:?}");
    }

    #[test]
    fn pkg_aggregate_install_renders_blank_line_then_footer() {
        let got = render_pkg_entries(
            vec![
                pkg_entry_pkg("foo", PkgEntryState::Missing),
                PkgEntry::AggregateInstall {
                    pkg_manager: "brew".into(),
                    command: "brew install foo".into(),
                    packages: vec!["foo".into()],
                },
            ],
            false,
        );
        // Last two non-empty lines: aggregate footer follows a blank line.
        let lines: Vec<&str> = got.lines().collect();
        let footer_idx = lines
            .iter()
            .position(|l| l.contains("To install all missing"))
            .expect("expected footer line");
        assert_eq!(lines[footer_idx - 1], "", "{got:?}");
        assert!(
            lines[footer_idx].contains("via brew: brew install foo"),
            "{got:?}",
        );
    }

    #[test]
    fn pkg_no_manager_warning_renders_inline_before_pkg_block() {
        let got = render_pkg_entries(
            vec![
                PkgEntry::NoPackageManagerDetected {
                    supported: vec!["brew".into()],
                },
                pkg_entry_pkg("foo", PkgEntryState::Missing),
            ],
            false,
        );
        let lines: Vec<&str> = got.lines().collect();
        assert!(lines[0].contains("no known package manager"), "{got:?}");
        assert!(lines[0].contains("Supported managers: brew"), "{got:?}");
        assert!(lines[1].contains("foo"), "{got:?}");
    }

    fn render_doctor_checks(checks: Vec<DoctorCheck>, color: bool) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, color, false, false);
            for c in checks {
                r.push(Event::DoctorCheck(c)).unwrap();
            }
            r.finalize().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    fn doctor_check(
        section: DoctorSection,
        label: &'static str,
        severity: DoctorSeverity,
        value: &str,
        hint: Option<&str>,
    ) -> DoctorCheck {
        DoctorCheck::Check {
            section,
            label: SmolStr::new_static(label),
            severity,
            value: value.to_string(),
            hint: hint.map(String::from),
            detail: Vec::new(),
        }
    }

    #[test]
    fn doctor_check_groups_by_section_with_blank_separator() {
        let got = render_doctor_checks(
            vec![
                DoctorCheck::SectionHeader {
                    section: DoctorSection::System,
                },
                doctor_check(
                    DoctorSection::System,
                    "os:",
                    DoctorSeverity::Info,
                    "macos",
                    None,
                ),
                DoctorCheck::SectionHeader {
                    section: DoctorSection::Repo,
                },
                doctor_check(
                    DoctorSection::Repo,
                    "git repo:",
                    DoctorSeverity::Ok,
                    "yes",
                    None,
                ),
            ],
            false,
        );
        // Bold-stripped, but no color: lines are plain.
        let want = "System\n  os:            macos\n\nConfig repo (~/.config/zenops)\n  git repo:      yes\n";
        assert_eq!(got, want, "{got:?}");
    }

    #[test]
    fn doctor_check_with_hint_renders_hint_after_value() {
        let got = render_doctor_checks(
            vec![doctor_check(
                DoctorSection::System,
                "git:",
                DoctorSeverity::Bad,
                "not found on PATH",
                Some("install git"),
            )],
            false,
        );
        assert!(got.contains("git:"), "{got:?}");
        assert!(got.contains("not found on PATH"), "{got:?}");
        assert!(got.contains("install git"), "{got:?}");
    }

    #[test]
    fn doctor_check_with_detail_indents_each_line_under_row() {
        let got = render_doctor_checks(
            vec![DoctorCheck::Check {
                section: DoctorSection::Config,
                label: SmolStr::new_static("status:"),
                severity: DoctorSeverity::Bad,
                value: "parse error".into(),
                hint: None,
                detail: vec!["/path/to/config.toml".into(), "expected `]`".into()],
            }],
            false,
        );
        assert!(got.contains("    /path/to/config.toml\n"), "{got:?}");
        assert!(got.contains("    expected `]`\n"), "{got:?}");
    }

    #[test]
    fn init_summary_renders_summary_with_remote_shell_and_pkg_count() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.push(Event::InitSummary(InitSummary {
                clone_path: PathBuf::from("/home/test/.config/zenops"),
                remote: Some("git@example.com:cfg.git".into()),
                shell: Some("bash".into()),
                pkg_count: 12,
            }))
            .unwrap();
            r.finalize().unwrap();
        }
        let got = String::from_utf8(buf).unwrap();
        assert!(
            got.contains("Cloned into /home/test/.config/zenops"),
            "{got:?}",
        );
        assert!(got.contains("remote: git@example.com:cfg.git"), "{got:?}");
        assert!(got.contains("shell:  bash"), "{got:?}");
        assert!(got.contains("pkgs:   12"), "{got:?}");
        assert!(got.contains("Next: run `zenops apply`"), "{got:?}");
    }

    fn json_line_for_pkg_entry(entry: PkgEntry) -> serde_json::Value {
        let mut buf: Vec<u8> = Vec::new();
        JsonOutput::new(&mut buf)
            .push(Event::PkgEntry(entry))
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        serde_json::from_str(s.trim_end()).unwrap()
    }

    #[test]
    fn json_pkg_entry_pkg_tags_event_and_kind_with_state() {
        let v = json_line_for_pkg_entry(PkgEntry::Pkg {
            name: SmolStr::new_static("starship"),
            key: SmolStr::new_static("starship"),
            description: Some("cross-shell prompt".into()),
            state: PkgEntryState::Missing,
            matched_detect: None,
            install_hints: PkgInstallHints {
                brew: vec!["starship".into()],
            },
        });
        assert_eq!(v["event"], "pkg_entry");
        assert_eq!(v["kind"], "pkg");
        assert_eq!(v["name"], "starship");
        assert_eq!(v["key"], "starship");
        assert_eq!(v["state"], "missing");
        assert_eq!(v["install_hints"]["brew"][0], "starship");
    }

    #[test]
    fn json_pkg_entry_aggregate_install_carries_command_and_packages() {
        let v = json_line_for_pkg_entry(PkgEntry::AggregateInstall {
            pkg_manager: "brew".into(),
            command: "brew install foo bar".into(),
            packages: vec!["foo".into(), "bar".into()],
        });
        assert_eq!(v["event"], "pkg_entry");
        assert_eq!(v["kind"], "aggregate_install");
        assert_eq!(v["pkg_manager"], "brew");
        assert_eq!(v["command"], "brew install foo bar");
        assert_eq!(v["packages"][0], "foo");
        assert_eq!(v["packages"][1], "bar");
    }

    #[test]
    fn json_pkg_entry_no_manager_warning_is_event() {
        let v = json_line_for_pkg_entry(PkgEntry::NoPackageManagerDetected {
            supported: vec!["brew".into()],
        });
        assert_eq!(v["event"], "pkg_entry");
        assert_eq!(v["kind"], "no_package_manager_detected");
        assert_eq!(v["supported"][0], "brew");
    }

    fn json_line_for_doctor_check(check: DoctorCheck) -> Option<serde_json::Value> {
        let mut buf: Vec<u8> = Vec::new();
        JsonOutput::new(&mut buf)
            .push(Event::DoctorCheck(check))
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        if s.is_empty() {
            None
        } else {
            Some(serde_json::from_str(s.trim_end()).unwrap())
        }
    }

    #[test]
    fn json_doctor_check_includes_section_severity_label_value() {
        let v = json_line_for_doctor_check(doctor_check(
            DoctorSection::System,
            "os:",
            DoctorSeverity::Info,
            "linux",
            None,
        ))
        .expect("Check variant should emit JSON");
        assert_eq!(v["event"], "doctor_check");
        assert_eq!(v["kind"], "check");
        assert_eq!(v["section"], "system");
        assert_eq!(v["label"], "os:");
        assert_eq!(v["severity"], "info");
        assert_eq!(v["value"], "linux");
    }

    #[test]
    fn json_doctor_check_section_header_is_skipped() {
        let v = json_line_for_doctor_check(DoctorCheck::SectionHeader {
            section: DoctorSection::Packages,
        });
        assert!(
            v.is_none(),
            "section header should not produce a JSON line, got: {v:?}",
        );
    }

    #[test]
    fn json_init_summary_includes_all_fields() {
        let mut buf: Vec<u8> = Vec::new();
        JsonOutput::new(&mut buf)
            .push(Event::InitSummary(InitSummary {
                clone_path: PathBuf::from("/home/test/.config/zenops"),
                remote: Some("git@example.com:cfg.git".into()),
                shell: Some("zsh".into()),
                pkg_count: 7,
            }))
            .unwrap();
        let s = String::from_utf8(buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(s.trim_end()).unwrap();
        assert_eq!(v["event"], "init_summary");
        assert_eq!(v["clone_path"], "/home/test/.config/zenops");
        assert_eq!(v["remote"], "git@example.com:cfg.git");
        assert_eq!(v["shell"], "zsh");
        assert_eq!(v["pkg_count"], 7);
    }

    #[test]
    fn terminal_renderer_flushes_status_block_before_pkg_block() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.push(Event::Status(pkg_missing("py", None))).unwrap();
            r.push(Event::PkgEntry(pkg_entry_pkg(
                "foo",
                PkgEntryState::Missing,
            )))
            .unwrap();
            r.finalize().unwrap();
        }
        let got = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = got.lines().collect();
        // First line = the status row from the Status event; second line = the
        // first pkg row. The two blocks are independent — no shared
        // column padding bleeds across.
        assert!(lines[0].starts_with("✗  py"), "{got:?}");
        assert!(lines[1].starts_with("✗ foo"), "{got:?}");
    }

    // ---- Error propagation -----------------------------------------------

    struct FailingWriter;

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn terminal_renderer_surfaces_writer_errors_on_finalize() {
        let mut w = FailingWriter;
        let mut r = TerminalRenderer::new(&mut w, false, false, false);
        r.push(Event::Status(pkg_missing("x", None))).unwrap();
        let err = r.finalize().unwrap_err();
        assert!(matches!(err, OutputError::Io(_)), "unexpected: {err:?}");
    }

    #[test]
    fn json_output_surfaces_writer_errors() {
        let mut w = FailingWriter;
        let err = JsonOutput::new(&mut w)
            .push(Event::Status(pkg_missing("x", None)))
            .unwrap_err();
        // `serde_json::to_writer` wraps the underlying IO failure in its own
        // `serde_json::Error`, which lifts into `OutputError::Json`. Either
        // variant is acceptable — we just care the error surfaced.
        assert!(
            matches!(err, OutputError::Io(_) | OutputError::Json(_)),
            "unexpected: {err:?}",
        );
    }
}
