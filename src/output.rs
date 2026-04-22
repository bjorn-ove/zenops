use similar::{ChangeTag, TextDiff};
use smol_str::SmolStr;
use std::{
    fmt,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;

use crate::{
    ansi::Styler,
    config_files::{ConfigFileDirs, ConfigFilePath},
    git::GitFileStatus,
};

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SymlinkStatus {
    Ok,
    WrongLink(PathBuf),
    /// The symlink does not exist and must be created
    New,
    /// The path is a file and not a symlink
    IsFile,
    /// The path is a directory and not a symlink
    IsDir,
    /// The symlink exists and points to the correct location, but the source does not exist.
    RealPathIsMissing,
    /// The directory that should contain the symlink is missing
    DstDirIsMissing,
}

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Ok,
    Modified,
    New,
}

#[derive(Debug, PartialEq, Clone, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum PkgStatus {
    Ok,
    /// A pkg the user expects to be present (`enable = "on"`) whose detect
    /// strategies don't match on the current host. `install_command` is the
    /// ready-to-run shell line (`"brew install python"`) when a package
    /// manager with a non-empty hint is detected, `None` otherwise.
    Missing {
        install_command: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedConfigFilePath {
    pub path: ConfigFilePath,
    pub full: Arc<Path>,
}

impl ResolvedConfigFilePath {
    pub fn resolve(path: ConfigFilePath, dirs: &ConfigFileDirs) -> Self {
        let full = Arc::from(path.resolved(dirs));
        Self { path, full }
    }

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

#[derive(Debug, PartialEq, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    Generated {
        want_content: Arc<str>,
        cur_content: Option<String>,
        path: ResolvedConfigFilePath,
        status: FileStatus,
    },
    Symlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
        status: SymlinkStatus,
    },
    Git {
        repo: ResolvedConfigFilePath,
        status: GitFileStatus,
    },
    /// Emitted when the zenops config repo has no uncommitted changes. The
    /// dirty case is covered per-file by `Git`.
    GitRepoClean {
        repo: ResolvedConfigFilePath,
    },
    Pkg {
        pkg: SmolStr,
        status: PkgStatus,
    },
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppliedAction {
    UpdatedFile(ResolvedConfigFilePath),
    CreatedFile(ResolvedConfigFilePath),
    CreatedSymlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
    },
    CreatedDir(ResolvedConfigFilePath),
}

/// Errors surfaced by [`Output`] implementations. `Io` comes from
/// `writeln!` / `write!` on the backing `Write`; `Json` from
/// `serde_json::to_writer` in `JsonOutput`. Both variants use
/// `#[error(transparent)]` so the user sees the underlying message
/// verbatim.
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub trait Output {
    fn push_status(&mut self, status: Status) -> Result<(), OutputError>;
    fn push_applied_action(&mut self, action: AppliedAction) -> Result<(), OutputError>;
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
    pub fn new(out: &'w mut dyn Write) -> Self {
        Self { out }
    }
}

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Status(Status),
    AppliedAction(AppliedAction),
}

impl Output for JsonOutput<'_> {
    fn push_status(&mut self, status: Status) -> Result<(), OutputError> {
        serde_json::to_writer(&mut *self.out, &Event::Status(status))?;
        writeln!(self.out)?;
        Ok(())
    }

    fn push_applied_action(&mut self, action: AppliedAction) -> Result<(), OutputError> {
        serde_json::to_writer(&mut *self.out, &Event::AppliedAction(action))?;
        writeln!(self.out)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Dim,
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
            Style::Dim => s.dim(),
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
    path: String,
    description: Vec<Segment>,
}

