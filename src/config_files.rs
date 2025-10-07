use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;

use crate::{
    error::Error,
    output::{FileStatus, Output, ResolvedConfigFilePath, Status, SymlinkStatus},
};
use safe_relative_path::SafeRelativePath;

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigFilePath {
    Home(Arc<SafeRelativePath>),
    DotConfig(Arc<SafeRelativePath>),
    Zenops(Arc<SafeRelativePath>),
}

impl ConfigFilePath {
    pub fn in_home(path: impl AsRef<SafeRelativePath>) -> Self {
        Self::Home(Arc::from(path.as_ref()))
    }

    pub fn _in_dot_config(path: impl AsRef<SafeRelativePath>) -> Self {
        Self::DotConfig(Arc::from(path.as_ref()))
    }

    pub fn resolved(&self, dirs: &ConfigFileDirs) -> PathBuf {
        match self {
            Self::Home(path) => path.to_path(&dirs.home),
            Self::DotConfig(path) => path.to_path(&dirs.config),
            Self::Zenops(path) => path.to_path(&dirs.zenops),
        }
    }

    pub fn human_path(&self) -> Cow<'_, str> {
        let (base, path) = match self {
            Self::Home(path) => ("~", path),
            Self::DotConfig(path) => ("~/.config", path),
            Self::Zenops(path) => ("~/.config/zenops", path),
        };
        if path.as_str().is_empty() {
            Cow::Borrowed(base)
        } else {
            Cow::Owned(format!("{base}/{path}"))
        }
    }
}

struct FileEntry {
    path: ConfigFilePath,
    src: ResolvedFileSource,
}

pub enum ConfigFileSource {
    Generated(String),
    SymlinkFrom(ConfigFilePath),
}

impl ConfigFileSource {
    fn into_resolved(self, config_files: &ConfigFiles<'_>) -> ResolvedFileSource {
        match self {
            ConfigFileSource::Generated(data) => ResolvedFileSource::Generated(Arc::from(data)),
            ConfigFileSource::SymlinkFrom(rel) => ResolvedFileSource::SymlinkFrom {
                full: Arc::from(rel.resolved(config_files.dirs)),
                rel,
            },
        }
    }
}

enum ResolvedFileSource {
    Generated(Arc<str>),
    SymlinkFrom {
        full: Arc<Path>,
        rel: ConfigFilePath,
    },
}

pub struct ConfigFileDirs {
    home: PathBuf,
    config: PathBuf,
    zenops: PathBuf,
}

impl ConfigFileDirs {
    pub fn load(home: PathBuf) -> Self {
        assert!(home.is_absolute(), "{home:?}");
        let config = home.join(".config");
        let zenops = home.join(".config/zenops");
        Self {
            home,
            config,
            zenops,
        }
    }

    pub fn zenops(&self) -> &Path {
        &self.zenops
    }
}

pub struct ConfigFiles<'dirs> {
    dirs: &'dirs ConfigFileDirs,
    files: IndexMap<Arc<Path>, FileEntry>,
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
            Arc::from(path.resolved(self.dirs)),
            FileEntry {
                path,
                src: src.into_resolved(self),
            },
        );
    }

    fn entry_status(&self, full: Arc<Path>, entry: &FileEntry) -> Status {
        let path = ResolvedConfigFilePath {
            path: entry.path.clone(),
            full,
        };

        match &entry.src {
            ResolvedFileSource::Generated(content) => {
                if path.full.exists() {
                    if let Ok(cur_content) = std::fs::read_to_string(&path.full) {
                        let status = if cur_content == content.as_ref() {
                            FileStatus::Ok
                        } else {
                            FileStatus::Modified
                        };
                        Status::Generated {
                            want_content: content.clone(),
                            cur_content: Some(cur_content),
                            path,
                            status,
                        }
                    } else {
                        todo!()
                    }
                } else {
                    Status::Generated {
                        want_content: content.clone(),
                        cur_content: None,
                        path,
                        status: FileStatus::New,
                    }
                }
            }
            ResolvedFileSource::SymlinkFrom { full, rel } => {
                let status = match SymlinkInfo::from_path(&path.full) {
                    SymlinkInfo::LinksTo(link_path) => {
                        if link_path == full.as_ref() {
                            SymlinkStatus::Ok
                        } else {
                            SymlinkStatus::WrongLink(link_path)
                        }
                    }
                    SymlinkInfo::NotFound => SymlinkStatus::New,
                    SymlinkInfo::NotSymlinkIsFile => todo!(),
                    SymlinkInfo::NotSymlinkIsDir => todo!(),
                };
                Status::Symlink {
                    real: ResolvedConfigFilePath {
                        path: rel.clone(),
                        full: full.clone(),
                    },
                    symlink: path,
                    status,
                }
            }
        }
    }

    pub fn check_status(&self, output: &mut dyn Output) {
        for (path, entry) in &self.files {
            output.push_status(self.entry_status(path.clone(), entry));
        }
    }

    pub fn apply_changes(&self) -> Result<(), Error> {
        for (path, entry) in &self.files {
            let status = self.entry_status(path.clone(), entry);
            match &status {
                Status::Generated {
                    want_content,
                    cur_content: _,
                    path,
                    status,
                } => {
                    match status {
                        FileStatus::Ok => {
                            log::debug!("Config is already up to date {path}");
                            continue;
                        }
                        FileStatus::Modified => {
                            log::info!("Updating modified config {path}");
                        }
                        FileStatus::New => {
                            log::info!("Creating new config {path}");
                        }
                    }
                    std::fs::write(&path.full, want_content.as_bytes())
                        .map_err(|e| Error::FailedToWriteConfig(path.to_owned(), e))?;
                }
                Status::Symlink {
                    real,
                    symlink,
                    status,
                } => match status {
                    SymlinkStatus::Ok => {
                        log::debug!("Symlink from {symlink} to {real} is already in place",);
                        continue;
                    }
                    SymlinkStatus::WrongLink(_) => todo!(),
                    SymlinkStatus::New => {
                        log::info!("Creating symlink from {symlink} to {real}",);
                        create_symlink(&real.full, &symlink.full)?;
                    }
                    SymlinkStatus::IsFile => {
                        return Err(Error::RefusingToOverwriteFileWithSymlink {
                            symlink: symlink.clone(),
                            real: real.clone(),
                        });
                    }
                },
                Status::Git { repo, status } => todo!("{repo}: {status:?}"),
            }
        }

        Ok(())
    }
}

enum SymlinkInfo {
    LinksTo(PathBuf),
    NotFound,
    NotSymlinkIsFile,
    NotSymlinkIsDir,
}

impl SymlinkInfo {
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
