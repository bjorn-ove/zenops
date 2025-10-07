use std::{fmt, sync::Arc};

use relative_path::RelativePath;

use crate::error::Error;

mod buf;
pub mod error;

pub use buf::SafeRelativePathBuf;
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
    pub const unsafe fn new_unchecked_from_str(v: &str) -> &Self {
        unsafe { &*(v as *const str as *const RelativePath as *const SafeRelativePath) }
    }

    /// Create a new safe relative path without checking
    ///
    /// # Safety
    /// The specified path must return successfully if passed to `Self::from_relative_path`
    pub const unsafe fn new_unchecked(v: &RelativePath) -> &Self {
        unsafe { &*(v as *const RelativePath as *const SafeRelativePath) }
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

    pub fn try_join(&self, path: impl AsRef<RelativePath>) -> Result<SafeRelativePathBuf, Error> {
        Ok(self.safe_join(Self::from_relative_path(&path)?))
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

impl AsRef<std::ffi::OsStr> for SafeRelativePath {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_str().as_ref()
    }
}

impl AsRef<SafeRelativePath> for SafeRelativePath {
    fn as_ref(&self) -> &SafeRelativePath {
        self
    }
}

impl<'a> From<&'a SafeRelativePath> for SafeRelativePathBuf {
    fn from(value: &'a SafeRelativePath) -> Self {
        value.to_safe_relative_path_buf()
    }
}

impl<'a> From<&'a SafeRelativePath> for Arc<SafeRelativePath> {
    fn from(value: &'a SafeRelativePath) -> Self {
        let arc_rel: Arc<RelativePath> = Arc::from(&value.0);
        unsafe { Arc::from_raw(Arc::into_raw(arc_rel) as *const SafeRelativePath) }
    }
}

impl fmt::Display for SafeRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}
