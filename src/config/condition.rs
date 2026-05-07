//! `[conditions]` and `pkg.*.when`: named, composable predicates that gate
//! whether a pkg is considered relevant on the current host.
//!
//! A [`Condition`] is one of seven kinds — `os`, `shell`, `hostname`,
//! `file_exists`, `all`, `any`, `not` — represented as a single-key TOML
//! table, with the key matching the variant in snake_case. The custom
//! [`Deserialize`] impl rejects empty tables, unknown kinds, and tables with
//! more than one kind, with explicit error messages naming the offender.
//!
//! Children of `all` / `any` / `not` (and the value of `pkg.*.when`) are a
//! [`ConditionOrRef`]: either a bare TOML string referring to a named
//! condition, or an inline [`Condition`] table.
//!
//! Built-ins (`linux`, `macos`, `bash`, `zsh`) ship in
//! `condition_builtins.toml` and are deep-merged into the user's
//! `[conditions]` table at load; user entries with the same name win.
//!
//! [`Conditions::compile`] runs a DFS over every entry to validate that
//! every named reference resolves and that the graph is acyclic, before any
//! pkg gets evaluated. [`Conditions::evaluate`] walks an already-compiled
//! tree against a [`HostContext`].

use std::collections::HashSet;
use std::path::Path;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de;
use smol_str::SmolStr;

use zenops_expand::{ExpandError, ExpandStr};

use super::pkg::{Os, Shell};

/// Sorted by canonical reading order so error messages are stable.
const KINDS: &[&str] = &[
    "os",
    "shell",
    "hostname",
    "file_exists",
    "all",
    "any",
    "not",
];

/// A named or inline predicate. Each variant carries a single field whose
/// name matches the variant in snake_case — what the user typed in TOML.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Os { os: Os },
    Shell { shell: Shell },
    Hostname { hostname: HostnameRegex },
    FileExists { file_exists: ExpandStr },
    All { all: Vec<ConditionOrRef> },
    Any { any: Vec<ConditionOrRef> },
    Not { not: Box<ConditionOrRef> },
}

/// The shape used inside `all` / `any` / `not` and at every gateable site
/// (e.g. `pkg.*.when`). A bare TOML string resolves through the
/// [`Conditions`] registry; an inline table is used as-is.
#[derive(Debug, Clone, PartialEq)]
pub enum ConditionOrRef {
    Ref(SmolStr),
    Inline(Condition),
}

/// Regex newtype for hostname matching. Compiled at deserialize time so a
/// bad pattern fails the load, not the first evaluation. `PartialEq` falls
/// back to source-string comparison since `regex::Regex` isn't `PartialEq`.
#[derive(Debug, Clone)]
pub struct HostnameRegex {
    src: SmolStr,
    re: regex::Regex,
}

impl HostnameRegex {
    pub fn is_match(&self, s: &str) -> bool {
        self.re.is_match(s)
    }

    pub fn as_str(&self) -> &str {
        self.src.as_str()
    }
}

impl PartialEq for HostnameRegex {
    fn eq(&self, other: &Self) -> bool {
        self.src == other.src
    }
}

/// Compiled, ref-checked, cycle-free registry of named conditions.
#[derive(Debug, Clone, PartialEq)]
pub struct Conditions {
    by_name: IndexMap<SmolStr, Condition>,
}

/// Inputs the evaluator needs to resolve a condition against the current
/// host. Built once per `Config`, threaded into every pkg-level decision.
pub struct HostContext<'a> {
    pub os: Os,
    pub shell: Option<Shell>,
    pub hostname: &'a str,
    pub home: &'a Path,
    pub system_inputs: &'a IndexMap<SmolStr, SmolStr>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown condition '{name}' referenced from '{from}'")]
    UnknownRef { name: SmolStr, from: SmolStr },
    #[error("condition cycle: {path}")]
    Cycle { path: String },
    #[error("Failed to expand file_exists path in condition: {0}")]
    Expand(#[from] ExpandError),
    #[error("Failed to check if {0:?} exists: {1}")]
    ExistsFailed(String, std::io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UnknownRef { name: a, from: b }, Self::UnknownRef { name: c, from: d }) => {
                a == c && b == d
            }
            (Self::Cycle { path: a }, Self::Cycle { path: b }) => a == b,
            (Self::Expand(a), Self::Expand(b)) => a == b,
            (Self::ExistsFailed(a, b), Self::ExistsFailed(c, d)) => a == c && b.kind() == d.kind(),
            _ => false,
        }
    }
}

