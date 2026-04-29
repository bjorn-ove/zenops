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

#[cfg(test)]
mod tests {
    use super::*;
    use zenops_expand::ExpandStr;

    fn lookup() -> IndexMap<SmolStr, SmolStr> {
        let mut m = IndexMap::new();
        m.insert(SmolStr::new_static("name"), SmolStr::new_static("octocat"));
        m.insert(
            SmolStr::new_static("path"),
            SmolStr::new_static("~/.cargo/bin"),
        );
        m
    }

    fn render(kind: ActionKind, optional: bool) -> Result<(bool, String), ExpandError> {
        let action = ShellInitAction { optional, kind };
        let mut buf = String::new();
        let wrote = write_action(&mut buf, &action, &lookup())?;
        Ok((wrote, buf))
    }

    #[test]
    fn write_action_comment_emits_hash_prefix() {
        let (wrote, body) = render(
            ActionKind::Comment {
                text: ExpandStr::new_static("hi ${name}"),
            },
            false,
        )
        .unwrap();
        assert!(wrote);
        assert_eq!(body, "# hi octocat");
    }

    #[test]
    fn write_action_source_translates_tilde_to_home_var() {
        let (_, body) = render(
            ActionKind::Source {
                path: ExpandStr::new_static("${path}/env"),
            },
            false,
        )
        .unwrap();
        assert_eq!(body, r#". "$HOME/.cargo/bin/env""#);
    }

    #[test]
    fn write_action_eval_output_joins_command_parts() {
        let (_, body) = render(
            ActionKind::EvalOutput {
                command: vec![
                    ExpandStr::new_static("starship"),
                    ExpandStr::new_static("init"),
                    ExpandStr::new_static("${name}"),
                ],
            },
            false,
        )
        .unwrap();
        assert_eq!(body, r#"eval "$(starship init octocat)""#);
    }

    #[test]
    fn write_action_source_output_uses_process_substitution() {
        let (_, body) = render(
            ActionKind::SourceOutput {
                command: vec![
                    ExpandStr::new_static("zoxide"),
                    ExpandStr::new_static("init"),
                    ExpandStr::new_static("zsh"),
                ],
            },
            false,
        )
        .unwrap();
        assert_eq!(body, "source <(zoxide init zsh)");
    }

    #[test]
    fn write_action_export_quotes_value() {
        let (_, body) = render(
            ActionKind::Export {
                name: ExpandStr::new_static("EDITOR"),
                value: ExpandStr::new_static("vim"),
            },
            false,
        )
        .unwrap();
        assert_eq!(body, r#"export EDITOR="vim""#);
    }

    #[test]
    fn write_action_line_writes_raw_template() {
        let (_, body) = render(
            ActionKind::Line {
                line: ExpandStr::new_static("setopt no_beep"),
            },
            false,
        )
        .unwrap();
        assert_eq!(body, "setopt no_beep");
    }

    #[test]
    fn write_action_path_prepend_renders_export_with_home_var() {
        let (_, body) = render(
            ActionKind::PathPrepend {
                value: ExpandStr::new_static("${path}"),
            },
            false,
        )
        .unwrap();
        assert_eq!(body, r#"export PATH="$HOME/.cargo/bin:$PATH""#);
    }

    #[test]
    fn write_action_path_append_renders_export_with_home_var() {
        let (_, body) = render(
            ActionKind::PathAppend {
                value: ExpandStr::new_static("${path}"),
            },
            false,
        )
        .unwrap();
        assert_eq!(body, r#"export PATH="$PATH:$HOME/.cargo/bin""#);
    }

    #[test]
    fn write_action_path_prepend_passes_absolute_through_unchanged() {
        let (_, body) = render(
            ActionKind::PathPrepend {
                value: ExpandStr::new_static("/usr/local/bin"),
            },
            false,
        )
        .unwrap();
        assert_eq!(body, r#"export PATH="/usr/local/bin:$PATH""#);
    }

    #[test]
    fn write_action_optional_unresolved_skips_silently() {
        let action = ShellInitAction {
            optional: true,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("missing ${nope}"),
            },
        };
        let mut buf = String::from("PRESERVE");
        let wrote = write_action(&mut buf, &action, &lookup()).unwrap();
        assert!(!wrote);
        assert_eq!(
            buf, "PRESERVE",
            "buffer must be truncated back to original length on optional skip",
        );
    }

    #[test]
    fn write_action_required_unresolved_truncates_and_propagates() {
        let action = ShellInitAction {
            optional: false,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("missing ${nope}"),
            },
        };
        let mut buf = String::from("PRESERVE");
        let err = write_action(&mut buf, &action, &lookup()).unwrap_err();
        assert!(matches!(err, ExpandError::Unresolved(_)));
        assert_eq!(buf, "PRESERVE");
    }

    #[test]
    fn write_pkg_inits_inserts_blank_line_between_groups() {
        let inputs: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        let pkg = PkgConfig::default();
        let header = ShellInitAction {
            optional: false,
            kind: ActionKind::Comment {
                text: ExpandStr::new_static("group A"),
            },
        };
        let line_a = ShellInitAction {
            optional: false,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("alpha"),
            },
        };
        let header_b = ShellInitAction {
            optional: false,
            kind: ActionKind::Comment {
                text: ExpandStr::new_static("group B"),
            },
        };
        let line_b = ShellInitAction {
            optional: false,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("beta"),
            },
        };
        let pkg_name = SmolStr::new_static("pkg");
        let actions = vec![
            (&pkg_name, &pkg, &header),
            (&pkg_name, &pkg, &line_a),
            (&pkg_name, &pkg, &header_b),
            (&pkg_name, &pkg, &line_b),
        ];

        let mut buf = String::new();
        write_pkg_inits(&mut buf, &actions, &inputs).unwrap();
        // Blank line goes after the non-comment that precedes a comment, and
        // a trailing blank line after the very last non-comment.
        assert_eq!(buf, "# group A\nalpha\n\n# group B\nbeta\n\n");
    }

    #[test]
    fn write_pkg_inits_skips_optional_unresolved_actions_without_emitting_blank() {
        let inputs: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        let pkg = PkgConfig::default();
        let line = ShellInitAction {
            optional: false,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("alpha"),
            },
        };
        let skipped = ShellInitAction {
            optional: true,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("missing ${nope}"),
            },
        };
        let pkg_name = SmolStr::new_static("pkg");
        let actions = vec![(&pkg_name, &pkg, &line), (&pkg_name, &pkg, &skipped)];

        let mut buf = String::new();
        write_pkg_inits(&mut buf, &actions, &inputs).unwrap();
        assert_eq!(buf, "alpha\n\n");
    }

