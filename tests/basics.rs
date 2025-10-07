use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use safe_relative_path::{SafeRelativePath, srpath};
use xshell::{Shell, cmd};
use zenops::{
    Args, Cmd,
    config_files::{ConfigFileDirs, ConfigFilePath},
    error::Error,
    git::GitFileStatus,
    output::{ResolvedConfigFilePath, Status},
};

#[derive(Debug, PartialEq)]
enum Entry {
    Status(Status),
}

#[derive(Default, Debug, PartialEq)]
pub struct Output {
    entries: Vec<Entry>,
}

impl zenops::output::Output for Output {
    fn push_status(&mut self, status: Status) {
        self.entries.push(Entry::Status(status))
    }
}

const CONFIG_DIR: &SafeRelativePath = srpath!("home/bob/.config/zenops");
const CONFIG_FILE: &SafeRelativePath = srpath!("home/bob/.config/zenops/config.toml");

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
        Self {
            root,
            dirs,
            default_args: Args {},
            sh,
        }
    }

    pub fn resolve_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref().to_str().unwrap();
        let path = SafeRelativePath::from_relative_path(path).unwrap();
        path.to_path(self.root.path())
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

    pub fn write_file(&self, path: impl AsRef<SafeRelativePath>, data: impl AsRef<[u8]>) {
        let path = path.as_ref().to_path(self.root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, data).unwrap();
    }

    pub fn append_file(&self, path: &SafeRelativePath, data: impl AsRef<[u8]>) {
        let path = path.to_path(self.root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .unwrap()
            .write_all(data.as_ref())
            .unwrap();
    }

    pub fn init_config(&self) {
        self.write_file(CONFIG_FILE, "");
        let _dir = self.sh.push_dir("home/bob/.config/zenops");
        cmd!(self.sh, "git init").ignore_stdout().run().unwrap();
        cmd!(self.sh, "git config commit.gpgsign false")
            .run()
            .unwrap();
        cmd!(self.sh, "git add config.toml")
            .ignore_stdout()
            .run()
            .unwrap();
        cmd!(self.sh, "git commit -m initial")
            .ignore_stdout()
            .run()
            .unwrap();
    }

    pub fn run(&self, cmd: &Cmd) -> Result<Output, Error> {
        let mut output = Output::default();
        zenops::real_main(&self.default_args, cmd, &self.dirs, &mut output)?;
        Ok(output)
    }
}

#[test]
fn missing_config() {
    let env = TestEnv::load();

    assert_eq!(
        env.run(&Cmd::Status),
        Err(Error::OpenDb(
            env.root.path().join("home/bob/.config/zenops/config.toml"),
            std::io::ErrorKind::NotFound.into()
        ))
    );

    // Check it works with a config file, but no .git repository
    env.write_file(CONFIG_FILE, "");

    assert_eq!(env.run(&Cmd::Status), Ok(Output { entries: vec![] }),);

    // Check it works with a .git repository
    env.init_config();

    assert_eq!(env.run(&Cmd::Status), Ok(Output { entries: vec![] }),);
}

#[test]
fn config_dir_git_status() {
    let env = TestEnv::load();

    env.init_config();

    assert_eq!(env.run(&Cmd::Status), Ok(Output { entries: vec![] }));

    env.append_file(CONFIG_FILE, "# Modification");
    env.write_file(
        CONFIG_DIR.safe_join(srpath!("untracked-file")),
        "# Untracked file",
    );

    assert_eq!(
        env.run(&Cmd::Status),
        Ok(Output {
            entries: vec![
                Entry::Status(Status::Git {
                    repo: env.cfpath("", ConfigFilePath::Zenops),
                    status: GitFileStatus::Modified(srpath!("config.toml").into()),
                }),
                Entry::Status(Status::Git {
                    repo: env.cfpath("", ConfigFilePath::Zenops),
                    status: GitFileStatus::Untracked(srpath!("untracked-file").into()),
                })
            ]
        })
    );
}
