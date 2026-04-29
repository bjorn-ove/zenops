use std::fmt;

/// Resolves `${name}` placeholders to their values.
pub trait ExpandLookup {
    /// Write the value of `name` into `f`, or return
    /// [`ExpandLookupError::Unresolved`] without writing anything.
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut dyn fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>>;
}

/// Try each lookup in order; return the first hit.
impl<const SIZE: usize, T: ExpandLookup + ?Sized> ExpandLookup for [&T; SIZE] {
    fn write_value<'a>(
        &self,
        name: &'a str,
        f: &mut dyn fmt::Write,
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
        f: &mut dyn fmt::Write,
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
        f: &mut dyn fmt::Write,
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
        f: &mut dyn fmt::Write,
    ) -> Result<(), ExpandLookupError<'a>> {
        if let Some(value) = self.get(name) {
            f.write_str(value.as_ref())?;
            Ok(())
        } else {
            Err(ExpandLookupError::Unresolved(name))
        }
    }
}

/// Error returned from [`ExpandLookup::write_value`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandLookupError<'a> {
    /// `name` is not known to this lookup.
    #[error("Failed to resolve `${{{0}}}`")]
    Unresolved(&'a str),
    /// The [`fmt::Write`] sink returned an error.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_to_string<L: ExpandLookup>(lookup: &L, key: &str) -> Result<String, String> {
        let mut buf = String::new();
        lookup
            .write_value(key, &mut buf)
            .map(|()| buf)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn btreemap_writes_value_for_known_key() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("name", "alice");
        assert_eq!(write_to_string(&m, "name").unwrap(), "alice");
    }

    #[test]
    fn btreemap_returns_unresolved_for_missing_key() {
        let m: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
        let mut buf = String::new();
        let err = m.write_value("missing", &mut buf).unwrap_err();
        assert_eq!(err, ExpandLookupError::Unresolved("missing"));
        assert!(buf.is_empty(), "lookup must not write on miss");
    }

    #[test]
    fn hashmap_writes_value_for_known_key() {
        let mut m = std::collections::HashMap::new();
        m.insert("name", "alice");
        assert_eq!(write_to_string(&m, "name").unwrap(), "alice");
    }

    #[test]
    fn hashmap_returns_unresolved_for_missing_key() {
        let m: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        let mut buf = String::new();
        let err = m.write_value("missing", &mut buf).unwrap_err();
        assert_eq!(err, ExpandLookupError::Unresolved("missing"));
        assert!(buf.is_empty());
    }

    #[cfg(feature = "indexmap")]
    #[test]
    fn indexmap_writes_value_for_known_key() {
        let mut m = indexmap::IndexMap::new();
        m.insert("name", "alice");
        assert_eq!(write_to_string(&m, "name").unwrap(), "alice");
    }

    #[cfg(feature = "indexmap")]
    #[test]
    fn indexmap_returns_unresolved_for_missing_key() {
        let m: indexmap::IndexMap<&str, &str> = indexmap::IndexMap::new();
        let mut buf = String::new();
        let err = m.write_value("missing", &mut buf).unwrap_err();
        assert_eq!(err, ExpandLookupError::Unresolved("missing"));
        assert!(buf.is_empty());
    }

    #[test]
    fn array_chain_returns_first_hit() {
        let mut a = std::collections::BTreeMap::new();
        a.insert("k", "from_a");
        let mut b = std::collections::BTreeMap::new();
        b.insert("k", "from_b");

        let chain: [&dyn ExpandLookup; 2] = [&a, &b];
        let mut buf = String::new();
        chain.write_value("k", &mut buf).unwrap();
        assert_eq!(buf, "from_a");
    }

    #[test]
    fn array_chain_falls_through_to_second_lookup() {
        let a: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
        let mut b = std::collections::BTreeMap::new();
        b.insert("k", "from_b");

        let chain: [&dyn ExpandLookup; 2] = [&a, &b];
        let mut buf = String::new();
        chain.write_value("k", &mut buf).unwrap();
        assert_eq!(buf, "from_b");
    }

    #[test]
    fn array_chain_returns_unresolved_when_no_lookup_has_key() {
        let a: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
        let b: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();

        let chain: [&dyn ExpandLookup; 2] = [&a, &b];
        let mut buf = String::new();
        let err = chain.write_value("missing", &mut buf).unwrap_err();
        assert_eq!(err, ExpandLookupError::Unresolved("missing"));
    }
}
