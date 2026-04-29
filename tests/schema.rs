//! End-to-end test for the `zenops schema` subcommand. The schema bundle
//! is part of the crate's SemVer-versioned public API; a regression in
//! `schema_for!(Event)` or `schema_for!(StoredConfig)` would otherwise
//! ship silently.

use std::process::Command;

mod test_env;

fn run_schema(home: &std::path::Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_zenops"))
        .env("HOME", home)
        .arg("schema")
        .output()
        .expect("spawn zenops");
    assert!(
        output.status.success(),
        "zenops schema exited {}; stderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    serde_json::from_str(&stdout).expect("stdout parses as a single JSON document")
}

#[test]
fn schema_emits_versioned_bundle_with_event_and_config_schemas() {
    let env = test_env::TestEnv::load();
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let bundle = run_schema(&home);

    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        bundle["zenops_version"], version,
        "zenops_version must match crate version, got: {bundle}",
    );
    assert_eq!(
        bundle["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "got: {bundle}",
    );
    let id = bundle["$id"].as_str().expect("$id is a string");
    assert!(
        id.contains(version),
        "$id must embed the crate version ({version}), got {id}",
    );
    assert_eq!(bundle["title"], "zenops schema bundle", "got: {bundle}");

    let schemas = &bundle["schemas"];
    assert!(schemas["output_event"].is_object(), "got: {bundle}");
    assert!(schemas["config"].is_object(), "got: {bundle}");
}

#[test]
fn schema_runs_without_a_config_repo() {
    // The whole point of `Cmd::Schema` is that it runs before `Config::load`,
    // so a fresh HOME with no zenops repo must still succeed.
    let env = test_env::TestEnv::load();
    let home = env.resolve_path(test_env::paths::HOME_DIR);
    std::fs::remove_dir_all(env.resolve_path(test_env::paths::ZENOPS_DIR)).ok();

    let _bundle = run_schema(&home);
}

#[test]
fn schema_output_ends_with_single_trailing_newline() {
    let env = test_env::TestEnv::load();
    let home = env.resolve_path(test_env::paths::HOME_DIR);

    let output = Command::new(env!("CARGO_BIN_EXE_zenops"))
        .env("HOME", home)
        .arg("schema")
        .output()
        .expect("spawn zenops");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.ends_with('\n'), "expected trailing newline");
    assert!(
        !stdout.ends_with("\n\n"),
        "expected exactly one trailing newline, got two or more",
    );
}
