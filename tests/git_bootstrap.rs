//! Direct exercises of the bootstrap entry points on
//! [`zenops::git::Git`] (`clone_to`, `init_repo`, `initial_commit`,
//! `commit_all_and_push`, `print_pre_apply_summary`) and the dispatch
//! arms of [`zenops::git::GitCmd`] that aren't reached by the
//! single-rebase test in [`tests/repo_git.rs`]. The porcelain v2 parser
//! has its own coverage in [`tests/git_status.rs`]; the unmerged case
//! lives here because it needs a remote, which the parser tests don't.

use similar_asserts::assert_eq;
use xshell::{Shell, cmd};
use zenops::{
    Cmd,
    error::Error,
    git::{Git, GitCmd, GitFileStatus},
};
use zenops_safe_relative_path::srpath;

use test_env::paths;

mod test_env;

fn shell() -> Shell {
    Shell::new().unwrap()
}

#[test]
fn clone_to_succeeds_into_empty_dir() {
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("hello.txt", "hi\n")]);
    let dest = env.resolve_path(srpath!("home/bob/cloned"));

    let sh = shell();
    Git::clone_to(&format!("file://{}", bare.display()), &dest, None, &sh)
        .expect("clone should succeed");

    assert!(dest.join(".git").exists(), "expected .git in clone dest");
    assert_eq!(
        std::fs::read_to_string(dest.join("hello.txt")).unwrap(),
        "hi\n",
    );
}

#[test]
fn clone_to_with_branch_checks_out_named_branch() {
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("hello.txt", "hi\n")]);

    // Create a `feature` branch on the bare via a sidecar clone.
    let sidecar = env.resolve_path(srpath!("home/bob/seed_branch"));
    let sh = shell();
    cmd!(sh, "git clone")
        .arg(&bare)
        .arg(&sidecar)
        .ignore_stdout()
        .ignore_stderr()
        .run()
        .unwrap();
    let _dir = sh.push_dir(&sidecar);
    cmd!(sh, "git config commit.gpgsign false").run().unwrap();
    cmd!(sh, "git checkout -b feature")
        .ignore_stderr()
        .run()
        .unwrap();
    std::fs::write(sidecar.join("feature.txt"), "feat\n").unwrap();
    cmd!(sh, "git add feature.txt").run().unwrap();
    cmd!(sh, "git commit -m feature-commit")
        .ignore_stdout()
        .run()
        .unwrap();
    cmd!(sh, "git push -u origin feature")
        .ignore_stdout()
        .ignore_stderr()
        .run()
        .unwrap();
    drop(_dir);

    let dest = env.resolve_path(srpath!("home/bob/cloned_branch"));
    Git::clone_to(
        &format!("file://{}", bare.display()),
        &dest,
        Some("feature"),
        &sh,
    )
    .expect("branch clone should succeed");

    let head = std::fs::read_to_string(dest.join(".git/HEAD")).unwrap();
    assert!(
        head.contains("refs/heads/feature"),
        "expected HEAD to point at refs/heads/feature, got: {head:?}",
    );
    assert!(dest.join("feature.txt").exists());
}

#[test]
fn clone_to_returns_init_clone_failed_on_bad_url() {
    let env = test_env::TestEnv::load();
    let dest = env.resolve_path(srpath!("home/bob/wont_be_created"));

    let sh = shell();
    let err = Git::clone_to("file:///definitely/does/not/exist.git", &dest, None, &sh)
        .expect_err("clone of nonexistent url must fail");

    assert!(
        matches!(err, Error::InitCloneFailed { .. }),
        "expected InitCloneFailed, got: {err:?}",
    );
}

#[test]
fn init_repo_creates_dot_git_in_existing_dir() {
    let env = test_env::TestEnv::load();
    let dir = env.resolve_path(srpath!("home/bob/fresh"));
    std::fs::create_dir_all(&dir).unwrap();

    let sh = shell();
    Git::init_repo(&dir, &sh).expect("init_repo should succeed");

    assert!(dir.join(".git").exists(), "expected .git after init_repo");
    assert!(
        Git::new(&dir, &sh)
            .is_git_repo()
            .expect("is_git_repo should not fail")
    );
}

