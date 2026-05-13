use std::path::Path;

use indexmap::IndexMap;
use smol_str::SmolStr;

use super::*;
use crate::config::condition::{ConditionOrRef, Conditions, HostContext};

fn inputs(pairs: &[(&str, &str)]) -> IndexMap<SmolStr, SmolStr> {
    pairs
        .iter()
        .map(|(k, v)| (SmolStr::new(k), SmolStr::new(v)))
        .collect()
}

fn system_empty() -> IndexMap<SmolStr, SmolStr> {
    IndexMap::new()
}

/// The standard built-in registry: `linux`, `macos`, `bash`, `zsh`. Mirrors
/// what `Config::load` deep-merges so unit tests behave like real loads.
fn builtin_conditions() -> Conditions {
    let toml_src = r#"
        [conditions]
        linux = { os = "linux" }
        macos = { os = "macos" }
        bash  = { shell = "bash" }
        zsh   = { shell = "zsh" }
    "#;
    #[derive(serde::Deserialize)]
    struct Holder {
        conditions: IndexMap<SmolStr, crate::config::condition::Condition>,
    }
    let h: Holder = toml::from_str(toml_src).unwrap();
    Conditions::compile(h.conditions).unwrap()
}

fn ctx<'a>(
    home: &'a Path,
    sys: &'a IndexMap<SmolStr, SmolStr>,
    shell: Option<Shell>,
) -> HostContext<'a> {
    HostContext {
        os: Os::current().expect("tests run on supported OS"),
        shell,
        hostname: "test-host",
        home,
        system_inputs: sys,
    }
}

fn current_os_str() -> &'static str {
    match Os::current().expect("tests run on supported OS") {
        Os::Linux => "linux",
        Os::Macos => "macos",
    }
}

fn other_os_str() -> &'static str {
    match Os::current().expect("tests run on supported OS") {
        Os::Linux => "macos",
        Os::Macos => "linux",
    }
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
            exists = "${brew_prefix}/opt/x"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());
    assert!(pkg.matched_detect(&conds, &c).unwrap().is_none());
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
            exists = "${root}/opt/x"
            "#,
    )
    .unwrap();
    let conds = builtin_conditions();
    let sys = inputs(&[("root", tmp.path().to_str().unwrap())]);
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
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
            exists = "{}/${{name}}"
            "#,
        tmp.path().display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = inputs(&[("name", "b")]);
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
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
            exists = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.enable_on_but_detect_missing(&conds, &c).unwrap());
}

#[test]
fn enable_on_with_missing_detect_flags_health_signal() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            exists = "/definitely/does/not/exist/zenops-test"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.enable_on_but_detect_missing(&conds, &c).unwrap());
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
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.enable_on_but_detect_missing(&conds, &c).unwrap());
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
            exists = "/definitely/does/not/exist/zenops-test"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.enable_on_but_detect_missing(&conds, &c).unwrap());
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
            exists = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
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
            exists = "/definitely/does/not/exist/zenops-test"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());
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
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
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
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
    // No detect ran, so `matched_detect` still has nothing to return.
    assert!(pkg.matched_detect(&conds, &c).unwrap().is_none());
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
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());
    assert!(pkg.is_disabled());
}

#[test]
fn when_named_other_os_gates_installation() {
    let other = other_os_str();
    let toml_src = format!(
        r#"
            enable = "on"
            when = "{other}"
            [install_hint.brew]
            packages = []
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());
    assert!(pkg.matched_detect(&conds, &c).unwrap().is_none());
}

