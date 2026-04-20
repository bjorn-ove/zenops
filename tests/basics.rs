use similar_asserts::assert_eq;
use zenops::{
    Cmd, ColorChoice,
    config_files::ConfigFilePath,
    error::Error,
    git::GitFileStatus,
    output::{AppliedAction, Status, SymlinkStatus},
};
use zenops_safe_relative_path::srpath;

use test_env::{Entry, Output, paths};

mod test_env;

#[test]
fn missing_config() {
    let env = test_env::TestEnv::load();

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Err(Error::OpenDb(
            env.resolve_path(paths::ZENOPS_CONFIG),
            std::io::ErrorKind::NotFound.into()
        ))
    );

    // Check it works with a config file, but no .git repository
    env.write_zenops_file(srpath!("config.toml"), "", None);

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output { entries: vec![] }),
    );

    // Check it works with a .git repository
    env.init_config("");

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output { entries: vec![] }),
    );
}

#[test]
fn config_dir_git_status() {
    let env = test_env::TestEnv::load();

    env.init_config("");

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output { entries: vec![] })
    );

    env.append_zenops_file(srpath!("config.toml"), "# Modification", None);
    env.write_zenops_file(srpath!("untracked-file"), "# Untracked file", None);

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
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
fn pkg_list_shows_defaults_as_missing() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    // None of the default pkgs' detect targets exist in a temp home.
    let out = env
        .run_pkg_list(false, false, false, ColorChoice::Never)
        .expect("pkg list should succeed");

    assert!(
        out.contains("cargo"),
        "expected cargo in output, got: {out}"
    );
    assert!(out.contains("sk"), "expected sk in output, got: {out}");
    assert!(
        out.contains("starship"),
        "expected starship in output, got: {out}"
    );
    // Each default pkg has a description, which should appear indented.
    assert!(
        out.contains("cross-shell prompt"),
        "expected starship description, got: {out}"
    );
    // Plain-text output must never contain ANSI escapes.
    assert!(
        !out.contains('\x1b'),
        "ColorChoice::Never output should not contain ANSI escapes, got: {out:?}"
    );

    // With color forced on, ANSI escapes must appear.
    let colored = env
        .run_pkg_list(false, false, false, ColorChoice::Always)
        .expect("pkg list --color always should succeed");
    assert!(
        colored.contains('\x1b'),
        "ColorChoice::Always output should contain ANSI escapes, got: {colored:?}"
    );
}

#[test]
fn pkg_list_aggregates_missing_packages_into_footer() {
    let env = test_env::TestEnv::load();
    // Use two pkgs whose detect strategies resolve against HOME only, so the
    // test is insensitive to whatever is installed on the machine running it.
    env.init_config(
        r#"
        [pkg.alpha]
        enable = "detect"
        description = "Alpha test pkg."
        [[pkg.alpha.detect]]
        type = "file"
        path = "~/.alpha-marker"
        [pkg.alpha.install_hint.brew]
        packages = ["alpha-formula"]

        [pkg.bravo]
        enable = "detect"
        description = "Bravo test pkg."
        [[pkg.bravo.detect]]
        type = "file"
        path = "~/.bravo-marker"
        [pkg.bravo.install_hint.brew]
        packages = ["bravo-formula"]
    "#,
    );

    let out = env
        .run_pkg_list(false, false, false, ColorChoice::Never)
        .expect("pkg list should succeed");

    let brew_available = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| std::path::Path::new(dir).join("brew").is_file());

    if brew_available {
        let footer_line = out
            .lines()
            .find(|l| l.contains("To install all missing via brew: brew install"))
            .unwrap_or_else(|| panic!("expected aggregate footer, got: {out}"));
        assert!(
            footer_line.contains("alpha-formula") && footer_line.contains("bravo-formula"),
            "footer should list both missing pkgs, got: {footer_line}"
        );
        // Each package appears exactly once across the whole output's footer.
        assert_eq!(
            footer_line.matches("alpha-formula").count(),
            1,
            "alpha-formula should appear once in footer, got: {footer_line}"
        );
    } else {
        assert!(
            !out.contains("via brew:"),
            "expected no install guidance without brew on PATH, got: {out}"
        );
        assert!(
            !out.contains("To install all missing"),
            "expected no aggregate footer without brew on PATH, got: {out}"
        );
    }
}