#[test]
fn initial_commit_records_staged_files_with_message() {
    let env = test_env::TestEnv::load();
    let dir = env.resolve_path(srpath!("home/bob/initial"));
    std::fs::create_dir_all(&dir).unwrap();
    let sh = shell();
    Git::init_repo(&dir, &sh).unwrap();
    cmd!(sh, "git -C {dir} config commit.gpgsign false")
        .run()
        .unwrap();
    cmd!(sh, "git -C {dir} config user.email zen@example.com")
        .run()
        .unwrap();
    cmd!(sh, "git -C {dir} config user.name Zen").run().unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();

    Git::initial_commit(&dir, &sh, "first commit").expect("initial_commit should succeed");

    let subject = cmd!(sh, "git -C {dir} log -1 --pretty=%s")
        .read()
        .expect("git log should succeed");
    assert_eq!(subject.trim(), "first commit");
    let tracked = cmd!(sh, "git -C {dir} ls-tree -r --name-only HEAD")
        .read()
        .unwrap();
    assert!(
        tracked.lines().any(|l| l == "a.txt"),
        "expected a.txt in HEAD tree, got: {tracked:?}",
    );
}

#[test]
fn commit_all_and_push_uploads_to_remote() {
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");
    // Uncommitted change: a new file under the zenops repo.
    env.write_zenops_file(srpath!("note.txt"), "hello\n", None);

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let sh = shell();
    Git::new(&zenops, &sh)
        .commit_all_and_push("zen update")
        .expect("commit_all_and_push should succeed");

    let remote_log = env.git_out(&bare, &["log", "--oneline", "main"]);
    assert!(
        remote_log.contains("zen update"),
        "remote did not receive the commit: {remote_log:?}",
    );
}

fn dirty_repo() -> (test_env::TestEnv, std::path::PathBuf) {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.append_zenops_file(srpath!("config.toml"), "\n# touched", None);
    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    (env, zenops)
}

#[test]
fn print_pre_apply_summary_runs_with_color_off() {
    let (_env, zenops) = dirty_repo();
    let sh = shell();
    Git::new(&zenops, &sh)
        .print_pre_apply_summary(false)
        .expect("print_pre_apply_summary(false) should succeed");
}

#[test]
fn print_pre_apply_summary_runs_with_color_on() {
    let (_env, zenops) = dirty_repo();
    let sh = shell();
    Git::new(&zenops, &sh)
        .print_pre_apply_summary(true)
        .expect("print_pre_apply_summary(true) should succeed");
}

