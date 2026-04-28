use similar_asserts::assert_eq;
use zenops::{Cmd, error::Error};

use test_env::paths;

mod test_env;

const MINIMAL_CONFIG: &str = r#"
[shell]
type = "bash"
[shell.environment]
[shell.alias]
"#;

fn init_cmd(url: &str) -> Cmd {
    Cmd::Init {
        url: Some(url.to_string()),
        branch: None,
        apply: false,
        yes: false,
    }
}

fn bootstrap_cmd() -> Cmd {
    Cmd::Init {
        url: None,
        branch: None,
        apply: false,
        yes: false,
    }
}

#[test]
fn init_clones_into_empty_existing_dir() {
    // TestEnv::load pre-creates an empty ~/.config/zenops — init should
    // remove the empty dir and clone on top of it.
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    env.run(&init_cmd(bare.to_str().unwrap()))
        .expect("init should succeed into empty existing dir");

    let config_path = env.resolve_path(paths::ZENOPS_CONFIG);
    assert!(config_path.exists(), "config.toml should exist after init");
    let git_dir = env.resolve_path(paths::ZENOPS_DIR).join(".git");
    assert!(git_dir.exists(), ".git should exist after init");
}

#[test]
fn init_clones_into_nonexistent_dir() {
    let env = test_env::TestEnv::load();
    env.delete_dir_all(paths::ZENOPS_DIR);
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    env.run(&init_cmd(bare.to_str().unwrap()))
        .expect("init should succeed into nonexistent dir");

    let config_path = env.resolve_path(paths::ZENOPS_CONFIG);
    assert!(config_path.exists(), "config.toml should exist after init");
}

#[test]
fn init_refuses_nonempty_dir() {
    let env = test_env::TestEnv::load();
    // Populate the zenops dir with an unrelated file so preflight sees it
    // as non-empty.
    env.write_zenops_file(
        zenops_safe_relative_path::srpath!("leftover"),
        "stale\n",
        None,
    );
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    let result = env.run(&init_cmd(bare.to_str().unwrap()));
    assert_eq!(
        result,
        Err(Error::InitDirNotEmpty(env.resolve_path(paths::ZENOPS_DIR))),
    );
}

#[test]
fn init_rejects_repo_without_config_toml() {
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("README.md", "no config here\n")]);

    let result = env.run(&init_cmd(bare.to_str().unwrap()));
    assert_eq!(
        result,
        Err(Error::InitNoConfigToml(env.resolve_path(paths::ZENOPS_DIR))),
    );

    // Directory was left in place so the user can inspect it.
    let readme = env.resolve_path(paths::ZENOPS_DIR).join("README.md");
    assert!(
        readme.exists(),
        "clone should be left in place for inspection"
    );
}

#[test]
fn init_bootstrap_refuses_existing_empty_dir() {
    // TestEnv::load pre-creates an empty ~/.config/zenops; bootstrap is
    // strict and refuses any existing directory.
    let env = test_env::TestEnv::load();

    let result = env.run(&bootstrap_cmd());
    assert_eq!(
        result,
        Err(Error::InitDirExists(env.resolve_path(paths::ZENOPS_DIR))),
    );
}

#[test]
fn init_bootstrap_refuses_existing_git_dir() {
    let env = test_env::TestEnv::load();
    // Drop a `.git` directory inside the pre-created zenops dir so the
    // preflight surfaces the more specific InitGitDirExists.
    env.create_dir(zenops_safe_relative_path::srpath!(
        "home/bob/.config/zenops/.git"
    ));

    let result = env.run(&bootstrap_cmd());
    assert_eq!(
        result,
        Err(Error::InitGitDirExists(env.resolve_path(paths::ZENOPS_DIR))),
    );
}

#[test]
fn init_bootstrap_needs_tty_when_dir_is_clear() {
    // Cargo test always runs with a non-TTY stdin, so once the directory
    // is out of the way bootstrap should refuse with InitNeedsTty.
    let env = test_env::TestEnv::load();
    env.delete_dir_all(paths::ZENOPS_DIR);

    let result = env.run(&bootstrap_cmd());
    assert_eq!(result, Err(Error::InitNeedsTty));
}

#[test]
fn init_with_apply_yes_runs_apply() {
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    env.run(&Cmd::Init {
        url: Some(bare.to_str().unwrap().to_string()),
        branch: None,
        apply: true,
        yes: true,
    })
    .expect("init --apply --yes should succeed");

    // A `[shell] type = "bash"` config makes apply generate
    // ~/.zenops_bash_profile; its existence proves apply ran end-to-end.
    let profile = env.resolve_path(zenops_safe_relative_path::srpath!(
        "home/bob/.zenops_bash_profile"
    ));
    assert!(
        profile.exists(),
        ".zenops_bash_profile should exist after init --apply, got missing at {profile:?}"
    );
}
