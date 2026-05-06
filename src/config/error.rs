//! `config`-scoped error type.
//!
//! Wraps the failure modes of [`super::Config::load`]: opening and parsing
//! `config.toml`, plus probing for a `brew` install prefix while building
//! system inputs. Exposed to the rest of the crate as `crate::Error::Config`
//! via `#[error(transparent)]` + `#[from]`.

use std::path::PathBuf;

/// Failure modes for `Config::load` and the helpers it drives.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O error opening `config.toml` (e.g. missing file, permission denied).
    #[error("Failed to open the database file {0:?}: {1}")]
    OpenDb(PathBuf, #[source] std::io::Error),
    /// `config.toml` parsed but failed TOML deserialization or schema validation.
    #[error("Failed to parse the database from file {0:?}: {1}")]
    ParseDb(PathBuf, #[source] toml::de::Error),
    /// Failed to stat a candidate brew install prefix while detecting the
    /// system package manager.
    #[error("Failed to probe for brew at {0:?}: {1}")]
    BrewProbeFailed(PathBuf, #[source] std::io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::OpenDb(l0, l1), Self::OpenDb(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            (Self::ParseDb(l0, l1), Self::ParseDb(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::BrewProbeFailed(l0, l1), Self::BrewProbeFailed(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
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
    fn brew_probe_failed_eq_and_ne() {
        let a = Error::BrewProbeFailed(
            PathBuf::from("/opt/homebrew/bin/brew"),
            io(io::ErrorKind::PermissionDenied),
        );
        let b = Error::BrewProbeFailed(
            PathBuf::from("/opt/homebrew/bin/brew"),
            io(io::ErrorKind::PermissionDenied),
        );
        let c = Error::BrewProbeFailed(
            PathBuf::from("/usr/local/bin/brew"),
            io(io::ErrorKind::PermissionDenied),
        );
        let d = Error::BrewProbeFailed(
            PathBuf::from("/opt/homebrew/bin/brew"),
            io(io::ErrorKind::NotFound),
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn cross_variant_compare_returns_false() {
        assert_ne!(
            Error::OpenDb(PathBuf::from("/x"), io(io::ErrorKind::NotFound)),
            Error::BrewProbeFailed(PathBuf::from("/x"), io(io::ErrorKind::NotFound)),
        );
    }
}
