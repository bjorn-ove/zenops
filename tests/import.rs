use std::path::PathBuf;

use similar_asserts::assert_eq;
use zenops::{
    Cmd,
    error::Error,
    import::ImportError,
    output::{ImportFileAction, ImportType},
};
use zenops_safe_relative_path::srpath;

use test_env::{Entry, TestEnv};

mod test_env;

const MINIMAL_CONFIG: &str = r#"
[shell]
type = "bash"
[shell.environment]
[shell.alias]
"#;

fn import_cmd(path: PathBuf) -> Cmd {
    Cmd::Import {
        path,
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: false,
        yes: true,
        dry_run: false,
    }
}

fn read_config(env: &TestEnv) -> toml::Value {
    let cfg_path = env.resolve_path(srpath!("home/bob/.config/zenops/config.toml"));
    let text = std::fs::read_to_string(&cfg_path).unwrap();
    toml::from_str(&text).unwrap()
}

#[test]
fn import_dot_config_happy_path() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"foo = 1\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("import should succeed");

    // Original is now a symlink.
    let original = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    let meta = std::fs::symlink_metadata(&original).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "original should be a symlink"
    );

    // Repo copy holds the bytes.
    let repo_copy = env.resolve_path(srpath!("home/bob/.config/zenops/configs/myapp/config.toml"));
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"foo = 1\n");

    // Symlink target matches.
    let target = std::fs::read_link(&original).unwrap();
    assert_eq!(target, repo_copy);

    // config.toml gained a [pkg.myapp] block with the configs entry.
    let cfg = read_config(&env);
    let entry = &cfg["pkg"]["myapp"]["configs"][0];
    assert_eq!(entry["type"].as_str(), Some(".config"));
    assert_eq!(entry["source"].as_str(), Some("configs/myapp"));
    assert_eq!(
        entry["symlinks"].as_array().unwrap()[0].as_str(),
        Some("config.toml"),
    );
    assert_eq!(
        cfg["pkg"]["myapp"]["install_hint"]["brew"]["packages"]
            .as_array()
            .unwrap()[0]
            .as_str(),
        Some("myapp"),
    );
}

#[test]
fn import_home_dotfile() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.zshrc"), b"export FOO=1\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.zshrc")),
        pkg: None,
        source: None,
        brew: vec!["zsh".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("import should succeed");

    let original = env.resolve_path(srpath!("home/bob/.zshrc"));
    let meta = std::fs::symlink_metadata(&original).unwrap();
    assert!(meta.file_type().is_symlink());

    let repo_copy = env.resolve_path(srpath!("home/bob/.config/zenops/configs/zshrc/.zshrc"));
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"export FOO=1\n");

    let cfg = read_config(&env);
    let entry = &cfg["pkg"]["zshrc"]["configs"][0];
    assert_eq!(entry["type"].as_str(), Some("home"));
    assert_eq!(entry["dir"].as_str(), Some(""));
    assert_eq!(
        entry["symlinks"].as_array().unwrap()[0].as_str(),
        Some(".zshrc"),
    );
}

#[test]
fn import_home_dotdir() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.ssh/config"), b"Host *\n");
    env.write_file(
        srpath!("home/bob/.ssh/known_hosts"),
        b"github.com ssh-rsa AAAA\n",
    );

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.ssh")),
        pkg: None,
        source: None,
        brew: vec!["openssh".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("import should succeed");

    for name in ["config", "known_hosts"] {
        let symlink_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.ssh/{name}").parse().unwrap();
        let symlink_path = env.resolve_path(&symlink_rel);
        assert!(
            std::fs::symlink_metadata(&symlink_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let repo_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/ssh/{name}")
                .parse()
                .unwrap();
        assert_eq!(
            std::fs::read_link(&symlink_path).unwrap(),
            env.resolve_path(&repo_rel),
        );
    }

    let cfg = read_config(&env);
    let entry = &cfg["pkg"]["ssh"]["configs"][0];
    assert_eq!(entry["type"].as_str(), Some("home"));
    assert_eq!(entry["dir"].as_str(), Some(".ssh"));
    let symlinks: Vec<&str> = entry["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(symlinks.contains(&"config"));
    assert!(symlinks.contains(&"known_hosts"));
}

#[test]
fn import_rejects_unsupported_layout_subdir() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        srpath!("home/bob/.config/myapp/themes/onedark.toml"),
        b"theme = 'dark'\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/myapp/themes")));
    let result = env.run(&cmd);
    match result {
        Err(Error::Import(ImportError::UnsupportedLayout(s))) => {
            assert!(
                s.contains(".config/myapp/themes"),
                "tail should be in error: {s:?}"
            );
        }
        other => panic!("expected UnsupportedLayout, got {other:?}"),
    }

    // Source untouched.
    let original = env.resolve_path(srpath!("home/bob/.config/myapp/themes/onedark.toml"));
    let meta = std::fs::symlink_metadata(&original).unwrap();
    assert!(
        meta.file_type().is_file(),
        "source should still be a real file"
    );
}

#[test]
fn import_rejects_unsupported_layout_non_dot() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/dotfiles/zsh/.zshrc"), b"\n");

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/dotfiles/zsh")));
    let result = env.run(&cmd);
    assert!(
        matches!(
            result,
            Err(Error::Import(ImportError::UnsupportedLayout(_)))
        ),
        "got {result:?}"
    );
}