/// Human-readable text renderer. Buffers all events so they can be emitted
/// in a column-aligned block by `finalize`. Honors `color` for ANSI escapes,
/// `show_diffs` for whether `Status::Generated` variants emit a unified diff
/// after the summary block, and `show_clean` for whether clean-state events
/// (e.g. `FileStatus::Ok`, `PkgStatus::Ok`, `GitRepoClean`) get a visible
/// line or are dropped.
pub struct TerminalRenderer<'w> {
    out: &'w mut dyn Write,
    styler: Styler,
    show_diffs: bool,
    show_clean: bool,
    lines: Vec<Line>,
    diffs: Vec<PendingDiff>,
    finalized: bool,
}

#[derive(Debug)]
struct PendingDiff {
    path: ResolvedConfigFilePath,
    cur_content: Option<String>,
    want_content: Arc<str>,
}

impl<'w> TerminalRenderer<'w> {
    pub fn new(out: &'w mut dyn Write, color: bool, show_diffs: bool, show_clean: bool) -> Self {
        Self {
            out,
            styler: Styler::new(color),
            show_diffs,
            show_clean,
            lines: Vec::new(),
            diffs: Vec::new(),
            finalized: false,
        }
    }

    fn write_line(&mut self, line: &Line, path_width: usize) -> Result<(), OutputError> {
        let s = &self.styler;
        let reset = s.reset();
        let marker_open = line.marker_style.open(s);
        let dim = s.dim();
        let pad = path_width.saturating_sub(line.path.chars().count());
        write!(
            self.out,
            "{marker_open}{marker}{reset}  {dim}{path}{reset}{spaces}",
            marker = line.marker,
            path = line.path,
            spaces = " ".repeat(pad),
        )?;
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

fn ok_line(path: String, description: &'static str) -> Line {
    Line {
        marker: '✓',
        marker_style: Style::Green,
        path,
        description: vec![Segment::new(Style::Dim, description)],
    }
}

fn status_to_line(status: &Status, show_clean: bool) -> Option<Line> {
    match status {
        Status::Generated {
            path,
            status: FileStatus::Ok,
            ..
        } => show_clean.then(|| ok_line(path.to_string(), "ok")),
        Status::Generated {
            path,
            status: FileStatus::Modified,
            ..
        } => Some(Line {
            marker: '~',
            marker_style: Style::Yellow,
            path: path.to_string(),
            description: vec![Segment::new(Style::Yellow, "modified")],
        }),
        Status::Generated {
            path,
            status: FileStatus::New,
            ..
        } => Some(Line {
            marker: '+',
            marker_style: Style::Green,
            path: path.to_string(),
            description: vec![Segment::new(Style::Green, "missing")],
        }),
        Status::Symlink {
            real,
            symlink,
            status: SymlinkStatus::Ok,
        } => show_clean.then(|| ok_line(format!("{symlink} → {real}"), "ok")),
        Status::Symlink {
            real,
            symlink,
            status: SymlinkStatus::WrongLink(actual),
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: format!("{symlink} → {real}"),
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
            marker_style: Style::Green,
            path: format!("{symlink} → {real}"),
            description: vec![Segment::new(Style::Green, "missing")],
        }),
        Status::Symlink {
            symlink,
            status: SymlinkStatus::IsFile,
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: symlink.to_string(),
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
            path: symlink.to_string(),
            description: vec![
                Segment::new(Style::Red, "is a dir"),
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
            path: real.to_string(),
            description: vec![Segment::new(Style::Red, "symlink source missing")],
        }),
        Status::Symlink {
            symlink,
            status: SymlinkStatus::DstDirIsMissing,
            ..
        } => Some(Line {
            marker: '✗',
            marker_style: Style::Red,
            path: symlink.to_string(),
            description: vec![Segment::new(Style::Red, "parent directory missing")],
        }),
        Status::Git { repo, status } => match status {
            GitFileStatus::Modified(p) => Some(Line {
                marker: 'M',
                marker_style: Style::Yellow,
                path: format!("{repo}/{p}"),
                description: vec![Segment::new(Style::Yellow, "modified")],
            }),
            GitFileStatus::Added(p) => Some(Line {
                marker: 'A',
                marker_style: Style::Green,
                path: format!("{repo}/{p}"),
                description: vec![Segment::new(Style::Green, "added")],
            }),
            GitFileStatus::Deleted(p) => Some(Line {
                marker: 'D',
                marker_style: Style::Red,
                path: format!("{repo}/{p}"),
                description: vec![Segment::new(Style::Red, "deleted")],
            }),
            GitFileStatus::Untracked(p) => Some(Line {
                marker: '?',
                marker_style: Style::Cyan,
                path: format!("{repo}/{p}"),
                description: vec![Segment::new(Style::Cyan, "untracked")],
            }),
            GitFileStatus::Other { code, path } => Some(Line {
                marker: '!',
                marker_style: Style::Magenta,
                path: format!("{repo}/{path}"),
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
            path: pkg.to_string(),
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
            path: pkg.to_string(),
            description: vec![Segment::new(Style::Red, "missing")],
        }),
        Status::Pkg {
            pkg,
            status: PkgStatus::Ok,
        } => show_clean.then(|| ok_line(pkg.to_string(), "ok")),
        Status::GitRepoClean { repo } => show_clean.then(|| ok_line(repo.to_string(), "clean")),
    }
}

fn action_to_line(action: &AppliedAction) -> Line {
    match action {
        AppliedAction::UpdatedFile(path) => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: path.to_string(),
            description: vec![Segment::new(Style::Green, "updated")],
        },
        AppliedAction::CreatedFile(path) => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: path.to_string(),
            description: vec![Segment::new(Style::Green, "created")],
        },
        AppliedAction::CreatedSymlink { real, symlink } => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: format!("{symlink} → {real}"),
            description: vec![Segment::new(Style::Green, "linked")],
        },
        AppliedAction::CreatedDir(path) => Line {
            marker: '✓',
            marker_style: Style::Green,
            path: path.to_string(),
            description: vec![Segment::new(Style::Green, "mkdir")],
        },
    }
}

