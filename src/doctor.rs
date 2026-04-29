//! `zenops doctor` — read-only environment diagnostic.
//!
//! Runs even when `config.toml` is missing or broken: those are the exact
//! moments the user needs the command most. Every `Config::load` error is
//! captured and rendered with actionable next steps instead of propagating,
//! so `doctor` can still report the rest of the environment. The pkg-health
//! section reuses the same `Status::Pkg` events as `zenops status`; every
//! other section emits `DoctorCheck` events through `Output`, so `-o json`
//! gets the same structured stream as the rest of the CLI.

use smol_str::SmolStr;
use xshell::{Shell, cmd};

use crate::{
    Args,
    config::{Config, pkg::which_on_path},
    config_files::ConfigFileDirs,
    error::Error,
    git::Git,
    output::{DoctorCheck, DoctorSection, DoctorSeverity, Event, Output},
    pkg_manager,
};

pub fn run(
    _args: &Args,
    dirs: &ConfigFileDirs,
    sh: &Shell,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let mut em = DoctorEmitter::new(output);
    system_block(&mut em)?;
    repo_block(dirs, sh, &mut em)?;
    let config = load_config_or_report(dirs, sh, &mut em)?;
    pkg_manager_block(&mut em)?;
    if let Some(config) = config.as_ref() {
        user_block(config, &mut em)?;
        shell_block(config, &mut em)?;
        packages_block(config, &mut em)?;
    }
    Ok(())
}

/// Pushes `DoctorCheck` events through an `Output` while tracking the current
/// section so callers don't repeat themselves. The `severity == Info` helper
/// (`info`) deliberately omits a hint to match the original `info` formatter,
/// which only ever rendered `{label} {value}` with no trailing dim text.
struct DoctorEmitter<'o> {
    section: DoctorSection,
    out: &'o mut dyn Output,
}

impl<'o> DoctorEmitter<'o> {
    fn new(out: &'o mut dyn Output) -> Self {
        Self {
            // Placeholder; `enter` is always called before any push.
            section: DoctorSection::System,
            out,
        }
    }

    fn enter(&mut self, section: DoctorSection) -> Result<(), Error> {
        self.section = section;
        // Always emit a section header so the renderer can print a
        // bold title (and JSON skips the no-op event). Sections with at
        // least one row would also imply the header through the row's
        // `section` field, but Packages has no `DoctorCheck` rows of its
        // own — its content is `Status::Pkg` events from `push_pkg_health`.
        self.out
            .push(Event::DoctorCheck(DoctorCheck::SectionHeader { section }))?;
        Ok(())
    }

    fn push(
        &mut self,
        label: &str,
        severity: DoctorSeverity,
        value: impl Into<String>,
        hint: Option<String>,
        detail: Vec<String>,
    ) -> Result<(), Error> {
        self.out.push(Event::DoctorCheck(DoctorCheck::Check {
            section: self.section,
            label: SmolStr::new(label),
            severity,
            value: value.into(),
            hint,
            detail,
        }))?;
        Ok(())
    }

    fn ok(&mut self, label: &str, value: impl Into<String>) -> Result<(), Error> {
        self.push(label, DoctorSeverity::Ok, value, None, Vec::new())
    }

    fn info(&mut self, label: &str, value: impl Into<String>) -> Result<(), Error> {
        self.push(label, DoctorSeverity::Info, value, None, Vec::new())
    }

    fn warn(
        &mut self,
        label: &str,
        value: impl Into<String>,
        hint: impl Into<String>,
    ) -> Result<(), Error> {
        self.push(
            label,
            DoctorSeverity::Warn,
            value,
            Some(hint.into()),
            Vec::new(),
        )
    }

    fn bad(
        &mut self,
        label: &str,
        value: impl Into<String>,
        hint: impl Into<String>,
    ) -> Result<(), Error> {
        self.push(
            label,
            DoctorSeverity::Bad,
            value,
            Some(hint.into()),
            Vec::new(),
        )
    }

    fn bad_with_detail(
        &mut self,
        label: &str,
        value: impl Into<String>,
        detail: Vec<String>,
    ) -> Result<(), Error> {
        self.push(label, DoctorSeverity::Bad, value, None, detail)
    }
}

fn system_block(em: &mut DoctorEmitter) -> Result<(), Error> {
    em.enter(DoctorSection::System)?;
    em.info("os:", std::env::consts::OS)?;
    report_bin(em, "git:", "git", "required by zenops — install git")?;
    report_bin(
        em,
        "zenops:",
        "zenops",
        "not on PATH — the install may be incomplete",
    )?;
    Ok(())
}

fn report_bin(
    em: &mut DoctorEmitter,
    label: &str,
    binary: &str,
    missing_hint: &str,
) -> Result<(), Error> {
    if which_on_path(binary) {
        em.ok(label, "found on PATH")
    } else {
        em.bad(label, "not found on PATH", missing_hint)
    }
}

fn repo_block(dirs: &ConfigFileDirs, sh: &Shell, em: &mut DoctorEmitter) -> Result<(), Error> {
    em.enter(DoctorSection::Repo)?;
    let zenops = dirs.zenops();

    if !zenops.exists() {
        em.bad(
            "path:",
            "missing",
            "run `zenops init <url>` to clone a config repo",
        )?;
        return Ok(());
    }
    em.info("path:", zenops.display().to_string())?;

    let git = Git::new(zenops, sh);
    if !git.is_git_repo()? {
        em.warn(
            "git repo:",
            "no",
            "not a git repo — `zenops repo` commands will fail",
        )?;
        return Ok(());
    }
    em.ok("git repo:", "yes")?;

    let remote = cmd!(sh, "git -C {zenops} remote get-url origin")
        .quiet()
        .ignore_stderr()
        .ignore_status()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match remote {
        Some(url) => em.info("remote:", url)?,
        None => em.warn(
            "remote:",
            "none",
            "add one with `git -C ~/.config/zenops remote add origin <url>`",
        )?,
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
        em.info("branch:", b)?;
    }

    if git.has_uncommitted_changes()? {
        em.warn(
            "uncommitted:",
            "yes",
            "`zenops apply` will offer to commit; `zenops repo diff` to review",
        )?;
    } else {
        em.ok("uncommitted:", "none")?;
    }
    Ok(())
}

