use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use similar_asserts::assert_eq;
use smol_str::SmolStr;
use zenops_safe_relative_path::{SafeRelativePath, SafeRelativePathBuf, srpath};

use super::*;
use crate::{config_files::ConfigFilePath, git::GitFileStatus};

fn home_path(rel: &str) -> ResolvedConfigFilePath {
    let srp = SafeRelativePath::from_relative_path(rel).unwrap();
    ResolvedConfigFilePath {
        path: ConfigFilePath::in_home(srp),
        full: Arc::from(Path::new("/home/test").join(rel)),
    }
}

fn render_status(status: Status, color: bool, show_diffs: bool) -> String {
    render_status_full(status, color, show_diffs, false)
}

fn render_status_full(status: Status, color: bool, show_diffs: bool, show_clean: bool) -> String {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, color, show_diffs, show_clean);
        r.push(Event::Status(status)).unwrap();
        r.finalize().unwrap();
    }
    String::from_utf8(buf).unwrap()
}

fn render_action(action: AppliedAction) -> String {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, false, false, false);
        r.push(Event::AppliedAction(action)).unwrap();
        r.finalize().unwrap();
    }
    String::from_utf8(buf).unwrap()
}

fn generated(cur: Option<&str>, want: &str, rel: &str, status: FileStatus) -> Status {
    Status::Generated {
        want_content: Arc::from(want),
        cur_content: cur.map(String::from),
        path: home_path(rel),
        status,
    }
}

#[test]
fn generated_ok_emits_nothing() {
    let s = generated(Some("x\n"), "x\n", "a.toml", FileStatus::Ok);
    assert_eq!(render_status(s, false, false), "");
}

#[test]
fn generated_ok_with_show_clean_renders_checkmark_line() {
    let s = generated(Some("x\n"), "x\n", "a.toml", FileStatus::Ok);
    assert_eq!(
        render_status_full(s, false, false, true),
        "✓  ~/a.toml  ok\n"
    );
}

#[test]
fn generated_modified_renders_tilde_marker_and_modified_word() {
    let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
    assert_eq!(render_status(s, false, false), "~  ~/a.toml  modified\n",);
}

#[test]
fn generated_new_renders_plus_marker_and_missing_word() {
    let s = generated(None, "x\n", "a.toml", FileStatus::New);
    assert_eq!(render_status(s, false, false), "+  ~/a.toml  missing\n");
}

#[test]
fn generated_modified_with_diff_renders_summary_then_blank_then_diff() {
    let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
    let got = render_status(s, false, true);
    assert!(got.starts_with("~  ~/a.toml  modified\n"), "{got:?}");
    assert!(
        got.contains("\n--- ~/a.toml (current)\n+++ ~/a.toml (generated)\n"),
        "{got:?}",
    );
    assert!(got.contains("-a\n"), "{got:?}");
    assert!(got.contains("+b\n"), "{got:?}");
}

#[test]
fn generated_new_with_diff_labels_dev_null() {
    let s = generated(None, "x\n", "a.toml", FileStatus::New);
    let got = render_status(s, false, true);
    assert!(got.starts_with("+  ~/a.toml  missing\n"), "{got:?}");
    assert!(
        got.contains("--- /dev/null\n+++ ~/a.toml (generated)\n"),
        "{got:?}",
    );
    assert!(got.contains("+x\n"), "{got:?}");
}

#[test]
fn generated_with_diff_color_off_contains_no_ansi_escapes() {
    let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
    let got = render_status(s, false, true);
    assert!(!got.contains('\x1b'), "unexpected ANSI escape: {got:?}");
}

#[test]
fn generated_with_diff_color_on_emits_ansi_escapes() {
    let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
    let got = render_status(s, true, true);
    assert!(got.contains("\x1b[31m-a\n\x1b[0m"), "{got:?}");
    assert!(got.contains("\x1b[32m+b\n\x1b[0m"), "{got:?}");
}

#[test]
fn generated_empty_current_still_labels_as_current() {
    let s = generated(Some(""), "hi\n", "empty.toml", FileStatus::Modified);
    let got = render_status(s, false, true);
    assert!(got.contains("--- ~/empty.toml (current)\n"), "{got:?}");
}

fn symlink(real: &str, sym: &str, status: SymlinkStatus) -> Status {
    Status::Symlink {
        real: home_path(real),
        symlink: home_path(sym),
        status,
    }
}

