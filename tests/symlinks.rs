use similar_asserts::assert_eq;
use zenops::{
    Cmd,
    config_files::ConfigFilePath,
    error::Error,
    output::{AppliedAction, Status, SymlinkStatus},
};
use zenops_safe_relative_path::srpath;

use test_env::{Entry, Output, paths};

mod test_env;

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
        [pkg.dummy-util]
        enable = "on"
        [pkg.dummy-util.install_hint.brew]
        packages = []
        [[pkg.dummy-util.configs]]
        type = ".config"
        source = "configs/dummy-util"
        symlinks = [
          "dummy-util.toml"
        ]

        [pkg.dummy2]
        enable = "on"
        [pkg.dummy2.install_hint.brew]
        packages = []
        [[pkg.dummy2.configs]]
        type = "home"
        dir = ".dummy2"
        source = "configs/dummy2"
        symlinks = [
          "dummy2.toml"
        ]
    "#,
    );

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
                Entry::Status(Status::Symlink {
                    real: dummy_real.clone(),
                    symlink: dummy_symlink.clone(),
                    status: SymlinkStatus::DstDirIsMissing {
                        dir: dummy_symlink.parent().unwrap(),
                    },
                }),
                Entry::Status(Status::Symlink {
                    real: dummy2_real.clone(),
                    symlink: dummy2_symlink.clone(),
                    status: SymlinkStatus::DstDirIsMissing {
                        dir: dummy2_symlink.parent().unwrap(),
                    },
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
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
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
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
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
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
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
            dry_run: false,
            allow_dirty: true,
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
            dry_run: false,
            allow_dirty: true,
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
fn pkg_configs_default_dir_name_to_pkg_key() {
    // `.config` entries under a pkg default their directory to the pkg key,
    // so `[pkg.helix]` lands at `~/.config/helix/` without an explicit `name`.
    let env = test_env::TestEnv::load();
    let real = env.cfpath("configs/helix/config.toml", ConfigFilePath::Zenops);
    let symlink = env.cfpath("helix/config.toml", ConfigFilePath::DotConfig);
    let dir = symlink.parent().unwrap();

    env.init_config(
        r#"
        [pkg.helix]
        enable = "on"
        [pkg.helix.install_hint.brew]
        packages = ["helix"]
        [[pkg.helix.configs]]
        type = ".config"
        source = "configs/helix"
        symlinks = ["config.toml"]
    "#,
    );

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
                Entry::Status(Status::Symlink {
                    real,
                    symlink,
                    status: SymlinkStatus::DstDirIsMissing { dir },
                }),
            ],
        })
    );
}

#[test]
fn pkg_configs_explicit_name_overrides_pkg_key() {
    // Useful when the pkg key and the on-disk config dir disagree (e.g. pkg
    // keyed `neovim` whose config dir is `nvim`).
    let env = test_env::TestEnv::load();
    let real = env.cfpath("configs/nvim/init.lua", ConfigFilePath::Zenops);
    let symlink = env.cfpath("nvim/init.lua", ConfigFilePath::DotConfig);
    let dir = symlink.parent().unwrap();

    env.init_config(
        r#"
        [pkg.neovim]
        enable = "on"
        [pkg.neovim.install_hint.brew]
        packages = ["neovim"]
        [[pkg.neovim.configs]]
        type = ".config"
        name = "nvim"
        source = "configs/nvim"
        symlinks = ["init.lua"]
    "#,
    );

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
                Entry::Status(Status::Symlink {
                    real,
                    symlink,
                    status: SymlinkStatus::DstDirIsMissing { dir },
                }),
            ],
        })
    );
}

#[test]
fn disabled_pkg_skips_its_configs() {
    // Configs gate on `is_installed`, so a disabled pkg contributes nothing.
    let env = test_env::TestEnv::load();

    env.init_config(
        r#"
        [pkg.ghost]
        enable = "disabled"
        [pkg.ghost.install_hint.brew]
        packages = []
        [[pkg.ghost.configs]]
        type = ".config"
        source = "configs/ghost"
        symlinks = ["ghost.toml"]
    "#,
    );

    assert_eq!(
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![env.git_repo_clean_entry()],
        })
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
        [pkg.dummy-util]
        enable = "on"
        [pkg.dummy-util.install_hint.brew]
        packages = []
        [[pkg.dummy-util.configs]]
        type = ".config"
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
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
                Entry::Status(Status::Symlink {
                    real: real.clone(),
                    symlink: symlink.clone(),
                    status: SymlinkStatus::IsFile,
                }),
            ]
        })
    );

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
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
        env.run(&Cmd::Status {
            diff: false,
            all: false
        }),
        Ok(Output {
            entries: vec![
                env.git_repo_clean_entry(),
                Entry::Status(Status::Symlink {
                    real: real.clone(),
                    symlink: symlink.clone(),
                    status: SymlinkStatus::IsDir,
                }),
            ]
        })
    );

    assert_eq!(
        env.run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
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
            allow_dirty: true,
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

#[test]
fn apply_refuses_to_overwrite_fifo_with_symlink() {
    // A FIFO (named pipe) sits at the managed symlink path. zenops must
    // not clobber non-file/non-directory entries; status reports IsOther
    // and apply errors with RefusingToOverwriteOtherWithSymlink.
    let env = test_env::TestEnv::load();
    let real = env.cfpath("configs/dummy/dummy.toml", ConfigFilePath::Zenops);
    let symlink = env.cfpath(".dummy/dummy.toml", ConfigFilePath::Home);

    env.init_config(
        r#"
        [pkg.dummy]
        enable = "on"
        [pkg.dummy.install_hint.brew]
        packages = []
        [[pkg.dummy.configs]]
        type = "home"
        dir = ".dummy"
        source = "configs/dummy"
        symlinks = ["dummy.toml"]
    "#,
    );
    env.write_zenops_file(
        srpath!("configs/dummy/dummy.toml"),
        "# real\n",
        Some("Add dummy.toml"),
    );

    // Create a FIFO at the managed path.
    env.ensure_dir_exists_for_file(paths::HOME_DIR.safe_join(srpath!(".dummy/dummy.toml")));
    let status = std::process::Command::new("mkfifo")
        .arg(symlink.full.as_ref())
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo failed");

    let status_out = env
        .run(&Cmd::Status {
            diff: false,
            all: false,
        })
        .expect("status should succeed");
    assert!(
        status_out.entries.iter().any(|e| matches!(
            e,
            Entry::Status(Status::Symlink {
                status: SymlinkStatus::IsOther,
                ..
            })
        )),
        "expected IsOther status, got {:?}",
        status_out.entries,
    );

    let result = env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
        allow_dirty: true,
    });
    assert_eq!(
        result,
        Err(Error::RefusingToOverwriteOtherWithSymlink(symlink)),
    );
    let _ = real;
}

