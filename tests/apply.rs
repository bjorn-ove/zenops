use similar_asserts::assert_eq;
use smol_str::SmolStr;
use zenops::{
    Cmd,
    config_files::ConfigFilePath,
    error::Error,
    git::GitFileStatus,
    output::{PkgStatus, Status},
    prompt::{PreApplyAnswer, parse_pre_apply_input},
};
use zenops_safe_relative_path::srpath;

use test_env::{Entry, Output, paths};

mod test_env;

#[test]
fn missing_config() {
    let env = test_env::TestEnv::load();

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Err(Error::OpenDb(
            env.resolve_path(paths::ZENOPS_CONFIG),
            std::io::ErrorKind::NotFound.into()
        ))
    );

    // Check it works with a config file, but no .git repository
    env.write_zenops_file(srpath!("config.toml"), "", None);

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output { entries: vec![] }),
    );

    // Check it works with a .git repository
    env.init_config("");

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![env.git_repo_clean_entry()],
        }),
    );
}

#[test]
fn config_dir_git_status() {
    let env = test_env::TestEnv::load();

    env.init_config("");

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![env.git_repo_clean_entry()],
        })
    );

    env.append_zenops_file(srpath!("config.toml"), "# Modification", None);
    env.write_zenops_file(srpath!("untracked-file"), "# Untracked file", None);

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Git {
                    repo: env.cfpath("", ConfigFilePath::Zenops),
                    status: GitFileStatus::Modified(srpath!("config.toml").into()),
                }),
                Entry::Status(Status::Git {
                    repo: env.cfpath("", ConfigFilePath::Zenops),
                    status: GitFileStatus::Untracked(srpath!("untracked-file").into()),
                })
            ]
        })
    );
}

#[test]
fn apply_warns_on_uncommitted_changes_with_yes_and_allow_dirty() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    // Modify a tracked file and add an untracked one — neither committed.
    env.append_zenops_file(srpath!("config.toml"), "# Local tweak", None);
    env.write_zenops_file(srpath!("local-note"), "wip", None);

    // --yes --allow-dirty: pre-apply prompt is skipped, warnings still surface, apply runs.
    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        }),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Git {
                    repo: env.cfpath("", ConfigFilePath::Zenops),
                    status: GitFileStatus::Modified(srpath!("config.toml").into()),
                }),
                Entry::Status(Status::Git {
                    repo: env.cfpath("", ConfigFilePath::Zenops),
                    status: GitFileStatus::Untracked(srpath!("local-note").into()),
                }),
            ]
        })
    );
}

#[test]
fn apply_yes_on_dirty_repo_errors_without_allow_dirty() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    env.append_zenops_file(srpath!("config.toml"), "# Local tweak", None);

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: false,
        }),
        Err(Error::DirtyRepoRequiresAllowDirty(
            env.resolve_path(paths::ZENOPS_DIR)
        )),
    );
}

#[test]
fn apply_warns_on_deleted_tracked_file() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    // Commit an extra tracked file, then remove it from the working tree.
    env.write_zenops_file(srpath!("extra"), "to be deleted", Some("add extra"));
    env.delete_file(paths::ZENOPS_DIR.safe_join(srpath!("extra")));

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        }),
        Ok(Output {
            entries: vec![Entry::Status(Status::Git {
                repo: env.cfpath("", ConfigFilePath::Zenops),
                status: GitFileStatus::Deleted(srpath!("extra").into()),
            })]
        })
    );
}

#[test]
fn apply_clean_repo_with_yes_does_not_require_allow_dirty() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: false,
        }),
        Ok(Output { entries: vec![] })
    );
}

#[test]
fn parse_pre_apply_input_covers_expected_answers() {
    assert_eq!(parse_pre_apply_input("c"), Some(PreApplyAnswer::Commit));
    assert_eq!(parse_pre_apply_input("C"), Some(PreApplyAnswer::Commit));
    assert_eq!(
        parse_pre_apply_input("commit\n"),
        Some(PreApplyAnswer::Commit)
    );
    assert_eq!(parse_pre_apply_input(""), Some(PreApplyAnswer::Continue));
    assert_eq!(parse_pre_apply_input("\n"), Some(PreApplyAnswer::Continue));
    assert_eq!(parse_pre_apply_input("y"), Some(PreApplyAnswer::Continue));
    assert_eq!(parse_pre_apply_input("YES"), Some(PreApplyAnswer::Continue));
    assert_eq!(parse_pre_apply_input("n"), Some(PreApplyAnswer::Abort));
    assert_eq!(parse_pre_apply_input("abort"), Some(PreApplyAnswer::Abort));
    assert_eq!(parse_pre_apply_input("maybe"), None);
}

#[test]
fn apply_emits_pkg_missing_when_on_plus_detect_misses() {
    // `enable = "on"` says the user expects this pkg. Detect misses on the
    // temp host → push a Status::Pkg{Missing} with the brew-install hint.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.ghosttool]
        enable = "on"
        [pkg.ghosttool.install_hint.brew]
        packages = ["ghosttool"]
        [pkg.ghosttool.detect]
        type = "file"
        path = "/definitely/does/not/exist/zenops-test-ghosttool"
    "#,
    );

    let brew_available = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| std::path::Path::new(dir).join("brew").is_file());

    let out = env
        .run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        })
        .expect("apply should succeed");

    let expected_install_command = brew_available.then(|| "brew install ghosttool".to_string());
    assert!(
        out.entries.contains(&Entry::Status(Status::Pkg {
            pkg: SmolStr::new("ghosttool"),
            status: PkgStatus::Missing {
                install_command: expected_install_command,
            },
        })),
        "expected Pkg{{Missing}} for ghosttool, got: {:?}",
        out.entries
    );
}