#[test]
fn symlink_ok_emits_nothing() {
    let s = symlink("src", "dst", SymlinkStatus::Ok);
    assert_eq!(render_status(s, false, false), "");
}

#[test]
fn symlink_ok_with_show_clean_renders_checkmark_line() {
    let s = symlink("src", "dst", SymlinkStatus::Ok);
    assert_eq!(
        render_status_full(s, false, false, true),
        "✓  ~/dst → ~/src  ok\n",
    );
}

#[test]
fn symlink_wrong_link_renders_arrow_and_actual_target() {
    let s = symlink(
        "src",
        "dst",
        SymlinkStatus::WrongLink(PathBuf::from("/other")),
    );
    assert_eq!(
        render_status(s, false, false),
        "✗  ~/dst → ~/src  wrong target /other\n",
    );
}

#[test]
fn symlink_new_renders_plus_and_arrow() {
    assert_eq!(
        render_status(symlink("s", "d", SymlinkStatus::New), false, false),
        "+  ~/d → ~/s  missing\n",
    );
}

#[test]
fn symlink_is_file_renders_cross_and_description() {
    assert_eq!(
        render_status(symlink("s", "d", SymlinkStatus::IsFile), false, false),
        "✗  ~/d  is a file, expected symlink\n",
    );
}

#[test]
fn symlink_is_dir_renders_cross_and_description() {
    assert_eq!(
        render_status(symlink("s", "d", SymlinkStatus::IsDir), false, false),
        "✗  ~/d  is a dir, expected symlink\n",
    );
}

#[test]
fn symlink_real_missing_reports_source_path() {
    assert_eq!(
        render_status(
            symlink("s", "d", SymlinkStatus::RealPathIsMissing),
            false,
            false,
        ),
        "✗  ~/s  symlink source missing\n",
    );
}

#[test]
fn symlink_dst_dir_missing_reports_symlink_path() {
    assert_eq!(
        render_status(
            symlink(
                "s",
                "d",
                SymlinkStatus::DstDirIsMissing { dir: home_path("") },
            ),
            false,
            false,
        ),
        "✗  ~/d  parent directory missing\n",
    );
}

fn zenops_path(rel: &str) -> ResolvedConfigFilePath {
    let srp = SafeRelativePath::from_relative_path(rel).unwrap();
    ResolvedConfigFilePath {
        path: ConfigFilePath::Zenops(Arc::from(srp)),
        full: Arc::from(Path::new("/home/test/.config/zenops").join(rel)),
    }
}

fn relpath(s: &str) -> SafeRelativePathBuf {
    srpath!("").safe_join(SafeRelativePath::from_relative_path(s).unwrap())
}

#[test]
fn git_variants_render_expected_lines() {
    let repo = zenops_path("");
    let cases: Vec<(GitFileStatus, &str)> = vec![
        (
            GitFileStatus::Modified(relpath("a.toml")),
            "M  ~/.config/zenops/a.toml  modified\n",
        ),
        (
            GitFileStatus::Added(relpath("b.toml")),
            "A  ~/.config/zenops/b.toml  added\n",
        ),
        (
            GitFileStatus::Deleted(relpath("c.toml")),
            "D  ~/.config/zenops/c.toml  deleted\n",
        ),
        (
            GitFileStatus::Untracked(relpath("d.toml")),
            "?  ~/.config/zenops/d.toml  untracked\n",
        ),
        (
            GitFileStatus::Other {
                code: SmolStr::new_static("UU"),
                path: relpath("e.toml"),
            },
            "!  ~/.config/zenops/e.toml  status UU\n",
        ),
    ];
    for (status, want) in cases {
        let s = Status::Git {
            repo: repo.clone(),
            status,
        };
        assert_eq!(render_status(s, false, false), want);
    }
}

fn pkg_missing(pkg: &'static str, install_command: Option<&str>) -> Status {
    Status::Pkg {
        pkg: SmolStr::new_static(pkg),
        status: PkgStatus::Missing {
            install_command: install_command.map(String::from),
        },
    }
}

fn pkg_ok(pkg: &'static str) -> Status {
    Status::Pkg {
        pkg: SmolStr::new_static(pkg),
        status: PkgStatus::Ok,
    }
}

