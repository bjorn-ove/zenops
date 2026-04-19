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
pub(crate) enum PkgEnable {
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
            Self::Which { binary } => which_on_path(binary),
        }
    }
}

impl std::fmt::Display for DetectStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(f, "{path}"),
            Self::Which { binary } => write!(f, "which {binary}"),
        }
    }
}

pub(crate) fn which_on_path(binary: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| Path::new(dir).join(binary).is_file())
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct InstallHint {
    pub brew: BrewHint,
    // Future package managers should be added as additional REQUIRED fields here
    // (e.g. apt, pacman, yum, dnf, zypper, apk, pkg) and wired into
    // `pkg_manager::DetectedPackageManager::{packages_for, install_command}`.
    // Keeping them required guarantees cross-manager completeness at parse time.
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct BrewHint {
    pub packages: Vec<String>,
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
pub(crate) struct PerShellActions {
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
pub(crate) struct StoredPkgShellConfig {
    pub env_init: PerShellActions,
    pub interactive_init: PerShellActions,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
pub(crate) struct StoredPkgConfig {
    #[serde(default)]
    pub enable: PkgEnable,
    #[serde(default)]
    pub detect: Vec<DetectStrategy>,
    #[serde(default)]
    pub inputs: IndexMap<SmolStr, SmolStr>,
    #[serde(default)]
    pub description: Option<String>,
    pub install_hint: InstallHint,
    #[serde(default)]
    pub shell: StoredPkgShellConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PkgConfig {
    pub(super) enable: PkgEnable,
    pub(super) detect: Vec<DetectStrategy>,
    pub description: Option<String>,
    pub install_hint: InstallHint,
    pub(super) env_init: PerShellActions,
    pub(super) interactive_init: PerShellActions,
}

impl StoredPkgConfig {
    pub fn resolve(self, pkg_name: &SmolStr) -> Result<PkgConfig, Error> {
        // A pkg that can never be "installed" (detection will always return false)
        // won't have its shell actions emitted — so we skip action resolution to
        // avoid erroring on unresolved inputs that the user will never hit.
        let can_ever_install = match self.enable {
            PkgEnable::On => true,
            PkgEnable::Detect => !self.detect.is_empty(),
            PkgEnable::Disabled => false,
        };

        let (env_init, interactive_init) = if can_ever_install {
            (
                resolve_per_shell(self.shell.env_init, &self.inputs, pkg_name)?,
                resolve_per_shell(self.shell.interactive_init, &self.inputs, pkg_name)?,
            )
        } else {
            (PerShellActions::default(), PerShellActions::default())
        };

        Ok(PkgConfig {
            enable: self.enable,
            detect: self.detect,
            description: self.description,
            install_hint: self.install_hint,
            env_init,
            interactive_init,
        })
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

    pub fn is_disabled(&self) -> bool {
        matches!(self.enable, PkgEnable::Disabled)
    }

    /// First detect strategy that matches, if any — used for debuggable output.
    pub fn matched_detect(&self, home: &Path) -> Option<&DetectStrategy> {
        match self.enable {
            PkgEnable::Detect => self.detect.iter().find(|s| s.check(home)),
            PkgEnable::On | PkgEnable::Disabled => None,
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
    command
        .into_iter()
        .map(|s| substitute(&s, inputs))
        .collect()
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
        assert_eq!(
            substitute("${greet}, ${name}!", &inputs).unwrap(),
            "hello, world!"
        );
    }

    #[test]
    fn substitute_missing_returns_name() {
        let inputs = inputs(&[]);
        assert_eq!(
            substitute("a ${missing} b", &inputs),
            Err(SmolStr::new("missing"))
        );
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
    fn install_hint_round_trips_from_toml() {
        let stored: StoredPkgConfig = toml::from_str(
            r#"
            description = "fuzzy finder"
            [install_hint.brew]
            packages = ["sk"]
            "#,
        )
        .unwrap();
        assert_eq!(stored.description.as_deref(), Some("fuzzy finder"));
        assert_eq!(stored.install_hint.brew.packages, vec!["sk".to_string()]);
        let resolved = stored.resolve(&SmolStr::new("sk")).unwrap();
        assert_eq!(resolved.description.as_deref(), Some("fuzzy finder"));
        assert_eq!(resolved.install_hint.brew.packages, vec!["sk".to_string()]);
    }

    #[test]
    fn install_hint_is_required_in_toml() {
        let err = toml::from_str::<StoredPkgConfig>(r#"description = "missing install_hint""#)
            .unwrap_err();
        assert!(
            err.to_string().contains("install_hint"),
            "expected error to mention install_hint, got: {err}"
        );
    }

    #[test]
    fn disabled_pkg_resolves_without_evaluating_actions() {
        let stored: StoredPkgConfig = toml::from_str(
            r#"
            enable = "disabled"
            [install_hint.brew]
            packages = []
            [[shell.env_init.bash]]
            type = "export"
            name = "X"
            value = "${missing_input}"
            "#,
        )
        .unwrap();
        // Under the previous behavior this would have short-circuited to None;
        // the new behavior resolves the pkg but skips action resolution so the
        // unresolved input does not error.
        let resolved = stored.resolve(&SmolStr::new("ghost")).unwrap();
        assert!(resolved.is_disabled());
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
