use std::path::Path;

use indexmap::IndexMap;
use smol_str::SmolStr;
use zenops_expand::{ExpandLookup, ExpandStr};

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
    File { path: ExpandStr },
    Which { binary: ExpandStr },
}

impl DetectStrategy {
    /// Expand placeholders and check the resulting detect target. An unresolved
    /// placeholder (e.g. `${brew_prefix}` on a brew-less system) means the
    /// strategy is inapplicable on this host — return `false`.
    pub fn check(&self, home: &Path, lookup: &impl ExpandLookup) -> bool {
        match self {
            Self::File { path } => {
                let Ok(expanded) = path.expand_to_string(lookup) else {
                    return false;
                };
                let resolved = expanded.replacen('~', &home.to_string_lossy(), 1);
                Path::new(&resolved).exists()
            }
            Self::Which { binary } => match binary.expand_to_string(lookup) {
                Ok(b) => which_on_path(&b),
                Err(_) => false,
            },
        }
    }
}

impl std::fmt::Display for DetectStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(f, "{}", path.as_template()),
            Self::Which { binary } => write!(f, "which {}", binary.as_template()),
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
    Comment { text: ExpandStr },
    Source { path: ExpandStr },
    EvalOutput { command: Vec<ExpandStr> },
    SourceOutput { command: Vec<ExpandStr> },
    Export { name: ExpandStr, value: ExpandStr },
    Line { line: ExpandStr },
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
pub(crate) struct PkgShellConfig {
    pub env_init: PerShellActions,
    pub interactive_init: PerShellActions,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
pub struct PkgConfig {
    #[serde(default)]
    pub(super) enable: PkgEnable,
    #[serde(default)]
    pub(super) detect: Vec<DetectStrategy>,
    #[serde(default)]
    pub(super) inputs: IndexMap<SmolStr, SmolStr>,
    #[serde(default)]
    pub description: Option<String>,
    pub install_hint: InstallHint,
    #[serde(default)]
    pub(crate) shell: PkgShellConfig,
}

impl PkgConfig {
    pub fn is_installed(&self, home: &Path, system_inputs: &IndexMap<SmolStr, SmolStr>) -> bool {
        match self.enable {
            PkgEnable::On => true,
            PkgEnable::Detect => {
                let lookup = [&self.inputs, system_inputs];
                self.detect.iter().any(|s| s.check(home, &lookup))
            }
            PkgEnable::Disabled => false,
        }
    }

    pub fn is_disabled(&self) -> bool {
        matches!(self.enable, PkgEnable::Disabled)
    }

    /// First detect strategy that matches, if any — used for debuggable output.
    pub fn matched_detect(
        &self,
        home: &Path,
        system_inputs: &IndexMap<SmolStr, SmolStr>,
    ) -> Option<&DetectStrategy> {
        match self.enable {
            PkgEnable::Detect => {
                let lookup = [&self.inputs, system_inputs];
                self.detect.iter().find(|s| s.check(home, &lookup))
            }
            PkgEnable::On | PkgEnable::Disabled => None,
        }
    }

    pub(crate) fn inputs(&self) -> &IndexMap<SmolStr, SmolStr> {
        &self.inputs
    }
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

    fn system_empty() -> IndexMap<SmolStr, SmolStr> {
        IndexMap::new()
    }

    #[test]
    fn install_hint_round_trips_from_toml() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            description = "fuzzy finder"
            [install_hint.brew]
            packages = ["sk"]
            "#,
        )
        .unwrap();
        assert_eq!(pkg.description.as_deref(), Some("fuzzy finder"));
        assert_eq!(pkg.install_hint.brew.packages, vec!["sk".to_string()]);
    }

    #[test]
    fn install_hint_is_required_in_toml() {
        let err =
            toml::from_str::<PkgConfig>(r#"description = "missing install_hint""#).unwrap_err();
        assert!(
            err.to_string().contains("install_hint"),
            "expected error to mention install_hint, got: {err}"
        );
    }

    #[test]
    fn detect_strategy_with_unresolved_input_reports_not_installed() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [[detect]]
            type = "file"
            path = "${brew_prefix}/opt/x"
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!pkg.is_installed(tmp.path(), &system_empty()));
        assert!(pkg.matched_detect(tmp.path(), &system_empty()).is_none());
    }

    #[test]
    fn detect_strategy_resolves_system_input_and_checks_file() {
        let tmp = tempfile::tempdir().unwrap();
        let opt = tmp.path().join("opt/x");
        std::fs::create_dir_all(&opt).unwrap();
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [[detect]]
            type = "file"
            path = "${root}/opt/x"
            "#,
        )
        .unwrap();
        let sys = inputs(&[("root", tmp.path().to_str().unwrap())]);
        assert!(pkg.is_installed(tmp.path(), &sys));
    }

    #[test]
    fn pkg_inputs_shadow_system_inputs_at_detect() {
        let tmp = tempfile::tempdir().unwrap();
        let marker_a = tmp.path().join("a");
        std::fs::write(&marker_a, "").unwrap();
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [inputs]
            name = "a"
            [[detect]]
            type = "file"
            path = "{}/${{name}}"
            "#,
            tmp.path().display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        let sys = inputs(&[("name", "b")]);
        assert!(pkg.is_installed(tmp.path(), &sys));
    }

    #[test]
    fn disabled_pkg_is_never_installed() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "disabled"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!pkg.is_installed(tmp.path(), &system_empty()));
        assert!(pkg.is_disabled());
    }
}
