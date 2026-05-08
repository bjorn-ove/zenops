//! `import`-scoped error type.
//!
//! Covers every failure of `zenops import`: path classification, the source
//! walk, the copy / remove / symlink steps of the apply phase, and reading
//! or writing `config.toml`. Wrapped into [`crate::Error`] as `Error::Import`
//! via `#[from]` + `#[error(transparent)]`.

use std::path::PathBuf;

use smol_str::SmolStr;

/// Failure modes for `zenops import`. Wrapped into [`crate::Error`] as
/// `Error::Import` via `#[from]`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Source path didn't exist on disk after canonicalization.
    #[error("Source path {0:?} does not exist")]
    SourceMissing(PathBuf),
    /// The path itself is a symlink — the user probably meant the target.
    #[error("Source path {0:?} is a symlink; point at the real file or directory instead")]
    SourceIsSymlink(PathBuf),
    /// Source contained no regular files to import.
    #[error("Source path {0:?} contains no regular files to import")]
    SourceEmpty(PathBuf),
    /// `.config/<x>` shape, but `<x>` is a regular file rather than a
    /// directory. We don't support importing a single file via the
    /// `.config` slot today.
    #[error(
        "Path {0:?} is a regular file; `~/.config/<x>` imports expect a directory. Use `~/.<file>` for single-file dotfiles, or move this into its own `~/.config/<x>/` directory first"
    )]
    ExpectedDirectory(PathBuf),
    /// Path resolved outside `$HOME` after canonicalization.
    #[error("Path {0:?} is not under $HOME")]
    PathNotUnderHome(PathBuf),
    /// Source canonicalized to the zenops repo dir itself (or a subpath
    /// of it) — importing it would copy the repo into itself.
    #[error(
        "Path {0:?} is the zenops repo (or inside it); refusing to import the repo into itself"
    )]
    CannotImportZenopsRepo(PathBuf),
    /// Zenops config dir / `config.toml` is missing — the user hasn't run
    /// `zenops init` yet.
    #[error(
        "No zenops config found at {0:?} — run `zenops init` first (or `zenops init <url>` to clone an existing one)"
    )]
    ZenopsRepoMissing(PathBuf),
    /// Resolved tail wasn't `.config/<x>` or `.<x>`. The string is the
    /// home-relative tail, included in the diagnostic.
    #[error(
        "Path layout {0:?} is not supported by `zenops import`; only ~/.config/<x> or ~/.<x> are recognized — point at the parent (e.g. `import .config/foo` instead of `.config/foo/themes`, or `.ssh` instead of `.ssh/config`)"
    )]
    UnsupportedLayout(String),
    /// The derived (or `--pkg`-supplied) pkg key isn't a single, safe path
    /// component.
    #[error("Cannot derive a valid pkg key from {0:?}; pass --pkg <KEY>")]
    NoDerivablePkgKey(String),
    /// Destination inside the zenops repo would clobber an existing file
    /// that isn't already the symlink target of the source.
    #[error("Destination {0:?} already exists; refusing to overwrite")]
    DestExists(PathBuf),
    /// More than one existing `[[pkg.<x>.configs]]` entry claims the
    /// imported path. Two configs with the same on-disk root are unusual —
    /// resolve by removing the duplicate before re-running.
    #[error(
        "Path {path:?} matches multiple managed configs ({}); resolve the duplicate in config.toml first",
        candidates.iter().map(SmolStr::as_str).collect::<Vec<_>>().join(", "),
    )]
    AmbiguousConfigMatch {
        /// Home-relative tail that matched multiple configs.
        path: PathBuf,
        /// Pkg keys whose configs all claim the path.
        candidates: Vec<SmolStr>,
    },
    /// Caller passed flags that only describe how to *create* a new pkg
    /// (e.g. `--pkg`, `--brew`) while the import is extending an existing
    /// managed config.
    #[error(
        "{} cannot be combined with importing a file into an already-managed config",
        flags.join(", "),
    )]
    ExtendFlagsInvalid {
        /// Flag names that conflict with extend mode.
        flags: Vec<&'static str>,
    },
    /// User pointed at a directory inside an already-managed config. The
    /// extend path only handles single files for now; recursive add of a
    /// subdirectory is a future extension.
    #[error(
        "Importing a subdirectory of an already-managed config isn't supported yet; pass a single file under {0:?}"
    )]
    ExtendDirectoryNotSupported(PathBuf),
    /// Caller is creating a brand-new pkg block but didn't provide
    /// install_hint info and didn't pass `--no-install-hint`.
    #[error(
        "pkg `{0}` is new — pass --brew <PKG>... to set the install hint, or --no-install-hint to skip"
    )]
    MissingInstallHint(String),
    /// User passed `--brew ""` (or otherwise asked to record an empty
    /// package name).
    #[error("--brew package name must not be empty")]
    EmptyBrewPackage,
    /// `--source <REL>` resolved outside `~/.config/zenops`. We refuse
    /// rather than risk landing files in arbitrary on-disk locations.
    #[error(
        "--source override {0:?} resolves outside the zenops repo; pass a path relative to ~/.config/zenops (e.g. `configs/<key>`)"
    )]
    SourceOverrideEscapesRepo(PathBuf),
    /// The derived (or `--pkg`-supplied) pkg key already names an
    /// existing pkg with at least one configs entry, and the imported
    /// path isn't under any of those entries.
    #[error(
        "pkg `{pkg}` already exists with managed configs; pass --pkg <NEW-KEY> to import under a different name, or --source <REL> to add another config to the same pkg"
    )]
    PkgKeyTaken {
        /// Pkg key that's already in use.
        pkg: SmolStr,
    },
    /// A prompt was needed but stdin isn't a TTY and `--yes` wasn't passed.
    #[error(
        "import requires a terminal for prompts; pass --yes (and --brew or --no-install-hint for new pkgs) to run non-interactively"
    )]
    NeedsTty,
    /// User declined the final confirmation.
    #[error("Import aborted")]
    Aborted,
    /// `config.toml` failed to parse as TOML.
    #[error("Failed to parse {0:?}: {1}")]
    ConfigParse(PathBuf, #[source] toml_edit::TomlError),
    /// Generic I/O error during the import (mkdir, walk, read).
    #[error("Import I/O error at {0:?}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    /// Copying a source file into the zenops repo failed.
    #[error("Failed to copy {src:?} -> {dst:?}: {source}")]
    Copy {
        /// File being copied from.
        src: PathBuf,
        /// File being copied to.
        dst: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Removing the original after copying it failed.
    #[error("Failed to remove original {0:?}: {1}")]
    RemoveOriginal(PathBuf, #[source] std::io::Error),
    /// Creating the in-place symlink failed.
    #[error("Failed to create symlink {symlink:?} -> {real:?}: {source}")]
    Symlink {
        /// Target the symlink should point at.
        real: PathBuf,
        /// Path the symlink was created at.
        symlink: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SourceMissing(l), Self::SourceMissing(r)) => l == r,
            (Self::SourceIsSymlink(l), Self::SourceIsSymlink(r)) => l == r,
            (Self::SourceEmpty(l), Self::SourceEmpty(r)) => l == r,
            (Self::ExpectedDirectory(l), Self::ExpectedDirectory(r)) => l == r,
            (Self::PathNotUnderHome(l), Self::PathNotUnderHome(r)) => l == r,
            (Self::CannotImportZenopsRepo(l), Self::CannotImportZenopsRepo(r)) => l == r,
            (Self::ZenopsRepoMissing(l), Self::ZenopsRepoMissing(r)) => l == r,
            (Self::UnsupportedLayout(l), Self::UnsupportedLayout(r)) => l == r,
            (Self::NoDerivablePkgKey(l), Self::NoDerivablePkgKey(r)) => l == r,
            (Self::DestExists(l), Self::DestExists(r)) => l == r,
            (Self::EmptyBrewPackage, Self::EmptyBrewPackage) => true,
            (Self::SourceOverrideEscapesRepo(l), Self::SourceOverrideEscapesRepo(r)) => l == r,
            (Self::PkgKeyTaken { pkg: l }, Self::PkgKeyTaken { pkg: r }) => l == r,
            (
                Self::AmbiguousConfigMatch {
                    path: l_path,
                    candidates: l_cands,
                },
                Self::AmbiguousConfigMatch {
                    path: r_path,
                    candidates: r_cands,
                },
            ) => l_path == r_path && l_cands == r_cands,
            (Self::ExtendFlagsInvalid { flags: l }, Self::ExtendFlagsInvalid { flags: r }) => {
                l == r
            }
            (Self::ExtendDirectoryNotSupported(l), Self::ExtendDirectoryNotSupported(r)) => l == r,
            (Self::MissingInstallHint(l), Self::MissingInstallHint(r)) => l == r,
            (Self::NeedsTty, Self::NeedsTty) => true,
            (Self::Aborted, Self::Aborted) => true,
            (Self::ConfigParse(l0, l1), Self::ConfigParse(r0, r1)) => {
                l0 == r0 && l1.to_string() == r1.to_string()
            }
            (Self::Io(l0, l1), Self::Io(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            (
                Self::Copy {
                    src: l_src,
                    dst: l_dst,
                    source: l_io,
                },
                Self::Copy {
                    src: r_src,
                    dst: r_dst,
                    source: r_io,
                },
            ) => l_src == r_src && l_dst == r_dst && l_io.kind() == r_io.kind(),
            (Self::RemoveOriginal(l0, l1), Self::RemoveOriginal(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (
                Self::Symlink {
                    real: l_real,
                    symlink: l_sym,
                    source: l_io,
                },
                Self::Symlink {
                    real: r_real,
                    symlink: r_sym,
                    source: r_io,
                },
            ) => l_real == r_real && l_sym == r_sym && l_io.kind() == r_io.kind(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;

    use similar_asserts::assert_eq;

    use super::*;

    fn io(kind: io::ErrorKind) -> io::Error {
        io::Error::from(kind)
    }

    #[test]
    fn unit_variants_compare_equal_to_themselves() {
        assert_eq!(Error::NeedsTty, Error::NeedsTty);
        assert_eq!(Error::Aborted, Error::Aborted);
    }

    #[test]
    fn path_variants_eq_and_ne() {
        assert_eq!(
            Error::SourceMissing(PathBuf::from("/a")),
            Error::SourceMissing(PathBuf::from("/a"))
        );
        assert_ne!(
            Error::SourceMissing(PathBuf::from("/a")),
            Error::SourceMissing(PathBuf::from("/b"))
        );
        assert_eq!(
            Error::DestExists(PathBuf::from("/a")),
            Error::DestExists(PathBuf::from("/a"))
        );
    }

    #[test]
    fn unsupported_layout_eq_and_ne() {
        assert_eq!(
            Error::UnsupportedLayout(".config/foo/themes".into()),
            Error::UnsupportedLayout(".config/foo/themes".into())
        );
        assert_ne!(
            Error::UnsupportedLayout(".config/foo/themes".into()),
            Error::UnsupportedLayout("dotfiles/zsh".into())
        );
    }

    #[test]
    fn io_eq_compares_path_and_kind() {
        let a = Error::Io(PathBuf::from("/a"), io(io::ErrorKind::Other));
        let b = Error::Io(PathBuf::from("/a"), io(io::ErrorKind::Other));
        let c = Error::Io(PathBuf::from("/a"), io(io::ErrorKind::NotFound));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn copy_eq_compares_paths_and_kind() {
        let a = Error::Copy {
            src: PathBuf::from("/s"),
            dst: PathBuf::from("/d"),
            source: io(io::ErrorKind::Other),
        };
        let b = Error::Copy {
            src: PathBuf::from("/s"),
            dst: PathBuf::from("/d"),
            source: io(io::ErrorKind::Other),
        };
        let c = Error::Copy {
            src: PathBuf::from("/s"),
            dst: PathBuf::from("/d"),
            source: io(io::ErrorKind::PermissionDenied),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
