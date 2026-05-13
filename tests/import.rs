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
fn import_dot_config_new_pkg_deeply_nested_file() {
    // The user's reported case: `zenops import ~/.config/some-app/dir/file.json`
    // when no [pkg.some-app] exists yet. Should create the pkg with a
    // single configs entry pointing at exactly that file.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        srpath!("home/bob/.config/some-app/dir/file.json"),
        b"{\"a\":1}\n",
    );
    env.write_file(srpath!("home/bob/.config/some-app/other.toml"), b"y\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/some-app/dir/file.json")),
        pkg: None,
        source: None,
        brew: vec!["some-app".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("deeply-nested import should succeed");

    let original = env.resolve_path(srpath!("home/bob/.config/some-app/dir/file.json"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink(),
    );
    let repo_copy = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/some-app/dir/file.json"
    ));
    assert_eq!(std::fs::read_link(&original).unwrap(), repo_copy);
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"{\"a\":1}\n");

    // Sibling untouched.
    let sibling = env.resolve_path(srpath!("home/bob/.config/some-app/other.toml"));
    assert!(
        std::fs::symlink_metadata(&sibling)
            .unwrap()
            .file_type()
            .is_file(),
    );

    let cfg = read_config(&env);
    let entry = &cfg["pkg"]["some-app"]["configs"][0];
    assert_eq!(entry["type"].as_str(), Some(".config"));
    assert_eq!(entry["source"].as_str(), Some("configs/some-app"));
    let symlinks: Vec<&str> = entry["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["dir/file.json"]);
}

#[test]
fn import_new_pkg_nested_file_collides_with_existing_pkg_key() {
    // [pkg.some-app] already exists with configs at a *different* on-disk
    // root (a home dotfile), so the nested .config/some-app/dir/file.json
    // path can't be folded into it. The user has to override with --pkg
    // or --source — same behavior as the existing collision guard for the
    // whole-directory case.
    let env = TestEnv::load();
    let preset = format!(
        "{}\n\
         [pkg.some-app]\n\
         [pkg.some-app.install_hint.brew]\n\
         packages = [\"some-app\"]\n\
         [[pkg.some-app.configs]]\n\
         type = \"home\"\n\
         dir = \".local/share/some-app\"\n\
         source = \"configs/some-app\"\n\
         symlinks = [\"main.toml\"]\n",
        MINIMAL_CONFIG,
    );
    env.init_config(&preset);
    env.write_file(srpath!("home/bob/.local/share/some-app/main.toml"), b"\n");
    env.write_file(
        srpath!("home/bob/.config/some-app/dir/file.json"),
        b"{\"a\":1}\n",
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/some-app/dir/file.json")));
    match env.run(&cmd) {
        Err(Error::Import(ImportError::PkgKeyTaken { pkg })) => {
            assert_eq!(pkg.as_str(), "some-app");
        }
        other => panic!("expected PkgKeyTaken, got {other:?}"),
    }
}

#[test]
fn import_rejects_nested_directory_for_new_pkg() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        srpath!("home/bob/.config/myapp/themes/onedark.toml"),
        b"theme = 'dark'\n",
    );

    // Nested *directory* (not a file) for a brand-new pkg: refused, because
    // single-file nested import is supported but recursive sub-dir import
    // is the user's call to make explicit (point at `.config/myapp` for
    // the whole pkg, or at a specific file under `themes/`).
    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/myapp/themes")));
    match env.run(&cmd) {
        Err(Error::Import(ImportError::NestedDirectoryNotSupported(p))) => {
            assert!(p.ends_with(".config/myapp/themes"), "got {p:?}");
        }
        other => panic!("expected NestedDirectoryNotSupported, got {other:?}"),
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
fn import_dot_config_new_pkg_nested_file_leaves_siblings_alone() {
    // ~/.config/unmanaged/ has no matching pkg, so this falls through to
    // the new-pkg path. We're pointing at a *nested* file and the import
    // should succeed: create pkg `unmanaged`, copy only that one file
    // into the repo, leave the sibling alone.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        srpath!("home/bob/.config/unmanaged/sub/file.toml"),
        b"x = 1\n",
    );
    env.write_file(srpath!("home/bob/.config/unmanaged/other.toml"), b"y = 2\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/unmanaged/sub/file.toml")),
        pkg: None,
        source: None,
        brew: Vec::new(),
        no_install_hint: true,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("nested-file import should succeed");

    // The targeted file is now a symlink into the repo, with its full
    // nested path preserved on the repo side.
    let original = env.resolve_path(srpath!("home/bob/.config/unmanaged/sub/file.toml"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink(),
    );
    let repo_copy = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/unmanaged/sub/file.toml"
    ));
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"x = 1\n");
    assert_eq!(std::fs::read_link(&original).unwrap(), repo_copy);

    // The sibling file is untouched — still a regular file, not copied.
    let sibling = env.resolve_path(srpath!("home/bob/.config/unmanaged/other.toml"));
    assert!(
        std::fs::symlink_metadata(&sibling)
            .unwrap()
            .file_type()
            .is_file(),
    );
    assert!(
        !env.resolve_path(srpath!(
            "home/bob/.config/zenops/configs/unmanaged/other.toml"
        ))
        .exists(),
        "sibling should not have been copied into the repo",
    );

    // config.toml has one configs entry covering only the imported file.
    let cfg = read_config(&env);
    let entries = cfg["pkg"]["unmanaged"]["configs"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["type"].as_str(), Some(".config"));
    assert_eq!(entries[0]["source"].as_str(), Some("configs/unmanaged"));
    let symlinks: Vec<&str> = entries[0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["sub/file.toml"]);
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

