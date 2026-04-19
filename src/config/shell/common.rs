use indexmap::IndexMap;
use smol_str::SmolStr;
use std::fmt::Write as _;

use crate::config::pkg::{ActionKind, ShellInitAction};

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub(in super::super) struct StoredShellConfig {
    pub(super) environment: IndexMap<SmolStr, SmolStr>,
    pub(super) alias: IndexMap<SmolStr, SmolStr>,
}

pub(super) fn render_posix(action: &ShellInitAction) -> String {
    match &action.kind {
        ActionKind::Comment { text } => format!("# {text}"),
        ActionKind::Source { path } => {
            let posix = if let Some(rest) = path.strip_prefix("~/") {
                format!("$HOME/{rest}")
            } else {
                path.clone()
            };
            format!(r#". "{posix}""#)
        }
        ActionKind::EvalOutput { command } => format!(r#"eval "$({})""#, command.join(" ")),
        ActionKind::SourceOutput { command } => format!("source <({})", command.join(" ")),
        ActionKind::Export { name, value } => format!(r#"export {name}="{value}""#),
    }
}

pub(super) fn write_pkg_inits(buf: &mut String, actions: &[&ShellInitAction]) {
    for (i, action) in actions.iter().enumerate() {
        _ = writeln!(buf, "{}", render_posix(action));
        let is_comment = matches!(action.kind, ActionKind::Comment { .. });
        let next_is_comment = matches!(
            actions.get(i + 1),
            Some(next) if matches!(next.kind, ActionKind::Comment { .. })
        );
        let is_last = actions.get(i + 1).is_none();
        if !is_comment && (is_last || next_is_comment) {
            buf.push('\n');
        }
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
