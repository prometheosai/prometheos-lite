//! Serve command handler

use anyhow::Context;
use clap::Parser;

use prometheos_lite::logger::Logger;

use super::super::runtime_builder::RuntimeBuilder;

#[derive(Debug, Parser)]
pub struct ServeCommand {
    /// Host address to bind to (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Port to listen on (default: 3000)
    #[arg(short, long, default_value = "3000")]
    pub port: u16,
}

impl ServeCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(true);
        logger.info(&format!(
            "Starting API server on {}:{}",
            self.host, self.port
        ));

        // Build runtime
        let runtime_builder = RuntimeBuilder::from_config()?;
        let runtime = runtime_builder.build_full()?;
        logger.info(&format!(
            "Loaded config for provider: {}",
            runtime_builder.config().provider
        ));

        // Create database
        let db_path = "prometheos.db".to_string();
        let _db = prometheos_lite::db::Db::new(&db_path)?;
        logger.info("Database initialized: prometheos.db");

        // Create embedding provider for API server
        let api_embedding = runtime_builder.build_embedding_provider();
        let memory_service = runtime_builder.build_memory_service()?;

        // Create AppState
        let app_state = std::sync::Arc::new(prometheos_lite::api::AppState::new(
            db_path,
            std::sync::Arc::new(runtime),
            api_embedding,
            memory_service,
        ).map_err(|e| anyhow::anyhow!("Failed to create AppState: {}", e))?);

        // Parse address
        let addr: std::net::SocketAddr = format!("{}:{}", self.host, self.port)
            .parse()
            .context("Invalid host or port")?;

        // Start server
        prometheos_lite::api::run_server(addr, app_state).await?;

        Ok(())
    }
}
