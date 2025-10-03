use std::fmt;

use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use crate::error::Error;

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub(super) enum StoredBrewPackage {
    Brew { version: Option<SmolStr> },
    Cask {},
}

impl StoredBrewPackage {
    pub fn into_package(self, name: SmolStr) -> BrewPackage {
        BrewPackage { name, stored: self }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrewPackage {
    name: SmolStr,
    stored: StoredBrewPackage,
}

impl BrewPackage {
    pub fn brew_spec(&self) -> Option<SmolStr> {
        let Self { name, stored } = self;
        match stored {
            StoredBrewPackage::Brew { version } => Some(if let Some(version) = version {
                format_smolstr!("{name}@{version}")
            } else {
                self.name.clone()
            }),
            StoredBrewPackage::Cask {} => None,
        }
    }

    pub fn cask_spec(&self) -> Option<SmolStr> {
        let Self { name, stored } = self;
        match stored {
            StoredBrewPackage::Brew { .. } => None,
            StoredBrewPackage::Cask {} => Some(name.clone()),
        }
    }

    pub fn from_split_spec(name: impl Into<SmolStr>, version: &str) -> Result<Self, Error> {
        if version.is_empty() {
            Ok(Self {
                name: name.into(),
                stored: StoredBrewPackage::Brew { version: None },
            })
        } else if version == "cask" {
            Ok(Self {
                name: name.into(),
                stored: StoredBrewPackage::Cask {},
            })
        } else {
            Ok(Self {
                name: name.into(),
                stored: StoredBrewPackage::Brew {
                    version: Some(version.to_smolstr()),
                },
            })
        }
    }
}

impl fmt::Display for BrewPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}
