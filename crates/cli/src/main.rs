//! Local analysis, simulation, ingest and evaluation CLI.
//!
//! FR-OUT-04. Commands parse their arguments and report the requested work;
//! each command is wired to the pipeline or simulator as that stage lands.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "doppler",
    bin_name = "doppler",
    version,
    about = "Doppler locatability audit tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze one recording.
    Analyze(AnalyzeArgs),
    /// Run a simulator scenario.
    Sim(SimArgs),
    /// Ingest ADS-B archives into the local cache.
    Ingest(IngestArgs),
    /// Evaluate a completed run against ground truth.
    Eval(EvalArgs),
}

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// Input audio or video file.
    path: PathBuf,
    /// Optional known recording start in UTC, ISO-8601.
    #[arg(long)]
    start: Option<String>,
}

#[derive(Debug, Args)]
struct SimArgs {
    /// Scenario JSON file.
    scenario: PathBuf,
    /// Output directory for audio and ground truth.
    #[arg(long, default_value = "data/runs")]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct IngestArgs {
    /// Archive source name.
    #[arg(long, default_value = "opensky")]
    source: String,
    /// Track cache directory.
    #[arg(long, default_value = "data/tracks")]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct EvalArgs {
    /// Run manifest JSON to evaluate.
    manifest: PathBuf,
}

fn main() {
    init_tracing();
    let cli = Cli::parse();
    let budget_s = pipeline::RunMode::Research.wall_time_limit_s();

    match cli.command {
        Command::Analyze(args) => {
            tracing::info!(
                path = %args.path.display(),
                start = ?args.start,
                budget_s,
                "analyze requested; estimator is not implemented yet"
            );
        }
        Command::Sim(args) => {
            tracing::info!(
                scenario = %args.scenario.display(),
                out = %args.out.display(),
                "simulation requested; simulator is not implemented yet"
            );
        }
        Command::Ingest(args) => {
            tracing::info!(
                source = %args.source,
                out = %args.out.display(),
                "ingest requested; archive client is not implemented yet"
            );
        }
        Command::Eval(args) => {
            tracing::info!(
                manifest = %args.manifest.display(),
                "evaluation requested; evaluator is not implemented yet"
            );
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(error) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("failed to install tracing subscriber: {error}");
    }
}
