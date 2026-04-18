mod shell;
mod stored_config_files;
mod stored_relative_path;

use std::sync::Arc;

use indexmap::IndexMap;
use safe_relative_path::srpath;
use serde::de;
use smol_str::SmolStr;
use xshell::{Shell, cmd};

use crate::{
    config::{
        shell::StoredShellEnvironment, stored_config_files::StoredConfigFilesBase,
        stored_relative_path::StoredRelativePath,
    },
    config_files::{ConfigFileDirs, ConfigFilePath, ConfigFiles},
    error::Error,
    git::Git,
    output::{Output, ResolvedConfigFilePath, Status},
    package_spec::{PackageSpec, PackageSpecSeed},
};

#[derive(Debug, Clone, PartialEq, Default)]
struct StoredPackages(IndexMap<SmolStr, PackageSpec>);

impl<'de> de::Deserialize<'de> for StoredPackages {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StoredPackages;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "map of packages")
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut ret = IndexMap::new();
                while let Some(name) = map.next_key::<SmolStr>()? {
                    let old = ret.insert(name.clone(), map.next_value_seed(PackageSpecSeed(name))?);
                    if let Some(old) = old {
                        return Err(de::Error::custom(format_args!(
                            "Duplicate entry for package {old}",
                        )));
                    }
                }
                Ok(StoredPackages(ret))
            }
        }

        d.deserialize_any(Visitor)
    }
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
struct StoredConfig {
    packages: StoredPackages,
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

    pub fn brew_brew_package_strings(&self) -> Vec<SmolStr> {
        let mut ret = Vec::new();
        for spec in self.stored.packages.0.values() {
            ret.extend(spec.brew_package().into_iter().flat_map(|v| v.brew_spec()));
        }
        ret
    }

    pub fn brew_cask_package_strings(&self) -> Vec<SmolStr> {
        let mut ret = Vec::new();
        for spec in self.stored.packages.0.values() {
            ret.extend(spec.brew_package().into_iter().flat_map(|v| v.cask_spec()));
        }
        ret
    }

    pub fn cargo_crates_io_packages(&self) -> Vec<SmolStr> {
        let mut ret = Vec::new();
        for spec in self.stored.packages.0.values() {
            ret.extend(
                spec.cargo_package()
                    .and_then(|v| v.get_name_if_crates_io_latest()),
            );
        }
        ret
    }

    pub fn has_cargo_packages(&self) -> bool {
        self.stored.packages.0.values().any(|spec| spec.is_cargo())
    }

    pub fn path_variable(&self) -> Option<String> {
        let mut paths = "$PATH".to_string();

        if let Some(spec) = self.stored.packages.0.get("python") {
            #[cfg(target_os = "macos")]
            if spec.is_brew() {
                paths.push_str(":$(brew --prefix python)/libexec/bin");
            }
        }

        if let Some(spec) = self.stored.packages.0.get("llvm") {
            #[cfg(target_os = "macos")]
            if spec.is_brew() {
                paths.insert_str(0, "$(brew --prefix)/opt/llvm/bin:");
            }
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