#[test]
fn pkg_list_all_flag_surfaces_disabled_pkgs() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.ghost]
        enable = "disabled"
        description = "A pkg the user opted out of."
        [pkg.ghost.install_hint.brew]
        packages = []
    "#,
    );

    let default_out = env
        .run_pkg_list(false, false, false, ColorChoice::Never)
        .expect("pkg list should succeed");
    assert!(
        !default_out.contains("ghost"),
        "disabled pkg should be hidden by default, got: {default_out}"
    );

    let all_out = env
        .run_pkg_list(true, false, false, ColorChoice::Never)
        .expect("pkg list --all should succeed");
    let ghost_line = all_out
        .lines()
        .find(|l| l.contains("ghost"))
        .unwrap_or_else(|| panic!("disabled pkg should appear with --all, got: {all_out}"));
    assert!(
        ghost_line.starts_with("- "),
        "disabled pkg row should begin with the `-` marker, got: {ghost_line:?}"
    );
}

#[test]
fn pkg_list_hides_pkgs_gated_to_other_os() {
    let other_os = if cfg!(target_os = "macos") {
        "linux"
    } else {
        "macos"
    };
    let env = test_env::TestEnv::load();
    env.init_config(&format!(
        r#"
        [pkg.alien]
        enable = "on"
        supported_os = ["{other_os}"]
        description = "Only applies on the other OS."
        [pkg.alien.install_hint.brew]
        packages = []
    "#
    ));

    let out = env
        .run_pkg_list(true, false, false, ColorChoice::Never)
        .expect("pkg list --all should succeed");
    assert!(
        !out.contains("alien"),
        "pkg gated to the other OS must not appear in the list, got: {out}"
    );
}

#[test]
fn pkg_list_hides_pkgs_gated_to_other_shell() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "zsh"
        [shell.environment]
        [shell.alias]

        [pkg.bash-only]
        enable = "on"
        supported_shells = ["bash"]
        description = "Bash-only pkg."
        [pkg.bash-only.install_hint.brew]
        packages = []
    "#,
    );

    let out = env
        .run_pkg_list(true, false, false, ColorChoice::Never)
        .expect("pkg list --all should succeed");
    assert!(
        !out.contains("bash-only"),
        "pkg gated to other shell must not appear in list, got: {out}"
    );
}

#[test]
fn pkg_list_shell_filter_is_independent_of_shell_actions() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "bash"
        [shell.environment]
        [shell.alias]

        [pkg.dual-actions]
        enable = "on"
        supported_shells = ["zsh"]
        description = "Has both shell actions but gated to zsh."
        [pkg.dual-actions.install_hint.brew]
        packages = []
        [[pkg.dual-actions.shell.interactive_init.bash]]
        type = "line"
        line = "echo from-bash"
        [[pkg.dual-actions.shell.interactive_init.zsh]]
        type = "line"
        line = "echo from-zsh"
    "#,
    );

    let out = env
        .run_pkg_list(false, false, false, ColorChoice::Never)
        .expect("pkg list should succeed");
    assert!(
        !out.contains("dual-actions"),
        "shell gate must hide pkg even when bash actions exist, got: {out}"
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
fn pkg_list_renders_name_override_instead_of_key() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.verbose-key-name]
        enable = "on"
        name = "short"
        description = "Pkg with a display-name override."
        [pkg.verbose-key-name.install_hint.brew]
        packages = []
    "#,
    );

    let out = env
        .run_pkg_list(false, false, false, ColorChoice::Never)
        .expect("pkg list should succeed");
    assert!(
        out.contains("short"),
        "display name override should appear in list, got: {out}"
    );
    assert!(
        !out.contains("verbose-key-name"),
        "map key should not leak when `name` override is set, got: {out}"
    );
}

