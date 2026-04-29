// Shared across multiple integration-test binaries; each binary uses a
// different subset, so per-binary dead-code warnings are expected.
#![allow(dead_code)]

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use xshell::{Shell, cmd};
use zenops::{
    Args, Cmd, ColorChoice,
    config_files::{ConfigFileDirs, ConfigFilePath},
    error::Error,
    output::{
        AppliedAction, BootstrapSummary, DoctorCheck, InitSummary, OutputError, PkgEntry,
        ResolvedConfigFilePath, Status,
    },
    pkg_list,
};
use zenops_safe_relative_path::{SafeRelativePath, srpath};

pub mod paths {
    use zenops_safe_relative_path::{SafeRelativePath, srpath};

    pub const HOME_DIR: &SafeRelativePath = srpath!("home/bob");
    pub const CONFIG_DIR: &SafeRelativePath = srpath!("home/bob/.config");
    pub const ZENOPS_DIR: &SafeRelativePath = srpath!("home/bob/.config/zenops");
    pub const ZENOPS_CONFIG: &SafeRelativePath = srpath!("home/bob/.config/zenops/config.toml");
}

pub struct TestEnv {
    #[allow(dead_code)] // Needed for automatic cleanup
    root: tempfile::TempDir,
    dirs: ConfigFileDirs,
    default_args: Args,
    sh: Shell,
}

impl TestEnv {
    pub fn load() -> Self {
        let root = tempfile::tempdir().unwrap();
        let dirs = ConfigFileDirs::load(root.path().join("home/bob"));
        let sh = Shell::new().unwrap();
        sh.change_dir(root.path());
        std::fs::create_dir_all(paths::ZENOPS_DIR.to_full_path(root.path())).unwrap();
        Self {
            root,
            dirs,
            default_args: Args {
                color: ColorChoice::Never,
            },
            sh,
        }
    }

    pub fn resolve_path(&self, path: impl AsRef<SafeRelativePath>) -> PathBuf {
        path.as_ref().to_full_path(self.root.path())
    }

    pub fn cfpath(
        &self,
        path: impl AsRef<str>,
        map: impl FnOnce(Arc<SafeRelativePath>) -> ConfigFilePath,
    ) -> ResolvedConfigFilePath {
        let path = map(Arc::from(
            SafeRelativePath::from_relative_path(path.as_ref()).unwrap(),
        ));
        let full = path.resolved(&self.dirs);

        ResolvedConfigFilePath {
            path,
            full: Arc::from(full),
        }
    }

    /// The `Status::GitRepoClean` entry emitted for this env's zenops repo
    /// when it has no uncommitted changes. Every test whose zenops repo is
    /// in a clean state has this prepended to its expected entries.
    pub fn git_repo_clean_entry(&self) -> Entry {
        Entry::Status(Status::GitRepoClean {
            repo: self.cfpath("", ConfigFilePath::Zenops),
        })
    }

    pub fn ensure_dir_exists_for_file(&self, file_path: impl AsRef<SafeRelativePath>) {
        if let Some(dir) = file_path.as_ref().safe_parent() {
            let path = dir.to_full_path(self.root.path());
            std::fs::create_dir_all(&path)
                .unwrap_or_else(|e| panic!("Failed to create directory {path:?}: {e}"));
        }
    }

    pub fn write_file(&self, path: impl AsRef<SafeRelativePath>, data: impl AsRef<[u8]>) {
        let path = path.as_ref();
        self.ensure_dir_exists_for_file(path);
        let full_path = path.to_full_path(self.root.path());
        std::fs::write(&full_path, data)
            .unwrap_or_else(|e| panic!("Failed to write to {path}: {e}\nFull path: {full_path:?}"));
    }

    pub fn delete_file(&self, path: impl AsRef<SafeRelativePath>) {
        let path = path.as_ref();
        self.ensure_dir_exists_for_file(path);
        let full_path = path.to_full_path(self.root.path());
        std::fs::remove_file(&full_path)
            .unwrap_or_else(|e| panic!("Failed to delete {path}: {e}\nFull path: {full_path:?}"));
    }

    pub fn delete_dir_all(&self, path: impl AsRef<SafeRelativePath>) {
        let path = path.as_ref();
        self.ensure_dir_exists_for_file(path);
        let full_path = path.to_full_path(self.root.path());
        std::fs::remove_dir_all(&full_path)
            .unwrap_or_else(|e| panic!("Failed to delete {path}: {e}\nFull path: {full_path:?}"));
    }

    pub fn create_dir(&self, path: impl AsRef<SafeRelativePath>) {
        let path = path.as_ref();
        let full_path = path.to_full_path(self.root.path());
        std::fs::create_dir_all(&full_path).unwrap_or_else(|e| {
            panic!("Failed to create directory {path}: {e}\nFull path: {full_path:?}")
        });
    }