#[test]
fn import_reconcile_detects_rename() {
    // Stand up a managed `.config` project with two files, run `zenops
    // import` on the root, then rename one of the home-side symlinks
    // (which leaves the symlink pointing at the old repo path) and
    // re-run import. Reconcile should plan a rename: move the repo
    // file, retarget the symlink, swap the rels in the array.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/helix/config.toml"), b"foo = 1\n");
    env.write_file(
        srpath!("home/bob/.config/helix/themes/dark.toml"),
        b"theme = 'dark'\n",
    );

    // First import: brings ~/.config/helix under management.
    let first = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix")),
        pkg: None,
        source: None,
        brew: vec!["helix".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&first).expect("first import should succeed");

    // Sanity: original symlink is in place pointing at the old repo path.
    let old_link = env.resolve_path(srpath!("home/bob/.config/helix/themes/dark.toml"));
    let old_repo = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/helix/themes/dark.toml"
    ));
    assert_eq!(std::fs::read_link(&old_link).unwrap(), old_repo);

    // User renames the symlink in place: themes/dark.toml -> themes/dracula.toml.
    // The symlink's target still points at the old repo path.
    let new_link = env.resolve_path(srpath!("home/bob/.config/helix/themes/dracula.toml"));
    std::fs::rename(&old_link, &new_link).unwrap();
    assert_eq!(std::fs::read_link(&new_link).unwrap(), old_repo);

    // Re-run import on the root: reconcile mode kicks in.
    let second = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&second).expect("reconcile rename should succeed");

    // Plan event reports a single RenameInRepo file action (themes/dark.toml
    // -> themes/dracula.toml), no spurious add or remove for the same file.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let rename_pairs: Vec<(&std::path::Path, &std::path::Path)> = plan
        .file_actions
        .iter()
        .filter_map(|a| match a {
            ImportFileAction::RenameInRepo { from, to } => Some((from.as_path(), to.as_path())),
            _ => None,
        })
        .collect();
    assert_eq!(
        rename_pairs,
        vec![(
            std::path::Path::new("themes/dark.toml"),
            std::path::Path::new("themes/dracula.toml"),
        )],
    );
    let has_unrelated_remove = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::RemoveFromRepo { rel } => {
            rel.as_path() == std::path::Path::new("themes/dark.toml")
        }
        _ => false,
    });
    assert!(
        !has_unrelated_remove,
        "rename source should not also be a delete"
    );
    let has_unrelated_add = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::MoveAndSymlink { rel } => {
            rel.as_path() == std::path::Path::new("themes/dracula.toml")
        }
        _ => false,
    });
    assert!(
        !has_unrelated_add,
        "rename target should not also be a copy"
    );

    // Plan TOML side: the array transitions via paired Append+Trim with a
    // shared after-preview that lists `themes/dracula.toml`, not the old name.
    let after_preview = plan
        .toml_changes
        .iter()
        .find_map(|c| match c {
            ImportTomlChange::AppendSymlinks {
                array_after_preview,
                ..
            }
            | ImportTomlChange::TrimSymlinks {
                array_after_preview,
                ..
            } => Some(array_after_preview.as_str()),
            _ => None,
        })
        .expect("expected an AppendSymlinks or TrimSymlinks change");
    assert!(
        after_preview.contains("themes/dracula.toml"),
        "after-preview should mention new name, got {after_preview}",
    );
    assert!(
        !after_preview.contains("themes/dark.toml"),
        "after-preview should not mention old name, got {after_preview}",
    );

    // After apply: repo file moved.
    let new_repo = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/helix/themes/dracula.toml"
    ));
    assert!(new_repo.exists(), "repo file should be at the new path");
    assert_eq!(std::fs::read(&new_repo).unwrap(), b"theme = 'dark'\n");
    assert!(
        !old_repo.exists(),
        "repo file at the old path should be gone",
    );

    // Symlink retargeted to the new repo path (no longer dangling).
    let target = std::fs::read_link(&new_link).unwrap();
    assert_eq!(target, new_repo);

    // Array updated.
    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(symlinks, vec!["config.toml", "themes/dracula.toml"]);
}

// =====================================================================
// "Willy-nilly" user scenarios: lock current good behavior + spec the
// rough edges. Grouped by intent rather than by error variant so the
// tests read as a tour of how a real user might reach for `import`.
// =====================================================================

// --- Tier 1: lock current good behavior --------------------------------

#[test]
fn import_rejects_nonexistent_path() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);

    let typoed = env.resolve_path(srpath!("home/bob/.config/myapp-typoed"));
    let cmd = import_cmd(typoed.clone());
    match env.run(&cmd) {
        Err(Error::Import(ImportError::SourceMissing(p))) => {
            assert_eq!(p, typoed);
        }
        other => panic!("expected SourceMissing, got {other:?}"),
    }
}

#[test]
fn import_accepts_trailing_slash() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"foo = 1\n");

    // Append a trailing slash to the path the user passes in.
    let resolved = env.resolve_path(srpath!("home/bob/.config/myapp"));
    let with_slash = PathBuf::from(format!("{}/", resolved.display()));

    let cmd = Cmd::Import {
        path: with_slash,
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd)
        .expect("trailing slash should not change behavior");

    let original = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let cfg = read_config(&env);
    assert_eq!(
        cfg["pkg"]["myapp"]["configs"][0]["type"].as_str(),
        Some(".config")
    );
}

#[test]
fn import_accepts_dotdot_canonicalizing_to_managed() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"foo = 1\n");
    // Make sure the sibling we route through actually exists, so canonicalize
    // can resolve `..`.
    env.create_dir(srpath!("home/bob/.config/sibling"));

    let detour = env
        .resolve_path(srpath!("home/bob/.config/sibling"))
        .join("..")
        .join("myapp");

    let cmd = Cmd::Import {
        path: detour,
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd)
        .expect("path with .. should canonicalize and import");

    let cfg = read_config(&env);
    assert_eq!(
        cfg["pkg"]["myapp"]["configs"][0]["source"].as_str(),
        Some("configs/myapp")
    );
}

#[test]
fn import_rejects_path_outside_home() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    // A managed-shape-looking path that lives outside the synthetic home.
    env.write_file(srpath!("elsewhere/.config/myapp/config.toml"), b"x\n");

    let cmd = import_cmd(env.resolve_path(srpath!("elsewhere/.config/myapp")));
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::PathNotUnderHome(_)))),
        "got {result:?}",
    );
}

#[test]
fn import_rejects_source_that_is_a_symlink() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    // Real config dir lives outside ~/.config; user has a symlink
    // pointing at it from the standard location.
    env.write_file(srpath!("home/bob/dotfiles/myapp/config.toml"), b"x\n");
    env.create_symlink(
        srpath!("home/bob/dotfiles/myapp"),
        srpath!("home/bob/.config/myapp"),
    );

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/myapp")));
    match env.run(&cmd) {
        Err(Error::Import(ImportError::SourceIsSymlink(p))) => {
            assert!(
                p.ends_with(".config/myapp"),
                "expected .config/myapp tail, got {p:?}",
            );
        }
        other => panic!("expected SourceIsSymlink, got {other:?}"),
    }
}

