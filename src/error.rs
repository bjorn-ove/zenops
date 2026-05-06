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

use crate::output::OutputError;

/// Crate-wide error. Each variant's user-facing string lives on its
/// `#[error(...)]` attribute (the `Display` impl); the doc comment here
/// adds the trigger context the message can't carry.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Wraps [`crate::config::ConfigError`].
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    /// A subprocess invoked through `xshell` failed (non-zero exit, signal,
    /// I/O error). The wrapped error carries the command and stderr.
    #[error("Failed to execute command")]
    Shell(#[from] xshell::Error),
    /// Wraps [`crate::config_files::ConfigFilesError`].
    #[error(transparent)]
    ConfigFiles(#[from] crate::config_files::ConfigFilesError),
    /// A `..`-traversal or other path-safety violation surfaced from
    /// [`zenops_safe_relative_path`].
    #[error(transparent)]
    SafeRelativePath(#[from] zenops_safe_relative_path::error::Error),
    /// Wraps [`crate::config::shell::ConfigShellError`].
    #[error(transparent)]
    ConfigShell(#[from] crate::config::shell::ConfigShellError),
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
    /// Wraps [`crate::prompt::PromptError`].
    #[error(transparent)]
    Prompt(#[from] crate::prompt::PromptError),
    /// An [`Output`](crate::output::Output) implementation failed to write
    /// (rendering or JSON serialization error).
    #[error(transparent)]
    Output(#[from] OutputError),
    /// Wraps [`crate::init::InitError`].
    #[error(transparent)]
    Init(#[from] crate::init::InitError),
    /// Wraps [`crate::config::ssh::SshError`].
    #[error(transparent)]
    Ssh(#[from] crate::config::ssh::SshError),
    /// Wraps [`crate::schema::SchemaError`].
    #[error(transparent)]
    Schema(#[from] crate::schema::SchemaError),
    /// Wraps [`crate::config::pkg::Error`].
    #[error(transparent)]
    PkgError(#[from] crate::config::pkg::Error),
    /// Wraps [`crate::utils::which::Error`].
    #[error(transparent)]
    Which(#[from] crate::utils::which::Error),
    /// `home::home_dir()` returned `None` — couldn't determine the user's
    /// home directory. Bubbled out of `main` rather than panicking.
    #[error("Could not determine the user's home directory")]
    NoHomeDir,
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Config(l), Self::Config(r)) => l == r,
            (Self::Shell(l0), Self::Shell(r0)) => l0.to_string() == r0.to_string(),
            (Self::ConfigFiles(l0), Self::ConfigFiles(r0)) => l0 == r0,
            (Self::SafeRelativePath(l0), Self::SafeRelativePath(r0)) => l0 == r0,
            (Self::ConfigShell(l), Self::ConfigShell(r)) => l == r,
            (Self::ApplyNeedsYesOrTty, Self::ApplyNeedsYesOrTty) => true,
            (Self::DirtyRepoRequiresAllowDirty(l0), Self::DirtyRepoRequiresAllowDirty(r0)) => {
                l0 == r0
            }
            (Self::Prompt(l), Self::Prompt(r)) => l == r,
            (Self::Output(l0), Self::Output(r0)) => l0.to_string() == r0.to_string(),
            (Self::Init(l0), Self::Init(r0)) => l0 == r0,
            (Self::Ssh(l0), Self::Ssh(r0)) => l0 == r0,
            (Self::Schema(l), Self::Schema(r)) => l == r,
            (Self::NoHomeDir, Self::NoHomeDir) => true,
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

    #[test]
    fn config_wrap_eq_delegates_to_inner() {
        let a = Error::Config(crate::config::ConfigError::OpenDb(
            PathBuf::from("/x"),
            io(io::ErrorKind::NotFound),
        ));
        let b = Error::Config(crate::config::ConfigError::OpenDb(
            PathBuf::from("/x"),
            io(io::ErrorKind::NotFound),
        ));
        let c = Error::Config(crate::config::ConfigError::OpenDb(
            PathBuf::from("/y"),
            io(io::ErrorKind::NotFound),
        ));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_config_error_wraps_in_config_variant() {
        let inner =
            crate::config::ConfigError::OpenDb(PathBuf::from("/x"), io(io::ErrorKind::Other));
        let e: Error = inner.into();
        assert!(matches!(e, Error::Config(_)));
    }

    #[test]
    fn shell_eq_compares_display_string() {
        let a = Error::Shell(xshell_err());
        let b = Error::Shell(xshell_err());
        assert_eq!(a, b);
    }

    #[test]
    fn config_files_wrap_eq_delegates_to_inner() {
        let a = Error::ConfigFiles(crate::config_files::ConfigFilesError::FailedToWriteConfig(
            rcfp("a"),
            io(io::ErrorKind::PermissionDenied),
        ));
        let b = Error::ConfigFiles(crate::config_files::ConfigFilesError::FailedToWriteConfig(
            rcfp("a"),
            io(io::ErrorKind::PermissionDenied),
        ));
        let c = Error::ConfigFiles(crate::config_files::ConfigFilesError::FailedToWriteConfig(
            rcfp("b"),
            io(io::ErrorKind::PermissionDenied),
        ));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_config_files_error_wraps_in_config_files_variant() {
        let inner =
            crate::config_files::ConfigFilesError::RefusingToOverwriteOtherWithSymlink(rcfp("a"));
        let e: Error = inner.into();
        assert!(matches!(e, Error::ConfigFiles(_)));
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
    fn config_shell_wrap_eq_delegates_to_inner() {
        let a = Error::ConfigShell(
            crate::config::shell::ConfigShellError::TemplateUnterminated {
                pkg: smol_str::SmolStr::new_static("p"),
            },
        );
        let b = Error::ConfigShell(
            crate::config::shell::ConfigShellError::TemplateUnterminated {
                pkg: smol_str::SmolStr::new_static("p"),
            },
        );
        let c = Error::ConfigShell(
            crate::config::shell::ConfigShellError::TemplateUnterminated {
                pkg: smol_str::SmolStr::new_static("q"),
            },
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_config_shell_error_wraps_in_config_shell_variant() {
        let inner = crate::config::shell::ConfigShellError::TemplateUnterminated {
            pkg: smol_str::SmolStr::new_static("p"),
        };
        let e: Error = inner.into();
        assert!(matches!(e, Error::ConfigShell(_)));
    }

    #[test]
    fn unit_variants_compare_equal_to_themselves() {
        assert_eq!(Error::ApplyNeedsYesOrTty, Error::ApplyNeedsYesOrTty);
        assert_eq!(Error::NoHomeDir, Error::NoHomeDir);
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
    fn prompt_wrap_eq_delegates_to_inner() {
        let a = Error::Prompt(crate::prompt::PromptError::Interrupted);
        let b = Error::Prompt(crate::prompt::PromptError::Interrupted);
        let c = Error::Prompt(crate::prompt::PromptError::Read(io(io::ErrorKind::Other)));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_prompt_error_wraps_in_prompt_variant() {
        let inner = crate::prompt::PromptError::Interrupted;
        let e: Error = inner.into();
        assert!(matches!(e, Error::Prompt(_)));
    }

    #[test]
    fn output_eq_compares_display_string() {
        let a = Error::Output(OutputError::Io(io(io::ErrorKind::BrokenPipe)));
        let b = Error::Output(OutputError::Io(io(io::ErrorKind::BrokenPipe)));
        assert_eq!(a, b);
    }

    #[test]
    fn init_wrap_eq_delegates_to_inner() {
        let a = Error::Init(crate::init::InitError::DirNotEmpty(PathBuf::from("/a")));
        let b = Error::Init(crate::init::InitError::DirNotEmpty(PathBuf::from("/a")));
        let c = Error::Init(crate::init::InitError::DirNotEmpty(PathBuf::from("/b")));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_init_error_wraps_in_init_variant() {
        let inner = crate::init::InitError::NeedsTty;
        let e: Error = inner.into();
        assert!(matches!(e, Error::Init(_)));
    }

    #[test]
    fn ssh_wrap_eq_delegates_to_inner() {
        let a = Error::Ssh(crate::config::ssh::SshError::CurlNotFound);
        let b = Error::Ssh(crate::config::ssh::SshError::CurlNotFound);
        assert_eq!(a, b);
    }

    #[test]
    fn from_ssh_error_wraps_in_ssh_variant() {
        let inner = crate::config::ssh::SshError::CurlNotFound;
        let e: Error = inner.into();
        assert!(matches!(e, Error::Ssh(_)));
    }

    #[test]
    fn schema_wrap_eq_delegates_to_inner() {
        let a = Error::Schema(crate::schema::SchemaError::Write(io(
            io::ErrorKind::BrokenPipe,
        )));
        let b = Error::Schema(crate::schema::SchemaError::Write(io(
            io::ErrorKind::BrokenPipe,
        )));
        let c = Error::Schema(crate::schema::SchemaError::Write(io(io::ErrorKind::Other)));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_schema_error_wraps_in_schema_variant() {
        let inner = crate::schema::SchemaError::Write(io(io::ErrorKind::BrokenPipe));
        let e: Error = inner.into();
        assert!(matches!(e, Error::Schema(_)));
    }

    #[test]
    fn cross_variant_compare_returns_false() {
        assert_ne!(Error::ApplyNeedsYesOrTty, Error::NoHomeDir);
        assert_ne!(
            Error::NoHomeDir,
            Error::DirtyRepoRequiresAllowDirty(PathBuf::from("/x")),
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
