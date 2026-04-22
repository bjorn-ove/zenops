mod ansi;
mod config;
pub mod config_files;
pub mod error;
pub mod git;
pub mod output;
pub mod pkg_list;
pub mod pkg_manager;
pub mod prompt;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// Resolve to a concrete on/off decision. Pass `stream_is_terminal`
    /// for the stream colors will actually be emitted to (stderr for
    /// the terminal renderer and the prompter, stdout for `pkg_list`).
    pub fn enabled(self, stream_is_terminal: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && stream_is_terminal,
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct Args {
    /// When to colorize output
    #[clap(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
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
    },
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
    Repo {
        #[command(subcommand)]
        command: GitCmd,
    },
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
        match *self {
            Cmd::Apply { pull_config, .. } => pull_config,
            Cmd::Status { .. } | Cmd::Pkg { .. } | Cmd::Repo { .. } | Cmd::Completions { .. } => {
                false
            }
        }
    }
}

fn build_prompter(yes: bool, dry_run: bool, color: bool) -> Result<Box<dyn Prompter>, Error> {
    if dry_run {
        Ok(Box::new(DryRunPrompter::new(color)))
    } else if yes {
        Ok(Box::new(YesPrompter))
    } else if std::io::stdin().is_terminal() {
        Ok(Box::new(TerminalPrompter::new(color)))
    } else {
        Err(Error::ApplyNeedsYesOrTty)
    }
}

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
    let sh = Shell::new().unwrap();
    let config = Config::load(dirs, &sh, command.should_update_self(args))?;
    let mut config_files = ConfigFiles::new(dirs);

    match command {
        Cmd::Apply {
            pull_config: _,
            yes,
            dry_run,
        } => {
            let stderr_color = args.color.enabled(std::io::stderr().is_terminal());
            let mut prompter = build_prompter(*yes, *dry_run, stderr_color)?;
            config.push_pkg_health(output)?;

            let git = Git::new(dirs.zenops(), &sh);
            if git.is_git_repo()? && git.has_uncommitted_changes()? {
                config.check_own_status(&sh, output)?;
                git.print_pre_apply_summary(stderr_color)?;
                match prompter.confirm_pre_apply()? {
                    PreApplyDecision::CommitAndPush { message } => {
                        git.commit_all_and_push(&message)?;
                    }
                    PreApplyDecision::Continue => {}
                    PreApplyDecision::Abort => return Ok(()),
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
            all,
            all_hints,
            verbose,
        } => {
            if pkg_manager::detect().is_none() {
                eprintln!(
                    "note: no known package manager detected on PATH; install \
                     guidance will be hidden. Supported managers: brew."
                );
            }
            let opts = pkg_list::Options {
                all: *all,
                all_hints: *all_hints,
                verbose: *verbose,
                color_enabled: args.color.enabled(std::io::stdout().is_terminal()),
            };
            print!("{}", pkg_list::render(&config, opts));
        }
        Cmd::Repo { command } => {
            command.passthru_dispatch_in(dirs.zenops(), &sh)?;
        }
        Cmd::Completions { .. } => unreachable!("handled before Config::load"),
    }

    Ok(())
}
