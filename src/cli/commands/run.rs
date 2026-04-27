//! Run command handler (deprecated)

use anyhow::Context;
use clap::Parser;

use prometheos_lite::{
    logger::Logger,
    llm::LlmClient,
    config::AppConfig,
};

#[derive(Debug, Parser)]
#[deprecated(since = "0.2.0", note = "Use 'flow' command instead")]
pub struct RunCommand {
    /// Task prompt to execute.
    pub task: String,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

impl RunCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        eprintln!("WARNING: The 'run' command is deprecated. Use 'flow' command instead.");
        eprintln!(
            "See migration guide: https://github.com/prometheos-ai/prometheos-lite/blob/main/docs/migration.md"
        );

        let logger = Logger::new(self.verbose);
        logger.info(&format!("Starting PrometheOS Lite (DEPRECATED)"));
        logger.info(&format!("Task: {}", self.task));

        let config = AppConfig::load()?;
        logger.info(&format!("Loaded config for provider: {}", config.provider));

        let llm = LlmClient::from_config(&config)?;

        // Use the deprecated orchestrator
        #[cfg(feature = "legacy")]
        {
            #[allow(deprecated)]
            let orchestrator =
                prometheos_lite::legacy::core::SequentialOrchestrator::with_logger(llm, logger.clone());

            logger.info("Initializing orchestrator...");
            let result = orchestrator.run(self.task.clone()).await?;

            logger.success("Task completed successfully");
            Ok(())
        }

        #[cfg(not(feature = "legacy"))]
        Err(anyhow::anyhow!("Legacy 'run' command requires the 'legacy' feature flag. Use 'flow' command instead."))
    }
}