/// Try to load `config.toml`. On every failure mode, push a `DoctorCheck`
/// explaining the failure and return `Ok(None)` so the rest of `doctor`
/// skips config-dependent checks. Only `Ok` returns a loaded `Config`.
/// This is the deliberate inversion of the usual `?` propagation: broken
/// config is the subject of the diagnosis, not an abort condition.
fn load_config_or_report<'d>(
    dirs: &'d ConfigFileDirs,
    sh: &Shell,
    em: &mut DoctorEmitter,
) -> Result<Option<Config<'d>>, Error> {
    em.enter(DoctorSection::Config)?;
    match Config::load(dirs, sh, false) {
        Ok(config) => {
            em.ok("status:", "loaded")?;
            Ok(Some(config))
        }
        Err(Error::OpenDb(path, io)) if io.kind() == std::io::ErrorKind::NotFound => {
            em.bad(
                "status:",
                "missing",
                format!(
                    "no config.toml at {}. Run `zenops init <url>` to clone one.",
                    path.display(),
                ),
            )?;
            Ok(None)
        }
        Err(Error::OpenDb(path, io)) => {
            em.bad(
                "status:",
                "unreadable",
                format!("{}: {}. Check permissions.", path.display(), io),
            )?;
            Ok(None)
        }
        Err(Error::ParseDb(path, toml_err)) => {
            // The toml crate's Display already carries line/column and a
            // caret span; ship those as `detail` lines so the renderer
            // can indent them under the row.
            let mut detail = vec![path.display().to_string()];
            for line in toml_err.to_string().lines() {
                detail.push(line.to_string());
            }
            let msg = toml_err.to_string();
            if msg.contains("unknown field") {
                detail.push("hint: check CHANGELOG.md for recent field renames.".to_string());
            } else if msg.contains("invalid type") || msg.contains("missing field") {
                detail.push(
                    "hint: see README.md for the expected shape of [shell], [[pkg.*.configs]], and friends."
                        .to_string(),
                );
            }
            em.bad_with_detail("status:", "parse error", detail)?;
            Ok(None)
        }
        Err(err) => {
            // UnresolvedInput / TemplateUnterminated / SafeRelativePath /
            // Shell: their `#[error]` messages are already user-targeted, so
            // don't re-wrap them.
            em.bad_with_detail("status:", "config failed to load", vec![err.to_string()])?;
            Ok(None)
        }
    }
}

fn pkg_manager_block(em: &mut DoctorEmitter) -> Result<(), Error> {
    em.enter(DoctorSection::PkgManager)?;
    match pkg_manager::detect() {
        Some(mgr) => em.ok("detected:", mgr.name())?,
        None => em.warn(
            "detected:",
            "none",
            "install hints won't render; supported managers: brew",
        )?,
    }
    Ok(())
}

fn user_block(config: &Config<'_>, em: &mut DoctorEmitter) -> Result<(), Error> {
    em.enter(DoctorSection::User)?;
    let inputs = config.system_inputs();
    match inputs.get("user.name").map(|v| v.as_str()) {
        Some(name) => em.info("name:", name)?,
        None => em.warn(
            "name:",
            "unset",
            "set [user].name in config.toml (used by the generated gitconfig)",
        )?,
    }
    match inputs.get("user.email").map(|v| v.as_str()) {
        Some(email) => em.info("email:", email)?,
        None => em.warn(
            "email:",
            "unset",
            "set [user].email in config.toml (used by the generated gitconfig)",
        )?,
    }
    Ok(())
}

fn shell_block(config: &Config<'_>, em: &mut DoctorEmitter) -> Result<(), Error> {
    em.enter(DoctorSection::Shell)?;
    let env_shell = std::env::var("SHELL").ok();
    let env_basename = env_shell
        .as_deref()
        .and_then(|s| std::path::Path::new(s).file_name())
        .and_then(|s| s.to_str())
        .map(str::to_string);

    match env_shell.as_deref() {
        Some(full) => em.info("$SHELL:", full)?,
        None => em.warn(
            "$SHELL:",
            "unset",
            "the OS should set $SHELL from /etc/passwd; check your user record",
        )?,
    }

    let configured = config.shell().map(shell_name);
    match configured {
        Some(name) => em.info("config:", name)?,
        None => em.info("config:", "(none)")?,
    }

    match (env_basename.as_deref(), configured) {
        (Some(env), Some(cfg)) if env == cfg => em.ok("match:", "yes")?,
        (Some(env), Some(cfg)) => em.warn(
            "match:",
            "no",
            format!(
                "running {env} but config targets {cfg}. Change shell with `chsh -s $(which {cfg})` or update [shell].type",
            ),
        )?,
        (Some(_), None) => em.info("match:", "n/a (no shell configured)")?,
        (None, _) => {}
    }
    Ok(())
}

fn shell_name(shell: crate::config::pkg::Shell) -> &'static str {
    match shell {
        crate::config::pkg::Shell::Bash => "bash",
        crate::config::pkg::Shell::Zsh => "zsh",
    }
}

fn packages_block(config: &Config<'_>, em: &mut DoctorEmitter) -> Result<(), Error> {
    em.enter(DoctorSection::Packages)?;
    config.push_pkg_health(em.out)?;
    Ok(())
}