#[test]
fn import_dot_config_new_pkg_single_file() {
    // Pointing at a single file directly under `~/.config/<pkg>/` creates
    // the pkg and imports just that one file.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");
    env.write_file(srpath!("home/bob/.config/myapp/other.toml"), b"y\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp/config.toml")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("single-file import should succeed");

    let original = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert!(
        std::fs::symlink_metadata(&original)
            .unwrap()
            .file_type()
            .is_symlink(),
    );

    // Sibling not touched.
    let sibling = env.resolve_path(srpath!("home/bob/.config/myapp/other.toml"));
    assert!(
        std::fs::symlink_metadata(&sibling)
            .unwrap()
            .file_type()
            .is_file(),
    );

    let cfg = read_config(&env);
    let entries = cfg["pkg"]["myapp"]["configs"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let symlinks: Vec<&str> = entries[0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["config.toml"]);
}

#[test]
fn import_rejects_empty_source_dir() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.create_dir(srpath!("home/bob/.config/myapp"));

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/myapp")));
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::SourceEmpty(_)))),
        "got {result:?}",
    );
}

#[test]
fn import_walks_subdirectories_recursively() {
    // A user with a nested config tree (top-level files plus a themes/
    // subdir) should see every regular file end up in the symlinks array,
    // each with its forward-slash relative path.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/helix/top.toml"), b"top\n");
    env.write_file(
        srpath!("home/bob/.config/helix/themes/dark.toml"),
        b"name = 'dark'\n",
    );

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix")),
        pkg: None,
        source: None,
        brew: vec!["helix".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("recursive import should succeed");

    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(symlinks, vec!["themes/dark.toml", "top.toml"]);

    // Each one is a real symlink in the home tree, pointing at its repo
    // copy.
    for rel in ["top.toml", "themes/dark.toml"] {
        let home_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{rel}").parse().unwrap();
        let meta = std::fs::symlink_metadata(env.resolve_path(&home_rel)).unwrap();
        assert!(meta.file_type().is_symlink(), "{rel} should be a symlink");
    }
}

#[test]
fn import_skips_dangling_symlink_in_source() {
    // A common shape: source dir has a regular file plus a symlink to a
    // path that no longer exists. The regular file imports normally; the
    // dangling link gets surfaced as a Skip.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"real\n");
    env.create_dangling_symlink(
        std::path::Path::new("/tmp/zenops-test-nonexistent"),
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

    // Dangling symlink left alone (and still dangling).
    let cache = env.resolve_path(srpath!("home/bob/.config/myapp/cache.dat"));
    assert!(
        std::fs::symlink_metadata(&cache)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // Plan flagged a Skip with reason "symlink".
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("expected ImportPlan");
    let dangling_skipped = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::Skip { path, reason } => {
            path == std::path::Path::new("cache.dat") && reason == "symlink"
        }
        _ => false,
    });
    assert!(
        dangling_skipped,
        "expected dangling cache.dat to be skipped: {:?}",
        plan.file_actions,
    );

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
fn import_rejects_malformed_config_toml() {
    let env = TestEnv::load();
    env.init_config("not = valid =\n");
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/myapp")));
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::ConfigParse(_, _)))),
        "got {result:?}",
    );
}

#[test]
fn import_rejects_when_no_tty_and_no_yes() {
    // TestEnv::default_args has stdin_is_terminal=false, so dropping --yes
    // (without --dry-run) hits the NeedsTty refusal before any path check.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: false,
        dry_run: false,
    };
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::NeedsTty))),
        "got {result:?}",
    );
}

#[test]
fn import_accepts_custom_source_override() {
    // --source picks an alternative in-repo destination directory.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"foo = 1\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: Some("configs/custom-name".into()),
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("--source override should succeed");

    // Repo copy lives under the overridden path.
    let repo_copy = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/custom-name/config.toml"
    ));
    assert_eq!(std::fs::read(&repo_copy).unwrap(), b"foo = 1\n");

    // Symlink target points there.
    let symlink = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert_eq!(std::fs::read_link(&symlink).unwrap(), repo_copy);

    // TOML records the overridden source.
    let cfg = read_config(&env);
    assert_eq!(
        cfg["pkg"]["myapp"]["configs"][0]["source"].as_str(),
        Some("configs/custom-name"),
    );
}

#[test]
fn import_rejects_custom_source_with_pre_existing_files() {
    // --source pointing at a dir already populated with non-symlink files
    // gets refused via the standard DestExists pathway.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_zenops_file(srpath!("configs/custom/config.toml"), b"old\n", None);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"new\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: Some("configs/custom".into()),
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    assert!(
        matches!(result, Err(Error::Import(ImportError::DestExists(_)))),
        "got {result:?}",
    );
}

