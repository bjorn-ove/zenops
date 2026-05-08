use std::path::PathBuf;

use similar_asserts::assert_eq;
use zenops::{
    Cmd,
    error::Error,
    import::ImportError,
    output::{ImportFileAction, ImportTomlChange, ImportType},
};
use zenops_safe_relative_path::{SafeRelativePath, srpath};

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

/// Build a config.toml fragment with a pre-existing `[pkg.<key>]` block
/// whose first `[[pkg.<key>.configs]]` entry already manages
/// `~/.config/<dir>/`. Used to seed extend-mode tests with a managed
/// config that the import path can extend.
fn dotconfig_preset(pkg_key: &str, dir: Option<&str>, existing_symlinks: &[&str]) -> String {
    let dir_decl = dir.map(|d| format!("name = \"{d}\"\n")).unwrap_or_default();
    let symlinks: Vec<String> = existing_symlinks
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect();
    format!(
        "{minimal}\n\
         [pkg.{pkg_key}]\n\
         [pkg.{pkg_key}.install_hint.brew]\n\
         packages = [\"{pkg_key}\"]\n\
         [[pkg.{pkg_key}.configs]]\n\
         type = \".config\"\n\
         {dir_decl}source = \"configs/{pkg_key}\"\n\
         symlinks = [{symlinks_inner}]\n",
        minimal = MINIMAL_CONFIG,
        symlinks_inner = symlinks.join(", "),
    )
}

#[test]
fn import_extend_dotconfig_happy_path() {
    let env = TestEnv::load();
    env.init_config(&dotconfig_preset("helix", None, &["config.toml"]));
    // The new file the user wants to bring under management.
    env.write_file(
        srpath!("home/bob/.config/helix/templates/new-template.toml"),
        b"[template]\nx = 1\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!(
        "home/bob/.config/helix/templates/new-template.toml"
    )));
    let out = env.run(&cmd).expect("extend import should succeed");

    // File was moved into the repo and replaced with a symlink.
    let original = env.resolve_path(srpath!(
        "home/bob/.config/helix/templates/new-template.toml"
    ));
    let meta = std::fs::symlink_metadata(&original).unwrap();
    assert!(meta.file_type().is_symlink());
    let repo_copy = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/helix/templates/new-template.toml"
    ));
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"[template]\nx = 1\n");
    assert_eq!(std::fs::read_link(&original).unwrap(), repo_copy);

    // Existing entry's symlinks array gained the rel path; no new entry was
    // created.
    let cfg = read_config(&env);
    let entries = cfg["pkg"]["helix"]["configs"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "no new configs entry should be added");
    let symlinks: Vec<&str> = entries[0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["config.toml", "templates/new-template.toml"]);

    // Plan event reports an AppendSymlinks change, not a CreatePkg /
    // AppendConfigsEntry pair.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("expected ImportPlan");
    assert!(!plan.created_pkg);
    assert_eq!(plan.r#type, ImportType::DotConfig);
    let toml_kinds: Vec<_> = plan
        .toml_changes
        .iter()
        .map(|c| match c {
            ImportTomlChange::CreatePkg { .. } => "create",
            ImportTomlChange::AppendConfigsEntry { .. } => "append_entry",
            ImportTomlChange::AppendSymlinks { paths, .. } => {
                if paths.is_empty() {
                    "append_symlinks_empty"
                } else {
                    "append_symlinks"
                }
            }
            _ => "other",
        })
        .collect();
    assert_eq!(toml_kinds, vec!["append_symlinks"]);
}

#[test]
fn import_extend_dotconfig_with_name_override() {
    // pkg key "neovim" but the on-disk dir is "nvim".
    let env = TestEnv::load();
    env.init_config(&dotconfig_preset("neovim", Some("nvim"), &["init.lua"]));
    env.write_file(
        srpath!("home/bob/.config/nvim/lua/plugins/none.lua"),
        b"return {}\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/nvim/lua/plugins/none.lua")));
    env.run(&cmd).expect("extend should match by name override");

    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["neovim"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(symlinks.contains(&"lua/plugins/none.lua"));
}

