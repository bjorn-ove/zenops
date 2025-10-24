use std::path::PathBuf;

use smol_str::SmolStr;

use crate::output::ResolvedConfigFilePath;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to open the database file {0:?}: {1}")]
    OpenDb(PathBuf, #[source] std::io::Error),
    #[error("Failed to parse the database from file {0:?}: {1}")]
    ParseDb(PathBuf, #[source] toml::de::Error),
    #[error("Failed to execute command")]
    Shell(xshell::Error),
    #[error("Failed to write config file {0}: {1}")]
    FailedToWriteConfig(ResolvedConfigFilePath, std::io::Error),
    #[error(transparent)]
    SafeRelativePath(safe_relative_path::error::Error),
    #[error("Not creating symlink {symlink} -> {real}: a file already exists")]
    RefusingToOverwriteFileWithSymlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
    },
    #[error("Invalid package spec {0:?}: {1}")]
    InvalidPackageSpec(String, &'static str),
    #[error("Unknown package kind {0:?} for {1:?}")]
    UnknownPackageKind(String, SmolStr),
    #[error("Failed to create directory {0:?}: {1}")]
    CreateDirectoryError(ResolvedConfigFilePath, std::io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::OpenDb(l0, l1), Self::OpenDb(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            (Self::ParseDb(l0, l1), Self::ParseDb(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::Shell(l0), Self::Shell(r0)) => l0.to_string() == r0.to_string(),
            (Self::FailedToWriteConfig(l0, l1), Self::FailedToWriteConfig(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (Self::SafeRelativePath(l0), Self::SafeRelativePath(r0)) => l0 == r0,
            (
                Self::RefusingToOverwriteFileWithSymlink {
                    real: l_real,
                    symlink: l_symlink,
                },
                Self::RefusingToOverwriteFileWithSymlink {
                    real: r_real,
                    symlink: r_symlink,
                },
            ) => l_real == r_real && l_symlink == r_symlink,
            (Self::InvalidPackageSpec(l0, l1), Self::InvalidPackageSpec(r0, r1)) => {
                l0 == r0 && l1 == r1
            }
            (Self::UnknownPackageKind(l0, l1), Self::UnknownPackageKind(r0, r1)) => {
                l0 == r0 && l1 == r1
            }
            (Self::CreateDirectoryError(l0, l1), Self::CreateDirectoryError(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            _ => false,
        }
    }
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
