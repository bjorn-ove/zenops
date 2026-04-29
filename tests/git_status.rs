//! Direct exercises of the `git status --porcelain=v2` parser in
//! [`zenops::git::Git::status`]. Builds real git states and asserts the
//! reduced [`GitFileStatus`] surface — the unit tests cover only
//! `parse_is_inside_work_tree`, leaving the porcelain v2 line shapes
//! (`1`, `2`, `?`, `!`) untested at workspace level.

use std::path::Path;

use similar_asserts::assert_eq;
use xshell::{Shell, cmd};
use zenops::git::{Git, GitFileStatus};
use zenops_safe_relative_path::{SafeRelativePath, srpath};

use test_env::paths;

mod test_env;

fn status_in(dir: &Path) -> Vec<GitFileStatus> {
    let sh = Shell::new().unwrap();
    Git::new(dir, &sh).status().expect("git status succeeds")
}

#[test]
fn status_reports_modified_for_a_changed_tracked_file() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    // Modify the tracked config.toml without committing.
    env.append_zenops_file(srpath!("config.toml"), "\n# touched", None);

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let entries = status_in(&zenops);
    assert_eq!(
        entries,
        vec![GitFileStatus::Modified(
            SafeRelativePath::from_relative_path("config.toml")
                .unwrap()
                .into()
        )],
    );
}

#[test]
fn status_reports_added_for_a_staged_only_new_file() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    // New file, staged but not committed → porcelain XY = "A.".
    env.write_zenops_file(srpath!("staged.txt"), "stuff\n", None);
    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let sh = Shell::new().unwrap();
    let _dir = sh.push_dir(&zenops);
    cmd!(sh, "git add staged.txt").run().unwrap();

    let entries = status_in(&zenops);
    assert_eq!(
        entries,
        vec![GitFileStatus::Added(
            SafeRelativePath::from_relative_path("staged.txt")
                .unwrap()
                .into()
        )],
    );
}

#[test]
fn status_reports_deleted_for_a_removed_tracked_file() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.write_zenops_file(srpath!("doomed.txt"), "bye\n", Some("add doomed"));
    env.delete_file(paths::ZENOPS_DIR.safe_join(srpath!("doomed.txt")));

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let entries = status_in(&zenops);
    assert_eq!(
        entries,
        vec![GitFileStatus::Deleted(
            SafeRelativePath::from_relative_path("doomed.txt")
                .unwrap()
                .into()
        )],
    );
}

#[test]
fn status_reports_untracked_for_a_brand_new_file() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    // Drop a file in the zenops repo without `git add`.
    env.write_file(
        paths::ZENOPS_DIR.safe_join(srpath!("never_added.txt")),
        "hi\n",
    );

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let entries = status_in(&zenops);
    assert_eq!(
        entries,
        vec![GitFileStatus::Untracked(
            SafeRelativePath::from_relative_path("never_added.txt")
                .unwrap()
                .into()
        )],
    );
}

#[test]
fn status_reports_modified_at_new_path_after_rename() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.write_zenops_file(srpath!("old.txt"), "data\n", Some("add old"));

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let sh = Shell::new().unwrap();
    let _dir = sh.push_dir(&zenops);
    cmd!(sh, "git mv old.txt new.txt").run().unwrap();

    let entries = status_in(&zenops);
    // Tag `2` (rename) reduces to Modified at the new path.
    assert_eq!(
        entries,
        vec![GitFileStatus::Modified(
            SafeRelativePath::from_relative_path("new.txt")
                .unwrap()
                .into()
        )],
    );
}

#[test]
fn status_returns_empty_for_a_clean_repo() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let entries = status_in(&zenops);
    assert!(entries.is_empty(), "expected empty status, got {entries:?}");
}

#[test]
fn is_git_repo_true_inside_work_tree_false_outside() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    let sh = Shell::new().unwrap();
    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    assert!(Git::new(&zenops, &sh).is_git_repo().unwrap());

    let home = env.resolve_path(paths::HOME_DIR);
    assert!(!Git::new(&home, &sh).is_git_repo().unwrap());
}

#[test]
fn has_uncommitted_changes_flips_with_a_modification() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    let sh = Shell::new().unwrap();
    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    assert!(!Git::new(&zenops, &sh).has_uncommitted_changes().unwrap());

    env.append_zenops_file(srpath!("config.toml"), "\n# touched", None);
    assert!(Git::new(&zenops, &sh).has_uncommitted_changes().unwrap());
}
