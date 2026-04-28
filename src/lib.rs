//! Library root for the `zenops` binary.
//!
//! Exposes the clap [`Cmd`] subcommand enum, the global [`Args`], the
//! [`ColorChoice`] resolver, and the [`real_main`] dispatcher that routes a
//! parsed command into the right module.
//!
//! `Init`, `Doctor`, and `Schema` are dispatched *before* `Config::load`
//! because they must work without — or independently of — a usable
//! `~/.config/zenops/config.toml`. Every other command goes through
//! `Config::load` first.
//!
//! See [`crate::output`] for the structured-event channel that all commands
//! emit through; the `zenops` binary entrypoint wires up `Cli` and picks a
//! renderer.

#![deny(missing_docs)]

mod ansi;
mod config;
pub mod config_files;
mod doctor;
pub mod error;
pub mod git;
mod init;
pub mod line_prompter;
pub mod output;
pub mod pkg_list;
pub mod pkg_manager;
pub mod prompt;
pub mod schema;

use std::io::IsTerminal;

use clap::Subcommand;
use xshell::Shell;

use crate::{
    config::Config,
    config_files::{ConfigFileDirs, ConfigFiles},
    error::Error,
    git::{Git, GitCmd},
    output::Output,
    prompt::{DryRunPrompter, PreApplyDecision, Prompter, TerminalPrompter, YesPrompter},
};

/// User-facing color policy for the renderer and prompter, parsed from
/// `--color`. Resolve to a concrete on/off via [`ColorChoice::enabled`];
/// `Auto` honours `NO_COLOR` and the target stream's TTY-ness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum ColorChoice {
    /// Color when the target stream is a TTY and `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Force color regardless of TTY or `NO_COLOR`.
    Always,
    /// Never emit ANSI escapes.
    Never,
}

impl ColorChoice {
    /// Resolve to a concrete on/off decision. Pass `stream_is_terminal`
    /// for the stream colors will actually be emitted to. Everything
    /// `Output`-driven (the renderer and the prompter) writes to stdout;
    /// only `log::*!` and the top-level fatal-error `eprintln!` go to
    /// stderr, so callers almost always pass `stdout().is_terminal()`.
    pub fn enabled(self, stream_is_terminal: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && stream_is_terminal,
        }
    }
}

/// Globals shared across every subcommand. Currently only `--color`; lives
/// in its own struct so subcommands can borrow it without redeclaring the
/// flag.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// When to colorize output
    #[clap(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}

