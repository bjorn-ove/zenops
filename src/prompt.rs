use std::io::{self, BufRead, Write};

use similar::{ChangeTag, DiffOp, TextDiff};

use crate::{error::Error, output::ResolvedConfigFilePath};

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
        let stderr = io::stderr();
        let mut out = stderr.lock();
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
        let stderr = io::stderr();
        let mut out = stderr.lock();
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
        let stderr = io::stderr();
        let mut out = stderr.lock();
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

fn color_code(color: bool, code: &'static str) -> &'static str {
    if color { code } else { "" }
}

fn color_reset(color: bool) -> &'static str {
    if color { "\x1b[0m" } else { "" }
}
