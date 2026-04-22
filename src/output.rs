use similar::{ChangeTag, TextDiff};
use smol_str::SmolStr;
use std::{
    fmt,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    config_files::{ConfigFileDirs, ConfigFilePath},
    git::GitFileStatus,
};

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
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

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
pub enum FileStatus {
    Ok,
    Modified,
    New,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, PartialEq, Clone)]
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

#[derive(Debug, PartialEq)]
pub enum AppliedAction {
    UpdatedFile(ResolvedConfigFilePath),
    CreatedFile(ResolvedConfigFilePath),
    CreatedSymlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
    },
    CreatedDir(ResolvedConfigFilePath),
}

pub trait Output {
    fn push_status(&mut self, status: Status);
    fn push_applied_action(&mut self, action: AppliedAction);
}

pub struct Log;

impl Output for Log {
    fn push_status(&mut self, status: Status) {
        match status {
            Status::Generated {
                want_content: _,
                cur_content: _,
                path,
                status,
            } => match status {
                FileStatus::Ok => log::debug!("GEN: {path} is unchanged"),
                FileStatus::Modified => log::info!("GEN: {path} is modified"),
                FileStatus::New => log::info!("GEN: {path} is missing"),
            },
            Status::Symlink {
                real,
                symlink,
                status,
            } => match status {
                SymlinkStatus::Ok => log::debug!("SYM: {symlink} is unchanged"),
                SymlinkStatus::WrongLink(path) => {
                    log::info!("SYM: {symlink} does not point to {real}, but instead {path:?}")
                }
                SymlinkStatus::New => log::info!("SYM: {symlink} is missing"),
                SymlinkStatus::IsFile => log::warn!("SYM: {symlink} is a file"),
                SymlinkStatus::IsDir => log::warn!("SYM: {symlink} is a directory"),
                SymlinkStatus::RealPathIsMissing => {
                    log::info!("SYM: symlink source {real} is missing")
                }
                SymlinkStatus::DstDirIsMissing => log::info!("SYM: {symlink} directory is missing"),
            },
            Status::Git { repo, status } => match status {
                GitFileStatus::Modified(path) => log::info!("GIT: {repo}/{path} is modified"),
                GitFileStatus::Added(path) => log::info!("GIT: {repo}/{path} is added"),
                GitFileStatus::Deleted(path) => log::info!("GIT: {repo}/{path} is deleted"),
                GitFileStatus::Untracked(path) => log::info!("GIT: {repo}/{path} is untracked"),
                GitFileStatus::Other { code, path } => {
                    log::info!("GIT: {repo}/{path} has status {code}")
                }
            },
            Status::PkgMissing {
                pkg,
                install_command: Some(cmd),
            } => log::warn!("{pkg} is missing — install with: {cmd}"),
            Status::PkgMissing {
                pkg,
                install_command: None,
            } => log::warn!("{pkg} is missing"),
        }
    }

    fn push_applied_action(&mut self, action: AppliedAction) {
        match action {
            AppliedAction::UpdatedFile(path) => log::info!("GEN: {path} was updated"),
            AppliedAction::CreatedFile(path) => log::info!("GEN: {path} was created"),
            AppliedAction::CreatedSymlink { real, symlink } => {
                log::info!("SYM: created {symlink} <- {real}")
            }
            AppliedAction::CreatedDir(path) => log::info!("DIR: {path} was created"),
        }
    }
}

pub struct DiffLog;

impl Output for DiffLog {
    fn push_status(&mut self, status: Status) {
        match status {
            Status::Generated {
                want_content,
                cur_content,
                path,
                status,
            } => {
                match status {
                    FileStatus::Ok => {
                        log::debug!("GEN: {path} is unchanged");
                        return;
                    }
                    FileStatus::Modified => log::info!("GEN: {path} is modified"),
                    FileStatus::New => log::info!("GEN: {path} is missing"),
                }
                let stderr = io::stderr();
                let mut out = stderr.lock();
                render_generated_diff(&mut out, &path, cur_content.as_deref(), &want_content)
                    .expect("writing a diff to stderr");
            }
            other => Log.push_status(other),
        }
    }

