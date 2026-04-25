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
        /// Enable debug mode with step-by-step execution
        #[arg(short, long)]
        debug: bool,
        /// Export timeline to file
        #[arg(long)]
        export_timeline: Option<std::path::PathBuf>,
    },
    /// Start the API server for the local chat interface
    Serve {
        /// Host address to bind to (default: 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on (default: 3000)
        #[arg(short, long, default_value = "3000")]
        port: u16,
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
            #[cfg(feature = "legacy")]
            {
                #[allow(deprecated)]
                let orchestrator =
                    prometheos_lite::legacy::core::SequentialOrchestrator::with_logger(llm, logger.clone());

                logger.info("Initializing orchestrator...");
                let result = orchestrator.run(task).await?;

                logger.success("Task completed successfully");
                Ok(())
            }

            #[cfg(not(feature = "legacy"))]
            return Err(anyhow::anyhow!("Legacy 'run' command requires the 'legacy' feature flag. Use 'flow' command instead."));
        }
        Commands::Flow {
            path,
            input,
            verbose,
            debug,
            export_timeline,
        } => {
            let logger = Logger::new(verbose);
            logger.info(&format!("Loading flow from: {}", path.display()));

            // Load config and create RuntimeContext
            let config = AppConfig::load()?;
            logger.info(&format!("Loaded config for provider: {}", config.provider));

            // Create RuntimeContext with services
            // Create LlmClient and wrap in OpenAiProvider for ModelRouter
            let llm_client = LlmClient::from_config(&config)?;
            let openai_provider = prometheos_lite::flow::OpenAiProvider::new(llm_client)
                .with_name(config.provider.clone());
            let model_router = std::sync::Arc::new(prometheos_lite::flow::ModelRouter::new(vec![
                Box::new(openai_provider)
            ]));
            
            let tool_runtime = std::sync::Arc::new(prometheos_lite::flow::ToolRuntime::new(
                prometheos_lite::flow::ToolSandboxProfile::new()
            ));
            
            // Create in-memory memory service with local embedding provider from config
            let embedding: Box<dyn prometheos_lite::flow::EmbeddingProvider> = Box::new(
                prometheos_lite::flow::LocalEmbeddingProvider::new(
                    config.embedding_url.clone(),
                    config.embedding_dimension,
                )
            );
            let in_memory_db = prometheos_lite::flow::MemoryDb::in_memory().unwrap();
            let memory_service = std::sync::Arc::new(prometheos_lite::flow::MemoryService::new(
                in_memory_db,
                embedding,
            ));

            let runtime = prometheos_lite::flow::RuntimeContext::full(
                model_router,
                tool_runtime,
                memory_service,
            );

            let mut flow_runner = runner::FlowRunner::from_json_file_with_runtime(path, Some(runtime))?;
            logger.info("Flow loaded successfully");

            let input_value = if let Some(input_str) = input {
                serde_json::from_str(&input_str).context("Failed to parse input JSON")?
            } else {
                serde_json::json!({})
            };

            // Enable debug mode if requested
            if debug {
                logger.info("Debug mode enabled");
                flow_runner.enable_debug_mode();
            }

            logger.info("Executing flow...");
            let state = flow_runner.run_with_input(input_value).await?;

            logger.success("Flow execution completed");

            // Render output
            println!(
                "\n[output]\n{}",
                serde_json::to_string_pretty(&state.get_all_outputs())?
            );

            // Export timeline if requested
            if let Some(timeline_path) = export_timeline {
                if let Some(tracer) = flow_runner.tracer() {
                    if let Ok(t) = tracer.lock() {
                        let timeline_json = t.export_timeline().context("Failed to export timeline")?;
                        std::fs::write(&timeline_path, timeline_json)
                            .context("Failed to write timeline file")?;
                        logger.info(&format!("Timeline exported to: {}", timeline_path.display()));
                    }
                } else {
                    eprintln!("Warning: No tracer available for timeline export");
                }
            }
        }
        Commands::Serve { host, port } => {
            let logger = Logger::new(true);
            logger.info(&format!("Starting API server on {}:{}", host, port));

            // Load config and create RuntimeContext
            let config = AppConfig::load()?;
            logger.info(&format!("Loaded config for provider: {}", config.provider));

            // Create RuntimeContext with services
            let llm_client = LlmClient::from_config(&config)?;
            let openai_provider = prometheos_lite::flow::OpenAiProvider::new(llm_client)
                .with_name(config.provider.clone());
            let model_router = std::sync::Arc::new(prometheos_lite::flow::ModelRouter::new(vec![
                Box::new(openai_provider)
            ]));

            let tool_runtime = std::sync::Arc::new(prometheos_lite::flow::ToolRuntime::new(
                prometheos_lite::flow::ToolSandboxProfile::new()
            ));

            // Create in-memory memory service with local embedding provider from config
            let embedding: Box<dyn prometheos_lite::flow::EmbeddingProvider> = Box::new(
                prometheos_lite::flow::LocalEmbeddingProvider::new(
                    config.embedding_url.clone(),
                    config.embedding_dimension,
                )
            );
            let in_memory_db = prometheos_lite::flow::MemoryDb::in_memory().unwrap();
            let memory_service = std::sync::Arc::new(prometheos_lite::flow::MemoryService::new(
                in_memory_db,
                embedding,
            ));

            let runtime = prometheos_lite::flow::RuntimeContext::full(
                model_router,
                tool_runtime,
                memory_service,
            );

            // Create database
            let db_path = "prometheos.db".to_string();
            let _db = prometheos_lite::db::Db::new(&db_path)?;
            logger.info("Database initialized: prometheos.db");

            // Create AppState
            let app_state = std::sync::Arc::new(prometheos_lite::api::AppState::new(
                db_path,
                std::sync::Arc::new(runtime)
            ));

            // Parse address
            let addr: std::net::SocketAddr = format!("{}:{}", host, port).parse()
                .context("Invalid host or port")?;

            // Start server
            prometheos_lite::api::run_server(addr, app_state).await?;
        }
    }

    Ok(())
}
