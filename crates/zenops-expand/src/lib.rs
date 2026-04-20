mod expand_lookup;

use std::fmt;

use serde::Deserialize;
use smol_str::SmolStr;

pub use expand_lookup::ExpandLookup;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    #[error("unresolved placeholder `${{{0}}}`")]
    Unresolved(SmolStr),
    #[error(transparent)]
    WriteFmt(#[from] fmt::Error),
    #[error("unterminated `${{` in template")]
    Unterminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct ExpandStr(SmolStr);

impl ExpandStr {
    pub fn new(raw: SmolStr) -> Self {
        Self(raw)
    }

    pub fn new_static(raw: &'static str) -> Self {
        Self(SmolStr::new_static(raw))
    }

    pub fn expand_to_string(&self, lookup: &impl ExpandLookup) -> Result<String, ExpandError> {
        let mut out = String::with_capacity(self.0.len() * 2);
        self.write_expanded(lookup, &mut out)?;
        Ok(out)
    }

    pub fn write_expanded(
        &self,
        lookup: &impl ExpandLookup,
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
