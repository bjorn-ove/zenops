//! `schema`-scoped error type.
//!
//! Wraps the failure modes specific to emitting the JSON Schema bundle
//! for `zenops schema`: serialising the bundle and writing it to stdout.
//! Exposed to the rest of the crate as `crate::Error::Schema` via
//! `#[error(transparent)]` + `#[from]`.

/// Failure modes for [`super::run`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `serde_json` failed to serialise the bundled JSON Schema.
    #[error("Failed to emit schema: {0}")]
    Emit(#[source] serde_json::Error),
    /// I/O error writing the serialised schema to stdout.
    #[error("Failed to write schema to stdout: {0}")]
    Write(#[source] std::io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Emit(l), Self::Emit(r)) => l.to_string() == r.to_string(),
            (Self::Write(l), Self::Write(r)) => l.kind() == r.kind(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use similar_asserts::assert_eq;

    use super::*;

    fn io(kind: io::ErrorKind) -> io::Error {
        io::Error::from(kind)
    }

    fn json_err() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").unwrap_err()
    }

    #[test]
    fn emit_eq_compares_display_string() {
        let a = Error::Emit(json_err());
        let b = Error::Emit(json_err());
        assert_eq!(a, b);
    }

    #[test]
    fn write_eq_compares_io_kind() {
        let a = Error::Write(io(io::ErrorKind::BrokenPipe));
        let b = Error::Write(io(io::ErrorKind::BrokenPipe));
        let c = Error::Write(io(io::ErrorKind::Other));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn cross_variant_compare_returns_false() {
        assert_ne!(
            Error::Emit(json_err()),
            Error::Write(io(io::ErrorKind::Other))
        );
    }
}
