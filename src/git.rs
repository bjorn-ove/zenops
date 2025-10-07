use std::path::Path;

use safe_relative_path::{SafeRelativePath, SafeRelativePathBuf};
use xshell::{Shell, cmd};

use crate::error::Error;

#[derive(Debug, PartialEq, Clone)]
pub enum GitFileStatus {
    Modified(SafeRelativePathBuf),
    Untracked(SafeRelativePathBuf),
}

pub struct Git<'path, 'shell> {
    path: &'path Path,
    sh: &'shell Shell,
}

impl<'path, 'shell> Git<'path, 'shell> {
    pub fn new(path: &'path Path, sh: &'shell Shell) -> Self {
        Self { path, sh }
    }

    pub fn is_git_repo(&self) -> Result<bool, Error> {
        let Self { path, sh } = self;
        match cmd!(sh, "git -C {path} rev-parse --is-inside-work-tree")
            .quiet()
            .ignore_status()
            .ignore_stderr()
            .read()?
            .trim()
        {
            "" => Ok(false),
            "true" => Ok(true),
            unk => todo!("{unk:?}"),
        }
    }

    pub fn status(&self) -> Result<Vec<GitFileStatus>, Error> {
        let Self { path, sh } = self;
        let mut ret = Vec::new();
        for line in cmd!(sh, "git -C {path} status --porcelain=v2")
            .quiet()
            .read()?
            .lines()
        {
            let mut cur = line;
            let mut next_arg = || {
                if cur.is_empty() {
                    return None;
                }
                let (ret, remain) = cur.split_once(' ').unwrap_or((cur, ""));
                cur = remain;
                Some(ret)
            };
            match next_arg().unwrap() {
                "1" => {
                    let xy_status = next_arg().unwrap();
                    let _submodule_state = next_arg().unwrap();
                    let _mode_head = next_arg().unwrap();
                    let _mode_index = next_arg().unwrap();
                    let _mode_worktree = next_arg().unwrap();
                    let _hash_head = next_arg().unwrap();
                    let _hash_index = next_arg().unwrap();
                    let path = cur;

                    match xy_status {
                        ".M" => ret.push(GitFileStatus::Modified(
                            SafeRelativePath::from_relative_path(path)?.into(),
                        )),
                        _ => todo!("Unknown status {xy_status}"),
                    }
                }
                "2" => todo!(),
                "u" => todo!(),
                "?" => {
                    ret.push(GitFileStatus::Untracked(
                        SafeRelativePath::from_relative_path(cur)?.into(),
                    ));
                }
                "!" => todo!(),
                tag => todo!("unknown tag {tag:?}"),
            }
        }
        Ok(ret)
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum GitCmd {
    Status {
        files: Vec<SafeRelativePathBuf>,
    },
    Diff {
        files: Vec<SafeRelativePathBuf>,
    },
    Add {
        files: Vec<SafeRelativePathBuf>,
    },
    Pull {
        #[arg(short, long, num_args = 0..=1, require_equals = true, default_missing_value = "", value_parser=["", "false", "true", "merges", "interactive"])]
        rebase: Option<String>,
    },
    Commit {
        #[clap(long, short)]
        all: bool,

        #[clap(long, short)]
        message: Option<String>,
    },
    Push {},
}

impl GitCmd {
    pub fn passthru_dispatch_in(
        &self,
        repo_dir: impl AsRef<Path>,
        sh: &Shell,
    ) -> Result<(), Error> {
        let _dir = sh.push_dir(repo_dir);

        match self {
            GitCmd::Status { files } => cmd!(sh, "git status").arg("--").args(files),
            GitCmd::Diff { files } => cmd!(sh, "git diff").arg("--").args(files),
            GitCmd::Add { files } => cmd!(sh, "git add").arg("--").args(files),
            GitCmd::Pull { rebase } => cmd!(sh, "git pull").args(rebase.as_ref().map(|v| {
                if v.is_empty() {
                    "--rebase".to_string()
                } else {
                    format!("--rebase={v}")
                }
            })),
            GitCmd::Commit { all, message } => cmd!(sh, "git commit")
                .args(all.then_some("-a"))
                .args(message.as_ref().into_iter().flat_map(|m| ["-m", m])),
            GitCmd::Push {} => cmd!(sh, "git push"),
        }
        .run()?;

        Ok(())
    }
}
