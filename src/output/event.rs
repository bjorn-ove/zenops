//! Per-command event payload types.
//!
//! [`Status`] and [`AppliedAction`] cover the apply/status event stream — each
//! variant maps to a marker + colored row in the column-aligned block.
//! `pkg`, `doctor`, and `init` produce different output shapes (pkg-list
//! rows, labeled environment checks, single-shot init summary) that don't
//! fit that table, so they get their own event types pushed through
//! the [`super::Event`] enum.

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use schemars::JsonSchema;
use serde::Serialize;
use smol_str::SmolStr;

use crate::{
    config_files::{ConfigFileDirs, ConfigFilePath},
    git::GitFileStatus,
};

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

/// Layout shape inferred from the imported path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportType {
    /// `~/.config/<x>` — adds a `[[pkg.<key>.configs]]` entry of `type =
    /// ".config"`.
    DotConfig,
    /// `~/.<x>` — adds a `[[pkg.<key>.configs]]` entry of `type = "home"`.
    Home,
}

/// Which kind of import the plan describes. Drives the header line in
/// the human renderer and lets JSON consumers tell the modes apart
/// without inspecting `created_pkg` / the `toml_changes` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// Importing into a brand-new `[pkg.<key>]` block.
    NewPkg,
    /// Adding a single file under an already-managed on-disk root.
    Extend,
    /// Diffing an already-managed root against its current `symlinks`
    /// array — adds files found on disk, drops paths whose home-side
    /// counterpart is gone.
    Reconcile,
}

/// One filesystem-side intent in an [`ImportPlan`]. Each variant
/// describes a single decision the apply phase will (or will not) act on;
/// new variants land here as `import` grows new shapes (per-file
/// include/exclude, replacing an already-managed file, etc.) without
/// reshaping the surrounding plan/event flow.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportFileAction {
    /// Move a file into the zenops repo and replace the original with a
    /// symlink pointing at the new location. `rel` is the path relative
    /// to [`ImportPlan::source`]; the in-repo destination is
    /// [`ImportPlan::repo_dest`] joined with the same tail.
    MoveAndSymlink {
        /// Path relative to the imported source root.
        rel: PathBuf,
    },
    /// Encountered during the walk but intentionally not touched (existing
    /// symlink, non-regular file, future: user-excluded). `path` is
    /// relative to [`ImportPlan::source`]; `reason` is a stable
    /// snake_case tag JSON consumers can switch on.
    Skip {
        /// Path relative to the imported source root.
        path: PathBuf,
        /// Stable reason tag (`"symlink"`, `"other"`,
        /// `"symlink_elsewhere"`, `"present_but_not_linked"`, …).
        reason: SmolStr,
    },
    /// Reconcile-mode action: the in-repo copy at `rel` will be deleted
    /// because the home-side counterpart no longer exists. `rel` is
    /// relative to the entry's repo destination (i.e.
    /// `[[pkg.<key>.configs]].source` joined with this rel).
    RemoveFromRepo {
        /// Path relative to the imported source root (== repo dest tail).
        rel: PathBuf,
    },
}

