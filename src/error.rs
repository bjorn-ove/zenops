use std::path::PathBuf;

use smol_str::SmolStr;

use crate::output::ResolvedConfigFilePath;

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
