pub mod error;

use std::fmt;

use relative_path::{RelativePath, RelativePathBuf};

use crate::error::Error;

pub use safe_relative_path_macros::srpath;

/// Represents a relative path that is guaranteed to not perform traversal using ..
///
/// NOTE: This does not protect against symlinks and similar
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SafeRelativePath(RelativePath);

impl SafeRelativePath {
    /// Create a new safe relative path without checking
    ///
    /// # Safety
    /// The specified path must return successfully if passed to `Self::from_relative_path`
    pub unsafe fn new_unchecked<P>(v: &P) -> &Self
    where
        P: AsRef<RelativePath> + ?Sized,
    {
        unsafe { &*(v.as_ref() as *const RelativePath as *const SafeRelativePath) }
    }

    pub fn from_relative_path<P>(v: &P) -> Result<&Self, Error>
    where
        P: AsRef<RelativePath> + ?Sized,
    {
        let v = v.as_ref();

        if !safe_relative_path_validator::is_safe_relative_path(v) {
            return Err(Error::PathGoesOutsideParent(v.to_relative_path_buf()));
        }

        Ok(unsafe { Self::new_unchecked(v) })
    }

    pub fn to_safe_relative_path_buf(&self) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.to_relative_path_buf())
    }

    pub fn normalize_safe(&self) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.normalize())
    }

    pub fn with_prefix(&self, v: impl AsRef<SafeRelativePath>) -> SafeRelativePathBuf {
        SafeRelativePathBuf(v.as_ref().join(self))
    }
}

impl std::ops::Deref for SafeRelativePath {
    type Target = RelativePath;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<RelativePath> for SafeRelativePath {
    fn as_ref(&self) -> &RelativePath {
        &self.0
    }
}

impl AsRef<SafeRelativePath> for SafeRelativePath {
    fn as_ref(&self) -> &SafeRelativePath {
        self
    }
}

impl fmt::Display for SafeRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SafeRelativePathBuf(RelativePathBuf);

impl AsRef<SafeRelativePath> for SafeRelativePathBuf {
    fn as_ref(&self) -> &SafeRelativePath {
        unsafe { SafeRelativePath::new_unchecked(&self.0) }
    }
}

impl std::ops::Deref for SafeRelativePathBuf {
    type Target = RelativePath;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<RelativePath> for SafeRelativePathBuf {
    fn as_ref(&self) -> &RelativePath {
        self
    }
}

impl fmt::Display for SafeRelativePathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}
