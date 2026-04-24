use indexmap::IndexMap;
use smol_str::SmolStr;
use std::fmt::Write as _;

use zenops_expand::{ExpandError, ExpandLookup};

use crate::{
    config::pkg::{ActionKind, PkgConfig, ShellInitAction},
    error::Error,
};

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
pub(in super::super) struct StoredShellConfig {
    pub(super) environment: IndexMap<SmolStr, SmolStr>,
    pub(super) alias: IndexMap<SmolStr, SmolStr>,
}

/// Writes a rendered action into `buf`. Returns `true` on emit, `false` when
/// the action is optional and a placeholder was unresolved (skip quietly).
fn write_action(
    buf: &mut String,
    action: &ShellInitAction,
    lookup: &impl ExpandLookup,
) -> Result<bool, ExpandError> {
    let restore = buf.len();
    match write_action_body(buf, &action.kind, lookup) {
        Ok(()) => Ok(true),
        Err(ExpandError::Unresolved(_)) if action.optional => {
            buf.truncate(restore);
            Ok(false)
        }
        Err(e) => {
            buf.truncate(restore);
            Err(e)
        }
    }
}

fn write_action_body(
    buf: &mut String,
    kind: &ActionKind,
    lookup: &impl ExpandLookup,
) -> Result<(), ExpandError> {
    match kind {
        ActionKind::Comment { text } => {
            buf.push_str("# ");
            text.write_expanded(lookup, buf)?;
        }
        ActionKind::Source { path } => {
            // POSIX `~/…` → `$HOME/…` translation needs the expanded path, so
            // materialize it once into a scratch String.
            let expanded = path.expand_to_string(lookup)?;
            write!(buf, r#". "{}""#, home_tilde_to_var(&expanded))?;
        }
        ActionKind::EvalOutput { command } => {
            buf.push_str(r#"eval "$("#);
            write_command(buf, command, lookup)?;
            buf.push_str(r#")""#);
        }
        ActionKind::SourceOutput { command } => {
            buf.push_str("source <(");
            write_command(buf, command, lookup)?;
            buf.push(')');
        }
        ActionKind::Export { name, value } => {
            buf.push_str("export ");
            name.write_expanded(lookup, buf)?;
            buf.push_str(r#"=""#);
            value.write_expanded(lookup, buf)?;
            buf.push('"');
        }
        ActionKind::Line { line } => {
            line.write_expanded(lookup, buf)?;
        }
        ActionKind::PathPrepend { value } => {
            let expanded = value.expand_to_string(lookup)?;
            write!(
                buf,
                r#"export PATH="{}:$PATH""#,
                home_tilde_to_var(&expanded)
            )?;
        }
        ActionKind::PathAppend { value } => {
            let expanded = value.expand_to_string(lookup)?;
            write!(
                buf,
                r#"export PATH="$PATH:{}""#,
                home_tilde_to_var(&expanded)
            )?;
        }
    }
    Ok(())
}

fn home_tilde_to_var(s: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = s.strip_prefix("~/") {
        std::borrow::Cow::Owned(format!("$HOME/{rest}"))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

fn write_command(
    buf: &mut String,
    command: &[zenops_expand::ExpandStr],
    lookup: &impl ExpandLookup,
) -> Result<(), ExpandError> {
    for (i, part) in command.iter().enumerate() {
        if i > 0 {
            buf.push(' ');
        }
        part.write_expanded(lookup, buf)?;
    }
    Ok(())
}

pub(super) fn write_pkg_inits(
    buf: &mut String,
    actions: &[(&SmolStr, &PkgConfig, &ShellInitAction)],
    system_inputs: &IndexMap<SmolStr, SmolStr>,
) -> Result<(), Error> {
    // Pass 1: render each action into its own scratch String (or None if an
    // optional action was skipped due to an unresolved placeholder).
    let mut rendered: Vec<Option<(&ShellInitAction, String)>> = Vec::with_capacity(actions.len());
    for (pkg_name, pkg, action) in actions {
        let lookup = [pkg.inputs(), system_inputs];
        let mut line = String::new();
        let wrote =
            write_action(&mut line, action, &lookup).map_err(|e| map_expand_err(e, pkg_name))?;
        rendered.push(if wrote { Some((*action, line)) } else { None });
    }

    // Pass 2: walk the emitted (non-None) subset and apply the legacy spacing
    // rule — after a non-comment, insert a blank line when followed by a
    // comment (group boundary) or at the end.
    let emitted: Vec<(&ShellInitAction, &String)> = rendered
        .iter()
        .filter_map(|entry| entry.as_ref().map(|(a, s)| (*a, s)))
        .collect();
    for (i, (action, line)) in emitted.iter().enumerate() {
        buf.push_str(line);
        buf.push('\n');
        let is_comment = matches!(action.kind, ActionKind::Comment { .. });
        let next_is_comment = emitted
            .get(i + 1)
            .is_some_and(|(next, _)| matches!(next.kind, ActionKind::Comment { .. }));
        let is_last = i + 1 == emitted.len();
        if !is_comment && (is_last || next_is_comment) {
            buf.push('\n');
        }
    }

    Ok(())
}

fn map_expand_err(e: ExpandError, pkg_name: &SmolStr) -> Error {
    match e {
        ExpandError::Unresolved(input) => Error::UnresolvedInput {
            pkg: pkg_name.clone(),
            input,
        },
        ExpandError::Unterminated => Error::TemplateUnterminated {
            pkg: pkg_name.clone(),
        },
        ExpandError::WriteFmt(_) => {
            // Writing into a `String` never fails; this branch is unreachable.
            Error::TemplateUnterminated {
                pkg: pkg_name.clone(),
            }
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
