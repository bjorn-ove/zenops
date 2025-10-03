use std::fmt;

use safe_relative_path::SafeRelativePath;
use serde::de;
use smol_str::{SmolStr, ToSmolStr};

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub(super) struct StoredSinglePathComponent(SmolStr);

impl AsRef<SafeRelativePath> for StoredSinglePathComponent {
    fn as_ref(&self) -> &SafeRelativePath {
        unsafe { SafeRelativePath::new_unchecked(self.0.as_str()) }
    }
}

impl fmt::Display for StoredSinglePathComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<'de> de::Deserialize<'de> for StoredSinglePathComponent {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StoredSinglePathComponent;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "single path component")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let path = SafeRelativePath::from_relative_path(v).map_err(de::Error::custom)?;

                // Check if the first path component is equal to the original string, if it is, we have one component
                if let Some(first) = path.components().map(|v| v.as_str()).next()
                    && v == first
                {
                    return Ok(StoredSinglePathComponent(v.to_smolstr()));
                }
                Err(de::Error::custom(format_args!("Invalid value {v:?}")))
            }
        }

        d.deserialize_any(Visitor)
    }
}
