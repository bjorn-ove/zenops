use std::path::Path;

use xshell::{Shell, cmd};

use crate::error::Error;

pub enum GitFileStatus {
    Modified(String),
}

pub struct Git<'path, 'shell> {
    path: &'path Path,
    sh: &'shell Shell,
}

impl<'path, 'shell> Git<'path, 'shell> {
    pub fn new(path: &'path Path, sh: &'shell Shell) -> Self {
        Self { path, sh }
    }

    pub fn status(&self) -> Result<Vec<GitFileStatus>, Error> {
        let Self { path, sh } = self;
        let mut ret = Vec::new();
        for line in cmd!(sh, "git -C {path} status --porcelain=v2")
            .quiet()
            .read()?
            .lines()
        {
            let mut it = line.split(' ');
            match it.next().unwrap() {
                "1" => {
                    let xy_status = it.next().unwrap();
                    let _submodule_state = it.next().unwrap();
                    let _mode_head = it.next().unwrap();
                    let _mode_index = it.next().unwrap();
                    let _mode_worktree = it.next().unwrap();
                    let _hash_head = it.next().unwrap();
                    let _hash_index = it.next().unwrap();
                    let path = it.next().unwrap();
                    assert_eq!(it.next(), None);

                    match xy_status {
                        ".M" => ret.push(GitFileStatus::Modified(path.to_string())),
                        _ => todo!("Unknown status {xy_status}"),
                    }
                }
                "2" => todo!(),
                "u" => todo!(),
                "?" => todo!(),
                "!" => todo!(),
                tag => todo!("unknown tag {tag:?}"),
            }
        }
        Ok(ret)
    }
}
