use std::path::Path;

use indexmap::IndexMap;
use smol_str::SmolStr;
use zenops_expand::{ExpandLookup, ExpandStr};

use super::pkg_config_files::PkgConfigFiles;

#[derive(serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
    /// Expect the pkg to be present. Installation state still gates on
    /// detect strategies (if any), so a miss yields `is_installed = false`
    /// just like `Detect`. The distinction is intent: `On` signals the user
    /// expects the pkg, so rendering commands (e.g. `zenops pkg`) may
    /// surface the miss more prominently than for `Detect`. Default so a
    /// bare `[pkg.x]` reads as "I want this."
    #[default]
    On,
    /// Opt-in: use the pkg when detect matches, ignore it otherwise. Right
    /// variant for tooling the user may or may not have installed; miss is
    /// treated as a non-event.
    Detect,
    Disabled,
}

/// A detect strategy wraps a concrete check (`kind`) with an optional OS gate.
/// When `os` is non-empty and doesn't include the current OS, `check()`
/// short-circuits to `false` — the strategy is treated as a miss on that host.
#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
pub struct DetectStrategy {
    #[serde(default)]
    pub os: Vec<Os>,
    #[serde(flatten)]
    pub kind: DetectKind,
}

/// Concrete detect checks. `File` and `Which` are leaves; `Any` and `All` are
/// combinators that let a single `detect` field express arbitrary boolean
/// logic by nesting other strategies.
#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DetectKind {
    File {
        path: ExpandStr,
    },
    Which {
        binary: ExpandStr,
    },
    /// Matches when **any** child strategy matches (short-circuits).
    Any {
        of: Vec<DetectStrategy>,
    },
    /// Matches when **every** child strategy matches. An empty `of` is
    /// vacuously true — callers should prefer omitting the pkg's `detect`
    /// field entirely to express "no check required".
    All {
        of: Vec<DetectStrategy>,
    },
}

impl DetectStrategy {
    /// Apply the OS gate first, then delegate to the kind. Unresolved
    /// `${var}` placeholders inside the leaf checks also yield `false`.
    pub fn check(&self, home: &Path, lookup: &impl ExpandLookup) -> bool {
        if !self.os.is_empty() {
            let Some(cur) = Os::current() else {
                return false;
            };
            if !self.os.contains(&cur) {
                return false;
            }
        }
        self.kind.check(home, lookup)
    }
}

impl DetectKind {
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
            Self::Any { of } => of.iter().any(|s| s.check(home, lookup)),
            Self::All { of } => of.iter().all(|s| s.check(home, lookup)),
        }
    }
}

impl std::fmt::Display for DetectStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.os.is_empty() {
            let names: Vec<&'static str> = self
                .os
                .iter()
                .map(|o| match o {
                    Os::Linux => "linux",
                    Os::Macos => "macos",
                })
                .collect();
            write!(f, "[os={}] ", names.join(","))?;
        }
        write!(f, "{}", self.kind)
    }
}

impl std::fmt::Display for DetectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File { path } => write!(f, "{}", path.as_template()),
            Self::Which { binary } => write!(f, "which {}", binary.as_template()),
            Self::Any { of } => write_combinator(f, "any", of),
            Self::All { of } => write_combinator(f, "all", of),
        }
    }
}

fn write_combinator(
    f: &mut std::fmt::Formatter<'_>,
    name: &str,
    of: &[DetectStrategy],
) -> std::fmt::Result {
    write!(f, "{name}(")?;
    for (i, s) in of.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{s}")?;
    }
    write!(f, ")")
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
    pub(super) detect: Option<DetectStrategy>,
    #[serde(default)]
    pub(super) inputs: IndexMap<SmolStr, SmolStr>,
    /// When non-empty, the pkg is only considered installed on the listed
    /// operating systems — an empty list means "any OS".
    #[serde(default)]
    pub(super) supported_os: Vec<Os>,
    /// When non-empty, the pkg only applies when the user has configured
    /// one of the listed shells — empty means "any shell". Unlike
    /// `supported_os`, this is a relevance filter rather than an
    /// "installed on the system" filter; it gates list visibility and
    /// init-action emission but not `is_installed`.
    #[serde(default)]
    pub(super) supported_shells: Vec<Shell>,
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
    /// Dotfiles owned by this pkg — symlinks or generated files under
    /// `~/.config/<name>/` or `~/<dir>/`. Only applied when the pkg is
    /// considered installed (see `is_installed`).
    #[serde(default)]
    pub(super) configs: Vec<PkgConfigFiles>,
}