#[test]
fn symlinked_configs() {
    let env = test_env::TestEnv::load();
    let dummy_config_symlink = paths::CONFIG_DIR.safe_join(srpath!("dummy-util/dummy-util.toml"));
    let dummy_real = env.cfpath("configs/dummy-util/dummy-util.toml", ConfigFilePath::Zenops);
    let dummy_symlink = env.cfpath("dummy-util/dummy-util.toml", ConfigFilePath::DotConfig);
    let dummy2_config_symlink = paths::HOME_DIR.safe_join(srpath!(".dummy2/dummy2.toml"));
    let dummy2_real = env.cfpath("configs/dummy2/dummy2.toml", ConfigFilePath::Zenops);
    let dummy2_symlink = env.cfpath(".dummy2/dummy2.toml", ConfigFilePath::Home);

    env.init_config(
        r#"
        [[configs]]
        type = ".config"
        name = "dummy-util"
        source = "configs/dummy-util"
        symlinks = [
          "dummy-util.toml"
        ]

        [[configs]]
        type = "home"
        dir = ".dummy2"
        source = "configs/dummy2"
        symlinks = [
          "dummy2.toml"
        ]
    "#,
    );

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Symlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                    status: SymlinkStatus::DstDirIsMissing
                }),
                Entry::Status(Status::Symlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                    status: SymlinkStatus::DstDirIsMissing
                })
            ]
        })
    );

    env.create_symlink(
        paths::ZENOPS_DIR.safe_join(srpath!("configs/dummy-util/dummy-util.toml")),
        &dummy_config_symlink,
    );

    env.create_symlink(
        paths::ZENOPS_DIR.safe_join(srpath!("configs/dummy2/dummy2.toml")),
        &dummy2_config_symlink,
    );

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Symlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                    status: SymlinkStatus::RealPathIsMissing
                }),
                Entry::Status(Status::Symlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                    status: SymlinkStatus::RealPathIsMissing
                })
            ]
        })
    );

    env.write_zenops_file(
        srpath!("configs/dummy-util/dummy-util.toml"),
        "# hello",
        Some("Added dummy-util.toml"),
    );

    env.write_zenops_file(
        srpath!("configs/dummy2/dummy2.toml"),
        "# hello2",
        Some("Added dummy2.toml"),
    );

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Symlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                    status: SymlinkStatus::Ok
                }),
                Entry::Status(Status::Symlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                    status: SymlinkStatus::Ok
                })
            ]
        })
    );

    env.delete_file(&dummy_config_symlink);
    env.delete_file(&dummy2_config_symlink);

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Symlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                    status: SymlinkStatus::New
                }),
                Entry::Status(Status::Symlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                    status: SymlinkStatus::New
                })
            ]
        })
    );

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false
        }),
        Ok(Output {
            entries: vec![
                Entry::AppliedAction(AppliedAction::CreatedSymlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                }),
                Entry::AppliedAction(AppliedAction::CreatedSymlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                })
            ]
        })
    );

    env.delete_dir_all(dummy_config_symlink.safe_parent().unwrap());
    env.delete_dir_all(dummy2_config_symlink.safe_parent().unwrap());

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false
        }),
        Ok(Output {
            entries: vec![
                Entry::AppliedAction(AppliedAction::CreatedDir(dummy_symlink.parent().unwrap())),
                Entry::AppliedAction(AppliedAction::CreatedSymlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                }),
                Entry::AppliedAction(AppliedAction::CreatedDir(dummy2_symlink.parent().unwrap())),
                Entry::AppliedAction(AppliedAction::CreatedSymlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                })
            ]
        })
    );
}

#[test]
fn completions_subcommand_generates_bash_script() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zenops"))
        .args(["completions", "bash"])
        .output()
        .expect("running zenops completions bash should succeed");
    assert!(
        output.status.success(),
        "zenops completions bash exited {}; stderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let script = String::from_utf8(output.stdout).unwrap();
    assert!(
        script.contains("_zenops"),
        "expected _zenops function in bash completions, got:\n{script}"
    );
    assert!(
        script.contains("complete "),
        "expected `complete` directive in bash completions, got:\n{script}"
    );
}

#[test]
fn completions_subcommand_generates_zsh_script() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zenops"))
        .args(["completions", "zsh"])
        .output()
        .expect("running zenops completions zsh should succeed");
    assert!(output.status.success());
    let script = String::from_utf8(output.stdout).unwrap();
    assert!(
        script.contains("#compdef zenops"),
        "expected `#compdef zenops` directive in zsh completions, got:\n{script}"
    );
}

#[test]
fn apply_emits_zsh_compinit_via_line_action() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "zsh"
        [shell.environment]
        [shell.alias]
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
    })
    .expect("apply should succeed");

    let zshrc_path = env.resolve_path(srpath!("home/bob/.zshrc"));
    let zshrc = std::fs::read_to_string(&zshrc_path)
        .unwrap_or_else(|e| panic!("failed to read {zshrc_path:?}: {e}"));

    assert!(
        zshrc.contains("# Initialize Zsh completions"),
        "expected compinit comment in generated zshrc, got:\n{zshrc}"
    );
    assert!(
        zshrc.contains("autoload -Uz compinit && compinit"),
        "expected verbatim compinit line in generated zshrc, got:\n{zshrc}"
    );
}