    pub fn append_file(&self, path: impl AsRef<SafeRelativePath>, data: impl AsRef<[u8]>) {
        let path = path.as_ref().to_full_path(self.root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .unwrap()
            .write_all(data.as_ref())
            .unwrap();
    }

    pub fn write_zenops_file(
        &self,
        path: impl AsRef<SafeRelativePath>,
        data: impl AsRef<[u8]>,
        commit: Option<&str>,
    ) {
        let path = path.as_ref();
        self.write_file(paths::ZENOPS_DIR.safe_join(path), data);
        if let Some(message) = commit {
            self.zenops_shell(|sh| {
                cmd!(sh, "git add {path}").ignore_stdout().run().unwrap();
                cmd!(sh, "git commit -m {message}")
                    .ignore_stdout()
                    .run()
                    .unwrap();
            });
        }
    }

    pub fn append_zenops_file(
        &self,
        path: impl AsRef<SafeRelativePath>,
        data: impl AsRef<[u8]>,
        commit: Option<&str>,
    ) {
        let path = path.as_ref();
        self.append_file(paths::ZENOPS_DIR.safe_join(path), data);
        if let Some(message) = commit {
            self.zenops_shell(|sh| {
                cmd!(sh, "git add {path}").ignore_stdout().run().unwrap();
                cmd!(sh, "git commit -m {message}")
                    .ignore_stdout()
                    .run()
                    .unwrap();
            });
        }
    }

    fn zenops_shell(&self, f: impl FnOnce(&Shell)) {
        let _dir = self.sh.push_dir(paths::ZENOPS_DIR.as_str());
        f(&self.sh)
    }

    pub fn init_config(&self, config: &str) {
        self.zenops_shell(|sh| {
            cmd!(sh, "git init").ignore_stdout().run().unwrap();
            cmd!(sh, "git config commit.gpgsign false").run().unwrap();
        });
        self.write_zenops_file(srpath!("config.toml"), config, Some("initial commit"));
    }

    /// Like [`Self::init_config`], but also creates a bare-repo remote named
    /// `origin` under `<tmp>/remote.git` and pushes the initial commit to its
    /// `main` branch. Returns the bare repo's path so tests can inspect it.
    pub fn init_config_with_remote(&self, config: &str) -> PathBuf {
        let bare = self.root.path().join("remote.git");
        cmd!(self.sh, "git init --bare")
            .arg(&bare)
            .ignore_stdout()
            .run()
            .unwrap();
        self.init_config(config);
        self.zenops_shell(|sh| {
            // Rename regardless of the host's init.defaultBranch setting.
            cmd!(sh, "git branch -M main").run().unwrap();
            cmd!(sh, "git remote add origin").arg(&bare).run().unwrap();
            cmd!(sh, "git push -u origin main")
                .ignore_stdout()
                .ignore_stderr()
                .run()
                .unwrap();
        });
        bare
    }

    /// Create a bare repo at `<tmp>/origin.git` seeded with the given files
    /// on `main`, without touching the test env's zenops dir. Returns the
    /// bare repo's path; tests can pass it as a `file:///...` URL to
    /// `Cmd::Init`.
    pub fn seed_bare_repo(&self, files: &[(&str, &str)]) -> PathBuf {
        let bare = self.root.path().join("origin.git");
        let seed = self.root.path().join("seed");
        cmd!(self.sh, "git init --bare")
            .arg(&bare)
            .ignore_stdout()
            .run()
            .unwrap();
        std::fs::create_dir(&seed).unwrap();
        let _dir = self.sh.push_dir(&seed);
        cmd!(self.sh, "git init")
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .unwrap();
        cmd!(self.sh, "git config commit.gpgsign false")
            .run()
            .unwrap();
        for (name, content) in files {
            std::fs::write(seed.join(name), content).unwrap();
            cmd!(self.sh, "git add").arg(name).run().unwrap();
        }
        cmd!(self.sh, "git commit -m seed")
            .ignore_stdout()
            .run()
            .unwrap();
        cmd!(self.sh, "git branch -M main").run().unwrap();
        cmd!(self.sh, "git remote add origin")
            .arg(&bare)
            .run()
            .unwrap();
        cmd!(self.sh, "git push -u origin main")
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .unwrap();
        bare
    }

    /// Clone the given bare repo into `<tmp>/sidecar`, add a file, commit,
    /// and push. Used to seed a new upstream commit that the zenops repo can
    /// then pull.
    pub fn seed_remote_commit(
        &self,
        bare: &Path,
        filename: &str,
        content: &str,
        message: &str,
    ) -> PathBuf {
        let sidecar = self.root.path().join("sidecar");
        cmd!(self.sh, "git clone")
            .arg(bare)
            .arg(&sidecar)
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .unwrap();
        let _dir = self.sh.push_dir(&sidecar);
        cmd!(self.sh, "git config commit.gpgsign false")
            .run()
            .unwrap();
        std::fs::write(sidecar.join(filename), content).unwrap();
        cmd!(self.sh, "git add").arg(filename).run().unwrap();
        cmd!(self.sh, "git commit -m {message}")
            .ignore_stdout()
            .run()
            .unwrap();
        cmd!(self.sh, "git push")
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .unwrap();
        sidecar
    }

    /// Run `git <args>` in `dir` and return its stdout.
    pub fn git_out(&self, dir: &Path, args: &[&str]) -> String {
        let _dir = self.sh.push_dir(dir);
        cmd!(self.sh, "git").args(args).read().unwrap()
    }

    pub fn run(&self, cmd: &Cmd) -> Result<Output, Error> {
        let mut output = Output::default();
        zenops::real_main(&self.default_args, cmd, &self.dirs, &mut output)?;
        Ok(output)
    }

    /// Run `zenops pkg` and return only the `PkgEntry` events that came back.
    /// Convenience for the pkg listing tests, which don't care about
    /// status/git events. Takes `pkg_list::Options` so new flags don't keep
    /// growing this signature.
    pub fn run_pkg_list(&self, opts: pkg_list::Options) -> Result<Vec<PkgEntry>, Error> {
        let pkg_list::Options {
            pattern,
            all,
            all_hints,
            verbose,
        } = opts;
        let out = self.run(&Cmd::Pkg {
            pattern,
            all,
            all_hints,
            verbose,
        })?;
        Ok(out
            .entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Pkg(p) => Some(p),
                _ => None,
            })
            .collect())
    }

    pub fn create_symlink(
        &self,
        real_path: impl AsRef<SafeRelativePath>,
        symlink_path: impl AsRef<SafeRelativePath>,
    ) {
        self.ensure_dir_exists_for_file(symlink_path.as_ref());
        let real_path = self.resolve_path(real_path);
        let symlink_path = self.resolve_path(symlink_path);
        std::os::unix::fs::symlink(&real_path, &symlink_path).unwrap_or_else(|e| {
            panic!("Failed to create symlink from {real_path:?} to {symlink_path:?}: {e}")
        });
    }

    pub fn create_dangling_symlink(
        &self,
        real_path: impl AsRef<Path>,
        symlink_path: impl AsRef<SafeRelativePath>,
    ) {
        self.ensure_dir_exists_for_file(symlink_path.as_ref());
        let symlink_path = self.resolve_path(symlink_path);
        std::os::unix::fs::symlink(real_path.as_ref(), &symlink_path).unwrap_or_else(|e| {
            panic!(
                "Failed to create symlink from {:?} to {symlink_path:?}: {e}",
                real_path.as_ref()
            )
        });
    }

    /// Set the mode bits on `rel`. Returns a guard that restores the
    /// original mode on drop so `tempfile` cleanup can recurse into the
    /// directory at the end of the test.
    pub fn chmod(&self, rel: impl AsRef<SafeRelativePath>, mode: u32) -> PermGuard {
        use std::os::unix::fs::PermissionsExt;
        let path = self.resolve_path(rel);
        let original = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("Failed to read metadata for {path:?}: {e}"))
            .permissions()
            .mode();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .unwrap_or_else(|e| panic!("Failed to chmod {path:?} to {mode:o}: {e}"));
        PermGuard { path, original }
    }
}