impl Conditions {
    /// Compile a deserialized `[conditions]` map: validate every named
    /// reference resolves and the dependency graph is acyclic.
    pub fn compile(by_name: IndexMap<SmolStr, Condition>) -> Result<Self, Error> {
        let mut validated: HashSet<SmolStr> = HashSet::new();
        for name in by_name.keys() {
            let mut stack: Vec<SmolStr> = Vec::new();
            walk(name, &by_name, &mut stack, &mut validated)?;
        }
        Ok(Self { by_name })
    }

    pub fn get(&self, name: &str) -> Option<&Condition> {
        self.by_name.get(name)
    }

    /// Evaluate a [`ConditionOrRef`] against the current host. The registry
    /// is the only source of named lookups; pkg-local overrides are
    /// intentionally not supported (conditions compose by being shared).
    pub fn evaluate(&self, cor: &ConditionOrRef, ctx: &HostContext<'_>) -> Result<bool, Error> {
        match cor {
            ConditionOrRef::Ref(name) => {
                let cond = self.by_name.get(name).ok_or_else(|| Error::UnknownRef {
                    name: name.clone(),
                    from: SmolStr::new_static("<runtime>"),
                })?;
                self.eval_condition(cond, ctx)
            }
            ConditionOrRef::Inline(c) => self.eval_condition(c, ctx),
        }
    }

    fn eval_condition(&self, c: &Condition, ctx: &HostContext<'_>) -> Result<bool, Error> {
        match c {
            Condition::Os { os } => Ok(*os == ctx.os),
            Condition::Shell { shell } => Ok(ctx.shell == Some(*shell)),
            Condition::Hostname { hostname } => Ok(hostname.is_match(ctx.hostname)),
            Condition::FileExists { file_exists } => {
                let expanded = file_exists.expand_to_string(ctx.system_inputs)?;
                let resolved = expanded.replacen('~', &ctx.home.to_string_lossy(), 1);
                Path::new(&resolved)
                    .try_exists()
                    .map_err(|e| Error::ExistsFailed(resolved, e))
            }
            Condition::All { all } => {
                for child in all {
                    if !self.evaluate(child, ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Condition::Any { any } => {
                for child in any {
                    if self.evaluate(child, ctx)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Condition::Not { not } => Ok(!self.evaluate(not, ctx)?),
        }
    }
}

fn walk(
    name: &SmolStr,
    by_name: &IndexMap<SmolStr, Condition>,
    stack: &mut Vec<SmolStr>,
    validated: &mut HashSet<SmolStr>,
) -> Result<(), Error> {
    if validated.contains(name) {
        return Ok(());
    }
    if let Some(cycle_start) = stack.iter().position(|n| n == name) {
        let mut path: Vec<&str> = stack[cycle_start..].iter().map(SmolStr::as_str).collect();
        path.push(name.as_str());
        return Err(Error::Cycle {
            path: path.join(" -> "),
        });
    }
    let Some(cond) = by_name.get(name) else {
        return Err(Error::UnknownRef {
            name: name.clone(),
            from: stack.last().cloned().unwrap_or_else(|| name.clone()),
        });
    };
    stack.push(name.clone());
    walk_condition(cond, by_name, stack, validated)?;
    stack.pop();
    validated.insert(name.clone());
    Ok(())
}

fn walk_condition(
    c: &Condition,
    by_name: &IndexMap<SmolStr, Condition>,
    stack: &mut Vec<SmolStr>,
    validated: &mut HashSet<SmolStr>,
) -> Result<(), Error> {
    match c {
        Condition::Os { .. }
        | Condition::Shell { .. }
        | Condition::Hostname { .. }
        | Condition::FileExists { .. } => Ok(()),
        Condition::All { all } => {
            for child in all {
                walk_or_ref(child, by_name, stack, validated)?;
            }
            Ok(())
        }
        Condition::Any { any } => {
            for child in any {
                walk_or_ref(child, by_name, stack, validated)?;
            }
            Ok(())
        }
        Condition::Not { not } => walk_or_ref(not, by_name, stack, validated),
    }
}

fn walk_or_ref(
    cor: &ConditionOrRef,
    by_name: &IndexMap<SmolStr, Condition>,
    stack: &mut Vec<SmolStr>,
    validated: &mut HashSet<SmolStr>,
) -> Result<(), Error> {
    match cor {
        ConditionOrRef::Ref(name) => walk(name, by_name, stack, validated),
        ConditionOrRef::Inline(c) => walk_condition(c, by_name, stack, validated),
    }
}

// ---------- Deserialize ----------

impl<'de> de::Deserialize<'de> for Condition {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Condition;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(
                    f,
                    "a condition table with exactly one of: {}",
                    KINDS.join(", ")
                )
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let key: String = match map.next_key()? {
                    Some(k) => k,
                    None => {
                        return Err(de::Error::custom(format!(
                            "empty condition table; expected exactly one of: {}",
                            KINDS.join(", ")
                        )));
                    }
                };
                let cond = match key.as_str() {
                    "os" => Condition::Os {
                        os: map.next_value()?,
                    },
                    "shell" => Condition::Shell {
                        shell: map.next_value()?,
                    },
                    "hostname" => Condition::Hostname {
                        hostname: map.next_value()?,
                    },
                    "file_exists" => Condition::FileExists {
                        file_exists: map.next_value()?,
                    },
                    "all" => Condition::All {
                        all: map.next_value()?,
                    },
                    "any" => Condition::Any {
                        any: map.next_value()?,
                    },
                    "not" => Condition::Not {
                        not: Box::new(map.next_value()?),
                    },
                    other => {
                        return Err(de::Error::custom(format!(
                            "unknown condition kind '{other}', expected one of: {}",
                            KINDS.join(", ")
                        )));
                    }
                };
                if let Some(extra) = map.next_key::<String>()? {
                    return Err(de::Error::custom(format!(
                        "condition must have exactly one kind, found '{key}' and '{extra}'"
                    )));
                }
                Ok(cond)
            }
        }

        d.deserialize_map(Visitor)
    }
}

impl<'de> de::Deserialize<'de> for ConditionOrRef {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = ConditionOrRef;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "a condition name (string) or an inline condition table")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ConditionOrRef::Ref(SmolStr::new(v)))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ConditionOrRef::Ref(SmolStr::new(v)))
            }

            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(ConditionOrRef::Ref(SmolStr::new(v)))
            }

            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let cond = Condition::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(ConditionOrRef::Inline(cond))
            }
        }

        d.deserialize_any(Visitor)
    }
}

