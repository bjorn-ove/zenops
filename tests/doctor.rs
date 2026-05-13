use xshell::{Shell, cmd};
use zenops::{
    Cmd,
    output::{PkgStatus, Status},
};
use zenops_safe_relative_path::srpath;

use test_env::{Entry, Output, paths};

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
        exists = "/definitely/does/not/exist/zenops-doctor-test"
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
fn doctor_reports_missing_zenops_dir_as_bad() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    // No zenops dir at all → repo_block must emit a Bad "path: missing"
    // row pointing to `zenops init <url>`.
    let env = test_env::TestEnv::load();
    env.delete_dir_all(test_env::paths::ZENOPS_DIR);

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_missing = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Repo,
                label,
                severity: DoctorSeverity::Bad,
                value,
                ..
            }) if label == "path:" && value == "missing"
        )
    });
    assert!(
        has_missing,
        "expected a Bad path:missing repo check, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_reports_no_remote_warn_when_repo_has_no_origin() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    // init_config sets up a git repo but does NOT add an `origin` remote.
    let env = test_env::TestEnv::load();
    env.init_config("");

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_no_remote = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Repo,
                label,
                severity: DoctorSeverity::Warn,
                value,
                ..
            }) if label == "remote:" && value == "none"
        )
    });
    assert!(
        has_no_remote,
        "expected a Warn remote:none row, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_reports_remote_info_when_origin_configured() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    let env = test_env::TestEnv::load();
    env.init_config_with_remote("");

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_remote_info = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Repo,
                label,
                severity: DoctorSeverity::Info,
                value,
                ..
            }) if label == "remote:" && value.contains("remote.git")
        )
    });
    assert!(
        has_remote_info,
        "expected an Info remote: row with the bare repo path, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_reports_uncommitted_changes_as_warn() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};
    use zenops_safe_relative_path::srpath;

    // Clean repo first, then dirty it without committing.
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.write_zenops_file(srpath!("untracked"), "stale\n", None);

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_uncommitted_warn = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Repo,
                label,
                severity: DoctorSeverity::Warn,
                value,
                ..
            }) if label == "uncommitted:" && value == "yes"
        )
    });
    assert!(
        has_uncommitted_warn,
        "expected a Warn uncommitted:yes row, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_user_block_warns_on_unset_name_and_email() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    // No `[user]` section in config → both name: and email: should be Warn.
    let env = test_env::TestEnv::load();
    env.init_config("");

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_user_name_warn = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::User,
                label,
                severity: DoctorSeverity::Warn,
                value,
                ..
            }) if label == "name:" && value == "unset"
        )
    });
    let has_user_email_warn = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::User,
                label,
                severity: DoctorSeverity::Warn,
                value,
                ..
            }) if label == "email:" && value == "unset"
        )
    });
    assert!(
        has_user_name_warn && has_user_email_warn,
        "expected Warn user:name/email unset rows, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_user_block_emits_info_when_name_and_email_set() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [user]
        name = "Ada Lovelace"
        email = "ada@example.com"
        "#,
    );

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_name_info = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::User,
                label,
                severity: DoctorSeverity::Info,
                value,
                ..
            }) if label == "name:" && value == "Ada Lovelace"
        )
    });
    let has_email_info = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::User,
                label,
                severity: DoctorSeverity::Info,
                value,
                ..
            }) if label == "email:" && value == "ada@example.com"
        )
    });
    assert!(
        has_name_info && has_email_info,
        "expected Info user:name/email rows, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_emits_section_headers_for_every_section() {
    use zenops::output::{DoctorCheck, DoctorSection};

    // Sanity check that each section opens with a SectionHeader event so
    // the renderer always has a title to print, including Packages (which
    // has no DoctorCheck rows of its own — content comes via Status::Pkg).
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [user]
        name = "Ada"
        email = "ada@example.com"
        "#,
    );

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let header = |want: DoctorSection| {
        out.entries.iter().any(|e| {
            matches!(
                e,
                Entry::Doctor(DoctorCheck::SectionHeader { section }) if *section == want,
            )
        })
    };
    for section in [
        DoctorSection::System,
        DoctorSection::Repo,
        DoctorSection::Config,
        DoctorSection::PkgManager,
        DoctorSection::User,
        DoctorSection::Shell,
        DoctorSection::Packages,
    ] {
        assert!(
            header(section),
            "missing SectionHeader for {section:?}, got: {:?}",
            out.entries,
        );
    }
}

