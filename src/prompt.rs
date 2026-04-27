use std::io::{self, BufRead, Write};

use similar::{ChangeTag, DiffOp, TextDiff};

use crate::{
    ansi::{color_code, color_reset},
    error::Error,
    output::ResolvedConfigFilePath,
};

pub enum PendingChange<'a> {
    CreateFile {
        path: &'a ResolvedConfigFilePath,
        content: &'a str,
    },
    UpdateFileHunk {
        path: &'a ResolvedConfigFilePath,
        index: usize,
        total: usize,
        diff: &'a TextDiff<'a, 'a, str>,
        ops: &'a [DiffOp],
    },
    CreateSymlink {
        real: &'a ResolvedConfigFilePath,
        symlink: &'a ResolvedConfigFilePath,
    },
    CreateSymlinkWithParent {
        real: &'a ResolvedConfigFilePath,
        symlink: &'a ResolvedConfigFilePath,
        parent: &'a ResolvedConfigFilePath,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum PreApplyDecision {
    CommitAndPush { message: String },
    Continue,
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

#[derive(Debug, PartialEq, Eq)]
pub enum PreApplyAnswer {
    Commit,
    Continue,
    Abort,
}

pub trait Prompter {
    fn confirm(&mut self, change: PendingChange<'_>) -> Result<bool, Error>;
    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error>;
}

pub struct YesPrompter;

impl Prompter for YesPrompter {
    fn confirm(&mut self, _change: PendingChange<'_>) -> Result<bool, Error> {
        Ok(true)
    }
    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error> {
        Ok(PreApplyDecision::Continue)
    }
}

pub struct TerminalPrompter {
    color: bool,
}

impl TerminalPrompter {
    pub fn new(color: bool) -> Self {
        Self { color }
    }
}

impl Prompter for TerminalPrompter {
    fn confirm(&mut self, change: PendingChange<'_>) -> Result<bool, Error> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        render_change(&mut out, &change, self.color).map_err(Error::PromptRead)?;
        loop {
            write!(out, "[Y/n] ").map_err(Error::PromptRead)?;
            out.flush().map_err(Error::PromptRead)?;

            let mut line = String::new();
            let n = io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(Error::PromptRead)?;
            if n == 0 {
                writeln!(out).map_err(Error::PromptRead)?;
                return Ok(false);
            }
            match line.trim().to_ascii_lowercase().as_str() {
                "" | "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => {
                    writeln!(out, "Please answer y or n.").map_err(Error::PromptRead)?;
                }
            }
        }
    }

    fn confirm_pre_apply(&mut self) -> Result<PreApplyDecision, Error> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        loop {
            write!(out, "[c]ommit & push / [Y]continue / [n]abort: ").map_err(Error::PromptRead)?;
            out.flush().map_err(Error::PromptRead)?;

            let mut line = String::new();
            let n = io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(Error::PromptRead)?;
            if n == 0 {
                writeln!(out).map_err(Error::PromptRead)?;
                return Ok(PreApplyDecision::Abort);
            }
            match parse_pre_apply_input(&line) {
                Some(PreApplyAnswer::Commit) => {
                    let message = read_commit_message(&mut out)?;
                    return Ok(PreApplyDecision::CommitAndPush { message });
                }
                Some(PreApplyAnswer::Continue) => return Ok(PreApplyDecision::Continue),
                Some(PreApplyAnswer::Abort) => return Ok(PreApplyDecision::Abort),
                None => {
                    writeln!(out, "Please answer c, y, or n.").map_err(Error::PromptRead)?;
                }
            }
        }
    }
}

fn read_commit_message(out: &mut dyn Write) -> Result<String, Error> {
    loop {
        write!(out, "Commit message: ").map_err(Error::PromptRead)?;
        out.flush().map_err(Error::PromptRead)?;
        let mut line = String::new();
        let n = io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(Error::PromptRead)?;
        if n == 0 {
            return Err(Error::PromptRead(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "no commit message provided",
            )));
        }
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            writeln!(out, "Commit message cannot be empty.").map_err(Error::PromptRead)?;
            continue;
        }
        return Ok(trimmed);
    }
}

pub struct DryRunPrompter {
    color: bool,
}

impl DryRunPrompter {
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
}
