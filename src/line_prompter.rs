//! Single-line prompt abstraction for interactive commands.
//!
//! [`LinePrompter`] decouples "show a prompt and read one line" from where
//! that line comes from. Production code uses [`RustylinePrompter`] so the
//! user gets Home/End, arrow keys, and proper UTF-8 backspace; tests use
//! [`BufReadPrompter`] to drive helpers from byte slices without a TTY.
//!
//! End-of-input is reported as one of three outcomes via [`LineOutcome`]:
//! [`LineOutcome::Line`] for input, [`LineOutcome::Eof`] for EOF / Ctrl-D
//! (caller may fall back to a default), and [`LineOutcome::Interrupted`]
//! for Ctrl-C (caller must abort).

use std::io::{self, BufRead, Write};

use rustyline::{DefaultEditor, error::ReadlineError};

/// What [`LinePrompter::read_line`] returned. Lets callers distinguish
/// "user pressed Enter on a closed/empty stdin" (fall back to default)
/// from "user pressed Ctrl-C" (abort).
pub enum LineOutcome {
    /// User typed a line. String has no trailing newline.
    Line(String),
    /// EOF: closed stdin in tests, Ctrl-D in rustyline. Callers may
    /// substitute a default value.
    Eof,
    /// User pressed Ctrl-C. Callers must abort whatever flow is running.
    Interrupted,
}

/// Show a prompt, read one line, optionally print a status line.
pub trait LinePrompter {
    /// Display `prompt` and read one line. The returned string is stripped
    /// of any trailing `\n` / `\r\n`. See [`LineOutcome`] for the three
    /// non-error variants.
    fn read_line(&mut self, prompt: &str) -> io::Result<LineOutcome>;
    /// Emit a status / re-prompt line such as "Please answer y or n.".
    fn writeln(&mut self, msg: &str) -> io::Result<()>;
}

/// Production impl: line editing via rustyline. Requires a TTY; callers
/// must check [`std::io::IsTerminal`] before constructing one.
pub struct RustylinePrompter {
    editor: DefaultEditor,
}

impl RustylinePrompter {
    /// Build a new rustyline-backed prompter. Fails if rustyline can't
    /// open the controlling terminal.
    pub fn new() -> io::Result<Self> {
        let editor = DefaultEditor::new().map_err(readline_to_io)?;
        Ok(Self { editor })
    }
}

impl LinePrompter for RustylinePrompter {
    fn read_line(&mut self, prompt: &str) -> io::Result<LineOutcome> {
        match self.editor.readline(prompt) {
            Ok(line) => Ok(LineOutcome::Line(line)),
            Err(ReadlineError::Eof) => Ok(LineOutcome::Eof),
            Err(ReadlineError::Interrupted) => Ok(LineOutcome::Interrupted),
            Err(ReadlineError::Io(e)) => Err(e),
            Err(e) => Err(io::Error::other(e)),
        }
    }

    fn writeln(&mut self, msg: &str) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        writeln!(out, "{msg}")
    }
}

fn readline_to_io(err: ReadlineError) -> io::Error {
    match err {
        ReadlineError::Io(e) => e,
        other => io::Error::other(other),
    }
}

/// Test / non-TTY impl: prompt is written to `writer`, line is read from
/// `reader`. The writer captures both prompts and `writeln` output so
/// tests can assert on what the user would have seen.
pub struct BufReadPrompter<R: BufRead, W: Write> {
    reader: R,
    writer: W,
}

impl<R: BufRead, W: Write> BufReadPrompter<R, W> {
    /// Wrap a reader/writer pair.
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R: BufRead, W: Write> LinePrompter for BufReadPrompter<R, W> {
    fn read_line(&mut self, prompt: &str) -> io::Result<LineOutcome> {
        write!(self.writer, "{prompt}")?;
        self.writer.flush()?;
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(LineOutcome::Eof);
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(LineOutcome::Line(line))
    }

    fn writeln(&mut self, msg: &str) -> io::Result<()> {
        writeln!(self.writer, "{msg}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(outcome: LineOutcome) -> Option<String> {
        match outcome {
            LineOutcome::Line(s) => Some(s),
            _ => None,
        }
    }

    #[test]
    fn bufread_strips_trailing_newline() {
        let input = b"hello\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::new());
        let outcome = prompter.read_line("> ").unwrap();
        assert_eq!(line(outcome), Some("hello".to_string()));
        assert_eq!(prompter.writer, b"> ");
    }

    #[test]
    fn bufread_strips_trailing_crlf() {
        let input = b"hello\r\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::new());
        let outcome = prompter.read_line("> ").unwrap();
        assert_eq!(line(outcome), Some("hello".to_string()));
    }

    #[test]
    fn bufread_eof_returns_eof() {
        let input = b"";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::new());
        let outcome = prompter.read_line("> ").unwrap();
        assert!(matches!(outcome, LineOutcome::Eof));
    }

    #[test]
    fn bufread_blank_line_returns_empty_string() {
        let input = b"\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::new());
        let outcome = prompter.read_line("> ").unwrap();
        assert_eq!(line(outcome), Some(String::new()));
    }

    #[test]
    fn bufread_writeln_writes_to_writer() {
        let input = b"";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::new());
        prompter.writeln("nope").unwrap();
        assert_eq!(prompter.writer, b"nope\n");
    }
}
