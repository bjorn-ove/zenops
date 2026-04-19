mod config;
pub mod config_files;
pub mod error;
pub mod git;
pub mod output;
pub mod pkg_list;
pub mod pkg_manager;

use std::io::IsTerminal;

use clap::Subcommand;
use xshell::Shell;

use crate::{
    config::Config,
    config_files::{ConfigFileDirs, ConfigFiles},
    error::Error,
    git::GitCmd,
    output::{DiffLog, Output},
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
    pub fn enabled(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
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
    },
    Status {
        /// Show a diff of what would change
        #[clap(long, short = 'd')]
        diff: bool,
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
}

impl Cmd {
    fn should_update_self(&self, _args: &Args) -> bool {
        match *self {
            Cmd::Apply { pull_config, .. } => pull_config,
            Cmd::Status { .. } | Cmd::Pkg { .. } | Cmd::Repo { .. } => false,
        }
    }
}

pub fn real_main(
    args: &Args,
    command: &Cmd,
    dirs: &ConfigFileDirs,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let sh = Shell::new().unwrap();
    let config = Config::load(dirs, &sh, command.should_update_self(args))?;
    let mut config_files = ConfigFiles::new(dirs);

    match command {
        Cmd::Apply { pull_config: _ } => {
            config.update_config_files(&sh, &mut config_files)?;
            config_files.apply_changes(output)?;
        }
        Cmd::Status { diff } => {
            config.check_own_status(&sh, output)?;
            config.update_config_files(&sh, &mut config_files)?;
            if *diff {
                config_files.check_status(&mut DiffLog);
            } else {
                config_files.check_status(output);
            }
        }
        Cmd::Pkg {
            all,
            all_hints,
            verbose,
        } => {
            let opts = pkg_list::Options {
                all: *all,
                all_hints: *all_hints,
                verbose: *verbose,
                color_enabled: args.color.enabled(),
            };
            print!("{}", pkg_list::render(&config, opts));
        }
        Cmd::Repo { command } => {
            command.passthru_dispatch_in(dirs.zenops(), &sh)?;
        }
    }

    Ok(())
}
