pub(crate) mod pkg;
mod pkg_config_files;
mod shell;
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
    },
    config_files::{ConfigFileDirs, ConfigFilePath, ConfigFiles},
    error::Error,
    git::Git,
    output::{Output, ResolvedConfigFilePath, Status},
    pkg_manager,
};

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
struct StoredConfig {
    shell: StoredShellEnvironment,
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

static DEFAULT_PKGS: &[(&str, &str)] = &[
    ("brew-macos", include_str!("config/pkgs/brew-macos.toml")),
    ("brew-linux", include_str!("config/pkgs/brew-linux.toml")),
    (
        "bashrc-chain",
        include_str!("config/pkgs/bashrc-chain.toml"),
    ),
    ("local-bin", include_str!("config/pkgs/local-bin.toml")),
    ("brew-python", include_str!("config/pkgs/brew-python.toml")),
    ("cargo", include_str!("config/pkgs/cargo.toml")),
    (
        "bash-completion",
        include_str!("config/pkgs/bash-completion.toml"),
    ),
    (
        "zsh-completions",
        include_str!("config/pkgs/zsh-completions.toml"),
    ),
    ("sk", include_str!("config/pkgs/sk.toml")),
    ("starship", include_str!("config/pkgs/starship.toml")),
    ("zenops", include_str!("config/pkgs/zenops.toml")),
    ("llvm", include_str!("config/pkgs/llvm.toml")),
];

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

        let mut merged = toml::Value::Table(Default::default());
        for (name, src) in DEFAULT_PKGS {
            let v: toml::Value = toml::from_str(src).map_err(|e| {
                Error::ParseDb(std::path::PathBuf::from(format!("<defaults:{name}>")), e)
            })?;
            deep_merge(&mut merged, v);
        }

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

    pub(crate) fn shell(&self) -> Option<Shell> {
        self.stored.shell.shell()
    }

    pub(crate) fn env_pkg_inits(
        &self,
        shell: Shell,
    ) -> Vec<(&SmolStr, &PkgConfig, &ShellInitAction)> {
        self.stored
            .pkg
            .iter()
            .filter(|(_, p)| p.is_installed(self.dirs.home(), &self.system_inputs))
            .filter(|(_, p)| p.supports_shell(Some(shell)))
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
            .filter(|(_, p)| p.supports_shell(Some(shell)))
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
            .filter(|(_, p)| p.supports_shell(Some(shell)))
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
        for (pkg_key, pkg) in &self.stored.pkg {
            if !pkg.is_installed(self.dirs.home(), &self.system_inputs) {
                continue;
            }
            for cfg in pkg.configs() {
                cfg.update_config_files(pkg_key, self, config_files)?;
            }
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
                })?;
            }
        }
        Ok(())
    }

    /// Emit a `Status::PkgMissing` for every pkg that the user declared
    /// `enable = "on"` on, yet whose detect strategies don't match on this
    /// host. No-op for `detect`/`disabled` pkgs — silence on miss is the
    /// defining behavior of `detect`. Called from the apply/status entry
    /// points, not from `Config::load` — a load isn't an event, and these
    /// observations should only surface from commands the user runs.
    pub fn push_pkg_health(&self, output: &mut dyn Output) -> Result<(), Error> {
        let manager = pkg_manager::detect();
        for (key, pkg) in &self.stored.pkg {
            if !pkg.enable_on_but_detect_missing(self.dirs.home(), &self.system_inputs) {
                continue;
            }
            let label = pkg.name.clone().unwrap_or_else(|| key.clone());
            let install_command = manager.and_then(|m| {
                let pkgs = m.packages_for(&pkg.install_hint);
                (!pkgs.is_empty()).then(|| m.install_command(pkgs))
            });
            output.push_status(Status::PkgMissing {
                pkg: label,
                install_command,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod readme_tests {
    use super::StoredConfig;

    /// Every ```toml block in README.md must deserialize as a full
    /// [`StoredConfig`]. Guards against docs silently drifting away from the
    /// real config shape (e.g. after a breaking rename like `[[configs]]` →
    /// `[[pkg.x.configs]]`).
    #[test]
    fn readme_toml_blocks_parse_as_stored_config() {
        let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
            .expect("read README.md");

        let mut blocks = Vec::new();
        let mut in_toml = false;
        let mut current = String::new();
        for line in readme.lines() {
            if in_toml {
                if line.trim_start().starts_with("```") {
                    blocks.push(std::mem::take(&mut current));
                    in_toml = false;
                } else {
                    current.push_str(line);
                    current.push('\n');
                }
            } else if line.trim_start().starts_with("```toml") {
                in_toml = true;
            }
        }
        assert!(!blocks.is_empty(), "README has no ```toml blocks");

        for (i, block) in blocks.iter().enumerate() {
            toml::from_str::<StoredConfig>(block).unwrap_or_else(|e| {
                panic!("README ```toml block #{i} failed to parse: {e}\n---\n{block}---")
            });
        }
    }
}
