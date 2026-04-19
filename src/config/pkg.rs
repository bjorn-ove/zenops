use std::path::Path;

use indexmap::IndexMap;
use smol_str::SmolStr;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    Bash,
    Zsh,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum PkgEnable {
    #[default]
    Detect,
    On,
    Disabled,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DetectStrategy {
    File { path: String },
    Which { binary: String },
}

impl DetectStrategy {
    pub fn check(&self, home: &Path) -> bool {
        match self {
            Self::File { path } => {
                let expanded = path.replacen('~', &home.to_string_lossy(), 1);
                Path::new(&expanded).exists()
            }
            Self::Which { binary } => std::env::var("PATH")
                .unwrap_or_default()
                .split(':')
                .any(|dir| Path::new(dir).join(binary).is_file()),
        }
    }
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub struct ShellInitAction {
    #[serde(default)]
    pub optional: bool,
    #[serde(flatten)]
    pub kind: ActionKind,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionKind {
    Comment { text: String },
    Source { path: String },
    EvalOutput { command: Vec<String> },
    SourceOutput { command: Vec<String> },
    Export { name: String, value: String },
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub(super) struct PerShellActions {
    pub bash: Vec<ShellInitAction>,
    pub zsh: Vec<ShellInitAction>,
}

impl PerShellActions {
    pub fn for_shell(&self, shell: Shell) -> &[ShellInitAction] {
        match shell {
            Shell::Bash => &self.bash,
            Shell::Zsh => &self.zsh,
        }
    }
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub(super) struct StoredPkgShellConfig {
    pub env_init: PerShellActions,
    pub interactive_init: PerShellActions,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub(super) struct StoredPkgConfig {
    pub enable: PkgEnable,
    pub detect: Vec<DetectStrategy>,
    pub inputs: IndexMap<SmolStr, SmolStr>,
    pub shell: StoredPkgShellConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PkgConfig {
    pub enable: PkgEnable,
    pub detect: Vec<DetectStrategy>,
    pub env_init: PerShellActions,
    pub interactive_init: PerShellActions,
}

impl StoredPkgConfig {
    pub fn resolve(self, pkg_name: &SmolStr) -> Result<Option<PkgConfig>, Error> {
        match self.enable {
            PkgEnable::Disabled => return Ok(None),
            PkgEnable::Detect if self.detect.is_empty() => return Ok(None),
            _ => {}
        }

        let env_init = resolve_per_shell(self.shell.env_init, &self.inputs, pkg_name)?;
        let interactive_init = resolve_per_shell(self.shell.interactive_init, &self.inputs, pkg_name)?;

        Ok(Some(PkgConfig {
            enable: self.enable,
            detect: self.detect,
            env_init,
            interactive_init,
        }))
    }
}

impl PkgConfig {
    pub fn is_installed(&self, home: &Path) -> bool {
        match self.enable {
            PkgEnable::On => true,
            PkgEnable::Detect => self.detect.iter().any(|s| s.check(home)),
            PkgEnable::Disabled => false,
        }
    }
}

fn resolve_per_shell(
    actions: PerShellActions,
    inputs: &IndexMap<SmolStr, SmolStr>,
    pkg_name: &SmolStr,
) -> Result<PerShellActions, Error> {
    Ok(PerShellActions {
        bash: resolve_actions(actions.bash, inputs, pkg_name)?,
        zsh: resolve_actions(actions.zsh, inputs, pkg_name)?,
    })
}

fn resolve_actions(
    actions: Vec<ShellInitAction>,
    inputs: &IndexMap<SmolStr, SmolStr>,
    pkg_name: &SmolStr,
) -> Result<Vec<ShellInitAction>, Error> {
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        if let Some(a) = resolve_action(action, inputs, pkg_name)? {
            out.push(a);
        }
    }
    Ok(out)
}

fn resolve_action(
    action: ShellInitAction,
    inputs: &IndexMap<SmolStr, SmolStr>,
    pkg_name: &SmolStr,
) -> Result<Option<ShellInitAction>, Error> {
    let ShellInitAction { optional, kind } = action;
    match resolve_kind(kind, inputs) {
        Ok(kind) => Ok(Some(ShellInitAction { optional, kind })),
        Err(input) => {
            if optional {
                Ok(None)
            } else {
                Err(Error::UnresolvedInput {
                    pkg: pkg_name.clone(),
                    input,
                })
            }
        }
    }
}

fn resolve_kind(
    kind: ActionKind,
    inputs: &IndexMap<SmolStr, SmolStr>,
) -> Result<ActionKind, SmolStr> {
    Ok(match kind {
        ActionKind::Comment { text } => ActionKind::Comment {
            text: substitute(&text, inputs)?,
        },
        ActionKind::Source { path } => ActionKind::Source {
            path: substitute(&path, inputs)?,
        },
        ActionKind::EvalOutput { command } => ActionKind::EvalOutput {
            command: substitute_vec(command, inputs)?,
        },
        ActionKind::SourceOutput { command } => ActionKind::SourceOutput {
            command: substitute_vec(command, inputs)?,
        },
        ActionKind::Export { name, value } => ActionKind::Export {
            name: substitute(&name, inputs)?,
            value: substitute(&value, inputs)?,
        },
    })
}

fn substitute_vec(
    command: Vec<String>,
    inputs: &IndexMap<SmolStr, SmolStr>,
) -> Result<Vec<String>, SmolStr> {
    command.into_iter().map(|s| substitute(&s, inputs)).collect()
}

fn substitute(s: &str, inputs: &IndexMap<SmolStr, SmolStr>) -> Result<String, SmolStr> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let end = after_open
            .find('}')
            .ok_or_else(|| SmolStr::new(&after_open[..after_open.len().min(32)]))?;
        let name = &after_open[..end];
        let value = inputs.get(name).ok_or_else(|| SmolStr::new(name))?;
        out.push_str(value);
        rest = &after_open[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(pairs: &[(&str, &str)]) -> IndexMap<SmolStr, SmolStr> {
        pairs
            .iter()
            .map(|(k, v)| (SmolStr::new(k), SmolStr::new(v)))
            .collect()
    }

    #[test]
    fn substitute_resolves_placeholders() {
        let inputs = inputs(&[("name", "world"), ("greet", "hello")]);
        assert_eq!(substitute("${greet}, ${name}!", &inputs).unwrap(), "hello, world!");
    }

    #[test]
    fn substitute_missing_returns_name() {
        let inputs = inputs(&[]);
        assert_eq!(substitute("a ${missing} b", &inputs), Err(SmolStr::new("missing")));
    }

    #[test]
    fn substitute_no_placeholders_passes_through() {
        let inputs = inputs(&[]);
        assert_eq!(substitute("plain text", &inputs).unwrap(), "plain text");
    }

    #[test]
    fn optional_action_skipped_when_input_missing() {
        let action = ShellInitAction {
            optional: true,
            kind: ActionKind::Export {
                name: "FOO".into(),
                value: "${missing}".into(),
            },
        };
        let pkg = SmolStr::new("testpkg");
        let result = resolve_action(action, &inputs(&[]), &pkg).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn required_action_errors_when_input_missing() {
        let action = ShellInitAction {
            optional: false,
            kind: ActionKind::Export {
                name: "FOO".into(),
                value: "${missing}".into(),
            },
        };
        let pkg = SmolStr::new("testpkg");
        let err = resolve_action(action, &inputs(&[]), &pkg).unwrap_err();
        assert_eq!(
            err,
            Error::UnresolvedInput {
                pkg: SmolStr::new("testpkg"),
                input: SmolStr::new("missing"),
            }
        );
    }

    #[test]
    fn action_resolves_when_input_present() {
        let action = ShellInitAction {
            optional: true,
            kind: ActionKind::Export {
                name: "FOO".into(),
                value: "${bar}".into(),
            },
        };
        let pkg = SmolStr::new("testpkg");
        let resolved = resolve_action(action, &inputs(&[("bar", "baz")]), &pkg)
            .unwrap()
            .unwrap();
        match resolved.kind {
            ActionKind::Export { value, .. } => assert_eq!(value, "baz"),
            _ => panic!("unexpected kind"),
        }
    }
}