impl<'de> de::Deserialize<'de> for HostnameRegex {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let src: SmolStr = SmolStr::deserialize(d)?;
        let re = regex::Regex::new(src.as_str()).map_err(de::Error::custom)?;
        Ok(Self { src, re })
    }
}

// ---------- JsonSchema ----------

impl schemars::JsonSchema for HostnameRegex {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "HostnameRegex".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "regex",
            "description": "Regular expression matched against the hostname.",
        })
    }
}

impl schemars::JsonSchema for Condition {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Condition".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let os = generator.subschema_for::<Os>();
        let shell = generator.subschema_for::<Shell>();
        let hostname = generator.subschema_for::<HostnameRegex>();
        let file_exists = generator.subschema_for::<ExpandStr>();
        let all_children = generator.subschema_for::<Vec<ConditionOrRef>>();
        let any_children = generator.subschema_for::<Vec<ConditionOrRef>>();
        let not_child = generator.subschema_for::<ConditionOrRef>();
        schemars::json_schema!({
            "description": "A predicate evaluated against the host. Exactly one kind per table.",
            "oneOf": [
                {
                    "type": "object",
                    "properties": { "os": os },
                    "required": ["os"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "shell": shell },
                    "required": ["shell"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "hostname": hostname },
                    "required": ["hostname"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "file_exists": file_exists },
                    "required": ["file_exists"],
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
                    "properties": { "any": any_children },
                    "required": ["any"],
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "properties": { "not": not_child },
                    "required": ["not"],
                    "additionalProperties": false,
                },
            ],
        })
    }
}

