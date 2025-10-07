use clap::Parser;
use zenops::config_files::ConfigFileDirs;

#[derive(Parser, Debug)]
#[command(name = "zenops", about = "ZenOps: your system’s calm overseer")]
pub struct Cli {
    #[clap(flatten)]
    args: zenops::Args,

    #[clap(long, short, default_value = "log")]
    output: OutputMode,

    #[command(subcommand)]
    command: zenops::Cmd,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum OutputMode {
    Log,
}

fn main() {
    let Cli {
        args,
        output,
        command,
    } = Cli::parse();

    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format_timestamp(None)
        .format_target(false)
        .init();

    let mut output: Box<dyn zenops::output::Output> = match output {
        OutputMode::Log => Box::new(zenops::output::Log),
    };

    let dirs = ConfigFileDirs::load(home::home_dir().unwrap());

    if let Err(e) = zenops::real_main(&args, &command, &dirs, output.as_mut()) {
        log::error!("{e}");
        std::process::exit(1);
    }
}
