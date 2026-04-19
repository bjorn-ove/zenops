pub(crate) mod pkg;
mod shell;
mod stored_config_files;
mod stored_relative_path;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;
use smol_str::SmolStr;
use xshell::cmd;
use zenops_safe_relative_path::srpath;

pub use crate::config::pkg::PkgConfig;

use crate::{
    config::{
        pkg::{Shell, ShellInitAction, StoredPkgConfig},
        shell::StoredShellEnvironment,
        stored_config_files::StoredConfigFilesBase,
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
    pkg: IndexMap<SmolStr, StoredPkgConfig>,
}

pub struct Config<'dirs> {
    dirs: &'dirs ConfigFileDirs,
    zenops_repo: ResolvedConfigFilePath,
    stored: StoredConfig,
    pkgs: IndexMap<SmolStr, PkgConfig>,
    brew_prefix: Option<PathBuf>,
}

fn detect_brew_prefix() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"];
    CANDIDATES
        .iter()
        .map(Path::new)
        .find(|prefix| prefix.join("bin/brew").exists())
        .map(PathBuf::from)
}

fn deep_merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                deep_merge(
                    b.entry(k).or_insert(toml::Value::Table(Default::default())),
                    v,
                );
            }
        }
        (base, overlay) => *base = overlay,
    }
}

impl<'dirs> Config<'dirs> {
    pub fn load(
        dirs: &'dirs ConfigFileDirs,
        sh: &xshell::Shell,
        update_self: bool,
    ) -> Result<Self, Error> {
        if update_self {
            let zenops_dir = dirs.zenops();
            cmd!(sh, "git -C {zenops_dir} pull --rebase").run()?;
        }

        let zenops_repo =
            ResolvedConfigFilePath::resolve(ConfigFilePath::Zenops(Arc::from(srpath!(""))), dirs);

        let path = dirs.zenops().join("config.toml");

        let defaults_str = include_str!("config/defaults.toml");
        let mut merged: toml::Value = toml::from_str(defaults_str)
            .map_err(|e| Error::ParseDb(std::path::PathBuf::from("<defaults>"), e))?;

        let user_bytes = std::fs::read(&path).map_err(|e| Error::OpenDb(path.clone(), e))?;
        let user_val: toml::Value =
            toml::from_slice(&user_bytes).map_err(|e| Error::ParseDb(path.to_path_buf(), e))?;

        deep_merge(&mut merged, user_val);

        let stored: StoredConfig = merged
            .try_into()
            .map_err(|e| Error::ParseDb(path.to_path_buf(), e))?;

        let mut pkgs = IndexMap::new();
        for (k, v) in stored.pkg.iter() {
            pkgs.insert(k.clone(), v.clone().resolve(k)?);
        }

        Ok(Self {
            dirs,
            zenops_repo,
            stored,
            pkgs,
            brew_prefix: detect_brew_prefix(),
        })
    }

    pub fn brew_prefix(&self) -> Option<&Path> {
        self.brew_prefix.as_deref()
    }

    pub fn has_brew_llvm(&self) -> bool {
        self.brew_prefix
            .as_ref()
            .is_some_and(|p| p.join("opt/llvm").exists())
    }

    pub fn has_brew_python(&self) -> bool {
        self.brew_prefix
            .as_ref()
            .is_some_and(|p| p.join("opt/python").exists())
    }

    pub fn pkgs(&self) -> &IndexMap<SmolStr, PkgConfig> {
        &self.pkgs
    }

    pub fn home(&self) -> &Path {
        self.dirs.home()
    }

    pub(crate) fn env_pkg_inits(&self, shell: Shell) -> Vec<&ShellInitAction> {
        self.pkgs
            .values()
            .filter(|p| p.is_installed(self.dirs.home()))
            .flat_map(|p| p.env_init.for_shell(shell))
            .collect()
    }

    pub(crate) fn interactive_pkg_inits(&self, shell: Shell) -> Vec<&ShellInitAction> {
        self.pkgs
            .values()
            .filter(|p| p.is_installed(self.dirs.home()))
            .flat_map(|p| p.interactive_init.for_shell(shell))
            .collect()
    }

    pub fn path_variable(&self) -> Option<String> {
        let mut paths = "$PATH".to_string();

        if self.has_brew_python() {
            paths.push_str(":$(brew --prefix python)/libexec/bin");
        }

        if self.has_brew_llvm() {
            paths.insert_str(0, "$(brew --prefix)/opt/llvm/bin:");
        }

        paths.push_str(":~/.local/bin");

        Some(paths)
    }

    pub fn update_config_files(
        &self,
        _sh: &xshell::Shell,
        config_files: &mut ConfigFiles<'_>,
    ) -> Result<(), Error> {
        self.stored.shell.update_config_files(self, config_files)?;
        for config in &self.stored.configs {
            config.update_config_files(self, config_files)?;
        }
        Ok(())
    }

    pub fn check_own_status(
        &self,
        sh: &xshell::Shell,
        output: &mut dyn Output,
    ) -> Result<(), Error> {
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