#[test]
fn pkg_missing_with_install_command_includes_hint() {
    assert_eq!(
        render_status(
            pkg_missing("python", Some("brew install python")),
            false,
            false
        ),
        "✗  python  missing — install: brew install python\n",
    );
}

#[test]
fn pkg_missing_without_install_command_is_terse() {
    assert_eq!(
        render_status(pkg_missing("python", None), false, false),
        "✗  python  missing\n",
    );
}

#[test]
fn pkg_ok_without_show_clean_emits_nothing() {
    assert_eq!(render_status(pkg_ok("python"), false, false), "");
}

#[test]
fn pkg_ok_with_show_clean_renders_checkmark_line() {
    assert_eq!(
        render_status_full(pkg_ok("python"), false, false, true),
        "✓  python  ok\n",
    );
}

#[test]
fn git_repo_clean_without_show_clean_emits_nothing() {
    let s = Status::GitRepoClean {
        repo: zenops_path(""),
    };
    assert_eq!(render_status(s, false, false), "");
}

#[test]
fn git_repo_clean_with_show_clean_renders_checkmark_line() {
    let s = Status::GitRepoClean {
        repo: zenops_path(""),
    };
    assert_eq!(
        render_status_full(s, false, false, true),
        "✓  ~/.config/zenops  clean\n",
    );
}

#[test]
fn applied_actions_render_expected_lines() {
    assert_eq!(
        render_action(AppliedAction::UpdatedFile(home_path("a.toml"))),
        "✓  ~/a.toml  updated\n",
    );
    assert_eq!(
        render_action(AppliedAction::CreatedFile(home_path("a.toml"))),
        "✓  ~/a.toml  created\n",
    );
    assert_eq!(
        render_action(AppliedAction::CreatedSymlink {
            real: home_path("src"),
            symlink: home_path("dst"),
        }),
        "✓  ~/dst → ~/src  linked\n",
    );
    assert_eq!(
        render_action(AppliedAction::CreatedDir(home_path("subdir"))),
        "✓  ~/subdir  mkdir\n",
    );
}

#[test]
fn multiple_lines_pad_path_column_to_widest() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, false, false, false);
        r.push(Event::Status(pkg_missing("py", None))).unwrap();
        r.push(Event::Status(generated(
            Some("a\n"),
            "b\n",
            "long/nested/path/file.toml",
            FileStatus::Modified,
        )))
        .unwrap();
        r.finalize().unwrap();
    }
    let got = String::from_utf8(buf).unwrap();
    let wide = "~/long/nested/path/file.toml".chars().count();
    let short = "py".chars().count();
    let pad = wide - short;
    let expected = format!(
        "✗  py{spaces}  missing\n~  ~/long/nested/path/file.toml  modified\n",
        spaces = " ".repeat(pad),
    );
    assert_eq!(got, expected);
}

#[test]
fn finalize_with_no_events_emits_nothing() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, false, false, false);
        r.finalize().unwrap();
    }
    assert_eq!(String::from_utf8(buf).unwrap(), "");
}

#[test]
fn finalize_is_idempotent() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, false, false, false);
        r.push(Event::Status(pkg_missing("x", None))).unwrap();
        r.finalize().unwrap();
        r.finalize().unwrap();
    }
    assert_eq!(String::from_utf8(buf).unwrap(), "✗  x  missing\n");
}

#[test]
fn color_on_wraps_marker_path_and_description_with_expected_escapes() {
    let s = generated(Some("a\n"), "b\n", "a.toml", FileStatus::Modified);
    let got = render_status(s, true, false);
    // yellow marker
    assert!(got.contains("\x1b[33m~\x1b[0m"), "{got:?}");
    // dim path
    assert!(got.contains("\x1b[2m~/a.toml\x1b[0m"), "{got:?}");
    // yellow "modified"
    assert!(got.contains("\x1b[33mmodified\x1b[0m"), "{got:?}");
}

#[test]
fn color_on_pkg_missing_install_command_is_bold_yellow() {
    let got = render_status(pkg_missing("py", Some("brew install py")), true, false);
    assert!(got.contains("\x1b[1;33mbrew install py\x1b[0m"), "{got:?}",);
}

