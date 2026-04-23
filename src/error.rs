use std::path::PathBuf;

use smol_str::SmolStr;

use crate::output::{OutputError, ResolvedConfigFilePath};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to open the database file {0:?}: {1}")]
    OpenDb(PathBuf, #[source] std::io::Error),
    #[error("Failed to parse the database from file {0:?}: {1}")]
    ParseDb(PathBuf, #[source] toml::de::Error),
    #[error("Failed to execute command")]
    Shell(xshell::Error),
    #[error("Failed to write config file {0}: {1}")]
    FailedToWriteConfig(ResolvedConfigFilePath, std::io::Error),
    #[error(transparent)]
    SafeRelativePath(zenops_safe_relative_path::error::Error),
    #[error("Not creating symlink {symlink} -> {real}: a file already exists")]
    RefusingToOverwriteFileWithSymlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
    },
    #[error("Not creating symlink {symlink} -> {real}: a directory already exists")]
    RefusingToOverwriteDirectoryWithSymlink {
        real: ResolvedConfigFilePath,
        symlink: ResolvedConfigFilePath,
    },
    #[error("Failed to create directory {0:?}: {1}")]
    CreateDirectoryError(ResolvedConfigFilePath, std::io::Error),
    #[error(
        "Package {pkg} references undefined input {input}; mark the action optional or set [pkg.{pkg}.inputs].{input}"
    )]
    UnresolvedInput { pkg: SmolStr, input: SmolStr },
    #[error("Package {pkg} has an unterminated `${{` in a template")]
    TemplateUnterminated { pkg: SmolStr },
    #[error(
        "apply requires a terminal for prompts; pass --yes to apply all changes non-interactively, or --dry-run to preview"
    )]
    ApplyNeedsYesOrTty,
    #[error("Failed to read confirmation from stdin: {0}")]
    PromptRead(#[source] std::io::Error),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error(
        "Cannot init: {0:?} already exists and is not empty. Remove it first, or use `zenops repo pull` if it's already a zenops repo."
    )]
    InitDirNotEmpty(PathBuf),
    #[error(
        "Cloned repo at {0:?} has no config.toml at its root. Is this a zenops config repo? The clone was left in place so you can inspect it."
    )]
    InitNoConfigToml(PathBuf),
    #[error("Failed to clone {url}: {source}")]
    InitCloneFailed {
        url: String,
        #[source]
        source: xshell::Error,
    },
    #[error("Init I/O error at {0:?}: {1}")]
    InitIo(PathBuf, #[source] std::io::Error),
    #[error(
        "curl is required to fetch GitHub SSH keys; install curl or switch the entry to type = \"manual\""
    )]
    CurlNotFound,
    #[error("Failed to fetch SSH keys for GitHub user {username}: {source}")]
    GithubKeyFetchFailed {
        username: SmolStr,
        #[source]
        source: xshell::Error,
    },
    #[error("Failed to parse SSH signing keys response for GitHub user {username}: {source}")]
    GithubKeyParseFailed {
        username: SmolStr,
        #[source]
        source: serde_json::Error,
    },
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::OpenDb(l0, l1), Self::OpenDb(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            (Self::ParseDb(l0, l1), Self::ParseDb(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::Shell(l0), Self::Shell(r0)) => l0.to_string() == r0.to_string(),
            (Self::FailedToWriteConfig(l0, l1), Self::FailedToWriteConfig(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (Self::SafeRelativePath(l0), Self::SafeRelativePath(r0)) => l0 == r0,
            (
                Self::RefusingToOverwriteFileWithSymlink {
                    real: l_real,
                    symlink: l_symlink,
                },
                Self::RefusingToOverwriteFileWithSymlink {
                    real: r_real,
                    symlink: r_symlink,
                },
            ) => l_real == r_real && l_symlink == r_symlink,
            (
                Self::RefusingToOverwriteDirectoryWithSymlink {
                    real: l_real,
                    symlink: l_symlink,
                },
                Self::RefusingToOverwriteDirectoryWithSymlink {
                    real: r_real,
                    symlink: r_symlink,
                },
            ) => l_real == r_real && l_symlink == r_symlink,
            (Self::CreateDirectoryError(l0, l1), Self::CreateDirectoryError(r0, r1)) => {
                l0 == r0 && l1.kind() == r1.kind()
            }
            (
                Self::UnresolvedInput {
                    pkg: l_pkg,
                    input: l_input,
                },
                Self::UnresolvedInput {
                    pkg: r_pkg,
                    input: r_input,
                },
            ) => l_pkg == r_pkg && l_input == r_input,
            (Self::TemplateUnterminated { pkg: l }, Self::TemplateUnterminated { pkg: r }) => {
                l == r
            }
            (Self::ApplyNeedsYesOrTty, Self::ApplyNeedsYesOrTty) => true,
            (Self::PromptRead(l0), Self::PromptRead(r0)) => l0.kind() == r0.kind(),
            (Self::Output(l0), Self::Output(r0)) => l0.to_string() == r0.to_string(),
            (Self::InitDirNotEmpty(l0), Self::InitDirNotEmpty(r0)) => l0 == r0,
            (Self::InitNoConfigToml(l0), Self::InitNoConfigToml(r0)) => l0 == r0,
            (
                Self::InitCloneFailed {
                    url: l_url,
                    source: l_src,
                },
                Self::InitCloneFailed {
                    url: r_url,
                    source: r_src,
                },
            ) => l_url == r_url && l_src.to_string() == r_src.to_string(),
            (Self::InitIo(l0, l1), Self::InitIo(r0, r1)) => l0 == r0 && l1.kind() == r1.kind(),
            (Self::CurlNotFound, Self::CurlNotFound) => true,
            (
                Self::GithubKeyFetchFailed {
                    username: l_user,
                    source: l_src,
                },
                Self::GithubKeyFetchFailed {
                    username: r_user,
                    source: r_src,
                },
            ) => l_user == r_user && l_src.to_string() == r_src.to_string(),
            (
                Self::GithubKeyParseFailed {
                    username: l_user,
                    source: l_src,
                },
                Self::GithubKeyParseFailed {
                    username: r_user,
                    source: r_src,
                },
            ) => l_user == r_user && l_src.to_string() == r_src.to_string(),
            _ => false,
        }
    }
}

impl From<xshell::Error> for Error {
    fn from(e: xshell::Error) -> Self {
        Self::Shell(e)
    }
}

impl From<zenops_safe_relative_path::error::Error> for Error {
    fn from(e: zenops_safe_relative_path::error::Error) -> Self {
        Self::SafeRelativePath(e)
    }
}
