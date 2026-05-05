//! `config_files`-scoped error type.
//!
//! Bundles every failure mode the config-file apply pass produces:
//! reads, writes, mkdir, symlink creation, and the refusals that fire
//! when a managed path collides with something we shouldn't clobber.
//! Wrapped into [`crate::Error`] as `Error::ConfigFiles` via
//! `#[from]` + `#[error(transparent)]`.

use std::path::PathBuf;

use crate::output::ResolvedConfigFilePath;

/// Failure modes for the config-file apply pass.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Writing a generated config file to disk failed.
    #[error("Failed to write config file {0}: {1}")]
    FailedToWriteConfig(ResolvedConfigFilePath, std::io::Error),
    /// Reading an existing managed file from disk failed (e.g. permission
    /// denied, or a directory occupies the path where a generated file should
    /// land).
    #[error("Failed to read existing config file {0}: {1}")]
    FailedToReadConfig(ResolvedConfigFilePath, #[source] std::io::Error),
    /// Stat-ing a managed path failed for a reason other than "not found"
    /// (e.g. permission denied on a parent). The wrapped path is whichever
    /// path the probe was attempted on.
    #[error("Failed to probe filesystem state at {0:?}: {1}")]
    SymlinkProbeFailed(PathBuf, #[source] std::io::Error),
    /// `symlink(2)` failed in [`super`]'s `create_symlink` helper.
    /// Bundles both ends so the user sees the symlink they were attempting
    /// and the underlying I/O reason.
    #[error("Failed to create symlink {symlink} -> {real}: {source}")]
    CreateSymlinkFailed {
        /// The intended symlink target (the file in the zenops repo).
        real: ResolvedConfigFilePath,
        /// Where the symlink was being created.
        symlink: ResolvedConfigFilePath,
        /// Underlying `symlink(2)` failure.
        #[source]
        source: std::io::Error,
    },
    /// Apply pass: a managed symlink already points at the intended target,
    /// but that target doesn't exist in the zenops repo. The user must add
    /// the file before zenops can manage it.
    #[error("Symlink {symlink} -> {real}: {real} does not exist in the zenops repo")]
    SymlinkRealPathMissing {
        /// The intended symlink target (the file in the zenops repo).
        real: ResolvedConfigFilePath,
        /// Where the symlink lives.
        symlink: ResolvedConfigFilePath,
    },
    /// Apply pass refused to clobber a path that is neither a regular file,
    /// directory, nor symlink (FIFO, socket, device node, etc.) with a
    /// managed symlink. The user must remove the existing entry first.
    #[error(
        "Not creating symlink at {0}: a non-file, non-directory entry (FIFO, socket, etc.) already exists"
    )]
    RefusingToOverwriteOtherWithSymlink(ResolvedConfigFilePath),
    /// Apply pass refused to clobber an existing regular file with a
    /// symlink. The user must remove the existing file first.
    #[error("Not creating symlink {symlink} -> {real}: a file already exists")]
    RefusingToOverwriteFileWithSymlink {
        /// The intended symlink target (the file in the zenops repo).
        real: ResolvedConfigFilePath,
        /// The path that already exists as a regular file.
        symlink: ResolvedConfigFilePath,
    },
    /// Apply pass refused to clobber an existing directory with a symlink.
    #[error("Not creating symlink {symlink} -> {real}: a directory already exists")]
    RefusingToOverwriteDirectoryWithSymlink {
        /// The intended symlink target.
        real: ResolvedConfigFilePath,
        /// The path that already exists as a directory.
        symlink: ResolvedConfigFilePath,
    },
    /// `mkdir -p` of a parent directory for a managed file failed.
    #[error("Failed to create directory {0:?}: {1}")]
    CreateDirectoryError(ResolvedConfigFilePath, std::io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::FailedToWriteConfig(l0, l1), Self::FailedToWriteConfig(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (Self::FailedToReadConfig(l0, l1), Self::FailedToReadConfig(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (Self::SymlinkProbeFailed(l0, l1), Self::SymlinkProbeFailed(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (
                Self::CreateSymlinkFailed {
                    real: l_real,
                    symlink: l_symlink,
                    source: l_src,
                },
                Self::CreateSymlinkFailed {
                    real: r_real,
                    symlink: r_symlink,
                    source: r_src,
                },
            ) => l_real == r_real && l_symlink == r_symlink && l_src.kind() == r_src.kind(),
            (
                Self::SymlinkRealPathMissing {
                    real: l_real,
                    symlink: l_symlink,
                },
                Self::SymlinkRealPathMissing {
                    real: r_real,
                    symlink: r_symlink,
                },
            ) => l_real == r_real && l_symlink == r_symlink,
            (
                Self::RefusingToOverwriteOtherWithSymlink(l0),
                Self::RefusingToOverwriteOtherWithSymlink(r0),
            ) => l0 == r0,
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
            (
                Self::RefusingToOverwriteDirectoryWithSymlink {
                    real: l_real,
                    symlink: l_symlink,
                },
                Self::RefusingToOverwriteDirectoryWithSymlink {
                    real: r_real,
                    symlink: r_symlink,
                },
            ) => l_real == r_real && l_symlink == r_symlink,
            (Self::CreateDirectoryError(l0, l1), Self::CreateDirectoryError(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use similar_asserts::assert_eq;
    use zenops_safe_relative_path::srpath;

    use crate::config_files::ConfigFilePath;
    use crate::output::ResolvedConfigFilePath;

    use super::*;

    fn rcfp(rel: &'static str) -> ResolvedConfigFilePath {
        let path = ConfigFilePath::Home(Arc::from(
            zenops_safe_relative_path::SafeRelativePath::from_relative_path(rel).unwrap(),
        ));
        let full = Arc::from(Path::new("/tmp").join(rel).as_path());
        ResolvedConfigFilePath { path, full }
    }

    fn io(kind: io::ErrorKind) -> io::Error {
        io::Error::from(kind)
    }

    #[test]
    fn failed_to_write_config_eq_and_ne() {
        let a = Error::FailedToWriteConfig(rcfp("a"), io(io::ErrorKind::PermissionDenied));
        let b = Error::FailedToWriteConfig(rcfp("a"), io(io::ErrorKind::PermissionDenied));
        let c = Error::FailedToWriteConfig(rcfp("b"), io(io::ErrorKind::PermissionDenied));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn failed_to_read_config_eq_and_ne() {
        let a = Error::FailedToReadConfig(rcfp("a"), io(io::ErrorKind::PermissionDenied));
        let b = Error::FailedToReadConfig(rcfp("a"), io(io::ErrorKind::PermissionDenied));
        let c = Error::FailedToReadConfig(rcfp("a"), io(io::ErrorKind::NotFound));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn symlink_probe_failed_eq_and_ne() {
        let a = Error::SymlinkProbeFailed(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let b = Error::SymlinkProbeFailed(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let c = Error::SymlinkProbeFailed(PathBuf::from("/y"), io(io::ErrorKind::Other));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn create_symlink_failed_eq_and_ne() {
        let mk = |real, symlink, kind| Error::CreateSymlinkFailed {
            real: rcfp(real),
            symlink: rcfp(symlink),
            source: io(kind),
        };
        assert_eq!(
            mk("r", "s", io::ErrorKind::AlreadyExists),
            mk("r", "s", io::ErrorKind::AlreadyExists)
        );
        assert_ne!(
            mk("r", "s", io::ErrorKind::AlreadyExists),
            mk("other", "s", io::ErrorKind::AlreadyExists)
        );
        assert_ne!(
            mk("r", "s", io::ErrorKind::AlreadyExists),
            mk("r", "s", io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn symlink_real_path_missing_eq_and_ne() {
        let a = Error::SymlinkRealPathMissing {
            real: rcfp("r"),
            symlink: rcfp("s"),
        };
        let b = Error::SymlinkRealPathMissing {
            real: rcfp("r"),
            symlink: rcfp("s"),
        };
        let c = Error::SymlinkRealPathMissing {
            real: rcfp("r"),
            symlink: rcfp("other"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn refusing_to_overwrite_other_eq_and_ne() {
        let a = Error::RefusingToOverwriteOtherWithSymlink(rcfp("a"));
        let b = Error::RefusingToOverwriteOtherWithSymlink(rcfp("a"));
        let c = Error::RefusingToOverwriteOtherWithSymlink(rcfp("b"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn refusing_to_overwrite_file_eq_and_ne() {
        let a = Error::RefusingToOverwriteFileWithSymlink {
            real: rcfp("r"),
            symlink: rcfp("s"),
        };
        let b = Error::RefusingToOverwriteFileWithSymlink {
            real: rcfp("r"),
            symlink: rcfp("s"),
        };
        let c = Error::RefusingToOverwriteFileWithSymlink {
            real: rcfp("r"),
            symlink: rcfp("t"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn refusing_to_overwrite_directory_eq_and_ne() {
        let a = Error::RefusingToOverwriteDirectoryWithSymlink {
            real: rcfp("r"),
            symlink: rcfp("s"),
        };
        let b = Error::RefusingToOverwriteDirectoryWithSymlink {
            real: rcfp("r"),
            symlink: rcfp("s"),
        };
        let c = Error::RefusingToOverwriteDirectoryWithSymlink {
            real: rcfp("other"),
            symlink: rcfp("s"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn create_directory_error_eq_and_ne() {
        let a = Error::CreateDirectoryError(rcfp("a"), io(io::ErrorKind::PermissionDenied));
        let b = Error::CreateDirectoryError(rcfp("a"), io(io::ErrorKind::PermissionDenied));
        let c = Error::CreateDirectoryError(rcfp("a"), io(io::ErrorKind::NotFound));
        assert_eq!(a, b);
        assert_ne!(a, c);
        let _ = srpath!("dummy");
    }
}
