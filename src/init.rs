//! Implementation of `zenops init`.
//!
//! `init` has two forms:
//!
//! - `zenops init <url>` — *clone* form. Pre-flights the target directory,
//!   calls [`Git::clone_to`], then either loads the freshly-cloned config and
//!   emits an [`InitSummary`], or hands off to `Apply` when `--apply` is set.
//!   The clone path may run on top of an empty existing
//!   `~/.config/zenops` (it's removed before the clone).
//!
//! - `zenops init` — *bootstrap* form. Interactively prompts for shell,
//!   name, and email; writes a minimal `config.toml`; runs `git init`; and
//!   makes the initial commit. The bootstrap path is strict: it refuses to
//!   run if `~/.config/zenops` already exists (even empty), and it requires
//!   a TTY for prompts.
//!
//! Both forms run *before* a `config.toml` exists, so they can't go through
//! the normal [`Config::load`] path in [`crate::real_main`].

use std::{
    fs,
    io::{self, IsTerminal},
};

use xshell::{Shell, cmd};

use crate::{
    Args, Cmd,
    config::Config,
    config_files::ConfigFileDirs,
    error::Error,
    git::Git,
    line_prompter::{LineOutcome, LinePrompter, RustylinePrompter},
    output::{BootstrapSummary, InitSummary, Output},
    real_main,
};

pub fn run(
    url: Option<&str>,
    branch: Option<&str>,
    apply: bool,
    yes: bool,
    dirs: &ConfigFileDirs,
    args: &Args,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let sh = Shell::new().unwrap();

    match url {
        Some(url) => run_clone(url, branch, apply, yes, dirs, args, &sh, output),
        None => run_bootstrap(apply, yes, dirs, args, &sh, output),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_clone(
    url: &str,
    branch: Option<&str>,
    apply: bool,
    yes: bool,
    dirs: &ConfigFileDirs,
    args: &Args,
    sh: &Shell,
    output: &mut dyn Output,
) -> Result<(), Error> {
    preflight_clone(dirs)?;

    Git::clone_to(url, dirs.zenops(), branch, sh)?;

    let config = Config::load(dirs, sh, false).map_err(|e| match e {
        Error::OpenDb(_, io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
            Error::InitNoConfigToml(dirs.zenops().to_path_buf())
        }
        other => other,
    })?;

    if apply {
        // Hand off to apply: the apply event stream is the contract here,
        // so suppress the init summary (it would just be noise before a
        // structured apply log).
        return apply_handoff(yes, dirs, args, output);
    }

    emit_clone_summary(dirs, sh, &config, output)
}

fn run_bootstrap(
    apply: bool,
    yes: bool,
    dirs: &ConfigFileDirs,
    args: &Args,
    sh: &Shell,
    output: &mut dyn Output,
) -> Result<(), Error> {
    preflight_bootstrap(dirs)?;
    if !io::stdin().is_terminal() {
        return Err(Error::InitNeedsTty);
    }

    let detected_shell = detect_shell_from_env(std::env::var("SHELL").ok().as_deref());
    let detected_name = detect_git_config(sh, "user.name");
    let detected_email = detect_git_config(sh, "user.email");

    let mut prompter = RustylinePrompter::new().map_err(Error::PromptRead)?;

    let shell = prompt_shell(&mut prompter, detected_shell)?;
    let name = prompt_with_default(&mut prompter, "Name", detected_name.as_deref())?;
    let email = prompt_with_default(&mut prompter, "Email", detected_email.as_deref())?;

    let zenops_dir = dirs.zenops();
    fs::create_dir_all(zenops_dir).map_err(|e| Error::InitIo(zenops_dir.to_path_buf(), e))?;

    let body = render_bootstrap_config(shell, name.as_deref(), email.as_deref());
    let cfg_path = zenops_dir.join("config.toml");
    fs::write(&cfg_path, &body).map_err(|e| Error::InitIo(cfg_path.clone(), e))?;

    Git::init_repo(zenops_dir, sh)?;
    Git::initial_commit(zenops_dir, sh, "Initial zenops config")?;

    if apply {
        // Same rationale as the clone path: the apply event stream is the
        // contract, so suppress the bootstrap summary.
        return apply_handoff(yes, dirs, args, output);
    }

    output.push_bootstrap_summary(BootstrapSummary {
        repo_path: zenops_dir.to_path_buf(),
        shell: shell.map(|s| s.to_string()),
        name,
        email,
    })?;
    Ok(())
}

fn apply_handoff(
    yes: bool,
    dirs: &ConfigFileDirs,
    args: &Args,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let apply_cmd = Cmd::Apply {
        pull_config: false,
        yes,
        dry_run: false,
        allow_dirty: false,
    };
    real_main(args, &apply_cmd, dirs, output)
}

fn preflight_clone(dirs: &ConfigFileDirs) -> Result<(), Error> {
    let zenops_dir = dirs.zenops();
    match fs::read_dir(zenops_dir) {
        Ok(mut iter) => {
            if iter.next().is_some() {
                return Err(Error::InitDirNotEmpty(zenops_dir.to_path_buf()));
            }
            fs::remove_dir(zenops_dir).map_err(|e| Error::InitIo(zenops_dir.to_path_buf(), e))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = zenops_dir.parent() {
                fs::create_dir_all(parent).map_err(|e| Error::InitIo(parent.to_path_buf(), e))?;
            }
        }
        Err(e) => return Err(Error::InitIo(zenops_dir.to_path_buf(), e)),
    }
    Ok(())
}

fn preflight_bootstrap(dirs: &ConfigFileDirs) -> Result<(), Error> {
    let zenops_dir = dirs.zenops();
    if zenops_dir.exists() {
        // Distinguish .git so the user gets a more specific message when
        // they're effectively re-initing an existing zenops repo.
        if zenops_dir.join(".git").exists() {
            return Err(Error::InitGitDirExists(zenops_dir.to_path_buf()));
        }
        return Err(Error::InitDirExists(zenops_dir.to_path_buf()));
    }
    if let Some(parent) = zenops_dir.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::InitIo(parent.to_path_buf(), e))?;
    }
    Ok(())
}