#[test]
fn ok_description_is_green_with_color_on() {
    let s = generated(Some("x\n"), "x\n", "a.toml", FileStatus::Ok);
    let got = render_status_full(s, true, false, true);
    assert!(got.contains("\x1b[32m✓\x1b[0m"), "{got:?}");
    assert!(got.contains("\x1b[32mok\x1b[0m"), "{got:?}");
}

#[test]
fn clean_description_is_green_with_color_on() {
    let s = Status::GitRepoClean {
        repo: zenops_path(""),
    };
    let got = render_status_full(s, true, false, true);
    assert!(got.contains("\x1b[32m✓\x1b[0m"), "{got:?}");
    assert!(got.contains("\x1b[32mclean\x1b[0m"), "{got:?}");
}

#[test]
fn symlink_ok_splits_zenops_prefix_and_bolds_arrow() {
    let s = Status::Symlink {
        real: zenops_path("configs/helix/config.toml"),
        symlink: home_path(".config/helix/config.toml"),
        status: SymlinkStatus::Ok,
    };
    let got = render_status_full(s, true, false, true);
    // left symlink path: dim
    assert!(
        got.contains("\x1b[2m~/.config/helix/config.toml\x1b[0m"),
        "{got:?}",
    );
    // arrow: bold
    assert!(got.contains("\x1b[1m → \x1b[0m"), "{got:?}");
    // right path zenops prefix: extra-dim (fades below the left-side dim)
    assert!(
        got.contains("\x1b[2;38;5;248m~/.config/zenops\x1b[0m"),
        "{got:?}",
    );
    // right path remainder: no opening escape, then reset
    assert!(got.contains("/configs/helix/config.toml\x1b[0m"), "{got:?}");
    // ok label: green
    assert!(got.contains("\x1b[32mok\x1b[0m"), "{got:?}");
}

#[test]
fn git_row_splits_zenops_prefix() {
    let repo = zenops_path("");
    let s = Status::Git {
        repo,
        status: GitFileStatus::Modified(relpath("configs/helix/config.toml")),
    };
    let got = render_status(s, true, false);
    assert!(
        got.contains("\x1b[2;38;5;248m~/.config/zenops\x1b[0m"),
        "{got:?}",
    );
    assert!(got.contains("/configs/helix/config.toml\x1b[0m"), "{got:?}");
    // tail must not be wrapped in a dim open
    assert!(
        !got.contains("\x1b[2m/configs/helix/config.toml"),
        "tail should not be dim: {got:?}",
    );
}

#[test]
fn path_column_padding_matches_visible_width_for_split_paths() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, true, false, false);
        // Short zenops-rooted path (splits): ~/.config/zenops/a.toml (22 chars visible)
        r.push(Event::Status(Status::Git {
            repo: zenops_path(""),
            status: GitFileStatus::Modified(relpath("a.toml")),
        }))
        .unwrap();
        // Longer home path (single dim segment)
        r.push(Event::Status(generated(
            Some("a\n"),
            "b\n",
            "long/nested/path/file.toml",
            FileStatus::Modified,
        )))
        .unwrap();
        r.finalize().unwrap();
    }
    let got = String::from_utf8(buf).unwrap();
    // Strip ANSI escapes to count visible chars per line.
    let stripped: String = {
        let mut out = String::new();
        let mut in_esc = false;
        for c in got.chars() {
            if in_esc {
                if c == 'm' {
                    in_esc = false;
                }
                continue;
            }
            if c == '\x1b' {
                in_esc = true;
                continue;
            }
            out.push(c);
        }
        out
    };
    let lines: Vec<&str> = stripped.lines().collect();
    assert_eq!(lines.len(), 2, "{stripped:?}");
    let short_visible = "~/.config/zenops/a.toml".chars().count();
    let long_visible = "~/long/nested/path/file.toml".chars().count();
    let pad = long_visible - short_visible;
    let expected_short = format!("M  ~/.config/zenops/a.toml{}  modified", " ".repeat(pad));
    let expected_long = "~  ~/long/nested/path/file.toml  modified";
    assert_eq!(lines[0], expected_short, "{stripped:?}");
    assert_eq!(lines[1], expected_long, "{stripped:?}");
}

// ---- JsonOutput -------------------------------------------------------