/// Top-level subcommand. The variants map 1:1 to user-visible commands;
/// each is dispatched by [`real_main`] (or, for `Completions`, by `main`
/// before `real_main` is reached).
#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Reconcile the live system with `config.toml`: prompt per change
    /// (unless `--yes`), write generated files, create symlinks, run shell
    /// init for any `pkg` that needs it. Honours the zenops repo's git
    /// state — a dirty repo prompts for commit-and-push or aborts under
    /// `--yes` without `--allow-dirty`.
    Apply {
        /// Pull the latest version of the config using git pull --rebase in the zenops config directory
        #[clap(long, short)]
        pull_config: bool,
        /// Apply every change without prompting.
        #[clap(long, short = 'y', conflicts_with = "dry_run")]
        yes: bool,
        /// Show each prompt with its diff, but apply nothing.
        #[clap(long, short = 'n')]
        dry_run: bool,
        /// Proceed even when the zenops config repo has uncommitted changes.
        /// Required alongside `--yes` when the repo is dirty; without it,
        /// `--yes` on a dirty repo aborts so automation surfaces divergence
        /// instead of silently applying uncommitted state.
        #[clap(long)]
        allow_dirty: bool,
    },
    /// Read-only sibling of `Apply`: report what would change without
    /// touching the filesystem.
    Status {
        /// Show a diff of what would change
        #[clap(long, short = 'd')]
        diff: bool,
        /// Also list items that already match the desired state
        #[clap(long, short = 'a')]
        all: bool,
    },
    /// List every configured package and whether its dependencies are met
    Pkg {
        /// Only list packages whose name or key contains one of these
        /// substrings (case-insensitive). Multiple patterns are ORed —
        /// `zenops pkg git curl` shows both.
        #[clap(value_name = "PATTERN")]
        pattern: Vec<String>,
        /// Include packages with `enable = "disabled"`
        #[clap(long)]
        all: bool,
        /// Show every install hint, not just the one for the detected package manager
        #[clap(long)]
        all_hints: bool,
        /// Show diagnostic details (the detect strategy that matched)
        #[clap(long, short)]
        verbose: bool,
    },
    /// Git pass-through against the zenops config repo
    /// (`~/.config/zenops`). Lets the user run common git operations
    /// without `cd`-ing.
    Repo {
        /// Which git operation to dispatch in the config repo.
        #[command(subcommand)]
        command: GitCmd,
    },
    /// Set up `~/.config/zenops`. With a URL, clones an existing zenops
    /// config repo and validates it has a `config.toml`. Without a URL,
    /// bootstraps a brand-new repo on disk by interactively prompting for
    /// shell, name, and email, writing a minimal `config.toml`, and making
    /// the initial commit. The bootstrap form refuses to run if
    /// `~/.config/zenops` already exists (even empty); the clone form
    /// allows an empty target. Authentication (SSH key, HTTPS credential
    /// helper) uses whatever git is already configured to use.
    Init {
        /// Git URL to clone (SSH or HTTPS). Passed verbatim to `git clone`.
        /// Omit to bootstrap a fresh repo at `~/.config/zenops` instead of
        /// cloning.
        url: Option<String>,
        /// Check out this branch or tag after cloning (default: remote's HEAD).
        /// Only valid with a URL.
        #[clap(long, short, requires = "url")]
        branch: Option<String>,
        /// After cloning, run `zenops apply`.
        #[clap(long)]
        apply: bool,
        /// With `--apply`, apply every change without prompting (equivalent
        /// to `zenops apply --yes`). Only meaningful together with `--apply`.
        #[clap(long, short = 'y', requires = "apply")]
        yes: bool,
    },
    /// Diagnose the local environment: config dir, git, shell, package
    /// manager, and package health. Read-only; keeps running even when
    /// `config.toml` is missing or fails to parse, so it stays useful on a
    /// broken machine.
    Doctor,
    /// Dump JSON Schema for every structured surface (command output events
    /// and the `config.toml` input) as a single bundle to stdout. The schema
    /// shape is versioned under the zenops crate version embedded in the
    /// bundle.
    Schema,
    /// Print a shell completion script for zenops to stdout.
    ///
    /// Normally sourced automatically by the built-in `zenops` pkg; you
    /// don't need to invoke this by hand.
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

impl Cmd {
    fn should_update_self(&self, _args: &Args) -> bool {
        match self {
            Cmd::Apply { pull_config, .. } => *pull_config,
            Cmd::Status { .. }
            | Cmd::Pkg { .. }
            | Cmd::Repo { .. }
            | Cmd::Init { .. }
            | Cmd::Doctor
            | Cmd::Schema
            | Cmd::Completions { .. } => false,
        }
    }
}

fn build_prompter(yes: bool, dry_run: bool, color: bool) -> Result<Box<dyn Prompter>, Error> {
    if dry_run {
        Ok(Box::new(DryRunPrompter::new(color)))
    } else if yes {
        Ok(Box::new(YesPrompter))
    } else if std::io::stdin().is_terminal() {
        Ok(Box::new(TerminalPrompter::new(color)?))
    } else {
        Err(Error::ApplyNeedsYesOrTty)
    }
}

