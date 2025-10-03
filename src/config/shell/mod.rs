use crate::{
    config::{Config, shell::bash::StoredBashConfig},
    config_files::ConfigFiles,
    error::Error,
};

mod bash;

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum StoredShellEnvironment {
    Bash(StoredBashConfig),
}

impl StoredShellEnvironment {
    pub fn update_config_files(
        &self,
        config: &Config,
        config_files: &mut ConfigFiles,
    ) -> Result<(), Error> {
        match self {
            Self::Bash(bash_config) => bash_config.make_config_files(config, config_files)?,
        }
        Ok(())
    }
}