#[test]
fn import_extend_home_pkg() {
    let env = TestEnv::load();
    let preset = format!(
        "{}\n\
         [pkg.gitcfg]\n\
         [pkg.gitcfg.install_hint.brew]\n\
         packages = [\"git\"]\n\
         [[pkg.gitcfg.configs]]\n\
         type = \"home\"\n\
         dir = \".local/share/gitcfg\"\n\
         source = \"configs/gitcfg\"\n\
         symlinks = [\"main.toml\"]\n",
        MINIMAL_CONFIG,
    );
    env.init_config(&preset);
    env.write_file(
        srpath!("home/bob/.local/share/gitcfg/profiles/work.toml"),
        b"profile = 'work'\n",
    );

    let cmd =
        import_cmd(env.resolve_path(srpath!("home/bob/.local/share/gitcfg/profiles/work.toml")));
    env.run(&cmd).expect("extend home pkg should succeed");

    let original = env.resolve_path(srpath!("home/bob/.local/share/gitcfg/profiles/work.toml"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["gitcfg"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["main.toml", "profiles/work.toml"]);
}

#[test]
fn import_extend_rejects_new_pkg_flags() {
    let env = TestEnv::load();
    env.init_config(&dotconfig_preset("helix", None, &["config.toml"]));
    env.write_file(
        srpath!("home/bob/.config/helix/themes/onedark.toml"),
        b"theme = 'dark'\n",
    );

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix/themes/onedark.toml")),
        pkg: Some("something-else".into()),
        source: None,
        brew: vec!["helix".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    match env.run(&cmd) {
        Err(Error::Import(ImportError::ExtendFlagsInvalid { flags })) => {
            assert!(flags.contains(&"--pkg"));
            assert!(flags.contains(&"--brew"));
        }
        other => panic!("expected ExtendFlagsInvalid, got {other:?}"),
    }

    // Source untouched.
    let src = env.resolve_path(srpath!("home/bob/.config/helix/themes/onedark.toml"));
    let meta = std::fs::symlink_metadata(&src).unwrap();
    assert!(meta.file_type().is_file());
}

#[test]
fn import_extend_rejects_directory_input() {
    let env = TestEnv::load();
    env.init_config(&dotconfig_preset("helix", None, &["config.toml"]));
    env.write_file(
        srpath!("home/bob/.config/helix/themes/onedark.toml"),
        b"theme = 'dark'\n",
    );

    // A directory under an already-managed config — single-file scope only.
    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix/themes")));
    match env.run(&cmd) {
        Err(Error::Import(ImportError::ExtendDirectoryNotSupported(p))) => {
            assert!(p.ends_with(".config/helix/themes"), "got {p:?}");
        }
        other => panic!("expected ExtendDirectoryNotSupported, got {other:?}"),
    }
}

#[test]
fn import_extend_dry_run_writes_nothing() {
    let env = TestEnv::load();
    env.init_config(&dotconfig_preset("helix", None, &["config.toml"]));
    env.write_file(
        srpath!("home/bob/.config/helix/themes/onedark.toml"),
        b"theme = 'dark'\n",
    );
    let original_text =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix/themes/onedark.toml")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: false,
        yes: false,
        dry_run: true,
    };
    let out = env.run(&cmd).expect("dry-run should succeed");

    // Source still a regular file, not a symlink.
    let src = env.resolve_path(srpath!("home/bob/.config/helix/themes/onedark.toml"));
    assert!(
        std::fs::symlink_metadata(&src)
            .unwrap()
            .file_type()
            .is_file()
    );
    // No file landed in the repo.
    assert!(
        !env.resolve_path(srpath!(
            "home/bob/.config/zenops/configs/helix/themes/onedark.toml"
        ))
        .exists()
    );
    // config.toml unchanged.
    let after =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();
    assert_eq!(after, original_text);
    // Plan still emitted, ImportApplied not.
    assert!(
        out.entries
            .iter()
            .any(|e| matches!(e, Entry::ImportPlan(_)))
    );
    assert!(
        out.entries
            .iter()
            .all(|e| !matches!(e, Entry::ImportApplied(_))),
    );
}

#[test]
fn import_extend_no_match_falls_back_to_layout_check() {
    // No managed pkg owns ~/.config/unmanaged/, so a deep file under it
    // doesn't trigger extend mode and the existing UnsupportedLayout
    // diagnostic fires (unchanged behavior).
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        srpath!("home/bob/.config/unmanaged/sub/file.toml"),
        b"x = 1\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/unmanaged/sub/file.toml")));
    match env.run(&cmd) {
        Err(Error::Import(ImportError::UnsupportedLayout(s))) => {
            assert!(s.contains(".config/unmanaged/sub"), "tail in error: {s:?}");
        }
        other => panic!("expected UnsupportedLayout, got {other:?}"),
    }
}