#[test]
fn import_extends_existing_pkg() {
    let env = TestEnv::load();
    let preset = format!(
        "{}\n[pkg.zsh]\ndescription = \"the Z shell\"\n[pkg.zsh.install_hint.brew]\npackages = [\"zsh\"]\n",
        MINIMAL_CONFIG,
    );
    env.init_config(&preset);
    env.write_file(srpath!("home/bob/.config/zsh/.zshenv"), b"# nothing\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/zsh")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("import should succeed");

    let cfg = read_config(&env);
    // install_hint preserved.
    assert_eq!(
        cfg["pkg"]["zsh"]["install_hint"]["brew"]["packages"]
            .as_array()
            .unwrap()[0]
            .as_str(),
        Some("zsh"),
    );
    // description preserved.
    assert_eq!(
        cfg["pkg"]["zsh"]["description"].as_str(),
        Some("the Z shell"),
    );
    // configs[0] now has the new entry.
    let entry = &cfg["pkg"]["zsh"]["configs"][0];
    assert_eq!(entry["type"].as_str(), Some(".config"));
    assert_eq!(entry["source"].as_str(), Some("configs/zsh"));
}

#[test]
fn import_refuses_when_dest_exists() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    // Pre-create a file at the destination — neither a symlink nor matching
    // anything, so import must refuse.
    env.write_zenops_file(srpath!("configs/myapp/config.toml"), b"old\n", None);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"new\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::DestExists(_)))),
        "got {result:?}"
    );
    // Source untouched.
    let src = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert_eq!(std::fs::read(&src).unwrap(), b"new\n");
    // Repo copy untouched.
    let dst = env.resolve_path(srpath!("home/bob/.config/zenops/configs/myapp/config.toml"));
    assert_eq!(std::fs::read(&dst).unwrap(), b"old\n");
}

#[test]
fn import_skips_existing_symlinks_in_source() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"real\n");
    // A second file that's a symlink to somewhere else — should be skipped.
    env.write_file(srpath!("home/bob/elsewhere/cache.dat"), b"x\n");
    env.create_symlink(
        srpath!("home/bob/elsewhere/cache.dat"),
        srpath!("home/bob/.config/myapp/cache.dat"),
    );

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let out = env.run(&cmd).expect("import should succeed");

    // Real file got moved+symlinked.
    let original = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // Pre-existing symlink left alone — still points at the original target.
    let pre_existing = env.resolve_path(srpath!("home/bob/.config/myapp/cache.dat"));
    let pre_target = std::fs::read_link(&pre_existing).unwrap();
    assert_eq!(
        pre_target,
        env.resolve_path(srpath!("home/bob/elsewhere/cache.dat")),
    );

    // Plan mentions the skip.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("expected ImportPlan");
    let skips: Vec<_> = plan
        .file_actions
        .iter()
        .filter_map(|a| match a {
            ImportFileAction::Skip { path, reason } => Some((path, reason)),
            _ => None,
        })
        .collect();
    assert_eq!(skips.len(), 1);
    assert_eq!(*skips[0].0, std::path::PathBuf::from("cache.dat"));
    assert_eq!(skips[0].1.as_str(), "symlink");

    // config.toml only lists the regular file.
    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["myapp"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["config.toml"]);
}

#[test]
fn import_yes_mode_without_install_hint_errors() {
    // No --brew, no --no-install-hint, and --yes (so no prompter is wired
    // up). New pkg → MissingInstallHint.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    match result {
        Err(Error::Import(ImportError::MissingInstallHint(k))) => {
            assert_eq!(k, "myapp");
        }
        other => panic!("expected MissingInstallHint, got {other:?}"),
    }

    // Source untouched.
    let src = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert_eq!(std::fs::read(&src).unwrap(), b"x\n");
}

#[test]
fn import_no_install_hint_skips_brew_prompt() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: true,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd)
        .expect("import should succeed with --no-install-hint");

    let cfg = read_config(&env);
    let packages = cfg["pkg"]["myapp"]["install_hint"]["brew"]["packages"]
        .as_array()
        .unwrap();
    assert!(
        packages.is_empty(),
        "--no-install-hint should write an empty packages array, got: {packages:?}"
    );
}

#[test]
fn import_dry_run_writes_nothing() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");
    let original_config_text =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: false,
        dry_run: true,
    };
    let out = env.run(&cmd).expect("dry-run should succeed");

    // Source untouched.
    let src = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    let meta = std::fs::symlink_metadata(&src).unwrap();
    assert!(
        meta.file_type().is_file(),
        "source should still be a real file"
    );
    assert_eq!(std::fs::read(&src).unwrap(), b"x\n");

    // No file landed in the repo.
    let repo_dir = env.resolve_path(srpath!("home/bob/.config/zenops/configs"));
    assert!(
        !repo_dir.exists() || std::fs::read_dir(&repo_dir).unwrap().next().is_none(),
        "configs/ should be empty after dry-run"
    );

    // config.toml unchanged.
    let after =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();
    assert_eq!(after, original_config_text);

    // ImportPlan still emitted so JSON consumers see the plan; no
    // ImportApplied event since dry-run skips the apply phase.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("expected ImportPlan");
    assert_eq!(plan.r#type, ImportType::DotConfig);
    assert_eq!(plan.pkg.as_str(), "myapp");
    assert!(
        out.entries
            .iter()
            .all(|e| !matches!(e, Entry::ImportApplied(_))),
        "dry-run should not emit ImportApplied",
    );
}

#[test]
fn import_idempotent_after_partial_run() {
    // Re-running import after a successful import should be a no-op (no
    // DestExists error). The plan calls this out as an explicit guarantee
    // for partial-failure recovery.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("first import should succeed");

    // Second run: every file already imported. SourceEmpty fires because
    // the planner short-circuits all already-imported files; no other
    // failure should escape.
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::SourceEmpty(_)))),
        "got {result:?}"
    );
}
