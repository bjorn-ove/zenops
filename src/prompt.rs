//! Interactive confirmation for `zenops apply`.
//!
//! [`Prompter`] is the trait `apply` calls to confirm each
//! [`PendingChange`] and to resolve the pre-apply
//! [`PreApplyDecision`] when the zenops repo has uncommitted changes.
//! Three impls cover the modes:
//!
//! - [`TerminalPrompter`] — interactive, prints a colorized diff and reads
//!   answers through [`crate::line_prompter::RustylinePrompter`].
//! - [`YesPrompter`] — accepts everything, used for `--yes`.
//! - [`DryRunPrompter`] — shows the same diff the terminal prompter would,
//!   but always answers "no", used for `--dry-run`.

use std::io::{self, Write};

use similar::{ChangeTag, DiffOp, TextDiff};

use crate::{
    ansi::{color_code, color_reset},
    error::Error,
    line_prompter::{LineOutcome, LinePrompter, RustylinePrompter},
    output::ResolvedConfigFilePath,
};

/// One change `apply` is about to make. Borrowed so the prompter can
/// render it without taking ownership of large buffers (especially the
/// generated body and the diff).
pub enum PendingChange<'a> {
    /// Write a brand-new generated file.
    CreateFile {
        /// Where the file will land.
        path: &'a ResolvedConfigFilePath,
        /// Body to write.
        content: &'a str,
    },
    /// Apply one hunk of a multi-hunk update to an existing generated
    /// file. Hunks are confirmed independently so the user can accept
    /// some and reject others.
    UpdateFileHunk {
        /// Target file.
        path: &'a ResolvedConfigFilePath,
        /// 1-based hunk index.
        index: usize,
        /// Total number of hunks in this file.
        total: usize,
        /// Full text diff (carries both sides plus context).
        diff: &'a TextDiff<'a, 'a, str>,
        /// The contiguous slice of diff ops that make up this hunk.
        ops: &'a [DiffOp],
    },
    /// Create a symlink in an existing parent directory.
    CreateSymlink {
        /// Symlink target (a file in the zenops repo).
        real: &'a ResolvedConfigFilePath,
        /// Where the symlink will live.
        symlink: &'a ResolvedConfigFilePath,
    },
    /// Create a symlink whose parent directory must also be created. A
    /// distinct variant so the prompter can warn the user that an extra
    /// `mkdir -p` will happen.
    CreateSymlinkWithParent {
        /// Symlink target.
        real: &'a ResolvedConfigFilePath,
        /// Where the symlink will live.
        symlink: &'a ResolvedConfigFilePath,
        /// The parent directory that will be created first.
        parent: &'a ResolvedConfigFilePath,
    },
    /// Replace a symlink that exists but points at the wrong target. zenops
    /// owns the managed entry, so the apply pass can correct drift; the
    /// prompter shows the current (wrong) target so the user can confirm
    /// before it's overwritten.
    ReplaceWrongSymlink {
        /// New target the symlink should point at.
        real: &'a ResolvedConfigFilePath,
        /// Where the (existing, wrong) symlink lives.
        symlink: &'a ResolvedConfigFilePath,
        /// What the symlink currently points at.
        current_target: &'a std::path::Path,
    },
}

/// Outcome of the pre-apply prompt that fires when the zenops repo has
/// uncommitted changes.
#[derive(Debug, PartialEq, Eq)]
pub enum PreApplyDecision {
    /// Stage everything, commit with `message`, push, then continue.
    CommitAndPush {
        /// Commit message the user typed.
        message: String,
    },
    /// Apply without committing — the repo stays dirty.
    Continue,
    /// Bail out of the apply.
    Abort,
}

/// Parse a single-char answer from the pre-apply prompt. `c` commits & pushes,
/// `y`/empty continues without committing, `n` aborts. Returns `None` for any
/// other input so the caller can re-prompt.
pub fn parse_pre_apply_input(line: &str) -> Option<PreApplyAnswer> {
    match line.trim().to_ascii_lowercase().as_str() {
        "c" | "commit" => Some(PreApplyAnswer::Commit),
        "" | "y" | "yes" => Some(PreApplyAnswer::Continue),
        "n" | "no" | "abort" => Some(PreApplyAnswer::Abort),
        _ => None,
    }
}

/// Parsed answer from the pre-apply prompt; lives separately from
/// [`PreApplyDecision`] because `Commit` here doesn't yet carry the
/// commit message — the prompter reads that in a follow-up step.
#[derive(Debug, PartialEq, Eq)]
pub enum PreApplyAnswer {
    /// User wants to commit (and supply a message next).
    Commit,
    /// User wants to apply without committing.
    Continue,
    /// User wants to abort the apply.
    Abort,
}

