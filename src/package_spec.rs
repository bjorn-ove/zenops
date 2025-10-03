mod brew;
mod cargo;

use std::{fmt, str::FromStr};

use serde::de;
use smol_str::SmolStr;

use crate::{
    error::Error,
    package_spec::{
        brew::{BrewPackage, StoredBrewPackage},
        cargo::{CargoPackage, StoredCargoPackage},
    },
};

#[derive(Debug, Clone, PartialEq)]
pub enum PackageSpec {
    Brew(BrewPackage),
    Cargo(CargoPackage),
}

impl PackageSpec {
    pub fn from_split_spec(
        kind: &str,
        name: impl Into<SmolStr>,
        version: &str,
    ) -> Result<Self, Error> {
        match kind {
            "cargo" => CargoPackage::from_split_spec(name, version).map(Self::Cargo),
            "brew" => BrewPackage::from_split_spec(name, version).map(Self::Brew),
            _ => Err(Error::UnknownPackageKind(kind.to_string(), name.into())),
        }
    }

    pub fn brew_package(&self) -> Option<&BrewPackage> {
        if let Self::Brew(p) = self {
            Some(p)
        } else {
            None
        }
    }

    pub fn cargo_package(&self) -> Option<&CargoPackage> {
        if let Self::Cargo(p) = self {
            Some(p)
        } else {
            None
        }
    }

    pub fn is_brew(&self) -> bool {
        matches!(self, Self::Brew(_))
    }

    pub fn is_cargo(&self) -> bool {
        matches!(self, Self::Cargo(_))
    }
}

impl FromStr for PackageSpec {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, remain) = s
            .split_once(':')
            .ok_or_else(|| Error::InvalidPackageSpec(s.to_string(), "Expected <kind>:..."))?;
        let (name, version) = remain.split_once('@').unwrap_or((remain, ""));
        Self::from_split_spec(kind, name, version)
    }
}

impl fmt::Display for PackageSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageSpec::Brew(p) => fmt::Display::fmt(p, f),
            PackageSpec::Cargo(p) => fmt::Display::fmt(p, f),
        }
    }
}

pub struct PackageSpecSeed(pub SmolStr);

impl<'de> de::DeserializeSeed<'de> for PackageSpecSeed {
    type Value = PackageSpec;

    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<PackageSpec, D::Error> {
        struct Visitor(SmolStr);

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = PackageSpec;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "version string or package spec")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let (kind, version) = v.split_once(':').unwrap_or((v, ""));
                PackageSpec::from_split_spec(kind, self.0, version).map_err(de::Error::custom)
            }

            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
                enum Inner {
                    Cargo(StoredCargoPackage),
                    Brew(StoredBrewPackage),
                }

                Ok(
                    match de::Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?
                    {
                        Inner::Cargo(p) => PackageSpec::Cargo(p.into_package(self.0)),
                        Inner::Brew(p) => PackageSpec::Brew(p.into_package(self.0)),
                    },
                )
            }
        }

        d.deserialize_any(Visitor(self.0))
    }
}
