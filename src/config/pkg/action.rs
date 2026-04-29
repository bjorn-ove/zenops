//! Shell init actions: lines a pkg contributes to the rendered shell
//! profile. [`ShellInitAction`] wraps an [`ActionKind`] (the concrete line:
//! `Source`, `Export`, `PathPrepend`, …) with an `optional` flag for
//! best-effort lines. [`PerShellActions`] / [`PkgShellConfig`] route them
//! into the right rc file (env, login, interactive) per shell.

use zenops_expand::ExpandStr;

use super::Shell;

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
pub struct ShellInitAction {
    #[serde(default)]
    pub optional: bool,
    #[serde(flatten)]
    pub kind: ActionKind,
}

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionKind {
    Comment {
        text: ExpandStr,
    },
    Source {
        path: ExpandStr,
    },
    EvalOutput {
        command: Vec<ExpandStr>,
    },
    SourceOutput {
        command: Vec<ExpandStr>,
    },
    Export {
        name: ExpandStr,
        value: ExpandStr,
    },
    Line {
        line: ExpandStr,
    },
    /// Emit `export PATH="VALUE:$PATH"` using the current shell's PATH
    /// syntax. The renderer owns the delimiter, quoting, and `$PATH`
    /// position so non-POSIX shells can change it in one place.
    PathPrepend {
        value: ExpandStr,
    },
    /// Emit `export PATH="$PATH:VALUE"` using the current shell's PATH
    /// syntax. See `PathPrepend` for the "renderer owns the how" rationale.
    PathAppend {
        value: ExpandStr,
    },
}

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
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

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub(crate) struct PkgShellConfig {
    pub env_init: PerShellActions,
    pub login_init: PerShellActions,
    pub interactive_init: PerShellActions,
}
