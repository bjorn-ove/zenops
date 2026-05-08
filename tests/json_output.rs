//! End-to-end `--output json` tests: spawn the real binary and verify the
//! NDJSON event stream on stdout per command. The `event` discriminator is
//! the public contract scripts depend on, so each test focuses on
//! "every line parses as JSON" + "the expected `event` types appear in the
//! expected order" — fine-grained shape testing lives in `output::tests`.

use std::{path::Path, process::Command};

mod test_env;

const MINIMAL_CONFIG: &str = r#"
[shell]
type = "bash"
[shell.environment]
[shell.alias]
"#;

/// Run `zenops -o json <args>` with `HOME=<temp home>` and return only the
/// stdout lines that parse as JSON. Non-JSON lines (the inherited git output
/// from `print_pre_apply_summary`, etc.) are intentionally tolerated: `-o
/// json` promises a structured stream of *zenops* events, not that no child
/// process ever writes to stdout.
fn run_json(home: &Path, args: &[&str]) -> Vec<serde_json::Value> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zenops"));
    cmd.env("HOME", home).env_remove("NO_COLOR");
    cmd.arg("-o").arg("json");
    for a in args {
        cmd.arg(a);
    }
    let output = cmd.output().expect("spawn zenops");
    assert!(
        output.status.success(),
        "zenops {args:?} exited {}; stderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("stdout is utf-8")
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[test]
fn doctor_emits_doctor_check_ndjson_to_stdout() {
    // Doctor runs even without a config repo, so this needs no setup
    // beyond pointing HOME at an empty dir.
    let env = test_env::TestEnv::load();
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let lines = run_json(&home, &["doctor"]);
    assert!(!lines.is_empty(), "doctor should emit at least one event");
    for line in &lines {
        assert_eq!(
            line["event"], "doctor_check",
            "every doctor line should be doctor_check, got: {line}",
        );
        // Section headers are skipped; every emitted line is a Check.
        assert_eq!(line["kind"], "check", "got: {line}");
    }
    // The System section always emits at least an `os:` info row.
    let has_os = lines
        .iter()
        .any(|l| l["section"] == "system" && l["label"] == "os:" && l["severity"] == "info");
    assert!(has_os, "expected a system/os: info row, got: {lines:?}");
}

#[test]
fn pkg_emits_pkg_entry_ndjson_to_stdout() {
    let env = test_env::TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let lines = run_json(&home, &["pkg"]);
    let mut saw_pkg_row = false;
    for line in &lines {
        assert_eq!(line["event"], "pkg_entry", "got: {line}");
        if line["kind"] == "pkg" {
            saw_pkg_row = true;
            assert!(line["state"].is_string(), "got: {line}");
        }
    }
    assert!(
        saw_pkg_row,
        "expected at least one pkg_entry/pkg row, got: {lines:?}",
    );
}

#[test]
fn status_emits_status_ndjson_to_stdout() {
    let env = test_env::TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let lines = run_json(&home, &["status"]);
    assert!(!lines.is_empty(), "status should emit at least one event");
    for line in &lines {
        assert_eq!(
            line["event"], "status",
            "every status line should be a status event, got: {line}",
        );
    }
}

#[test]
fn init_emits_init_summary_ndjson_to_stdout() {
    let env = test_env::TestEnv::load();
    let bare = env.seed_bare_repo(&[("config.toml", MINIMAL_CONFIG)]);
    // `zenops init` requires the destination dir to be missing or empty.
    // TestEnv pre-creates an empty zenops dir; the preflight removes it.
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let lines = run_json(&home, &["init", bare.to_str().unwrap()]);
    let summaries: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|l| l["event"] == "init_summary")
        .collect();
    assert_eq!(
        summaries.len(),
        1,
        "expected exactly one init_summary, got: {lines:?}",
    );
    let s = summaries[0];
    assert!(s["clone_path"].is_string(), "got: {s}");
    // The cloned config layers on top of zenops's compiled-in defaults
    // (see `DEFAULT_PKGS` in src/config.rs), so `pkg_count` is never zero.
    assert!(s["pkg_count"].as_u64().is_some_and(|n| n > 0), "got: {s}");
}

#[test]
fn apply_emits_applied_action_ndjson_to_stdout() {
    let env = test_env::TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let lines = run_json(&home, &["apply", "--yes"]);
    let mut saw_action = false;
    for line in &lines {
        let event = line["event"].as_str().unwrap_or("");
        assert!(
            event == "status" || event == "applied_action",
            "apply should only emit status/applied_action events, got: {line}",
        );
        if event == "applied_action" {
            saw_action = true;
        }
    }
    assert!(
        saw_action,
        "minimal-config apply should emit at least one applied_action, got: {lines:?}",
    );
}

#[test]
fn import_emits_import_plan_and_applied_ndjson_to_stdout() {
    // Happy-path import: the JSON stream must surface both the
    // ImportPlan event (so dry-run-style consumers can preview) and the
    // ImportApplied event (so they can confirm the apply happened).
    let env = test_env::TestEnv::load();
    env.init_config(MINIMAL_CONFIG);
    env.write_file(
        zenops_safe_relative_path::srpath!("home/bob/.config/myapp/config.toml"),
        b"foo = 1\n",
    );
    let home = env.resolve_path(test_env::paths::HOME_DIR);
    let target = env
        .resolve_path(zenops_safe_relative_path::srpath!("home/bob/.config/myapp"))
        .to_string_lossy()
        .into_owned();

    let lines = run_json(&home, &["import", &target, "--brew", "myapp", "--yes"]);

    let plans: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|l| l["event"] == "import_plan")
        .collect();
    let applieds: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|l| l["event"] == "import_applied")
        .collect();

    assert_eq!(
        plans.len(),
        1,
        "expected exactly one import_plan, got: {lines:?}"
    );
    assert_eq!(
        applieds.len(),
        1,
        "expected exactly one import_applied, got: {lines:?}",
    );

    let plan = plans[0];
    assert_eq!(plan["pkg"], "myapp", "plan: {plan}");
    let applied = applieds[0];
    assert_eq!(applied["pkg"], "myapp", "applied: {applied}");
    // is_noop is `skip_serializing_if = !x`, so a non-noop apply omits the
    // field entirely; treat both `false` and missing as the same signal.
    assert!(
        applied["is_noop"].is_null() || applied["is_noop"] == false,
        "applied: {applied}",
    );
}
