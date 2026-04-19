use std::fmt;

use serde::de;

use zenops_safe_relative_path::{SafeRelativePath, SafeRelativePathBuf};

/// Represents a relative path that cannot leave its parent directory,
/// unless there is filesystem shenanigans (e.g. symlinks).
#[derive(Clone, Debug)]
pub(super) struct StoredRelativePath {
    /// The original path, as written in the config, for display and serialization purposes
    org: String,
    /// The normalized path
    normal: SafeRelativePathBuf,
}

impl StoredRelativePath {
    /// Returns the unique part to use when implementing traits
    /// NOTE: self.normal is generated from self.org and can't be otherwise modified, so no need to include it
    const fn unique_part(&self) -> &String {
        &self.org
    }
}

impl PartialEq for StoredRelativePath {
    fn eq(&self, other: &Self) -> bool {
        self.unique_part() == other.unique_part()
    }
}

impl Eq for StoredRelativePath {}

impl std::hash::Hash for StoredRelativePath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.unique_part().hash(state);
    }
}

impl fmt::Display for StoredRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.org, f)
    }
}

impl std::ops::Deref for StoredRelativePath {
    type Target = SafeRelativePath;

    fn deref(&self) -> &Self::Target {
        self.normal.as_ref()
    }
}

impl AsRef<SafeRelativePath> for StoredRelativePath {
    fn as_ref(&self) -> &SafeRelativePath {
        self.normal.as_ref()
    }
}

impl<'de> de::Deserialize<'de> for StoredRelativePath {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StoredRelativePath;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "version string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StoredRelativePath {
                    org: v.to_string(),
                    normal: SafeRelativePath::from_relative_path(v)
                        .map_err(de::Error::custom)?
                        .normalize_safe(),
                })
            }
        }

        d.deserialize_any(Visitor)
    }
}
