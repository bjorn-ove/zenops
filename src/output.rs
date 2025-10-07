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
    New,
    IsFile,
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

pub trait Output {
    fn push_status(&mut self, status: Status);
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
            },
            Status::Git { repo, status } => match status {
                GitFileStatus::Modified(path) => log::info!("GIT: {repo}/{path} is modified"),
                GitFileStatus::Untracked(path) => log::info!("GIT: {repo}/{path} is untracked"),
            },
        }
    }
}
