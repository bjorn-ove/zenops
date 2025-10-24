use safe_relative_path::{SafeRelativePath, srpath};
use std::{io::Write, path::PathBuf, sync::Arc};
use xshell::{Shell, cmd};
use zenops::{
    Args, Cmd,
    config_files::{ConfigFileDirs, ConfigFilePath},
    error::Error,
    output::{AppliedAction, ResolvedConfigFilePath, Status},
};

pub mod paths {
    use safe_relative_path::{SafeRelativePath, srpath};

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
            default_args: Args {},
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

    pub fn run(&self, cmd: &Cmd) -> Result<Output, Error> {
        let mut output = Output::default();
        zenops::real_main(&self.default_args, cmd, &self.dirs, &mut output)?;
        Ok(output)
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
}

#[derive(Debug, PartialEq)]
pub enum Entry {
    Status(Status),
    AppliedAction(AppliedAction),
}

#[derive(Default, Debug, PartialEq)]
pub struct Output {
    pub entries: Vec<Entry>,
}

impl zenops::output::Output for Output {
    fn push_status(&mut self, status: Status) {
        self.entries.push(Entry::Status(status))
    }

    fn push_applied_action(&mut self, action: AppliedAction) {
        self.entries.push(Entry::AppliedAction(action));
    }
}