fn json_line_for_status(status: Status) -> serde_json::Value {
    let mut buf: Vec<u8> = Vec::new();
    JsonOutput::new(&mut buf)
        .push(Event::Status(status))
        .unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.ends_with('\n'), "JSON line must end with newline: {s:?}");
    assert_eq!(
        s.matches('\n').count(),
        1,
        "expected exactly one line: {s:?}"
    );
    serde_json::from_str(s.trim_end()).unwrap()
}

fn json_line_for_action(action: AppliedAction) -> serde_json::Value {
    let mut buf: Vec<u8> = Vec::new();
    JsonOutput::new(&mut buf)
        .push(Event::AppliedAction(action))
        .unwrap();
    let s = String::from_utf8(buf).unwrap();
    serde_json::from_str(s.trim_end()).unwrap()
}

#[test]
fn json_status_generated_tags_event_and_kind() {
    let v = json_line_for_status(generated(
        Some("a\n"),
        "b\n",
        "alpha.toml",
        FileStatus::Modified,
    ));
    assert_eq!(v["event"], "status");
    assert_eq!(v["kind"], "generated");
    assert_eq!(v["want_content"], "b\n");
    assert_eq!(v["cur_content"], "a\n");
    assert_eq!(v["status"], "modified");
}

#[test]
fn json_status_symlink_wrong_link_preserves_target_path() {
    let v = json_line_for_status(symlink(
        "src",
        "dst",
        SymlinkStatus::WrongLink(PathBuf::from("/other")),
    ));
    assert_eq!(v["event"], "status");
    assert_eq!(v["kind"], "symlink");
    assert_eq!(v["status"]["kind"], "wrong_link");
    assert_eq!(v["status"]["data"], "/other");
}

#[test]
fn json_status_git_tags_nested_git_status_kind() {
    let repo = zenops_path("");
    let v = json_line_for_status(Status::Git {
        repo,
        status: GitFileStatus::Untracked(relpath("x.toml")),
    });
    assert_eq!(v["event"], "status");
    assert_eq!(v["kind"], "git");
    assert_eq!(v["status"]["kind"], "untracked");
    assert_eq!(v["status"]["data"], "x.toml");
}

#[test]
fn json_status_pkg_missing_preserves_install_command() {
    let v = json_line_for_status(pkg_missing("python", Some("brew install python")));
    assert_eq!(v["event"], "status");
    assert_eq!(v["kind"], "pkg");
    assert_eq!(v["pkg"], "python");
    assert_eq!(v["status"]["kind"], "missing");
    assert_eq!(
        v["status"]["data"]["install_command"],
        "brew install python"
    );
}

#[test]
fn json_status_pkg_ok_tags_kind_ok() {
    let v = json_line_for_status(pkg_ok("python"));
    assert_eq!(v["event"], "status");
    assert_eq!(v["kind"], "pkg");
    assert_eq!(v["pkg"], "python");
    assert_eq!(v["status"]["kind"], "ok");
}

#[test]
fn json_status_git_repo_clean_emits_event() {
    let repo = zenops_path("");
    let v = json_line_for_status(Status::GitRepoClean { repo });
    assert_eq!(v["event"], "status");
    assert_eq!(v["kind"], "git_repo_clean");
    assert_eq!(v["repo"]["path"]["path"], "");
}

#[test]
fn json_applied_action_tags_event_and_kind() {
    let v = json_line_for_action(AppliedAction::CreatedFile(home_path("a.toml")));
    assert_eq!(v["event"], "applied_action");
    assert_eq!(v["kind"], "created_file");
}

#[test]
fn json_multiple_events_produce_jsonl() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut out = JsonOutput::new(&mut buf);
        out.push(Event::Status(pkg_missing("python", None)))
            .unwrap();
        out.push(Event::AppliedAction(AppliedAction::CreatedDir(home_path(
            "d",
        ))))
        .unwrap();
    }
    let s = String::from_utf8(buf).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 2, "{s:?}");
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first["event"], "status");
    assert_eq!(second["event"], "applied_action");
}

// ---- New event types: PkgEntry / DoctorCheck / InitSummary -----------

fn pkg_entry_pkg(name: &'static str, state: PkgEntryState) -> PkgEntry {
    PkgEntry::Pkg {
        name: SmolStr::new_static(name),
        key: SmolStr::new_static(name),
        description: None,
        state,
        matched_detect: None,
        install_hints: PkgInstallHints::default(),
    }
}

