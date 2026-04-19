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
