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
    Shell(xshell::Error),
    /// Writing a generated config file to disk failed.
    #[error("Failed to write config file {0}: {1}")]
    FailedToWriteConfig(ResolvedConfigFilePath, std::io::Error),
    /// A `..`-traversal or other path-safety violation surfaced from
    /// [`zenops_safe_relative_path`].
    #[error(transparent)]
    SafeRelativePath(zenops_safe_relative_path::error::Error),
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
            (Self::Output(l0), Self::Output(r0)) => l0.to_string() == r0.to_string(),
            (Self::InitDirNotEmpty(l0), Self::InitDirNotEmpty(r0)) => l0 == r0,
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

impl From<xshell::Error> for Error {
    fn from(e: xshell::Error) -> Self {
        Self::Shell(e)
    }
}

impl From<zenops_safe_relative_path::error::Error> for Error {
    fn from(e: zenops_safe_relative_path::error::Error) -> Self {
        Self::SafeRelativePath(e)
    }
}