    fn push_applied_action(&mut self, action: AppliedAction) {
        Log.push_applied_action(action);
    }
}

fn render_generated_diff(
    out: &mut dyn Write,
    path: &ResolvedConfigFilePath,
    cur_content: Option<&str>,
    want_content: &str,
) -> io::Result<()> {
    let old = cur_content.unwrap_or("");
    if cur_content.is_some() {
        writeln!(out, "--- {path} (current)")?;
    } else {
        writeln!(out, "--- /dev/null")?;
    }
    writeln!(out, "+++ {path} (generated)")?;
    let diff = TextDiff::from_lines(old, want_content);
    for change in diff.iter_all_changes() {
        let (prefix, color, reset) = match change.tag() {
            ChangeTag::Delete => ("-", "\x1b[31m", "\x1b[0m"),
            ChangeTag::Insert => ("+", "\x1b[32m", "\x1b[0m"),
            ChangeTag::Equal => (" ", "\x1b[2m", "\x1b[0m"),
        };
        write!(out, "{color}{prefix}{change}{reset}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use similar_asserts::assert_eq;
    use zenops_safe_relative_path::SafeRelativePath;

    use super::*;
    use crate::config_files::ConfigFilePath;

    fn home_path(rel: &str) -> ResolvedConfigFilePath {
        let srp = SafeRelativePath::from_relative_path(rel).unwrap();
        ResolvedConfigFilePath {
            path: ConfigFilePath::in_home(srp),
            full: Arc::from(Path::new("/home/test").join(rel)),
        }
    }

    fn render(cur: Option<&str>, want: &str, rel: &str) -> String {
        let p = home_path(rel);
        let mut buf: Vec<u8> = Vec::new();
        render_generated_diff(&mut buf, &p, cur, want).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn render_generated_diff_labels_new_file_from_dev_null() {
        let got = render(None, "x\n", "alpha.toml");
        assert!(
            got.starts_with("--- /dev/null\n+++ ~/alpha.toml (generated)\n"),
            "header wrong: {got:?}",
        );
        assert!(
            got.contains("\x1b[32m+x\n\x1b[0m"),
            "green insert missing: {got:?}",
        );
    }

    #[test]
    fn render_generated_diff_labels_modified_file_with_current_path() {
        let got = render(Some("a\n"), "b\n", "alpha.toml");
        assert!(
            got.starts_with("--- ~/alpha.toml (current)\n+++ ~/alpha.toml (generated)\n"),
            "header wrong: {got:?}",
        );
        assert!(
            got.contains("\x1b[31m-a\n\x1b[0m"),
            "red delete missing: {got:?}",
        );
        assert!(
            got.contains("\x1b[32m+b\n\x1b[0m"),
            "green insert missing: {got:?}",
        );
    }

    #[test]
    fn render_generated_diff_wraps_equal_lines_in_dim_color() {
        let got = render(Some("keep\nold\n"), "keep\nnew\n", "c.toml");
        assert!(
            got.contains("\x1b[2m keep\n\x1b[0m"),
            "dim-wrapped context line missing: {got:?}",
        );
        assert!(got.contains("\x1b[31m-old\n\x1b[0m"), "{got:?}");
        assert!(got.contains("\x1b[32m+new\n\x1b[0m"), "{got:?}");
    }

    #[test]
    fn render_generated_diff_empty_current_still_labels_as_current() {
        // `cur_content: Some("")` means the file exists but is empty — the
        // header must still say "(current)", not "/dev/null".
        let got = render(Some(""), "hi\n", "empty.toml");
        assert!(
            got.starts_with("--- ~/empty.toml (current)\n"),
            "header wrong: {got:?}",
        );
    }

    #[test]
    fn render_generated_diff_no_changes_emits_only_headers() {
        let got = render(Some("same\n"), "same\n", "same.toml");
        // Two header lines plus a single unchanged body line.
        assert_eq!(
            got,
            "--- ~/same.toml (current)\n+++ ~/same.toml (generated)\n\x1b[2m same\n\x1b[0m",
        );
    }
}