#[test]
fn import_records_multiple_brew_packages() {
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["foo".into(), "bar".into(), "baz".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd)
        .expect("import with multiple --brew should succeed");

    let cfg = read_config(&env);
    let packages: Vec<&str> = cfg["pkg"]["myapp"]["install_hint"]["brew"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(packages, vec!["foo", "bar", "baz"]);
}

// --- Tier 2: assert desired UX (some currently fail) -------------------

#[test]
fn import_refuses_when_zenops_repo_missing() {
    // No init_config: the user hasn't run `zenops init` yet. Importing
    // should refuse with a friendly message pointing at `zenops init`,
    // not a bare I/O NotFound.
    let env = TestEnv::load();
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
    let result = env.run(&cmd);
    let Err(err) = result else {
        panic!("expected refusal, got {result:?}");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("zenops init"),
        "error should point at `zenops init`, got: {msg}",
    );
}

#[test]
fn import_refuses_to_import_zenops_repo_itself() {
    // The user, exploring, points at their zenops repo dir. We can't
    // recurse the repo into itself; refuse with a dedicated message.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/zenops")),
        pkg: None,
        source: None,
        brew: vec!["zenops".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    let Err(err) = result else {
        panic!("expected refusal, got {result:?}");
    };
    let msg = err.to_string();
    assert!(
        msg.to_ascii_lowercase().contains("zenops"),
        "error should reference the zenops repo, got: {msg}",
    );

    // Critically: nothing got copied into the repo.
    let configs_dir = env.resolve_path(srpath!("home/bob/.config/zenops/configs"));
    assert!(
        !configs_dir.exists() || std::fs::read_dir(&configs_dir).unwrap().next().is_none(),
        "no files should have been copied into the zenops repo's configs/ dir",
    );
}

#[test]
fn import_rejects_regular_file_at_dot_config_slot() {
    // ~/.config/<x> is sometimes a regular file (single-file configs).
    // We don't support that shape today; refuse cleanly rather than
    // leaking a raw NotADirectory IO error.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp"), b"# single-file config\n");

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/myapp")));
    let result = env.run(&cmd);
    let Err(err) = result else {
        panic!("expected refusal, got {result:?}");
    };
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("director") || msg.contains("layout"),
        "error should mention directory/layout, got: {msg}",
    );
}

#[test]
fn import_skips_vcs_subdir_in_source() {
    // User points at a directory that happens to contain a `.git` subdir
    // (e.g. they cloned their dotfiles into ~/.config/myapp). We must
    // never recursively import the git repo — that copies thousands of
    // pack files into the zenops repo.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x = 1\n");
    env.write_file(
        srpath!("home/bob/.config/myapp/.git/HEAD"),
        b"ref: refs/heads/main\n",
    );
    env.write_file(srpath!("home/bob/.config/myapp/.git/config"), b"[core]\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let out = env.run(&cmd).expect("import should succeed; .git skipped");

    // .git contents not symlinked into the home dir's `.git/` (the
    // original tree is preserved, and no new `.git/HEAD -> ...` symlink
    // appears that would break the repo).
    let head = env.resolve_path(srpath!("home/bob/.config/myapp/.git/HEAD"));
    let meta = std::fs::symlink_metadata(&head).unwrap();
    assert!(
        meta.file_type().is_file(),
        ".git/HEAD must remain a regular file (not a symlink)",
    );

    // .git contents not in the repo copy.
    let repo_git_dir = env.resolve_path(srpath!("home/bob/.config/zenops/configs/myapp/.git"));
    assert!(
        !repo_git_dir.exists(),
        ".git must not be copied into the zenops repo",
    );

    // Plan reports the skip with a vcs reason (so json consumers see why
    // we ignored it).
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let saw_vcs_skip = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::Skip { path, reason } => path.starts_with(".git") && reason == "vcs",
        _ => false,
    });
    assert!(
        saw_vcs_skip,
        "expected a `.git` skip with reason=\"vcs\", got {:?}",
        plan.file_actions,
    );

    // config.toml only mentions the regular file.
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
fn import_refuses_source_override_outside_repo() {
    // --source must stay inside ~/.config/zenops. An absolute override or
    // one that escapes via `..` could land files anywhere on disk —
    // refuse it.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: Some("../escape".into()),
        brew: vec!["myapp".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    let Err(err) = result else {
        panic!("expected refusal, got {result:?}");
    };
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("source") || msg.contains("outside") || msg.contains("escape"),
        "error should mention --source / outside-repo, got: {msg}",
    );

    // Nothing landed outside the repo.
    let escaped = env.resolve_path(srpath!("home/bob/.config/escape"));
    assert!(
        !escaped.exists(),
        "no files should have escaped the zenops repo",
    );
}

#[test]
fn import_refuses_pkg_key_collision() {
    // First import claims pkg `foo` for ~/.config/foo. Second import of
    // ~/.foo (which would derive the same default pkg key) should refuse
    // with a clear collision message rather than silently piling a second
    // configs entry into [pkg.foo].
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/foo/config.toml"), b"a\n");
    env.write_file(srpath!("home/bob/.foo/init"), b"b\n");

    let first = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/foo")),
        pkg: None,
        source: None,
        brew: vec!["foo".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&first).expect("first import should succeed");

    let second = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.foo")),
        pkg: None,
        source: None,
        brew: vec!["foo".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&second);
    let Err(err) = result else {
        panic!("expected refusal on default-key collision, got {result:?}");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("foo"),
        "error should name the colliding pkg key `foo`, got: {msg}",
    );
    assert!(
        msg.contains("--pkg")
            || msg.contains("--source")
            || msg.to_ascii_lowercase().contains("already"),
        "error should suggest --pkg / --source or reference an existing pkg, got: {msg}",
    );

    // Second import didn't mutate state.
    let dotfoo_init = env.resolve_path(srpath!("home/bob/.foo/init"));
    assert!(
        std::fs::symlink_metadata(&dotfoo_init)
            .unwrap()
            .file_type()
            .is_file(),
        "~/.foo/init should still be a regular file, not a symlink",
    );
}

#[test]
fn import_refuses_brew_with_empty_string() {
    // `--brew ""` would land an empty string into install_hint.brew.packages,
    // which is never useful. Refuse early.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/myapp")),
        pkg: None,
        source: None,
        brew: vec!["".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    assert!(
        result.is_err(),
        "empty --brew value should be refused, got {result:?}",
    );

    // Confirm we didn't write a malformed install_hint to disk.
    let cfg_text =
        std::fs::read_to_string(env.resolve_path(srpath!("home/bob/.config/zenops/config.toml")))
            .unwrap();
    assert!(
        !cfg_text.contains("packages = [\"\"]"),
        "should not have written empty brew package, got config.toml:\n{cfg_text}",
    );
}

#[test]
fn import_rejects_config_root_itself() {
    // ~/.config/ is an alias for "everything I configure". Importing it
    // would recursively walk every app dir, including ~/.config/zenops
    // itself. Refuse with a hint at the supported shapes.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    // Seed a couple of real config dirs so the would-be walk would have
    // something to grab.
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"x\n");
    env.write_file(srpath!("home/bob/.config/other/init"), b"y\n");

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config")),
        pkg: None,
        source: None,
        brew: vec!["whatever".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    let Err(err) = result else {
        panic!("expected refusal for ~/.config/ root, got {result:?}");
    };
    let msg = err.to_string();
    assert!(
        msg.contains(".config/<x>")
            || msg.contains(".<x>")
            || msg.to_ascii_lowercase().contains("config root"),
        "error should hint at supported shapes, got: {msg}",
    );

    // Source dirs untouched.
    let myapp_cfg = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    assert!(
        std::fs::symlink_metadata(&myapp_cfg)
            .unwrap()
            .file_type()
            .is_file(),
        "no nested file should have been symlinked",
    );
}

// --- Tier 3: message-tweak / lower priority ---------------------------

#[test]
fn import_refuses_home_root_with_helpful_message() {
    // ~/ is the deepest "I want everything" mistake. Today the layout
    // check fires with an empty-tail UnsupportedLayout message; we want
    // a friendlier, non-empty hint.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob")),
        pkg: None,
        source: None,
        brew: vec!["whatever".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    let result = env.run(&cmd);
    let Err(err) = result else {
        panic!("expected refusal for ~/, got {result:?}");
    };
    let msg = err.to_string();
    // The diagnostic must actually say something useful — the bare empty
    // tail (`""`) on its own is not a helpful message.
    let trimmed = msg.replace('"', "");
    assert!(
        trimmed.to_ascii_lowercase().contains("home")
            || trimmed.contains(".config/<x>")
            || trimmed.to_ascii_lowercase().contains("config dir"),
        "error should give a clearer hint than an empty tail, got: {msg}",
    );
}

// =====================================================================
// Realistic-content / multi-step user journeys. These exercise import
// the way a real user with a populated config dir would: many files of
// varied shapes, edits over time, and reconcile passes that mix
// add/delete/rename across multiple files.
// =====================================================================

#[test]
fn import_handles_diverse_file_shapes() {
    // Populate a managed dir with the kinds of files a real Helix-style
    // config tree contains: 12 files spanning mixed extensions, nested
    // subdirs (up to 3 levels), an empty file, a binary blob, names with
    // spaces and unicode, and a root-level README. After one import
    // every file must land as a symlink, every repo copy must be
    // byte-identical, and the symlinks array must list all 12 paths.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);

    let binary_blob: Vec<u8> = (0u8..=255u8).chain(0u8..=120u8).collect();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("config.toml", b"theme = 'dracula'\n".to_vec()),
        (
            "README.md",
            b"# helix\n\nKey bindings live in keymap.json.\n".to_vec(),
        ),
        ("init.lua", b"-- editor init\n".to_vec()),
        ("keymap.json", br#"{"q":"quit"}"#.to_vec()),
        ("themes/dark.toml", b"name = 'dark'\n".to_vec()),
        ("themes/dracula.toml", b"name = 'dracula'\n".to_vec()),
        ("runtime/grammars/rust.so", binary_blob.clone()),
        (
            "runtime/queries/rust/highlights.scm",
            b"(identifier) @variable\n".to_vec(),
        ),
        (
            "colors/onedark.vim",
            b"\" onedark\nset background=dark\n".to_vec(),
        ),
        ("notes/empty.txt", Vec::new()),
        ("notes/spaces in name.toml", b"key = 'value'\n".to_vec()),
        ("notes/üñíçødé.toml", b"unicode = true\n".to_vec()),
    ];

    for (rel, data) in &files {
        let home_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{rel}").parse().unwrap();
        env.write_file(&home_rel, data.as_slice());
    }

    let cmd = Cmd::Import {
        path: env.resolve_path(srpath!("home/bob/.config/helix")),
        pkg: None,
        source: None,
        brew: vec!["helix".into()],
        no_install_hint: false,
        yes: true,
        dry_run: false,
    };
    env.run(&cmd).expect("import should succeed");

    // Every file is now a symlink on the home side, and every repo copy
    // matches the source bytes exactly (including the empty file and the
    // binary blob).
    for (rel, data) in &files {
        let home_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{rel}").parse().unwrap();
        let home_path = env.resolve_path(&home_rel);
        let meta = std::fs::symlink_metadata(&home_path)
            .unwrap_or_else(|e| panic!("missing home-side path {rel}: {e}"));
        assert!(meta.file_type().is_symlink(), "{rel} should be a symlink",);

        let repo_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{rel}")
                .parse()
                .unwrap();
        let repo_path = env.resolve_path(&repo_rel);
        let repo_bytes = std::fs::read(&repo_path)
            .unwrap_or_else(|e| panic!("missing repo copy for {rel}: {e}"));
        assert_eq!(
            repo_bytes, *data,
            "repo copy for {rel} must be byte-identical to source",
        );
        assert_eq!(
            std::fs::read_link(&home_path).unwrap(),
            repo_path,
            "{rel} symlink should point at its repo copy",
        );
    }

    // symlinks array (sorted) lists every file with forward-slash separators,
    // including the unicode and spaces-in-name entries.
    let cfg = read_config(&env);
    let mut got: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    got.sort();
    let mut expected: Vec<&str> = files.iter().map(|(rel, _)| *rel).collect();
    expected.sort();
    assert_eq!(got, expected);
}

#[test]
fn import_preserves_executable_mode_bits() {
    // A user keeps an executable script alongside a regular config file
    // in their managed dir. After import the repo copy of the script
    // must keep its executable bits (otherwise the symlinked script is
    // useless), and the regular file must not gain execute permission.
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        srpath!("home/bob/.config/myapp/config.toml"),
        b"key = 'value'\n",
    );
    env.write_file(
        srpath!("home/bob/.config/myapp/scripts/runme.sh"),
        b"#!/bin/sh\necho hi\n",
    );
    let _exec_guard = env.chmod(srpath!("home/bob/.config/myapp/scripts/runme.sh"), 0o755);

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

    let exec_repo = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/myapp/scripts/runme.sh"
    ));
    let exec_mode = std::fs::metadata(&exec_repo).unwrap().permissions().mode() & 0o777;
    assert!(
        exec_mode & 0o100 != 0,
        "repo copy of runme.sh should have user-execute bit set, got mode {exec_mode:o}",
    );

    let plain_repo = env.resolve_path(srpath!("home/bob/.config/zenops/configs/myapp/config.toml"));
    let plain_mode = std::fs::metadata(&plain_repo).unwrap().permissions().mode() & 0o777;
    assert!(
        plain_mode & 0o111 == 0,
        "repo copy of config.toml must not be executable, got mode {plain_mode:o}",
    );

    // Both home-side files are symlinks and resolve to the repo copies.
    for (home, repo) in [
        (
            srpath!("home/bob/.config/myapp/scripts/runme.sh"),
            &exec_repo,
        ),
        (srpath!("home/bob/.config/myapp/config.toml"), &plain_repo),
    ] {
        let home_path = env.resolve_path(home);
        assert!(
            std::fs::symlink_metadata(&home_path)
                .unwrap()
                .file_type()
                .is_symlink(),
        );
        assert_eq!(&std::fs::read_link(&home_path).unwrap(), repo);
    }
}

