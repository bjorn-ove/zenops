use std::path::PathBuf;

use smol_str::SmolStr;

use crate::config_files::ConfigFilePath;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to open the database file {0:?}: {1}")]
    OpenDb(PathBuf, #[source] std::io::Error),
    #[error("Failed to parse the database from file {0:?}: {1}")]
    ParseDb(PathBuf, #[source] toml::de::Error),
    #[error("Failed to execute command")]
    Shell(xshell::Error),
    #[error("Failed to write config file {p}: {1}", p = .0.human_path())]
    FailedToWriteConfig(ConfigFilePath, std::io::Error),
    #[error(transparent)]
    SafeRelativePath(safe_relative_path::error::Error),
    #[error("Not creating symlink {dst} -> {src}: a file already exists", src = .0.human_path(), dst = .1.human_path())]
    RefusingToOverwriteFileWithSymlink(ConfigFilePath, ConfigFilePath),
    #[error("Invalid package spec {0:?}: {1}")]
    InvalidPackageSpec(String, &'static str),
    #[error("Unknown package kind {0:?} for {1:?}")]
    UnknownPackageKind(String, SmolStr),
}

impl From<xshell::Error> for Error {
    fn from(e: xshell::Error) -> Self {
        Self::Shell(e)
    }
}

impl From<safe_relative_path::error::Error> for Error {
    fn from(e: safe_relative_path::error::Error) -> Self {
        Self::SafeRelativePath(e)
    }
}