impl PkgConfig {
    pub fn is_installed(&self, home: &Path, system_inputs: &IndexMap<SmolStr, SmolStr>) -> bool {
        if !self.supports_current_os() {
            return false;
        }
        match self.enable {
            // `on` and `detect` run the same installation check; an absent
            // `detect` field means "nothing to check" → installed. They
            // diverge only in how consumers *surface* a miss: for `on`,
            // `enable_on_but_detect_missing` flags it so callers can push
            // a `Status::Pkg { status: PkgStatus::Missing }` to structured
            // output; `detect` miss is silent by design.
            PkgEnable::On | PkgEnable::Detect => {
                let Some(detect) = self.detect.as_ref() else {
                    return true;
                };
                let lookup = [&self.inputs, system_inputs];
                detect.check(home, &lookup)
            }
            PkgEnable::Disabled => false,
        }
    }

    /// Config-health predicate: `true` only when the user declared
    /// `enable = "on"` with a detect strategy that doesn't match on the
    /// current host. Rendering layers use this to push a user-facing
    /// "pkg is missing" signal via `Output`. Returns `false` for `detect`
    /// (miss is silent), `disabled`, OS-gated-out pkgs, and `on` pkgs with
    /// absent or matching detect.
    pub fn enable_on_but_detect_missing(
        &self,
        home: &Path,
        system_inputs: &IndexMap<SmolStr, SmolStr>,
    ) -> bool {
        if !matches!(self.enable, PkgEnable::On) {
            return false;
        }
        if !self.supports_current_os() {
            return false;
        }
        let Some(detect) = self.detect.as_ref() else {
            return false;
        };
        let lookup = [&self.inputs, system_inputs];
        !detect.check(home, &lookup)
    }

    /// Complement of [`Self::enable_on_but_detect_missing`] within `enable =
    /// "on"`. True when the user declared `enable = "on"`, there's a detect
    /// strategy, and it matches on the current host — a real positive check
    /// that something got verified. Used to emit a clean-state `Status::Pkg
    /// { status: Ok }` so `zenops status --all` can show the pkg was
    /// looked at. Absent-detect pkgs (e.g. meta/scaffolding configs like
    /// `bashrc-chain`) stay silent: "no detect" means "nothing to report
    /// as verified." Like its counterpart, silent for `detect` /
    /// `disabled` / OS-gated-out pkgs.
    pub fn enable_on_and_detect_matches(
        &self,
        home: &Path,
        system_inputs: &IndexMap<SmolStr, SmolStr>,
    ) -> bool {
        if !matches!(self.enable, PkgEnable::On) {
            return false;
        }
        if !self.supports_current_os() {
            return false;
        }
        let Some(detect) = self.detect.as_ref() else {
            return false;
        };
        let lookup = [&self.inputs, system_inputs];
        detect.check(home, &lookup)
    }

    pub fn is_disabled(&self) -> bool {
        matches!(self.enable, PkgEnable::Disabled)
    }

    /// The top-level detect strategy when it matches on the current host —
    /// used for debuggable output. Inside an `any` / `all` combinator this
    /// returns the wrapper; consumers that care about the matching leaf can
    /// walk `.kind` themselves.
    pub fn matched_detect(
        &self,
        home: &Path,
        system_inputs: &IndexMap<SmolStr, SmolStr>,
    ) -> Option<&DetectStrategy> {
        if !self.supports_current_os() {
            return None;
        }
        match self.enable {
            PkgEnable::On | PkgEnable::Detect => {
                let detect = self.detect.as_ref()?;
                let lookup = [&self.inputs, system_inputs];
                detect.check(home, &lookup).then_some(detect)
            }
            PkgEnable::Disabled => None,
        }
    }

    pub(crate) fn supports_current_os(&self) -> bool {
        self.supported_os.is_empty()
            || Os::current().is_some_and(|os| self.supported_os.contains(&os))
    }

    pub(crate) fn supports_shell(&self, shell: Option<Shell>) -> bool {
        self.supported_shells.is_empty()
            || shell.is_some_and(|s| self.supported_shells.contains(&s))
    }

    pub(crate) fn inputs(&self) -> &IndexMap<SmolStr, SmolStr> {
        &self.inputs
    }

    pub(super) fn configs(&self) -> &[PkgConfigFiles] {
        &self.configs
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
            [detect]
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
            [detect]
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
            [detect]
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
    fn enable_on_with_matching_detect_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join(".marker");
        std::fs::write(&marker, "").unwrap();
        let toml_src = format!(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            "#,
            marker.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
    }

    #[test]
    fn enable_on_with_missing_detect_flags_health_signal() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "/definitely/does/not/exist/zenops-test"
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
    }

    #[test]
    fn enable_on_with_empty_detect_is_silent() {
        // No detect strategies → nothing to check → no health signal even
        // under `enable = "on"`. This is the "always-on meta-pkg" shape
        // (bashrc-chain, local-bin, zenops).
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
    }

    #[test]
    fn enable_detect_miss_is_silent() {
        // Silence on miss is the point of `detect`; don't surface a signal.
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "/definitely/does/not/exist/zenops-test"
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
    }

