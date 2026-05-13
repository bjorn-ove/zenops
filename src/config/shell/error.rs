//! `config::shell`-scoped error type.
//!
//! Wraps the failure modes that surface while expanding `${...}`
//! placeholders inside a pkg's shell-init templates. Exposed to the rest
//! of the crate as `crate::Error::ConfigShell` via `#[error(transparent)]`
//! + `#[from]`.

use smol_str::SmolStr;

/// Failure modes for the shell-init `write_pkg_inits` path and the
/// helpers it drives. Both variants name the offending pkg so the user
/// can jump straight to its `[pkg.<name>]` block.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A pkg's template referenced an input that's not declared anywhere
    /// (no system input, no `[pkg.<name>.inputs]` entry).
    #[error(
        "Package {pkg} references undefined input {input}; mark the action optional or set [pkg.{pkg}.inputs].{input}"
    )]
    UnresolvedInput {
        /// The pkg whose template failed to resolve.
        pkg: SmolStr,
        /// The input name that wasn't found.
        input: SmolStr,
    },
    /// A pkg template contains a `${` with no matching `}`.
    #[error("Package {pkg} has an unterminated `${{` in a template")]
    TemplateUnterminated {
        /// The pkg whose template failed to parse.
        pkg: SmolStr,
    },
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::UnresolvedInput {
                    pkg: l_pkg,
                    input: l_input,
                },
                Self::UnresolvedInput {
                    pkg: r_pkg,
                    input: r_input,
                },
            ) => l_pkg == r_pkg && l_input == r_input,
            (Self::TemplateUnterminated { pkg: l }, Self::TemplateUnterminated { pkg: r }) => {
                l == r
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use similar_asserts::assert_eq;

    use super::*;

    #[test]
    fn unresolved_input_eq_and_ne() {
        let a = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("i"),
        };
        let b = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("i"),
        };
        let c = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("other"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn template_unterminated_eq_and_ne() {
        let a = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("p"),
        };
        let b = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("p"),
        };
        let c = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("q"),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn cross_variant_compare_returns_false() {
        let u = Error::UnresolvedInput {
            pkg: SmolStr::new_static("p"),
            input: SmolStr::new_static("i"),
        };
        let t = Error::TemplateUnterminated {
            pkg: SmolStr::new_static("p"),
        };
        assert_ne!(u, t);
    }
}
