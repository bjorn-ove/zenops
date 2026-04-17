use crate::{
    config::{Config, shell::bash::StoredBashConfig, shell::zsh::StoredZshConfig},
    config_files::ConfigFiles,
    error::Error,
};

mod bash;
mod zsh;

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
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
            Self::Bash(bash_config) => bash_config.make_config_files(config, config_files),
            Self::Zsh(zsh_config) => zsh_config.make_config_files(config, config_files),
        }
    }
}
