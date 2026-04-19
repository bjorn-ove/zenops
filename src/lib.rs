mod config;
pub mod config_files;
pub mod error;
pub mod git;
pub mod output;

use clap::Subcommand;
use xshell::Shell;

use crate::{
    config::Config,
    config_files::{ConfigFileDirs, ConfigFiles},
    error::Error,
    git::GitCmd,
    output::{DiffLog, Output},
};

#[derive(clap::Args, Debug)]
pub struct Args {}

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
    Repo {
        #[command(subcommand)]
        command: GitCmd,
    },
}

impl Cmd {
    fn should_update_self(&self, _args: &Args) -> bool {
        match *self {
            Cmd::Apply { pull_config, .. } => pull_config,
            Cmd::Status { .. } | Cmd::Repo { .. } => false,
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
        Cmd::Repo { command } => {
            command.passthru_dispatch_in(dirs.zenops(), &sh)?;
        }
    }

    Ok(())
}
