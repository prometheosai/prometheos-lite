use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "prometheos",
    version,
    about = "Local-first multi-agent coding CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run PrometheOS on a task prompt.
    Run {
        /// Task prompt to execute.
        task: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { task } => {
            println!("Running task: {task}");
        }
    }
}