#[test]
fn apply_is_silent_for_detect_variant_miss() {
    // Silence on miss is the point of `enable = "detect"`; no Pkg event.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.quietpkg]
        enable = "detect"
        [pkg.quietpkg.install_hint.brew]
        packages = ["quietpkg"]
        [pkg.quietpkg.detect]
        type = "file"
        path = "/definitely/does/not/exist/zenops-test-quietpkg"
    "#,
    );

    let out = env
        .run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        })
        .expect("apply should succeed");

    assert!(
        !out.entries.iter().any(|e| matches!(
            e,
            Entry::Status(Status::Pkg { pkg, .. }) if pkg == "quietpkg"
        )),
        "detect-miss should not push any Pkg event, got: {:?}",
        out.entries
    );
}

#[test]
fn apply_pkg_missing_with_no_install_hint_has_no_command() {
    // `on` + detect miss + empty brew packages → Pkg{Missing} without an
    // install command; the user just sees "X is missing".
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.hintless]
        enable = "on"
        [pkg.hintless.install_hint.brew]
        packages = []
        [pkg.hintless.detect]
        type = "file"
        path = "/definitely/does/not/exist/zenops-test-hintless"
    "#,
    );

    let out = env
        .run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        })
        .expect("apply should succeed");

    assert!(
        out.entries.contains(&Entry::Status(Status::Pkg {
            pkg: SmolStr::new("hintless"),
            status: PkgStatus::Missing {
                install_command: None,
            },
        })),
        "expected Pkg{{Missing}} without install_command, got: {:?}",
        out.entries
    );
}

#[test]
fn apply_no_pkg_missing_when_detect_is_empty() {
    // `on` + no detect → nothing to check, no signal to emit. This is the
    // always-on meta-pkg shape (local-bin, bashrc-chain, etc.).
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.metapkg]
        enable = "on"
        [pkg.metapkg.install_hint.brew]
        packages = []
    "#,
    );

    let out = env
        .run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        })
        .expect("apply should succeed");

    assert!(
        !out.entries.iter().any(|e| matches!(
            e,
            Entry::Status(Status::Pkg { pkg, .. }) if pkg == "metapkg"
        )),
        "empty-detect pkg should not emit any Pkg event, got: {:?}",
        out.entries
    );
}

#[test]
fn apply_filters_pkg_by_supported_shells() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "zsh"
        [shell.environment]
        [shell.alias]

        [pkg.alien-shell]
        enable = "on"
        supported_shells = ["bash"]
        [pkg.alien-shell.install_hint.brew]
        packages = []
        [[pkg.alien-shell.shell.interactive_init.zsh]]
        type = "line"
        line = "echo wrong-shell"
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
        allow_dirty: true,
    })
    .expect("apply should succeed");

    let zshrc = std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.zshrc")))
        .expect("zshrc should exist");
    assert!(
        !zshrc.contains("echo wrong-shell"),
        "pkg gated by supported_shells must not contribute on the wrong shell, got:\n{zshrc}"
    );
}

#[test]
fn apply_filters_pkg_by_supported_os() {
    // A pkg that targets the other OS must be suppressed: its env_init action
    // must not reach .zshenv.
    let other_os = if cfg!(target_os = "macos") {
        "linux"
    } else {
        "macos"
    };
    let env = test_env::TestEnv::load();
    env.init_config(&format!(
        r#"
        [shell]
        type = "zsh"
        [shell.environment]
        [shell.alias]

        [pkg.alien]
        enable = "on"
        supported_os = ["{other_os}"]
        [pkg.alien.install_hint.brew]
        packages = []
        [[pkg.alien.shell.env_init.zsh]]
        type = "line"
        line = "echo wrong-os"
    "#
    ));

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
        allow_dirty: true,
    })
    .expect("apply should succeed");

    let zshenv = std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.zshenv")))
        .expect("zshenv should exist");
    assert!(
        !zshenv.contains("echo wrong-os"),
        "pkg gated by supported_os must not contribute on the wrong OS, got:\n{zshenv}"
    );
}

#[test]
fn entry_status_propagates_unreadable_generated_file() {
    // A managed shell config wants to land at ~/.zenops_bash_profile, but a
    // directory occupies that path. read_to_string fails with IsADirectory;
    // expect Error::FailedToReadConfig rather than a `not yet implemented`
    // panic.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "bash"
        [shell.environment]
        [shell.alias]
    "#,
    );

    env.create_dir(srpath!("home/bob/.zenops_bash_profile"));

    let result = env.run(&Cmd::Status {
        diff: false,
        all: false,
    });

    match result {
        Err(Error::FailedToReadConfig(p, e)) => {
            assert_eq!(
                p.path,
                ConfigFilePath::in_home(srpath!(".zenops_bash_profile"))
            );
            // IsADirectory landed in stable rust 1.83+. Older targets surface
            // the same condition as Other; accept both.
            assert!(
                matches!(
                    e.kind(),
                    std::io::ErrorKind::IsADirectory | std::io::ErrorKind::Other,
                ),
                "unexpected I/O error kind: {:?}",
                e.kind(),
            );
        }
        other => panic!("expected FailedToReadConfig, got {other:?}"),
    }
}