/// One config.toml-side intent in an [`ImportPlan`]. Same extensibility
/// model as [`ImportFileAction`]: future shapes (extending an existing
/// configs entry, replacing one outright, …) land as new variants without
/// touching the surrounding flow.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportTomlChange {
    /// Insert a new `[pkg.<key>]` block. `block_preview` is a copy-paste
    /// representation of the lines the apply phase will write.
    CreatePkg {
        /// Pkg key the new block lives under.
        pkg: SmolStr,
        /// Brew packages baked into the new block's `install_hint` (empty
        /// when the user passed `--no-install-hint`).
        brew_packages: Vec<String>,
        /// TOML snippet representing the new block (header + body).
        block_preview: String,
    },
    /// Append a new `[[pkg.<key>.configs]]` table entry. `entry_preview`
    /// is the body of the entry (the `type`/`source`/`symlinks`/… fields)
    /// without the `[[pkg.<key>.configs]]` header line.
    AppendConfigsEntry {
        /// Pkg key the entry is appended under.
        pkg: SmolStr,
        /// TOML snippet representing the entry's body.
        entry_preview: String,
    },
    /// Append paths to the `symlinks` array of an existing
    /// `[[pkg.<key>.configs]]` entry. `paths` is the delta — entries that
    /// weren't already listed; an empty `paths` means the file rel was
    /// already present and the array is untouched.
    AppendSymlinks {
        /// Pkg key whose configs entry gains the new symlinks.
        pkg: SmolStr,
        /// Index of the configs entry inside `[pkg.<key>].configs` (the
        /// array of tables).
        config_index: usize,
        /// New paths being appended to the `symlinks` array (delta only).
        paths: Vec<String>,
        /// TOML snippet showing the array as it will read after the
        /// append, e.g. `symlinks = ["existing", "new"]`.
        array_after_preview: String,
    },
    /// Drop paths from the `symlinks` array of an existing
    /// `[[pkg.<key>.configs]]` entry. Emitted by reconcile mode when a
    /// path listed in the array no longer has a home-side counterpart.
    /// The repo-side copy is removed via a paired
    /// [`ImportFileAction::RemoveFromRepo`] action.
    TrimSymlinks {
        /// Pkg key whose configs entry loses the listed symlinks.
        pkg: SmolStr,
        /// Index of the configs entry inside `[pkg.<key>].configs`.
        config_index: usize,
        /// Paths being removed from the `symlinks` array.
        paths: Vec<String>,
        /// TOML snippet showing the array as it will read after the
        /// trim, e.g. `symlinks = ["surviving"]`.
        array_after_preview: String,
    },
}

/// Pre-apply plan for `zenops import`. Emitted before any mutation runs
/// so the user (and JSON consumers) can review every intended change
/// before the confirmation prompt.
///
/// `file_actions` and `toml_changes` are lists of typed actions so new
/// import shapes (single-file extend, reconcile-existing-entry, …) can
/// land as new variants without restructuring the event itself.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImportPlan {
    /// Pkg key the import will land under (`[pkg.<key>]`).
    pub pkg: SmolStr,
    /// `true` when this plan creates the `[pkg.<key>]` block; `false`
    /// when extending or reconciling an existing pkg. Equivalent to
    /// `mode == ImportMode::NewPkg`; kept for backward compatibility
    /// with v0.14.0 JSON consumers.
    pub created_pkg: bool,
    /// Which kind of import this is (new pkg / extend / reconcile).
    pub mode: ImportMode,
    /// Layout shape that drove path classification.
    pub r#type: ImportType,
    /// Where the files are being imported from (absolute,
    /// post-canonicalize). Per-file actions list paths relative to this.
    pub source: PathBuf,
    /// Where the files will live in the zenops repo (absolute). Per-file
    /// actions list repo-side paths relative to this.
    pub repo_dest: PathBuf,
    /// Filesystem-side intents (one per source file, including skips).
    pub file_actions: Vec<ImportFileAction>,
    /// `config.toml`-side intents.
    pub toml_changes: Vec<ImportTomlChange>,
}

/// Post-apply confirmation event for `zenops import`. Emitted after
/// `config.toml` has been updated and the filesystem moves have landed,
/// so the renderer can print a "next steps" hint that wouldn't make
/// sense pre-apply (or under `--dry-run`).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImportApplied {
    /// Pkg key the import landed under.
    pub pkg: SmolStr,
    /// `true` when the plan made no filesystem or `config.toml` changes
    /// (reconcile run on an already-in-sync managed root). The renderer
    /// suppresses the "next steps" hint in that case.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_noop: bool,
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
    /// File deleted from disk (today: a repo-side copy dropped by
    /// `zenops import` reconcile when the home-side counterpart is gone).
    RemovedFile(ResolvedConfigFilePath),
    /// Now-empty parent directory removed after [`Self::RemovedFile`]
    /// pruned its last child.
    RemovedDir(ResolvedConfigFilePath),
}
