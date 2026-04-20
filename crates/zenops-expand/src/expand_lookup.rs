use std::fmt;

pub trait ExpandLookup {
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut impl fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>>;
}

impl<const SIZE: usize, T: ExpandLookup> ExpandLookup for [&T; SIZE] {
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut impl fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>> {
        for expander in self {
            match expander.write_value(name, f) {
                Ok(()) => return Ok(()),
                Err(ExpandLookupError::Unresolved(_)) => continue,
                Err(ExpandLookupError::WriteFmt(e)) => return Err(ExpandLookupError::WriteFmt(e)),
            }
        }
        Err(ExpandLookupError::Unresolved(name))
    }
}

#[cfg(feature = "indexmap")]
impl<K, V> ExpandLookup for indexmap::IndexMap<K, V>
where
    K: std::borrow::Borrow<str>,
    V: AsRef<str>,
{
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut impl fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>> {
        if let Some(value) = self.get(name) {
            f.write_str(value.as_ref())?;
            Ok(())
        } else {
            Err(ExpandLookupError::Unresolved(name))
        }
    }
}

impl<K, V> ExpandLookup for std::collections::BTreeMap<K, V>
where
    K: Ord + std::borrow::Borrow<str>,
    V: AsRef<str>,
{
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut impl fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>> {
        if let Some(value) = self.get(name) {
            f.write_str(value.as_ref())?;
            Ok(())
        } else {
            Err(ExpandLookupError::Unresolved(name))
        }
    }
}

impl<K, V> ExpandLookup for std::collections::HashMap<K, V>
where
    K: Eq + std::hash::Hash + std::borrow::Borrow<str>,
    V: AsRef<str>,
{
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut impl fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>> {
        if let Some(value) = self.get(name) {
            f.write_str(value.as_ref())?;
            Ok(())
        } else {
            Err(ExpandLookupError::Unresolved(name))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandLookupError<'a> {
    #[error("Failed to resolve `${{{0}}}`")]
    Unresolved(&'a str),
    #[error(transparent)]
    WriteFmt(#[from] fmt::Error),
}

impl<'a> From<ExpandLookupError<'a>> for crate::ExpandError {
    fn from(value: ExpandLookupError<'a>) -> Self {
        match value {
            ExpandLookupError::Unresolved(name) => crate::ExpandError::Unresolved(name.into()),
            ExpandLookupError::WriteFmt(e) => crate::ExpandError::WriteFmt(e),
        }
    }
}
