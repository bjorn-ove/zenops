//! `zenops doctor` — read-only environment diagnostic.
//!
//! Runs even when `config.toml` is missing or broken: those are the exact
//! moments the user needs the command most. Every `Config::load` error is
//! captured and rendered with actionable next steps instead of propagating,
//! so `doctor` can still report the rest of the environment. The pkg-health
//! section reuses the same `Status::Pkg` events as `zenops status`; the
//! narrative sections are written as plain text to stderr, matching the
//! style of `init::print_summary`.

use std::io::IsTerminal;

use xshell::{Shell, cmd};

use crate::{
    Args,
    ansi::Styler,
    config::{Config, pkg::which_on_path},
    config_files::ConfigFileDirs,
    error::Error,
    git::Git,
    output::Output,
    pkg_manager,
};

pub fn run(
    args: &Args,
    dirs: &ConfigFileDirs,
    sh: &Shell,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let color = args.color.enabled(std::io::stderr().is_terminal());
    let styler = Styler::new(color);

    system_block(&styler);
    repo_block(dirs, sh, &styler)?;
    let config = load_config_or_report(dirs, sh, &styler);
    pkg_manager_block(&styler);
    if let Some(config) = config.as_ref() {
        user_block(config, &styler);
        shell_block(config, &styler);
        packages_block(config, &styler, output)?;
    }
    Ok(())
}

fn section(styler: &Styler, title: &str) {
    eprintln!("{}{}{}", styler.bold(), title, styler.reset());
}

fn ok(styler: &Styler, label: &str, value: &str) {
    eprintln!(
        "  {label:<14} {}{}{}",
        styler.green(),
        value,
        styler.reset()
    );
}

fn info(label: &str, value: &str) {
    eprintln!("  {label:<14} {value}");
}

fn warn(styler: &Styler, label: &str, value: &str, hint: &str) {
    eprintln!(
        "  {label:<14} {}{}{}  {}{}{}",
        styler.yellow(),
        value,
        styler.reset(),
        styler.dim(),
        hint,
        styler.reset(),
    );
}

fn bad(styler: &Styler, label: &str, value: &str, hint: &str) {
    eprintln!(
        "  {label:<14} {}{}{}  {}{}{}",
        styler.red(),
        value,
        styler.reset(),
        styler.dim(),
        hint,
        styler.reset(),
    );
}

fn blank() {
    eprintln!();
}

fn system_block(styler: &Styler) {
    section(styler, "System");
    info("os:", std::env::consts::OS);
    report_bin("git:", "git", styler, "required by zenops — install git");
    report_bin(
        "zenops:",
        "zenops",
        styler,
        "not on PATH — the install may be incomplete",
    );
    blank();
}

fn report_bin(label: &str, binary: &str, styler: &Styler, missing_hint: &str) {
    if which_on_path(binary) {
        ok(styler, label, "found on PATH");
    } else {
        bad(styler, label, "not found on PATH", missing_hint);
    }
}

fn repo_block(dirs: &ConfigFileDirs, sh: &Shell, styler: &Styler) -> Result<(), Error> {
    let zenops = dirs.zenops();
    section(styler, "Config repo (~/.config/zenops)");

    if !zenops.exists() {
        bad(
            styler,
            "path:",
            "missing",
            "run `zenops init <url>` to clone a config repo",
        );
        blank();
        return Ok(());
    }
    info("path:", &zenops.display().to_string());

    let git = Git::new(zenops, sh);
    if !git.is_git_repo()? {
        warn(
            styler,
            "git repo:",
            "no",
            "not a git repo — `zenops repo` commands will fail",
        );
        blank();
        return Ok(());
    }
    ok(styler, "git repo:", "yes");

    let remote = cmd!(sh, "git -C {zenops} remote get-url origin")
        .quiet()
        .ignore_stderr()
        .ignore_status()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match remote {
        Some(url) => info("remote:", &url),
        None => warn(
            styler,
            "remote:",
            "none",
            "add one with `git -C ~/.config/zenops remote add origin <url>`",
        ),
    }

    let branch = cmd!(sh, "git -C {zenops} rev-parse --abbrev-ref HEAD")
        .quiet()
        .ignore_stderr()
        .ignore_status()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD");
    if let Some(b) = branch {
        info("branch:", &b);
    }

    if git.has_uncommitted_changes()? {
        warn(
            styler,
            "uncommitted:",
            "yes",
            "`zenops apply` will offer to commit; `zenops repo diff` to review",
        );
    } else {
        ok(styler, "uncommitted:", "none");
    }
    blank();
    Ok(())
}

