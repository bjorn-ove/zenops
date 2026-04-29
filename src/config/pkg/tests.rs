use indexmap::IndexMap;
use smol_str::SmolStr;

use super::*;

fn inputs(pairs: &[(&str, &str)]) -> IndexMap<SmolStr, SmolStr> {
    pairs
        .iter()
        .map(|(k, v)| (SmolStr::new(k), SmolStr::new(v)))
        .collect()
}

fn system_empty() -> IndexMap<SmolStr, SmolStr> {
    IndexMap::new()
}

#[test]
fn install_hint_round_trips_from_toml() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            description = "fuzzy finder"
            [install_hint.brew]
            packages = ["sk"]
            "#,
    )
    .unwrap();
    assert_eq!(pkg.description.as_deref(), Some("fuzzy finder"));
    assert_eq!(pkg.install_hint.brew.packages, vec!["sk".to_string()]);
}

#[test]
fn install_hint_is_required_in_toml() {
    let err = toml::from_str::<PkgConfig>(r#"description = "missing install_hint""#).unwrap_err();
    assert!(
        err.to_string().contains("install_hint"),
        "expected error to mention install_hint, got: {err}"
    );
}

#[test]
fn detect_strategy_with_unresolved_input_reports_not_installed() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "${brew_prefix}/opt/x"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(!pkg.is_installed(tmp.path(), &system_empty()));
    assert!(pkg.matched_detect(tmp.path(), &system_empty()).is_none());
}

#[test]
fn detect_strategy_resolves_system_input_and_checks_file() {
    let tmp = tempfile::tempdir().unwrap();
    let opt = tmp.path().join("opt/x");
    std::fs::create_dir_all(&opt).unwrap();
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "${root}/opt/x"
            "#,
    )
    .unwrap();
    let sys = inputs(&[("root", tmp.path().to_str().unwrap())]);
    assert!(pkg.is_installed(tmp.path(), &sys));
}

#[test]
fn pkg_inputs_shadow_system_inputs_at_detect() {
    let tmp = tempfile::tempdir().unwrap();
    let marker_a = tmp.path().join("a");
    std::fs::write(&marker_a, "").unwrap();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [inputs]
            name = "a"
            [detect]
            type = "file"
            path = "{}/${{name}}"
            "#,
        tmp.path().display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let sys = inputs(&[("name", "b")]);
    assert!(pkg.is_installed(tmp.path(), &sys));
}

#[test]
fn enable_on_with_matching_detect_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let toml_src = format!(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
}

#[test]
fn enable_on_with_missing_detect_flags_health_signal() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "/definitely/does/not/exist/zenops-test"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
}

#[test]
fn enable_on_with_empty_detect_is_silent() {
    // No detect strategies → nothing to check → no health signal even
    // under `enable = "on"`. This is the "always-on meta-pkg" shape
    // (bashrc-chain, local-bin, zenops).
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
}

#[test]
fn enable_detect_miss_is_silent() {
    // Silence on miss is the point of `detect`; don't surface a signal.
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "/definitely/does/not/exist/zenops-test"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
}

