use similar_asserts::assert_eq;
use zenops::{Cmd, error::Error};

use test_env::{Entry, paths};

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
    // TestEnv pins `Args::stdin_is_terminal` to false regardless of how
    // `cargo test` was launched, so once the directory is out of the way
    // bootstrap must refuse with InitNeedsTty.
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

#[test]
fn init_apply_false_emits_init_summary_event() {
    use zenops::output::InitSummary;

    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    let out = env
        .run(&Cmd::Init {
            url: Some(bare.to_str().unwrap().to_string()),
            branch: None,
            apply: false,
            yes: false,
        })
        .expect("init should succeed");
    let summaries: Vec<&InitSummary> = out
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Init(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(
        summaries.len(),
        1,
        "expected exactly one init_summary, got: {:?}",
        out.entries,
    );
    assert_eq!(summaries[0].shell.as_deref(), Some("bash"));
}

#[test]
fn init_apply_false_emits_init_summary_with_remote_url() {
    // The seeded bare repo's path is set as `origin` by Git::clone_to. The
    // summary's `remote` field must reflect that — exercises the Some(url)
    // branch in emit_clone_summary.
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    let out = env
        .run(&init_cmd(bare.to_str().unwrap()))
        .expect("init should succeed");
    let summary = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::Init(s) => Some(s),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected one InitSummary, got: {:?}", out.entries));
    assert_eq!(
        summary.remote.as_deref(),
        Some(bare.to_str().unwrap()),
        "expected remote URL to match the bare repo path",
    );
}

#[test]
fn init_clone_summary_handles_zsh_shell() {
    use zenops::output::InitSummary;

    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[(
        "config.toml",
        r#"
[shell]
type = "zsh"
[shell.environment]
[shell.alias]
"#,
    )]);

    let out = env
        .run(&init_cmd(bare.to_str().unwrap()))
        .expect("init should succeed");
    let summary: &InitSummary = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::Init(s) => Some(s),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected one InitSummary, got: {:?}", out.entries));
    assert_eq!(summary.shell.as_deref(), Some("zsh"));
}

#[test]
fn init_clone_summary_omits_shell_when_not_configured() {
    use zenops::output::InitSummary;

    // No `[shell]` section → emit_clone_summary's `config.shell().map(...)`
    // is None.
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", "")]);

    let out = env
        .run(&init_cmd(bare.to_str().unwrap()))
        .expect("init should succeed");
    let summary: &InitSummary = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::Init(s) => Some(s),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected one InitSummary, got: {:?}", out.entries));
    assert_eq!(summary.shell, None);
}

#[test]
fn init_clone_summary_pkg_count_includes_user_packages() {
    use zenops::output::InitSummary;

    // pkg_count reflects user-defined entries plus zenops' built-in pkgs;
    // adding three user pkgs shifts the count by +3 vs the empty-config
    // baseline.
    let env = test_env::TestEnv::load();
    let baseline_bare = env.seed_bare_repo(&[("config.toml", "")]);
    let baseline_out = env
        .run(&init_cmd(baseline_bare.to_str().unwrap()))
        .expect("baseline init should succeed");
    let baseline = baseline_out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::Init(s) => Some(s.pkg_count),
            _ => None,
        })
        .unwrap();

    let env2 = test_env::TestEnv::load();
    let extended_bare = env2.seed_bare_repo(&[(
        "config.toml",
        r#"
[pkg.alpha]
[pkg.alpha.install_hint.brew]
packages = ["alpha"]

[pkg.beta]
[pkg.beta.install_hint.brew]
packages = ["beta"]

[pkg.gamma]
[pkg.gamma.install_hint.brew]
packages = ["gamma"]
"#,
    )]);
    let out: zenops::output::InitSummary = env2
        .run(&init_cmd(extended_bare.to_str().unwrap()))
        .expect("init should succeed")
        .entries
        .into_iter()
        .find_map(|e| match e {
            Entry::Init(s) => Some(s),
            _ => None,
        })
        .unwrap();
    let _: InitSummary = out;
    assert_eq!(out.pkg_count, baseline + 3);
}

#[test]
fn init_apply_true_does_not_emit_init_summary() {
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);

    let out = env
        .run(&Cmd::Init {
            url: Some(bare.to_str().unwrap().to_string()),
            branch: None,
            apply: true,
            yes: true,
        })
        .expect("init --apply --yes should succeed");
    let has_summary = out.entries.iter().any(|e| matches!(e, Entry::Init(_)));
    assert!(
        !has_summary,
        "init --apply should defer to apply's event stream and skip init_summary, got: {:?}",
        out.entries,
    );
}