/// Try to load `config.toml`. On every failure mode, render a user-facing
/// explanation with a next step and return `None` so the rest of `doctor`
/// skips config-dependent checks. Only `Ok` keeps the function returning a
/// loaded `Config`. This is the deliberate inversion of the usual `?`
/// propagation: broken config is the subject of the diagnosis, not an
/// abort condition.
fn load_config_or_report<'d>(
    dirs: &'d ConfigFileDirs,
    sh: &Shell,
    styler: &Styler,
) -> Option<Config<'d>> {
    section(styler, "Config (~/.config/zenops/config.toml)");
    match Config::load(dirs, sh, false) {
        Ok(config) => {
            ok(styler, "status:", "loaded");
            blank();
            Some(config)
        }
        Err(Error::OpenDb(path, io)) if io.kind() == std::io::ErrorKind::NotFound => {
            bad(
                styler,
                "status:",
                "missing",
                &format!(
                    "no config.toml at {}. Run `zenops init <url>` to clone one.",
                    path.display(),
                ),
            );
            blank();
            None
        }
        Err(Error::OpenDb(path, io)) => {
            bad(
                styler,
                "status:",
                "unreadable",
                &format!("{}: {}. Check permissions.", path.display(), io),
            );
            blank();
            None
        }
        Err(Error::ParseDb(path, toml_err)) => {
            bad(styler, "status:", "parse error", "");
            eprintln!("    {}", path.display());
            // The toml crate's Display already carries line/column and a
            // caret span — indent each line so it nests under `status:`.
            for line in toml_err.to_string().lines() {
                eprintln!("    {line}");
            }
            let msg = toml_err.to_string();
            if msg.contains("unknown field") {
                eprintln!(
                    "    {}hint: check CHANGELOG.md for recent field renames.{}",
                    styler.dim(),
                    styler.reset(),
                );
            } else if msg.contains("invalid type") || msg.contains("missing field") {
                eprintln!(
                    "    {}hint: see README.md for the expected shape of [shell], [[pkg.*.configs]], and friends.{}",
                    styler.dim(),
                    styler.reset(),
                );
            }
            blank();
            None
        }
        Err(err) => {
            // UnresolvedInput / TemplateUnterminated / SafeRelativePath /
            // Shell: their `#[error]` messages are already user-targeted, so
            // don't re-wrap them.
            bad(styler, "status:", "config failed to load", "");
            eprintln!("    {err}");
            blank();
            None
        }
    }
}

fn pkg_manager_block(styler: &Styler) {
    section(styler, "Package manager");
    match pkg_manager::detect() {
        Some(mgr) => ok(styler, "detected:", mgr.name()),
        None => warn(
            styler,
            "detected:",
            "none",
            "install hints won't render; supported managers: brew",
        ),
    }
    blank();
}

fn user_block(config: &Config<'_>, styler: &Styler) {
    section(styler, "User");
    let inputs = config.system_inputs();
    match inputs.get("user.name").map(|v| v.as_str()) {
        Some(name) => info("name:", name),
        None => warn(
            styler,
            "name:",
            "unset",
            "set [user].name in config.toml (used by the generated gitconfig)",
        ),
    }
    match inputs.get("user.email").map(|v| v.as_str()) {
        Some(email) => info("email:", email),
        None => warn(
            styler,
            "email:",
            "unset",
            "set [user].email in config.toml (used by the generated gitconfig)",
        ),
    }
    blank();
}

fn shell_block(config: &Config<'_>, styler: &Styler) {
    section(styler, "Shell");
    let env_shell = std::env::var("SHELL").ok();
    let env_basename = env_shell
        .as_deref()
        .and_then(|s| std::path::Path::new(s).file_name())
        .and_then(|s| s.to_str())
        .map(str::to_string);

    match env_shell.as_deref() {
        Some(full) => info("$SHELL:", full),
        None => warn(
            styler,
            "$SHELL:",
            "unset",
            "the OS should set $SHELL from /etc/passwd; check your user record",
        ),
    }

    let configured = config.shell().map(shell_name);
    match configured {
        Some(name) => info("config:", name),
        None => info("config:", "(none)"),
    }

    match (env_basename.as_deref(), configured) {
        (Some(env), Some(cfg)) if env == cfg => ok(styler, "match:", "yes"),
        (Some(env), Some(cfg)) => warn(
            styler,
            "match:",
            "no",
            &format!(
                "running {env} but config targets {cfg}. Change shell with `chsh -s $(which {cfg})` or update [shell].type",
            ),
        ),
        (Some(_), None) => info("match:", "n/a (no shell configured)"),
        (None, _) => {}
    }
    blank();
}

fn shell_name(shell: crate::config::pkg::Shell) -> &'static str {
    match shell {
        crate::config::pkg::Shell::Bash => "bash",
        crate::config::pkg::Shell::Zsh => "zsh",
    }
}

fn packages_block(
    config: &Config<'_>,
    styler: &Styler,
    output: &mut dyn Output,
) -> Result<(), Error> {
    section(styler, "Packages");
    config.push_pkg_health(output)?;
    blank();
    Ok(())
}
