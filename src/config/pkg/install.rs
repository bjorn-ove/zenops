//! Per-package-manager install hints. Mirrors
//! [`super::super::super::output::PkgInstallHints`] and the runtime side of
//! `pkg_manager::DetectedPackageManager` — extend in lockstep when adding a
//! new manager.

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct InstallHint {
    pub brew: BrewHint,
    // Future package managers should be added as additional REQUIRED fields here
    // (e.g. apt, pacman, yum, dnf, zypper, apk, pkg) and wired into
    // `pkg_manager::DetectedPackageManager::{packages_for, install_command}`.
    // Keeping them required guarantees cross-manager completeness at parse time.
}

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct BrewHint {
    pub packages: Vec<String>,
}
