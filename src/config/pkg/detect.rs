//! The `detect` configuration language: leaf checks (`File`, `Which`) and
//! combinators (`Any`, `All`) that compose them. Used by [`super::PkgConfig`]
//! to decide whether a configured pkg is installed on the current host.
//! Host-level gating (OS, shell, hostname, …) lives in `pkg.*.when` and the
//! shared `[conditions]` registry — not here.

use std::path::Path;

use zenops_expand::{ExpandLookup, ExpandStr};

use super::error::Error;

/// A detect strategy. `File` and `Which` are leaves; `Any` and `All` are
/// combinators that let a single `detect` field express arbitrary boolean
/// logic by nesting other strategies.
#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
}

impl DetectStrategy {
    /// Run the check. Unresolved `${var}` placeholders inside leaf checks
    /// yield `false` rather than an error.
    pub fn check(&self, home: &Path, lookup: &impl ExpandLookup) -> Result<bool, Error> {
        match self {
            Self::File { path } => {
                let Ok(expanded) = path.expand_to_string(lookup) else {
                    return Ok(false);
                };
                let resolved = expanded.replacen('~', &home.to_string_lossy(), 1);
                Path::new(&resolved)
                    .try_exists()
                    .map_err(|e| Error::ExistsFailed(resolved, e))
            }
            Self::Which { binary } => Ok(crate::utils::which::expand_and_exists(binary, lookup)?),
            Self::Any { of } => {
                for s in of {
                    if s.check(home, lookup)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::All { of } => {
                for s in of {
                    if !s.check(home, lookup)? {
                        return Ok(false);
                    }
                }
                Ok(true)
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