fn emit_clone_summary(
    dirs: &ConfigFileDirs,
    sh: &Shell,
    config: &Config<'_>,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let dest = dirs.zenops();
    let remote = cmd!(sh, "git -C {dest} remote get-url origin")
        .quiet()
        .ignore_stderr()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let shell = config.shell().map(|s| match s {
        crate::config::pkg::Shell::Bash => "bash".to_string(),
        crate::config::pkg::Shell::Zsh => "zsh".to_string(),
    });

    output.push_init_summary(InitSummary {
        clone_path: dest.to_path_buf(),
        remote,
        shell,
        pkg_count: config.pkgs().len(),
    })?;
    Ok(())
}

/// The two shells the bootstrap config writer knows how to spell. Anything
/// else (fish, nu, csh, …) maps to "no shell configured" — the user can
/// fill that in by hand later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapShell {
    Bash,
    Zsh,
}

impl BootstrapShell {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
        }
    }
}

impl std::fmt::Display for BootstrapShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn detect_shell_from_env(shell_var: Option<&str>) -> Option<BootstrapShell> {
    let path = shell_var?;
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    match name {
        "bash" => Some(BootstrapShell::Bash),
        "zsh" => Some(BootstrapShell::Zsh),
        _ => None,
    }
}

fn detect_git_config(sh: &Shell, key: &str) -> Option<String> {
    cmd!(sh, "git config --global --get {key}")
        .quiet()
        .ignore_status()
        .ignore_stderr()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_trimmed_line(
    prompter: &mut dyn LinePrompter,
    prompt: &str,
) -> Result<Option<String>, Error> {
    match prompter.read_line(prompt).map_err(Error::PromptRead)? {
        // EOF — treat as blank input so the caller falls back to its
        // default; the TTY check at entry should normally prevent us
        // from getting here at all.
        LineOutcome::Eof => Ok(None),
        LineOutcome::Interrupted => Err(Error::PromptInterrupted),
        LineOutcome::Line(line) => {
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
    }
}

fn prompt_with_default(
    prompter: &mut dyn LinePrompter,
    label: &str,
    default: Option<&str>,
) -> Result<Option<String>, Error> {
    let prompt = match default {
        Some(d) => format!("{label} [{d}]: "),
        None => format!("{label}: "),
    };
    match read_trimmed_line(prompter, &prompt)? {
        Some(answer) => Ok(Some(answer)),
        None => Ok(default.map(str::to_string)),
    }
}

fn prompt_shell(
    prompter: &mut dyn LinePrompter,
    default: Option<BootstrapShell>,
) -> Result<Option<BootstrapShell>, Error> {
    loop {
        let prompt = match default {
            Some(d) => format!("Shell (bash/zsh/none) [{d}]: "),
            None => "Shell (bash/zsh/none) [none]: ".to_string(),
        };
        let answer = read_trimmed_line(prompter, &prompt)?;
        let normalized = answer.as_deref().map(str::to_ascii_lowercase);

        match normalized.as_deref() {
            None => return Ok(default),
            Some("bash") => return Ok(Some(BootstrapShell::Bash)),
            Some("zsh") => return Ok(Some(BootstrapShell::Zsh)),
            Some("none") => return Ok(None),
            Some(_) => {
                prompter
                    .writeln("Please answer bash, zsh, or none.")
                    .map_err(Error::PromptRead)?;
            }
        }
    }
}

fn render_bootstrap_config(
    shell: Option<BootstrapShell>,
    name: Option<&str>,
    email: Option<&str>,
) -> String {
    let mut body = String::new();

    if let Some(s) = shell {
        body.push_str("[shell]\n");
        body.push_str(&format!("type = \"{}\"\n", s.as_str()));
    }

    if name.is_some() || email.is_some() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("[user]\n");
        if let Some(n) = name {
            body.push_str(&format!("name = {}\n", toml_string(n)));
        }
        if let Some(e) = email {
            body.push_str(&format!("email = {}\n", toml_string(e)));
        }
    }

    body
}

/// Serialize a string as a TOML basic string. Escapes `\` and `"`; control
/// characters in user-supplied identity strings are extremely unlikely
/// here, so we don't try to be more clever than that.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_shell_from_env_bash() {
        assert_eq!(
            detect_shell_from_env(Some("/bin/bash")),
            Some(BootstrapShell::Bash)
        );
    }

    #[test]
    fn detect_shell_from_env_zsh() {
        assert_eq!(
            detect_shell_from_env(Some("/usr/bin/zsh")),
            Some(BootstrapShell::Zsh)
        );
    }

    #[test]
    fn detect_shell_from_env_other() {
        assert_eq!(detect_shell_from_env(Some("/usr/local/bin/fish")), None);
        assert_eq!(detect_shell_from_env(Some("")), None);
        assert_eq!(detect_shell_from_env(None), None);
    }

    #[test]
    fn render_empty_config() {
        assert_eq!(render_bootstrap_config(None, None, None), "");
    }

    #[test]
    fn render_shell_only() {
        assert_eq!(
            render_bootstrap_config(Some(BootstrapShell::Bash), None, None),
            "[shell]\ntype = \"bash\"\n"
        );
    }

    #[test]
    fn render_user_only() {
        assert_eq!(
            render_bootstrap_config(None, Some("Alice"), Some("a@example.com")),
            "[user]\nname = \"Alice\"\nemail = \"a@example.com\"\n"
        );
    }

    #[test]
    fn render_full() {
        assert_eq!(
            render_bootstrap_config(
                Some(BootstrapShell::Zsh),
                Some("Alice"),
                Some("a@example.com"),
            ),
            "[shell]\ntype = \"zsh\"\n\n[user]\nname = \"Alice\"\nemail = \"a@example.com\"\n"
        );
    }

    #[test]
    fn render_name_only() {
        assert_eq!(
            render_bootstrap_config(None, Some("Alice"), None),
            "[user]\nname = \"Alice\"\n"
        );
    }

    #[test]
    fn toml_string_escapes_quotes_and_backslashes() {
        assert_eq!(toml_string(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    use crate::line_prompter::BufReadPrompter;

    #[test]
    fn prompt_with_default_blank_line_uses_default() {
        let input = b"\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let answer = prompt_with_default(&mut prompter, "Name", Some("Alice")).unwrap();
        assert_eq!(answer, Some("Alice".to_string()));
    }

    #[test]
    fn prompt_with_default_explicit_input_overrides() {
        let input = b"Bob\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let answer = prompt_with_default(&mut prompter, "Name", Some("Alice")).unwrap();
        assert_eq!(answer, Some("Bob".to_string()));
    }

    #[test]
    fn prompt_with_default_no_default_blank_returns_none() {
        let input = b"\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let answer = prompt_with_default(&mut prompter, "Email", None).unwrap();
        assert_eq!(answer, None);
    }

    #[test]
    fn prompt_shell_accepts_named_choice() {
        let input = b"bash\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let answer = prompt_shell(&mut prompter, None).unwrap();
        assert_eq!(answer, Some(BootstrapShell::Bash));
    }

    #[test]
    fn prompt_shell_blank_uses_default() {
        let input = b"\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let answer = prompt_shell(&mut prompter, Some(BootstrapShell::Zsh)).unwrap();
        assert_eq!(answer, Some(BootstrapShell::Zsh));
    }

    #[test]
    fn prompt_shell_none_keyword_clears_default() {
        let input = b"none\n";
        let mut prompter = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let answer = prompt_shell(&mut prompter, Some(BootstrapShell::Bash)).unwrap();
        assert_eq!(answer, None);
    }

    #[test]
    fn prompt_shell_rejects_invalid_then_accepts() {
        let input = b"fish\nzsh\n";
        let mut output = Vec::<u8>::new();
        {
            let mut prompter = BufReadPrompter::new(&input[..], &mut output);
            let answer = prompt_shell(&mut prompter, None).unwrap();
            assert_eq!(answer, Some(BootstrapShell::Zsh));
        }
        let written = String::from_utf8(output).unwrap();
        assert!(written.contains("Please answer bash, zsh, or none."));
    }
}