/// How `apply` consults the user before each change. Stateful (e.g.
/// terminal locks); always borrowed mutably.
pub trait Prompter {
    /// Ask whether to perform a single pending change. Returning `Ok(false)`
    /// skips this change without aborting the run.
    fn confirm(&mut self, change: PendingChange<'_>) -> Result<bool, Error>;
    /// Ask what to do when the zenops config repo has uncommitted changes.
    /// Called once per `apply`, before any per-change prompts.
    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error>;
}

/// Prompter for `--yes`: accepts every change and continues past the
/// pre-apply prompt without committing.
pub struct YesPrompter;

impl Prompter for YesPrompter {
    fn confirm(&mut self, _change: PendingChange<'_>) -> Result<bool, Error> {
        Ok(true)
    }
    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error> {
        Ok(PreApplyDecision::Continue)
    }
}

/// Interactive prompter: renders each pending change to stdout and reads
/// y/n (or commit/continue/abort) through rustyline so the user gets
/// Home/End, arrow keys, and proper UTF-8 line editing.
pub struct TerminalPrompter {
    color: bool,
    line: RustylinePrompter,
}

impl TerminalPrompter {
    /// Build a prompter that uses ANSI color when `color` is true. Fails
    /// if rustyline can't open the controlling terminal.
    pub fn new(color: bool) -> Result<Self, Error> {
        let line = RustylinePrompter::new().map_err(Error::PromptRead)?;
        Ok(Self { color, line })
    }
}

impl Prompter for TerminalPrompter {
    fn confirm(&mut self, change: PendingChange<'_>) -> Result<bool, Error> {
        {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            render_change(&mut out, &change, self.color).map_err(Error::PromptRead)?;
        }
        loop {
            let line = match self.line.read_line("[Y/n] ").map_err(Error::PromptRead)? {
                LineOutcome::Line(s) => s,
                LineOutcome::Eof => return Ok(false),
                LineOutcome::Interrupted => return Err(Error::PromptInterrupted),
            };
            match line.trim().to_ascii_lowercase().as_str() {
                "" | "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => {
                    self.line
                        .writeln("Please answer y or n.")
                        .map_err(Error::PromptRead)?;
                }
            }
        }
    }

    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error> {
        loop {
            let line = match self
                .line
                .read_line("[c]ommit & push / [Y]continue / [n]abort: ")
                .map_err(Error::PromptRead)?
            {
                LineOutcome::Line(s) => s,
                LineOutcome::Eof => return Ok(PreApplyDecision::Abort),
                LineOutcome::Interrupted => return Err(Error::PromptInterrupted),
            };
            match parse_pre_apply_input(&line) {
                Some(PreApplyAnswer::Commit) => {
                    let message = read_commit_message(&mut self.line)?;
                    return Ok(PreApplyDecision::CommitAndPush { message });
                }
                Some(PreApplyAnswer::Continue) => return Ok(PreApplyDecision::Continue),
                Some(PreApplyAnswer::Abort) => return Ok(PreApplyDecision::Abort),
                None => {
                    self.line
                        .writeln("Please answer c, y, or n.")
                        .map_err(Error::PromptRead)?;
                }
            }
        }
    }
}

fn read_commit_message(prompter: &mut dyn LinePrompter) -> Result<String, Error> {
    loop {
        let line = match prompter
            .read_line("Commit message: ")
            .map_err(Error::PromptRead)?
        {
            LineOutcome::Line(s) => s,
            LineOutcome::Eof => {
                return Err(Error::PromptRead(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "no commit message provided",
                )));
            }
            LineOutcome::Interrupted => return Err(Error::PromptInterrupted),
        };
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            prompter
                .writeln("Commit message cannot be empty.")
                .map_err(Error::PromptRead)?;
            continue;
        }
        return Ok(trimmed);
    }
}

/// Prompter for `--dry-run`: shows the same per-change preview as the
/// terminal prompter but always answers "no", so nothing is written.
pub struct DryRunPrompter {
    color: bool,
}

impl DryRunPrompter {
    /// Build a dry-run prompter that uses ANSI color when `color` is true.
    pub fn new(color: bool) -> Self {
        Self { color }
    }
}

impl Prompter for DryRunPrompter {
    fn confirm(&mut self, change: PendingChange<'_>) -> Result<bool, Error> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        render_change(&mut out, &change, self.color).map_err(Error::PromptRead)?;
        writeln!(out, "[Y/n] (dry-run: skipping)").map_err(Error::PromptRead)?;
        Ok(false)
    }

    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error> {
        Ok(PreApplyDecision::Continue)
    }
}

