use std::fmt;

use crate::SafeRelativePath;
use serde::de;
use smol_str::{SmolStr, ToSmolStr};

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SinglePathComponent(SmolStr);

impl SinglePathComponent {
    pub fn as_safe_relative_path(&self) -> &SafeRelativePath {
        unsafe { SafeRelativePath::new_unchecked_from_str(self.0.as_str()) }
    }
}

impl AsRef<SafeRelativePath> for SinglePathComponent {
    fn as_ref(&self) -> &SafeRelativePath {
        self.as_safe_relative_path()
    }
}

impl fmt::Display for SinglePathComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::ops::Deref for SinglePathComponent {
    type Target = SafeRelativePath;

    fn deref(&self) -> &Self::Target {
        self.as_safe_relative_path()
    }
}

impl<'de> de::Deserialize<'de> for SinglePathComponent {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = SinglePathComponent;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "single path component")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let path = SafeRelativePath::from_relative_path(v).map_err(de::Error::custom)?;

                // Check if the first path component is equal to the original string, if it is, we have one component
                if let Some(first) = path.0.components().map(|v| v.as_str()).next()
                    && v == first
                {
                    return Ok(SinglePathComponent(v.to_smolstr()));
                }
                Err(de::Error::custom(format_args!("Invalid value {v:?}")))
            }
        }

        d.deserialize_any(Visitor)
    }
}