#[test]
fn import_idempotent_after_partial_run() {
    // Re-running import on an already-managed root flips into reconcile
    // mode. With everything in sync, reconcile is a no-op success — no
    // DestExists or SourceEmpty errors escape.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let first = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&first).expect("first import should succeed");

    // Second run: pkg is now managed, so this reconciles. With nothing
    // to add or remove, the plan is a no-op and applies cleanly.
    let second = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&second).expect("reconcile no-op should succeed");
}

/// Seed an already-managed `[pkg.<key>]` whose `[[pkg.<key>.configs]]`
/// entry manages `~/.config/<dir>/`, plus the on-disk repo copies and
/// symlinks for each entry in `existing`. Mirrors a realistic state
/// after a successful first import — reconcile tests use this to
/// describe the starting point.
fn seed_managed_dotconfig(
    env: &TestEnv,
    pkg_key: &str,
    dir: Option<&str>,
    existing: &[(&str, &[u8])],
) {
    let symlink_strs: Vec<&str> = existing.iter().map(|(rel, _)| *rel).collect();
    let preset = dotconfig_preset(pkg_key, dir, &symlink_strs);
    env.init_config(&preset);
    let on_disk_dir = dir.unwrap_or(pkg_key);
    for (rel, body) in existing {
        let repo_rel = format!("home/bob/.config/zenops/configs/{pkg_key}/{rel}");
        let home_rel = format!("home/bob/.config/{on_disk_dir}/{rel}");
        env.write_file(
            SafeRelativePath::from_relative_path(&repo_rel).unwrap(),
            *body,
        );
        env.create_symlink(
            SafeRelativePath::from_relative_path(&repo_rel).unwrap(),
            SafeRelativePath::from_relative_path(&home_rel).unwrap(),
        );
    }
}

#[test]
fn import_reconcile_adds_new_files() {
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[("config.toml", b"theme = 'dracula'\n")],
    );
    // A new file the user dropped into the managed dir without re-running.
    env.write_file(
        srpath!("home/bob/.config/helix/themes/dark.toml"),
        b"name = 'dark'\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    // New file is now a symlink to the repo copy.
    let original = env.resolve_path(srpath!("home/bob/.config/helix/themes/dark.toml"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let repo_copy = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/helix/themes/dark.toml"
    ));
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"name = 'dark'\n");

    // symlinks array now lists both paths.
    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["config.toml", "themes/dark.toml"]);

    // Plan event flags this as a non-noop apply with an AppendSymlinks
    // change.
    let applied = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportApplied(a) => Some(a),
            _ => None,
        })
        .expect("ImportApplied expected");
    assert!(!applied.is_noop);
}

#[test]
fn import_reconcile_removes_missing_files() {
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[
            ("config.toml", b"x\n"),
            ("themes/dark.toml", b"name = 'dark'\n"),
        ],
    );
    // User deleted the symlink AND the repo copy by hand.
    env.delete_file(srpath!("home/bob/.config/helix/themes/dark.toml"));
    env.delete_file(srpath!(
        "home/bob/.config/zenops/configs/helix/themes/dark.toml"
    ));

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    env.run(&cmd).expect("reconcile should succeed");

    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["config.toml"]);

    // The empty `themes/` directory under the repo got pruned.
    assert!(
        !env.resolve_path(srpath!("home/bob/.config/zenops/configs/helix/themes"))
            .exists()
    );
}

