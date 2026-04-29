use similar_asserts::assert_eq;
use zenops::{Cmd, git::GitCmd};
use zenops_safe_relative_path::srpath;

use test_env::paths;

mod test_env;

#[test]
fn repo_commit_records_staged_changes_via_all_flag() {
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.append_zenops_file(srpath!("config.toml"), "\n# added", None);

    env.run(&Cmd::Repo {
        command: GitCmd::Commit {
            all: true,
            message: Some("update config".into()),
        },
    })
    .expect("repo commit should succeed");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let head_subject = env.git_out(&zenops, &["log", "-1", "--pretty=%s"]);
    assert_eq!(head_subject.trim(), "update config");
}

#[test]
fn repo_push_uploads_local_commits_to_bare_remote() {
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");
    env.write_zenops_file(srpath!("note"), "hi", Some("add note"));

    env.run(&Cmd::Repo {
        command: GitCmd::Push {},
    })
    .expect("repo push should succeed");

    let remote_log = env.git_out(&bare, &["log", "--oneline", "main"]);
    assert!(
        remote_log.contains("add note"),
        "remote did not receive the pushed commit: {remote_log:?}",
    );
}

#[test]
fn repo_pull_rebase_fast_forwards_remote_commits() {
    let env = test_env::TestEnv::load();
    let bare = env.init_config_with_remote("");

    // Seed a commit on the remote from a sidecar clone.
    env.seed_remote_commit(&bare, "from-remote", "hello\n", "remote commit");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    assert!(
        !env.git_out(&zenops, &["log", "--oneline"])
            .contains("remote commit"),
        "precondition: local repo should not yet have the remote commit",
    );

    env.run(&Cmd::Repo {
        command: GitCmd::Pull {
            rebase: Some(String::new()),
        },
    })
    .expect("repo pull --rebase should succeed");

    let local_log = env.git_out(&zenops, &["log", "--oneline"]);
    assert!(
        local_log.contains("remote commit"),
        "local repo did not pick up the remote commit: {local_log:?}",
    );
    assert!(
        zenops.join("from-remote").exists(),
        "pulled file should exist in the local working tree",
    );
}