#[test]
fn status_reports_other_for_unmerged_file() {
    // Set up divergent commits on the same path so `git pull --no-rebase`
    // produces a merge conflict, then assert the porcelain v2 `u` line
    // surfaces as `GitFileStatus::Other`.
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");

    // Modify config.toml on the remote via a sidecar clone.
    let sidecar = env.resolve_path(srpath!("home/bob/conflict_sidecar"));
    let sh = shell();
    cmd!(sh, "git clone")
        .arg(&bare)
        .arg(&sidecar)
        .ignore_stdout()
        .ignore_stderr()
        .run()
        .unwrap();
    {
        let _dir = sh.push_dir(&sidecar);
        cmd!(sh, "git config commit.gpgsign false").run().unwrap();
        std::fs::write(sidecar.join("config.toml"), "# from-remote\n").unwrap();
        cmd!(sh, "git add config.toml").run().unwrap();
        cmd!(sh, "git commit -m remote-edit")
            .ignore_stdout()
            .run()
            .unwrap();
        cmd!(sh, "git push")
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .unwrap();
    }

    // Divergent local commit on the same file.
    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    {
        let _dir = sh.push_dir(&zenops);
        std::fs::write(zenops.join("config.toml"), "# from-local\n").unwrap();
        cmd!(sh, "git add config.toml").run().unwrap();
        cmd!(sh, "git commit -m local-edit")
            .ignore_stdout()
            .run()
            .unwrap();
        // Force a merge with explicit author identity (older git on macOS may
        // require this).
        cmd!(sh, "git config user.email zen@example.com")
            .run()
            .unwrap();
        cmd!(sh, "git config user.name Zen").run().unwrap();
        cmd!(sh, "git fetch origin").ignore_stderr().run().unwrap();
        // `git merge` exits non-zero on conflict; ignore the status so we can
        // proceed to assert the unmerged state.
        cmd!(sh, "git merge --no-edit origin/main")
            .ignore_status()
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .unwrap();
    }

    let entries = Git::new(&zenops, &sh)
        .status()
        .expect("status should succeed");
    let unmerged: Vec<&GitFileStatus> = entries
        .iter()
        .filter(|e| matches!(e, GitFileStatus::Other { .. }))
        .collect();
    assert!(
        !unmerged.is_empty(),
        "expected at least one Other entry for the unmerged config.toml, got: {entries:?}",
    );
    let has_config = unmerged.iter().any(|e| match e {
        GitFileStatus::Other { path, .. } => path.as_str() == "config.toml",
        _ => false,
    });
    assert!(
        has_config,
        "expected config.toml among unmerged entries, got: {unmerged:?}",
    );
}

#[test]
fn repo_pull_without_rebase_fast_forwards_remote_commits() {
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");
    env.seed_remote_commit(&bare, "from-remote", "hi\n", "remote commit");

    env.run(&Cmd::Repo {
        command: GitCmd::Pull { rebase: None },
    })
    .expect("repo pull (no rebase) should succeed");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    assert!(zenops.join("from-remote").exists());
}

#[test]
fn repo_pull_with_rebase_false_runs() {
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");
    env.seed_remote_commit(&bare, "from-remote", "hi\n", "remote commit");

    env.run(&Cmd::Repo {
        command: GitCmd::Pull {
            rebase: Some("false".into()),
        },
    })
    .expect("repo pull --rebase=false should succeed");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    assert!(zenops.join("from-remote").exists());
}

#[test]
fn repo_status_filters_by_provided_path() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.append_zenops_file(srpath!("config.toml"), "\n# touched", None);

    env.run(&Cmd::Repo {
        command: GitCmd::Status {
            files: vec![srpath!("config.toml").to_safe_relative_path_buf()],
        },
    })
    .expect("repo status with files should succeed");
}

#[test]
fn repo_diff_filters_by_provided_path() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.append_zenops_file(srpath!("config.toml"), "\n# touched", None);

    env.run(&Cmd::Repo {
        command: GitCmd::Diff {
            files: vec![srpath!("config.toml").to_safe_relative_path_buf()],
        },
    })
    .expect("repo diff with files should succeed");
}

#[test]
fn repo_commit_without_all_flag_records_only_indexed() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.write_zenops_file(srpath!("staged.txt"), "x\n", None);

    // Stage staged.txt without committing — `all = false` should commit
    // only what's already in the index.
    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let _ = env.git_out(&zenops, &["add", "staged.txt"]);

    // Modify another file in the worktree (not staged); `all = false`
    // must NOT include this in the commit.
    env.append_zenops_file(srpath!("config.toml"), "\n# unstaged", None);

    env.run(&Cmd::Repo {
        command: GitCmd::Commit {
            all: false,
            message: Some("idx".into()),
        },
    })
    .expect("repo commit (all=false) should succeed");

    let head_subject = env.git_out(&zenops, &["log", "-1", "--pretty=%s"]);
    assert_eq!(head_subject.trim(), "idx");

    let head_files = env.git_out(&zenops, &["show", "--name-only", "--pretty=", "HEAD"]);
    let names: Vec<&str> = head_files.lines().collect();
    assert!(
        names.contains(&"staged.txt"),
        "expected staged.txt in HEAD commit, got: {names:?}",
    );
    assert!(
        !names.contains(&"config.toml"),
        "config.toml was unstaged; it must NOT be in HEAD, got: {names:?}",
    );
    // The unstaged change should still show as modified.
    let still_dirty = env.git_out(&zenops, &["status", "--porcelain"]);
    assert!(
        still_dirty.lines().any(|l| l.ends_with("config.toml")),
        "expected config.toml still dirty, got: {still_dirty:?}",
    );
}

