use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::error::Error;
use safe_relative_path::{SafeRelativePath, SafeRelativePathBuf};

#[derive(Debug, Clone)]
pub enum ConfigFilePath {
    Home(SafeRelativePathBuf),
    DotConfig(SafeRelativePathBuf),
    Zenops(SafeRelativePathBuf),
}

impl ConfigFilePath {
    pub fn in_home(path: impl AsRef<SafeRelativePath>) -> Self {
        Self::Home(path.as_ref().to_safe_relative_path_buf())
    }

    pub fn _in_dot_config(path: impl AsRef<SafeRelativePath>) -> Self {
        Self::DotConfig(path.as_ref().to_safe_relative_path_buf())
    }

    pub fn resolved(&self, config_files: &ConfigFiles) -> PathBuf {
        let dirs = config_files.dirs;
        match self {
            Self::Home(path) => path.to_path(&dirs.home),
            Self::DotConfig(path) => path.to_path(&dirs.config),
            Self::Zenops(path) => path.to_path(&dirs.zenops),
        }
    }

    pub fn human_path(&self) -> String {
        match self {
            Self::Home(path) => format!("~/{path}"),
            Self::DotConfig(path) => format!("~/.config/{path}"),
            Self::Zenops(path) => format!("~/.config/zenops/{path}"),
        }
    }
}

struct FileEntry {
    path: ConfigFilePath,
    src: ResolvedFileSource,
}

pub enum ConfigFileSource {
    Raw(String),
    SymlinkFrom(ConfigFilePath),
}

impl ConfigFileSource {
    fn into_resolved(self, config_files: &ConfigFiles<'_>) -> ResolvedFileSource {
        match self {
            ConfigFileSource::Raw(data) => ResolvedFileSource::Raw(data),
            ConfigFileSource::SymlinkFrom(rel) => ResolvedFileSource::SymlinkFrom {
                full: rel.resolved(config_files),
                rel,
            },
        }
    }
}

enum ResolvedFileSource {
    Raw(String),
    SymlinkFrom { full: PathBuf, rel: ConfigFilePath },
}

pub struct ConfigFileDirs {
    home: PathBuf,
    config: PathBuf,
    zenops: PathBuf,
}

impl ConfigFileDirs {
    pub fn load() -> Self {
        let home = home::home_dir().unwrap();
        let config = home.join(".config");
        let zenops = home.join(".config/zenops");
        Self {
            home,
            config,
            zenops,
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
}

pub struct ConfigFiles<'dirs> {
    dirs: &'dirs ConfigFileDirs,
    files: IndexMap<PathBuf, FileEntry>,
}

impl<'dirs> ConfigFiles<'dirs> {
    pub fn new(dirs: &'dirs ConfigFileDirs) -> Self {
        Self {
            dirs,
            files: IndexMap::new(),
        }
    }

    pub fn add(&mut self, path: ConfigFilePath, src: ConfigFileSource) {
        self.files.insert(
            path.resolved(self),
            FileEntry {
                path,
                src: src.into_resolved(self),
            },
        );
    }

    pub fn check_status(&self) {
        for (path, entry) in &self.files {
            let status = match &entry.src {
                ResolvedFileSource::Raw(content) => {
                    if path.exists() {
                        if let Ok(cur_content) = std::fs::read(path) {
                            if cur_content == content.as_bytes() {
                                "ok"
                            } else {
                                "modified"
                            }
                        } else {
                            todo!()
                        }
                    } else {
                        "new file"
                    }
                }
                ResolvedFileSource::SymlinkFrom { full, rel: _ } => {
                    match SymlinkStatus::from_path(path) {
                        SymlinkStatus::LinksTo(link_path) => {
                            if &link_path == full {
                                "ok"
                            } else {
                                "modified link"
                            }
                        }
                        SymlinkStatus::NotFound => "new link",
                        SymlinkStatus::NotSymlinkIsFile => todo!(),
                        SymlinkStatus::NotSymlinkIsDir => todo!(),
                    }
                }
            };
            if status == "ok" {
                log::debug!("    {status}: {}", entry.path.human_path());
            } else {
                log::info!("    {status}: {}", entry.path.human_path());
            }
        }
    }

    pub fn apply_changes(&self) -> Result<(), Error> {
        for (path, entry) in &self.files {
            match &entry.src {
                ResolvedFileSource::Raw(content) => {
                    if path.exists() {
                        if let Ok(cur_content) = std::fs::read(path) {
                            if cur_content == content.as_bytes() {
                                log::debug!(
                                    "Config is already up to date {}",
                                    entry.path.human_path()
                                );
                                continue;
                            } else {
                                log::info!("Updating modified config {}", entry.path.human_path());
                            }
                        } else {
                            todo!()
                        }
                    } else {
                        if let Some(parent_dir) = path.parent() {
                            if !parent_dir.is_dir() {
                                log::info!("Creating new directory {parent_dir:?}");
                            }
                        } else {
                            todo!()
                        }
                        log::info!("Creating new config {}", entry.path.human_path());
                    }
                    std::fs::write(path, content)
                        .map_err(|e| Error::FailedToWriteConfig(entry.path.clone(), e))?;
                }
                ResolvedFileSource::SymlinkFrom { full, rel } => {
                    match SymlinkStatus::from_path(path) {
                        SymlinkStatus::LinksTo(link_path) => {
                            if &link_path == full {
                                log::debug!(
                                    "Symlink from {} to {} is already in place",
                                    rel.human_path(),
                                    entry.path.human_path()
                                );
                                continue;
                            } else {
                                todo!()
                            }
                        }
                        SymlinkStatus::NotFound => {
                            log::info!(
                                "Creating symlink from {} to {}",
                                rel.human_path(),
                                entry.path.human_path()
                            );
                            create_symlink(full, path)?;
                        }
                        SymlinkStatus::NotSymlinkIsFile => {
                            return Err(Error::RefusingToOverwriteFileWithSymlink(
                                rel.clone(),
                                entry.path.clone(),
                            ));
                        }
                        SymlinkStatus::NotSymlinkIsDir => todo!(),
                    }
                }
            };
        }

        Ok(())
    }
}

enum SymlinkStatus {
    LinksTo(PathBuf),
    NotFound,
    NotSymlinkIsFile,
    NotSymlinkIsDir,
}

impl SymlinkStatus {
    pub fn from_path(p: impl AsRef<Path>) -> Self {
        let p = p.as_ref();
        match p.symlink_metadata() {
            Ok(meta) => {
                if meta.is_symlink() {
                    match std::fs::read_link(p) {
                        Ok(link_path) => Self::LinksTo(link_path),
                        Err(e) => todo!("{p:?} {e:?}"),
                    }
                } else if meta.is_file() {
                    Self::NotSymlinkIsFile
                } else if meta.is_dir() {
                    Self::NotSymlinkIsDir
                } else {
                    todo!()
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound) => Self::NotFound,
            Err(e) => todo!("{e:?}"),
        }
    }
}

#[cfg(unix)]
fn create_symlink(real_path: &Path, symlink_path: &Path) -> Result<(), Error> {
    match std::os::unix::fs::symlink(real_path, symlink_path) {
        Ok(()) => Ok(()),
        Err(e) => {
            if matches!(e.kind(), std::io::ErrorKind::AlreadyExists) {
                todo!()
            } else {
                todo!()
            }
        }
    }
}