fn render_pkg_entries(entries: Vec<PkgEntry>, color: bool) -> String {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, color, false, false);
        for e in entries {
            r.push(Event::PkgEntry(e)).unwrap();
        }
        r.finalize().unwrap();
    }
    String::from_utf8(buf).unwrap()
}

#[test]
fn pkg_entries_pad_name_column_to_widest() {
    let got = render_pkg_entries(
        vec![
            pkg_entry_pkg("py", PkgEntryState::Missing),
            pkg_entry_pkg("starship", PkgEntryState::Installed),
        ],
        false,
    );
    let lines: Vec<&str> = got.lines().collect();
    // "starship" (8 chars) is the widest name; "py" should pad to 8.
    assert_eq!(lines[0], "✗ py      ", "{got:?}");
    assert_eq!(lines[1], "✓ starship", "{got:?}");
}

#[test]
fn pkg_entry_disabled_uses_dash_marker() {
    let got = render_pkg_entries(vec![pkg_entry_pkg("ghost", PkgEntryState::Disabled)], false);
    assert!(got.starts_with("- ghost"), "{got:?}");
}

#[test]
fn pkg_entry_missing_with_brew_hint_renders_indented_hint_line() {
    let got = render_pkg_entries(
        vec![PkgEntry::Pkg {
            name: SmolStr::new_static("foo"),
            key: SmolStr::new_static("foo"),
            description: None,
            state: PkgEntryState::Missing,
            matched_detect: None,
            install_hints: PkgInstallHints {
                brew: vec!["foo-formula".into()],
            },
        }],
        false,
    );
    assert!(got.contains("✗ foo"), "{got:?}");
    assert!(got.contains("brew: foo-formula"), "{got:?}");
}

#[test]
fn pkg_aggregate_install_renders_blank_line_then_footer() {
    let got = render_pkg_entries(
        vec![
            pkg_entry_pkg("foo", PkgEntryState::Missing),
            PkgEntry::AggregateInstall {
                pkg_manager: "brew".into(),
                command: "brew install foo".into(),
                packages: vec!["foo".into()],
            },
        ],
        false,
    );
    // Last two non-empty lines: aggregate footer follows a blank line.
    let lines: Vec<&str> = got.lines().collect();
    let footer_idx = lines
        .iter()
        .position(|l| l.contains("To install all missing"))
        .expect("expected footer line");
    assert_eq!(lines[footer_idx - 1], "", "{got:?}");
    assert!(
        lines[footer_idx].contains("via brew: brew install foo"),
        "{got:?}",
    );
}

#[test]
fn pkg_no_manager_warning_renders_inline_before_pkg_block() {
    let got = render_pkg_entries(
        vec![
            PkgEntry::NoPackageManagerDetected {
                supported: vec!["brew".into()],
            },
            pkg_entry_pkg("foo", PkgEntryState::Missing),
        ],
        false,
    );
    let lines: Vec<&str> = got.lines().collect();
    assert!(lines[0].contains("no known package manager"), "{got:?}");
    assert!(lines[0].contains("Supported managers: brew"), "{got:?}");
    assert!(lines[1].contains("foo"), "{got:?}");
}

fn render_doctor_checks(checks: Vec<DoctorCheck>, color: bool) -> String {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, color, false, false);
        for c in checks {
            r.push(Event::DoctorCheck(c)).unwrap();
        }
        r.finalize().unwrap();
    }
    String::from_utf8(buf).unwrap()
}

fn doctor_check(
    section: DoctorSection,
    label: &'static str,
    severity: DoctorSeverity,
    value: &str,
    hint: Option<&str>,
) -> DoctorCheck {
    DoctorCheck::Check {
        section,
        label: SmolStr::new_static(label),
        severity,
        value: value.to_string(),
        hint: hint.map(String::from),
        detail: Vec::new(),
    }
}

#[test]
fn doctor_check_groups_by_section_with_blank_separator() {
    let got = render_doctor_checks(
        vec![
            DoctorCheck::SectionHeader {
                section: DoctorSection::System,
            },
            doctor_check(
                DoctorSection::System,
                "os:",
                DoctorSeverity::Info,
                "macos",
                None,
            ),
            DoctorCheck::SectionHeader {
                section: DoctorSection::Repo,
            },
            doctor_check(
                DoctorSection::Repo,
                "git repo:",
                DoctorSeverity::Ok,
                "yes",
                None,
            ),
        ],
        false,
    );
    // Bold-stripped, but no color: lines are plain.
    let want =
        "System\n  os:            macos\n\nConfig repo (~/.config/zenops)\n  git repo:      yes\n";
    assert_eq!(got, want, "{got:?}");
}