#[test]
fn apply_emits_path_actions_inline_grouped_with_comments() {
    // User config adds two pkgs that each contribute one PATH fragment next to
    // a comment header. Each PATH action must emit its own `export PATH=…`
    // line right at its position in the action stream so it stays grouped
    // with the preceding comment, rather than being collected into one
    // trailing export.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "bash"
        [shell.environment]
        [shell.alias]

        [pkg.front]
        enable = "on"
        [pkg.front.install_hint.brew]
        packages = []
        [[pkg.front.shell.env_init.bash]]
        type = "comment"
        text = "Front setup"
        [[pkg.front.shell.env_init.bash]]
        type = "path_prepend"
        value = "/opt/front/bin"

        [pkg.back]
        enable = "on"
        [pkg.back.install_hint.brew]
        packages = []
        [[pkg.back.shell.env_init.bash]]
        type = "comment"
        text = "Back setup"
        [[pkg.back.shell.env_init.bash]]
        type = "path_append"
        value = "/opt/back/bin"
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
    })
    .expect("apply should succeed");

    let rc_path = env.resolve_path(srpath!("home/bob/.zenops_bash_profile"));
    let rc = std::fs::read_to_string(&rc_path)
        .unwrap_or_else(|e| panic!("failed to read {rc_path:?}: {e}"));

    // Each PATH action renders as its own inline export line, using POSIX
    // shell syntax. The renderer owns the "how" (delimiter, quoting, $PATH
    // position) — the TOML just declares prepend vs append.
    assert!(
        rc.contains(r#"export PATH="/opt/front/bin:$PATH""#),
        "expected inline prepend export line, got:\n{rc}"
    );
    assert!(
        rc.contains(r#"export PATH="$PATH:/opt/back/bin""#),
        "expected inline append export line, got:\n{rc}"
    );
    // Default-on pkg.local-bin contributes ~/.local/bin, translated to $HOME
    // form so shells don't treat the tilde literally.
    assert!(
        rc.contains(r#"export PATH="$PATH:$HOME/.local/bin""#),
        "expected local-bin append with $HOME translation, got:\n{rc}"
    );

    // Grouping: the prepend export must sit under the "Front setup" comment
    // (no blank line between comment and action), not drift to the bottom.
    let front_comment_idx = rc.find("# Front setup").expect("front comment");
    let front_export_idx = rc
        .find(r#"export PATH="/opt/front/bin:$PATH""#)
        .expect("front export");
    let back_comment_idx = rc.find("# Back setup").expect("back comment");
    let back_export_idx = rc
        .find(r#"export PATH="$PATH:/opt/back/bin""#)
        .expect("back export");
    assert!(front_comment_idx < front_export_idx);
    assert!(front_export_idx < back_comment_idx);
    assert!(back_comment_idx < back_export_idx);
}

#[test]
fn apply_emits_login_init_actions_for_bash() {
    // The built-in pkg.bashrc-chain is always on and emits into login_init.bash.
    // Verify its line reaches the generated bash profile.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "bash"
        [shell.environment]
        [shell.alias]
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
    })
    .expect("apply should succeed");

    let rc_path = env.resolve_path(srpath!("home/bob/.zenops_bash_profile"));
    let rc = std::fs::read_to_string(&rc_path)
        .unwrap_or_else(|e| panic!("failed to read {rc_path:?}: {e}"));

    assert!(
        rc.contains("[ -f ~/.bashrc ] && source ~/.bashrc"),
        "expected bashrc-chain line in bash profile, got:\n{rc}"
    );
}

#[test]
fn apply_skips_zprofile_when_no_login_init_zsh_actions() {
    // Without any pkg contributing login_init.zsh actions, .zprofile must
    // not be generated at all. On a brew-less host the default pkg.brew-macos
    // fails detection, so nothing writes to .zprofile.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "zsh"
        [shell.environment]
        [shell.alias]
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
    })
    .expect("apply should succeed");

    let zprofile_path = env.resolve_path(srpath!("home/bob/.zprofile"));
    // Detect based on the test host: if brew is present, .zprofile will exist.
    // Without brew on PATH, .zprofile must not be created.
    let brew_present = std::path::Path::new("/opt/homebrew/bin/brew").exists()
        || std::path::Path::new("/usr/local/bin/brew").exists()
        || std::path::Path::new("/home/linuxbrew/.linuxbrew/bin/brew").exists();
    if brew_present && cfg!(target_os = "macos") {
        // On macOS with brew, pkg.brew-macos will emit login_init.zsh.
        assert!(
            zprofile_path.exists(),
            "on macOS with brew, .zprofile should be generated"
        );
    } else {
        assert!(
            !zprofile_path.exists(),
            "no login_init.zsh actions → .zprofile must not be written"
        );
    }
}