    #[test]
    fn enable_on_with_matching_detect_is_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join(".marker");
        std::fs::write(&marker, "").unwrap();
        let toml_src = format!(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            "#,
            marker.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn enable_on_with_missing_detect_is_not_installed() {
        // `on` + detect miss treats the pkg as not installed, same as
        // `detect` + miss. Rendering code is responsible for any visual
        // distinction; this predicate only reports installation state.
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "on"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "/definitely/does/not/exist/zenops-test"
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(!pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn enable_on_with_empty_detect_is_installed() {
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
    fn empty_detect_with_enable_detect_means_installed() {
        // A pkg with `enable = "detect"` but no detect strategies has nothing
        // to check, so it's treated as installed. This lets config-only pkgs
        // stay ergonomic without setting `enable = "on"`.
        let pkg: PkgConfig = toml::from_str(
            r#"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
        // No detect ran, so `matched_detect` still has nothing to return.
        assert!(pkg.matched_detect(tmp.path(), &system_empty()).is_none());
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
    fn supported_shells_round_trips_from_toml() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            supported_shells = ["bash", "zsh"]
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        assert_eq!(pkg.supported_shells, vec![Shell::Bash, Shell::Zsh]);
    }

    #[test]
    fn supports_shell_empty_list_means_any() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        assert!(pkg.supports_shell(Some(Shell::Bash)));
        assert!(pkg.supports_shell(Some(Shell::Zsh)));
        assert!(pkg.supports_shell(None));
    }

    #[test]
    fn supports_shell_filters_by_list() {
        let pkg: PkgConfig = toml::from_str(
            r#"
            supported_shells = ["bash"]
            [install_hint.brew]
            packages = []
            "#,
        )
        .unwrap();
        assert!(pkg.supports_shell(Some(Shell::Bash)));
        assert!(!pkg.supports_shell(Some(Shell::Zsh)));
        assert!(!pkg.supports_shell(None));
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
    fn any_combinator_matches_when_any_child_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let present = tmp.path().join(".present");
        std::fs::write(&present, "").unwrap();
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "any"
            of = [
              {{ type = "file", path = "/definitely/does/not/exist/zenops-test" }},
              {{ type = "file", path = "{}" }},
            ]
            "#,
            present.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn all_combinator_requires_every_child() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::write(&a, "").unwrap();
        // Only `a` exists — `all` should miss.
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "all"
            of = [
              {{ type = "file", path = "{}" }},
              {{ type = "file", path = "{}" }},
            ]
            "#,
            a.display(),
            b.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(!pkg.is_installed(tmp.path(), &system_empty()));

        std::fs::write(&b, "").unwrap();
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn nested_combinators_compose() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        // any(all(a, b), which=<missing>) — inner `all` hits, outer `any` hits.
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "any"
            of = [
              {{ type = "all", of = [
                {{ type = "file", path = "{}" }},
                {{ type = "file", path = "{}" }},
              ] }},
              {{ type = "which", binary = "definitely-not-on-path-zenops-test" }},
            ]
            "#,
            a.display(),
            b.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn per_strategy_os_skips_when_os_mismatches() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join(".marker");
        std::fs::write(&marker, "").unwrap();
        let other = match Os::current().expect("tests run on supported OS") {
            Os::Linux => "macos",
            Os::Macos => "linux",
        };
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            os = ["{other}"]
            "#,
            marker.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        // The file exists, but the strategy is gated to the other OS — skip.
        assert!(!pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn per_strategy_os_allows_when_os_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join(".marker");
        std::fs::write(&marker, "").unwrap();
        let current = match Os::current().expect("tests run on supported OS") {
            Os::Linux => "linux",
            Os::Macos => "macos",
        };
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            os = ["{current}"]
            "#,
            marker.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn empty_os_list_means_any_os() {
        // An `os = []` (or field omitted) strategy is applicable on every OS.
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join(".marker");
        std::fs::write(&marker, "").unwrap();
        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "file"
            path = "{}"
            os = []
            "#,
            marker.display()
        );
        let pkg: PkgConfig = toml::from_str(&toml_src).unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
    }

    #[test]
    fn absent_detect_field_is_installed_and_silent() {
        // No `detect` field at all → nothing to check → installed, no health
        // signal even under `enable = "on"`. This is the "always-on meta-pkg"
        // shape (bashrc-chain, local-bin, zenops).
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
        assert!(!pkg.enable_on_but_detect_missing(tmp.path(), &system_empty()));
    }

    #[test]
    fn all_with_empty_of_matches_vacuously() {
        // An empty `all` is vacuously true. Documented so we don't quietly
        // change this later; in practice, users should omit `detect` entirely
        // if they mean "no check required".
        let pkg: PkgConfig = toml::from_str(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = []
            [detect]
            type = "all"
            of = []
            "#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        assert!(pkg.is_installed(tmp.path(), &system_empty()));
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