#[test]
fn import_write_through_symlink_propagates_bytes() {
    // After import, the home-side path is a symlink. A user editing
    // through it (the natural `:w` in their editor) should see the new
    // bytes appear on both sides without breaking the symlink.
    let env = TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(srpath!("home/bob/.config/myapp/config.toml"), b"v1\n");

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

    let home_path = env.resolve_path(srpath!("home/bob/.config/myapp/config.toml"));
    let repo_path = env.resolve_path(srpath!("home/bob/.config/zenops/configs/myapp/config.toml"));
    let original_target = std::fs::read_link(&home_path).unwrap();

    // Write through the symlink — std::fs::write follows symlinks on Unix.
    std::fs::write(&home_path, b"v2\n").unwrap();

    // Both sides see the new bytes.
    assert_eq!(std::fs::read(&home_path).unwrap(), b"v2\n");
    assert_eq!(std::fs::read(&repo_path).unwrap(), b"v2\n");

    // Symlink unchanged: still a symlink, still pointing at the same target.
    let meta = std::fs::symlink_metadata(&home_path).unwrap();
    assert!(meta.file_type().is_symlink());
    assert_eq!(std::fs::read_link(&home_path).unwrap(), original_target);
}

/// Compose a config.toml fragment with two managed `.config` pkgs back-to-back.
/// Used by the multi-pkg isolation test; `dotconfig_preset` only handles one
/// pkg at a time.
fn two_dotconfig_pkgs(
    a_key: &str,
    a_symlinks: &[&str],
    b_key: &str,
    b_symlinks: &[&str],
) -> String {
    let fmt_block = |key: &str, symlinks: &[&str]| {
        let arr: Vec<String> = symlinks.iter().map(|s| format!("\"{s}\"")).collect();
        format!(
            "[pkg.{key}]\n\
             [pkg.{key}.install_hint.brew]\n\
             packages = [\"{key}\"]\n\
             [[pkg.{key}.configs]]\n\
             type = \".config\"\n\
             source = \"configs/{key}\"\n\
             symlinks = [{}]\n",
            arr.join(", "),
        )
    };
    format!(
        "{}\n{}\n{}",
        MINIMAL_CONFIG,
        fmt_block(a_key, a_symlinks),
        fmt_block(b_key, b_symlinks),
    )
}