#[test]
fn doctor_reports_unreadable_config_as_bad() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    // Hits the non-NotFound `OpenDb` arm: the file exists but can't be
    // read. chmod 0o000 produces PermissionDenied. PermGuard restores the
    // mode on drop so tempfile cleanup can recurse into the dir.
    let env = test_env::TestEnv::load();
    env.init_config("");
    let _guard = env.chmod(test_env::paths::ZENOPS_CONFIG, 0o000);

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail when config.toml is unreadable");
    let has_unreadable = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Config,
                label,
                severity: DoctorSeverity::Bad,
                value,
                ..
            }) if label == "status:" && value == "unreadable"
        )
    });
    assert!(
        has_unreadable,
        "expected a Bad status:unreadable config check, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_reports_parse_error_with_invalid_type_hint() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    // [shell] is a tagged enum; `type = 42` triggers a serde "invalid
    // type" error during deserialization. Doctor's parse-error arm should
    // attach the README hint detail line for the invalid-type / missing-
    // field family.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [shell]
        type = 42
        "#,
    );

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail on a parse error");
    let detail = out
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
        detail.iter().any(|line| line.contains("README.md")),
        "expected README hint in invalid-type parse error detail, got: {detail:?}",
    );
}

#[test]
fn doctor_reports_parse_error_with_missing_field_hint() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};

    // PkgConfig requires `install_hint`; declaring a [pkg.x] with nothing
    // else triggers the serde "missing field" error path.
    let env = test_env::TestEnv::load();
    env.init_config(
        r#"
        [pkg.zenops-doctor-missing-field]
        description = "no install_hint"
        "#,
    );

    let out = env
        .run(&Cmd::Doctor)
        .expect("doctor must not fail on a parse error");
    let detail = out
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
        detail.iter().any(|line| line.contains("README.md")),
        "expected README hint in missing-field parse error detail, got: {detail:?}",
    );
}

#[test]
fn doctor_reports_zenops_dir_not_a_git_repo() {
    use zenops::output::{DoctorCheck, DoctorSection, DoctorSeverity};
    use zenops_safe_relative_path::srpath;

    // Strip the .git dir: zenops dir exists but is not a git repo. Hits
    // the `git.is_git_repo()? == false` branch in `repo_block`.
    let env = test_env::TestEnv::load();
    env.init_config("");
    env.delete_dir_all(srpath!("home/bob/.config/zenops/.git"));

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let has_no_git = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Repo,
                label,
                severity: DoctorSeverity::Warn,
                value,
                ..
            }) if label == "git repo:" && value == "no"
        )
    });
    assert!(
        has_no_git,
        "expected a Warn git repo:no row, got: {:?}",
        out.entries,
    );
}

#[test]
fn doctor_omits_branch_row_on_detached_head() {
    use zenops::output::{DoctorCheck, DoctorSection};

    // After init_config, the zenops repo is on a normal branch. Detach HEAD
    // by checking out the commit's SHA directly. doctor's `repo_block`
    // filters `git rev-parse --abbrev-ref HEAD == "HEAD"` and skips the
    // `branch:` info row entirely — covers the false arm of the
    // `if let Some(b) = branch` filter.
    let env = test_env::TestEnv::load();
    env.init_config("");

    let zenops = env.resolve_path(paths::ZENOPS_DIR);
    let sh = Shell::new().unwrap();
    let _dir = sh.push_dir(&zenops);
    let head_sha = cmd!(sh, "git rev-parse HEAD").read().unwrap();
    cmd!(sh, "git checkout --detach {head_sha}")
        .ignore_stdout()
        .ignore_stderr()
        .run()
        .unwrap();
    drop(_dir);

    let out = env.run(&Cmd::Doctor).expect("doctor must succeed");
    let any_branch_row = out.entries.iter().any(|e| {
        matches!(
            e,
            Entry::Doctor(DoctorCheck::Check {
                section: DoctorSection::Repo,
                label,
                ..
            }) if label == "branch:"
        )
    });
    assert!(
        !any_branch_row,
        "detached HEAD must not emit a branch: row, got: {:?}",
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