#[test]
fn apply_routes_login_init_zsh_action_to_zprofile() {
    // User-defined pkg that contributes a login_init.zsh action: the line
    // must land in .zprofile, not .zshenv or .zshrc.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "zsh"
        [shell.environment]
        [shell.alias]

        [pkg.greeter]
        enable = "on"
        [pkg.greeter.install_hint.brew]
        packages = []
        [[pkg.greeter.shell.login_init.zsh]]
        type = "line"
        line = "echo hello-from-login"
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
    })
    .expect("apply should succeed");

    let zprofile = std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.zprofile")))
        .expect("zprofile should be generated when a pkg contributes login_init.zsh");
    let zshenv = std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.zshenv")))
        .expect("zshenv should exist");
    let zshrc = std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.zshrc")))
        .expect("zshrc should exist");

    assert!(
        zprofile.contains("echo hello-from-login"),
        "login_init.zsh action must appear in .zprofile, got:\n{zprofile}"
    );
    assert!(
        !zshenv.contains("echo hello-from-login"),
        "login_init.zsh action must NOT appear in .zshenv, got:\n{zshenv}"
    );
    assert!(
        !zshrc.contains("echo hello-from-login"),
        "login_init.zsh action must NOT appear in .zshrc, got:\n{zshrc}"
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
fn apply_injects_zenops_completions_into_generated_bash_profile() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "bash"
        [shell.environment]
        [shell.alias]
    "#,
    );

    env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
    })
    .expect("apply should succeed");

    let rc_path = env.resolve_path(srpath!("home/bob/.zenops_bash_profile"));
    let rc = std::fs::read_to_string(&rc_path)
        .unwrap_or_else(|e| panic!("failed to read {rc_path:?}: {e}"));

    assert!(
        rc.contains("# zenops shell completions"),
        "expected zenops completions comment in generated bash profile, got:\n{rc}"
    );
    assert!(
        rc.contains("source <(zenops completions bash)"),
        "expected source line for zenops completions in generated bash profile, got:\n{rc}"
    );
}

fn init_single_symlink_env() -> (
    test_env::TestEnv,
    zenops_safe_relative_path::SafeRelativePathBuf,
    zenops::output::ResolvedConfigFilePath,
    zenops::output::ResolvedConfigFilePath,
) {
    let env = test_env::TestEnv::load();
    let symlink_full = paths::CONFIG_DIR.safe_join(srpath!("dummy-util/dummy-util.toml"));
    let real = env.cfpath("configs/dummy-util/dummy-util.toml", ConfigFilePath::Zenops);
    let symlink = env.cfpath("dummy-util/dummy-util.toml", ConfigFilePath::DotConfig);

    env.init_config(
        r#"
        [[configs]]
        type = ".config"
        name = "dummy-util"
        source = "configs/dummy-util"
        symlinks = [
          "dummy-util.toml"
        ]
    "#,
    );
    env.write_zenops_file(
        srpath!("configs/dummy-util/dummy-util.toml"),
        "# hello",
        Some("Added dummy-util.toml"),
    );

    (env, symlink_full, real, symlink)
}

#[test]
fn symlink_dst_is_regular_file() {
    let (env, symlink_full, real, symlink) = init_single_symlink_env();

    // A regular (non-symlink) file sits where the symlink should go.
    env.write_file(&symlink_full, "# pre-existing user file\n");

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output {
            entries: vec![Entry::Status(Status::Symlink {
                real: real.clone(),
                symlink: symlink.clone(),
                status: SymlinkStatus::IsFile,
            })]
        })
    );

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false
        }),
        Err(Error::RefusingToOverwriteFileWithSymlink {
            real: real.clone(),
            symlink: symlink.clone(),
        })
    );
}

#[test]
fn symlink_dst_is_directory() {
    let (env, symlink_full, real, symlink) = init_single_symlink_env();

    // A directory sits where the symlink should go.
    env.create_dir(&symlink_full);

    assert_eq!(
        env.run(&Cmd::Status { diff: false }),
        Ok(Output {
            entries: vec![Entry::Status(Status::Symlink {
                real: real.clone(),
                symlink: symlink.clone(),
                status: SymlinkStatus::IsDir,
            })]
        })
    );

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false
        }),
        Err(Error::RefusingToOverwriteDirectoryWithSymlink {
            real: real.clone(),
            symlink: symlink.clone(),
        })
    );
}

#[test]
fn apply_dry_run_skips_all_changes() {
    let (env, symlink_full, _real, _symlink) = init_single_symlink_env();

    // Dry-run: every change is rendered as a prompt but never applied.
    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: false,
            dry_run: true,
        }),
        Ok(Output { entries: vec![] }),
    );

    // The symlink must not have been created on disk.
    let symlink_disk = env.resolve_path(&symlink_full);
    assert!(
        symlink_disk.symlink_metadata().is_err(),
        "dry-run should not have created {symlink_disk:?}"
    );
}
