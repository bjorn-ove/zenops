//! Thin git wrapper used for the zenops config repo.
//!
//! [`Git`] is bound to a working-tree path and a shared [`xshell::Shell`].
//! It covers the small surface zenops actually needs: `is_git_repo`,
//! `has_uncommitted_changes`, parsing `git status --porcelain=v2` output
//! into [`GitFileStatus`], commit-and-push, and a one-shot
//! [`Git::clone_to`] used by `init` before any `Git` instance exists.
//!
//! `git`'s own stdio is streamed through so SSH-passphrase and
//! HTTPS-credential-helper prompts still reach the user's TTY.

use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;
use smol_str::SmolStr;
use xshell::{Shell, cmd};
use zenops_safe_relative_path::{SafeRelativePath, SafeRelativePathBuf};

use crate::error::Error;

/// Summary of a single file's state in `git status --porcelain=v2`. The
/// effective state is reduced from the worktree side (Y) of the XY pair,
/// falling back to the index side (X) when Y is `.`. Anything we don't
/// recognise — type-change, copy, unmerged, etc. — surfaces as
/// [`Self::Other`] with the raw code so the user still sees something.
#[derive(Debug, PartialEq, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum GitFileStatus {
    /// Tracked file modified (also covers type-change, rename, copy).
    Modified(SafeRelativePathBuf),
    /// New file staged for commit.
    Added(SafeRelativePathBuf),
    /// Tracked file deleted.
    Deleted(SafeRelativePathBuf),
    /// Untracked file present in the worktree.
    Untracked(SafeRelativePathBuf),
    /// Anything else; raw porcelain code retained for diagnostics.
    Other {
        /// Raw XY status code from `git status --porcelain=v2`.
        code: SmolStr,
        /// File path as reported by git.
        path: SafeRelativePathBuf,
    },
}

/// Reduce a porcelain=v2 `XY` pair to the kind we care about. We prefer the
/// worktree side (Y); if it's `.` (unchanged) we fall back to the index side
/// (X) so `M.` / `A.` / `D.` (staged-only) still surface.
fn status_from_xy(xy: &str, path: SafeRelativePathBuf) -> GitFileStatus {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    let effective = if y != '.' { y } else { x };
    match effective {
        'M' | 'T' | 'R' | 'C' => GitFileStatus::Modified(path),
        'A' => GitFileStatus::Added(path),
        'D' => GitFileStatus::Deleted(path),
        _ => GitFileStatus::Other {
            code: SmolStr::new(xy),
            path,
        },
    }
}

/// Handle to a git working tree. Borrows the path and the shared
/// [`xshell::Shell`] so callers can construct one cheaply per use without
/// duplicating environment setup.
pub struct Git<'path, 'shell> {
    path: &'path Path,
    sh: &'shell Shell,
}

impl<'path, 'shell> Git<'path, 'shell> {
    /// Bind to an existing working tree. The path doesn't have to be a
    /// repo — use [`Self::is_git_repo`] to check.
    pub fn new(path: &'path Path, sh: &'shell Shell) -> Self {
        Self { path, sh }
    }

    /// Clone `url` into `dest`, streaming git's own stdio so SSH
    /// passphrase and HTTPS credential-helper prompts still reach the TTY.
    /// `dest` must not exist (git refuses to clone into a populated
    /// directory) — the init flow handles that pre-flight.
    pub fn clone_to(url: &str, dest: &Path, branch: Option<&str>, sh: &Shell) -> Result<(), Error> {
        let mut c = cmd!(sh, "git clone");
        if let Some(b) = branch {
            c = c.arg("--branch").arg(b);
        }
        c = c.arg(url).arg(dest);
        c.run().map_err(|e| Error::InitCloneFailed {
            url: url.to_string(),
            source: e,
        })?;
        Ok(())
    }

    /// `true` if the bound path is inside a git work tree. Returns `false`
    /// — not an error — for non-repo paths so callers can branch cleanly.
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

