use clap::{Parser, Subcommand};

use prometheos_lite::{
    config::AppConfig,
    core::SequentialOrchestrator,
    fs::{FileParser, FileWriter},
    logger::Logger,
    llm::LlmClient,
};

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
    Run {
        /// Task prompt to execute.
        task: String,
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { task, verbose } => {
            let logger = Logger::new(verbose);
            logger.info(&format!("Starting PrometheOS Lite"));
            logger.info(&format!("Task: {}", task));

            let config = AppConfig::load()?;
            logger.info(&format!("Loaded config for provider: {}", config.provider));

            let llm = LlmClient::from_config(&config)?;
            let orchestrator = SequentialOrchestrator::with_logger(llm, logger.clone());
            
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
                    logger.success(&format!("Written {} file(s) to {}", written.len(), writer.output_dir().display()));
                }
            }
            if let Some(review) = result.review() {
                println!("\n[reviewer]\n{review}");
            }

            logger.success("Task completed successfully");
        }
    }

    Ok(())
}
