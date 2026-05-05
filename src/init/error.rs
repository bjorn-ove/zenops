//! `init`-scoped error type.
//!
//! Wraps every failure mode the `zenops init` flows can produce, both
//! clone (`zenops init <url>`) and bootstrap (`zenops init`). Exposed
//! to the rest of the crate as `crate::Error::Init` via
//! `#[error(transparent)]` + `#[from]`, so call-sites compose with `?`.
//!
//! `GitInitFailed` and `CloneFailed` already wrap `xshell::Error`
//! typedly, so the enum deliberately does **not** add a generic
//! `#[from] xshell::Error`: that would let xshell errors take a different
//! path than the typed clone/init-repo paths.

use std::path::PathBuf;

/// Failure modes for `zenops init` (clone form and bootstrap form).
/// Wrapped into [`crate::Error`] as `Error::Init` via `#[from]`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `zenops init` target directory exists and is non-empty. The user
    /// must clear it (or use `zenops repo pull` if it's already a clone).
    #[error(
        "Cannot init: {0:?} already exists and is not empty. Remove it first, or use `zenops repo pull` if it's already a zenops repo."
    )]
    DirNotEmpty(PathBuf),
    /// `zenops init` (bootstrap form, no URL) found the target directory
    /// already exists. Bootstrap refuses to touch any existing path so it
    /// can never clobber a previous setup.
    #[error(
        "Cannot init: {0:?} already exists. Bootstrap refuses to touch an existing path; remove it first, or pass a URL to clone into it."
    )]
    DirExists(PathBuf),
    /// `zenops init` bootstrap target already contains a `.git` directory.
    /// Reported instead of [`Self::DirExists`] when the existing
    /// directory looks like a git repo.
    #[error(
        "Cannot init: {0:?} already contains a .git directory. Looks like a zenops repo already exists; remove it first or skip init."
    )]
    GitDirExists(PathBuf),
    /// `git init` itself failed during `zenops init` bootstrap.
    #[error("Failed to initialize git repo: {source}")]
    GitInitFailed {
        /// Underlying xshell failure.
        #[source]
        source: xshell::Error,
    },
    /// `zenops init` (bootstrap form, no URL) was invoked without a TTY.
    /// Prompts can't run reliably without one, so we refuse rather than
    /// silently accept defaults from a closed stdin.
    #[error(
        "init bootstrap requires a terminal for prompts; clone with a URL instead, or run from a TTY"
    )]
    NeedsTty,
    /// `zenops init` cloned successfully but the repo lacks a `config.toml`
    /// at its root, so it isn't a zenops config repo. The clone is left in
    /// place for inspection.
    #[error(
        "Cloned repo at {0:?} has no config.toml at its root. Is this a zenops config repo? The clone was left in place so you can inspect it."
    )]
    NoConfigToml(PathBuf),
    /// `git clone` itself failed during `zenops init` (auth, network,
    /// invalid URL).
    #[error("Failed to clone {url}: {source}")]
    CloneFailed {
        /// The URL passed to `git clone`.
        url: String,
        /// The underlying xshell failure.
        #[source]
        source: xshell::Error,
    },
    /// I/O error during `zenops init` pre-flight (e.g. probing the target
    /// directory, removing it before clone).
    #[error("Init I/O error at {0:?}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::DirNotEmpty(l), Self::DirNotEmpty(r)) => l == r,
            (Self::DirExists(l), Self::DirExists(r)) => l == r,
            (Self::GitDirExists(l), Self::GitDirExists(r)) => l == r,
            (Self::GitInitFailed { source: l }, Self::GitInitFailed { source: r }) => {
                l.to_string() == r.to_string()
            }
            (Self::NeedsTty, Self::NeedsTty) => true,
            (Self::NoConfigToml(l), Self::NoConfigToml(r)) => l == r,
            (
                Self::CloneFailed {
                    url: l_url,
                    source: l_src,
                },
                Self::CloneFailed {
                    url: r_url,
                    source: r_src,
                },
            ) => l_url == r_url && l_src.to_string() == r_src.to_string(),
            (Self::Io(l0, l1), Self::Io(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;

    use similar_asserts::assert_eq;
    use xshell::{Shell, cmd};

    use super::*;

    fn io(kind: io::ErrorKind) -> io::Error {
        io::Error::from(kind)
    }

    fn xshell_err() -> xshell::Error {
        let sh = Shell::new().unwrap();
        cmd!(sh, "false").quiet().run().unwrap_err()
    }

    #[test]
    fn unit_variants_compare_equal_to_themselves() {
        assert_eq!(Error::NeedsTty, Error::NeedsTty);
    }

    #[test]
    fn dir_variants_eq_and_ne() {
        assert_eq!(
            Error::DirNotEmpty(PathBuf::from("/a")),
            Error::DirNotEmpty(PathBuf::from("/a"))
        );
        assert_ne!(
            Error::DirNotEmpty(PathBuf::from("/a")),
            Error::DirNotEmpty(PathBuf::from("/b"))
        );
        assert_eq!(
            Error::DirExists(PathBuf::from("/a")),
            Error::DirExists(PathBuf::from("/a"))
        );
        assert_eq!(
            Error::GitDirExists(PathBuf::from("/a")),
            Error::GitDirExists(PathBuf::from("/a"))
        );
        assert_eq!(
            Error::NoConfigToml(PathBuf::from("/a")),
            Error::NoConfigToml(PathBuf::from("/a"))
        );
    }

    #[test]
    fn init_git_init_failed_eq_compares_display_string() {
        let a = Error::GitInitFailed {
            source: xshell_err(),
        };
        let b = Error::GitInitFailed {
            source: xshell_err(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn init_clone_failed_eq_compares_url_and_display() {
        let a = Error::CloneFailed {
            url: "https://example/x".to_string(),
            source: xshell_err(),
        };
        let b = Error::CloneFailed {
            url: "https://example/x".to_string(),
            source: xshell_err(),
        };
        let c = Error::CloneFailed {
            url: "https://example/y".to_string(),
            source: xshell_err(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn init_io_eq_and_ne() {
        let a = Error::Io(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let b = Error::Io(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let c = Error::Io(PathBuf::from("/x"), io(io::ErrorKind::NotFound));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