#[test]
fn initial_commit_returns_err_when_git_dir_corrupted() {
    // Healthy state: init_repo + a worktree file. Then break the
    // precondition by wiping `.git/objects` so the commit cannot write
    // its blob/tree/commit records. Real git emits a real failure;
    // the wrapper's `?` propagates.
    let env = test_env::TestEnv::load();
    let dir = env.resolve_path(srpath!("home/bob/corrupted"));
    std::fs::create_dir_all(&dir).unwrap();
    let sh = shell();
    Git::init_repo(&dir, &sh).unwrap();
    cmd!(sh, "git -C {dir} config commit.gpgsign false")
        .run()
        .unwrap();
    cmd!(sh, "git -C {dir} config user.email zen@example.com")
        .run()
        .unwrap();
    cmd!(sh, "git -C {dir} config user.name Zen").run().unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();

    // Break it: nuke the object store.
    std::fs::remove_dir_all(dir.join(".git/objects")).unwrap();

    let err = Git::initial_commit(&dir, &sh, "should fail")
        .expect_err("initial_commit must fail with .git/objects removed");
    let _ = err; // exact xshell error shape is not part of the contract
}

#[test]
fn commit_all_and_push_returns_err_when_remote_deleted() {
    // Healthy state: zenops repo + bare remote. Break the precondition
    // by deleting the bare so push has nowhere to land.
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");
    env.write_zenops_file(srpath!("note.txt"), "hi\n", None);

    std::fs::remove_dir_all(&bare).unwrap();

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let sh = shell();
    let err = Git::new(&zenops, &sh)
        .commit_all_and_push("nope")
        .expect_err("commit_all_and_push must fail with remote deleted");
    let _ = err;
}

#[test]
fn has_uncommitted_changes_returns_err_when_called_outside_repo() {
    // `is_git_repo` ignores stderr/status so it returns Ok(false) for
    // a non-repo dir; `has_uncommitted_changes` does NOT, so the `?`
    // on `read()` propagates the non-zero exit.
    let env = test_env::TestEnv::load();
    let dir = env.resolve_path(srpath!("home/bob/not_a_repo"));
    std::fs::create_dir_all(&dir).unwrap();
    let sh = shell();
    let err = Git::new(&dir, &sh)
        .has_uncommitted_changes()
        .expect_err("has_uncommitted_changes must fail outside a git repo");
    let _ = err;
}

#[test]
fn print_pre_apply_summary_returns_err_when_called_outside_repo() {
    let env = test_env::TestEnv::load();
    let dir = env.resolve_path(srpath!("home/bob/no_repo_here"));
    std::fs::create_dir_all(&dir).unwrap();
    let sh = shell();
    let err = Git::new(&dir, &sh)
        .print_pre_apply_summary(false)
        .expect_err("print_pre_apply_summary must fail outside a git repo");
    let _ = err;
}

#[test]
fn repo_pull_with_rebase_true_runs() {
    // Covers the `format!("--rebase={v}")` else-arm in GitCmd::Pull.
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");
    env.seed_remote_commit(&bare, "from-remote", "hi\n", "remote commit");

    env.run(&Cmd::Repo {
        command: GitCmd::Pull {
            rebase: Some("true".into()),
        },
    })
    .expect("repo pull --rebase=true should succeed");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    assert!(zenops.join("from-remote").exists());
}
