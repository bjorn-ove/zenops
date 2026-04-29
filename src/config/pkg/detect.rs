//! The `detect` configuration language: leaf checks (`File`, `Which`) and
//! combinators (`Any`, `All`) that compose them, with optional per-strategy
//! OS gating. Used by [`super::PkgConfig`] to decide whether a configured
//! pkg is installed on the current host.

use std::path::Path;

use zenops_expand::{ExpandLookup, ExpandStr};

use super::Os;

/// A detect strategy wraps a concrete check (`kind`) with an optional OS gate.
/// When `os` is non-empty and doesn't include the current OS, `check()`
/// short-circuits to `false` — the strategy is treated as a miss on that host.
#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
pub struct DetectStrategy {
    #[serde(default)]
    pub os: Vec<Os>,
    #[serde(flatten)]
    pub kind: DetectKind,
}

/// Concrete detect checks. `File` and `Which` are leaves; `Any` and `All` are
/// combinators that let a single `detect` field express arbitrary boolean
/// logic by nesting other strategies.
#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DetectKind {
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
}

impl DetectStrategy {
    /// Apply the OS gate first, then delegate to the kind. Unresolved
    /// `${var}` placeholders inside the leaf checks also yield `false`.
    pub fn check(&self, home: &Path, lookup: &impl ExpandLookup) -> bool {
        if !self.os.is_empty() {
            let Some(cur) = Os::current() else {
                return false;
            };
            if !self.os.contains(&cur) {
                return false;
            }
        }
        self.kind.check(home, lookup)
    }
}

impl DetectKind {
    pub fn check(&self, home: &Path, lookup: &impl ExpandLookup) -> bool {
        match self {
            Self::File { path } => {
                let Ok(expanded) = path.expand_to_string(lookup) else {
                    return false;
                };
                let resolved = expanded.replacen('~', &home.to_string_lossy(), 1);
                Path::new(&resolved).exists()
            }
            Self::Which { binary } => match binary.expand_to_string(lookup) {
                Ok(b) => which_on_path(&b),
                Err(_) => false,
            },
            Self::Any { of } => of.iter().any(|s| s.check(home, lookup)),
            Self::All { of } => of.iter().all(|s| s.check(home, lookup)),
        }
    }
}

impl std::fmt::Display for DetectStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.os.is_empty() {
            let names: Vec<&'static str> = self
                .os
                .iter()
                .map(|o| match o {
                    Os::Linux => "linux",
                    Os::Macos => "macos",
                })
                .collect();
            write!(f, "[os={}] ", names.join(","))?;
        }
        write!(f, "{}", self.kind)
    }
}

impl std::fmt::Display for DetectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(f, "{}", path.as_template()),
            Self::Which { binary } => write!(f, "which {}", binary.as_template()),
            Self::Any { of } => write_combinator(f, "any", of),
            Self::All { of } => write_combinator(f, "all", of),
        }
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

pub(crate) fn which_on_path(binary: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| Path::new(dir).join(binary).is_file())
}
