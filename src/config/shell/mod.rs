use crate::{
    config::{Config, pkg::Shell, shell::bash::StoredBashConfig, shell::zsh::StoredZshConfig},
    config_files::ConfigFiles,
    error::Error,
};

mod bash;
mod common;
mod zsh;

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StoredShellEnvironment {
    #[default]
    None,
    Bash(StoredBashConfig),
    Zsh(StoredZshConfig),
}

impl StoredShellEnvironment {
    pub fn update_config_files(
        &self,
        config: &Config,
        config_files: &mut ConfigFiles,
    ) -> Result<(), Error> {
        match self {
            Self::None => Ok(()),
            Self::Bash(shell_config) => bash::make_config_files(shell_config, config, config_files),
            Self::Zsh(shell_config) => zsh::make_config_files(shell_config, config, config_files),
        }
    }

    pub(super) fn shell(&self) -> Option<Shell> {
        match self {
            Self::None => None,
            Self::Bash(_) => Some(Shell::Bash),
            Self::Zsh(_) => Some(Shell::Zsh),
        }
    }
}
