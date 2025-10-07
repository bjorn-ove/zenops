use std::{fmt, sync::Arc};

use relative_path::{RelativePath, RelativePathBuf};

use crate::{SafeRelativePath, error::Error};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SafeRelativePathBuf(RelativePathBuf);

impl SafeRelativePathBuf {
    pub fn from_relative_path<P>(v: &P) -> Result<Self, Error>
    where
        P: AsRef<RelativePath> + ?Sized,
    {
        SafeRelativePath::from_relative_path(v).map(|p| p.to_safe_relative_path_buf())
    }

    fn as_safe_rel_path(&self) -> &SafeRelativePath {
        unsafe { SafeRelativePath::new_unchecked(&self.0) }
    }
}

impl std::str::FromStr for SafeRelativePathBuf {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_relative_path(s)
    }
}

impl AsRef<SafeRelativePath> for SafeRelativePathBuf {
    fn as_ref(&self) -> &SafeRelativePath {
        self.as_safe_rel_path()
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

impl AsRef<std::ffi::OsStr> for SafeRelativePathBuf {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.as_safe_rel_path().as_ref()
    }
}

impl fmt::Display for SafeRelativePathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<SafeRelativePathBuf> for Arc<SafeRelativePath> {
    fn from(value: SafeRelativePathBuf) -> Self {
        let arc_rel: Arc<RelativePath> = Arc::from(value.0);
        unsafe { Arc::from_raw(Arc::into_raw(arc_rel) as *const SafeRelativePath) }
    }
}

impl SafeRelativePath {
    pub fn to_safe_relative_path_buf(&self) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.to_relative_path_buf())
    }

    pub fn normalize_safe(&self) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.normalize())
    }

    pub fn with_prefix(&self, v: impl AsRef<SafeRelativePath>) -> SafeRelativePathBuf {
        SafeRelativePathBuf(v.as_ref().join(self))
    }

    pub fn safe_join(&self, path: impl AsRef<SafeRelativePath>) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.join(&path.as_ref().0))
    }
}
