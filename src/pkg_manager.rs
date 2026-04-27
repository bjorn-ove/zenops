//! Tiny abstraction over the host's package manager.
//!
//! [`detect`] probes `PATH` and returns a [`DetectedPackageManager`] when a
//! supported manager is available. Today only Homebrew is wired up; `apt`,
//! `pacman`, and friends are flagged as expansion points in-source — adding
//! one means extending the enum, [`detect`], [`DetectedPackageManager::packages_for`],
//! [`DetectedPackageManager::install_command`], and the matching
//! `InstallHint` field.

use crate::config::pkg::{InstallHint, which_on_path};

/// A package manager zenops successfully detected on the current host.
/// Today only Homebrew; the variant enumerates expansion points for
/// future managers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedPackageManager {
    /// Homebrew (macOS or Linuxbrew).
    Brew,
    // Future package managers (apt, pacman, yum, dnf, zypper, apk, pkg, …)
    // should be added here, detected in `detect`, and wired into
    // `packages_for` / `install_command` / `InstallHint`.
}

impl DetectedPackageManager {
    /// Stable lowercase identifier, suitable for shell strings, JSON
    /// output, and serde tags (e.g. `"brew"`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Brew => "brew",
        }
    }

    /// Packages this manager would install for the given install hint.
    /// Returns an empty slice when the hint has no entry for this manager.
    pub fn packages_for(self, hint: &InstallHint) -> &[String] {
        match self {
            Self::Brew => &hint.brew.packages,
        }
    }

    /// Build the one-shot command that installs the given packages via this manager.
    pub fn install_command(self, packages: &[String]) -> String {
        match self {
            Self::Brew => format!("brew install {}", packages.join(" ")),
        }
    }
}

/// Probe `PATH` for a supported package manager. Returns the first match
/// in priority order, or `None` if nothing supported is on `PATH`.
pub fn detect() -> Option<DetectedPackageManager> {
    if which_on_path("brew") {
        return Some(DetectedPackageManager::Brew);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::pkg::BrewHint;

    #[test]
    fn packages_for_brew_returns_packages_list() {
        let hint = InstallHint {
            brew: BrewHint {
                packages: vec!["sk".into(), "fd".into()],
            },
        };
        assert_eq!(
            DetectedPackageManager::Brew.packages_for(&hint),
            ["sk", "fd"]
        );
    }

    #[test]
    fn packages_for_empty_returns_empty_slice() {
        let hint = InstallHint::default();
        assert!(DetectedPackageManager::Brew.packages_for(&hint).is_empty());
    }

    #[test]
    fn install_command_joins_packages_with_spaces() {
        let pkgs = vec!["sk".into(), "starship".into()];
        assert_eq!(
            DetectedPackageManager::Brew.install_command(&pkgs),
            "brew install sk starship"
        );
    }
}
