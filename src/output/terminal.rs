//! Human-readable, column-aligned text renderer.
//!
//! Status / applied-action events are buffered into a single column-aligned
//! block flushed on transition to a different event category or on
//! `finalize`. Pkg-entry events are similarly buffered (so the name column
//! can pad to the widest visible row). Doctor checks and init summaries
//! render eagerly.

use similar::{ChangeTag, TextDiff};
use smol_str::SmolStr;
use std::{io::Write, sync::Arc};

use crate::{ansi::Styler, config_files::ConfigFilePath, git::GitFileStatus};

use super::{
    AppliedAction, BootstrapSummary, DoctorCheck, DoctorSection, DoctorSeverity, Event, FileStatus,
    InitSummary, Output, OutputError, PkgEntry, PkgEntryState, PkgInstallHints, PkgStatus,
    ResolvedConfigFilePath, Status, SymlinkStatus,
};

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
