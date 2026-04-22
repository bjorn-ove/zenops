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
    ansi::{color_code, color_reset},
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
    /// A pkg the user expects to be present (`enable = "on"`) whose detect
    /// strategies don't match on the current host. `install_command` is the
    /// ready-to-run shell line (`"brew install python"`) when a package
    /// manager with a non-empty hint is detected, `None` otherwise.
    PkgMissing {
        pkg: SmolStr,
        install_command: Option<String>,
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

/// Human-readable text renderer. Writes every event directly to `out`,
/// honoring `color` for ANSI escapes and `show_diffs` for whether
/// `Status::Generated` variants emit a unified diff alongside their summary
/// line.
pub struct TerminalRenderer<'w> {
    out: &'w mut dyn Write,
    color: bool,
    show_diffs: bool,
}

impl<'w> TerminalRenderer<'w> {
    pub fn new(out: &'w mut dyn Write, color: bool, show_diffs: bool) -> Self {
        Self {
            out,
            color,
            show_diffs,
        }
    }

    fn render_generated_diff(
        &mut self,
        path: &ResolvedConfigFilePath,
        cur_content: Option<&str>,
        want_content: &str,
    ) -> Result<(), OutputError> {
        let old = cur_content.unwrap_or("");
        if cur_content.is_some() {
            writeln!(self.out, "--- {path} (current)")?;
        } else {
            writeln!(self.out, "--- /dev/null")?;
        }
        writeln!(self.out, "+++ {path} (generated)")?;
        let diff = TextDiff::from_lines(old, want_content);
        let reset = color_reset(self.color);
        for change in diff.iter_all_changes() {
            let (prefix, code) = match change.tag() {
                ChangeTag::Delete => ("-", "\x1b[31m"),
                ChangeTag::Insert => ("+", "\x1b[32m"),
                ChangeTag::Equal => (" ", "\x1b[2m"),
            };
            let open = color_code(self.color, code);
            write!(self.out, "{open}{prefix}{change}{reset}")?;
        }
        Ok(())
    }
}

