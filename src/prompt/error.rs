//! `prompt`-scoped error type.
//!
//! Wraps the failure modes of the interactive confirmation flow used by
//! `zenops apply`: reading a line from stdin, and the user pressing Ctrl-C.
//! Exposed to the rest of the crate as `crate::Error::Prompt` via
//! `#[error(transparent)]` + `#[from]`.

/// Failure modes for [`super::Prompter`] implementations and the helpers
/// they use.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O error reading a yes/no answer from stdin.
    #[error("Failed to read confirmation from stdin: {0}")]
    Read(#[source] std::io::Error),
    /// User pressed Ctrl-C at an interactive prompt. Distinct from a
    /// closed stdin or Ctrl-D so callers can abort the whole run.
    #[error("Interrupted")]
    Interrupted,
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Read(l), Self::Read(r)) => l.kind() == r.kind(),
            (Self::Interrupted, Self::Interrupted) => true,
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

    #[test]
    fn read_eq_compares_io_kind() {
        let a = Error::Read(io(io::ErrorKind::UnexpectedEof));
        let b = Error::Read(io(io::ErrorKind::UnexpectedEof));
        let c = Error::Read(io(io::ErrorKind::Other));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn interrupted_eq() {
        assert_eq!(Error::Interrupted, Error::Interrupted);
    }

    #[test]
    fn cross_variant_compare_returns_false() {
        assert_ne!(Error::Interrupted, Error::Read(io(io::ErrorKind::Other)));
    }
}
