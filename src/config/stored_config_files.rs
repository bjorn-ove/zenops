use std::sync::Arc;

use safe_relative_path::SafeRelativePath;

use crate::config_files::{ConfigFilePath, ConfigFileSource};

use super::{Config, ConfigFiles, Error, StoredRelativePath, StoredSinglePathComponent};

#[derive(serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredConfigFiles {
    name: StoredSinglePathComponent,
    source: StoredRelativePath,
    #[serde(default)]
    symlinks: Vec<StoredRelativePath>,
}

impl StoredConfigFiles {
    pub fn update_config_files(
        &self,
        _config: &Config,
        config_files: &mut ConfigFiles,
        make_config_path: impl Fn(Arc<SafeRelativePath>) -> ConfigFilePath,
    ) -> Result<(), Error> {
        for symlink in &self.symlinks {
            config_files.add(
                make_config_path(Arc::from(symlink.with_prefix(&self.name))),
                ConfigFileSource::SymlinkFrom(ConfigFilePath::Zenops(Arc::from(
                    symlink.with_prefix(&self.source),
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
    DotConfig(StoredConfigFiles),
}

impl StoredConfigFilesBase {
    pub fn update_config_files(
        &self,
        config: &Config,
        config_files: &mut ConfigFiles,
    ) -> Result<(), Error> {
        match self {
            Self::DotConfig(files) => {
                files.update_config_files(config, config_files, ConfigFilePath::DotConfig)?
            }
        }
        Ok(())
    }
}