#[test]
fn doctor_check_with_hint_renders_hint_after_value() {
    let got = render_doctor_checks(
        vec![doctor_check(
            DoctorSection::System,
            "git:",
            DoctorSeverity::Bad,
            "not found on PATH",
            Some("install git"),
        )],
        false,
    );
    assert!(got.contains("git:"), "{got:?}");
    assert!(got.contains("not found on PATH"), "{got:?}");
    assert!(got.contains("install git"), "{got:?}");
}

#[test]
fn doctor_check_with_detail_indents_each_line_under_row() {
    let got = render_doctor_checks(
        vec![DoctorCheck::Check {
            section: DoctorSection::Config,
            label: SmolStr::new_static("status:"),
            severity: DoctorSeverity::Bad,
            value: "parse error".into(),
            hint: None,
            detail: vec!["/path/to/config.toml".into(), "expected `]`".into()],
        }],
        false,
    );
    assert!(got.contains("    /path/to/config.toml\n"), "{got:?}");
    assert!(got.contains("    expected `]`\n"), "{got:?}");
}

#[test]
fn init_summary_renders_summary_with_remote_shell_and_pkg_count() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, false, false, false);
        r.push(Event::InitSummary(InitSummary {
            clone_path: PathBuf::from("/home/test/.config/zenops"),
            remote: Some("git@example.com:cfg.git".into()),
            shell: Some("bash".into()),
            pkg_count: 12,
        }))
        .unwrap();
        r.finalize().unwrap();
    }
    let got = String::from_utf8(buf).unwrap();
    assert!(
        got.contains("Cloned into /home/test/.config/zenops"),
        "{got:?}",
    );
    assert!(got.contains("remote: git@example.com:cfg.git"), "{got:?}");
    assert!(got.contains("shell:  bash"), "{got:?}");
    assert!(got.contains("pkgs:   12"), "{got:?}");
    assert!(got.contains("Next: run `zenops apply`"), "{got:?}");
}

fn json_line_for_pkg_entry(entry: PkgEntry) -> serde_json::Value {
    let mut buf: Vec<u8> = Vec::new();
    JsonOutput::new(&mut buf)
        .push(Event::PkgEntry(entry))
        .unwrap();
    let s = String::from_utf8(buf).unwrap();
    serde_json::from_str(s.trim_end()).unwrap()
}

#[test]
fn json_pkg_entry_pkg_tags_event_and_kind_with_state() {
    let v = json_line_for_pkg_entry(PkgEntry::Pkg {
        name: SmolStr::new_static("starship"),
        key: SmolStr::new_static("starship"),
        description: Some("cross-shell prompt".into()),
        state: PkgEntryState::Missing,
        matched_detect: None,
        install_hints: PkgInstallHints {
            brew: vec!["starship".into()],
        },
    });
    assert_eq!(v["event"], "pkg_entry");
    assert_eq!(v["kind"], "pkg");
    assert_eq!(v["name"], "starship");
    assert_eq!(v["key"], "starship");
    assert_eq!(v["state"], "missing");
    assert_eq!(v["install_hints"]["brew"][0], "starship");
}

#[test]
fn json_pkg_entry_aggregate_install_carries_command_and_packages() {
    let v = json_line_for_pkg_entry(PkgEntry::AggregateInstall {
        pkg_manager: "brew".into(),
        command: "brew install foo bar".into(),
        packages: vec!["foo".into(), "bar".into()],
    });
    assert_eq!(v["event"], "pkg_entry");
    assert_eq!(v["kind"], "aggregate_install");
    assert_eq!(v["pkg_manager"], "brew");
    assert_eq!(v["command"], "brew install foo bar");
    assert_eq!(v["packages"][0], "foo");
    assert_eq!(v["packages"][1], "bar");
}

