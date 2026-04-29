use zenops::Cmd;
use zenops_safe_relative_path::srpath;

mod test_env;

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
        allow_dirty: true,
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
        allow_dirty: true,
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
        allow_dirty: true,
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
        allow_dirty: true,
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
        allow_dirty: true,
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
        allow_dirty: true,
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