#[test]
fn import_reconcile_renames_multiple_files_in_one_batch() {
    // Three home-side symlinks renamed in a single user session before
    // a reconcile pass. Each rename leaves the symlink pointing at its
    // old repo path, mirroring the single-file case in
    // `import_reconcile_detects_rename`. The plan should detect all
    // three as renames, not as add+remove pairs.
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[
            ("config.toml", b"x\n"),
            ("themes/dark.toml", b"name = 'dark'\n"),
            ("themes/light.toml", b"name = 'light'\n"),
            ("keymap.toml", b"q = 'quit'\n"),
        ],
    );

    let renames: &[(&str, &str)] = &[
        ("themes/dark.toml", "themes/midnight.toml"),
        ("themes/light.toml", "themes/daylight.toml"),
        ("keymap.toml", "bindings.toml"),
    ];
    for (from, to) in renames {
        let from_path: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{from}").parse().unwrap();
        let to_path: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{to}").parse().unwrap();
        std::fs::rename(env.resolve_path(&from_path), env.resolve_path(&to_path)).unwrap();
    }

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    // Plan reports exactly 3 RenameInRepo actions, no spurious add/remove
    // for any of the renamed rels.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let mut rename_pairs: Vec<(String, String)> = plan
        .file_actions
        .iter()
        .filter_map(|a| match a {
            ImportFileAction::RenameInRepo { from, to } => Some((
                from.to_string_lossy().into_owned(),
                to.to_string_lossy().into_owned(),
            )),
            _ => None,
        })
        .collect();
    rename_pairs.sort();
    let mut expected: Vec<(String, String)> = renames
        .iter()
        .map(|(f, t)| ((*f).to_string(), (*t).to_string()))
        .collect();
    expected.sort();
    assert_eq!(
        rename_pairs, expected,
        "expected 3 renames, got file_actions: {:?}",
        plan.file_actions,
    );
    let stray_remove = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::RemoveFromRepo { rel } => renames
            .iter()
            .any(|(from, _)| rel.as_path() == std::path::Path::new(from)),
        _ => false,
    });
    assert!(!stray_remove, "no rename source should also be a delete");
    let stray_add = plan.file_actions.iter().any(|a| match a {
        ImportFileAction::MoveAndSymlink { rel } => renames
            .iter()
            .any(|(_, to)| rel.as_path() == std::path::Path::new(to)),
        _ => false,
    });
    assert!(!stray_add, "no rename target should also be a copy");

    // End state: every renamed repo file lives at its new path and the
    // home-side symlink points there.
    for (from, to) in renames {
        let old_repo: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{from}")
                .parse()
                .unwrap();
        let new_repo: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{to}")
                .parse()
                .unwrap();
        let new_link: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{to}").parse().unwrap();
        assert!(
            !env.resolve_path(&old_repo).exists(),
            "old repo path {from} should be gone",
        );
        assert!(
            env.resolve_path(&new_repo).exists(),
            "new repo path {to} should exist",
        );
        assert_eq!(
            std::fs::read_link(env.resolve_path(&new_link)).unwrap(),
            env.resolve_path(&new_repo),
            "{to} symlink should retarget to new repo path",
        );
    }

    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(
        symlinks,
        vec![
            "bindings.toml",
            "config.toml",
            "themes/daylight.toml",
            "themes/midnight.toml",
        ],
    );
}

