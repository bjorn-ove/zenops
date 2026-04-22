use std::{fmt::Write, io::IsTerminal};

use crate::{
    ColorChoice,
    ansi::Styler,
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
            color_enabled: color.enabled(std::io::stdout().is_terminal()),
        },
    ))
}

fn hint_prefix(s: &Styler) -> &'static str {
    s.bold_yellow()
}

pub fn render(config: &Config, opts: Options) -> String {
    let manager = pkg_manager::detect();

    let home = config.home();
    let configured_shell = config.shell();
    // Entries carry (display_label, pkg). The map key is only used to look up
    // the pkg; OS / shell filtering and `name` override happen here so the
    // rendered output matches what actually applies on this host.
    let mut entries: Vec<(&str, &PkgConfig)> = config
        .pkgs()
        .iter()
        .filter(|(_, p)| opts.all || !p.is_disabled())
        .filter(|(_, p)| p.supports_current_os())
        .filter(|(_, p)| p.supports_shell(configured_shell))
        .map(|(key, pkg)| (pkg.name.as_deref().unwrap_or(key.as_str()), pkg))
        .collect();
    entries.sort_by_key(|(label, _)| *label);

    if entries.is_empty() {
        return "No packages configured.\n".to_string();
    }

    let name_width = entries
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(0);

    let s = Styler::new(opts.color_enabled);
    let reset = s.reset();
    let mut out = String::new();
    let mut aggregate_packages: Vec<String> = Vec::new();
    // Marker + space precedes the name; description column starts after the
    // name column. Indent for continuation lines aligns under the description.
    let indent = " ".repeat(2 + name_width + 2);

    for (label, pkg) in entries {
        let (status_color, marker) = if pkg.is_disabled() {
            (s.dim(), "-")
        } else if pkg.is_installed(home, config.system_inputs()) {
            (s.green(), "\u{2713}")
        } else {
            (s.red(), "\u{2717}")
        };

        let _ = write!(
            out,
            "{status_color}{marker}{reset} {bold}{label:<name_width$}{reset}",
            bold = s.bold(),
            label = label,
            name_width = name_width,
        );
        if let Some(desc) = pkg.description.as_deref() {
            let _ = write!(out, "  {dim}{desc}{reset}", dim = s.dim());
        }
        let _ = writeln!(out);

        if opts.verbose
            && let Some(d) = pkg.matched_detect(home, config.system_inputs())
        {
            let _ = writeln!(out, "{indent}{dim}detect: {d}{reset}", dim = s.dim());
        }

        if !pkg.is_installed(home, config.system_inputs()) && !pkg.is_disabled() {
            let hint = hint_prefix(&s);
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
            hint = hint_prefix(&s),
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
    use indexmap::IndexMap;
    use smol_str::SmolStr;

    fn pkg_with_brew(packages: &[&str]) -> PkgConfig {
        let toml_src = format!(
            r#"
            [install_hint.brew]
            packages = [{}]
            "#,
            packages
                .iter()
                .map(|p| format!("\"{p}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        toml::from_str(&toml_src).unwrap()
    }

    #[test]
    fn all_hints_lines_lists_every_manager_with_packages() {
        let p = pkg_with_brew(&["sk"]);
        assert_eq!(all_hints_lines(&p), vec!["brew: sk".to_string()]);
    }

    #[test]
    fn all_hints_lines_skips_managers_with_empty_packages() {
        let p = pkg_with_brew(&[]);
        assert!(all_hints_lines(&p).is_empty());
    }

    #[test]
    fn matched_detect_returns_first_matching_strategy() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let marker = home.join(".marker");
        std::fs::write(&marker, "").unwrap();

        let toml_src = format!(
            r#"
            enable = "detect"
            [install_hint.brew]
            packages = ["foo"]
            [detect]
            type = "file"
            path = "{}"
            "#,
            marker.display()
        );
        let p: PkgConfig = toml::from_str(&toml_src).unwrap();
        let system: IndexMap<SmolStr, SmolStr> = IndexMap::new();
        assert!(p.is_installed(home, &system));
        assert!(p.matched_detect(home, &system).is_some());
    }
}