#[test]
fn import_reconcile_mixed_add_and_remove() {
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[
            ("config.toml", b"x\n"),
            ("themes/dark.toml", b"name = 'dark'\n"),
        ],
    );
    // Drop one tracked file, add a new one.
    env.delete_file(srpath!("home/bob/.config/helix/themes/dark.toml"));
    env.delete_file(srpath!(
        "home/bob/.config/zenops/configs/helix/themes/dark.toml"
    ));
    env.write_file(
        srpath!("home/bob/.config/helix/keymap.toml"),
        b"q = 'quit'\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(symlinks, vec!["config.toml", "keymap.toml"]);

    // New file is a symlink; old repo copy and dir are gone.
    let new_link =
        std::fs::symlink_metadata(env.resolve_path(srpath!("home/bob/.config/helix/keymap.toml")))
            .unwrap();
    assert!(new_link.file_type().is_symlink());
    assert!(
        !env.resolve_path(srpath!(
            "home/bob/.config/zenops/configs/helix/themes/dark.toml"
        ))
        .exists()
    );
    assert!(
        !env.resolve_path(srpath!("home/bob/.config/zenops/configs/helix/themes"))
            .exists()
    );

    // Plan event reports both an append and a trim.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let kinds: Vec<_> = plan
        .toml_changes
        .iter()
        .map(|c| match c {
            ImportTomlChange::AppendSymlinks { .. } => "append",
            ImportTomlChange::TrimSymlinks { .. } => "trim",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["append", "trim"]);
}

#[test]
fn import_reconcile_no_op_marks_applied_noop() {
    let env = TestEnv::load();
    seed_managed_dotconfig(&env, "helix", None, &[("config.toml", b"x\n")]);
    let cfg_before =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile no-op should succeed");

    // Nothing in config.toml should have changed.
    let cfg_after =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();
    assert_eq!(cfg_after, cfg_before);

    let applied = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportApplied(a) => Some(a),
            _ => None,
        })
        .expect("ImportApplied expected");
    assert!(applied.is_noop);
}

#[test]
fn import_reconcile_dry_run_writes_nothing() {
    let env = TestEnv::load();
    seed_managed_dotconfig(&env, "helix", None, &[("config.toml", b"x\n")]);
    env.write_file(
        srpath!("home/bob/.config/helix/keymap.toml"),
        b"q = 'quit'\n",
    );
    let cfg_before =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: false,
        yes: false,
        dry_run: true,
    };
    let out = env.run(&cmd).expect("reconcile dry-run should succeed");

    // Source still a regular file.
    let src = env.resolve_path(srpath!("home/bob/.config/helix/keymap.toml"));
    assert!(
        std::fs::symlink_metadata(&src)
            .unwrap()
            .file_type()
            .is_file()
    );
    // No repo copy.
    assert!(
        !env.resolve_path(srpath!("home/bob/.config/zenops/configs/helix/keymap.toml"))
            .exists()
    );
    // config.toml unchanged.
    let cfg_after =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();
    assert_eq!(cfg_after, cfg_before);
    // Plan emitted, ImportApplied not.
    assert!(
        out.entries
            .iter()
            .any(|e| matches!(e, Entry::ImportPlan(_)))
    );
    assert!(
        out.entries
            .iter()
            .all(|e| !matches!(e, Entry::ImportApplied(_)))
    );
}

#[test]
fn import_reconcile_rejects_new_pkg_flags() {
    let env = TestEnv::load();
    seed_managed_dotconfig(&env, "helix", None, &[("config.toml", b"x\n")]);
    env.write_file(
        srpath!("home/bob/.config/helix/keymap.toml"),
        b"q = 'quit'\n",
    );

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix")),
        pkg: Some("other".into()),
        source: None,
        brew: vec!["helix".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    match env.run(&cmd) {
        Err(Error::Import(ImportError::ExtendFlagsInvalid { flags })) => {
            assert!(flags.contains(&"--pkg"));
            assert!(flags.contains(&"--brew"));
        }
        other => panic!("expected ExtendFlagsInvalid, got {other:?}"),
    }
}

#[test]
fn import_reconcile_skips_symlink_elsewhere() {
    let env = TestEnv::load();
    seed_managed_dotconfig(&env, "helix", None, &[("config.toml", b"x\n")]);
    // A user-managed symlink pointing outside the repo.
    env.write_file(srpath!("home/bob/elsewhere/cache.dat"), b"y\n");
    env.create_symlink(
        srpath!("home/bob/elsewhere/cache.dat"),
        srpath!("home/bob/.config/helix/cache.dat"),
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    // Symlink left untouched.
    let target =
        std::fs::read_link(env.resolve_path(srpath!("home/bob/.config/helix/cache.dat"))).unwrap();
    assert_eq!(
        target,
        env.resolve_path(srpath!("home/bob/elsewhere/cache.dat"))
    );

    // Array unchanged.
    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["config.toml"]);

    // Plan emits a Skip with reason "symlink_elsewhere".
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let has_skip = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::Skip { reason, .. } => reason == "symlink_elsewhere",
        _ => false,
    });
    assert!(
        has_skip,
        "expected symlink_elsewhere skip, got {:?}",
        plan.file_actions
    );
}

