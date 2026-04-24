use std::sync::Arc;

use zenops_safe_relative_path::{SafeRelativePathBuf, SinglePathComponent};

use crate::config_files::{ConfigFilePath, ConfigFileSource};

use super::{Config, ConfigFiles, Error, stored_relative_path::StoredRelativePath};

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredConfigFiles {
    source: StoredRelativePath,
    #[serde(default)]
    symlinks: Vec<StoredRelativePath>,
}

impl StoredConfigFiles {
    fn update_config_files<'a>(
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

#[derive(serde::Deserialize, schemars::JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, tag = "type")]
pub(super) enum PkgConfigFiles {
    #[serde(rename = ".config")]
    DotConfig {
        /// Override for the `~/.config/<name>/` directory. Defaults to the
        /// pkg key when omitted — e.g. `[pkg.helix]` lands at `~/.config/helix/`.
        /// Only set this when the pkg key differs from the config dir (e.g. a
        /// pkg keyed as `neovim` whose config dir is `nvim`).
        #[serde(default)]
        name: Option<SinglePathComponent>,
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

impl PkgConfigFiles {
    pub fn update_config_files(
        &self,
        pkg_key: &str,
        config: &Config,
        config_files: &mut ConfigFiles,
    ) -> Result<(), Error> {
        match self {
            Self::DotConfig { name, configs } => {
                let fallback;
                let dir: &SinglePathComponent = match name {
                    Some(n) => n,
                    None => {
                        fallback = SinglePathComponent::try_new(pkg_key)?;
                        &fallback
                    }
                };
                configs.update_config_files(config, config_files, |symlink| {
                    ConfigFilePath::DotConfig(Arc::from(dir.safe_join(symlink)))
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
