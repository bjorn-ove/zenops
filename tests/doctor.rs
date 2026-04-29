use zenops::{
    Cmd,
    output::{PkgStatus, Status},
};
use zenops_safe_relative_path::srpath;

use test_env::{Entry, Output};

mod test_env;

/// Filter `entries` down to just the `Entry::Status` variants — doctor's
/// non-pkg sections are tested separately via `Entry::Doctor`.
fn status_entries_only(out: &Output) -> Vec<&Entry> {
    out.entries
        .iter()
        .filter(|e| matches!(e, Entry::Status(_)))
        .collect()
}

#[test]
fn doctor_runs_without_config() {
    // No config.toml, zenops dir is not a git repo. Where `status` bails
    // with Error::OpenDb, `doctor` must swallow the load failure and
    // finish successfully — that's the whole point of the command.
    let env = test_env::TestEnv::load();

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail when config.toml is missing");
    // No Status events — push_pkg_health is never reached without a
    // config. Doctor's narrative checks live under `Entry::Doctor`.
    assert!(
        status_entries_only(&out).is_empty(),
        "expected no status events without a config, got: {:?}",
        status_entries_only(&out),
    );
}

#[test]
fn doctor_runs_with_broken_config() {
    // Syntactically invalid TOML. Any other command returns Err(ParseDb);
    // `doctor` must keep going and report the failure inline.
    let env = test_env::TestEnv::load();
    env.write_zenops_file(srpath!("config.toml"), "[[[ not toml", None);

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail on a malformed config.toml");
    assert!(
        status_entries_only(&out).is_empty(),
        "expected no status events on a malformed config, got: {:?}",
        status_entries_only(&out),
    );
}

#[test]
fn doctor_runs_with_unknown_field_in_config() {
    // `deny_unknown_fields` on StoredConfig catches typos / renamed fields.
    // Doctor must render the error instead of propagating.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = "bash"
        totally_not_a_real_field = 1
        "#,
    );

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail on an unknown-field ParseDb error");
    assert!(
        status_entries_only(&out).is_empty(),
        "expected no status events when config fails to parse, got: {:?}",
        status_entries_only(&out),
    );
}

#[test]
fn doctor_emits_pkg_missing_for_enable_on_with_missing_detect() {
    // With a valid config that declares `enable = "on"` for a pkg whose
    // detect strategy can't match on the test host, doctor reuses
    // Config::push_pkg_health and emits a Status::Pkg::Missing event —
    // same channel the `status` command uses.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.zenops-doctor-test]
        enable = "on"
        [pkg.zenops-doctor-test.install_hint.brew]
        packages = ["zenops-doctor-fake"]
        [pkg.zenops-doctor-test.detect]
        type = "file"
        path = "/definitely/does/not/exist/zenops-doctor-test"
        "#,
    );

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must succeed when config loads");

    let status_only = status_entries_only(&out);
    // The exact set of Status::Pkg events depends on whether the host
    // running the test has brew / cargo / sk / etc. installed; but our
    // fake pkg with its guaranteed-missing detect must appear.
    let has_expected = status_only.iter().any(|e| {
        matches!(
            e,
            Entry::Status(Status::Pkg {
                pkg,
                status: PkgStatus::Missing { .. },
            }) if pkg == "zenops-doctor-test"
        )
    });
    assert!(
        has_expected,
        "expected a Status::Pkg::Missing for zenops-doctor-test, got: {status_only:?}",
    );
    // Every Status entry should be a Status::Pkg (doctor never emits Git
    // or ConfigFile events — those are status' territory).
    for e in &status_only {
        assert!(
            matches!(e, Entry::Status(Status::Pkg { .. })),
            "doctor emitted unexpected status event: {e:?}",
        );
    }
}

#[test]
fn doctor_emits_doctor_check_events_for_system_section() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    let env = test_env::TestEnv::load();
    env.init_config("");

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    // The System section always includes an `os:` Info row.
    let has_os_info = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::System,
                label,
                severity: DoctorSeverity::Info,
                ..
            }) if label == "os:"
        )
    });
    assert!(
        has_os_info,
        "expected a system/os: info DoctorCheck, got: {:?}",
        out.entries,
    );
    // And a SectionHeader event opens each section so the renderer can
    // print its bold title.
    let has_system_header = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::SectionHeader {
                section: DoctorSection::System
            })
        )
    });
    assert!(
        has_system_header,
        "expected a System SectionHeader event, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_emits_bad_check_with_detail_for_parse_error() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    let env = test_env::TestEnv::load();
    env.write_zenops_file(srpath!("config.toml"), "[[[ not toml", None);

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail on a malformed config.toml");
    let parse_error = out
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Config,
                label,
                severity: DoctorSeverity::Bad,
                value,
                detail,
                ..
            }) if label == "status:" && value == "parse error" => Some(detail),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a parse-error doctor check, got: {:?}",
                out.entries
            )
        });
    assert!(
        !parse_error.is_empty(),
        "parse-error check should carry multi-line detail body, got: {parse_error:?}",
    );
}