fn render_change(out: &mut dyn Write, change: &PendingChange<'_>, color: bool) -> io::Result<()> {
    match change {
        PendingChange::CreateFile { path, content } => {
            writeln!(out, "Create {path}?")?;
            render_new_file(out, content, color)
        }
        PendingChange::UpdateFileHunk {
            path,
            index,
            total,
            diff,
            ops,
        } => {
            writeln!(out, "Update {path} — hunk {index}/{total}?")?;
            render_single_hunk(out, diff, ops, color)
        }
        PendingChange::CreateSymlink { real, symlink } => {
            writeln!(out, "Create symlink {symlink} -> {real}?")
        }
        PendingChange::CreateSymlinkWithParent {
            real,
            symlink,
            parent,
        } => writeln!(
            out,
            "Create symlink {symlink} -> {real}? (will also create directory {parent})"
        ),
        PendingChange::ReplaceWrongSymlink {
            real,
            symlink,
            current_target,
        } => writeln!(
            out,
            "Replace symlink {symlink}: currently -> {} -> {real}?",
            current_target.display(),
        ),
    }
}

fn render_new_file(out: &mut dyn Write, content: &str, color: bool) -> io::Result<()> {
    let open = color_code(color, "\x1b[32m");
    let close = color_reset(color);
    for line in content.lines() {
        writeln!(out, "{open}+{line}{close}")?;
    }
    if !content.ends_with('\n') && !content.is_empty() {
        writeln!(out, "\\ No newline at end of file")?;
    }
    Ok(())
}

