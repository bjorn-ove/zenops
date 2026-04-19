use std::path::Path;

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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShellInitAction {
    Source { path: String, comment: Option<String> },
    EvalOutput { command: Vec<String>, comment: Option<String> },
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
    pub fn resolve(self) -> Option<PkgConfig> {
        match self.enable {
            PkgEnable::Disabled => None,
            PkgEnable::Detect if self.detect.is_empty() => None,
            _ => Some(PkgConfig {
                enable: self.enable,
                detect: self.detect,
                env_init: self.shell.env_init,
                interactive_init: self.shell.interactive_init,
            }),
        }
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
