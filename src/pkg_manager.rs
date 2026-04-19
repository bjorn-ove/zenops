use crate::config::pkg::{InstallHint, which_on_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedPackageManager {
    Brew,
    // Future package managers (apt, pacman, yum, dnf, zypper, apk, pkg, …)
    // should be added here, detected in `detect`, and wired into
    // `packages_for` / `install_command` / `InstallHint`.
}

impl DetectedPackageManager {
    pub fn name(self) -> &'static str {
        match self {
            Self::Brew => "brew",
        }
    }

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