#[test]
fn import_reconcile_skips_present_but_not_linked() {
    let env = TestEnv::load();
    // Seed array with `config.toml` listed, but the home-side path is a
    // regular file (no symlink) — `apply` would convert it; reconcile
    // shouldn't touch it.
    let preset = dotconfig_preset("helix", None, &["config.toml"]);
    env.init_config(&preset);
    env.write_file(
        srpath!("home/bob/.config/zenops/configs/helix/config.toml"),
        b"x\n",
    );
    env.write_file(srpath!("home/bob/.config/helix/config.toml"), b"y\n");

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    // home-side still a regular file, repo copy still has its bytes.
    let src = env.resolve_path(srpath!("home/bob/.config/helix/config.toml"));
    assert!(
        std::fs::symlink_metadata(&src)
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(std::fs::read(&src).unwrap(), b"y\n");

    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let has_skip = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::Skip { reason, .. } => reason == "present_but_not_linked",
        _ => false,
    });
    assert!(has_skip);
}

#[test]
fn import_reconcile_home_type_entry() {
    // Reconcile a `type = "home"` entry (here pretending `gitcfg` lives
    // under `~/.local/share/gitcfg`).
    let env = TestEnv::load();
    let preset = format!(
        "{}\n\
         [pkg.gitcfg]\n\
         [pkg.gitcfg.install_hint.brew]\n\
         packages = [\"git\"]\n\
         [[pkg.gitcfg.configs]]\n\
         type = \"home\"\n\
         dir = \".local/share/gitcfg\"\n\
         source = \"configs/gitcfg\"\n\
         symlinks = [\"main.toml\"]\n",
        MINIMAL_CONFIG,
    );
    env.init_config(&preset);
    env.write_file(
        srpath!("home/bob/.config/zenops/configs/gitcfg/main.toml"),
        b"x\n",
    );
    env.create_symlink(
        srpath!("home/bob/.config/zenops/configs/gitcfg/main.toml"),
        srpath!("home/bob/.local/share/gitcfg/main.toml"),
    );
    env.write_file(
        srpath!("home/bob/.local/share/gitcfg/profiles/work.toml"),
        b"profile = 'work'\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.local/share/gitcfg")));
    env.run(&cmd).expect("home reconcile should succeed");

    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["gitcfg"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(symlinks, vec!["main.toml", "profiles/work.toml"]);
}

#[test]
fn import_reconcile_name_overridden_dotconfig() {
    // pkg key "neovim" but the on-disk dir is "nvim" (via name override).
    let env = TestEnv::load();
    seed_managed_dotconfig(&env, "neovim", Some("nvim"), &[("init.lua", b"-- start\n")]);
    env.write_file(
        srpath!("home/bob/.config/nvim/keymap.lua"),
        b"-- bindings\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/nvim")));
    env.run(&cmd)
        .expect("reconcile should match by name override");

    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["neovim"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(symlinks, vec!["init.lua", "keymap.lua"]);
}

#[test]
fn import_reconcile_strict_descendant_still_extends() {
    // Pointing at a single file under a managed root continues to hit
    // extend mode (one new entry appended), not reconcile.
    let env = TestEnv::load();
    seed_managed_dotconfig(&env, "helix", None, &[("config.toml", b"x\n")]);
    env.write_file(
        srpath!("home/bob/.config/helix/themes/dark.toml"),
        b"name = 'dark'\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix/themes/dark.toml")));
    let out = env.run(&cmd).expect("extend (single file) should succeed");

    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    // No TrimSymlinks ever fires from extend mode.
    let has_trim = plan
        .toml_changes
        .iter()
        .any(|c| matches!(c, ImportTomlChange::TrimSymlinks { .. }));
    assert!(!has_trim);
}
