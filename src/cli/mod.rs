use anyhow::Context;
use clap::{Parser, Subcommand};

use prometheos_lite::{
    config::AppConfig,
    fs::{FileParser, FileWriter},
    llm::LlmClient,
    logger::Logger,
};

mod runner;

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
    /// Run PrometheOS on a task prompt.
    #[deprecated(since = "0.2.0", note = "Use 'flow' command instead")]
    Run {
        /// Task prompt to execute.
        task: String,
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Run a flow from a JSON file
    Flow {
        /// Path to the flow file
        path: std::path::PathBuf,
        /// Input data for the flow (JSON string)
        #[arg(short, long)]
        input: Option<String>,
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { task, verbose } => {
            eprintln!("WARNING: The 'run' command is deprecated. Use 'flow' command instead.");
            eprintln!(
                "See migration guide: https://github.com/prometheos-ai/prometheos-lite/blob/main/docs/migration.md"
            );

            let logger = Logger::new(verbose);
            logger.info(&format!("Starting PrometheOS Lite (DEPRECATED)"));
            logger.info(&format!("Task: {}", task));

            let config = AppConfig::load()?;
            logger.info(&format!("Loaded config for provider: {}", config.provider));

            let llm = LlmClient::from_config(&config)?;

            // Use the deprecated orchestrator
            #[allow(deprecated)]
            let orchestrator =
                prometheos_lite::core::SequentialOrchestrator::with_logger(llm, logger.clone());

            logger.info("Initializing orchestrator...");
            let result = orchestrator.run(task).await?;

            if let Some(plan) = result.plan() {
                println!("\n[planner]\n{plan}");
            }
            if let Some(output) = result.generated_output() {
                println!("\n[builder]\n{output}");

                logger.info("Parsing generated files...");
                let files = FileParser::parse_files(output)?;
                logger.info(&format!("Found {} file(s) to write", files.len()));

                if !files.is_empty() {
                    logger.info("Writing files to disk...");
                    let writer = FileWriter::new()?;
                    let written = writer.write_files(&files)?;
                    logger.success(&format!(
                        "Written {} file(s) to {}",
                        written.len(),
                        writer.output_dir().display()
                    ));
                }
            }
            if let Some(review) = result.review() {
                println!("\n[reviewer]\n{review}");
            }

            logger.success("Task completed successfully");
        }
        Commands::Flow {
            path,
            input,
            verbose,
        } => {
            let logger = Logger::new(verbose);
            logger.info(&format!("Loading flow from: {}", path.display()));

            let mut flow_runner = runner::FlowRunner::from_json_file(path)?;
            logger.info("Flow loaded successfully");

            let input_value = if let Some(input_str) = input {
                serde_json::from_str(&input_str).context("Failed to parse input JSON")?
            } else {
                serde_json::json!({})
            };

            logger.info("Executing flow...");
            let state = flow_runner.run_with_input(input_value).await?;

            logger.success("Flow execution completed");
            println!(
                "\n[output]\n{}",
                serde_json::to_string_pretty(&state.get_all_outputs())?
            );
        }
    }

    Ok(())
}
