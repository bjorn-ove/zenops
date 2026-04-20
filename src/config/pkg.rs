use std::path::Path;

use indexmap::IndexMap;
use smol_str::SmolStr;
use zenops_expand::{ExpandLookup, ExpandStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    Bash,
    Zsh,
}

/// Operating systems a pkg may opt into supporting. Extend as new platforms are
/// added. Kept intentionally coarse; finer targeting (e.g. macOS Apple Silicon
/// vs Intel, specific Linux distros) is deferred until a real use case arrives.
#[derive(serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    Linux,
    Macos,
}

impl Os {
    pub fn current() -> Option<Self> {
        match std::env::consts::OS {
            "linux" => Some(Self::Linux),
            "macos" => Some(Self::Macos),
            _ => None,
        }
    }
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
    pub login_init: PerShellActions,
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
    /// When non-empty, the pkg is only considered installed on the listed
    /// operating systems — an empty list means "any OS".
    #[serde(default)]
    pub(super) supported_os: Vec<Os>,
    /// Optional display label, used by `pkg_list` instead of the map key.
    /// Lets two OS-gated entries (e.g. `brew-linux` / `brew-macos`) share a
    /// single user-facing name while keeping distinct config keys.
    #[serde(default)]
    pub name: Option<SmolStr>,
    #[serde(default)]
    pub description: Option<String>,
    pub install_hint: InstallHint,
    #[serde(default)]
    pub(crate) shell: PkgShellConfig,
}

impl PkgConfig {
    pub fn is_installed(&self, home: &Path, system_inputs: &IndexMap<SmolStr, SmolStr>) -> bool {
        if !self.supports_current_os() {
            return false;
        }
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
        if !self.supports_current_os() {
            return None;
        }
        match self.enable {
            PkgEnable::Detect => {
                let lookup = [&self.inputs, system_inputs];
                self.detect.iter().find(|s| s.check(home, &lookup))
            }
            PkgEnable::On | PkgEnable::Disabled => None,
        }
    }

    pub(crate) fn supports_current_os(&self) -> bool {
        self.supported_os.is_empty()
            || Os::current().is_some_and(|os| self.supported_os.contains(&os))
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

    #[test]
    fn supported_os_gates_installation() {
        // Pick the OS that is not current so the pkg must be filtered out.
        let other = match Os::current().expect("tests run on supported OS") {
            Os::Linux => "macos",
            Os::Macos => "linux",
        };
        let toml_src = format!(
            r#"
            enable = "on"
            supported_os = ["{other}"]
            [install_hint.brew]
            packages = []
            "#
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!pkg.is_installed(tmp.path(), &system_empty()));
        assert!(pkg.matched_detect(tmp.path(), &system_empty()).is_none());
    }

    #[test]
    fn supported_os_allows_installation_when_current_os_listed() {
        let current = match Os::current().expect("tests run on supported OS") {
            Os::Linux => "linux",
            Os::Macos => "macos",
        };
        let toml_src = format!(
            r#"
            enable = "on"
            supported_os = ["{current}"]
            [install_hint.brew]
            packages = []
            "#
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn empty_supported_os_means_any_os() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn name_field_round_trips_from_toml() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            name = "brew"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        assert_eq!(pkg.name.as_deref(), Some("brew"));
    }

    #[test]
    fn name_field_defaults_to_none() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        assert!(pkg.name.is_none());
    }

    #[test]
    fn path_action_kinds_round_trip_from_toml() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [[shell.env_init.bash]]
            type = "path_prepend"
            value = "/opt/foo/bin"
            [[shell.env_init.bash]]
            type = "path_append"
            value = "/opt/bar/bin"
            "#,
        )
        .unwrap();
        let actions = &pkg.shell.env_init.for_shell(Shell::Bash);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0].kind, ActionKind::PathPrepend { .. }));
        assert!(matches!(actions[1].kind, ActionKind::PathAppend { .. }));
    }
}
