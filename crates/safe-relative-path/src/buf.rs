use std::{fmt, sync::Arc};

use relative_path::{RelativePath, RelativePathBuf};
use serde::{de, ser};

use crate::{SafeRelativePath, error::Error};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl ser::Serialize for SafeRelativePathBuf {
    fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> de::Deserialize<'de> for SafeRelativePathBuf {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = SafeRelativePathBuf;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "path")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                SafeRelativePathBuf::from_relative_path(v).map_err(de::Error::custom)
            }
        }
        d.deserialize_str(Visitor)
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
    type Target = SafeRelativePath;

    fn deref(&self) -> &Self::Target {
        unsafe { SafeRelativePath::new_unchecked(&self.0) }
    }
}

impl AsRef<RelativePath> for SafeRelativePathBuf {
    fn as_ref(&self) -> &RelativePath {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for SafeRelativePathBuf {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.as_safe_rel_path().as_ref()
    }
}

impl fmt::Debug for SafeRelativePathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
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
        SafeRelativePathBuf(self.0.to_relative_path_buf())
    }

    pub fn normalize_safe(&self) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.0.normalize())
    }

    pub fn safe_join(&self, path: impl AsRef<SafeRelativePath>) -> SafeRelativePathBuf {
        SafeRelativePathBuf(self.0.join(&path.as_ref().0))
    }
}
