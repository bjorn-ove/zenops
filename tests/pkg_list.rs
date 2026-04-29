use similar_asserts::assert_eq;
use zenops::{
    output::{PkgEntry, PkgEntryState},
    pkg_list,
};

mod test_env;

/// Helper for the pkg_list_* tests: extract the (name, state) pair from a
/// `PkgEntry::Pkg` variant; ignore everything else.
fn pkg_row(entry: &PkgEntry) -> Option<(&str, PkgEntryState)> {
    if let PkgEntry::Pkg { name, state, .. } = entry {
        Some((name.as_str(), *state))
    } else {
        None
    }
}

#[test]
fn pkg_list_shows_defaults_as_missing() {
    let env = test_env::TestEnv::load();
    env.init_config("");

    // None of the default pkgs' detect targets exist in a temp home.
    let entries = env
        .run_pkg_list(pkg_list::Options::default())
        .expect("pkg list should succeed");

    let names: Vec<&str> = entries.iter().filter_map(pkg_row).map(|(n, _)| n).collect();
    assert!(
        names.contains(&"cargo"),
        "expected cargo in entries: {names:?}"
    );
    assert!(names.contains(&"sk"), "expected sk in entries: {names:?}");
    assert!(
        names.contains(&"starship"),
        "expected starship in entries: {names:?}",
    );
    let starship_desc = entries.iter().find_map(|e| match e {
        PkgEntry::Pkg {
            name, description, ..
        } if name == "starship" => description.as_deref(),
        _ => None,
    });
    assert_eq!(
        starship_desc.unwrap_or_default(),
        "starship — cross-shell prompt.",
        "starship entry should carry its description verbatim",
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
        [pkg.alpha.detect]
        type = "file"
        path = "~/.alpha-marker"
        [pkg.alpha.install_hint.brew]
        packages = ["alpha-formula"]

        [pkg.bravo]
        enable = "detect"
        description = "Bravo test pkg."
        [pkg.bravo.detect]
        type = "file"
        path = "~/.bravo-marker"
        [pkg.bravo.install_hint.brew]
        packages = ["bravo-formula"]
    "#,
    );

    let entries = env
        .run_pkg_list(pkg_list::Options::default())
        .expect("pkg list should succeed");

    let brew_available = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| std::path::Path::new(dir).join("brew").is_file());

    let aggregate = entries.iter().find_map(|e| match e {
        PkgEntry::AggregateInstall {
            command, packages, ..
        } => Some((command.as_str(), packages.as_slice())),
        _ => None,
    });

    if brew_available {
        let (command, packages) =
            aggregate.unwrap_or_else(|| panic!("expected aggregate install entry: {entries:?}"));
        assert!(
            packages.iter().any(|p| p == "alpha-formula")
                && packages.iter().any(|p| p == "bravo-formula"),
            "aggregate should list both missing pkgs, got: {packages:?}",
        );
        assert!(
            command.starts_with("brew install"),
            "aggregate command should be a brew install line, got: {command:?}",
        );
    } else {
        assert!(
            aggregate.is_none(),
            "expected no aggregate install without brew on PATH, got: {entries:?}",
        );
        // Without a manager detected, the warning fires.
        assert!(
            entries
                .iter()
                .any(|e| matches!(e, PkgEntry::NoPackageManagerDetected { .. })),
            "expected NoPackageManagerDetected event, got: {entries:?}",
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

    let default_entries = env
        .run_pkg_list(pkg_list::Options::default())
        .expect("pkg list should succeed");
    assert!(
        !default_entries
            .iter()
            .any(|e| matches!(pkg_row(e), Some(("ghost", _)))),
        "disabled pkg should be hidden by default, got: {default_entries:?}",
    );

    let all_entries = env
        .run_pkg_list(pkg_list::Options {
            all: true,
            ..Default::default()
        })
        .expect("pkg list --all should succeed");
    let ghost_state = all_entries
        .iter()
        .find_map(|e| match pkg_row(e) {
            Some(("ghost", state)) => Some(state),
            _ => None,
        })
        .unwrap_or_else(|| panic!("disabled pkg should appear with --all, got: {all_entries:?}"));
    assert_eq!(
        ghost_state,
        PkgEntryState::Disabled,
        "disabled pkg should carry PkgEntryState::Disabled",
    );
}

#[test]
fn pkg_list_pattern_filters_by_substring() {
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.alpha]
        enable = "detect"
        [pkg.alpha.detect]
        type = "file"
        path = "~/.alpha-marker"
        [pkg.alpha.install_hint.brew]
        packages = []

        [pkg.bravo]
        enable = "detect"
        [pkg.bravo.detect]
        type = "file"
        path = "~/.bravo-marker"
        [pkg.bravo.install_hint.brew]
        packages = []
    "#,
    );

    let pkg_names = |entries: &[PkgEntry]| -> Vec<String> {
        entries
            .iter()
            .filter_map(pkg_row)
            .map(|(n, _)| n.to_string())
            .collect()
    };
    let only_test_pkgs = |names: Vec<String>| -> Vec<String> {
        names
            .into_iter()
            .filter(|n| n == "alpha" || n == "bravo")
            .collect()
    };

    // Single pattern → only matching pkg appears among the test pkgs.
    let entries = env
        .run_pkg_list(pkg_list::Options {
            pattern: vec!["alpha".into()],
            ..Default::default()
        })
        .expect("filter should succeed");
    assert_eq!(
        only_test_pkgs(pkg_names(&entries)),
        vec!["alpha".to_string()],
        "filter excludes bravo, got: {entries:?}",
    );

    // Case-insensitive.
    let entries = env
        .run_pkg_list(pkg_list::Options {
            pattern: vec!["ALPHA".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(
        entries
            .iter()
            .any(|e| matches!(pkg_row(e), Some(("alpha", _)))),
        "uppercase pattern should match alpha, got: {entries:?}",
    );

    // Multi-pattern OR.
    let entries = env
        .run_pkg_list(pkg_list::Options {
            pattern: vec!["alpha".into(), "bravo".into()],
            ..Default::default()
        })
        .unwrap();
    let mut both = only_test_pkgs(pkg_names(&entries));
    both.sort();
    assert_eq!(both, vec!["alpha".to_string(), "bravo".to_string()]);

    // Non-matching → no pkg rows at all (other event types may still fire).
    let entries = env
        .run_pkg_list(pkg_list::Options {
            pattern: vec!["zzz-nothing".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(
        entries.iter().all(|e| !matches!(e, PkgEntry::Pkg { .. })),
        "no rows should match 'zzz-nothing', got: {entries:?}",
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

    let entries = env
        .run_pkg_list(pkg_list::Options {
            all: true,
            ..Default::default()
        })
        .expect("pkg list --all should succeed");
    assert!(
        !entries
            .iter()
            .any(|e| matches!(pkg_row(e), Some(("alien", _)))),
        "pkg gated to the other OS must not appear in the list, got: {entries:?}",
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

    let entries = env
        .run_pkg_list(pkg_list::Options {
            all: true,
            ..Default::default()
        })
        .expect("pkg list --all should succeed");
    assert!(
        !entries
            .iter()
            .any(|e| matches!(pkg_row(e), Some(("bash-only", _)))),
        "pkg gated to other shell must not appear in list, got: {entries:?}",
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

    let entries = env
        .run_pkg_list(pkg_list::Options::default())
        .expect("pkg list should succeed");
    assert!(
        !entries
            .iter()
            .any(|e| matches!(pkg_row(e), Some(("dual-actions", _)))),
        "shell gate must hide pkg even when bash actions exist, got: {entries:?}",
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

    let entries = env
        .run_pkg_list(pkg_list::Options::default())
        .expect("pkg list should succeed");
    let entry = entries
        .iter()
        .find_map(|e| match e {
            PkgEntry::Pkg { name, key, .. } if key == "verbose-key-name" => {
                Some(name.as_str().to_string())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected pkg entry for verbose-key-name, got: {entries:?}"));
    assert_eq!(entry, "short", "display name override should win over key");
}
