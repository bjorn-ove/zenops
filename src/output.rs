use similar::{ChangeTag, TextDiff};
use std::{
    fmt,
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
                SymlinkStatus::RealPathIsMissing => {
                    log::info!("SYM: symlink source {real} is missing")
                }
                SymlinkStatus::DstDirIsMissing => log::info!("SYM: {symlink} directory is missing"),
            },
            Status::Git { repo, status } => match status {
                GitFileStatus::Modified(path) => log::info!("GIT: {repo}/{path} is modified"),
                GitFileStatus::Untracked(path) => log::info!("GIT: {repo}/{path} is untracked"),
            },
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
                let old = cur_content.as_deref().unwrap_or("");
                let old_label = if cur_content.is_some() {
                    format!("--- {path} (current)")
                } else {
                    "--- /dev/null".to_string()
                };
                eprintln!("{old_label}");
                eprintln!("+++ {path} (generated)");
                let diff = TextDiff::from_lines(old, want_content.as_ref());
                for change in diff.iter_all_changes() {
                    let (prefix, color, reset) = match change.tag() {
                        ChangeTag::Delete => ("-", "\x1b[31m", "\x1b[0m"),
                        ChangeTag::Insert => ("+", "\x1b[32m", "\x1b[0m"),
                        ChangeTag::Equal => (" ", "\x1b[2m", "\x1b[0m"),
                    };
                    eprint!("{color}{prefix}{change}{reset}");
                }
            }
            other => Log.push_status(other),
        }
    }

    fn push_applied_action(&mut self, action: AppliedAction) {
        Log.push_applied_action(action);
    }
}
