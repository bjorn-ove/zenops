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
        pkg::{Shell, ShellInitAction},
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
    pkg: IndexMap<SmolStr, PkgConfig>,
}

pub struct Config<'dirs> {
    dirs: &'dirs ConfigFileDirs,
    zenops_repo: ResolvedConfigFilePath,
    stored: StoredConfig,
    system_inputs: IndexMap<SmolStr, SmolStr>,
}

fn detect_brew_prefix() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"];
    CANDIDATES
        .iter()
        .map(Path::new)
        .find(|prefix| prefix.join("bin/brew").exists())
        .map(PathBuf::from)
}

fn build_system_inputs(brew_prefix: Option<&Path>) -> IndexMap<SmolStr, SmolStr> {
    let mut m = IndexMap::new();
    if let Some(p) = brew_prefix {
        m.insert(
            SmolStr::new_static("brew_prefix"),
            SmolStr::new(p.to_string_lossy()),
        );
    }
    m.insert(
        SmolStr::new_static("os"),
        SmolStr::new_static(std::env::consts::OS),
    );
    m
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

        let system_inputs = build_system_inputs(detect_brew_prefix().as_deref());

        Ok(Self {
            dirs,
            zenops_repo,
            stored,
            system_inputs,
        })
    }

    pub fn pkgs(&self) -> &IndexMap<SmolStr, PkgConfig> {
        &self.stored.pkg
    }

    pub fn home(&self) -> &Path {
        self.dirs.home()
    }

    pub fn system_inputs(&self) -> &IndexMap<SmolStr, SmolStr> {
        &self.system_inputs
    }

    pub(crate) fn env_pkg_inits(
        &self,
        shell: Shell,
    ) -> Vec<(&SmolStr, &PkgConfig, &ShellInitAction)> {
        self.stored
            .pkg
            .iter()
            .filter(|(_, p)| p.is_installed(self.dirs.home(), &self.system_inputs))
            .flat_map(|(name, p)| {
                p.shell
                    .env_init
                    .for_shell(shell)
                    .iter()
                    .map(move |a| (name, p, a))
            })
            .collect()
    }

    pub(crate) fn login_pkg_inits(
        &self,
        shell: Shell,
    ) -> Vec<(&SmolStr, &PkgConfig, &ShellInitAction)> {
        self.stored
            .pkg
            .iter()
            .filter(|(_, p)| p.is_installed(self.dirs.home(), &self.system_inputs))
            .flat_map(|(name, p)| {
                p.shell
                    .login_init
                    .for_shell(shell)
                    .iter()
                    .map(move |a| (name, p, a))
            })
            .collect()
    }

    pub(crate) fn interactive_pkg_inits(
        &self,
        shell: Shell,
    ) -> Vec<(&SmolStr, &PkgConfig, &ShellInitAction)> {
        self.stored
            .pkg
            .iter()
            .filter(|(_, p)| p.is_installed(self.dirs.home(), &self.system_inputs))
            .flat_map(|(name, p)| {
                p.shell
                    .interactive_init
                    .for_shell(shell)
                    .iter()
                    .map(move |a| (name, p, a))
            })
            .collect()
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