    /// Per-file status from `git status --porcelain=v2`. Renames surface
    /// at the new path; ignored files (`!`) are skipped; unmerged entries
    /// fold into [`GitFileStatus::Other`] without distinguishing the
    /// conflict state.
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
                    let path = SafeRelativePath::from_relative_path(cur)?.into();
                    ret.push(status_from_xy(xy_status, path));
                }
                "2" => {
                    // Rename/copy: `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI>
                    // <X><score> <path>\t<origPath>`. We surface the new path.
                    let xy_status = next_arg().unwrap();
                    let _submodule_state = next_arg().unwrap();
                    let _mode_head = next_arg().unwrap();
                    let _mode_index = next_arg().unwrap();
                    let _mode_worktree = next_arg().unwrap();
                    let _hash_head = next_arg().unwrap();
                    let _hash_index = next_arg().unwrap();
                    let _rename_score = next_arg().unwrap();
                    let new_path = cur.split_once('\t').map(|(p, _)| p).unwrap_or(cur);
                    let path = SafeRelativePath::from_relative_path(new_path)?.into();
                    ret.push(status_from_xy(xy_status, path));
                }
                "u" => {
                    // Unmerged: `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2>
                    // <h3> <path>`. We don't distinguish unmerged states; just
                    // surface it as Other so the user sees it.
                    let xy_status = next_arg().unwrap();
                    for _ in 0..9 {
                        let _ = next_arg().unwrap();
                    }
                    let path = SafeRelativePath::from_relative_path(cur)?.into();
                    ret.push(GitFileStatus::Other {
                        code: SmolStr::new(xy_status),
                        path,
                    });
                }
                "?" => {
                    ret.push(GitFileStatus::Untracked(
                        SafeRelativePath::from_relative_path(cur)?.into(),
                    ));
                }
                "!" => {
                    // Ignored file; should only appear with --ignored, but
                    // skip defensively rather than panic.
                }
                tag => {
                    // Unknown line format — skip rather than abort; the user
                    // still sees the git diff we print alongside this list.
                    log::debug!("git status --porcelain=v2: unknown line tag {tag:?}");
                }
            }
        }
        Ok(ret)
    }

    /// Fast check: does the working tree have any uncommitted changes
    /// (modified, staged, deleted, or untracked)? Avoids parsing individual
    /// status codes so it's robust against exotic states.
    pub fn has_uncommitted_changes(&self) -> Result<bool, Error> {
        let Self { path, sh } = self;
        let out = cmd!(sh, "git -C {path} status --porcelain")
            .quiet()
            .read()?;
        Ok(!out.is_empty())
    }

    /// Stage everything (including untracked and deletions), commit with the
    /// given message, then push. Stops at the first failing step.
    pub fn commit_all_and_push(&self, message: &str) -> Result<(), Error> {
        let Self { path, sh } = self;
        cmd!(sh, "git -C {path} add -A").run()?;
        cmd!(sh, "git -C {path} commit -m {message}").run()?;
        cmd!(sh, "git -C {path} push").run()?;
        Ok(())
    }

    /// Render `git status -s` + `git diff HEAD` to the inherited stdout so
    /// the user can review what they're about to (optionally) commit.
    /// Untracked files appear in the status summary but their contents are
    /// not shown.
    pub fn print_pre_apply_summary(&self, color: bool) -> Result<(), Error> {
        let Self { path, sh } = self;
        // `git status` ignores `--color`; drive it via `-c color.status=…`.
        let color_setting = if color { "always" } else { "never" };
        let status_color = format!("color.status={color_setting}");
        cmd!(sh, "git -C {path} -c {status_color} status -s").run()?;
        let diff_color = if color {
            "--color=always"
        } else {
            "--color=never"
        };
        cmd!(sh, "git -C {path} diff HEAD {diff_color}").run()?;
        Ok(())
    }
}

/// Subset of git operations exposed via `zenops repo <cmd>`. Each variant
/// runs the corresponding `git` invocation inside the zenops config repo
/// and inherits stdio so output matches what the user would see from a
/// regular git command.
#[derive(clap::Subcommand, Debug)]
pub enum GitCmd {
    /// `git status [-- <files>...]` in the zenops repo.
    Status {
        /// Optional path filter passed after `--`.
        files: Vec<SafeRelativePathBuf>,
    },
    /// `git diff [-- <files>...]` in the zenops repo.
    Diff {
        /// Optional path filter passed after `--`.
        files: Vec<SafeRelativePathBuf>,
    },
    /// `git add [-- <files>...]` in the zenops repo.
    Add {
        /// Files to stage.
        files: Vec<SafeRelativePathBuf>,
    },
    /// `git pull` in the zenops repo. `--rebase` accepts the same values
    /// as upstream git (`true`, `false`, `merges`, `interactive`); a bare
    /// `--rebase` (no value) maps to `--rebase` with no argument.
    Pull {
        /// Optional `--rebase[=value]`. `None` runs a plain pull.
        #[arg(short, long, num_args = 0..=1, require_equals = true, default_missing_value = "", value_parser=["", "false", "true", "merges", "interactive"])]
        rebase: Option<String>,
    },
    /// `git commit` in the zenops repo.
    Commit {
        /// Pass `-a` to stage all tracked, modified files first.
        #[clap(long, short)]
        all: bool,

        /// Commit message; if omitted git opens the editor as usual.
        #[clap(long, short)]
        message: Option<String>,
    },
    /// `git push` in the zenops repo.
    Push {},
}

impl GitCmd {
    /// Run the chosen git operation against `repo_dir`, inheriting stdio
    /// so output and prompts (e.g. credential helper) reach the user
    /// directly.
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
