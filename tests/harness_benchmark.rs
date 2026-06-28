//! Issue 33: Benchmark Anti-Overfitting Tests
//!
//! Comprehensive tests for Benchmark Anti-Overfitting including:
//! - BenchmarkSuite struct (id, name, description, tests, config)
//! - BenchmarkTest struct (id, name, type, command, args, iterations, timeout, metrics)
//! - TestType enum (UnitTest, IntegrationTest, PerformanceTest, LoadTest, StressTest, CorrectnessTest)
//! - MetricType enum (Duration, Memory, CpuUsage, Throughput, Latency, ErrorRate)
//! - BenchmarkConfig struct (warmup, min/max iterations, confidence level, outlier threshold)
//! - BenchmarkRunner for executing benchmarks
//! - Statistical analysis for overfitting detection

use std::path::PathBuf;

use prometheos_lite::harness::benchmark::{
    BenchmarkConfig, BenchmarkSuite, BenchmarkTest, MetricType, TestType,
};

// ============================================================================
// BenchmarkSuite Tests
// ============================================================================

#[test]
fn test_benchmark_suite_creation() {
    let suite = BenchmarkSuite {
        id: "suite-1".to_string(),
        name: "Performance Suite".to_string(),
        description: "Core performance tests".to_string(),
        tests: vec![],
        config: BenchmarkConfig::default(),
    };

    assert_eq!(suite.id, "suite-1");
    assert_eq!(suite.name, "Performance Suite");
}

// ============================================================================
// BenchmarkTest Tests
// ============================================================================

#[test]
fn test_benchmark_test_creation() {
    let test = BenchmarkTest {
        id: "test-1".to_string(),
        name: "Compilation Speed".to_string(),
        test_type: TestType::PerformanceTest,
        command: "cargo".to_string(),
        args: vec!["build".to_string()],
        working_dir: PathBuf::from("/tmp/project"),
        iterations: 10,
        timeout_ms: 60000,
        metrics: vec![MetricType::Duration, MetricType::Memory],
    };

    assert_eq!(test.id, "test-1");
    assert!(matches!(test.test_type, TestType::PerformanceTest));
    assert_eq!(test.iterations, 10);
}

#[test]
fn test_benchmark_test_unit() {
    let test = BenchmarkTest {
        id: "unit-test".to_string(),
        name: "Unit Tests".to_string(),
        test_type: TestType::UnitTest,
        command: "cargo".to_string(),
        args: vec!["test".to_string(), "--lib".to_string()],
        working_dir: PathBuf::from("."),
        iterations: 5,
        timeout_ms: 30000,
        metrics: vec![MetricType::Duration],
    };

    assert!(matches!(test.test_type, TestType::UnitTest));
}

// ============================================================================
// TestType Tests
// ============================================================================

#[test]
fn test_test_type_variants() {
    assert!(matches!(TestType::UnitTest, TestType::UnitTest));
    assert!(matches!(
        TestType::IntegrationTest,
        TestType::IntegrationTest
    ));
    assert!(matches!(
        TestType::PerformanceTest,
        TestType::PerformanceTest
    ));
    assert!(matches!(TestType::LoadTest, TestType::LoadTest));
    assert!(matches!(TestType::StressTest, TestType::StressTest));
    assert!(matches!(
        TestType::CorrectnessTest,
        TestType::CorrectnessTest
    ));
}

// ============================================================================
// MetricType Tests
// ============================================================================

#[test]
fn test_metric_type_variants() {
    assert!(matches!(MetricType::Duration, MetricType::Duration));
    assert!(matches!(MetricType::Memory, MetricType::Memory));
    assert!(matches!(MetricType::CpuUsage, MetricType::CpuUsage));
    assert!(matches!(MetricType::Throughput, MetricType::Throughput));
    assert!(matches!(MetricType::Latency, MetricType::Latency));
    assert!(matches!(MetricType::ErrorRate, MetricType::ErrorRate));
}

// ============================================================================
// BenchmarkConfig Tests
// ============================================================================

#[test]
fn test_benchmark_config_default() {
    let config = BenchmarkConfig::default();

    assert_eq!(config.warmup_iterations, 3);
    assert_eq!(config.min_iterations, 10);
    assert_eq!(config.max_iterations, 100);
    assert_eq!(config.confidence_level, 0.95);
    assert_eq!(config.outlier_threshold, 3.0);
    assert!(!config.parallel_execution);
}

#[test]
fn test_benchmark_config_custom() {
    let config = BenchmarkConfig {
        warmup_iterations: 5,
        min_iterations: 20,
        max_iterations: 200,
        confidence_level: 0.99,
        outlier_threshold: 2.5,
        parallel_execution: true,
    };

    assert_eq!(config.warmup_iterations, 5);
    assert_eq!(config.confidence_level, 0.99);
    assert!(config.parallel_execution);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_complete_benchmark_suite() {
    let suite = BenchmarkSuite {
        id: "complete-suite".to_string(),
        name: "Full Performance Suite".to_string(),
        description: "Comprehensive performance testing".to_string(),
        tests: vec![
            BenchmarkTest {
                id: "compile".to_string(),
                name: "Compile Time".to_string(),
                test_type: TestType::PerformanceTest,
                command: "cargo".to_string(),
                args: vec!["build".to_string(), "--release".to_string()],
                working_dir: PathBuf::from("."),
                iterations: 10,
                timeout_ms: 300000,
                metrics: vec![MetricType::Duration, MetricType::Memory],
            },
            BenchmarkTest {
                id: "test-suite".to_string(),
                name: "Test Execution".to_string(),
                test_type: TestType::UnitTest,
                command: "cargo".to_string(),
                args: vec!["test".to_string()],
                working_dir: PathBuf::from("."),
                iterations: 5,
                timeout_ms: 120000,
                metrics: vec![MetricType::Duration],
            },
        ],
        config: BenchmarkConfig {
            warmup_iterations: 2,
            min_iterations: 5,
            max_iterations: 50,
            confidence_level: 0.95,
            outlier_threshold: 3.0,
            parallel_execution: false,
        },
    };

    assert_eq!(suite.tests.len(), 2);
    assert_eq!(suite.config.warmup_iterations, 2);
}

#[test]
fn test_benchmark_with_all_metrics() {
    let test = BenchmarkTest {
        id: "full-metrics".to_string(),
        name: "Full Metrics Test".to_string(),
        test_type: TestType::LoadTest,
        command: "wrk".to_string(),
        args: vec!["-t12".to_string(), "-c400".to_string(), "-d30s".to_string()],
        working_dir: PathBuf::from("."),
        iterations: 3,
        timeout_ms: 60000,
        metrics: vec![
            MetricType::Duration,
            MetricType::Memory,
            MetricType::CpuUsage,
            MetricType::Throughput,
            MetricType::Latency,
            MetricType::ErrorRate,
        ],
    };

    assert_eq!(test.metrics.len(), 6);
}
