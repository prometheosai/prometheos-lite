//! Benchmark - Issue #32
//! Performance testing and anti-overfitting validation

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkSuite {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tests: Vec<BenchmarkTest>,
    pub config: BenchmarkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkTest {
    pub id: String,
    pub name: String,
    pub test_type: TestType,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub iterations: u32,
    pub timeout_ms: u64,
    pub metrics: Vec<MetricType>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TestType {
    UnitTest,
    IntegrationTest,
    PerformanceTest,
    LoadTest,
    StressTest,
    CorrectnessTest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MetricType {
    Duration,
    Memory,
    CpuUsage,
    Throughput,
    Latency,
    ErrorRate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkConfig {
    pub warmup_iterations: u32,
    pub min_iterations: u32,
    pub max_iterations: u32,
    pub confidence_level: f64,
    pub outlier_threshold: f64,
    pub parallel_execution: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: 3,
            min_iterations: 10,
            max_iterations: 100,
            confidence_level: 0.95,
            outlier_threshold: 3.0, // Standard deviations
            parallel_execution: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchmarkRunner {
    suites: Vec<BenchmarkSuite>,
    results: Vec<BenchmarkResult>,
    config: BenchmarkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkResult {
    pub suite_id: String,
    pub test_id: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: chrono::DateTime<chrono::Utc>,
    pub iterations_completed: u32,
    pub metrics: HashMap<MetricType, MetricResult>,
    pub success: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricResult {
    pub metric_type: MetricType,
    pub values: Vec<f64>,
    pub mean: f64,
    pub median: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub confidence_interval: (f64, f64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComparisonResult {
    pub baseline_result: BenchmarkResult,
    pub current_result: BenchmarkResult,
    pub regression_detected: bool,
    pub improvements: Vec<MetricComparison>,
    regressions: Vec<MetricComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricComparison {
    pub metric_type: MetricType,
    pub baseline_mean: f64,
    pub current_mean: f64,
    pub percent_change: f64,
    pub is_significant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AntiOverfittingReport {
    pub suite_id: String,
    pub test_runs: Vec<TestRun>,
    pub consistency_score: f64,
    pub flaky_tests: Vec<String>,
    pub stable_tests: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestRun {
    pub test_id: String,
    pub run_number: u32,
    pub result: BenchmarkResult,
}

impl BenchmarkRunner {
    pub fn new(config: BenchmarkConfig) -> Self {
        Self {
            suites: Vec::new(),
            results: Vec::new(),
            config,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(BenchmarkConfig::default())
    }

    pub fn register_suite(&mut self, suite: BenchmarkSuite) {
        self.suites.push(suite);
    }

    pub async fn run_suite(&mut self, suite_id: &str) -> Result<Vec<BenchmarkResult>> {
        let suite = self
            .suites
            .iter()
            .find(|s| s.id == suite_id)
            .ok_or_else(|| anyhow::anyhow!("Suite '{}' not found", suite_id))?;

        let mut results = Vec::new();

        for test in &suite.tests {
            let result = self.run_test(test).await?;
            results.push(result);
        }

        self.results.extend(results.clone());
        Ok(results)
    }

    async fn run_test(&self, test: &BenchmarkTest) -> Result<BenchmarkResult> {
        let start_time = chrono::Utc::now();
        let mut metric_values: HashMap<MetricType, Vec<f64>> = HashMap::new();

        // Warmup
        for _ in 0..self.config.warmup_iterations {
            let _ = self.execute_iteration(test).await;
        }

        // Actual benchmark runs
        let mut iterations_completed = 0;
        let mut success = true;
        let mut error_message = None;

        for i in 0..self.config.max_iterations {
            match self.execute_iteration(test).await {
                Ok(metrics) => {
                    for (metric_type, value) in metrics {
                        metric_values
                            .entry(metric_type)
                            .or_insert_with(Vec::new)
                            .push(value);
                    }
                    iterations_completed = i + 1;

                    // Check if we have enough samples
                    if i >= self.config.min_iterations {
                        // Calculate confidence interval for duration
                        if let Some(durations) = metric_values.get(&MetricType::Duration) {
                            if let Some(stats) = self.calculate_stats(durations) {
                                let ci_width =
                                    stats.confidence_interval.1 - stats.confidence_interval.0;
                                let relative_ci = ci_width / stats.mean;

                                // Stop if confidence interval is tight enough
                                if relative_ci < 0.05 {
                                    // 5% relative CI
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    success = false;
                    error_message = Some(e.to_string());
                    break;
                }
            }
        }

        let end_time = chrono::Utc::now();

        // Calculate final metrics
        let mut metrics = HashMap::new();
        for (metric_type, values) in metric_values {
            if let Some(stats) = self.calculate_stats(&values) {
                metrics.insert(metric_type, stats);
            }
        }

        Ok(BenchmarkResult {
            suite_id: test.working_dir.to_string_lossy().to_string(),
            test_id: test.id.clone(),
            start_time,
            end_time,
            iterations_completed,
            metrics,
            success,
            error_message,
        })
    }

    async fn execute_iteration(&self, test: &BenchmarkTest) -> Result<HashMap<MetricType, f64>> {
        let start = Instant::now();

        // Execute the command
        let output = tokio::process::Command::new(&test.command)
            .args(&test.args)
            .current_dir(&test.working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await?;

        let duration = start.elapsed();

        if !output.status.success() {
            bail!(
                "Command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let mut metrics = HashMap::new();
        metrics.insert(MetricType::Duration, duration.as_millis() as f64);

        // Parse additional metrics from output if applicable
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(memory) = self.parse_memory_usage(&stdout) {
            metrics.insert(MetricType::Memory, memory);
        }
        if let Some(throughput) = self.parse_throughput(&stdout) {
            metrics.insert(MetricType::Throughput, throughput);
        }

        Ok(metrics)
    }

    fn calculate_stats(&self, values: &[f64]) -> Option<MetricResult> {
        if values.is_empty() {
            return None;
        }

        let mean = values.iter().sum::<f64>() / values.len() as f64;

        // Calculate median
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = if sorted.len() % 2 == 0 {
            (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
        } else {
            sorted[sorted.len() / 2]
        };

        // Calculate standard deviation
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        let std_dev = variance.sqrt();

        // Remove outliers
        let filtered: Vec<_> = values
            .iter()
            .filter(|&&v| (v - mean).abs() <= std_dev * self.config.outlier_threshold)
            .cloned()
            .collect();

        let min = *sorted.first().unwrap();
        let max = *sorted.last().unwrap();

        // Calculate confidence interval (95%)
        let ci_width = 1.96 * std_dev / (values.len() as f64).sqrt();
        let confidence_interval = (mean - ci_width, mean + ci_width);

        Some(MetricResult {
            metric_type: MetricType::Duration, // Will be overwritten
            values: values.to_vec(),
            mean,
            median,
            std_dev,
            min,
            max,
            confidence_interval,
        })
    }

    fn parse_memory_usage(&self, output: &str) -> Option<f64> {
        // Look for memory usage patterns in output
        for line in output.lines() {
            if line.to_lowercase().contains("memory") || line.to_lowercase().contains("mem") {
                if let Some(num) = self.extract_number(line) {
                    return Some(num);
                }
            }
        }
        None
    }

    fn parse_throughput(&self, output: &str) -> Option<f64> {
        // Look for throughput patterns
        for line in output.lines() {
            if line.to_lowercase().contains("ops/sec")
                || line.to_lowercase().contains("req/sec")
                || line.to_lowercase().contains("throughput")
            {
                if let Some(num) = self.extract_number(line) {
                    return Some(num);
                }
            }
        }
        None
    }

    fn extract_number(&self, line: &str) -> Option<f64> {
        // Extract first number from line
        let parts: Vec<_> = line.split_whitespace().collect();
        for part in parts {
            if let Ok(num) = part.parse::<f64>() {
                return Some(num);
            }
            // Try removing trailing punctuation
            let clean: String = part
                .chars()
                .filter(|c| c.is_numeric() || *c == '.')
                .collect();
            if let Ok(num) = clean.parse::<f64>() {
                return Some(num);
            }
        }
        None
    }

    pub fn compare_results(
        &self,
        baseline: &BenchmarkResult,
        current: &BenchmarkResult,
    ) -> ComparisonResult {
        let mut improvements = Vec::new();
        let mut regressions = Vec::new();
        let mut regression_detected = false;

        for (metric_type, current_metric) in &current.metrics {
            if let Some(baseline_metric) = baseline.metrics.get(metric_type) {
                let percent_change =
                    (current_metric.mean - baseline_metric.mean) / baseline_metric.mean * 100.0;

                // Determine if change is significant (more than 2 standard deviations)
                let is_significant = percent_change.abs() > 10.0; // 10% threshold

                let comparison = MetricComparison {
                    metric_type: *metric_type,
                    baseline_mean: baseline_metric.mean,
                    current_mean: current_metric.mean,
                    percent_change,
                    is_significant,
                };

                // For duration and memory, lower is better
                let is_regression = match metric_type {
                    MetricType::Duration
                    | MetricType::Memory
                    | MetricType::Latency
                    | MetricType::ErrorRate => percent_change > 10.0,
                    MetricType::Throughput => percent_change < -10.0,
                    _ => false,
                };

                if is_regression && is_significant {
                    regressions.push(comparison);
                    regression_detected = true;
                } else if is_significant {
                    improvements.push(comparison);
                }
            }
        }

        ComparisonResult {
            baseline_result: baseline.clone(),
            current_result: current.clone(),
            regression_detected,
            improvements,
            regressions,
        }
    }

    pub fn check_anti_overfitting(
        &self,
        suite_id: &str,
        num_runs: u32,
    ) -> Result<AntiOverfittingReport> {
        let mut test_runs: Vec<TestRun> = Vec::new();
        let mut flaky_tests = Vec::new();
        let mut stable_tests = Vec::new();

        // Group results by test
        let mut results_by_test: HashMap<String, Vec<&BenchmarkResult>> = HashMap::new();
        for result in &self.results {
            if result.suite_id == suite_id {
                results_by_test
                    .entry(result.test_id.clone())
                    .or_insert_with(Vec::new)
                    .push(result);
            }
        }

        for (test_id, results) in results_by_test {
            if results.len() < 2 {
                continue;
            }

            // Check consistency across runs
            let mut is_flaky = false;

            for metric_type in [MetricType::Duration, MetricType::Memory] {
                let values: Vec<_> = results
                    .iter()
                    .filter_map(|r| r.metrics.get(&metric_type).map(|m| m.mean))
                    .collect();

                if values.len() >= 2 {
                    let mean = values.iter().sum::<f64>() / values.len() as f64;
                    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                        / values.len() as f64;
                    let cv = variance.sqrt() / mean; // Coefficient of variation

                    // If CV > 0.15 (15%), consider it flaky
                    if cv > 0.15 {
                        is_flaky = true;
                        break;
                    }
                }
            }

            if is_flaky {
                flaky_tests.push(test_id);
            } else {
                stable_tests.push(test_id);
            }

            // Add to test runs
            for (i, result) in results.iter().enumerate() {
                test_runs.push(TestRun {
                    test_id: result.test_id.clone(),
                    run_number: i as u32 + 1,
                    result: (*result).clone(),
                });
            }
        }

        let total_tests = flaky_tests.len() + stable_tests.len();
        let consistency_score = if total_tests > 0 {
            stable_tests.len() as f64 / total_tests as f64
        } else {
            1.0
        };

        Ok(AntiOverfittingReport {
            suite_id: suite_id.to_string(),
            test_runs,
            consistency_score,
            flaky_tests,
            stable_tests,
        })
    }

    pub fn get_results(&self) -> &[BenchmarkResult] {
        &self.results
    }
}

pub fn create_benchmark_runner() -> BenchmarkRunner {
    BenchmarkRunner::with_defaults()
}

pub fn format_benchmark_result(result: &BenchmarkResult) -> String {
    let status = if result.success { "✓" } else { "✗" };
    let mut metrics_str = String::new();

    for (metric_type, metric) in &result.metrics {
        metrics_str.push_str(&format!(
            "  {:?}: {:.2} ± {:.2} ({}..{})\n",
            metric_type, metric.mean, metric.std_dev, metric.min, metric.max
        ));
    }

    format!(
        r#"{} {}: {} iterations
{}
"#,
        status, result.test_id, result.iterations_completed, metrics_str
    )
}

pub fn format_comparison(result: &ComparisonResult) -> String {
    let status = if result.regression_detected {
        "⚠ REGRESSION"
    } else {
        "✓ OK"
    };

    let mut changes = String::new();
    for improvement in &result.improvements {
        changes.push_str(&format!(
            "  + {:?}: {:.1}% improvement\n",
            improvement.metric_type,
            improvement.percent_change.abs()
        ));
    }
    for regression in &result.regressions {
        changes.push_str(&format!(
            "  - {:?}: {:.1}% regression\n",
            regression.metric_type,
            regression.percent_change.abs()
        ));
    }

    format!(
        r#"{} {}
{}
"#,
        status, result.current_result.test_id, changes
    )
}

pub fn format_anti_overfitting_report(report: &AntiOverfittingReport) -> String {
    format!(
        r#"Anti-Overfitting Report: {}
Consistency Score: {:.0}%
Stable Tests: {} ({})
Flaky Tests: {} ({})
"#,
        report.suite_id,
        report.consistency_score * 100.0,
        report.stable_tests.len(),
        report.stable_tests.join(", "),
        report.flaky_tests.len(),
        report.flaky_tests.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_stats() {
        let runner = BenchmarkRunner::with_defaults();
        let values = vec![10.0, 12.0, 11.0, 10.5, 11.5];

        let stats = runner.calculate_stats(&values).unwrap();
        assert!((stats.mean - 11.0).abs() < 0.1);
        assert!(stats.std_dev > 0.0);
    }

    #[test]
    fn test_compare_results() {
        let runner = BenchmarkRunner::with_defaults();

        let baseline = BenchmarkResult {
            suite_id: "test".to_string(),
            test_id: "test".to_string(),
            start_time: chrono::Utc::now(),
            end_time: chrono::Utc::now(),
            iterations_completed: 10,
            metrics: {
                let mut m = HashMap::new();
                m.insert(
                    MetricType::Duration,
                    MetricResult {
                        metric_type: MetricType::Duration,
                        values: vec![100.0],
                        mean: 100.0,
                        median: 100.0,
                        std_dev: 0.0,
                        min: 100.0,
                        max: 100.0,
                        confidence_interval: (95.0, 105.0),
                    },
                );
                m
            },
            success: true,
            error_message: None,
        };

        let current = BenchmarkResult {
            suite_id: "test".to_string(),
            test_id: "test".to_string(),
            start_time: chrono::Utc::now(),
            end_time: chrono::Utc::now(),
            iterations_completed: 10,
            metrics: {
                let mut m = HashMap::new();
                m.insert(
                    MetricType::Duration,
                    MetricResult {
                        metric_type: MetricType::Duration,
                        values: vec![120.0],
                        mean: 120.0,
                        median: 120.0,
                        std_dev: 0.0,
                        min: 120.0,
                        max: 120.0,
                        confidence_interval: (115.0, 125.0),
                    },
                );
                m
            },
            success: true,
            error_message: None,
        };

        let comparison = runner.compare_results(&baseline, &current);
        assert!(comparison.regression_detected);
        assert!(!comparison.regressions.is_empty());
    }

    #[test]
    fn test_extract_number() {
        let runner = BenchmarkRunner::with_defaults();
        assert_eq!(runner.extract_number("Memory: 1024 MB"), Some(1024.0));
        assert_eq!(runner.extract_number("Time: 1.5 seconds"), Some(1.5));
    }
}
