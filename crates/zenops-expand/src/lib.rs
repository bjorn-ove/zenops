#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

//! String newtype that carries `${name}` placeholders which must be expanded
//! before use.
//!
//! [`ExpandStr`] deliberately does not implement `Display`, `AsRef<str>`, or
//! `Deref<str>` — callers go through [`ExpandStr::expand_to_string`] or
//! [`ExpandStr::write_expanded`] with an [`ExpandLookup`]. `${name}` is the
//! only placeholder syntax; there is no escape sequence.
//!
//! # Features
//!
//! - `indexmap` — [`ExpandLookup`] impl for [`indexmap::IndexMap`].

mod expand_lookup;

use std::fmt;

use serde::Deserialize;
use smol_str::SmolStr;

pub use expand_lookup::{ExpandLookup, ExpandLookupError};

/// Error returned from expanding an [`ExpandStr`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    /// A `${name}` placeholder was not resolved by the lookup.
    #[error("unresolved placeholder `${{{0}}}`")]
    Unresolved(SmolStr),
    /// The [`fmt::Write`] sink returned an error.
    #[error(transparent)]
    WriteFmt(#[from] fmt::Error),
    /// A `${` sequence was never closed by `}`.
    #[error("unterminated `${{` in template")]
    Unterminated,
}

/// A template string containing `${name}` placeholders.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct ExpandStr(SmolStr);

impl ExpandStr {
    /// Wrap a template string.
    pub fn new(raw: SmolStr) -> Self {
        Self(raw)
    }

    /// Wrap a `'static` template string without allocating.
    pub fn new_static(raw: &'static str) -> Self {
        Self(SmolStr::new_static(raw))
    }

    /// Expand the template into a new [`String`].
    ///
    /// Each `${name}` is replaced with the value `lookup` writes for that
    /// name. Literal characters pass through unchanged.
    ///
    /// # Example
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use zenops_expand::ExpandStr;
    ///
    /// let t = ExpandStr::new_static("${greeting}, ${name}!");
    ///
    /// let mut lookup: HashMap<&str, &str> = HashMap::new();
    /// lookup.insert("greeting", "hi");
    /// lookup.insert("name", "Ada");
    ///
    /// assert_eq!(t.expand_to_string(&lookup).unwrap(), "hi, Ada!");
    /// ```
    pub fn expand_to_string(
        &self,
        lookup: &(impl ExpandLookup + ?Sized),
    ) -> Result<String, ExpandError> {
        let mut out = String::with_capacity(self.0.len() * 2);
        self.write_expanded(lookup, &mut out)?;
        Ok(out)
    }

    /// Expand the template into an existing [`fmt::Write`] sink.
    ///
    /// Equivalent to [`expand_to_string`] but writes into a caller-supplied
    /// buffer, so multiple templates can be concatenated without
    /// intermediate allocations. On error the sink may have been written
    /// to partially.
    ///
    /// # Example
    ///
    /// ```
    /// use std::collections::HashMap;
    /// use std::fmt::Write;
    /// use zenops_expand::ExpandStr;
    ///
    /// let mut lookup: HashMap<&str, &str> = HashMap::new();
    /// lookup.insert("user", "ada");
    ///
    /// let mut out = String::from("path=");
    /// let t = ExpandStr::new_static("/home/${user}");
    /// t.write_expanded(&lookup, &mut out).unwrap();
    /// write!(out, ";").unwrap();
    ///
    /// assert_eq!(out, "path=/home/ada;");
    /// ```
    ///
    /// [`expand_to_string`]: ExpandStr::expand_to_string
    pub fn write_expanded(
        &self,
        lookup: &(impl ExpandLookup + ?Sized),
        f: &mut impl fmt::Write,
    ) -> Result<(), ExpandError> {
        let mut rest = self.0.as_str();
        while let Some(start) = rest.find("${") {
            f.write_str(&rest[..start])?;
            let after_open = &rest[start + 2..];
            let end = after_open.find('}').ok_or(ExpandError::Unterminated)?;
            let name = &after_open[..end];
            lookup.write_value(name, f)?;
            rest = &after_open[end + 1..];
        }
        f.write_str(rest)?;
        Ok(())
    }

    /// Get the raw template string.
    pub fn as_template(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_lookup(pairs: &[(&str, &str)]) -> HashMap<String, SmolStr> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), SmolStr::new(*v)))
            .collect()
    }

    #[test]
    fn literal_passthrough() {
        let s = ExpandStr::new_static("plain text");
        assert_eq!(s.expand_to_string(&make_lookup(&[])).unwrap(), "plain text");
    }

    #[test]
    fn resolves_single() {
        let s = ExpandStr::new_static("hello ${name}!");
        let m = make_lookup(&[("name", "world")]);
        assert_eq!(s.expand_to_string(&m).unwrap(), "hello world!");
    }

    #[test]
    fn resolves_adjacent_and_repeated() {
        let s = ExpandStr::new_static("${a}${b}-${a}");
        let m = make_lookup(&[("a", "X"), ("b", "Y")]);
        assert_eq!(s.expand_to_string(&m).unwrap(), "XY-X");
    }

    #[test]
    fn resolves_at_boundaries() {
        let s = ExpandStr::new_static("${a}");
        let m = make_lookup(&[("a", "A")]);
        assert_eq!(s.expand_to_string(&m).unwrap(), "A");
    }

    #[test]
    fn unresolved_key_errors() {
        let s = ExpandStr::new_static("a ${missing} b");
        let m = make_lookup(&[]);
        assert_eq!(
            s.expand_to_string(&m),
            Err(ExpandError::Unresolved(SmolStr::new_static("missing"))),
        );
    }

    #[test]
    fn unterminated_errors() {
        let s = ExpandStr::new_static("a ${oops");
        let m = make_lookup(&[]);
        assert_eq!(s.expand_to_string(&m), Err(ExpandError::Unterminated));
    }

    #[test]
    fn deserializes_from_toml_string() {
        #[derive(Deserialize)]
        struct Holder {
            v: ExpandStr,
        }
        let h: Holder = toml::from_str(r#"v = "x-${y}-z""#).unwrap();
        assert_eq!(h.v.as_template(), "x-${y}-z");
    }

    #[test]
    fn dyn_compatible() {
        let a = make_lookup(&[("a", "A")]);
        let b = make_lookup(&[("b", "B")]);

        // &dyn ExpandLookup accepted directly.
        let dyn_lookup: &dyn ExpandLookup = &a;
        let s = ExpandStr::new_static("${a}");
        assert_eq!(s.expand_to_string(dyn_lookup).unwrap(), "A");

        // Heterogeneous chain via [&dyn ExpandLookup; N].
        let chain: [&dyn ExpandLookup; 2] = [&a, &b];
        let s = ExpandStr::new_static("${a}/${b}");
        assert_eq!(s.expand_to_string(&chain).unwrap(), "A/B");
    }

    #[test]
    fn array_lookup_falls_through_and_propagates_unresolved() {
        let primary = make_lookup(&[("a", "from-primary")]);
        let fallback = make_lookup(&[("b", "from-fallback")]);
        let chain = [&primary, &fallback];

        let s = ExpandStr::new_static("${a}/${b}");
        assert_eq!(
            s.expand_to_string(&chain).unwrap(),
            "from-primary/from-fallback"
        );

        // If nothing in the chain resolves, the final result must be
        // Unresolved — not a silent empty expansion.
        let s = ExpandStr::new_static("${missing}");
        assert_eq!(
            s.expand_to_string(&chain),
            Err(ExpandError::Unresolved(SmolStr::new_static("missing"))),
        );
    }
}
