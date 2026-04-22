use std::{
    borrow::Cow,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;
use serde::Serialize;

use crate::{
    error::Error,
    output::{AppliedAction, FileStatus, Output, ResolvedConfigFilePath, Status, SymlinkStatus},
    prompt::{PendingChange, Prompter},
};
use similar::{DiffOp, TextDiff};
use zenops_safe_relative_path::SafeRelativePath;

#[derive(Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "path", rename_all = "snake_case")]
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
            Self::Home(path) => path.to_full_path(&dirs.home),
            Self::DotConfig(path) => path.to_full_path(&dirs.config),
            Self::Zenops(path) => path.to_full_path(&dirs.zenops),
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

    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Home(path) => Some(Self::Home(Arc::from(path.safe_parent()?))),
            Self::DotConfig(path) => Some(Self::DotConfig(Arc::from(path.safe_parent()?))),
            Self::Zenops(path) => Some(Self::Zenops(Arc::from(path.safe_parent()?))),
        }
    }
}

impl std::fmt::Debug for ConfigFilePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt::Debug::fmt(&self.human_path(), f)
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

    pub fn home(&self) -> &Path {
        &self.home
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
                            match full.symlink_metadata() {
                                Ok(_) => SymlinkStatus::Ok,
                                Err(e) => match e.kind() {
                                    std::io::ErrorKind::NotFound => {
                                        SymlinkStatus::RealPathIsMissing
                                    }
                                    unk => todo!("{unk:?}"),
                                },
                            }
                        } else {
                            SymlinkStatus::WrongLink(link_path)
                        }
                    }
                    SymlinkInfo::NotFound => {
                        match path.full.parent().map(|v| v.symlink_metadata()) {
                            None | Some(Ok(_)) => SymlinkStatus::New,
                            Some(Err(e)) => match e.kind() {
                                std::io::ErrorKind::NotFound => SymlinkStatus::DstDirIsMissing,
                                unk => todo!("{unk:?}"),
                            },
                        }
                    }
                    SymlinkInfo::NotSymlinkIsFile => SymlinkStatus::IsFile,
                    SymlinkInfo::NotSymlinkIsDir => SymlinkStatus::IsDir,
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

    pub fn check_status(&self, output: &mut dyn Output) -> Result<(), Error> {
        for (path, entry) in &self.files {
            output.push_status(self.entry_status(path.clone(), entry))?;
        }
        Ok(())
    }

    pub fn apply_changes(
        &self,
        output: &mut dyn Output,
        prompter: &mut dyn Prompter,
    ) -> Result<(), Error> {
        for (path, entry) in &self.files {
            let status = self.entry_status(path.clone(), entry);
            match status {
                Status::Generated {
                    status: FileStatus::Ok,
                    ..
                } => {}
                Status::Generated {
                    status: FileStatus::New,
                    want_content,
                    path,
                    ..
                } => {
                    if !prompter.confirm(PendingChange::CreateFile {
                        path: &path,
                        content: &want_content,
                    })? {
                        continue;
                    }
                    std::fs::write(&path.full, want_content.as_bytes())
                        .map_err(|e| Error::FailedToWriteConfig(path.to_owned(), e))?;
                    output.push_applied_action(AppliedAction::CreatedFile(path))?;
                }
                Status::Generated {
                    status: FileStatus::Modified,
                    want_content,
                    cur_content,
                    path,
                } => {
                    let cur = cur_content.as_deref().unwrap_or("");
                    let want = want_content.as_ref();
                    let diff = TextDiff::from_lines(cur, want);
                    let groups = diff.grouped_ops(3);
                    let total = groups.len();
                    let mut approvals = Vec::with_capacity(total);
                    for (i, ops) in groups.iter().enumerate() {
                        approvals.push(prompter.confirm(PendingChange::UpdateFileHunk {
                            path: &path,
                            index: i + 1,
                            total,
                            diff: &diff,
                            ops,
                        })?);
                    }
                    if !approvals.iter().any(|&a| a) {
                        continue;
                    }
                    let content = reconstruct(cur, want, &groups, &approvals);
                    std::fs::write(&path.full, content.as_bytes())
                        .map_err(|e| Error::FailedToWriteConfig(path.to_owned(), e))?;
                    output.push_applied_action(AppliedAction::UpdatedFile(path))?;
                }
                Status::Symlink {
                    status: SymlinkStatus::Ok,
                    ..
                } => {}
                Status::Symlink {
                    status: SymlinkStatus::New,
                    real,
                    symlink,
                } => {
                    if !prompter.confirm(PendingChange::CreateSymlink {
                        real: &real,
                        symlink: &symlink,
                    })? {
                        continue;
                    }
                    create_symlink(&real.full, &symlink.full)?;
                    output.push_applied_action(AppliedAction::CreatedSymlink { real, symlink })?;
                }
                Status::Symlink {
                    status: SymlinkStatus::DstDirIsMissing,
                    real,
                    symlink,
                } => {
                    let dir = symlink.parent().unwrap_or_else(|| {
                        todo!("This should not be possible due to earlier check")
                    });
                    if !prompter.confirm(PendingChange::CreateSymlinkWithParent {
                        real: &real,
                        symlink: &symlink,
                        parent: &dir,
                    })? {
                        continue;
                    }
                    match std::fs::create_dir_all(&dir.full) {
                        Ok(()) => {}
                        Err(e) => return Err(Error::CreateDirectoryError(dir, e)),
                    }
                    output.push_applied_action(AppliedAction::CreatedDir(dir))?;
                    create_symlink(&real.full, &symlink.full)?;
                    output.push_applied_action(AppliedAction::CreatedSymlink { real, symlink })?;
                }
                Status::Symlink {
                    status: SymlinkStatus::IsFile,
                    real,
                    symlink,
                } => {
                    return Err(Error::RefusingToOverwriteFileWithSymlink { real, symlink });
                }
                Status::Symlink {
                    status: SymlinkStatus::IsDir,
                    real,
                    symlink,
                } => {
                    return Err(Error::RefusingToOverwriteDirectoryWithSymlink { real, symlink });
                }
                Status::Symlink {
                    status: SymlinkStatus::WrongLink(_) | SymlinkStatus::RealPathIsMissing,
                    ..
                } => todo!(),
                Status::Git { repo, status } => todo!("{repo}: {status:?}"),
                Status::GitRepoClean { .. } | Status::Pkg { .. } => {
                    unreachable!(
                        "GitRepoClean/Pkg events are pushed directly to Output, not through ConfigFiles",
                    )
                }
            }
        }

        Ok(())
    }
}