#[test]
fn enable_on_with_matching_detect_is_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let toml_src = format!(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn enable_on_with_missing_detect_is_not_installed() {
    // `on` + detect miss treats the pkg as not installed, same as
    // `detect` + miss. Rendering code is responsible for any visual
    // distinction; this predicate only reports installation state.
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "/definitely/does/not/exist/zenops-test"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(!pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn enable_on_with_empty_detect_is_installed() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn empty_detect_with_enable_detect_means_installed() {
    // A pkg with `enable = "detect"` but no detect strategies has nothing
    // to check, so it's treated as installed. This lets config-only pkgs
    // stay ergonomic without setting `enable = "on"`.
    let pkg: PkgConfig = toml::from_str(
        r#"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
    // No detect ran, so `matched_detect` still has nothing to return.
    assert!(pkg.matched_detect(tmp.path(), &system_empty()).is_none());
}

#[test]
fn disabled_pkg_is_never_installed() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "disabled"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(!pkg.is_installed(tmp.path(), &system_empty()));
    assert!(pkg.is_disabled());
}

#[test]
fn supported_os_gates_installation() {
    // Pick the OS that is not current so the pkg must be filtered out.
    let other = match Os::current().expect("tests run on supported OS") {
        Os::Linux => "macos",
        Os::Macos => "linux",
    };
    let toml_src = format!(
        r#"
            enable = "on"
            supported_os = ["{other}"]
            [install_hint.brew]
            packages = []
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(!pkg.is_installed(tmp.path(), &system_empty()));
    assert!(pkg.matched_detect(tmp.path(), &system_empty()).is_none());
}

#[test]
fn supported_os_allows_installation_when_current_os_listed() {
    let current = match Os::current().expect("tests run on supported OS") {
        Os::Linux => "linux",
        Os::Macos => "macos",
    };
    let toml_src = format!(
        r#"
            enable = "on"
            supported_os = ["{current}"]
            [install_hint.brew]
            packages = []
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn empty_supported_os_means_any_os() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn supported_shells_round_trips_from_toml() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            supported_shells = ["bash", "zsh"]
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    assert_eq!(pkg.supported_shells, vec![Shell::Bash, Shell::Zsh]);
}

#[test]
fn supports_shell_empty_list_means_any() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    assert!(pkg.supports_shell(Some(Shell::Bash)));
    assert!(pkg.supports_shell(Some(Shell::Zsh)));
    assert!(pkg.supports_shell(None));
}

#[test]
fn supports_shell_filters_by_list() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            supported_shells = ["bash"]
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    assert!(pkg.supports_shell(Some(Shell::Bash)));
    assert!(!pkg.supports_shell(Some(Shell::Zsh)));
    assert!(!pkg.supports_shell(None));
}

#[test]
fn name_field_round_trips_from_toml() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            name = "brew"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    assert_eq!(pkg.name.as_deref(), Some("brew"));
}

#[test]
fn name_field_defaults_to_none() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    assert!(pkg.name.is_none());
}

#[test]
fn any_combinator_matches_when_any_child_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let present = tmp.path().join(".present");
    std::fs::write(&present, "").unwrap();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "any"
            of = [
              {{ type = "file", path = "/definitely/does/not/exist/zenops-test" }},
              {{ type = "file", path = "{}" }},
            ]
            "#,
        present.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn all_combinator_requires_every_child() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::write(&a, "").unwrap();
    // Only `a` exists — `all` should miss.
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "all"
            of = [
              {{ type = "file", path = "{}" }},
              {{ type = "file", path = "{}" }},
            ]
            "#,
        a.display(),
        b.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(!pkg.is_installed(tmp.path(), &system_empty()));

    std::fs::write(&b, "").unwrap();
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn nested_combinators_compose() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::write(&a, "").unwrap();
    std::fs::write(&b, "").unwrap();
    // any(all(a, b), which=<missing>) — inner `all` hits, outer `any` hits.
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "any"
            of = [
              {{ type = "all", of = [
                {{ type = "file", path = "{}" }},
                {{ type = "file", path = "{}" }},
              ] }},
              {{ type = "which", binary = "definitely-not-on-path-zenops-test" }},
            ]
            "#,
        a.display(),
        b.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn per_strategy_os_skips_when_os_mismatches() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let other = match Os::current().expect("tests run on supported OS") {
        Os::Linux => "macos",
        Os::Macos => "linux",
    };
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            os = ["{other}"]
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    // The file exists, but the strategy is gated to the other OS — skip.
    assert!(!pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn per_strategy_os_allows_when_os_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let current = match Os::current().expect("tests run on supported OS") {
        Os::Linux => "linux",
        Os::Macos => "macos",
    };
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            os = ["{current}"]
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn empty_os_list_means_any_os() {
    // An `os = []` (or field omitted) strategy is applicable on every OS.
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            os = []
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn absent_detect_field_is_installed_and_silent() {
    // No `detect` field at all → nothing to check → installed, no health
    // signal even under `enable = "on"`. This is the "always-on meta-pkg"
    // shape (bashrc-chain, local-bin, zenops).
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
    assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
}

#[test]
fn all_with_empty_of_matches_vacuously() {
    // An empty `all` is vacuously true. Documented so we don't quietly
    // change this later; in practice, users should omit `detect` entirely
    // if they mean "no check required".
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "all"
            of = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    assert!(pkg.is_installed(tmp.path(), &system_empty()));
}

#[test]
fn path_action_kinds_round_trip_from_toml() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [[shell.env_init.bash]]
            type = "path_prepend"
            value = "/opt/foo/bin"
            [[shell.env_init.bash]]
            type = "path_append"
            value = "/opt/bar/bin"
            "#,
    )
    .unwrap();
    let actions = &pkg.shell.env_init.for_shell(Shell::Bash);
    assert_eq!(actions.len(), 2);
    assert!(matches!(actions[0].kind, ActionKind::PathPrepend { .. }));
    assert!(matches!(actions[1].kind, ActionKind::PathAppend { .. }));
}
