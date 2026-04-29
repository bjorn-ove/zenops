//! Package configuration: the `[pkg.<key>]` table.
//!
//! [`PkgConfig`] is the parsed shape; submodules host the supporting
//! sublanguages:
//!
//! - [`detect`] — the `detect = ...` strategy language (file/which leaves +
//!   any/all combinators with optional OS gating).
//! - [`install`] — `install_hint` — per-package-manager install commands.
//! - [`action`] — `shell.{env_init,login_init,interactive_init}` shell-init
//!   action lines and the per-shell routing.

mod action;
mod detect;
mod install;

#[cfg(test)]
mod tests;

use std::path::Path;

use indexmap::IndexMap;
use smol_str::SmolStr;

use super::pkg_config_files::PkgConfigFiles;

pub(crate) use action::PkgShellConfig;
pub use action::{ActionKind, ShellInitAction};
pub use detect::DetectStrategy;
pub(crate) use detect::which_on_path;
// `BrewHint` is reachable from outside the crate via this re-export; only the
// pkg_manager test module imports it directly today, which the non-test build
// can't see.
#[allow(unused_imports)]
pub use install::BrewHint;
pub use install::InstallHint;

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Shell {
    Bash,
    Zsh,
}

/// Operating systems a pkg may opt into supporting. Extend as new platforms are
/// added. Kept intentionally coarse; finer targeting (e.g. macOS Apple Silicon
/// vs Intel, specific Linux distros) is deferred until a real use case arrives.
#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
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

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
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