    #[test]
    fn write_pkg_inits_required_unresolved_returns_unresolved_input() {
        let inputs: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        let pkg = PkgConfig::default();
        let line = ShellInitAction {
            optional: false,
            kind: ActionKind::Line {
                line: ExpandStr::new_static("missing ${nope}"),
            },
        };
        let pkg_name = SmolStr::new_static("mypkg");
        let actions = vec![(&pkg_name, &pkg, &line)];

        let mut buf = String::new();
        let err = write_pkg_inits(&mut buf, &actions, &inputs).unwrap_err();
        match err {
            Error::UnresolvedInput { pkg, input } => {
                assert_eq!(pkg, "mypkg");
                assert_eq!(input, "nope");
            }
            other => panic!("expected UnresolvedInput, got {other:?}"),
        }
    }

    #[test]
    fn write_environment_emits_blank_line_after_non_empty_block() {
        let mut env: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        env.insert(SmolStr::new_static("FOO"), SmolStr::new_static("bar"));
        let mut buf = String::new();
        write_environment(&mut buf, &env);
        assert_eq!(buf, "export FOO=bar\n\n");
    }

    #[test]
    fn write_environment_empty_writes_nothing() {
        let env: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        let mut buf = String::from("KEEP");
        write_environment(&mut buf, &env);
        assert_eq!(buf, "KEEP");
    }

    #[test]
    fn write_aliases_emits_blank_line_after_non_empty_block() {
        let mut alias: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        alias.insert(SmolStr::new_static("ll"), SmolStr::new_static("\"ls -l\""));
        let mut buf = String::new();
        write_aliases(&mut buf, &alias);
        assert_eq!(buf, "alias ll=\"ls -l\"\n\n");
    }

    #[test]
    fn write_aliases_empty_writes_nothing() {
        let alias: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        let mut buf = String::from("KEEP");
        write_aliases(&mut buf, &alias);
        assert_eq!(buf, "KEEP");
    }

    #[test]
    fn home_tilde_to_var_replaces_tilde_prefix() {
        assert_eq!(home_tilde_to_var("~/foo"), "$HOME/foo");
    }

    #[test]
    fn home_tilde_to_var_passes_other_paths_through() {
        assert_eq!(home_tilde_to_var("/etc/foo"), "/etc/foo");
        assert_eq!(home_tilde_to_var("relative/foo"), "relative/foo");
        assert_eq!(home_tilde_to_var(""), "");
    }
}