fn render_single_hunk(
    out: &mut dyn Write,
    diff: &TextDiff<'_, '_, str>,
    ops: &[DiffOp],
    color: bool,
) -> io::Result<()> {
    let first = ops.first().expect("hunk has at least one op");
    let last = ops.last().expect("hunk has at least one op");
    let old_start = first.old_range().start;
    let old_len = last.old_range().end - old_start;
    let new_start = first.new_range().start;
    let new_len = last.new_range().end - new_start;

    let header_open = color_code(color, "\x1b[36m");
    let header_close = color_reset(color);
    writeln!(
        out,
        "{header_open}@@ -{},{} +{},{} @@{header_close}",
        old_start + 1,
        old_len,
        new_start + 1,
        new_len,
    )?;
    for op in ops {
        for change in diff.iter_changes(op) {
            let (prefix, open, close) = match change.tag() {
                ChangeTag::Delete => ("-", color_code(color, "\x1b[31m"), color_reset(color)),
                ChangeTag::Insert => ("+", color_code(color, "\x1b[32m"), color_reset(color)),
                ChangeTag::Equal => (" ", color_code(color, "\x1b[2m"), color_reset(color)),
            };
            write!(out, "{open}{prefix}{change}{close}")?;
            if change.missing_newline() {
                writeln!(out, "\n\\ No newline at end of file")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use similar::TextDiff;
    use similar_asserts::assert_eq;
    use zenops_safe_relative_path::SafeRelativePath;

    use super::*;
    use crate::{config_files::ConfigFilePath, output::ResolvedConfigFilePath};

    fn home_path(rel: &str) -> ResolvedConfigFilePath {
        let srp = SafeRelativePath::from_relative_path(rel).unwrap();
        ResolvedConfigFilePath {
            path: ConfigFilePath::in_home(srp),
            full: Arc::from(Path::new("/home/test").join(rel)),
        }
    }

    fn render_to_string(change: PendingChange<'_>, color: bool) -> String {
        let mut buf: Vec<u8> = Vec::new();
        render_change(&mut buf, &change, color).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn render_new_file_emits_plus_prefix_and_missing_newline_marker() {
        let mut buf: Vec<u8> = Vec::new();
        render_new_file(&mut buf, "alpha\nbeta", false).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "+alpha\n+beta\n\\ No newline at end of file\n",
        );
    }

    #[test]
    fn render_new_file_skips_marker_when_content_ends_in_newline() {
        let mut buf: Vec<u8> = Vec::new();
        render_new_file(&mut buf, "alpha\nbeta\n", false).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "+alpha\n+beta\n");
    }

    #[test]
    fn render_new_file_empty_content_emits_nothing() {
        let mut buf: Vec<u8> = Vec::new();
        render_new_file(&mut buf, "", false).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn render_change_create_file_writes_prompt_and_body() {
        let p = home_path("alpha.toml");
        assert_eq!(
            render_to_string(
                PendingChange::CreateFile {
                    path: &p,
                    content: "x\n",
                },
                false,
            ),
            "Create ~/alpha.toml?\n+x\n",
        );
    }

    #[test]
    fn render_change_create_symlink_writes_single_line() {
        let real = home_path("src.txt");
        let symlink = home_path("dst.txt");
        assert_eq!(
            render_to_string(
                PendingChange::CreateSymlink {
                    real: &real,
                    symlink: &symlink,
                },
                false,
            ),
            "Create symlink ~/dst.txt -> ~/src.txt?\n",
        );
    }

    #[test]
    fn render_change_create_symlink_with_parent_mentions_parent_dir() {
        let real = home_path("src.txt");
        let symlink = home_path("sub/dst.txt");
        let parent = home_path("sub");
        assert_eq!(
            render_to_string(
                PendingChange::CreateSymlinkWithParent {
                    real: &real,
                    symlink: &symlink,
                    parent: &parent,
                },
                false,
            ),
            "Create symlink ~/sub/dst.txt -> ~/src.txt? (will also create directory ~/sub)\n",
        );
    }

    #[test]
    fn render_single_hunk_emits_unified_diff_header_and_markers() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        let diff = TextDiff::from_lines(old, new);
        let groups = diff.grouped_ops(3);
        assert_eq!(groups.len(), 1);

        let mut buf: Vec<u8> = Vec::new();
        render_single_hunk(&mut buf, &diff, &groups[0], false).unwrap();
        let got = String::from_utf8(buf).unwrap();

        assert!(
            got.starts_with("@@ -1,3 +1,3 @@\n"),
            "header wrong: {got:?}",
        );
        assert!(got.contains(" a\n"), "context before missing: {got:?}");
        assert!(got.contains("-b\n"), "delete line missing: {got:?}");
        assert!(got.contains("+B\n"), "insert line missing: {got:?}");
        assert!(got.contains(" c\n"), "context after missing: {got:?}");
    }

    #[test]
    fn render_single_hunk_uses_ansi_colors_when_enabled() {
        let diff = TextDiff::from_lines("a\n", "b\n");
        let groups = diff.grouped_ops(3);
        let mut buf: Vec<u8> = Vec::new();
        render_single_hunk(&mut buf, &diff, &groups[0], true).unwrap();
        let got = String::from_utf8(buf).unwrap();

        assert!(got.contains("\x1b[36m@@"), "cyan header missing: {got:?}");
        assert!(got.contains("\x1b[31m-a"), "red delete missing: {got:?}");
        assert!(got.contains("\x1b[32m+b"), "green insert missing: {got:?}");
        assert!(got.contains("\x1b[0m"), "reset missing: {got:?}");
    }

    #[test]
    fn render_single_hunk_marks_missing_trailing_newline() {
        let diff = TextDiff::from_lines("a\n", "a");
        let groups = diff.grouped_ops(3);
        let mut buf: Vec<u8> = Vec::new();
        render_single_hunk(&mut buf, &diff, &groups[0], false).unwrap();
        let got = String::from_utf8(buf).unwrap();
        assert!(
            got.contains("\\ No newline at end of file"),
            "missing-newline marker not emitted: {got:?}",
        );
    }

    #[test]
    fn color_helpers_return_empty_when_disabled() {
        assert_eq!(color_code(false, "\x1b[36m"), "");
        assert_eq!(color_reset(false), "");
    }

    #[test]
    fn color_helpers_return_codes_when_enabled() {
        assert_eq!(color_code(true, "\x1b[36m"), "\x1b[36m");
        assert_eq!(color_reset(true), "\x1b[0m");
    }

    #[test]
    fn render_change_replace_wrong_symlink_includes_current_target() {
        let real = home_path("src.txt");
        let symlink = home_path("dst.txt");
        let current = Path::new("/somewhere/else");
        assert_eq!(
            render_to_string(
                PendingChange::ReplaceWrongSymlink {
                    real: &real,
                    symlink: &symlink,
                    current_target: current,
                },
                false,
            ),
            "Replace symlink ~/dst.txt: currently -> /somewhere/else -> ~/src.txt?\n",
        );
    }

    #[test]
    fn render_change_update_file_hunk_writes_header_and_diff() {
        let p = home_path("alpha.toml");
        let diff = TextDiff::from_lines("a\nb\nc\n", "a\nB\nc\n");
        let groups = diff.grouped_ops(3);
        assert_eq!(groups.len(), 1);

        let mut buf: Vec<u8> = Vec::new();
        render_change(
            &mut buf,
            &PendingChange::UpdateFileHunk {
                path: &p,
                index: 1,
                total: 1,
                diff: &diff,
                ops: &groups[0],
            },
            false,
        )
        .unwrap();
        let got = String::from_utf8(buf).unwrap();

        assert!(
            got.starts_with("Update ~/alpha.toml — hunk 1/1?\n"),
            "header wrong: {got:?}",
        );
        assert!(got.contains("@@ -1,3 +1,3 @@"));
        assert!(got.contains("-b\n"));
        assert!(got.contains("+B\n"));
    }

    #[test]
    fn parse_pre_apply_input_recognizes_commit() {
        assert_eq!(parse_pre_apply_input("c"), Some(PreApplyAnswer::Commit));
        assert_eq!(
            parse_pre_apply_input("commit"),
            Some(PreApplyAnswer::Commit)
        );
        assert_eq!(
            parse_pre_apply_input("  COMMIT \n"),
            Some(PreApplyAnswer::Commit),
        );
    }

    #[test]
    fn parse_pre_apply_input_recognizes_continue() {
        assert_eq!(parse_pre_apply_input(""), Some(PreApplyAnswer::Continue));
        assert_eq!(parse_pre_apply_input("y"), Some(PreApplyAnswer::Continue));
        assert_eq!(parse_pre_apply_input("yes"), Some(PreApplyAnswer::Continue));
        assert_eq!(
            parse_pre_apply_input(" YES "),
            Some(PreApplyAnswer::Continue),
        );
    }

    #[test]
    fn parse_pre_apply_input_recognizes_abort() {
        assert_eq!(parse_pre_apply_input("n"), Some(PreApplyAnswer::Abort));
        assert_eq!(parse_pre_apply_input("no"), Some(PreApplyAnswer::Abort));
        assert_eq!(parse_pre_apply_input("abort"), Some(PreApplyAnswer::Abort),);
    }

    #[test]
    fn parse_pre_apply_input_returns_none_for_garbage() {
        assert_eq!(parse_pre_apply_input("maybe"), None);
        assert_eq!(parse_pre_apply_input("?"), None);
    }

    struct ScriptedPrompter {
        scripted: std::collections::VecDeque<LineOutcome>,
        warnings: Vec<String>,
    }

    impl ScriptedPrompter {
        fn new(outcomes: Vec<LineOutcome>) -> Self {
            Self {
                scripted: outcomes.into(),
                warnings: Vec::new(),
            }
        }
    }

    impl LinePrompter for ScriptedPrompter {
        fn read_line(&mut self, _prompt: &str) -> io::Result<LineOutcome> {
            Ok(self
                .scripted
                .pop_front()
                .expect("ScriptedPrompter ran out of outcomes"))
        }

        fn writeln(&mut self, msg: &str) -> io::Result<()> {
            self.warnings.push(msg.to_string());
            Ok(())
        }
    }

    #[test]
    fn read_commit_message_returns_trimmed_line_on_happy_path() {
        let mut p = ScriptedPrompter::new(vec![LineOutcome::Line("  hello world  ".into())]);
        let got = read_commit_message(&mut p).unwrap();
        assert_eq!(got, "hello world");
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn read_commit_message_retries_on_empty_then_accepts_real_message() {
        let mut p = ScriptedPrompter::new(vec![
            LineOutcome::Line("".into()),
            LineOutcome::Line("   ".into()),
            LineOutcome::Line("real message".into()),
        ]);
        let got = read_commit_message(&mut p).unwrap();
        assert_eq!(got, "real message");
        assert_eq!(p.warnings.len(), 2);
        assert!(p.warnings[0].contains("cannot be empty"));
    }

    #[test]
    fn read_commit_message_eof_returns_unexpected_eof_error() {
        let mut p = ScriptedPrompter::new(vec![LineOutcome::Eof]);
        let err = read_commit_message(&mut p).unwrap_err();
        match err {
            Error::PromptRead(io_err) => {
                assert_eq!(io_err.kind(), io::ErrorKind::UnexpectedEof);
            }
            other => panic!("expected PromptRead, got {other:?}"),
        }
    }

    #[test]
    fn read_commit_message_interrupted_returns_prompt_interrupted() {
        let mut p = ScriptedPrompter::new(vec![LineOutcome::Interrupted]);
        let err = read_commit_message(&mut p).unwrap_err();
        assert_eq!(err, Error::PromptInterrupted);
    }

    #[test]
    fn yes_prompter_accepts_every_change_and_continues() {
        let mut p = YesPrompter;
        let real = home_path("src.txt");
        let symlink = home_path("dst.txt");
        assert!(
            p.confirm(PendingChange::CreateSymlink {
                real: &real,
                symlink: &symlink,
            })
            .unwrap()
        );
        assert_eq!(p.confirm_pre_apply().unwrap(), PreApplyDecision::Continue);
    }
}
