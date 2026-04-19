use relative_path::{Component, RelativePath};

pub fn is_safe_relative_path(path: impl AsRef<RelativePath>) -> bool {
    path.as_ref()
        .components()
        .all(|c| matches!(c, Component::CurDir | Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_safe_paths() {
        assert!(is_safe_relative_path("foo/bar"));
        assert!(is_safe_relative_path("./foo/bar"));
        assert!(is_safe_relative_path(""));
        assert!(is_safe_relative_path("."));
    }

    #[test]
    fn test_validate_unsafe_paths() {
        assert!(!is_safe_relative_path("../foo"));
        assert!(!is_safe_relative_path("foo/../bar"));
        assert!(!is_safe_relative_path("foo/../../bar"));
        assert!(!is_safe_relative_path("foo/../../foo/bar"));
        assert!(!is_safe_relative_path(".."));
        assert!(!is_safe_relative_path("a/b/c/../.."));
        assert!(!is_safe_relative_path("a/../.."));
    }
}
