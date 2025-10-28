use std::sync::Arc;

use safe_relative_path::{SafeRelativePathBuf, SinglePathComponent};

use crate::config_files::{ConfigFilePath, ConfigFileSource};

use super::{Config, ConfigFiles, Error, StoredRelativePath};

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredConfigFiles {
    source: StoredRelativePath,
    #[serde(default)]
    symlinks: Vec<StoredRelativePath>,
}

impl StoredConfigFiles {
    pub fn update_config_files<'a>(
        &'a self,
        _config: &Config,
        config_files: &mut ConfigFiles,
        make_config_path: impl Fn(&'a StoredRelativePath) -> ConfigFilePath,
    ) -> Result<(), Error> {
        for symlink in &self.symlinks {
            config_files.add(
                make_config_path(symlink),
                ConfigFileSource::SymlinkFrom(ConfigFilePath::Zenops(Arc::from(
                    self.source.safe_join(symlink),
                ))),
            );
        }
        Ok(())
    }
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, tag = "type")]
pub(super) enum StoredConfigFilesBase {
    #[serde(rename = ".config")]
    DotConfig {
        name: SinglePathComponent,
        #[serde(flatten)]
        configs: StoredConfigFiles,
    },
    #[serde(rename = "home")]
    Home {
        dir: SafeRelativePathBuf,
        #[serde(flatten)]
        configs: StoredConfigFiles,
    },
}

impl StoredConfigFilesBase {
    pub fn update_config_files(
        &self,
        config: &Config,
        config_files: &mut ConfigFiles,
    ) -> Result<(), Error> {
        match self {
            Self::DotConfig { name, configs } => {
                configs.update_config_files(config, config_files, |symlink| {
                    ConfigFilePath::DotConfig(Arc::from(name.safe_join(symlink)))
                })?
            }
            Self::Home { dir, configs } => {
                configs.update_config_files(config, config_files, |symlink| {
                    ConfigFilePath::Home(Arc::from(dir.safe_join(symlink)))
                })?
            }
        }
        Ok(())
    }
}
