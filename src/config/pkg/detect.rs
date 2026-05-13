//! The `detect` configuration language: leaf checks (`File`, `Which`),
//! combinators (`Any`, `All`), and a host gate (`When`) that compose them.
//! Used by [`super::PkgConfig`] to decide whether a configured pkg is
//! installed on the current host. Pkg-level host gating still lives on
//! `pkg.*.when` (see [`super::PkgConfig::when`]); `When` here is the
//! detect-local form that lets a single pkg express OS-divergent detect
//! paths without splitting into two `[pkg.*]` entries.
//!
//! Each variant is identified on the wire by which key is present in the
//! table: `exists` → [`File`], `which` → [`Which`], `any` → [`Any`],
//! `all` → [`All`], `when` (paired with `then`) → [`When`]. The custom
//! [`Deserialize`] impl rejects empty tables, unknown kinds, and tables
//! that mix kinds, with explicit error messages naming the offender.
//!
//! [`File`]: DetectStrategy::File
//! [`Which`]: DetectStrategy::Which
//! [`Any`]: DetectStrategy::Any
//! [`All`]: DetectStrategy::All
//! [`When`]: DetectStrategy::When

use serde::de;
use zenops_expand::{ExpandLookup, ExpandStr};

use super::error::Error;
use crate::config::condition::{ConditionOrRef, Conditions, HostContext};

/// Sorted by canonical reading order so error messages are stable.
const KINDS: &[&str] = &["exists", "which", "any", "all", "when"];

/// A detect strategy. `File` and `Which` are leaves; `Any` and `All` are
/// combinators that compose other strategies; `When` gates a subtree on a
/// host-level condition. See the module docs for the wire-format mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum DetectStrategy {
    File {
        path: ExpandStr,
    },
    Which {
        binary: ExpandStr,
    },
    /// Matches when **any** child strategy matches (short-circuits).
    Any {
        of: Vec<DetectStrategy>,
    },
    /// Matches when **every** child strategy matches. An empty `of` is
    /// vacuously true — callers should prefer omitting the pkg's `detect`
    /// field entirely to express "no check required".
    All {
        of: Vec<DetectStrategy>,
    },
    /// Gates a subtree on a host-level condition. Evaluates as `false`
    /// (never "skip / filter") when `when` fails, so that an `All` whose
    /// children are all gated to a different host correctly evaluates as
    /// `false` rather than empty-set-vacuously `true`.
    When {
        when: ConditionOrRef,
        then: Box<DetectStrategy>,
    },
}

impl DetectStrategy {
    /// Run the check. Unresolved `${var}` placeholders inside leaf checks
    /// yield `false` rather than an error. A `When` whose condition fails
    /// also yields `false` — see the variant docs.
    pub fn check(
        &self,
        conditions: &Conditions,
        ctx: &HostContext<'_>,
        lookup: &impl ExpandLookup,
    ) -> Result<bool, Error> {
        match self {
            Self::File { path } => {
                let Ok(expanded) = path.expand_to_string(lookup) else {
                    return Ok(false);
                };
                let resolved = expanded.replacen('~', &ctx.home.to_string_lossy(), 1);
                std::path::Path::new(&resolved)
                    .try_exists()
                    .map_err(|e| Error::ExistsFailed(resolved, e))
            }
            Self::Which { binary } => Ok(crate::utils::which::expand_and_exists(binary, lookup)?),
            Self::Any { of } => {
                for s in of {
                    if s.check(conditions, ctx, lookup)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::All { of } => {
                for s in of {
                    if !s.check(conditions, ctx, lookup)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::When { when, then } => {
                if !conditions.evaluate(when, ctx)? {
                    return Ok(false);
                }
                then.check(conditions, ctx, lookup)
            }
        }
    }
}

impl std::fmt::Display for DetectStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(f, "{}", path.as_template()),
            Self::Which { binary } => write!(f, "which {}", binary.as_template()),
            Self::Any { of } => write_combinator(f, "any", of),
            Self::All { of } => write_combinator(f, "all", of),
            Self::When { when, then } => write!(f, "when({}, {then})", format_when(when)),
        }
    }
}

fn format_when(cor: &ConditionOrRef) -> String {
    match cor {
        ConditionOrRef::Ref(name) => name.to_string(),
        ConditionOrRef::Inline(c) => format!("{c:?}"),
    }
}

fn write_combinator(
    f: &mut std::fmt::Formatter<'_>,
    name: &str,
    of: &[DetectStrategy],
) -> std::fmt::Result {
    write!(f, "{name}(")?;
    for (i, s) in of.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{s}")?;
    }
    write!(f, ")")
}