#[test]
fn apply_errors_when_real_path_missing() {
    // The user's managed symlink correctly points at the zenops repo
    // location, but the file at that location was never committed to the
    // repo (or was removed). zenops can't synthesise the file content; apply
    // must surface this as a clean error rather than panicking.
    let env = test_env::TestEnv::load();
    let real = env.cfpath("configs/dummy/dummy.toml", ConfigFilePath::Zenops);
    let symlink = env.cfpath(".dummy/dummy.toml", ConfigFilePath::Home);

    env.init_config(
        r#"
        [pkg.dummy]
        enable = "on"
        [pkg.dummy.install_hint.brew]
        packages = []
        [[pkg.dummy.configs]]
        type = "home"
        dir = ".dummy"
        source = "configs/dummy"
        symlinks = ["dummy.toml"]
    "#,
    );

    // Pre-create the symlink pointing at the repo path, but never write the
    // real target into the repo.
    env.create_symlink(
        paths::ZENOPS_DIR.safe_join(srpath!("configs/dummy/dummy.toml")),
        paths::HOME_DIR.safe_join(srpath!(".dummy/dummy.toml")),
    );

    let result = env.run(&Cmd::Apply {
        pull_config: false,
        yes: true,
        dry_run: false,
        allow_dirty: true,
    });

    assert_eq!(result, Err(Error::SymlinkRealPathMissing { real, symlink }),);
}

#[test]
fn apply_replaces_wrong_symlink() {
    // A symlink at the managed path exists but points somewhere else than
    // the zenops repo target. Apply must replace it (with the user's --yes
    // confirmation) and emit ReplacedSymlink.
    let env = test_env::TestEnv::load();
    let real = env.cfpath("configs/dummy/dummy.toml", ConfigFilePath::Zenops);
    let symlink = env.cfpath(".dummy/dummy.toml", ConfigFilePath::Home);

    env.init_config(
        r#"
        [pkg.dummy]
        enable = "on"
        [pkg.dummy.install_hint.brew]
        packages = []
        [[pkg.dummy.configs]]
        type = "home"
        dir = ".dummy"
        source = "configs/dummy"
        symlinks = ["dummy.toml"]
    "#,
    );

    env.write_zenops_file(
        srpath!("configs/dummy/dummy.toml"),
        "# real\n",
        Some("Add dummy.toml"),
    );

    // Pre-create a symlink at the managed path pointing somewhere wrong.
    env.create_dangling_symlink(
        std::path::Path::new("/tmp/zenops-wrong-target-does-not-exist"),
        paths::HOME_DIR.safe_join(srpath!(".dummy/dummy.toml")),
    );

    // Status: drift surfaces as WrongLink.
    let status_out = env
        .run(&Cmd::Status {
            diff: false,
            all: false,
        })
        .expect("status should succeed");
    assert!(
        status_out.entries.iter().any(|e| matches!(
            e,
            Entry::Status(Status::Symlink {
                status: SymlinkStatus::WrongLink(_),
                ..
            })
        )),
        "expected WrongLink status, got {:?}",
        status_out.entries,
    );

    // Apply: confirms via --yes and emits ReplacedSymlink.
    let apply_out = env
        .run(&Cmd::Apply {
            pull_config: false,
            yes: true,
            dry_run: false,
            allow_dirty: true,
        })
        .expect("apply should succeed");
    assert_eq!(
        apply_out,
        Output {
            entries: vec![Entry::AppliedAction(AppliedAction::ReplacedSymlink {
                real: real.clone(),
                symlink: symlink.clone(),
            })],
        },
    );

    // The symlink now points at the real target.
    let resolved = std::fs::read_link(&symlink.full).expect("read_link");
    assert_eq!(resolved, real.full.as_ref());
}
