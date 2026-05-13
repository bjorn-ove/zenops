use std::path::PathBuf;

use zenops_expand::{ExpandError, ExpandLookup, ExpandStr};

/// Failures from PATH-binary lookup. "Binary not found" is *not* an error
/// here — `get_path` maps the underlying `which::Error::CannotFindBinaryPath`
/// / `CannotGetCurrentDirAndPathListEmpty` to `Ok(None)` — so this enum
/// only covers the cases where the user's input itself was bad or the
/// lookup machinery broke.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `${...}` placeholder expansion on the binary spec failed before
    /// `which` was even invoked. First field is the unexpanded spec.
    #[error("Failed to expand executable path from {0:?}: {1}")]
    ExpandError(ExpandStr, ExpandError),
    /// `which::which` returned an error other than "not found" — typically
    /// `CannotCanonicalize` after a match was found but couldn't be
    /// resolved. First field is the binary name that was looked up.
    #[error("Failed to find executable from {0:?}: {1}")]
    Which(String, which::Error),
}

pub fn get_path(binary: impl AsRef<str> + Into<String>) -> Result<Option<PathBuf>, Error> {
    ::which::which(binary.as_ref())
        .map(Some)
        .or_else(|e| match e {
            which::Error::CannotFindBinaryPath
            | which::Error::CannotGetCurrentDirAndPathListEmpty => Ok(None),
            which::Error::CannotCanonicalize => Err(Error::Which(binary.into(), e)),
        })
}

pub fn exists(binary: impl AsRef<str> + Into<String>) -> Result<bool, Error> {
    get_path(binary).map(|v| v.is_some())
}

pub fn expand_and_get_path(
    binary: &ExpandStr,
    lookup: &impl ExpandLookup,
) -> Result<Option<PathBuf>, Error> {
    let b = binary
        .expand_to_string(lookup)
        .map_err(|e| Error::ExpandError(binary.clone(), e))?;
    get_path(b)
}

pub fn expand_and_exists(binary: &ExpandStr, lookup: &impl ExpandLookup) -> Result<bool, Error> {
    expand_and_get_path(binary, lookup).map(|v| v.is_some())
}
