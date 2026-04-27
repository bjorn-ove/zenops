use crate::{
    config::{Config, PkgConfig},
    error::Error,
    output::{Output, PkgEntry, PkgEntryState, PkgInstallHints},
    pkg_manager,
};

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Case-insensitive substrings; a pkg passes if any pattern matches its
    /// display name or map key. Empty = no filter.
    pub pattern: Vec<String>,
    /// Include pkgs whose `enable = "disabled"`.
    pub all: bool,
    /// Show every install hint on a pkg, not just the one for the detected manager.
    pub all_hints: bool,
    /// Show extra diagnostic lines (e.g. the detect strategy that matched).
    pub verbose: bool,
}

/// Walk the configured packages in display order and push one [`PkgEntry`]
/// per visible row through `output`. The renderer chosen by `-o` decides
/// formatting (column-aligned text vs. NDJSON). Emits, in order:
///
/// 1. A single [`PkgEntry::NoPackageManagerDetected`] when no supported
///    package manager is on PATH (so install hints will be hidden).
/// 2. One [`PkgEntry::Pkg`] per visible package, after the same OS / shell
///    filtering that used to happen inside `render`.
/// 3. A single [`PkgEntry::AggregateInstall`] footer summarising the
///    combined install command across every missing pkg, when a manager is
///    detected and at least one pkg contributes packages.
pub fn push(config: &Config, opts: Options, output: &mut dyn Output) -> Result<(), Error> {
    let manager = pkg_manager::detect();
    if manager.is_none() {
        output.push_pkg_entry(PkgEntry::NoPackageManagerDetected {
            supported: vec!["brew".to_string()],
        })?;
    }

    let home = config.home();
    let configured_shell = config.shell();
    let needles: Vec<String> = opts.pattern.iter().map(|p| p.to_lowercase()).collect();
    // Entries carry (display_label, key, pkg). The map key stays distinct
    // from the display label so JSON consumers can correlate even when
    // `pkg.name` overrides the key.
    let mut entries: Vec<(&str, &smol_str::SmolStr, &PkgConfig)> = config
        .pkgs()
        .iter()
        .filter(|(_, p)| opts.all || !p.is_disabled())
        .filter(|(_, p)| p.supports_current_os())
        .filter(|(_, p)| p.supports_shell(configured_shell))
        .map(|(key, pkg)| (pkg.name.as_deref().unwrap_or(key.as_str()), key, pkg))
        .filter(|(label, key, _)| {
            needles.is_empty()
                || needles.iter().any(|n| {
                    label.to_lowercase().contains(n) || key.as_str().to_lowercase().contains(n)
                })
        })
        .collect();
    entries.sort_by_key(|(label, _, _)| *label);

    let mut aggregate_packages: Vec<String> = Vec::new();

    for (label, key, pkg) in entries {
        let state = if pkg.is_disabled() {
            PkgEntryState::Disabled
        } else if pkg.is_installed(home, config.system_inputs()) {
            PkgEntryState::Installed
        } else {
            PkgEntryState::Missing
        };

        let matched_detect = if opts.verbose {
            pkg.matched_detect(home, config.system_inputs())
                .map(|d| d.to_string())
        } else {
            None
        };

        // Install hints are only meaningful for missing pkgs. For
        // `--all-hints` we surface every populated manager; otherwise just
        // the detected manager (consistent with the human-mode flow that
        // hides install guidance when no manager is detected).
        let install_hints = if !matches!(state, PkgEntryState::Missing) {
            PkgInstallHints::default()
        } else if opts.all_hints {
            PkgInstallHints {
                brew: pkg.install_hint.brew.packages.clone(),
            }
        } else if let Some(mgr) = manager {
            let pkgs = mgr.packages_for(&pkg.install_hint);
            if pkgs.is_empty() {
                PkgInstallHints::default()
            } else {
                aggregate_packages.extend(pkgs.iter().cloned());
                PkgInstallHints {
                    brew: pkgs.to_vec(),
                }
            }
        } else {
            PkgInstallHints::default()
        };

        output.push_pkg_entry(PkgEntry::Pkg {
            name: smol_str::SmolStr::new(label),
            key: key.clone(),
            description: pkg.description.clone(),
            state,
            matched_detect,
            install_hints,
        })?;
    }

    if !opts.all_hints
        && let Some(mgr) = manager
        && !aggregate_packages.is_empty()
    {
        output.push_pkg_entry(PkgEntry::AggregateInstall {
            pkg_manager: mgr.name().to_string(),
            command: mgr.install_command(&aggregate_packages),
            packages: aggregate_packages,
        })?;
    }

    Ok(())
}
