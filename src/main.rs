use std::io::{self, IsTerminal};

use clap::{CommandFactory, Parser};
use zenops::config_files::ConfigFileDirs;

#[derive(Parser, Debug)]
#[command(name = "zenops", about = "ZenOps: your system’s calm overseer")]
pub struct Cli {
    #[clap(flatten)]
    args: zenops::Args,

    /// How to render status and apply events.
    #[clap(long, short, global = true, value_enum, default_value_t = OutputMode::Human)]
    output: OutputMode,

    #[command(subcommand)]
    command: zenops::Cmd,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum OutputMode {
    /// Human-readable text to stderr, with optional ANSI color.
    Human,
    /// Newline-delimited JSON to stderr, one event per line.
    Json,
}

fn main() {
    let Cli {
        args,
        output,
        command,
    } = Cli::parse();

    if let zenops::Cmd::Completions { shell } = &command {
        let mut cmd = Cli::command();
        clap_complete::generate(*shell, &mut cmd, "zenops", &mut std::io::stdout());
        return;
    }

    // Kept so power users can set `RUST_LOG=debug` and see the remaining
    // diagnostic breadcrumbs (currently the unknown git-status-tag line
    // in `git.rs`). Zero-config beyond that.
    env_logger::init();

    let stderr = io::stderr();
    let stderr_is_terminal = stderr.is_terminal();
    let color = args.color.enabled(stderr_is_terminal);
    let show_diffs = matches!(command, zenops::Cmd::Status { diff: true });

    let mut lock = stderr.lock();
    let mut renderer: Box<dyn zenops::output::Output> = match output {
        OutputMode::Human => Box::new(zenops::output::TerminalRenderer::new(
            &mut lock, color, show_diffs,
        )),
        OutputMode::Json => Box::new(zenops::output::JsonOutput::new(&mut lock)),
    };

    let dirs = ConfigFileDirs::load(home::home_dir().unwrap());

    if let Err(e) = zenops::real_main(&args, &command, &dirs, renderer.as_mut()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