/// RAII guard that restores a path's original Unix mode on drop. Tests
/// that chmod managed paths use this so `tempfile`'s recursive cleanup
/// can still descend into them after the test body completes.
pub struct PermGuard {
    path: PathBuf,
    original: u32,
}

impl Drop for PermGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ =
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.original));
    }
}

#[derive(Debug, PartialEq)]
pub enum Entry {
    Status(Status),
    AppliedAction(AppliedAction),
    Pkg(PkgEntry),
    Doctor(DoctorCheck),
    Init(InitSummary),
    Bootstrap(BootstrapSummary),
}

#[derive(Default, Debug, PartialEq)]
pub struct Output {
    pub entries: Vec<Entry>,
}

impl zenops::output::Output for Output {
    fn push_status(&mut self, status: Status) -> Result<(), OutputError> {
        self.entries.push(Entry::Status(status));
        Ok(())
    }

    fn push_applied_action(&mut self, action: AppliedAction) -> Result<(), OutputError> {
        self.entries.push(Entry::AppliedAction(action));
        Ok(())
    }

    fn push_pkg_entry(&mut self, entry: PkgEntry) -> Result<(), OutputError> {
        self.entries.push(Entry::Pkg(entry));
        Ok(())
    }

    fn push_doctor_check(&mut self, check: DoctorCheck) -> Result<(), OutputError> {
        self.entries.push(Entry::Doctor(check));
        Ok(())
    }

    fn push_init_summary(&mut self, summary: InitSummary) -> Result<(), OutputError> {
        self.entries.push(Entry::Init(summary));
        Ok(())
    }

    fn push_bootstrap_summary(&mut self, summary: BootstrapSummary) -> Result<(), OutputError> {
        self.entries.push(Entry::Bootstrap(summary));
        Ok(())
    }
}
