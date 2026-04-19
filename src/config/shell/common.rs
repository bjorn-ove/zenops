use indexmap::IndexMap;
use smol_str::SmolStr;
use std::fmt::Write as _;

use crate::config::pkg::ShellInitAction;

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub(in super::super) struct StoredShellConfig {
    pub(super) environment: IndexMap<SmolStr, SmolStr>,
    pub(super) alias: IndexMap<SmolStr, SmolStr>,
}

pub(super) fn render_posix(action: &ShellInitAction) -> String {
    let (comment, line) = match action {
        ShellInitAction::Source { path, comment } => {
            let posix = if let Some(rest) = path.strip_prefix("~/") {
                format!("$HOME/{rest}")
            } else {
                path.clone()
            };
            (comment.as_deref(), format!(r#". "{posix}""#))
        }
        ShellInitAction::EvalOutput { command, comment } => {
            (comment.as_deref(), format!(r#"eval "$({})""#, command.join(" ")))
        }
    };
    match comment {
        Some(c) => format!("# {c}\n{line}"),
        None => line,
    }
}

pub(super) fn write_pkg_inits(buf: &mut String, actions: &[&ShellInitAction]) {
    for action in actions {
        _ = writeln!(buf, "{}", render_posix(action));
        buf.push('\n');
    }
}

pub(super) fn write_environment(buf: &mut String, environment: &IndexMap<SmolStr, SmolStr>) {
    if !environment.is_empty() {
        for (name, value) in environment {
            _ = writeln!(buf, "export {name}={value}");
        }
        buf.push('\n');
    }
}

pub(super) fn write_aliases(buf: &mut String, alias: &IndexMap<SmolStr, SmolStr>) {
    if !alias.is_empty() {
        for (name, value) in alias {
            _ = writeln!(buf, "alias {name}={value}");
        }
        buf.push('\n');
    }
}

pub(super) fn write_path_variable(buf: &mut String, path: &str) {
    _ = writeln!(buf, "export PATH={path}");
    buf.push('\n');
}

#[cfg(target_os = "macos")]
pub(super) fn write_brew_llvm_flags(buf: &mut String) {
    _ = writeln!(buf, "# LLVM compiler flags (brew-managed)");
    _ = writeln!(buf, "export LDFLAGS=-L/opt/homebrew/opt/llvm/lib");
    _ = writeln!(buf, "export CPPFLAGS=-L/opt/homebrew/opt/llvm/include");
    buf.push('\n');
}

pub(super) fn write_sk_setup(buf: &mut String, shell_name: &str) {
    _ = writeln!(buf, "# Setup sk (fuzzy find)");
    _ = writeln!(
        buf,
        "export SKIM_DEFAULT_COMMAND=\"fd --type f --hidden --exclude .git\""
    );
    _ = writeln!(buf, "source <(sk --shell {shell_name})");
    _ = writeln!(buf, "source <(sk --shell {shell_name} --shell-bindings)");
    buf.push('\n');
}
