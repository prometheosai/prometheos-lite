//! Benchmark command handler

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Instant;

use prometheos_lite::{
    flow::testing::{FlowTestRunner, TestFixture},
    logger::Logger,
};

#[derive(Debug, Parser)]
pub struct BenchCommand {
    #[command(subcommand)]
    pub action: BenchAction,
}

#[derive(Debug, Subcommand)]
pub enum BenchAction {
    /// Run benchmark tasks
    Run(RunBenchCommand),
}

#[derive(Debug, Parser)]
pub struct RunBenchCommand {
    /// Benchmark task to run (direct-chat, planning, codegen, or all)
    #[arg(short, long, default_value = "all")]
    pub task: String,
    /// Number of iterations
    #[arg(short, long, default_value = "1")]
    pub iterations: u32,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

impl BenchCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        match &self.action {
            BenchAction::Run(cmd) => cmd.execute().await,
        }
    }
}

impl RunBenchCommand {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let logger = Logger::new(self.verbose);
        logger.info(&format!("Running benchmarks: {}", self.task));

        let benchmarks_dir = PathBuf::from("benchmarks");
        let mut results = Vec::new();

        let tasks = if self.task == "all" {
            vec!["direct-chat", "planning", "codegen"]
        } else {
            vec![self.task.as_str()]
        };

        for task in tasks {
            let flow_path = benchmarks_dir.join(format!("{}.flow.yaml", task));
            if !flow_path.exists() {
                logger.info(&format!("Benchmark not found: {}", task));
                continue;
            }

            logger.info(&format!("Running benchmark: {}", task));

            for i in 0..self.iterations {
                logger.info(&format!("  Iteration {}/{}", i + 1, self.iterations));

                let fixture = TestFixture::new(serde_json::json!({
                    "message": "test message"
                }));

                let test_runner = FlowTestRunner::new(flow_path.clone()).with_tracing();

                let start = Instant::now();
                let result = test_runner.run_test(&fixture).await;
                let duration_ms = start.elapsed().as_millis();

                let benchmark_result = match &result {
                    Ok(test_result) => BenchmarkResult {
                        task: task.to_string(),
                        iteration: i + 1,
                        success: true,
                        duration_ms,
                        error: None,
                        llm_calls: test_result.metrics.llm_calls,
                        tool_calls: test_result.metrics.tool_calls,
                        budget_exceeded: false, // TODO: Detect from test_result
                    },
                    Err(e) => BenchmarkResult {
                        task: task.to_string(),
                        iteration: i + 1,
                        success: false,
                        duration_ms,
                        error: Some(e.to_string()),
                        llm_calls: 0,
                        tool_calls: 0,
                        budget_exceeded: e.to_string().contains("budget"),
                    },
                };

                results.push(benchmark_result);

                if result.is_ok() {
                    logger.success(&format!(
                        "  Iteration {} completed in {}ms",
                        i + 1,
                        duration_ms
                    ));
                } else {
                    logger.error(&format!("  Iteration {} failed", i + 1));
                }
            }
        }

        // Generate report
        let report = self.generate_report(&results);
        println!("\n[benchmark_report]");
        println!("{}", serde_json::to_string_pretty(&report)?);

        logger.success("Benchmark run completed");
        Ok(())
    }

    fn generate_report(&self, results: &[BenchmarkResult]) -> BenchmarkReport {
        let mut task_stats: std::collections::HashMap<String, TaskStats> =
            std::collections::HashMap::new();

        for result in results {
            let stats = task_stats
                .entry(result.task.clone())
                .or_insert_with(TaskStats::new);
            stats.total_runs += 1;
            if result.success {
                stats.successful_runs += 1;
                stats.total_duration_ms += result.duration_ms;
                stats.total_llm_calls += result.llm_calls;
                stats.total_tool_calls += result.tool_calls;
            } else {
                stats.failed_runs += 1;
            }
            if result.budget_exceeded {
                stats.budget_exceeded_count += 1;
            }
        }

        let mut task_reports = Vec::new();
        for (task, stats) in task_stats {
            let success_rate = if stats.total_runs > 0 {
                stats.successful_runs as f64 / stats.total_runs as f64
            } else {
                0.0
            };

            let median_runtime_ms = if stats.successful_runs > 0 {
                stats.total_duration_ms / stats.successful_runs as u128
            } else {
                0
            };

            let llm_calls_per_run = if stats.successful_runs > 0 {
                stats.total_llm_calls / stats.successful_runs
            } else {
                0
            };

            let tool_calls_per_run = if stats.successful_runs > 0 {
                stats.total_tool_calls / stats.successful_runs
            } else {
                0
            };

            let budget_exceeded_rate = if stats.total_runs > 0 {
                stats.budget_exceeded_count as f64 / stats.total_runs as f64
            } else {
                0.0
            };

            task_reports.push(TaskReport {
                task,
                task_success_rate: success_rate,
                median_runtime_ms,
                llm_calls_per_run,
                tool_calls_per_run,
                budget_exceeded_rate,
                flow_failure_rate: if stats.total_runs > 0 {
                    stats.failed_runs as f64 / stats.total_runs as f64
                } else {
                    0.0
                },
            });
        }

        BenchmarkReport {
            task_reports,
            total_runs: results.len() as u32,
        }
    }
}

#[derive(Debug, Clone)]
struct BenchmarkResult {
    task: String,
    iteration: u32,
    success: bool,
    duration_ms: u128,
    error: Option<String>,
    llm_calls: u32,
    tool_calls: u32,
    budget_exceeded: bool,
}

#[derive(Debug, Clone)]
struct TaskStats {
    total_runs: u32,
    successful_runs: u32,
    failed_runs: u32,
    total_duration_ms: u128,
    total_llm_calls: u32,
    total_tool_calls: u32,
    budget_exceeded_count: u32,
}

impl TaskStats {
    fn new() -> Self {
        Self {
            total_runs: 0,
            successful_runs: 0,
            failed_runs: 0,
            total_duration_ms: 0,
            total_llm_calls: 0,
            total_tool_calls: 0,
            budget_exceeded_count: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct BenchmarkReport {
    task_reports: Vec<TaskReport>,
    total_runs: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
struct TaskReport {
    task: String,
    task_success_rate: f64,
    median_runtime_ms: u128,
    llm_calls_per_run: u32,
    tool_calls_per_run: u32,
    budget_exceeded_rate: f64,
    flow_failure_rate: f64,
}
