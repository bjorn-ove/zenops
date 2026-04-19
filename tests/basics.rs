use safe_relative_path::srpath;
use similar_asserts::assert_eq;
use zenops::{
    Cmd,
    config_files::ConfigFilePath,
    error::Error,
    git::GitFileStatus,
    output::{AppliedAction, Status, SymlinkStatus},
};

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

    assert_eq!(env.run(&Cmd::Status { diff: false }), Ok(Output { entries: vec![] }),);

    // Check it works with a .git repository
    env.init_config("");

    assert_eq!(env.run(&Cmd::Status { diff: false }), Ok(Output { entries: vec![] }),);
}

#[test]
fn config_dir_git_status() {
    let env = test_env::TestEnv::load();

    env.init_config("");

    assert_eq!(env.run(&Cmd::Status { diff: false }), Ok(Output { entries: vec![] }));

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
        env.run(&Cmd::Apply { pull_config: false }),
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
        env.run(&Cmd::Apply { pull_config: false }),
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
