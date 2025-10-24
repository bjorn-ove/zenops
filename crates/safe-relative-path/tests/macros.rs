use safe_relative_path::{SafeRelativePath, srpath};

#[test]
fn test_basic_paths() {
    let path1 = srpath!("foo/bar");
    let path2 = srpath!("baz");
    let path3 = srpath!("deep/nested/path/file.txt");

    assert_eq!(path1.to_string(), "foo/bar");
    assert_eq!(path2.to_string(), "baz");
    assert_eq!(path3.to_string(), "deep/nested/path/file.txt");
}

#[test]
fn test_current_dir_paths() {
    let path1 = srpath!("./foo");
    let path2 = srpath!(".");
    let path3 = srpath!("./bar/baz");

    assert_eq!(path1.to_string(), "./foo");
    assert_eq!(path2.to_string(), ".");
    assert_eq!(path3.to_string(), "./bar/baz");
}

#[test]
fn test_empty_path() {
    let path = srpath!("");
    assert_eq!(path.to_string(), "");
}

#[test]
fn test_path_operations() {
    let path = srpath!("foo/bar");
    let path_buf = path.to_safe_relative_path_buf();

    assert_eq!(path_buf.to_string(), "foo/bar");

    let prefixed = srpath!("base").safe_join(path);
    assert_eq!(prefixed.to_string(), "base/foo/bar");
}

#[test]
fn test_correct_type() {
    let _: &SafeRelativePath = srpath!("foo/bar");
}
