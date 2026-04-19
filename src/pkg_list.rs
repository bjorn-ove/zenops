use std::fmt::Write;

use crate::{
    ColorChoice,
    config::{Config, PkgConfig},
    config_files::ConfigFileDirs,
    error::Error,
    pkg_manager,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Include pkgs whose `enable = "disabled"`.
    pub all: bool,
    /// Show every install hint on a pkg, not just the one for the detected manager.
    pub all_hints: bool,
    /// Show extra diagnostic lines (e.g. the detect strategy that matched).
    pub verbose: bool,
    /// Emit ANSI colour escapes in the rendered output.
    pub color_enabled: bool,
}

/// Convenience entry point for tests and external callers: load the config and
/// render the list in a single call. `color` is resolved via `ColorChoice::enabled`
/// so callers don't have to care about TTY/`NO_COLOR` detection.
pub fn list_from_dirs(
    dirs: &ConfigFileDirs,
    all: bool,
    all_hints: bool,
    verbose: bool,
    color: ColorChoice,
) -> Result<String, Error> {
    let sh = xshell::Shell::new().unwrap();
    let config = Config::load(dirs, &sh, false)?;
    Ok(render(
        &config,
        Options {
            all,
            all_hints,
            verbose,
            color_enabled: color.enabled(),
        },
    ))
}

struct Styles {
    on: bool,
}

impl Styles {
    const BOLD: &'static str = "\x1b[1m";
    const DIM: &'static str = "\x1b[2m";
    const GREEN: &'static str = "\x1b[32m";
    const RED: &'static str = "\x1b[31m";
    const YELLOW: &'static str = "\x1b[33m";
    const RESET: &'static str = "\x1b[0m";

    fn ansi(&self, code: &'static str) -> &'static str {
        if self.on { code } else { "" }
    }

    fn bold(&self) -> &'static str {
        self.ansi(Self::BOLD)
    }
    fn dim(&self) -> &'static str {
        self.ansi(Self::DIM)
    }
    fn green(&self) -> &'static str {
        self.ansi(Self::GREEN)
    }
    fn red(&self) -> &'static str {
        self.ansi(Self::RED)
    }
    fn reset(&self) -> &'static str {
        self.ansi(Self::RESET)
    }
    fn hint(&self) -> String {
        if self.on {
            format!("{}{}", Self::BOLD, Self::YELLOW)
        } else {
            String::new()
        }
    }
}

pub fn render(config: &Config, opts: Options) -> String {
    let manager = pkg_manager::detect();
    if manager.is_none() {
        log::warn!(
            "No known package manager detected on PATH; install guidance will be hidden. \
             Supported managers: brew."
        );
    }

    let home = config.home();
    let mut entries: Vec<(&str, &PkgConfig)> = config
        .pkgs()
        .iter()
        .filter(|(_, p)| opts.all || !p.is_disabled())
        .map(|(name, pkg)| (name.as_str(), pkg))
        .collect();
    entries.sort_by_key(|(name, _)| *name);

    if entries.is_empty() {
        return "No packages configured.\n".to_string();
    }

    let name_width = entries
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);

    let s = Styles {
        on: opts.color_enabled,
    };
    let reset = s.reset();
    let mut out = String::new();
    let mut aggregate_packages: Vec<String> = Vec::new();
    // Marker + space precedes the name; description column starts after the
    // name column. Indent for continuation lines aligns under the description.
    let indent = " ".repeat(2 + name_width + 2);

    for (name, pkg) in entries {
        let (status_color, marker) = if pkg.is_disabled() {
            (s.dim(), "-")
        } else if pkg.is_installed(home) {
            (s.green(), "\u{2713}")
        } else {
            (s.red(), "\u{2717}")
        };

        let _ = write!(
            out,
            "{status_color}{marker}{reset} {bold}{name:<name_width$}{reset}",
            bold = s.bold(),
            name = name,
            name_width = name_width,
        );
        if let Some(desc) = pkg.description.as_deref() {
            let _ = write!(out, "  {dim}{desc}{reset}", dim = s.dim());
        }
        let _ = writeln!(out);

        if opts.verbose
            && let Some(d) = pkg.matched_detect(home)
        {
            let _ = writeln!(out, "{indent}{dim}detect: {d}{reset}", dim = s.dim());
        }

        if !pkg.is_installed(home) && !pkg.is_disabled() {
            let hint = s.hint();
            if opts.all_hints {
                for line in all_hints_lines(pkg) {
                    let _ = writeln!(out, "{indent}{hint}{line}{reset}");
                }
            } else if let Some(mgr) = manager {
                let pkgs = mgr.packages_for(&pkg.install_hint);
                if !pkgs.is_empty() {
                    let _ = writeln!(
                        out,
                        "{indent}{hint}via {mgr}: {list}{reset}",
                        mgr = mgr.name(),
                        list = pkgs.join(" "),
                    );
                    aggregate_packages.extend(pkgs.iter().cloned());
                }
            }
        }
    }

    if !opts.all_hints
        && let Some(mgr) = manager
        && !aggregate_packages.is_empty()
    {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{hint}To install all missing via {name}: {cmd}{reset}",
            hint = s.hint(),
            name = mgr.name(),
            cmd = mgr.install_command(&aggregate_packages),
        );
    }

    out
}

fn all_hints_lines(pkg: &PkgConfig) -> Vec<String> {
    let hint = &pkg.install_hint;
    let mut lines = Vec::new();
    if !hint.brew.packages.is_empty() {
        lines.push(format!("brew: {}", hint.brew.packages.join(" ")));
    }
    // Extend here as new managers land on `InstallHint`.
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::pkg::{BrewHint, DetectStrategy, InstallHint, PkgEnable, StoredPkgConfig};
    use smol_str::SmolStr;

    fn pkg(name: &str, stored: StoredPkgConfig) -> (SmolStr, PkgConfig) {
        let resolved = stored.resolve(&SmolStr::new(name)).unwrap();
        (SmolStr::new(name), resolved)
    }

    fn stored_with_brew(packages: &[&str]) -> StoredPkgConfig {
        StoredPkgConfig {
            install_hint: InstallHint {
                brew: BrewHint {
                    packages: packages.iter().map(|p| (*p).to_string()).collect(),
                },
            },
            ..Default::default()
        }
    }

    #[test]
    fn all_hints_lines_lists_every_manager_with_packages() {
        let (_, p) = pkg("sk", stored_with_brew(&["sk"]));
        assert_eq!(all_hints_lines(&p), vec!["brew: sk".to_string()]);
    }

    #[test]
    fn all_hints_lines_skips_managers_with_empty_packages() {
        let (_, p) = pkg("ghost", stored_with_brew(&[]));
        assert!(all_hints_lines(&p).is_empty());
    }

    #[test]
    fn matched_detect_returns_first_matching_strategy() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let marker = home.join(".marker");
        std::fs::write(&marker, "").unwrap();

        let (_, p) = pkg(
            "foo",
            StoredPkgConfig {
                enable: PkgEnable::Detect,
                detect: vec![DetectStrategy::File {
                    path: marker.to_string_lossy().into_owned(),
                }],
                ..stored_with_brew(&["foo"])
            },
        );

        assert!(p.is_installed(home));
        assert!(p.matched_detect(home).is_some());
    }
}
