use core::fmt;

use smol_str::SmolStr;

use crate::error::Error;

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub(super) enum StoredCargoPackage {
    CratesIo {},
}

impl StoredCargoPackage {
    pub fn into_package(self, name: SmolStr) -> CargoPackage {
        CargoPackage { name, stored: self }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CargoPackage {
    name: SmolStr,
    stored: StoredCargoPackage,
}

impl CargoPackage {
    pub fn get_name_if_crates_io_latest(&self) -> Option<SmolStr> {
        let Self { name, stored } = self;
        match stored {
            StoredCargoPackage::CratesIo {} => Some(name.clone()),
        }
    }

    pub fn from_split_spec(name: impl Into<SmolStr>, version: &str) -> Result<Self, Error> {
        if version.is_empty() {
            Ok(Self {
                name: name.into(),
                stored: StoredCargoPackage::CratesIo {},
            })
        } else {
            todo!()
        }
    }
}

impl fmt::Display for CargoPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}