#[test]
fn import_reconcile_detects_subdir_rename() {
    // The user renames a whole subdir on the home side. Each child
    // symlink under it still points at the old repo path. End-state
    // assertions are strict; the plan-shape (rename vs add+remove) is
    // recorded as an observation since rename detection currently works
    // per-file rather than per-dir.
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[
            ("themes/dark.toml", b"name = 'dark'\n"),
            ("themes/light.toml", b"name = 'light'\n"),
        ],
    );

    let themes_dir = env.resolve_path(srpath!("home/bob/.config/helix/themes"));
    let colors_dir = env.resolve_path(srpath!("home/bob/.config/helix/colors"));
    std::fs::rename(&themes_dir, &colors_dir).unwrap();

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    // End-state checks (independent of plan shape).
    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(symlinks, vec!["colors/dark.toml", "colors/light.toml"]);

    for name in ["dark.toml", "light.toml"] {
        let new_link: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/colors/{name}")
                .parse()
                .unwrap();
        let new_repo: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/colors/{name}")
                .parse()
                .unwrap();
        assert!(
            env.resolve_path(&new_repo).exists(),
            "new repo path colors/{name} should exist",
        );
        assert_eq!(
            std::fs::read_link(env.resolve_path(&new_link)).unwrap(),
            env.resolve_path(&new_repo),
        );
    }
    assert!(
        !env.resolve_path(srpath!("home/bob/.config/zenops/configs/helix/themes"))
            .exists(),
        "old themes/ dir should be pruned from repo",
    );

    // Plan-shape observation: record whether the implementation modeled
    // this as 2 renames or as add+remove pairs. Either is acceptable
    // end-to-end; the user can decide whether to extend rename detection.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let renames = plan
        .file_actions
        .iter()
        .filter(|a| matches!(a, ImportFileAction::RenameInRepo { .. }))
        .count();
    let removes = plan
        .file_actions
        .iter()
        .filter(|a| matches!(a, ImportFileAction::RemoveFromRepo { .. }))
        .count();
    let copies = plan
        .file_actions
        .iter()
        .filter(|a| matches!(a, ImportFileAction::MoveAndSymlink { .. }))
        .count();
    // Either: 2 renames (and no add/remove) OR 2 add + 2 remove. Observed:
    // current rename detector recognizes both children individually
    // (renames=2), since each home-side symlink still points at its old
    // repo path.
    let detected_as_renames = renames == 2 && removes == 0 && copies == 0;
    let detected_as_add_remove = renames == 0 && removes == 2 && copies == 2;
    assert!(
        detected_as_renames || detected_as_add_remove,
        "subdir rename plan should be either 2 renames or 2 add+2 remove, got: \
         renames={renames} removes={removes} copies={copies}; actions={:?}",
        plan.file_actions,
    );
}

#[test]
fn import_reconcile_handles_cross_subdir_restructure() {
    // The user moves a single file across subdirs (themes/dark.toml ->
    // colors/dark.toml). The home-side symlink still points at the old
    // repo path, same mechanism as the existing single-file rename test
    // but with a different parent dir.
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[("themes/dark.toml", b"name = 'dark'\n")],
    );

    let from = env.resolve_path(srpath!("home/bob/.config/helix/themes/dark.toml"));
    let to = env.resolve_path(srpath!("home/bob/.config/helix/colors/dark.toml"));
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::rename(&from, &to).unwrap();

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    let cfg = read_config(&env);
    let symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(symlinks, vec!["colors/dark.toml"]);

    let new_repo = env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/helix/colors/dark.toml"
    ));
    assert!(new_repo.exists());
    assert!(
        !env.resolve_path(srpath!(
            "home/bob/.config/zenops/configs/helix/themes/dark.toml"
        ))
        .exists()
    );
    assert!(
        !env.resolve_path(srpath!("home/bob/.config/zenops/configs/helix/themes"))
            .exists(),
        "empty themes/ dir should be pruned",
    );
    assert_eq!(std::fs::read_link(&to).unwrap(), new_repo);

    // Plan-shape: rename vs add+remove. Either is fine end-to-end.
    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");
    let detected_as_rename = plan.file_actions.iter().any(|a| {
        matches!(a, ImportFileAction::RenameInRepo { from, to }
            if from.as_path() == std::path::Path::new("themes/dark.toml")
                && to.as_path() == std::path::Path::new("colors/dark.toml"))
    });
    let detected_as_add_remove = plan.file_actions.iter().any(|a| {
        matches!(a, ImportFileAction::RemoveFromRepo { rel }
            if rel.as_path() == std::path::Path::new("themes/dark.toml"))
    }) && plan.file_actions.iter().any(|a| {
        matches!(a, ImportFileAction::MoveAndSymlink { rel }
                if rel.as_path() == std::path::Path::new("colors/dark.toml"))
    });
    // Observed: cross-subdir restructure is detected as a rename, since the
    // home-side symlink still points at its old repo path.
    assert!(
        detected_as_rename || detected_as_add_remove,
        "cross-subdir restructure should plan as rename or add+remove, got: {:?}",
        plan.file_actions,
    );
}

#[test]
fn import_reconcile_handles_mixed_add_delete_rename() {
    // A single reconcile pass with three classes of change at once: two
    // new files in a brand-new subdir, two deletes, and one rename. The
    // plan should pick up each one in the right shape, and the array
    // should land at the expected final state with no orphan repo files.
    let env = TestEnv::load();
    seed_managed_dotconfig(
        &env,
        "helix",
        None,
        &[
            ("config.toml", b"x\n"),
            ("themes/dark.toml", b"name = 'dark'\n"),
            ("themes/light.toml", b"name = 'light'\n"),
            ("keymap.toml", b"q = 'quit'\n"),
        ],
    );

    // Two adds (top-level new subdir).
    env.write_file(
        srpath!("home/bob/.config/helix/colors/red.toml"),
        b"hue = 'red'\n",
    );
    env.write_file(
        srpath!("home/bob/.config/helix/colors/blue.toml"),
        b"hue = 'blue'\n",
    );

    // Two deletes — drop the home-side symlinks; reconcile will trim the
    // repo copies through RemoveFromRepo.
    env.delete_file(srpath!("home/bob/.config/helix/themes/light.toml"));
    env.delete_file(srpath!("home/bob/.config/helix/keymap.toml"));

    // One rename — move dark.toml -> midnight.toml in place.
    let from = env.resolve_path(srpath!("home/bob/.config/helix/themes/dark.toml"));
    let to = env.resolve_path(srpath!("home/bob/.config/helix/themes/midnight.toml"));
    std::fs::rename(&from, &to).unwrap();

    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    let out = env.run(&cmd).expect("reconcile should succeed");

    let plan = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::ImportPlan(p) => Some(p),
            _ => None,
        })
        .expect("ImportPlan expected");

    // Shape-counts: at least 2 MoveAndSymlink (new files), at least 2
    // RemoveFromRepo (deleted files), exactly 1 RenameInRepo.
    let copies = plan
        .file_actions
        .iter()
        .filter(|a| matches!(a, ImportFileAction::MoveAndSymlink { .. }))
        .count();
    let removes = plan
        .file_actions
        .iter()
        .filter(|a| matches!(a, ImportFileAction::RemoveFromRepo { .. }))
        .count();
    let renames = plan
        .file_actions
        .iter()
        .filter(|a| matches!(a, ImportFileAction::RenameInRepo { .. }))
        .count();
    assert!(
        copies >= 2,
        "expected ≥2 MoveAndSymlink, got {copies}; actions={:?}",
        plan.file_actions,
    );
    assert!(
        removes >= 2,
        "expected ≥2 RemoveFromRepo, got {removes}; actions={:?}",
        plan.file_actions,
    );
    assert_eq!(
        renames, 1,
        "expected exactly 1 RenameInRepo, got {renames}; actions={:?}",
        plan.file_actions,
    );

    // End state: array sorted matches the expected 4 surviving rels.
    let cfg = read_config(&env);
    let mut symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    symlinks.sort();
    assert_eq!(
        symlinks,
        vec![
            "colors/blue.toml",
            "colors/red.toml",
            "config.toml",
            "themes/midnight.toml",
        ],
    );

    // Repo files: new ones present, deleted ones gone, renamed at new path.
    for rel in [
        "colors/red.toml",
        "colors/blue.toml",
        "config.toml",
        "themes/midnight.toml",
    ] {
        let p: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{rel}")
                .parse()
                .unwrap();
        assert!(env.resolve_path(&p).exists(), "{rel} should exist in repo");
    }
    for rel in ["themes/dark.toml", "themes/light.toml", "keymap.toml"] {
        let p: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{rel}")
                .parse()
                .unwrap();
        assert!(
            !env.resolve_path(&p).exists(),
            "{rel} should be gone from repo",
        );
    }

    // Symlinks resolve to their new repo paths (no danglers among the survivors).
    for rel in [
        "colors/red.toml",
        "colors/blue.toml",
        "config.toml",
        "themes/midnight.toml",
    ] {
        let home_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{rel}").parse().unwrap();
        let repo_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{rel}")
                .parse()
                .unwrap();
        assert_eq!(
            std::fs::read_link(env.resolve_path(&home_rel)).unwrap(),
            env.resolve_path(&repo_rel),
        );
    }
}