/// Dispatch a parsed [`Cmd`] to its module. `Init`, `Doctor`, and
/// `Schema` are handled before `Config::load` so they remain usable on a
/// fresh or broken machine; everything else loads the config first and
/// then routes through `Config` / [`ConfigFiles`].
///
/// `Completions` is a no-op here — `main` handles it before calling in,
/// because it needs the top-level `Cli` for clap's `CommandFactory`.
pub fn real_main(
    args: &Args,
    command: &Cmd,
    dirs: &ConfigFileDirs,
    output: &mut dyn Output,
) -> Result<(), Error> {
    if let Cmd::Completions { .. } = command {
        // Handled by main.rs where the top-level `Cli` is in scope;
        // real_main must not touch config because completions run at every
        // interactive shell startup.
        return Ok(());
    }
    if let Cmd::Init {
        url,
        branch,
        apply,
        yes,
    } = command
    {
        // Init runs before a config.toml exists, so it cannot go through the
        // normal Config::load path below.
        return init::run(
            url.as_deref(),
            branch.as_deref(),
            *apply,
            *yes,
            dirs,
            args,
            output,
        );
    }
    if let Cmd::Doctor = command {
        // Doctor must survive a missing or broken config.toml — it's the
        // command the user runs when things are wrong. Dispatch before
        // Config::load so load failures can be caught and rendered with
        // actionable hints inside doctor::run.
        let sh = Shell::new().unwrap();
        return doctor::run(args, dirs, &sh, output);
    }
    if let Cmd::Schema = command {
        return schema::run(&mut std::io::stdout().lock());
    }
    let sh = Shell::new().unwrap();
    let config = Config::load(dirs, &sh, command.should_update_self(args))?;
    let mut config_files = ConfigFiles::new(dirs);

    match command {
        Cmd::Apply {
            pull_config: _,
            yes,
            dry_run,
            allow_dirty,
        } => {
            let stdout_color = args.color.enabled(std::io::stdout().is_terminal());
            let mut prompter = build_prompter(*yes, *dry_run, stdout_color)?;
            config.push_pkg_health(output)?;

            let git = Git::new(dirs.zenops(), &sh);
            if git.is_git_repo()? && git.has_uncommitted_changes()? {
                config.check_own_status(&sh, output)?;
                // `--yes` without `--allow-dirty` aborts so CI/cron surface
                // divergence instead of silently applying uncommitted state.
                // `--dry-run` writes nothing, so it's always safe to continue.
                // `--allow-dirty` in any mode bypasses the prompt entirely.
                if *yes && !*allow_dirty {
                    return Err(Error::DirtyRepoRequiresAllowDirty(
                        dirs.zenops().to_path_buf(),
                    ));
                }
                if !*allow_dirty {
                    git.print_pre_apply_summary(stdout_color)?;
                    match prompter.confirm_pre_apply()? {
                        PreApplyDecision::CommitAndPush { message } => {
                            git.commit_all_and_push(&message)?;
                        }
                        PreApplyDecision::Continue => {}
                        PreApplyDecision::Abort => return Ok(()),
                    }
                }
            }

            config.update_config_files(&sh, &mut config_files)?;
            config_files.apply_changes(output, prompter.as_mut())?;
        }
        Cmd::Status { diff: _, all: _ } => {
            config.push_pkg_health(output)?;
            config.check_own_status(&sh, output)?;
            config.update_config_files(&sh, &mut config_files)?;
            config_files.check_status(output)?;
        }
        Cmd::Pkg {
            pattern,
            all,
            all_hints,
            verbose,
        } => {
            pkg_list::push(
                &config,
                pkg_list::Options {
                    pattern: pattern.clone(),
                    all: *all,
                    all_hints: *all_hints,
                    verbose: *verbose,
                },
                output,
            )?;
        }
        Cmd::Repo { command } => {
            command.passthru_dispatch_in(dirs.zenops(), &sh)?;
        }
        Cmd::Init { .. } => unreachable!("handled before Config::load"),
        Cmd::Doctor => unreachable!("handled before Config::load"),
        Cmd::Schema => unreachable!("handled before Config::load"),
        Cmd::Completions { .. } => unreachable!("handled before Config::load"),
    }

    Ok(())
}