impl Output for TerminalRenderer<'_> {
    fn push_status(&mut self, status: Status) -> Result<(), OutputError> {
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

    fn push_applied_action(&mut self, action: AppliedAction) -> Result<(), OutputError> {
        self.lines.push(action_to_line(&action));
        Ok(())
    }

    fn finalize(&mut self) -> Result<(), OutputError> {
        if self.finalized {
            return Ok(());
        }
        self.finalized = true;
        if self.lines.is_empty() && self.diffs.is_empty() {
            return Ok(());
        }
        let path_width = self
            .lines
            .iter()
            .map(|l| l.path.chars().count())
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
            r.push_status(status).unwrap();
            r.finalize().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    fn render_action(action: AppliedAction) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = TerminalRenderer::new(&mut buf, false, false, false);
            r.push_applied_action(action).unwrap();
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
                symlink("s", "d", SymlinkStatus::DstDirIsMissing),
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
            r.push_status(pkg_missing("py", None)).unwrap();
            r.push_status(generated(
                Some("a\n"),
                "b\n",
                "long/nested/path/file.toml",
                FileStatus::Modified,
            ))
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
            r.push_status(pkg_missing("x", None)).unwrap();
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

    // ---- JsonOutput -------------------------------------------------------

    fn json_line_for_status(status: Status) -> serde_json::Value {
        let mut buf: Vec<u8> = Vec::new();
        JsonOutput::new(&mut buf).push_status(status).unwrap();
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
            .push_applied_action(action)
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
            out.push_status(pkg_missing("python", None)).unwrap();
            out.push_applied_action(AppliedAction::CreatedDir(home_path("d")))
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
        r.push_status(pkg_missing("x", None)).unwrap();
        let err = r.finalize().unwrap_err();
        assert!(matches!(err, OutputError::Io(_)), "unexpected: {err:?}");
    }

    #[test]
    fn json_output_surfaces_writer_errors() {
        let mut w = FailingWriter;
        let err = JsonOutput::new(&mut w)
            .push_status(pkg_missing("x", None))
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