#[test]
fn when_named_current_os_allows_installation() {
    let current = current_os_str();
    let toml_src = format!(
        r#"
            enable = "on"
            when = "{current}"
            [install_hint.brew]
            packages = []
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn absent_when_means_unconditional() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn when_inline_table_works_without_registry_entry() {
    let current = current_os_str();
    let toml_src = format!(
        r#"
            enable = "on"
            when = {{ os = "{current}" }}
            [install_hint.brew]
            packages = []
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = Conditions::compile(IndexMap::new()).unwrap();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn when_named_shell_gates_relevance_when_shell_set() {
    let pkg: PkgConfig = toml::from_str(
        r#"
            when = "bash"
            [install_hint.brew]
            packages = []
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c_bash = ctx(tmp.path(), &sys, Some(Shell::Bash));
    let c_zsh = ctx(tmp.path(), &sys, Some(Shell::Zsh));
    let c_none = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c_bash).unwrap());
    assert!(!pkg.is_installed(&conds, &c_zsh).unwrap());
    // Unset shell can't satisfy `shell = "bash"` — same as the old empty
    // `supported_shells` semantics for `Shell::None`.
    assert!(!pkg.is_installed(&conds, &c_none).unwrap());
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
            any = [
              {{ exists = "/definitely/does/not/exist/zenops-test" }},
              {{ exists = "{}" }},
            ]
            "#,
        present.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
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
            all = [
              {{ exists = "{}" }},
              {{ exists = "{}" }},
            ]
            "#,
        a.display(),
        b.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());

    std::fs::write(&b, "").unwrap();
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    assert!(pkg.is_installed(&conds, &c).unwrap());
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
            any = [
              {{ all = [
                {{ exists = "{}" }},
                {{ exists = "{}" }},
              ] }},
              {{ which = "definitely-not-on-path-zenops-test" }},
            ]
            "#,
        a.display(),
        b.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn detect_strategy_displays_file_leaf() {
    let s: super::detect::DetectStrategy = toml::from_str(
        r#"
            exists = "/opt/x"
        "#,
    )
    .unwrap();
    assert_eq!(s.to_string(), "/opt/x");
}

#[test]
fn detect_strategy_displays_which_leaf() {
    let s: super::detect::DetectStrategy = toml::from_str(
        r#"
            which = "rg"
        "#,
    )
    .unwrap();
    assert_eq!(s.to_string(), "which rg");
}

#[test]
fn detect_strategy_displays_any_combinator_with_nested_leaves() {
    let s: super::detect::DetectStrategy = toml::from_str(
        r#"
            any = [
                { exists = "/a" },
                { which = "b" },
            ]
        "#,
    )
    .unwrap();
    assert_eq!(s.to_string(), "any(/a, which b)");
}

#[test]
fn detect_strategy_displays_all_combinator_empty() {
    let s: super::detect::DetectStrategy = toml::from_str(
        r#"
            all = []
        "#,
    )
    .unwrap();
    assert_eq!(s.to_string(), "all()");
}

#[test]
fn detect_strategy_displays_nested_combinators() {
    let s: super::detect::DetectStrategy = toml::from_str(
        r#"
            all = [
                { exists = "/x" },
                { any = [
                    { which = "rg" },
                ] },
            ]
        "#,
    )
    .unwrap();
    assert_eq!(s.to_string(), "all(/x, any(which rg))");
}

#[test]
fn detect_when_true_evaluates_inner_strategy() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let current = current_os_str();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            when = "{current}"
            [detect.then]
            exists = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn detect_when_false_evaluates_as_false_not_skipped() {
    // Regression guard for the "skip vs false" semantic: if `when` is false,
    // the node must report false, never be filtered from a parent combinator.
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let other = other_os_str();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            when = "{other}"
            [detect.then]
            exists = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn detect_all_with_when_false_child_is_false_not_vacuously_true() {
    // The load-bearing test: an `all` whose every child is gated to another
    // host must evaluate false. Filtering when-false children would empty
    // the list and flip `all` to vacuously true — a detection bug.
    let other = other_os_str();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            all = [
              {{ when = "{other}", then = {{ exists = "/anywhere" }} }},
              {{ when = "{other}", then = {{ exists = "/anywhere" }} }},
            ]
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn detect_any_with_one_matching_when_branch_matches() {
    // OS-divergent detect inside a single pkg: only the branch gated to the
    // current host should be reached, and if its inner check passes, the
    // outer `any` matches.
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let current = current_os_str();
    let other = other_os_str();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            any = [
              {{ when = "{other}",   then = {{ exists = "/definitely/missing" }} }},
              {{ when = "{current}", then = {{ exists = "{}" }} }},
            ]
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn detect_when_with_inline_condition_table() {
    // Mirrors `when_inline_table_works_without_registry_entry` at the
    // pkg level: `when` inside detect accepts an inline condition table.
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".marker");
    std::fs::write(&marker, "").unwrap();
    let current = current_os_str();
    let toml_src = format!(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            when = {{ os = "{current}" }}
            [detect.then]
            exists = "{}"
            "#,
        marker.display()
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let conds = Conditions::compile(IndexMap::new()).unwrap();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(pkg.is_installed(&conds, &c).unwrap());
}

#[test]
fn detect_when_displays_with_named_ref() {
    let s: super::detect::DetectStrategy = toml::from_str(
        r#"
            when = "macos"
            [then]
            exists = "/opt/x"
        "#,
    )
    .unwrap();
    assert_eq!(s.to_string(), "when(macos, /opt/x)");
}

#[test]
fn detect_which_with_unresolved_input_reports_not_installed() {
    // Hits the `Err(_) => false` arm in `DetectKind::check` for `Which`:
    // an unresolved `${var}` in the binary template can't be expanded, so
    // the strategy is treated as a miss rather than a hard error.
    let pkg: PkgConfig = toml::from_str(
        r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            which = "${unresolved}"
            "#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    match pkg.is_installed(&conds, &c).unwrap_err() {
        Error::Which(crate::utils::which::Error::ExpandError(
            value,
            zenops_expand::ExpandError::Unresolved(var),
        )) => {
            assert_eq!(value.as_template(), "${unresolved}");
            assert_eq!(var, "unresolved");
        }
        _ => panic!("Expected Which(ExpandError) variant"),
    }
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

// ----------- migration / loud-failure -----------

#[test]
fn legacy_supported_os_field_fails_to_load() {
    let err = toml::from_str::<PkgConfig>(
        r#"
            supported_os = ["linux"]
            [install_hint.brew]
            packages = []
        "#,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("supported_os"),
        "expected error to name 'supported_os', got: {err}"
    );
}

#[test]
fn legacy_supported_shells_field_fails_to_load() {
    let err = toml::from_str::<PkgConfig>(
        r#"
            supported_shells = ["bash"]
            [install_hint.brew]
            packages = []
        "#,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("supported_shells"),
        "expected error to name 'supported_shells', got: {err}"
    );
}

#[test]
fn when_evaluating_to_false_silences_health_signal() {
    // A pkg gated to the other OS via `when` should not surface a missing
    // signal, mirroring the old `supported_os` behavior.
    let other = other_os_str();
    let toml_src = format!(
        r#"
            enable = "on"
            when = "{other}"
            [install_hint.brew]
            packages = []
            [detect]
            exists = "/definitely/does/not/exist/zenops-test"
            "#
    );
    let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let conds = builtin_conditions();
    let sys = system_empty();
    let c = ctx(tmp.path(), &sys, None);
    assert!(!pkg.enable_on_but_detect_missing(&conds, &c).unwrap());
}

#[test]
fn when_with_inline_condition_drops_into_unconditional_registry() {
    // Sanity: `when = { ... }` doesn't need a [conditions] entry to work.
    let pkg: PkgConfig = toml::from_str(
        r#"
            when = { not = "zsh" }
            [install_hint.brew]
            packages = []
        "#,
    )
    .unwrap();
    matches!(pkg.when, Some(ConditionOrRef::Inline(_)));
}
