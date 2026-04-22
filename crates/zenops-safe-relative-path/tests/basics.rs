use std::{ffi::OsStr, path::Path, sync::Arc};

use relative_path::RelativePath;
use zenops_safe_relative_path::{
    SafeRelativePath, SafeRelativePathBuf, SinglePathComponent, error::Error,
};

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

#[test]
fn from_relative_path_rejects_parent_traversal() {
    let err = SafeRelativePath::from_relative_path("../escape").unwrap_err();
    assert_eq!(err, Error::PathGoesOutsideParent("../escape".into()));
    let err = SafeRelativePath::from_relative_path("foo/../../bar").unwrap_err();
    assert_eq!(err, Error::PathGoesOutsideParent("foo/../../bar".into()));
}

#[test]
fn try_join_succeeds_for_safe_path_and_fails_for_traversal() {
    let base = SafeRelativePath::from_relative_path("base").unwrap();

    let joined = base.try_join("nested/file").unwrap();
    assert_eq!(joined.as_str(), "base/nested/file");

    let err = base.try_join("../escape").unwrap_err();
    assert_eq!(err, Error::PathGoesOutsideParent("../escape".into()));
}

#[test]
fn safe_parent_walks_up_one_component_at_a_time() {
    let p = SafeRelativePath::from_relative_path("a/b/c").unwrap();
    assert_eq!(p.safe_parent().unwrap().as_str(), "a/b");
    assert_eq!(
        p.safe_parent().unwrap().safe_parent().unwrap().as_str(),
        "a",
    );
    // Root relative path has no parent.
    let root = SafeRelativePath::from_relative_path("").unwrap();
    assert!(root.safe_parent().is_none());
}

#[test]
fn to_full_path_appends_relative_to_base() {
    let p = SafeRelativePath::from_relative_path("sub/file.txt").unwrap();
    assert_eq!(
        p.to_full_path("/tmp/base"),
        Path::new("/tmp/base/sub/file.txt"),
    );
}

#[test]
fn normalize_safe_collapses_current_dir_segments() {
    let p = SafeRelativePath::from_relative_path("./foo/./bar").unwrap();
    assert_eq!(p.normalize_safe().as_str(), "foo/bar");
}

#[test]
fn safe_relative_path_buf_round_trips_via_from_str_and_display() {
    let buf: SafeRelativePathBuf = "foo/bar".parse().unwrap();
    assert_eq!(format!("{buf}"), "foo/bar");
    assert_eq!(format!("{buf:?}"), "\"foo/bar\"");

    let err = "../nope".parse::<SafeRelativePathBuf>().unwrap_err();
    assert_eq!(err, Error::PathGoesOutsideParent("../nope".into()));
}

#[test]
fn safe_relative_path_buf_asref_variants_all_point_at_same_bytes() {
    let buf: SafeRelativePathBuf = SafeRelativePathBuf::from_relative_path("hello/world").unwrap();

    // &RelativePath: drives relative-path crate interop.
    let rel: &RelativePath = buf.as_ref();
    assert_eq!(rel.as_str(), "hello/world");

    // &OsStr: lets the type be passed to std::process::Command etc.
    let os: &OsStr = buf.as_ref();
    assert_eq!(os, OsStr::new("hello/world"));

    // &SafeRelativePath: canonical borrow + Deref target.
    let borrowed: &SafeRelativePath = buf.as_ref();
    assert_eq!(borrowed.as_str(), "hello/world");
    assert_eq!((*buf).as_str(), "hello/world");
}

#[test]
fn safe_relative_path_asref_relative_path_and_debug() {
    let p = SafeRelativePath::from_relative_path("a/b").unwrap();
    let rel: &RelativePath = p.as_ref();
    assert_eq!(rel.as_str(), "a/b");
    assert_eq!(format!("{p:?}"), "\"a/b\"");
}

#[test]
fn arc_safe_relative_path_conversions_preserve_contents() {
    // From<SafeRelativePathBuf>
    let buf = SafeRelativePathBuf::from_relative_path("one/two").unwrap();
    let arc: Arc<SafeRelativePath> = buf.into();
    assert_eq!(arc.as_str(), "one/two");

    // From<&SafeRelativePath>
    let p = SafeRelativePath::from_relative_path("alpha").unwrap();
    let arc: Arc<SafeRelativePath> = p.into();
    assert_eq!(arc.as_str(), "alpha");
}

#[test]
fn single_path_component_accepts_a_single_segment() {
    let c = SinglePathComponent::try_new("name").unwrap();
    assert_eq!(format!("{c}"), "name");
    // AsRef<SafeRelativePath>
    let p: &SafeRelativePath = c.as_ref();
    assert_eq!(p.as_str(), "name");
    // Deref to SafeRelativePath
    assert_eq!((*c).as_str(), "name");
}

#[test]
fn single_path_component_rejects_multi_component_paths() {
    let err = SinglePathComponent::try_new("a/b").unwrap_err();
    assert_eq!(err, Error::NotASinglePathComponent("a/b".to_string()));
}

#[test]
fn single_path_component_rejects_parent_traversal() {
    // `..` fails the SafeRelativePath check *before* the single-component
    // check, so the error is PathGoesOutsideParent.
    let err = SinglePathComponent::try_new("..").unwrap_err();
    assert_eq!(err, Error::PathGoesOutsideParent("..".into()));
}
