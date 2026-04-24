use std::fs;

use xshell::{Shell, cmd};

use crate::{
    Args, Cmd,
    config::Config,
    config_files::ConfigFileDirs,
    error::Error,
    git::Git,
    output::{InitSummary, Output},
    real_main,
};

pub fn run(
    url: &str,
    branch: Option<&str>,
    apply: bool,
    yes: bool,
    dirs: &ConfigFileDirs,
    args: &Args,
    output: &mut dyn Output,
) -> Result<(), Error> {
    preflight(dirs)?;

    let sh = Shell::new().unwrap();
    Git::clone_to(url, dirs.zenops(), branch, &sh)?;

    let config = Config::load(dirs, &sh, false).map_err(|e| match e {
        Error::OpenDb(_, io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
            Error::InitNoConfigToml(dirs.zenops().to_path_buf())
        }
        other => other,
    })?;

    if apply {
        // Hand off to apply: the apply event stream is the contract here,
        // so suppress the init summary (it would just be noise before a
        // structured apply log).
        let apply_cmd = Cmd::Apply {
            pull_config: false,
            yes,
            dry_run: false,
            allow_dirty: false,
        };
        return real_main(args, &apply_cmd, dirs, output);
    }

    emit_summary(dirs, &sh, &config, output)
}

fn preflight(dirs: &ConfigFileDirs) -> Result<(), Error> {
    let zenops_dir = dirs.zenops();
    match fs::read_dir(zenops_dir) {
        Ok(mut iter) => {
            if iter.next().is_some() {
                return Err(Error::InitDirNotEmpty(zenops_dir.to_path_buf()));
            }
            fs::remove_dir(zenops_dir).map_err(|e| Error::InitIo(zenops_dir.to_path_buf(), e))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = zenops_dir.parent() {
                fs::create_dir_all(parent).map_err(|e| Error::InitIo(parent.to_path_buf(), e))?;
            }
        }
        Err(e) => return Err(Error::InitIo(zenops_dir.to_path_buf(), e)),
    }
    Ok(())
}

fn emit_summary(
    dirs: &ConfigFileDirs,
    sh: &Shell,
    config: &Config<'_>,
    output: &mut dyn Output,
) -> Result<(), Error> {
    let dest = dirs.zenops();
    let remote = cmd!(sh, "git -C {dest} remote get-url origin")
        .quiet()
        .ignore_stderr()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let shell = config.shell().map(|s| match s {
        crate::config::pkg::Shell::Bash => "bash".to_string(),
        crate::config::pkg::Shell::Zsh => "zsh".to_string(),
    });

    output.push_init_summary(InitSummary {
        clone_path: dest.to_path_buf(),
        remote,
        shell,
        pkg_count: config.pkgs().len(),
    })?;
    Ok(())
}