// ---------- Deserialize ----------

impl<'de> de::Deserialize<'de> for DetectStrategy {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = DetectStrategy;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(
                    f,
                    "a detect table with exactly one of: {} (or `when` paired with `then`)",
                    KINDS.join(", ")
                )
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let first_key: String = match map.next_key()? {
                    Some(k) => k,
                    None => {
                        return Err(de::Error::custom(format!(
                            "empty detect table; expected exactly one of: {}",
                            KINDS.join(", ")
                        )));
                    }
                };

                match first_key.as_str() {
                    "exists" => {
                        let path: ExpandStr = map.next_value()?;
                        forbid_extra_keys(&mut map, &first_key)?;
                        Ok(DetectStrategy::File { path })
                    }
                    "which" => {
                        let binary: ExpandStr = map.next_value()?;
                        forbid_extra_keys(&mut map, &first_key)?;
                        Ok(DetectStrategy::Which { binary })
                    }
                    "any" => {
                        let of: Vec<DetectStrategy> = map.next_value()?;
                        forbid_extra_keys(&mut map, &first_key)?;
                        Ok(DetectStrategy::Any { of })
                    }
                    "all" => {
                        let of: Vec<DetectStrategy> = map.next_value()?;
                        forbid_extra_keys(&mut map, &first_key)?;
                        Ok(DetectStrategy::All { of })
                    }
                    "when" | "then" => {
                        let mut when_val: Option<ConditionOrRef> = None;
                        let mut then_val: Option<Box<DetectStrategy>> = None;

                        match first_key.as_str() {
                            "when" => when_val = Some(map.next_value()?),
                            "then" => then_val = Some(Box::new(map.next_value()?)),
                            _ => unreachable!(),
                        }

                        while let Some(k) = map.next_key::<String>()? {
                            match k.as_str() {
                                "when" if when_val.is_some() => {
                                    return Err(de::Error::custom(
                                        "`when` strategy has duplicate `when` key",
                                    ));
                                }
                                "when" => when_val = Some(map.next_value()?),
                                "then" if then_val.is_some() => {
                                    return Err(de::Error::custom(
                                        "`when` strategy has duplicate `then` key",
                                    ));
                                }
                                "then" => then_val = Some(Box::new(map.next_value()?)),
                                other => {
                                    return Err(de::Error::custom(format!(
                                        "`when` strategy has unexpected key '{other}'; expected exactly `when` and `then`"
                                    )));
                                }
                            }
                        }

                        let when = when_val.ok_or_else(|| {
                            de::Error::custom(
                                "found `then` without a matching `when`; a detect strategy with `then` must also have `when`",
                            )
                        })?;
                        let then = then_val.ok_or_else(|| {
                            de::Error::custom("`when` strategy is missing the `then` key")
                        })?;
                        Ok(DetectStrategy::When { when, then })
                    }
                    other => Err(de::Error::custom(format!(
                        "unknown detect kind '{other}', expected one of: {}",
                        KINDS.join(", ")
                    ))),
                }
            }
        }

        d.deserialize_map(Visitor)
    }
}

fn forbid_extra_keys<'de, A: de::MapAccess<'de>>(map: &mut A, key: &str) -> Result<(), A::Error> {
    if let Some(extra) = map.next_key::<String>()? {
        return Err(de::Error::custom(format!(
            "detect must have exactly one kind, found '{key}' and '{extra}'"
        )));
    }
    Ok(())
}

// ---------- JsonSchema ----------

