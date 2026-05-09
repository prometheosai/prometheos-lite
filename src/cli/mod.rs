use clap::{Parser, Subcommand};

pub mod commands;
pub mod runner;
pub mod runtime_builder;

#[derive(Debug, Parser)]
#[command(
    name = "prometheos",
    version,
    about = "Local-first multi-agent coding CLI"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run PrometheOS on a task prompt (deprecated)
    #[deprecated(since = "0.2.0", note = "Use 'flow' command instead")]
    Run(commands::run::RunCommand),
    /// Run a flow from a JSON or YAML file
    Flow(commands::flow::FlowCommand),
    /// P2-014: Harness V1.6 commands for autonomous/assisted coding
    ///
    /// Run harness on a task, inspect results, dry-run, apply, or rollback.
    /// This is the primary interface for the V1.6 coding harness.
    Harness(commands::harness::HarnessCommand),
    /// Start the API server for the local chat interface
    Serve(commands::serve::ServeCommand),
    /// Run benchmark tasks
    Bench(commands::bench::BenchCommand),
    /// Manage WorkContexts
    Work(commands::work::WorkCommand),
    /// Manage domain templates
    Templates(commands::templates::TemplatesCommand),
    /// Run provider/system/validation diagnostics
    Diagnostics(commands::diagnostics::DiagnosticsArgs),
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(cmd) => cmd.execute().await,
        Commands::Flow(cmd) => cmd.execute().await,
        Commands::Harness(cmd) => cmd.execute().await,
        Commands::Serve(cmd) => cmd.execute().await,
        Commands::Bench(cmd) => cmd.execute().await,
        Commands::Work(cmd) => cmd.execute().await,
        Commands::Templates(cmd) => cmd.execute().await,
        Commands::Diagnostics(args) => commands::diagnostics::handle_diagnostics_command(args).await,
    }
}
