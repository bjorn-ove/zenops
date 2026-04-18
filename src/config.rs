mod shell;
mod stored_config_files;
mod stored_relative_path;

use std::{path::Path, sync::Arc};

use safe_relative_path::srpath;
use xshell::{Shell, cmd};

use crate::{
    config::{
        shell::StoredShellEnvironment, stored_config_files::StoredConfigFilesBase,
    },
    config_files::{ConfigFileDirs, ConfigFilePath, ConfigFiles},
    error::Error,
    git::Git,
    output::{Output, ResolvedConfigFilePath, Status},
};

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
struct StoredConfig {
    shell: StoredShellEnvironment,
    configs: Vec<StoredConfigFilesBase>,
}

pub struct Config<'dirs> {
    dirs: &'dirs ConfigFileDirs,
    zenops_repo: ResolvedConfigFilePath,
    stored: StoredConfig,
}

impl<'dirs> Config<'dirs> {
    pub fn load(dirs: &'dirs ConfigFileDirs, sh: &Shell, update_self: bool) -> Result<Self, Error> {
        if update_self {
            let zenops_dir = dirs.zenops();
            cmd!(sh, "git -C {zenops_dir} pull --rebase").run()?;
        }

        let zenops_repo =
            ResolvedConfigFilePath::resolve(ConfigFilePath::Zenops(Arc::from(srpath!(""))), dirs);

        let path = dirs.zenops().join("config.toml");
        Ok(Self {
            dirs,
            zenops_repo,
            stored: toml::from_slice(
                &std::fs::read(&path).map_err(|e| Error::OpenDb(path.clone(), e))?,
            )
            .map_err(|e| Error::ParseDb(path.to_path_buf(), e))?,
        })
    }

    pub fn has_cargo(&self) -> bool {
        self.dirs.home().join(".cargo/env").exists()
    }

    pub fn has_starship(&self) -> bool {
        self.dirs.home().join(".cargo/bin/starship").exists()
            || Path::new("/opt/homebrew/bin/starship").exists()
            || Path::new("/usr/local/bin/starship").exists()
    }

    pub fn has_sk(&self) -> bool {
        self.dirs.home().join(".cargo/bin/sk").exists()
            || Path::new("/opt/homebrew/bin/sk").exists()
            || Path::new("/usr/local/bin/sk").exists()
    }

    #[cfg(target_os = "macos")]
    pub fn has_brew_llvm(&self) -> bool {
        Path::new("/opt/homebrew/opt/llvm").exists()
    }

    #[cfg(target_os = "macos")]
    pub fn has_brew_python(&self) -> bool {
        Path::new("/opt/homebrew/opt/python").exists()
    }

    pub fn path_variable(&self) -> Option<String> {
        let mut paths = "$PATH".to_string();

        #[cfg(target_os = "macos")]
        if self.has_brew_python() {
            paths.push_str(":$(brew --prefix python)/libexec/bin");
        }

        #[cfg(target_os = "macos")]
        if self.has_brew_llvm() {
            paths.insert_str(0, "$(brew --prefix)/opt/llvm/bin:");
        }

        paths.push_str(":~/.local/bin");

        Some(paths)
    }

    pub fn update_config_files(
        &self,
        _sh: &Shell,
        config_files: &mut ConfigFiles<'_>,
    ) -> Result<(), Error> {
        self.stored.shell.update_config_files(self, config_files)?;
        for config in &self.stored.configs {
            config.update_config_files(self, config_files)?;
        }
        Ok(())
    }

    pub fn check_own_status(&self, sh: &Shell, output: &mut dyn Output) -> Result<(), Error> {
        let git = Git::new(self.dirs.zenops(), sh);
        if git.is_git_repo()? {
            for status in git.status()? {
                output.push_status(Status::Git {
                    repo: self.zenops_repo.clone(),
                    status,
                });
            }
        }
        Ok(())
    }
}
