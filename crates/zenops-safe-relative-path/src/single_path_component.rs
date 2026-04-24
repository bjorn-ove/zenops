use std::fmt;

use crate::{SafeRelativePath, error::Error};
use serde::de;
use smol_str::{SmolStr, ToSmolStr};

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SinglePathComponent(SmolStr);

impl SinglePathComponent {
    /// Construct a [`SinglePathComponent`] from a string. Fails if the string
    /// contains traversal (`..`) or more than one path component.
    pub fn try_new(v: &str) -> Result<Self, Error> {
        let path = SafeRelativePath::from_relative_path(v)?;
        let first = path.0.components().map(|c| c.as_str()).next();
        if first == Some(v) {
            Ok(Self(v.to_smolstr()))
        } else {
            Err(Error::NotASinglePathComponent(v.to_string()))
        }
    }

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

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for SinglePathComponent {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SinglePathComponent".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A single path component — no separators, no `..` traversal.",
            "pattern": "^[^/]+$",
        })
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
                SinglePathComponent::try_new(v).map_err(de::Error::custom)
            }
        }

        d.deserialize_any(Visitor)
    }
}