#[test]
fn json_pkg_entry_no_manager_warning_is_event() {
    let v = json_line_for_pkg_entry(PkgEntry::NoPackageManagerDetected {
        supported: vec!["brew".into()],
    });
    assert_eq!(v["event"], "pkg_entry");
    assert_eq!(v["kind"], "no_package_manager_detected");
    assert_eq!(v["supported"][0], "brew");
}

fn json_line_for_doctor_check(check: DoctorCheck) -> Option<serde_json::Value> {
    let mut buf: Vec<u8> = Vec::new();
    JsonOutput::new(&mut buf)
        .push(Event::DoctorCheck(check))
        .unwrap();
    let s = String::from_utf8(buf).unwrap();
    if s.is_empty() {
        None
    } else {
        Some(serde_json::from_str(s.trim_end()).unwrap())
    }
}

#[test]
fn json_doctor_check_includes_section_severity_label_value() {
    let v = json_line_for_doctor_check(doctor_check(
        DoctorSection::System,
        "os:",
        DoctorSeverity::Info,
        "linux",
        None,
    ))
    .expect("Check variant should emit JSON");
    assert_eq!(v["event"], "doctor_check");
    assert_eq!(v["kind"], "check");
    assert_eq!(v["section"], "system");
    assert_eq!(v["label"], "os:");
    assert_eq!(v["severity"], "info");
    assert_eq!(v["value"], "linux");
}

#[test]
fn json_doctor_check_section_header_is_skipped() {
    let v = json_line_for_doctor_check(DoctorCheck::SectionHeader {
        section: DoctorSection::Packages,
    });
    assert!(
        v.is_none(),
        "section header should not produce a JSON line, got: {v:?}",
    );
}

#[test]
fn json_init_summary_includes_all_fields() {
    let mut buf: Vec<u8> = Vec::new();
    JsonOutput::new(&mut buf)
        .push(Event::InitSummary(InitSummary {
            clone_path: PathBuf::from("/home/test/.config/zenops"),
            remote: Some("git@example.com:cfg.git".into()),
            shell: Some("zsh".into()),
            pkg_count: 7,
        }))
        .unwrap();
    let s = String::from_utf8(buf).unwrap();
    let v: serde_json::Value = serde_json::from_str(s.trim_end()).unwrap();
    assert_eq!(v["event"], "init_summary");
    assert_eq!(v["clone_path"], "/home/test/.config/zenops");
    assert_eq!(v["remote"], "git@example.com:cfg.git");
    assert_eq!(v["shell"], "zsh");
    assert_eq!(v["pkg_count"], 7);
}

#[test]
fn terminal_renderer_flushes_status_block_before_pkg_block() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut r = TerminalRenderer::new(&mut buf, false, false, false);
        r.push(Event::Status(pkg_missing("py", None))).unwrap();
        r.push(Event::PkgEntry(pkg_entry_pkg(
            "foo",
            PkgEntryState::Missing,
        )))
        .unwrap();
        r.finalize().unwrap();
    }
    let got = String::from_utf8(buf).unwrap();
    let lines: Vec<&str> = got.lines().collect();
    // First line = the status row from the Status event; second line = the
    // first pkg row. The two blocks are independent — no shared
    // column padding bleeds across.
    assert!(lines[0].starts_with("✗  py"), "{got:?}");
    assert!(lines[1].starts_with("✗ foo"), "{got:?}");
}

// ---- Error propagation -----------------------------------------------

struct FailingWriter;

impl std::io::Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("boom"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn terminal_renderer_surfaces_writer_errors_on_finalize() {
    let mut w = FailingWriter;
    let mut r = TerminalRenderer::new(&mut w, false, false, false);
    r.push(Event::Status(pkg_missing("x", None))).unwrap();
    let err = r.finalize().unwrap_err();
    assert!(matches!(err, OutputError::Io(_)), "unexpected: {err:?}");
}

#[test]
fn json_output_surfaces_writer_errors() {
    let mut w = FailingWriter;
    let err = JsonOutput::new(&mut w)
        .push(Event::Status(pkg_missing("x", None)))
        .unwrap_err();
    // `serde_json::to_writer` wraps the underlying IO failure in its own
    // `serde_json::Error`, which lifts into `OutputError::Json`. Either
    // variant is acceptable — we just care the error surfaced.
    assert!(
        matches!(err, OutputError::Io(_) | OutputError::Json(_)),
        "unexpected: {err:?}",
    );
}
