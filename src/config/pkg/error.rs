/// Failures from pkg `detect` / `doctor` evaluation: OS resolution, path
/// existence probes, binary lookups, and condition references.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `std::env::consts::OS` returned a value that doesn't map to a
    /// supported `Os` variant.
    #[error("Unknown OS: {0}")]
    UnknownOs(&'static str),
    /// `Path::try_exists` failed for an `exists = "..."` detect leaf. The
    /// expanded path is the first field; the IO error is the second.
    #[error("Failed to check if {0:?} exists: {1}")]
    ExistsFailed(String, std::io::Error),
    /// A `which = "..."` detect leaf hit an error other than "not found"
    /// (typically a canonicalize failure on the matched binary).
    #[error(transparent)]
    Which(#[from] crate::utils::which::Error),
    /// Looking up or evaluating a `[conditions]` entry referenced from a
    /// pkg's `when` gate failed (unknown name, regex compile error, …).
    #[error(transparent)]
    Condition(#[from] crate::config::condition::Error),
}
