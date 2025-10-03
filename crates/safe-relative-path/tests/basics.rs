use safe_relative_path::SafeRelativePath;

#[test]
fn test_basic_paths() {
    assert_eq!(
        SafeRelativePath::from_relative_path("foo/bar")
            .unwrap()
            .as_str(),
        "foo/bar"
    );
    assert_eq!(
        SafeRelativePath::from_relative_path("baz")
            .unwrap()
            .as_str(),
        "baz"
    );
}
