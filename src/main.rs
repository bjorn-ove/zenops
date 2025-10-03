mod config;
mod config_files;
mod error;
mod git;
mod package_spec;

use clap::{Parser, Subcommand};
use xshell::{Shell, cmd};

use crate::{
    config::Config,
    config_files::{ConfigFileDirs, ConfigFiles},
    error::Error,
};

#[derive(Parser, Debug)]
#[command(name = "zenops", about = "ZenOps: your system’s calm overseer")]
pub struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

impl Cli {
    fn should_update_self(&self) -> bool {
        match self.command {
            Cmd::UpdateConfig { pull_config, .. } => pull_config,
            Cmd::Upgrade { pull_config, .. } => pull_config,
            Cmd::Status => false,
        }
    }
}

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
    Status,
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

fn real_main() -> Result<(), Error> {
    let args = Cli::parse();
    let dirs = ConfigFileDirs::load();
    let sh = Shell::new().unwrap();
    let config = Config::load(&dirs, &sh, args.should_update_self())?;
    let mut config_files = ConfigFiles::new(&dirs);

    match args.command {
        Cmd::UpdateConfig { pull_config: _ } => {
            config.update_config_files(&sh, &mut config_files)?;
            config_files.apply_changes()?;
        }
        Cmd::Upgrade { pull_config: _ } => {
            install_or_upgrade_packages(&sh, &config)?;
            config.update_config_files(&sh, &mut config_files)?;
            config_files.apply_changes()?;
        }
        Cmd::Status => {
            config.check_own_status(&sh)?;
            config.update_config_files(&sh, &mut config_files)?;
            config_files.check_status();
        }
    }

    Ok(())
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format_timestamp(None)
        .format_target(false)
        .init();

    if let Err(e) = real_main() {
        log::error!("{e}");
        std::process::exit(1);
    }
}
