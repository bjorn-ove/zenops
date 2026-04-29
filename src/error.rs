//! Crate-wide error type.
//!
//! [`Error`] is the unified failure mode for every fallible operation in the
//! `zenops` binary — config load, file writes, git invocations, prompt I/O,
//! init pre-flight, schema emission. Variants use `thiserror` and either own
//! their source (typed `#[source]`) or transparently re-export a foreign
//! error ([`OutputError`], [`zenops_safe_relative_path::error::Error`]).
//!
//! The [`PartialEq`] impl is for tests only: `std::io::Error` and
//! `xshell::Error` aren't naturally comparable, so the impl falls back to
//! comparing [`std::io::ErrorKind`] or [`Display`](std::fmt::Display) output
//! variant by variant.

use std::path::PathBuf;

use smol_str::SmolStr;

use crate::output::{OutputError, ResolvedConfigFilePath};

/// Crate-wide error. Each variant's user-facing string lives on its
/// `#[error(...)]` attribute (the `Display` impl); the doc comment here
/// adds the trigger context the message can't carry.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O error opening `config.toml` (e.g. missing file, permission denied).
    #[error("Failed to open the database file {0:?}: {1}")]
    OpenDb(PathBuf, #[source] std::io::Error),
    /// `config.toml` parsed but failed TOML deserialization or schema validation.
    #[error("Failed to parse the database from file {0:?}: {1}")]
    ParseDb(PathBuf, #[source] toml::de::Error),
    /// A subprocess invoked through `xshell` failed (non-zero exit, signal,
    /// I/O error). The wrapped error carries the command and stderr.
    #[error("Failed to execute command")]
    Shell(#[from] xshell::Error),
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
    /// `symlink(2)` failed in [`crate::config_files`]'s `create_symlink`
    /// helper. Bundles both ends so the user sees the symlink they were
    /// attempting and the underlying I/O reason.
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
    /// A `..`-traversal or other path-safety violation surfaced from
    /// [`zenops_safe_relative_path`].
    #[error(transparent)]
    SafeRelativePath(#[from] zenops_safe_relative_path::error::Error),
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
    /// A pkg's template referenced an input that's not declared anywhere
    /// (no system input, no `[pkg.<name>.inputs]` entry).
    #[error(
        "Package {pkg} references undefined input {input}; mark the action optional or set [pkg.{pkg}.inputs].{input}"
    )]
    UnresolvedInput {
        /// The pkg whose template failed to resolve.
        pkg: SmolStr,
        /// The input name that wasn't found.
        input: SmolStr,
    },
    /// A pkg template contains a `${` with no matching `}`.
    #[error("Package {pkg} has an unterminated `${{` in a template")]
    TemplateUnterminated {
        /// The pkg whose template failed to parse.
        pkg: SmolStr,
    },
    /// `apply` was invoked without a TTY and without `--yes`/`--dry-run`,
    /// so there's no way to confirm prompts.
    #[error(
        "apply requires a terminal for prompts; pass --yes to apply all changes non-interactively, or --dry-run to preview"
    )]
    ApplyNeedsYesOrTty,
    /// `apply --yes` was invoked on a dirty zenops repo without
    /// `--allow-dirty`. The check exists so cron/CI surfaces divergence
    /// instead of silently applying uncommitted state.
    #[error(
        "zenops config repo at {0:?} has uncommitted changes. Commit them first, or re-run with --allow-dirty to apply anyway."
    )]
    DirtyRepoRequiresAllowDirty(PathBuf),
    /// I/O error reading a yes/no answer from stdin.
    #[error("Failed to read confirmation from stdin: {0}")]
    PromptRead(#[source] std::io::Error),
    /// User pressed Ctrl-C at an interactive prompt. Distinct from a
    /// closed stdin or Ctrl-D so callers can abort the whole run.
    #[error("Interrupted")]
    PromptInterrupted,
    /// An [`Output`](crate::output::Output) implementation failed to write
    /// (rendering or JSON serialization error).
    #[error(transparent)]
    Output(#[from] OutputError),
    /// `zenops init` target directory exists and is non-empty. The user
    /// must clear it (or use `zenops repo pull` if it's already a clone).
    #[error(
        "Cannot init: {0:?} already exists and is not empty. Remove it first, or use `zenops repo pull` if it's already a zenops repo."
    )]
    InitDirNotEmpty(PathBuf),
    /// `zenops init` (bootstrap form, no URL) found the target directory
    /// already exists. Bootstrap refuses to touch any existing path so it
    /// can never clobber a previous setup.
    #[error(
        "Cannot init: {0:?} already exists. Bootstrap refuses to touch an existing path; remove it first, or pass a URL to clone into it."
    )]
    InitDirExists(PathBuf),
    /// `zenops init` bootstrap target already contains a `.git` directory.
    /// Reported instead of [`Self::InitDirExists`] when the existing
    /// directory looks like a git repo.
    #[error(
        "Cannot init: {0:?} already contains a .git directory. Looks like a zenops repo already exists; remove it first or skip init."
    )]
    InitGitDirExists(PathBuf),
    /// `git init` itself failed during `zenops init` bootstrap.
    #[error("Failed to initialize git repo: {source}")]
    InitGitInitFailed {
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
    InitNeedsTty,
    /// `zenops init` cloned successfully but the repo lacks a `config.toml`
    /// at its root, so it isn't a zenops config repo. The clone is left in
    /// place for inspection.
    #[error(
        "Cloned repo at {0:?} has no config.toml at its root. Is this a zenops config repo? The clone was left in place so you can inspect it."
    )]
    InitNoConfigToml(PathBuf),
    /// `git clone` itself failed during `zenops init` (auth, network,
    /// invalid URL).
    #[error("Failed to clone {url}: {source}")]
    InitCloneFailed {
        /// The URL passed to `git clone`.
        url: String,
        /// The underlying xshell failure.
        #[source]
        source: xshell::Error,
    },
    /// I/O error during `zenops init` pre-flight (e.g. probing the target
    /// directory, removing it before clone).
    #[error("Init I/O error at {0:?}: {1}")]
    InitIo(PathBuf, #[source] std::io::Error),
    /// `curl` isn't on `PATH` and is needed to fetch a user's GitHub keys.
    #[error(
        "curl is required to fetch GitHub SSH keys; install curl or switch the entry to type = \"manual\""
    )]
    CurlNotFound,
    /// `curl https://api.github.com/users/<u>/ssh_signing_keys` failed.
    #[error("Failed to fetch SSH keys for GitHub user {username}: {source}")]
    GithubKeyFetchFailed {
        /// GitHub username queried.
        username: SmolStr,
        /// Underlying xshell/curl failure.
        #[source]
        source: xshell::Error,
    },
    /// GitHub returned a body that didn't match the expected SSH-signing-key JSON shape.
    #[error("Failed to parse SSH signing keys response for GitHub user {username}: {source}")]
    GithubKeyParseFailed {
        /// GitHub username queried.
        username: SmolStr,
        /// Underlying serde_json failure.
        #[source]
        source: serde_json::Error,
    },
    /// `serde_json` failed to serialise the bundled JSON Schema.
    #[error("Failed to emit schema: {0}")]
    SchemaEmit(#[source] serde_json::Error),
    /// I/O error writing the serialised schema to stdout.
    #[error("Failed to write schema to stdout: {0}")]
    SchemaWrite(#[source] std::io::Error),
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
            (
                Self::UnresolvedInput {
                    pkg: l_pkg,
                    input: l_input,
                },
                Self::UnresolvedInput {
                    pkg: r_pkg,
                    input: r_input,
                },
            ) => l_pkg == r_pkg && l_input == r_input,
            (Self::TemplateUnterminated { pkg: l }, Self::TemplateUnterminated { pkg: r }) => {
                l == r
            }
            (Self::ApplyNeedsYesOrTty, Self::ApplyNeedsYesOrTty) => true,
            (Self::DirtyRepoRequiresAllowDirty(l0), Self::DirtyRepoRequiresAllowDirty(r0)) => {
                l0 == r0
            }
            (Self::PromptRead(l0), Self::PromptRead(r0)) => l0.kind() == r0.kind(),
            (Self::PromptInterrupted, Self::PromptInterrupted) => true,
            (Self::Output(l0), Self::Output(r0)) => l0.to_string() == r0.to_string(),
            (Self::InitDirNotEmpty(l0), Self::InitDirNotEmpty(r0)) => l0 == r0,
            (Self::InitDirExists(l0), Self::InitDirExists(r0)) => l0 == r0,
            (Self::InitGitDirExists(l0), Self::InitGitDirExists(r0)) => l0 == r0,
            (
                Self::InitGitInitFailed { source: l_src },
                Self::InitGitInitFailed { source: r_src },
            ) => l_src.to_string() == r_src.to_string(),
            (Self::InitNeedsTty, Self::InitNeedsTty) => true,
            (Self::InitNoConfigToml(l0), Self::InitNoConfigToml(r0)) => l0 == r0,
            (
                Self::InitCloneFailed {
                    url: l_url,
                    source: l_src,
                },
                Self::InitCloneFailed {
                    url: r_url,
                    source: r_src,
                },
            ) => l_url == r_url && l_src.to_string() == r_src.to_string(),
            (Self::InitIo(l0, l1), Self::InitIo(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            (Self::CurlNotFound, Self::CurlNotFound) => true,
            (
                Self::GithubKeyFetchFailed {
                    username: l_user,
                    source: l_src,
                },
                Self::GithubKeyFetchFailed {
                    username: r_user,
                    source: r_src,
                },
            ) => l_user == r_user && l_src.to_string() == r_src.to_string(),
            (
                Self::GithubKeyParseFailed {
                    username: l_user,
                    source: l_src,
                },
                Self::GithubKeyParseFailed {
                    username: r_user,
                    source: r_src,
                },
            ) => l_user == r_user && l_src.to_string() == r_src.to_string(),
            (Self::SchemaEmit(l), Self::SchemaEmit(r)) => l.to_string() == r.to_string(),
            (Self::SchemaWrite(l), Self::SchemaWrite(r)) => l.kind() == r.kind(),
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
    use xshell::{Shell, cmd};
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

    fn xshell_err() -> xshell::Error {
        // Re-running the same failing command produces equal `to_string()`,
        // which is what `PartialEq` compares for `Self::Shell(_)`.
        let sh = Shell::new().unwrap();
        cmd!(sh, "false").quiet().run().unwrap_err()
    }

    fn json_err() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").unwrap_err()
    }

    #[test]
    fn open_db_eq_compares_path_and_io_kind() {
        let a = Error::OpenDb(PathBuf::from("/x"), io(io::ErrorKind::NotFound));
        let b = Error::OpenDb(PathBuf::from("/x"), io(io::ErrorKind::NotFound));
        let c = Error::OpenDb(PathBuf::from("/y"), io(io::ErrorKind::NotFound));
        let d = Error::OpenDb(PathBuf::from("/x"), io(io::ErrorKind::PermissionDenied));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn parse_db_eq_compares_path_and_inner_error() {
        let toml_err = toml::from_str::<toml::Value>("not = valid = toml").unwrap_err();
        let toml_err2 = toml::from_str::<toml::Value>("not = valid = toml").unwrap_err();
        let a = Error::ParseDb(PathBuf::from("/x"), toml_err);
        let b = Error::ParseDb(PathBuf::from("/x"), toml_err2);
        assert_eq!(a, b);
    }

    #[test]
    fn shell_eq_compares_display_string() {
        let a = Error::Shell(xshell_err());
        let b = Error::Shell(xshell_err());
        assert_eq!(a, b);
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
    fn safe_relative_path_eq_delegates_to_inner() {
        let traversal_err =
            zenops_safe_relative_path::SafeRelativePath::from_relative_path("..").unwrap_err();
        let traversal_err2 =
            zenops_safe_relative_path::SafeRelativePath::from_relative_path("..").unwrap_err();
        let a = Error::SafeRelativePath(traversal_err);
        let b = Error::SafeRelativePath(traversal_err2);
        let c = Error::SafeRelativePath(
            zenops_safe_relative_path::error::Error::NotASinglePathComponent("a/b".to_string()),
        );
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
    }

    #[test]
    fn unresolved_input_eq_and_ne() {
        let a = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("i"),
        };
        let b = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("i"),
        };
        let c = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("other"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn template_unterminated_eq_and_ne() {
        let a = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("p"),
        };
        let b = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("p"),
        };
        let c = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("q"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn unit_variants_compare_equal_to_themselves() {
        assert_eq!(Error::ApplyNeedsYesOrTty, Error::ApplyNeedsYesOrTty);
        assert_eq!(Error::PromptInterrupted, Error::PromptInterrupted);
        assert_eq!(Error::InitNeedsTty, Error::InitNeedsTty);
        assert_eq!(Error::CurlNotFound, Error::CurlNotFound);
    }

    #[test]
    fn dirty_repo_requires_allow_dirty_eq_and_ne() {
        let a = Error::DirtyRepoRequiresAllowDirty(PathBuf::from("/x"));
        let b = Error::DirtyRepoRequiresAllowDirty(PathBuf::from("/x"));
        let c = Error::DirtyRepoRequiresAllowDirty(PathBuf::from("/y"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn prompt_read_eq_compares_io_kind() {
        let a = Error::PromptRead(io(io::ErrorKind::UnexpectedEof));
        let b = Error::PromptRead(io(io::ErrorKind::UnexpectedEof));
        let c = Error::PromptRead(io(io::ErrorKind::Other));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn output_eq_compares_display_string() {
        let a = Error::Output(OutputError::Io(io(io::ErrorKind::BrokenPipe)));
        let b = Error::Output(OutputError::Io(io(io::ErrorKind::BrokenPipe)));
        assert_eq!(a, b);
    }

    #[test]
    fn init_dir_variants_eq_and_ne() {
        assert_eq!(
            Error::InitDirNotEmpty(PathBuf::from("/a")),
            Error::InitDirNotEmpty(PathBuf::from("/a"))
        );
        assert_ne!(
            Error::InitDirNotEmpty(PathBuf::from("/a")),
            Error::InitDirNotEmpty(PathBuf::from("/b"))
        );
        assert_eq!(
            Error::InitDirExists(PathBuf::from("/a")),
            Error::InitDirExists(PathBuf::from("/a"))
        );
        assert_ne!(
            Error::InitDirExists(PathBuf::from("/a")),
            Error::InitDirExists(PathBuf::from("/b"))
        );
        assert_eq!(
            Error::InitGitDirExists(PathBuf::from("/a")),
            Error::InitGitDirExists(PathBuf::from("/a"))
        );
        assert_eq!(
            Error::InitNoConfigToml(PathBuf::from("/a")),
            Error::InitNoConfigToml(PathBuf::from("/a"))
        );
    }

    #[test]
    fn init_git_init_failed_eq_compares_display_string() {
        let a = Error::InitGitInitFailed {
            source: xshell_err(),
        };
        let b = Error::InitGitInitFailed {
            source: xshell_err(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn init_clone_failed_eq_compares_url_and_display() {
        let a = Error::InitCloneFailed {
            url: "https://example/x".to_string(),
            source: xshell_err(),
        };
        let b = Error::InitCloneFailed {
            url: "https://example/x".to_string(),
            source: xshell_err(),
        };
        let c = Error::InitCloneFailed {
            url: "https://example/y".to_string(),
            source: xshell_err(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn init_io_eq_and_ne() {
        let a = Error::InitIo(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let b = Error::InitIo(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let c = Error::InitIo(PathBuf::from("/x"), io(io::ErrorKind::NotFound));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn github_key_fetch_failed_eq_and_ne() {
        let a = Error::GithubKeyFetchFailed {
            username: SmolStr::new_static("u"),
            source: xshell_err(),
        };
        let b = Error::GithubKeyFetchFailed {
            username: SmolStr::new_static("u"),
            source: xshell_err(),
        };
        let c = Error::GithubKeyFetchFailed {
            username: SmolStr::new_static("v"),
            source: xshell_err(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn github_key_parse_failed_eq_and_ne() {
        let a = Error::GithubKeyParseFailed {
            username: SmolStr::new_static("u"),
            source: json_err(),
        };
        let b = Error::GithubKeyParseFailed {
            username: SmolStr::new_static("u"),
            source: json_err(),
        };
        let c = Error::GithubKeyParseFailed {
            username: SmolStr::new_static("v"),
            source: json_err(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn schema_emit_eq_compares_display_string() {
        let a = Error::SchemaEmit(json_err());
        let b = Error::SchemaEmit(json_err());
        assert_eq!(a, b);
    }

    #[test]
    fn schema_write_eq_compares_io_kind() {
        let a = Error::SchemaWrite(io(io::ErrorKind::BrokenPipe));
        let b = Error::SchemaWrite(io(io::ErrorKind::BrokenPipe));
        let c = Error::SchemaWrite(io(io::ErrorKind::Other));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn cross_variant_compare_returns_false() {
        assert_ne!(Error::ApplyNeedsYesOrTty, Error::PromptInterrupted);
        assert_ne!(
            Error::CurlNotFound,
            Error::OpenDb(PathBuf::from("/x"), io(io::ErrorKind::NotFound))
        );
        let _ = srpath!("dummy"); // keep srpath import in use
    }

    #[test]
    fn from_xshell_error_wraps_in_shell_variant() {
        let e: Error = xshell_err().into();
        assert!(matches!(e, Error::Shell(_)));
    }

    #[test]
    fn from_safe_relative_path_error_wraps() {
        let inner = zenops_safe_relative_path::error::Error::NotASinglePathComponent("a/b".into());
        let e: Error = inner.into();
        assert!(matches!(e, Error::SafeRelativePath(_)));
    }
}
