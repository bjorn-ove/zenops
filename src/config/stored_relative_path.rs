use std::fmt;

use serde::de;

use zenops_safe_relative_path::{SafeRelativePath, SafeRelativePathBuf};

/// Represents a relative path that cannot leave its parent directory,
/// unless there is filesystem shenanigans (e.g. symlinks).
#[derive(Clone, Debug)]
pub(super) struct StoredRelativePath {
    /// The original path, as written in the config, for display and serialization purposes
    org: String,
    /// The normalized path
    normal: SafeRelativePathBuf,
}

impl StoredRelativePath {
    /// Returns the unique part to use when implementing traits
    /// NOTE: self.normal is generated from self.org and can't be otherwise modified, so no need to include it
    const fn unique_part(&self) -> &String {
        &self.org
    }
}

impl PartialEq for StoredRelativePath {
    fn eq(&self, other: &Self) -> bool {
        self.unique_part() == other.unique_part()
    }
}

impl Eq for StoredRelativePath {}

impl std::hash::Hash for StoredRelativePath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.unique_part().hash(state);
    }
}

impl fmt::Display for StoredRelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.org, f)
    }
}

impl std::ops::Deref for StoredRelativePath {
    type Target = SafeRelativePath;

    fn deref(&self) -> &Self::Target {
        self.normal.as_ref()
    }
}

impl AsRef<SafeRelativePath> for StoredRelativePath {
    fn as_ref(&self) -> &SafeRelativePath {
        self.normal.as_ref()
    }
}

impl schemars::JsonSchema for StoredRelativePath {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "StoredRelativePath".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Relative path under the zenops config repo; normalized after parse, `..` traversal rejected.",
        })
    }
}

impl<'de> de::Deserialize<'de> for StoredRelativePath {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StoredRelativePath;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "version string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StoredRelativePath {
                    org: v.to_string(),
                    normal: SafeRelativePath::from_relative_path(v)
                        .map_err(de::Error::custom)?
                        .normalize_safe(),
                })
            }
        }

        d.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use serde::Deserialize;
    use similar_asserts::assert_eq;

    use super::*;

    #[derive(Deserialize)]
    struct W {
        p: StoredRelativePath,
    }

    fn parse(p: &str) -> Result<StoredRelativePath, toml::de::Error> {
        toml::from_str::<W>(&format!("p = \"{p}\"")).map(|w| w.p)
    }

    fn hash_of(p: &StoredRelativePath) -> u64 {
        let mut h = DefaultHasher::new();
        p.hash(&mut h);
        h.finish()
    }

    #[test]
    fn deserialize_accepts_normal_path() {
        let got = parse("configs/app").unwrap();
        assert_eq!(got.to_string(), "configs/app");
    }

    #[test]
    fn deserialize_rejects_traversal() {
        let err = parse("../escape").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("outside") || msg.contains("parent"),
            "unexpected error: {msg}",
        );
    }

    #[test]
    fn display_returns_original_text_not_normalized_form() {
        let got = parse("./foo").unwrap();
        assert_eq!(got.to_string(), "./foo");
        assert_eq!(got.as_ref().as_str(), "foo");
    }

    #[test]
    fn deref_returns_normalized_safe_relative_path() {
        let got = parse("a/./b").unwrap();
        let derefed: &SafeRelativePath = &got;
        assert_eq!(derefed.as_str(), "a/b");
    }

    #[test]
    fn partial_eq_uses_original_text_not_normalized() {
        let a = parse("./foo").unwrap();
        let b = parse("foo").unwrap();
        assert_eq!(a.as_ref().as_str(), b.as_ref().as_str());
        assert_ne!(a, b);
    }

    #[test]
    fn equal_originals_compare_equal_and_hash_equal() {
        let a = parse("foo/bar").unwrap();
        let b = parse("foo/bar").unwrap();
        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn json_schema_name_is_stable() {
        use schemars::JsonSchema;
        assert_eq!(StoredRelativePath::schema_name(), "StoredRelativePath");
    }

    #[test]
    fn json_schema_describes_string_with_traversal_note() {
        use schemars::{JsonSchema, SchemaGenerator};
        let mut gen_ = SchemaGenerator::default();
        let schema = StoredRelativePath::json_schema(&mut gen_);
        let v = schema.as_value();
        assert_eq!(v["type"], "string");
        assert!(
            v["description"]
                .as_str()
                .is_some_and(|s| s.contains("traversal")),
            "description missing traversal note: {v}",
        );
    }
}
