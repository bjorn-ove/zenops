//! The materialiser: turns the parsed `Config` into desired-state events
//! and applied-change events.
//!
//! [`ConfigFiles`] accumulates two kinds of intent — generated file content
//! and symlinks — keyed by [`ConfigFilePath`]. Each `ConfigFilePath` flavour
//! ([`ConfigFilePath::Home`], [`ConfigFilePath::DotConfig`],
//! [`ConfigFilePath::Zenops`]) is resolved against the matching root in
//! [`ConfigFileDirs`] (`~`, `~/.config`, `~/.config/zenops`).
//!
//! [`ConfigFiles::check_status`] (used by `zenops status`) compares the
//! desired state to disk and pushes [`crate::output::Status`] events.
//! [`ConfigFiles::apply_changes`] (used by `zenops apply`) walks the same
//! desired state, prompts via a [`crate::prompt::Prompter`], and pushes
//! [`crate::output::AppliedAction`] events for changes it actually writes.
//!
//! All managed paths are [`zenops_safe_relative_path::SafeRelativePath`] so
//! `..`-traversal can't escape the configured root.

use std::{
    borrow::Cow,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::Serialize;

use crate::{
    error::Error,
    output::{
        AppliedAction, Event, FileStatus, Output, ResolvedConfigFilePath, Status, SymlinkStatus,
    },
    prompt::{PendingChange, Prompter},
};
use similar::{DiffOp, TextDiff};
use zenops_safe_relative_path::SafeRelativePath;

/// A path relative to one of the three managed roots. The variant carries
/// the relative tail; resolving against a [`ConfigFileDirs`] yields a real
/// [`PathBuf`]. The relative tail is a [`SafeRelativePath`], so `..` can't
/// escape the root.
#[derive(Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "path", rename_all = "snake_case")]
pub enum ConfigFilePath {
    /// Rooted at `$HOME`.
    Home(Arc<SafeRelativePath>),
    /// Rooted at `$HOME/.config`.
    DotConfig(Arc<SafeRelativePath>),
    /// Rooted at `$HOME/.config/zenops` — the cloned config repo itself.
    Zenops(Arc<SafeRelativePath>),
}

impl ConfigFilePath {
    /// Construct a [`Self::Home`] path.
    pub fn in_home(path: impl AsRef<SafeRelativePath>) -> Self {
        Self::Home(Arc::from(path.as_ref()))
    }

    /// Construct a [`Self::DotConfig`] path. Underscored because no caller
    /// needs it today; kept for symmetry with the other constructors.
    pub fn _in_dot_config(path: impl AsRef<SafeRelativePath>) -> Self {
        Self::DotConfig(Arc::from(path.as_ref()))
    }

    /// Resolve to an absolute filesystem path against the matching root in
    /// `dirs`.
    pub fn resolved(&self, dirs: &ConfigFileDirs) -> PathBuf {
        match self {
            Self::Home(path) => path.to_full_path(&dirs.home),
            Self::DotConfig(path) => path.to_full_path(&dirs.config),
            Self::Zenops(path) => path.to_full_path(&dirs.zenops),
        }
    }

    /// Render with the symbolic root (`~`, `~/.config`, `~/.config/zenops`)
    /// for user-facing output. Use [`Self::resolved`] when you need a real
    /// path.
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

    /// The parent directory in the same root, or `None` if this path is
    /// already the root.
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

/// How a managed file gets its contents. Either zenops generates the body
/// itself (e.g. shell rc, gitconfig) or it points at a file inside the
/// zenops repo and creates a symlink.
pub enum ConfigFileSource {
    /// Body is rendered by zenops; the materialiser writes it verbatim.
    Generated(String),
    /// File is a symlink pointing at this in-repo path.
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

/// Internal subset of [`Status`] that `entry_status` can actually produce —
/// only the file-level variants. Lifting these out lets `apply_changes`
/// match exhaustively without an `unreachable!` arm for the channel-level
/// `Status::Git`/`GitRepoClean`/`Pkg` events that other code paths emit
/// directly.
#[derive(Debug)]
enum FileEntryStatus {
    Generated {
        want_content: Arc<str>,
        cur_content: Option<String>,
        path: ResolvedConfigFilePath,
        status: FileStatus,
    },
    Symlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
        status: SymlinkStatus,
    },
}

impl From<FileEntryStatus> for Status {
    fn from(s: FileEntryStatus) -> Self {
        match s {
            FileEntryStatus::Generated {
                want_content,
                cur_content,
                path,
                status,
            } => Status::Generated {
                want_content,
                cur_content,
                path,
                status,
            },
            FileEntryStatus::Symlink {
                real,
                symlink,
                status,
            } => Status::Symlink {
                real,
                symlink,
                status,
            },
        }
    }
}

/// The three resolved root paths every [`ConfigFilePath`] is anchored to.
/// Built once at startup from the user's home directory and threaded
/// through every command.
pub struct ConfigFileDirs {
    home: PathBuf,
    config: PathBuf,
    zenops: PathBuf,
}

impl ConfigFileDirs {
    /// Build the three roots from an absolute home directory. Panics if
    /// `home` is relative — every caller in production passes
    /// `home::home_dir()`, which is absolute on every supported platform.
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

    /// Absolute path of `$HOME`.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Absolute path of the cloned zenops config repo
    /// (`$HOME/.config/zenops`).
    pub fn zenops(&self) -> &Path {
        &self.zenops
    }
}

/// In-memory accumulator of every managed file: the desired body or
/// symlink target, indexed by its resolved absolute path so duplicates
/// from different declaration paths collapse. Insertion order is preserved
/// and drives the order of emitted [`Status`] / [`AppliedAction`] events.
pub struct ConfigFiles<'dirs> {
    dirs: &'dirs ConfigFileDirs,
    files: IndexMap<Arc<Path>, FileEntry>,
}

impl<'dirs> ConfigFiles<'dirs> {
    /// Empty accumulator bound to a set of resolved roots.
    pub fn new(dirs: &'dirs ConfigFileDirs) -> Self {
        Self {
            dirs,
            files: IndexMap::new(),
        }
    }

    /// Register a file. Subsequent inserts at the same resolved absolute
    /// path overwrite the previous entry (last-write-wins).
    pub fn add(&mut self, path: ConfigFilePath, src: ConfigFileSource) {
        self.files.insert(
            Arc::from(path.resolved(self.dirs)),
            FileEntry {
                path,
                src: src.into_resolved(self),
            },
        );
    }

    fn entry_status(&self, full: Arc<Path>, entry: &FileEntry) -> Result<FileEntryStatus, Error> {
        let path = ResolvedConfigFilePath {
            path: entry.path.clone(),
            full,
        };

        match &entry.src {
            ResolvedFileSource::Generated(content) => {
                if path.full.exists() {
                    let cur_content = std::fs::read_to_string(&path.full)
                        .map_err(|e| Error::FailedToReadConfig(path.clone(), e))?;
                    let status = if cur_content == content.as_ref() {
                        FileStatus::Ok
                    } else {
                        FileStatus::Modified
                    };
                    Ok(FileEntryStatus::Generated {
                        want_content: content.clone(),
                        cur_content: Some(cur_content),
                        path,
                        status,
                    })
                } else {
                    Ok(FileEntryStatus::Generated {
                        want_content: content.clone(),
                        cur_content: None,
                        path,
                        status: FileStatus::New,
                    })
                }
            }
            ResolvedFileSource::SymlinkFrom { full, rel } => {
                let status = match SymlinkInfo::from_path(&path.full)? {
                    SymlinkInfo::LinksTo(link_path) => {
                        if link_path == full.as_ref() {
                            match full.symlink_metadata() {
                                Ok(_) => SymlinkStatus::Ok,
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    SymlinkStatus::RealPathIsMissing
                                }
                                Err(e) => {
                                    return Err(Error::SymlinkProbeFailed(full.to_path_buf(), e));
                                }
                            }
                        } else {
                            SymlinkStatus::WrongLink(link_path)
                        }
                    }
                    SymlinkInfo::NotFound => match path.parent() {
                        None => SymlinkStatus::New,
                        Some(parent) => match parent.full.symlink_metadata() {
                            Ok(_) => SymlinkStatus::New,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                SymlinkStatus::DstDirIsMissing { dir: parent }
                            }
                            Err(e) => {
                                return Err(Error::SymlinkProbeFailed(
                                    parent.full.to_path_buf(),
                                    e,
                                ));
                            }
                        },
                    },
                    SymlinkInfo::NotSymlinkIsFile => SymlinkStatus::IsFile,
                    SymlinkInfo::NotSymlinkIsDir => SymlinkStatus::IsDir,
                    SymlinkInfo::NotSymlinkIsOther => SymlinkStatus::IsOther,
                };
                Ok(FileEntryStatus::Symlink {
                    real: ResolvedConfigFilePath {
                        path: rel.clone(),
                        full: full.clone(),
                    },
                    symlink: path,
                    status,
                })
            }
        }
    }

    /// Read-only pass: emit one [`Status`] per registered file describing
    /// how the live filesystem compares to the desired state. Used by
    /// `zenops status` and as the pre-change pass of `zenops apply`.
    pub fn check_status(&self, output: &mut dyn Output) -> Result<(), Error> {
        for (path, entry) in &self.files {
            output.push(Event::Status(
                self.entry_status(path.clone(), entry)?.into(),
            ))?;
        }
        Ok(())
    }

    /// Apply pass: walk every registered file in insertion order, prompt
    /// the [`Prompter`] for each pending change, and emit one
    /// [`AppliedAction`] per change actually written. A `false` from the
    /// prompter skips that change without aborting the rest of the run.
    pub fn apply_changes(
        &self,
        output: &mut dyn Output,
        prompter: &mut dyn Prompter,
    ) -> Result<(), Error> {
        for (path, entry) in &self.files {
            let status = self.entry_status(path.clone(), entry)?;
            match status {
                FileEntryStatus::Generated {
                    status: FileStatus::Ok,
                    ..
                } => {}
                FileEntryStatus::Generated {
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
                    if let Some(parent) = path.parent()
                        && !parent.full.exists()
                    {
                        std::fs::create_dir_all(&parent.full)
                            .map_err(|e| Error::CreateDirectoryError(parent.clone(), e))?;
                        output.push(Event::AppliedAction(AppliedAction::CreatedDir(parent)))?;
                    }
                    std::fs::write(&path.full, want_content.as_bytes())
                        .map_err(|e| Error::FailedToWriteConfig(path.to_owned(), e))?;
                    output.push(Event::AppliedAction(AppliedAction::CreatedFile(path)))?;
                }
                FileEntryStatus::Generated {
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
                    output.push(Event::AppliedAction(AppliedAction::UpdatedFile(path)))?;
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::Ok,
                    ..
                } => {}
                FileEntryStatus::Symlink {
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
                    create_symlink(&real, &symlink)?;
                    output.push(Event::AppliedAction(AppliedAction::CreatedSymlink {
                        real,
                        symlink,
                    }))?;
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::DstDirIsMissing { dir },
                    real,
                    symlink,
                } => {
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
                    output.push(Event::AppliedAction(AppliedAction::CreatedDir(dir)))?;
                    create_symlink(&real, &symlink)?;
                    output.push(Event::AppliedAction(AppliedAction::CreatedSymlink {
                        real,
                        symlink,
                    }))?;
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::IsFile,
                    real,
                    symlink,
                } => {
                    return Err(Error::RefusingToOverwriteFileWithSymlink { real, symlink });
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::IsDir,
                    real,
                    symlink,
                } => {
                    return Err(Error::RefusingToOverwriteDirectoryWithSymlink { real, symlink });
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::WrongLink(current_target),
                    real,
                    symlink,
                } => {
                    if !prompter.confirm(PendingChange::ReplaceWrongSymlink {
                        real: &real,
                        symlink: &symlink,
                        current_target: &current_target,
                    })? {
                        continue;
                    }
                    std::fs::remove_file(&symlink.full)
                        .map_err(|e| Error::SymlinkProbeFailed(symlink.full.to_path_buf(), e))?;
                    create_symlink(&real, &symlink)?;
                    output.push(Event::AppliedAction(AppliedAction::ReplacedSymlink {
                        real,
                        symlink,
                    }))?;
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::RealPathIsMissing,
                    real,
                    symlink,
                } => {
                    return Err(Error::SymlinkRealPathMissing { real, symlink });
                }
                FileEntryStatus::Symlink {
                    status: SymlinkStatus::IsOther,
                    symlink,
                    ..
                } => {
                    return Err(Error::RefusingToOverwriteOtherWithSymlink(symlink));
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
    NotSymlinkIsOther,
}

impl SymlinkInfo {
    pub fn from_path(p: impl AsRef<Path>) -> Result<Self, Error> {
        let p = p.as_ref();
        match p.symlink_metadata() {
            Ok(meta) => {
                if meta.is_symlink() {
                    match std::fs::read_link(p) {
                        Ok(link_path) => Ok(Self::LinksTo(link_path)),
                        Err(e) => Err(Error::SymlinkProbeFailed(p.to_path_buf(), e)),
                    }
                } else if meta.is_file() {
                    Ok(Self::NotSymlinkIsFile)
                } else if meta.is_dir() {
                    Ok(Self::NotSymlinkIsDir)
                } else {
                    Ok(Self::NotSymlinkIsOther)
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound) => Ok(Self::NotFound),
            Err(e) => Err(Error::SymlinkProbeFailed(p.to_path_buf(), e)),
        }
    }
}

#[cfg(unix)]
fn create_symlink(
    real: &ResolvedConfigFilePath,
    symlink: &ResolvedConfigFilePath,
) -> Result<(), Error> {
    std::os::unix::fs::symlink(&real.full, &symlink.full).map_err(|source| {
        Error::CreateSymlinkFailed {
            real: real.clone(),
            symlink: symlink.clone(),
            source,
        }
    })
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

    /// RAII guard that restores a directory's mode on drop so `tempfile`'s
    /// recursive cleanup can still descend after a test chmod-ed it to 0.
    struct ModeGuard {
        path: PathBuf,
        original: u32,
    }

    impl ModeGuard {
        fn chmod(path: PathBuf, mode: u32) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let original = std::fs::metadata(&path).unwrap().permissions().mode();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            Self { path, original }
        }
    }

    impl Drop for ModeGuard {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(self.original),
            );
        }
    }

    /// Build a `ConfigFileDirs` rooted at a fresh tempdir's `home/`.
    /// Returns the tempdir (drop = cleanup) and the dirs.
    fn fresh_dirs() -> (tempfile::TempDir, ConfigFileDirs) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".config/zenops")).unwrap();
        let dirs = ConfigFileDirs::load(home);
        (tmp, dirs)
    }

    #[test]
    fn entry_status_propagates_real_target_probe_error() {
        // Symlink at ~/foo correctly points at a file in the zenops repo, but
        // the file's parent dir has been chmod'd to 0o000 so symlink_metadata
        // on the real target returns PermissionDenied. Expect SymlinkProbeFailed.
        let (_tmp, dirs) = fresh_dirs();
        let real_dir = dirs.zenops().join("configs/foo");
        std::fs::create_dir_all(&real_dir).unwrap();
        let real_path = real_dir.join("bar.toml");
        std::fs::write(&real_path, b"# bar\n").unwrap();

        let symlink_path = dirs.home().join("foo");
        std::os::unix::fs::symlink(&real_path, &symlink_path).unwrap();

        let mut config_files = ConfigFiles::new(&dirs);
        config_files.add(
            ConfigFilePath::in_home(SafeRelativePath::from_relative_path("foo").unwrap()),
            ConfigFileSource::SymlinkFrom(ConfigFilePath::Zenops(Arc::from(
                SafeRelativePath::from_relative_path("configs/foo/bar.toml").unwrap(),
            ))),
        );

        let _guard = ModeGuard::chmod(real_dir, 0o000);

        let (path, entry) = config_files.files.first().unwrap();
        let result = config_files.entry_status(path.clone(), entry);
        match result {
            Err(Error::SymlinkProbeFailed(p, e)) => {
                assert_eq!(p, real_path);
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected SymlinkProbeFailed, got {other:?}"),
        }
    }

    /// Build a `ResolvedConfigFilePath` rooted in `home` for tests that
    /// poke at `create_symlink` directly.
    fn home_resolved(home: &Path, rel: &str) -> ResolvedConfigFilePath {
        let srp = SafeRelativePath::from_relative_path(rel).unwrap();
        let full = srp.to_full_path(home);
        ResolvedConfigFilePath {
            path: ConfigFilePath::in_home(srp),
            full: Arc::from(full),
        }
    }

    #[test]
    fn create_symlink_already_exists() {
        let (_tmp, dirs) = fresh_dirs();
        let real = home_resolved(dirs.home(), "real");
        std::fs::write(&real.full, b"real\n").unwrap();
        let symlink = home_resolved(dirs.home(), "dst");
        std::fs::write(&symlink.full, b"in the way\n").unwrap();

        match create_symlink(&real, &symlink) {
            Err(Error::CreateSymlinkFailed { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
            }
            other => panic!("expected CreateSymlinkFailed/AlreadyExists, got {other:?}"),
        }
    }

    #[test]
    fn create_symlink_parent_missing() {
        let (_tmp, dirs) = fresh_dirs();
        let real = home_resolved(dirs.home(), "real");
        std::fs::write(&real.full, b"real\n").unwrap();
        let symlink = home_resolved(dirs.home(), "no_such_dir/dst");

        match create_symlink(&real, &symlink) {
            Err(Error::CreateSymlinkFailed { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected CreateSymlinkFailed/NotFound, got {other:?}"),
        }
    }

    #[test]
    fn create_symlink_parent_not_writable() {
        let (_tmp, dirs) = fresh_dirs();
        let real = home_resolved(dirs.home(), "real");
        std::fs::write(&real.full, b"real\n").unwrap();
        let read_only_dir = dirs.home().join("ro");
        std::fs::create_dir(&read_only_dir).unwrap();
        let symlink = home_resolved(dirs.home(), "ro/dst");

        let _guard = ModeGuard::chmod(read_only_dir, 0o555);

        match create_symlink(&real, &symlink) {
            Err(Error::CreateSymlinkFailed { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected CreateSymlinkFailed/PermissionDenied, got {other:?}"),
        }
    }

    #[test]
    fn entry_status_propagates_probe_error_when_path_unreachable() {
        // The symlink path's parent has been chmod'd to 0o000 so traversal
        // into it fails with PermissionDenied. SymlinkInfo::from_path can't
        // even lstat the path; expect SymlinkProbeFailed surfacing the
        // symlink path that couldn't be probed.
        //
        // (This is the trigger for both the from_path catch-all and the
        // entry_status dst-parent catch-all: the same chmod hits from_path
        // first. The dst-parent branch is preserved as defensive propagation
        // for the race-only edge case where the path itself returns NotFound
        // but probing the parent returns a non-NotFound I/O error.)
        let (_tmp, dirs) = fresh_dirs();

        let real_path = dirs.zenops().join("configs/foo/bar.toml");
        std::fs::create_dir_all(real_path.parent().unwrap()).unwrap();
        std::fs::write(&real_path, b"# bar\n").unwrap();

        let sub = dirs.home().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let symlink_path = sub.join("foo");

        let mut config_files = ConfigFiles::new(&dirs);
        config_files.add(
            ConfigFilePath::in_home(SafeRelativePath::from_relative_path("sub/foo").unwrap()),
            ConfigFileSource::SymlinkFrom(ConfigFilePath::Zenops(Arc::from(
                SafeRelativePath::from_relative_path("configs/foo/bar.toml").unwrap(),
            ))),
        );

        let _guard = ModeGuard::chmod(sub.clone(), 0o000);

        let (path, entry) = config_files.files.first().unwrap();
        let result = config_files.entry_status(path.clone(), entry);
        match result {
            Err(Error::SymlinkProbeFailed(p, e)) => {
                assert_eq!(p, symlink_path);
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected SymlinkProbeFailed, got {other:?}"),
        }
    }
}
