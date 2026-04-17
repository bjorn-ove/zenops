mod config;
pub mod config_files;
pub mod error;
pub mod git;
pub mod output;
mod package_spec;

use clap::Subcommand;
use xshell::{Shell, cmd};

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
    UpdateConfig {
        /// Pull the latest version of the config using git pull --rebase in the zenops config directory
        #[clap(long, short)]
        pull_config: bool,
    },
    Upgrade {
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
            Cmd::UpdateConfig { pull_config, .. } => pull_config,
            Cmd::Upgrade { pull_config, .. } => pull_config,
            Cmd::Status { .. } | Cmd::Repo { .. } => false,
        }
    }
}

fn install_or_upgrade_packages(sh: &Shell, config: &Config) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        {
            let packages = config.brew_brew_package_strings();
            if !packages.is_empty() {
                log::info!("Installing {} packages using brew", packages.len());
                cmd!(sh, "brew install {packages...}").run()?;
            } else {
                log::info!("No brew packages to install");
            }
        }

        {
            let packages = config.brew_cask_package_strings();
            if !packages.is_empty() {
                log::info!("Installing {} cask packages using brew", packages.len());
                cmd!(sh, "brew install --cask {packages...}").run()?;
            } else {
                log::info!("No brew cask packages to install");
            }
        }
    }

    {
        let packages = config.cargo_crates_io_packages();
        if !packages.is_empty() {
            log::info!(
                "Installing {} packages from crates.io using cargo",
                packages.len()
            );
            cmd!(sh, "cargo install-update {packages...}").run()?;
        } else {
            log::info!("No cargo crates.io packages to install");
        }
    }

    Ok(())
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
        Cmd::UpdateConfig { pull_config: _ } => {
            config.update_config_files(&sh, &mut config_files)?;
            config_files.apply_changes(output)?;
        }
        Cmd::Upgrade { pull_config: _ } => {
            install_or_upgrade_packages(&sh, &config)?;
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