impl schemars::JsonSchema for DetectStrategy {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DetectStrategy".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let exists = generator.subschema_for::<ExpandStr>();
        let which = generator.subschema_for::<ExpandStr>();
        let any_children = generator.subschema_for::<Vec<DetectStrategy>>();
        let all_children = generator.subschema_for::<Vec<DetectStrategy>>();
        let when_cond = generator.subschema_for::<ConditionOrRef>();
        let then_child = generator.subschema_for::<DetectStrategy>();
        schemars::json_schema!({
            "description": "A check for whether a pkg is installed. Exactly one kind per table; `when` additionally requires `then`.",
            "oneOf": [
                {
                    "type": "object",
                    "properties": { "exists": exists },
                    "required": ["exists"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "which": which },
                    "required": ["which"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "any": any_children },
                    "required": ["any"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "all": all_children },
                    "required": ["all"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "when": when_cond, "then": then_child },
                    "required": ["when", "then"],
                    "additionalProperties": false,
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_err(toml_src: &str) -> String {
        toml::from_str::<DetectStrategy>(toml_src)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn unknown_kind_lists_valid_kinds() {
        let err = parse_err(r#"nonsense = "x""#);
        assert!(
            err.contains("unknown detect kind 'nonsense'"),
            "expected unknown-kind message, got: {err}"
        );
        for kind in KINDS {
            assert!(err.contains(kind), "expected '{kind}' in error, got: {err}");
        }
    }

    #[test]
    fn empty_table_errors() {
        let err = parse_err("");
        assert!(
            err.contains("empty detect table"),
            "expected empty-table message, got: {err}"
        );
    }

    #[test]
    fn multiple_kinds_error_names_both() {
        let err = parse_err(
            r#"
                exists = "/x"
                which = "y"
            "#,
        );
        assert!(
            err.contains("exactly one kind"),
            "expected 'exactly one kind' message, got: {err}"
        );
        assert!(
            err.contains("'exists'") && err.contains("'which'"),
            "expected both keys to appear in error, got: {err}"
        );
    }

    #[test]
    fn when_missing_then_errors() {
        let err = parse_err(r#"when = "macos""#);
        assert!(
            err.contains("missing the `then` key"),
            "expected missing-then message, got: {err}"
        );
    }

    #[test]
    fn when_with_unexpected_extra_key_errors() {
        let err = parse_err(
            r#"
                when = "macos"
                then = { exists = "/x" }
                bogus = 1
            "#,
        );
        assert!(
            err.contains("'bogus'"),
            "expected error to name 'bogus', got: {err}"
        );
    }

    #[test]
    fn when_then_reversed_works() {
        let reversed: DetectStrategy = toml::from_str(
            r#"
                then = { exists = "/x" }
                when = "macos"
            "#,
        )
        .unwrap();
        let canonical: DetectStrategy = toml::from_str(
            r#"
                when = "macos"
                then = { exists = "/x" }
            "#,
        )
        .unwrap();
        assert_eq!(reversed, canonical);
    }

    #[test]
    fn then_without_when_errors() {
        let err = parse_err(r#"then = { exists = "/x" }"#);
        assert!(
            err.contains("`then` without a matching `when`"),
            "expected then-without-when message, got: {err}"
        );
    }

    #[test]
    fn rejects_legacy_tagged_form() {
        // Migration guard: the old `type = "..."` shape must fail with a
        // diagnostic that names the new keys, so users see how to update.
        let err = parse_err(
            r#"
                type = "file"
                path = "/x"
            "#,
        );
        assert!(
            err.contains("unknown detect kind 'type'"),
            "expected error to name 'type' as unknown, got: {err}"
        );
    }

    #[test]
    fn duplicate_when_errors() {
        let err = parse_err(
            r#"
                when = "macos"
                when = "linux"
            "#,
        );
        // TOML's own parser rejects the duplicate key before we see it;
        // either path is acceptable — the user gets a clear diagnostic.
        assert!(
            err.contains("duplicate") || err.contains("redefin"),
            "expected duplicate-key message, got: {err}"
        );
    }
}