#[test]
fn import_isolates_pkgs_during_reconcile() {
    // Two pkgs are managed side-by-side. Adding a new file under one and
    // running reconcile on it must leave the other pkg's array, repo
    // files, and symlinks untouched.
    let env = TestEnv::load();
    let preset = two_dotconfig_pkgs(
        "helix",
        &["config.toml", "themes/dark.toml"],
        "nvim",
        &["init.lua", "lua/keymaps.lua"],
    );
    env.init_config(&preset);

    // Seed both pkgs' on-disk state.
    let helix_files = [
        ("config.toml", b"theme = 'dark'\n".as_slice()),
        ("themes/dark.toml", b"name = 'dark'\n".as_slice()),
    ];
    for (rel, body) in helix_files {
        let repo_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/helix/{rel}")
                .parse()
                .unwrap();
        let home_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/helix/{rel}").parse().unwrap();
        env.write_file(&repo_rel, body);
        env.create_symlink(&repo_rel, &home_rel);
    }
    let nvim_files = [
        ("init.lua", b"-- nvim\n".as_slice()),
        ("lua/keymaps.lua", b"-- keymaps\n".as_slice()),
    ];
    for (rel, body) in nvim_files {
        let repo_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/zenops/configs/nvim/{rel}")
                .parse()
                .unwrap();
        let home_rel: zenops_safe_relative_path::SafeRelativePathBuf =
            format!("home/bob/.config/nvim/{rel}").parse().unwrap();
        env.write_file(&repo_rel, body);
        env.create_symlink(&repo_rel, &home_rel);
    }

    // Capture nvim baseline before any change.
    let nvim_init_target_before =
        std::fs::read_link(env.resolve_path(srpath!("home/bob/.config/nvim/init.lua"))).unwrap();
    let nvim_keymaps_target_before =
        std::fs::read_link(env.resolve_path(srpath!("home/bob/.config/nvim/lua/keymaps.lua")))
            .unwrap();
    let nvim_init_bytes_before =
        std::fs::read(env.resolve_path(srpath!("home/bob/.config/zenops/configs/nvim/init.lua")))
            .unwrap();
    let nvim_keymaps_bytes_before = std::fs::read(env.resolve_path(srpath!(
        "home/bob/.config/zenops/configs/nvim/lua/keymaps.lua"
    )))
    .unwrap();

    // Add a file under helix only; reconcile helix.
    env.write_file(
        srpath!("home/bob/.config/helix/keymap.toml"),
        b"q = 'quit'\n",
    );
    let cmd = import_cmd(env.resolve_path(srpath!("home/bob/.config/helix")));
    env.run(&cmd).expect("helix reconcile should succeed");

    // Helix saw the new entry.
    let cfg = read_config(&env);
    let mut helix_symlinks: Vec<&str> = cfg["pkg"]["helix"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    helix_symlinks.sort();
    assert_eq!(
        helix_symlinks,
        vec!["config.toml", "keymap.toml", "themes/dark.toml"],
    );

    // Nvim's array is byte-identical to the seed.
    let nvim_symlinks: Vec<&str> = cfg["pkg"]["nvim"]["configs"][0]["symlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(nvim_symlinks, vec!["init.lua", "lua/keymaps.lua"]);

    // Nvim's symlinks still point at the same repo paths and the bytes
    // are unchanged.
    assert_eq!(
        std::fs::read_link(env.resolve_path(srpath!("home/bob/.config/nvim/init.lua"))).unwrap(),
        nvim_init_target_before,
    );
    assert_eq!(
        std::fs::read_link(env.resolve_path(srpath!("home/bob/.config/nvim/lua/keymaps.lua")))
            .unwrap(),
        nvim_keymaps_target_before,
    );
    assert_eq!(
        std::fs::read(env.resolve_path(srpath!("home/bob/.config/zenops/configs/nvim/init.lua")))
            .unwrap(),
        nvim_init_bytes_before,
    );
    assert_eq!(
        std::fs::read(env.resolve_path(srpath!(
            "home/bob/.config/zenops/configs/nvim/lua/keymaps.lua"
        )))
        .unwrap(),
        nvim_keymaps_bytes_before,
    );

    // Nvim's repo dir contains exactly its two seed files (no helix bleed).
    let nvim_repo = env.resolve_path(srpath!("home/bob/.config/zenops/configs/nvim"));
    let mut top: Vec<String> = std::fs::read_dir(&nvim_repo)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    top.sort();
    assert_eq!(top, vec!["init.lua".to_string(), "lua".to_string()]);
    let mut lua_dir: Vec<String> = std::fs::read_dir(nvim_repo.join("lua"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    lua_dir.sort();
    assert_eq!(lua_dir, vec!["keymaps.lua".to_string()]);
}