fn reconstruct(old: &str, new: &str, groups: &[Vec<DiffOp>], approvals: &[bool]) -> String {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut out = String::with_capacity(old.len().max(new.len()));
    let mut old_idx = 0;

    for (group, &approved) in groups.iter().zip(approvals) {
        let group_old_start = group.first().expect("non-empty group").old_range().start;
        let group_old_end = group.last().expect("non-empty group").old_range().end;

        while old_idx < group_old_start {
            out.push_str(old_lines[old_idx]);
            old_idx += 1;
        }

        if approved {
            for op in group {
                match *op {
                    DiffOp::Equal { old_index, len, .. } => {
                        for i in 0..len {
                            out.push_str(old_lines[old_index + i]);
                        }
                    }
                    DiffOp::Delete { .. } => {}
                    DiffOp::Insert {
                        new_index, new_len, ..
                    }
                    | DiffOp::Replace {
                        new_index, new_len, ..
                    } => {
                        for i in 0..new_len {
                            out.push_str(new_lines[new_index + i]);
                        }
                    }
                }
            }
        } else {
            while old_idx < group_old_end {
                out.push_str(old_lines[old_idx]);
                old_idx += 1;
            }
        }
        old_idx = group_old_end;
    }

    while old_idx < old_lines.len() {
        out.push_str(old_lines[old_idx]);
        old_idx += 1;
    }
    out
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
    use std::io::ErrorKind;

    match std::os::unix::fs::symlink(real_path, symlink_path) {
        Ok(()) => Ok(()),
        Err(e) => match e.kind() {
            ErrorKind::AlreadyExists => todo!(),
            ErrorKind::NotFound => todo!(),
            unk => todo!("{unk:?}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups_for(old: &str, new: &str) -> Vec<Vec<DiffOp>> {
        TextDiff::from_lines(old, new).grouped_ops(3)
    }

    #[test]
    fn reconstruct_all_approved_equals_want() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        let groups = groups_for(old, new);
        let approvals = vec![true; groups.len()];
        assert_eq!(reconstruct(old, new, &groups, &approvals), new);
    }

    #[test]
    fn reconstruct_none_approved_equals_cur() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        let groups = groups_for(old, new);
        let approvals = vec![false; groups.len()];
        assert_eq!(reconstruct(old, new, &groups, &approvals), old);
    }

    #[test]
    fn reconstruct_mixed_applies_only_approved_hunks() {
        // Two distant hunks so grouped_ops(3) splits them.
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        let new = "1\nX\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\nY\n15\n";
        let groups = groups_for(old, new);
        assert_eq!(groups.len(), 2, "expected two separate hunks");
        // Approve only the first hunk.
        let approvals = vec![true, false];
        let got = reconstruct(old, new, &groups, &approvals);
        let expected = "1\nX\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        assert_eq!(got, expected);
        // And the opposite: approve only the second hunk.
        let approvals = vec![false, true];
        let got = reconstruct(old, new, &groups, &approvals);
        let expected = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\nY\n15\n";
        assert_eq!(got, expected);
    }
}