impl Output for TerminalRenderer<'_> {
    fn push_status(&mut self, status: Status) -> Result<(), OutputError> {
        match status {
            Status::Generated {
                status: FileStatus::Ok,
                ..
            } => {}
            Status::Generated {
                want_content,
                cur_content,
                path,
                status: FileStatus::Modified,
            } => {
                writeln!(self.out, "GEN: {path} is modified")?;
                if self.show_diffs {
                    self.render_generated_diff(&path, cur_content.as_deref(), &want_content)?;
                }
            }
            Status::Generated {
                want_content,
                cur_content,
                path,
                status: FileStatus::New,
            } => {
                writeln!(self.out, "GEN: {path} is missing")?;
                if self.show_diffs {
                    self.render_generated_diff(&path, cur_content.as_deref(), &want_content)?;
                }
            }
            Status::Symlink {
                status: SymlinkStatus::Ok,
                ..
            } => {}
            Status::Symlink {
                real,
                symlink,
                status: SymlinkStatus::WrongLink(path),
            } => writeln!(
                self.out,
                "SYM: {symlink} does not point to {real}, but instead {path:?}",
            )?,
            Status::Symlink {
                symlink,
                status: SymlinkStatus::New,
                ..
            } => writeln!(self.out, "SYM: {symlink} is missing")?,
            Status::Symlink {
                symlink,
                status: SymlinkStatus::IsFile,
                ..
            } => writeln!(self.out, "SYM: {symlink} is a file")?,
            Status::Symlink {
                symlink,
                status: SymlinkStatus::IsDir,
                ..
            } => writeln!(self.out, "SYM: {symlink} is a directory")?,
            Status::Symlink {
                real,
                status: SymlinkStatus::RealPathIsMissing,
                ..
            } => writeln!(self.out, "SYM: symlink source {real} is missing")?,
            Status::Symlink {
                symlink,
                status: SymlinkStatus::DstDirIsMissing,
                ..
            } => writeln!(self.out, "SYM: {symlink} directory is missing")?,
            Status::Git { repo, status } => match status {
                GitFileStatus::Modified(path) => {
                    writeln!(self.out, "GIT: {repo}/{path} is modified")?
                }
                GitFileStatus::Added(path) => writeln!(self.out, "GIT: {repo}/{path} is added")?,
                GitFileStatus::Deleted(path) => {
                    writeln!(self.out, "GIT: {repo}/{path} is deleted")?
                }
                GitFileStatus::Untracked(path) => {
                    writeln!(self.out, "GIT: {repo}/{path} is untracked")?
                }
                GitFileStatus::Other { code, path } => {
                    writeln!(self.out, "GIT: {repo}/{path} has status {code}")?
                }
            },
            Status::PkgMissing {
                pkg,
                install_command: Some(cmd),
            } => writeln!(self.out, "{pkg} is missing — install with: {cmd}")?,
            Status::PkgMissing {
                pkg,
                install_command: None,
            } => writeln!(self.out, "{pkg} is missing")?,
        }
        Ok(())
    }

    fn push_applied_action(&mut self, action: AppliedAction) -> Result<(), OutputError> {
        match action {
            AppliedAction::UpdatedFile(path) => writeln!(self.out, "GEN: {path} was updated")?,
            AppliedAction::CreatedFile(path) => writeln!(self.out, "GEN: {path} was created")?,
            AppliedAction::CreatedSymlink { real, symlink } => {
                writeln!(self.out, "SYM: created {symlink} <- {real}")?
            }
            AppliedAction::CreatedDir(path) => writeln!(self.out, "DIR: {path} was created")?,
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
        let mut buf: Vec<u8> = Vec::new();
        TerminalRenderer::new(&mut buf, color, show_diffs)
            .push_status(status)
            .unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn render_action(action: AppliedAction) -> String {
        let mut buf: Vec<u8> = Vec::new();
        TerminalRenderer::new(&mut buf, false, false)
            .push_applied_action(action)
            .unwrap();
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
    fn generated_modified_summary_without_diff_is_single_line() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        assert_eq!(
            render_status(s, false, false),
            "GEN: ~/a.toml is modified\n"
        );
    }

    #[test]
    fn generated_new_summary_without_diff_is_single_line() {
        let s = generated(None, "x\n", "a.toml", FileStatus::New);
        assert_eq!(render_status(s, false, false), "GEN: ~/a.toml is missing\n");
    }

    #[test]
    fn generated_modified_with_diff_labels_current_and_generated() {
        let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
        let got = render_status(s, false, true);
        assert!(got.starts_with("GEN: ~/a.toml is modified\n"), "{got:?}");
        assert!(
            got.contains("--- ~/a.toml (current)\n+++ ~/a.toml (generated)\n"),
            "{got:?}"
        );
        assert!(got.contains("-a\n"), "{got:?}");
        assert!(got.contains("+b\n"), "{got:?}");
    }

    #[test]
    fn generated_new_with_diff_labels_dev_null() {
        let s = generated(None, "x\n", "a.toml", FileStatus::New);
        let got = render_status(s, false, true);
        assert!(got.starts_with("GEN: ~/a.toml is missing\n"), "{got:?}");
        assert!(
            got.contains("--- /dev/null\n+++ ~/a.toml (generated)\n"),
            "{got:?}"
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
    fn symlink_wrong_link_reports_actual_target() {
        let s = symlink(
            "src",
            "dst",
            SymlinkStatus::WrongLink(PathBuf::from("/other")),
        );
        assert_eq!(
            render_status(s, false, false),
            "SYM: ~/dst does not point to ~/src, but instead \"/other\"\n",
        );
    }

    #[test]
    fn symlink_variants_render_expected_lines() {
        assert_eq!(
            render_status(symlink("s", "d", SymlinkStatus::New), false, false),
            "SYM: ~/d is missing\n",
        );
        assert_eq!(
            render_status(symlink("s", "d", SymlinkStatus::IsFile), false, false),
            "SYM: ~/d is a file\n",
        );
        assert_eq!(
            render_status(symlink("s", "d", SymlinkStatus::IsDir), false, false),
            "SYM: ~/d is a directory\n",
        );
        assert_eq!(
            render_status(
                symlink("s", "d", SymlinkStatus::RealPathIsMissing),
                false,
                false
            ),
            "SYM: symlink source ~/s is missing\n",
        );
        assert_eq!(
            render_status(
                symlink("s", "d", SymlinkStatus::DstDirIsMissing),
                false,
                false
            ),
            "SYM: ~/d directory is missing\n",
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
                "GIT: ~/.config/zenops/a.toml is modified\n",
            ),
            (
                GitFileStatus::Added(relpath("b.toml")),
                "GIT: ~/.config/zenops/b.toml is added\n",
            ),
            (
                GitFileStatus::Deleted(relpath("c.toml")),
                "GIT: ~/.config/zenops/c.toml is deleted\n",
            ),
            (
                GitFileStatus::Untracked(relpath("d.toml")),
                "GIT: ~/.config/zenops/d.toml is untracked\n",
            ),
            (
                GitFileStatus::Other {
                    code: SmolStr::new_static("UU"),
                    path: relpath("e.toml"),
                },
                "GIT: ~/.config/zenops/e.toml has status UU\n",
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

    #[test]
    fn pkg_missing_with_install_command_includes_hint() {
        let s = Status::PkgMissing {
            pkg: SmolStr::new_static("python"),
            install_command: Some("brew install python".to_string()),
        };
        assert_eq!(
            render_status(s, false, false),
            "python is missing — install with: brew install python\n",
        );
    }

    #[test]
    fn pkg_missing_without_install_command_is_terse() {
        let s = Status::PkgMissing {
            pkg: SmolStr::new_static("python"),
            install_command: None,
        };
        assert_eq!(render_status(s, false, false), "python is missing\n");
    }

    #[test]
    fn applied_actions_render_expected_lines() {
        assert_eq!(
            render_action(AppliedAction::UpdatedFile(home_path("a.toml"))),
            "GEN: ~/a.toml was updated\n",
        );
        assert_eq!(
            render_action(AppliedAction::CreatedFile(home_path("a.toml"))),
            "GEN: ~/a.toml was created\n",
        );
        assert_eq!(
            render_action(AppliedAction::CreatedSymlink {
                real: home_path("src"),
                symlink: home_path("dst"),
            }),
            "SYM: created ~/dst <- ~/src\n",
        );
        assert_eq!(
            render_action(AppliedAction::CreatedDir(home_path("subdir"))),
            "DIR: ~/subdir was created\n",
        );
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
        let v = json_line_for_status(Status::PkgMissing {
            pkg: SmolStr::new_static("python"),
            install_command: Some("brew install python".to_string()),
        });
        assert_eq!(v["event"], "status");
        assert_eq!(v["kind"], "pkg_missing");
        assert_eq!(v["pkg"], "python");
        assert_eq!(v["install_command"], "brew install python");
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
            out.push_status(Status::PkgMissing {
                pkg: SmolStr::new_static("python"),
                install_command: None,
            })
            .unwrap();
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
    fn terminal_renderer_surfaces_writer_errors() {
        let mut w = FailingWriter;
        let err = TerminalRenderer::new(&mut w, false, false)
            .push_status(Status::PkgMissing {
                pkg: SmolStr::new_static("x"),
                install_command: None,
            })
            .unwrap_err();
        assert!(matches!(err, OutputError::Io(_)), "unexpected: {err:?}");
    }

    #[test]
    fn json_output_surfaces_writer_errors() {
        let mut w = FailingWriter;
        let err = JsonOutput::new(&mut w)
            .push_status(Status::PkgMissing {
                pkg: SmolStr::new_static("x"),
                install_command: None,
            })
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