impl schemars::JsonSchema for ConditionOrRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ConditionOrRef".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let condition = generator.subschema_for::<Condition>();
        schemars::json_schema!({
            "description": "Either a name from `[conditions]` (string) or an inline condition (table).",
            "oneOf": [
                { "type": "string", "description": "Reference to a named condition." },
                condition,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use std::collections::HashMap;

    fn no_inputs() -> IndexMap<SmolStr, SmolStr> {
        IndexMap::new()
    }

    fn ctx<'a>(
        os: Os,
        shell: Option<Shell>,
        hostname: &'a str,
        home: &'a Path,
        sys: &'a IndexMap<SmolStr, SmolStr>,
    ) -> HostContext<'a> {
        HostContext {
            os,
            shell,
            hostname,
            home,
            system_inputs: sys,
        }
    }

    fn parse_conditions(toml_src: &str) -> IndexMap<SmolStr, Condition> {
        #[derive(serde::Deserialize)]
        struct Holder {
            conditions: IndexMap<SmolStr, Condition>,
        }
        let h: Holder = toml::from_str(toml_src).unwrap();
        h.conditions
    }

    #[test]
    fn os_variant_round_trips_and_evaluates() {
        let map = parse_conditions(
            r#"
            [conditions.linux]
            os = "linux"
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("linux"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, None, "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Macos, None, "", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn shell_variant_treats_unset_shell_as_no_match() {
        let map = parse_conditions(
            r#"
            [conditions.zsh]
            shell = "zsh"
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("zsh"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, Some(Shell::Zsh), "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Linux, Some(Shell::Bash), "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Linux, None, "", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn hostname_regex_matches_anchored_pattern() {
        let map = parse_conditions(
            r#"
            [conditions.work]
            hostname = "^work-.*"
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("work"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, None, "work-laptop", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Linux, None, "dev-box", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn file_exists_expands_tilde_and_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join(".marker");
        std::fs::write(&marker, "").unwrap();
        let map = parse_conditions(
            r#"
            [conditions.has]
            file_exists = "~/.marker"
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("has"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, None, "", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn all_combinator_uses_named_refs() {
        let map = parse_conditions(
            r#"
            [conditions.linux]
            os = "linux"
            [conditions.bash]
            shell = "bash"
            [conditions.linux_bash]
            all = ["linux", "bash"]
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("linux_bash"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, Some(Shell::Bash), "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Linux, Some(Shell::Zsh), "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Macos, Some(Shell::Bash), "", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn any_combinator_uses_inline_and_ref_mix() {
        let map = parse_conditions(
            r#"
            [conditions.linux]
            os = "linux"
            [conditions.either]
            any = ["linux", { shell = "zsh" }]
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("either"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, None, "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            regs.evaluate(&c, &ctx(Os::Macos, Some(Shell::Zsh), "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Macos, Some(Shell::Bash), "", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn not_combinator_inverts() {
        let map = parse_conditions(
            r#"
            [conditions.zsh]
            shell = "zsh"
            [conditions.not_zsh]
            not = "zsh"
        "#,
        );
        let regs = Conditions::compile(map).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        let c = ConditionOrRef::Ref(SmolStr::new_static("not_zsh"));
        assert!(
            regs.evaluate(&c, &ctx(Os::Linux, Some(Shell::Bash), "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&c, &ctx(Os::Linux, Some(Shell::Zsh), "", tmp.path(), &sys))
                .unwrap()
        );
    }

    #[test]
    fn unknown_kind_errors_with_helpful_message() {
        let err = toml::from_str::<HashMap<SmolStr, Condition>>(r#"x = { oops = 1 }"#)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unknown condition kind 'oops'"),
            "expected message about unknown kind, got: {err}"
        );
        assert!(
            err.contains("os, shell, hostname, file_exists, all, any, not"),
            "expected list of valid kinds, got: {err}"
        );
    }

    #[test]
    fn empty_kind_table_errors() {
        let err = toml::from_str::<HashMap<SmolStr, Condition>>("x = {}")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("empty condition table"),
            "expected empty-table error, got: {err}"
        );
    }

    #[test]
    fn multiple_kinds_error() {
        let err = toml::from_str::<HashMap<SmolStr, Condition>>(
            r#"x = { os = "linux", shell = "bash" }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("exactly one kind"),
            "expected multiple-kinds error, got: {err}"
        );
    }

    #[test]
    fn unknown_ref_caught_at_compile() {
        let map = parse_conditions(
            r#"
            [conditions.a]
            all = ["nope"]
        "#,
        );
        let err = Conditions::compile(map).unwrap_err();
        assert!(
            matches!(err, Error::UnknownRef { ref name, .. } if name.as_str() == "nope"),
            "expected UnknownRef(nope), got: {err:?}"
        );
    }

    #[test]
    fn cycle_caught_at_compile() {
        let map = parse_conditions(
            r#"
            [conditions.a]
            all = ["b"]
            [conditions.b]
            all = ["a"]
        "#,
        );
        let err = Conditions::compile(map).unwrap_err();
        match err {
            Error::Cycle { ref path } => {
                assert!(path.contains("a") && path.contains("b"), "got: {path}");
            }
            other => panic!("expected Cycle, got: {other:?}"),
        }
    }

    #[test]
    fn inline_when_with_no_registry_entry_works() {
        let regs = Conditions::compile(IndexMap::new()).unwrap();
        let cor: ConditionOrRef =
            toml::from_str::<HashMap<SmolStr, ConditionOrRef>>(r#"w = { os = "linux" }"#)
                .unwrap()
                .remove("w")
                .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let sys = no_inputs();
        assert!(
            regs.evaluate(&cor, &ctx(Os::Linux, None, "", tmp.path(), &sys))
                .unwrap()
        );
        assert!(
            !regs
                .evaluate(&cor, &ctx(Os::Macos, None, "", tmp.path(), &sys))
                .unwrap()
        );
    }
}
