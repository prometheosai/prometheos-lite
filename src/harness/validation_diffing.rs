//! P2-Issue7: Validation result comparison and diffing
//!
//! This module provides comprehensive validation result comparison and diffing
//! capabilities with detailed analysis of changes, regressions, and improvements.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn, error};

/// P2-Issue7: Validation result comparison configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationComparisonConfig {
    /// Comparison strategies
    pub comparison_strategies: Vec<ComparisonStrategy>,
    /// Diff configuration
    pub diff_config: DiffConfig,
    /// Analysis configuration
    pub analysis_config: AnalysisConfig,
    /// Reporting configuration
    pub reporting_config: ReportingConfig,
}

/// P2-Issue7: Comparison strategies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComparisonStrategy {
    /// Strategy name
    pub name: String,
    /// Strategy type
    pub strategy_type: ComparisonStrategyType,
    /// Priority (higher = more important)
    pub priority: u8,
    /// Enabled
    pub enabled: bool,
    /// Configuration
    pub config: serde_json::Value,
}

/// P2-Issue7: Comparison strategy types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComparisonStrategyType {
    /// Exact comparison
    Exact,
    /// Semantic comparison
    Semantic,
    /// Performance comparison
    Performance,
    /// Issue comparison
    Issue,
    /// Resource usage comparison
    ResourceUsage,
    /// Custom comparison
    Custom,
}

/// P2-Issue7: Diff configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffConfig {
    /// Diff algorithm
    pub algorithm: DiffAlgorithm,
    /// Context lines
    pub context_lines: usize,
    /// Ignore whitespace
    pub ignore_whitespace: bool,
    /// Ignore case
    pub ignore_case: bool,
    /// Max diff size
    pub max_diff_size: usize,
}

/// P2-Issue7: Diff algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiffAlgorithm {
    /// Myers algorithm
    Myers,
    /// Patience diff
    Patience,
    /// Histogram diff
    Histogram,
    /// Minimal diff
    Minimal,
    /// Unified diff
    Unified,
}

/// P2-Issue7: Analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisConfig {
    /// Enable regression detection
    pub regression_detection_enabled: bool,
    /// Enable improvement detection
    pub improvement_detection_enabled: bool,
    /// Performance analysis
    pub performance_analysis: PerformanceAnalysisConfig,
    /// Issue analysis
    pub issue_analysis: IssueAnalysisConfig,
    /// Trend analysis
    pub trend_analysis: TrendAnalysisConfig,
}

/// P2-Issue7: Performance analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceAnalysisConfig {
    /// Performance regression threshold (percentage)
    pub regression_threshold_percent: f64,
    /// Performance improvement threshold (percentage)
    pub improvement_threshold_percent: f64,
    /// Statistical significance level
    pub significance_level: f64,
    /// Minimum sample size
    pub min_sample_size: usize,
}

/// P2-Issue7: Issue analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IssueAnalysisConfig {
    /// Issue severity mapping
    pub severity_mapping: HashMap<String, crate::harness::validation_artifacts::ValidationIssueSeverity>,
    /// Issue category mapping
    pub category_mapping: HashMap<String, String>,
    /// Issue pattern detection
    pub pattern_detection_enabled: bool,
    /// Issue correlation analysis
    pub correlation_analysis_enabled: bool,
}

/// P2-Issue7: Trend analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrendAnalysisConfig {
    /// Trend window size
    pub window_size: usize,
    /// Trend detection algorithm
    pub algorithm: TrendAlgorithm,
    /// Minimum trend points
    pub min_trend_points: usize,
    /// Trend confidence threshold
    pub confidence_threshold: f64,
}

/// P2-Issue7: Trend algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrendAlgorithm {
    /// Linear regression
    LinearRegression,
    /// Moving average
    MovingAverage,
    /// Exponential smoothing
    ExponentialSmoothing,
    /// Seasonal decomposition
    SeasonalDecomposition,
}

/// P2-Issue7: Reporting configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportingConfig {
    /// Report formats
    pub formats: Vec<ReportFormat>,
    /// Include details
    pub include_details: bool,
    /// Include recommendations
    pub include_recommendations: bool,
    /// Maximum report size
    pub max_report_size: usize,
    /// Template configuration
    pub template_config: ReportTemplateConfig,
}

/// P2-Issue7: Report formats
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReportFormat {
    /// JSON format
    Json,
    /// HTML format
    Html,
    /// Markdown format
    Markdown,
    /// Plain text format
    Text,
    /// CSV format
    Csv,
}

/// P2-Issue7: Report template configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportTemplateConfig {
    /// Template path
    pub template_path: Option<String>,
    /// Custom CSS
    pub custom_css: Option<String>,
    /// Include charts
    pub include_charts: bool,
    /// Chart configuration
    pub chart_config: ChartConfig,
}

/// P2-Issue7: Chart configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChartConfig {
    /// Chart types
    pub chart_types: Vec<ChartType>,
    /// Chart width
    pub width: u32,
    /// Chart height
    pub height: u32,
    /// Color scheme
    pub color_scheme: String,
}

/// P2-Issue7: Chart types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChartType {
    /// Line chart
    Line,
    /// Bar chart
    Bar,
    /// Pie chart
    Pie,
    /// Scatter plot
    Scatter,
    /// Heatmap
    Heatmap,
}

/// P2-Issue7: Validation result comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationComparison {
    /// Comparison ID
    pub id: String,
    /// Baseline result
    pub baseline: crate::harness::validation::ValidationResult,
    /// Current result
    pub current: crate::harness::validation::ValidationResult,
    /// Comparison timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Comparison summary
    pub summary: ComparisonSummary,
    /// Detailed diffs
    pub diffs: Vec<ValidationDiff>,
    /// Performance comparison
    pub performance_comparison: PerformanceComparison,
    /// Issue comparison
    pub issue_comparison: IssueComparison,
    /// Resource usage comparison
    pub resource_comparison: ResourceUsageComparison,
    /// Analysis results
    pub analysis: ComparisonAnalysis,
}

/// P2-Issue7: Comparison summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComparisonSummary {
    /// Overall status change
    pub status_change: StatusChange,
    /// Performance change
    pub performance_change: PerformanceChange,
    /// Issue count change
    pub issue_count_change: IssueCountChange,
    /// Resource usage change
    pub resource_change: ResourceChange,
    /// Overall assessment
    pub overall_assessment: ComparisonAssessment,
}

/// P2-Issue7: Status change
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StatusChange {
    /// No change
    NoChange,
    /// Improved
    Improved,
    /// Regressed
    Regressed,
    /// Mixed
    Mixed,
}

/// P2-Issue7: Performance change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceChange {
    /// Execution time change (percentage)
    pub execution_time_change_percent: f64,
    /// Memory usage change (percentage)
    pub memory_change_percent: f64,
    /// CPU usage change (percentage)
    pub cpu_change_percent: f64,
    /// Overall performance assessment
    pub assessment: PerformanceAssessment,
}

/// P2-Issue7: Performance assessment
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PerformanceAssessment {
    /// No significant change
    NoSignificantChange,
    /// Improved
    Improved,
    /// Regressed
    Regressed,
    /// Mixed results
    Mixed,
}

/// P2-Issue7: Issue count change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IssueCountChange {
    /// Total issues change
    pub total_change: i32,
    /// Critical issues change
    pub critical_change: i32,
    /// Error issues change
    pub error_change: i32,
    /// Warning issues change
    pub warning_change: i32,
    /// Info issues change
    pub info_change: i32,
}

/// P2-Issue7: Resource change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceChange {
    /// Memory usage change (MB)
    pub memory_change_mb: f64,
    /// CPU time change (seconds)
    pub cpu_time_change_sec: f64,
    /// Disk I/O change (MB)
    pub disk_io_change_mb: f64,
    /// Network I/O change (MB)
    pub network_io_change_mb: f64,
    /// File handles change
    pub file_handles_change: i32,
}

/// P2-Issue7: Comparison assessment
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComparisonAssessment {
    /// No significant changes
    NoSignificantChanges,
    /// Improvements detected
    ImprovementsDetected,
    /// Regressions detected
    RegressionsDetected,
    /// Mixed changes
    MixedChanges,
    /// Cannot determine
    CannotDetermine,
}

/// P2-Issue7: Validation diff
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationDiff {
    /// Diff type
    pub diff_type: DiffType,
    /// Field name
    pub field_name: String,
    /// Baseline value
    pub baseline_value: serde_json::Value,
    /// Current value
    pub current_value: serde_json::Value,
    /// Change description
    pub change_description: String,
    /// Change significance
    pub significance: ChangeSignificance,
}

/// P2-Issue7: Diff types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiffType {
    /// Added
    Added,
    /// Removed
    Removed,
    /// Modified
    Modified,
    /// Moved
    Moved,
}

/// P2-Issue7: Change significance
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeSignificance {
    /// Trivial change
    Trivial,
    /// Minor change
    Minor,
    /// Moderate change
    Moderate,
    /// Major change
    Major,
    /// Critical change
    Critical,
}

/// P2-Issue7: Performance comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceComparison {
    /// Execution time comparison
    pub execution_time: MetricComparison,
    /// Memory usage comparison
    pub memory_usage: MetricComparison,
    /// CPU usage comparison
    pub cpu_usage: MetricComparison,
    /// Disk I/O comparison
    pub disk_io: MetricComparison,
    /// Network I/O comparison
    pub network_io: MetricComparison,
    /// Overall performance score
    pub overall_score: f64,
}

/// P2-Issue7: Metric comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricComparison {
    /// Baseline value
    pub baseline: f64,
    /// Current value
    pub current: f64,
    /// Absolute change
    pub absolute_change: f64,
    /// Percentage change
    pub percentage_change: f64,
    /// Statistical significance
    pub significance: StatisticalSignificance,
}

/// P2-Issue7: Statistical significance
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StatisticalSignificance {
    /// Not significant
    NotSignificant,
    /// Marginally significant
    MarginallySignificant,
    /// Significant
    Significant,
    /// Highly significant
    HighlySignificant,
}

/// P2-Issue7: Issue comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IssueComparison {
    /// New issues
    pub new_issues: Vec<crate::harness::validation_artifacts::ValidationIssue>,
    /// Resolved issues
    pub resolved_issues: Vec<crate::harness::validation_artifacts::ValidationIssue>,
    /// Persisting issues
    pub persisting_issues: Vec<PersistingIssue>,
    /// Issue severity changes
    pub severity_changes: Vec<IssueSeverityChange>,
    /// Issue category changes
    pub category_changes: Vec<IssueCategoryChange>,
}

/// P2-Issue7: Persisting issue
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistingIssue {
    /// Issue ID
    pub id: String,
    /// Issue details
    pub issue: crate::harness::validation_artifacts::ValidationIssue,
    /// Persistence duration
    pub persistence_duration: Duration,
    /// First occurrence
    pub first_occurrence: chrono::DateTime<chrono::Utc>,
}

/// P2-Issue7: Issue severity change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IssueSeverityChange {
    /// Issue ID
    pub issue_id: String,
    /// Issue message
    pub message: String,
    /// Previous severity
    pub previous_severity: crate::harness::validation_artifacts::ValidationIssueSeverity,
    /// Current severity
    pub current_severity: crate::harness::validation_artifacts::ValidationIssueSeverity,
    /// Change impact
    pub impact: SeverityChangeImpact,
}

/// P2-Issue7: Severity change impact
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SeverityChangeImpact {
    /// Improved (less severe)
    Improved,
    /// Worsened (more severe)
    Worsened,
    /// No impact
    NoImpact,
}

/// P2-Issue7: Issue category change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IssueCategoryChange {
    /// Issue ID
    pub issue_id: String,
    /// Issue message
    pub message: String,
    /// Previous category
    pub previous_category: String,
    /// Current category
    pub current_category: String,
    /// Change reason
    pub reason: String,
}

/// P2-Issue7: Resource usage comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceUsageComparison {
    /// Memory usage comparison
    pub memory: MetricComparison,
    /// CPU time comparison
    pub cpu_time: MetricComparison,
    /// Disk I/O comparison
    pub disk_io: MetricComparison,
    /// Network I/O comparison
    pub network_io: MetricComparison,
    /// File handles comparison
    pub file_handles: MetricComparison,
    /// Overall resource efficiency score
    pub efficiency_score: f64,
}

/// P2-Issue7: Comparison analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComparisonAnalysis {
    /// Regression analysis
    pub regression_analysis: RegressionAnalysis,
    /// Improvement analysis
    pub improvement_analysis: ImprovementAnalysis,
    /// Trend analysis
    pub trend_analysis: TrendAnalysis,
    /// Recommendations
    pub recommendations: Vec<Recommendation>,
    /// Risk assessment
    pub risk_assessment: RiskAssessment,
}

/// P2-Issue7: Regression analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionAnalysis {
    /// Regressions detected
    pub regressions: Vec<Regression>,
    /// Regression severity distribution
    pub severity_distribution: HashMap<RegressionSeverity, usize>,
    /// Regression impact score
    pub impact_score: f64,
    /// Root cause analysis
    pub root_causes: Vec<String>,
}

/// P2-Issue7: Regression
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Regression {
    /// Regression ID
    pub id: String,
    /// Regression type
    pub regression_type: RegressionType,
    /// Description
    pub description: String,
    /// Severity
    pub severity: RegressionSeverity,
    /// Affected components
    pub affected_components: Vec<String>,
    /// Estimated impact
    pub estimated_impact: RegressionImpact,
}

/// P2-Issue7: Regression types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegressionType {
    /// Performance regression
    Performance,
    /// Functional regression
    Functional,
    /// Security regression
    Security,
    /// Resource regression
    Resource,
    /// Quality regression
    Quality,
}

/// P2-Issue7: Regression severity
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegressionSeverity {
    /// Low severity
    Low,
    /// Medium severity
    Medium,
    /// High severity
    High,
    /// Critical severity
    Critical,
}

/// P2-Issue7: Regression impact
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegressionImpact {
    /// Minimal impact
    Minimal,
    /// Minor impact
    Minor,
    /// Moderate impact
    Moderate,
    /// Major impact
    Major,
    /// Severe impact
    Severe,
}

/// P2-Issue7: Improvement analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImprovementAnalysis {
    /// Improvements detected
    pub improvements: Vec<Improvement>,
    /// Improvement category distribution
    pub category_distribution: HashMap<String, usize>,
    /// Improvement impact score
    pub impact_score: f64,
    /// Improvement confidence
    pub confidence: f64,
}

/// P2-Issue7: Improvement
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Improvement {
    /// Improvement ID
    pub id: String,
    /// Improvement type
    pub improvement_type: ImprovementType,
    /// Description
    pub description: String,
    /// Magnitude
    pub magnitude: ImprovementMagnitude,
    /// Affected components
    pub affected_components: Vec<String>,
    /// Confidence level
    pub confidence: f64,
}

/// P2-Issue7: Improvement types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImprovementType {
    /// Performance improvement
    Performance,
    /// Quality improvement
    Quality,
    /// Resource efficiency improvement
    ResourceEfficiency,
    /// Usability improvement
    Usability,
    /// Security improvement
    Security,
}

/// P2-Issue7: Improvement magnitude
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImprovementMagnitude {
    /// Minor improvement
    Minor,
    /// Moderate improvement
    Moderate,
    /// Significant improvement
    Significant,
    /// Major improvement
    Major,
}

/// P2-Issue7: Trend analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrendAnalysis {
    /// Performance trends
    pub performance_trends: Vec<Trend>,
    /// Quality trends
    pub quality_trends: Vec<Trend>,
    /// Resource trends
    pub resource_trends: Vec<Trend>,
    /// Overall trend direction
    pub overall_direction: TrendDirection,
    /// Trend confidence
    pub confidence: f64,
}

/// P2-Issue7: Trend
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trend {
    /// Metric name
    pub metric_name: String,
    /// Trend direction
    pub direction: TrendDirection,
    /// Trend strength
    pub strength: f64,
    /// Trend duration
    pub duration: Duration,
    /// Data points
    pub data_points: Vec<TrendDataPoint>,
}

/// P2-Issue7: Trend direction
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrendDirection {
    /// Improving
    Improving,
    /// Degrading
    Degrading,
    /// Stable
    Stable,
    /// Fluctuating
    Fluctuating,
}

/// P2-Issue7: Trend data point
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrendDataPoint {
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Value
    pub value: f64,
    /// Context
    pub context: HashMap<String, String>,
}

/// P2-Issue7: Recommendation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recommendation {
    /// Recommendation ID
    pub id: String,
    /// Recommendation type
    pub recommendation_type: RecommendationType,
    /// Priority
    pub priority: RecommendationPriority,
    /// Title
    pub title: String,
    /// Description
    pub description: String,
    /// Actionable steps
    pub actionable_steps: Vec<String>,
    /// Expected impact
    pub expected_impact: String,
    /// Effort required
    pub effort: EffortLevel,
    /// Confidence
    pub confidence: f64,
}

/// P2-Issue7: Recommendation types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecommendationType {
    /// Performance recommendation
    Performance,
    /// Quality recommendation
    Quality,
    /// Security recommendation
    Security,
    /// Resource recommendation
    Resource,
    /// Process recommendation
    Process,
}

/// P2-Issue7: Recommendation priority
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecommendationPriority {
    /// Low priority
    Low,
    /// Medium priority
    Medium,
    /// High priority
    High,
    /// Critical priority
    Critical,
}

/// P2-Issue7: Effort level
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EffortLevel {
    /// Minimal effort
    Minimal,
    /// Low effort
    Low,
    /// Medium effort
    Medium,
    /// High effort
    High,
    /// Significant effort
    Significant,
}

/// P2-Issue7: Risk assessment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskAssessment {
    /// Overall risk level
    pub overall_risk: RiskLevel,
    /// Risk factors
    pub risk_factors: Vec<RiskFactor>,
    /// Mitigation strategies
    pub mitigation_strategies: Vec<MitigationStrategy>,
    /// Risk score
    pub risk_score: f64,
}

/// P2-Issue7: Risk levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    /// Low risk
    Low,
    /// Medium risk
    Medium,
    /// High risk
    High,
    /// Critical risk
    Critical,
}

/// P2-Issue7: Risk factor
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskFactor {
    /// Factor name
    pub name: String,
    /// Factor type
    pub factor_type: RiskFactorType,
    /// Impact level
    pub impact: ImpactLevel,
    /// Probability
    pub probability: f64,
    /// Description
    pub description: String,
}

/// P2-Issue7: Risk factor types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskFactorType {
    /// Performance risk
    Performance,
    /// Quality risk
    Quality,
    /// Security risk
    Security,
    /// Resource risk
    Resource,
    /// Operational risk
    Operational,
}

/// P2-Issue7: Impact levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImpactLevel {
    /// Minimal impact
    Minimal,
    /// Minor impact
    Minor,
    /// Moderate impact
    Moderate,
    /// Major impact
    Major,
    /// Severe impact
    Severe,
}

/// P2-Issue7: Mitigation strategy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MitigationStrategy {
    /// Strategy name
    pub name: String,
    /// Strategy description
    pub description: String,
    /// Effectiveness
    pub effectiveness: f64,
    /// Cost
    pub cost: CostLevel,
    /// Implementation time
    pub implementation_time: Duration,
}

/// P2-Issue7: Cost levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CostLevel {
    /// Low cost
    Low,
    /// Medium cost
    Medium,
    /// High cost
    High,
    /// Very high cost
    VeryHigh,
}

/// P2-Issue7: Validation comparison engine
pub struct ValidationComparisonEngine {
    config: ValidationComparisonConfig,
    comparison_strategies: Vec<Box<dyn ComparisonStrategy>>,
    analyzers: Vec<Box<dyn ComparisonAnalyzer>>,
    reporters: Vec<Box<dyn ComparisonReporter>>,
}

impl Default for ValidationComparisonConfig {
    fn default() -> Self {
        Self {
            comparison_strategies: vec![
                ComparisonStrategy {
                    name: "exact".to_string(),
                    strategy_type: ComparisonStrategyType::Exact,
                    priority: 100,
                    enabled: true,
                    config: serde_json::json!({}),
                },
                ComparisonStrategy {
                    name: "semantic".to_string(),
                    strategy_type: ComparisonStrategyType::Semantic,
                    priority: 90,
                    enabled: true,
                    config: serde_json::json!({}),
                },
                ComparisonStrategy {
                    name: "performance".to_string(),
                    strategy_type: ComparisonStrategyType::Performance,
                    priority: 80,
                    enabled: true,
                    config: serde_json::json!({}),
                },
            ],
            diff_config: DiffConfig {
                algorithm: DiffAlgorithm::Myers,
                context_lines: 3,
                ignore_whitespace: false,
                ignore_case: false,
                max_diff_size: 10000,
            },
            analysis_config: AnalysisConfig {
                regression_detection_enabled: true,
                improvement_detection_enabled: true,
                performance_analysis: PerformanceAnalysisConfig {
                    regression_threshold_percent: 10.0,
                    improvement_threshold_percent: 5.0,
                    significance_level: 0.05,
                    min_sample_size: 3,
                },
                issue_analysis: IssueAnalysisConfig {
                    severity_mapping: HashMap::new(),
                    category_mapping: HashMap::new(),
                    pattern_detection_enabled: true,
                    correlation_analysis_enabled: true,
                },
                trend_analysis: TrendAnalysisConfig {
                    window_size: 10,
                    algorithm: TrendAlgorithm::LinearRegression,
                    min_trend_points: 5,
                    confidence_threshold: 0.8,
                },
            },
            reporting_config: ReportingConfig {
                formats: vec![ReportFormat::Json, ReportFormat::Html],
                include_details: true,
                include_recommendations: true,
                max_report_size: 100000,
                template_config: ReportTemplateConfig {
                    template_path: None,
                    custom_css: None,
                    include_charts: true,
                    chart_config: ChartConfig {
                        chart_types: vec![ChartType::Line, ChartType::Bar],
                        width: 800,
                        height: 600,
                        color_scheme: "default".to_string(),
                    },
                },
            },
        }
    }
}

impl ValidationComparisonEngine {
    /// Create new comparison engine
    pub fn new() -> Self {
        Self::with_config(ValidationComparisonConfig::default())
    }
    
    /// Create engine with custom configuration
    pub fn with_config(config: ValidationComparisonConfig) -> Self {
        let mut engine = Self {
            comparison_strategies: Vec::new(),
            analyzers: Vec::new(),
            reporters: Vec::new(),
            config,
        };
        
        engine.initialize_strategies();
        engine.initialize_analyzers();
        engine.initialize_reporters();
        
        engine
    }
    
    /// Compare two validation results
    pub async fn compare(
        &self,
        baseline: crate::harness::validation::ValidationResult,
        current: crate::harness::validation::ValidationResult,
    ) -> Result<ValidationComparison> {
        let comparison_id = format!("comp_{}", chrono::Utc::now().timestamp_nanos());
        
        info!("Starting validation comparison: {}", comparison_id);
        
        // Create comparison object
        let mut comparison = ValidationComparison {
            id: comparison_id.clone(),
            baseline: baseline.clone(),
            current: current.clone(),
            timestamp: chrono::Utc::now(),
            summary: ComparisonSummary::default(),
            diffs: Vec::new(),
            performance_comparison: PerformanceComparison::default(),
            issue_comparison: IssueComparison::default(),
            resource_comparison: ResourceUsageComparison::default(),
            analysis: ComparisonAnalysis::default(),
        };
        
        // Apply comparison strategies
        for strategy in &self.comparison_strategies {
            if self.is_strategy_enabled(strategy) {
                strategy.compare(&baseline, &current, &mut comparison).await?;
            }
        }
        
        // Generate summary
        self.generate_summary(&mut comparison).await?;
        
        // Run analysis
        for analyzer in &self.analyzers {
            analyzer.analyze(&mut comparison).await?;
        }
        
        info!("Validation comparison completed: {}", comparison_id);
        
        Ok(comparison)
    }
    
    /// Compare multiple validation results
    pub async fn compare_multiple(
        &self,
        results: Vec<crate::harness::validation::ValidationResult>,
    ) -> Result<Vec<ValidationComparison>> {
        let mut comparisons = Vec::new();
        
        for i in 1..results.len() {
            let comparison = self.compare(results[i-1].clone(), results[i].clone()).await?;
            comparisons.push(comparison);
        }
        
        Ok(comparisons)
    }
    
    /// Generate comparison report
    pub async fn generate_report(
        &self,
        comparison: &ValidationComparison,
        format: ReportFormat,
    ) -> Result<String> {
        for reporter in &self.reporters {
            if reporter.supports_format(format) {
                return reporter.generate_report(comparison, format).await;
            }
        }
        
        Err(anyhow::anyhow!("No reporter supports format: {:?}", format))
    }
    
    /// Check if strategy is enabled
    fn is_strategy_enabled(&self, strategy: &Box<dyn ComparisonStrategy>) -> bool {
        let strategy_name = strategy.name();
        self.config.comparison_strategies
            .iter()
            .find(|s| s.name == strategy_name)
            .map(|s| s.enabled)
            .unwrap_or(false)
    }
    
    /// Generate comparison summary
    async fn generate_summary(&self, comparison: &mut ValidationComparison) -> Result<()> {
        // Compare status
        comparison.summary.status_change = self.compare_status(
            &comparison.baseline.status,
            &comparison.current.status,
        );
        
        // Compare performance
        comparison.summary.performance_change = self.calculate_performance_change(
            &comparison.performance_comparison,
        );
        
        // Compare issue counts
        comparison.summary.issue_count_change = self.calculate_issue_count_change(
            &comparison.issue_comparison,
        );
        
        // Compare resource usage
        comparison.summary.resource_change = self.calculate_resource_change(
            &comparison.resource_comparison,
        );
        
        // Determine overall assessment
        comparison.summary.overall_assessment = self.determine_overall_assessment(
            &comparison.summary,
        );
        
        Ok(())
    }
    
    /// Compare validation status
    fn compare_status(
        &self,
        baseline: &crate::harness::validation::ValidationStatus,
        current: &crate::harness::validation::ValidationStatus,
    ) -> StatusChange {
        use crate::harness::validation::ValidationStatus;
        
        match (baseline, current) {
            (ValidationStatus::Passed, ValidationStatus::Passed) => StatusChange::NoChange,
            (ValidationStatus::Failed, ValidationStatus::Passed) => StatusChange::Improved,
            (ValidationStatus::Passed, ValidationStatus::Failed) => StatusChange::Regressed,
            _ => StatusChange::Mixed,
        }
    }
    
    /// Calculate performance change
    fn calculate_performance_change(&self, perf_comp: &PerformanceComparison) -> PerformanceChange {
        let time_change = perf_comp.execution_time.percentage_change;
        let memory_change = perf_comp.memory_usage.percentage_change;
        let cpu_change = perf_comp.cpu_usage.percentage_change;
        
        let assessment = if time_change > 10.0 || memory_change > 10.0 || cpu_change > 10.0 {
            PerformanceAssessment::Regressed
        } else if time_change < -5.0 || memory_change < -5.0 || cpu_change < -5.0 {
            PerformanceAssessment::Improved
        } else {
            PerformanceAssessment::NoSignificantChange
        };
        
        PerformanceChange {
            execution_time_change_percent: time_change,
            memory_change_percent: memory_change,
            cpu_change_percent: cpu_change,
            assessment,
        }
    }
    
    /// Calculate issue count change
    fn calculate_issue_count_change(&self, issue_comp: &IssueComparison) -> IssueCountChange {
        let baseline_total = issue_comp.persisting_issues.len() + issue_comp.resolved_issues.len();
        let current_total = issue_comp.new_issues.len() + issue_comp.persisting_issues.len();
        
        IssueCountChange {
            total_change: current_total as i32 - baseline_total as i32,
            critical_change: 0, // Would need detailed issue analysis
            error_change: 0,
            warning_change: 0,
            info_change: 0,
        }
    }
    
    /// Calculate resource change
    fn calculate_resource_change(&self, resource_comp: &ResourceUsageComparison) -> ResourceChange {
        ResourceChange {
            memory_change_mb: resource_comp.memory.absolute_change,
            cpu_time_change_sec: resource_comp.cpu_time.absolute_change / 1000.0,
            disk_io_change_mb: resource_comp.disk_io.absolute_change,
            network_io_change_mb: resource_comp.network_io.absolute_change,
            file_handles_change: resource_comp.file_handles.absolute_change as i32,
        }
    }
    
    /// Determine overall assessment
    fn determine_overall_assessment(&self, summary: &ComparisonSummary) -> ComparisonAssessment {
        let has_regressions = matches!(summary.status_change, StatusChange::Regressed) ||
            matches!(summary.performance_change.assessment, PerformanceAssessment::Regressed) ||
            summary.issue_count_change.total_change > 0;
        
        let has_improvements = matches!(summary.status_change, StatusChange::Improved) ||
            matches!(summary.performance_change.assessment, PerformanceAssessment::Improved) ||
            summary.issue_count_change.total_change < 0;
        
        match (has_regressions, has_improvements) {
            (true, true) => ComparisonAssessment::MixedChanges,
            (true, false) => ComparisonAssessment::RegressionsDetected,
            (false, true) => ComparisonAssessment::ImprovementsDetected,
            (false, false) => ComparisonAssessment::NoSignificantChanges,
        }
    }
    
    /// Initialize comparison strategies
    fn initialize_strategies(&mut self) {
        self.comparison_strategies.push(Box::new(ExactComparisonStrategy::new()));
        self.comparison_strategies.push(Box::new(SemanticComparisonStrategy::new()));
        self.comparison_strategies.push(Box::new(PerformanceComparisonStrategy::new()));
        self.comparison_strategies.push(Box::new(IssueComparisonStrategy::new()));
        self.comparison_strategies.push(Box::new(ResourceUsageComparisonStrategy::new()));
    }
    
    /// Initialize analyzers
    fn initialize_analyzers(&mut self) {
        self.analyzers.push(Box::new(RegressionAnalyzer::new()));
        self.analyzers.push(Box::new(ImprovementAnalyzer::new()));
        self.analyzers.push(Box::new(TrendAnalyzer::new()));
    }
    
    /// Initialize reporters
    fn initialize_reporters(&mut self) {
        self.reporters.push(Box::new(JsonComparisonReporter::new()));
        self.reporters.push(Box::new(HtmlComparisonReporter::new()));
        self.reporters.push(Box::new(MarkdownComparisonReporter::new()));
    }
}

/// P2-Issue7: Comparison strategy trait
pub trait ComparisonStrategy: Send + Sync {
    /// Strategy name
    fn name(&self) -> String;
    
    /// Compare two validation results
    async fn compare(
        &self,
        baseline: &crate::harness::validation::ValidationResult,
        current: &crate::harness::validation::ValidationResult,
        comparison: &mut ValidationComparison,
    ) -> Result<()>;
}

/// P2-Issue7: Exact comparison strategy
pub struct ExactComparisonStrategy;

impl ExactComparisonStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonStrategy for ExactComparisonStrategy {
    fn name(&self) -> String {
        "exact".to_string()
    }
    
    async fn compare(
        &self,
        baseline: &crate::harness::validation::ValidationResult,
        current: &crate::harness::validation::ValidationResult,
        comparison: &mut ValidationComparison,
    ) -> Result<()> {
        // Compare basic fields
        if baseline.status != current.status {
            comparison.diffs.push(ValidationDiff {
                diff_type: DiffType::Modified,
                field_name: "status".to_string(),
                baseline_value: serde_json::to_value(&baseline.status)?,
                current_value: serde_json::to_value(&current.status)?,
                change_description: format!("Status changed from {:?} to {:?}", baseline.status, current.status),
                significance: ChangeSignificance::Major,
            });
        }
        
        // Compare duration
        match (baseline.duration_ms, current.duration_ms) {
            (Some(baseline_duration), Some(current_duration)) => {
                if baseline_duration != current_duration {
                    let change_percent = ((current_duration as f64 - baseline_duration as f64) / baseline_duration as f64) * 100.0;
                    let significance = if change_percent.abs() > 20.0 {
                        ChangeSignificance::Major
                    } else if change_percent.abs() > 10.0 {
                        ChangeSignificance::Moderate
                    } else {
                        ChangeSignificance::Minor
                    };
                    
                    comparison.diffs.push(ValidationDiff {
                        diff_type: DiffType::Modified,
                        field_name: "duration_ms".to_string(),
                        baseline_value: serde_json::Value::Number(baseline_duration.into()),
                        current_value: serde_json::Value::Number(current_duration.into()),
                        change_description: format!("Duration changed by {:.1}%", change_percent),
                        significance,
                    });
                }
            }
            _ => {}
        }
        
        Ok(())
    }
}

/// P2-Issue7: Semantic comparison strategy
pub struct SemanticComparisonStrategy;

impl SemanticComparisonStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonStrategy for SemanticComparisonStrategy {
    fn name(&self) -> String {
        "semantic".to_string()
    }
    
    async fn compare(
        &self,
        _baseline: &crate::harness::validation::ValidationResult,
        _current: &crate::harness::validation::ValidationResult,
        _comparison: &mut ValidationComparison,
    ) -> Result<()> {
        // Semantic comparison would analyze the meaning of changes
        // This is a placeholder implementation
        Ok(())
    }
}

/// P2-Issue7: Performance comparison strategy
pub struct PerformanceComparisonStrategy;

impl PerformanceComparisonStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonStrategy for PerformanceComparisonStrategy {
    fn name(&self) -> String {
        "performance".to_string()
    }
    
    async fn compare(
        &self,
        baseline: &crate::harness::validation::ValidationResult,
        current: &crate::harness::validation::ValidationResult,
        comparison: &mut ValidationComparison,
    ) -> Result<()> {
        // Compare execution time
        let execution_time = self.compare_metric(
            baseline.duration_ms.unwrap_or(0) as f64,
            current.duration_ms.unwrap_or(0) as f64,
        );
        comparison.performance_comparison.execution_time = execution_time;
        
        // Compare resource usage
        if let (Some(baseline_usage), Some(current_usage)) = (&baseline.resource_usage, &current.resource_usage) {
            comparison.performance_comparison.memory_usage = self.compare_metric(
                baseline_usage.memory_used_mb,
                current_usage.memory_used_mb,
            );
            
            comparison.performance_comparison.cpu_usage = self.compare_metric(
                baseline_usage.cpu_time_ms as f64,
                current_usage.cpu_time_ms as f64,
            );
        }
        
        Ok(())
    }
}

impl PerformanceComparisonStrategy {
    fn compare_metric(&self, baseline: f64, current: f64) -> MetricComparison {
        let absolute_change = current - baseline;
        let percentage_change = if baseline > 0.0 {
            (absolute_change / baseline) * 100.0
        } else {
            0.0
        };
        
        let significance = if percentage_change.abs() > 20.0 {
            StatisticalSignificance::HighlySignificant
        } else if percentage_change.abs() > 10.0 {
            StatisticalSignificance::Significant
        } else if percentage_change.abs() > 5.0 {
            StatisticalSignificance::MarginallySignificant
        } else {
            StatisticalSignificance::NotSignificant
        };
        
        MetricComparison {
            baseline,
            current,
            absolute_change,
            percentage_change,
            significance,
        }
    }
}

/// P2-Issue7: Issue comparison strategy
pub struct IssueComparisonStrategy;

impl IssueComparisonStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonStrategy for IssueComparisonStrategy {
    fn name(&self) -> String {
        "issue".to_string()
    }
    
    async fn compare(
        &self,
        baseline: &crate::harness::validation::ValidationResult,
        current: &crate::harness::validation::ValidationResult,
        comparison: &mut ValidationComparison,
    ) -> Result<()> {
        // Extract issues from validation artifacts
        let baseline_issues = self.extract_issues(baseline);
        let current_issues = self.extract_issues(current);
        
        // Compare issues
        let (new_issues, resolved_issues, persisting_issues) = self.compare_issue_lists(
            &baseline_issues,
            &current_issues,
        );
        
        comparison.issue_comparison = IssueComparison {
            new_issues,
            resolved_issues,
            persisting_issues,
            severity_changes: Vec::new(),
            category_changes: Vec::new(),
        };
        
        Ok(())
    }
}

impl IssueComparisonStrategy {
    fn extract_issues(&self, result: &crate::harness::validation::ValidationResult) -> Vec<crate::harness::validation_artifacts::ValidationIssue> {
        // In a real implementation, this would extract issues from validation artifacts
        // For now, return empty list
        Vec::new()
    }
    
    fn compare_issue_lists(
        &self,
        baseline: &[crate::harness::validation_artifacts::ValidationIssue],
        current: &[crate::harness::validation_artifacts::ValidationIssue],
    ) -> (
        Vec<crate::harness::validation_artifacts::ValidationIssue>,
        Vec<crate::harness::validation_artifacts::ValidationIssue>,
        Vec<PersistingIssue>,
    ) {
        let mut new_issues = Vec::new();
        let mut resolved_issues = Vec::new();
        let mut persisting_issues = Vec::new();
        
        // Simple comparison based on issue messages
        let baseline_messages: std::collections::HashSet<_> = baseline.iter().map(|i| &i.message).collect();
        let current_messages: std::collections::HashSet<_> = current.iter().map(|i| &i.message).collect();
        
        for issue in current {
            if !baseline_messages.contains(&issue.message) {
                new_issues.push(issue.clone());
            }
        }
        
        for issue in baseline {
            if !current_messages.contains(&issue.message) {
                resolved_issues.push(issue.clone());
            } else {
                persisting_issues.push(PersistingIssue {
                    id: issue.id.clone(),
                    issue: issue.clone(),
                    persistence_duration: Duration::from_secs(3600), // Placeholder
                    first_occurrence: chrono::Utc::now(), // Placeholder
                });
            }
        }
        
        (new_issues, resolved_issues, persisting_issues)
    }
}

/// P2-Issue7: Resource usage comparison strategy
pub struct ResourceUsageComparisonStrategy;

impl ResourceUsageComparisonStrategy {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonStrategy for ResourceUsageComparisonStrategy {
    fn name(&self) -> String {
        "resource_usage".to_string()
    }
    
    async fn compare(
        &self,
        baseline: &crate::harness::validation::ValidationResult,
        current: &crate::harness::validation::ValidationResult,
        comparison: &mut ValidationComparison,
    ) -> Result<()> {
        // Compare resource usage
        if let (Some(baseline_usage), Some(current_usage)) = (&baseline.resource_usage, &current.resource_usage) {
            comparison.resource_comparison.memory = MetricComparison {
                baseline: baseline_usage.memory_used_mb,
                current: current_usage.memory_used_mb,
                absolute_change: current_usage.memory_used_mb - baseline_usage.memory_used_mb,
                percentage_change: ((current_usage.memory_used_mb - baseline_usage.memory_used_mb) / baseline_usage.memory_used_mb) * 100.0,
                significance: StatisticalSignificance::NotSignificant,
            };
            
            comparison.resource_comparison.cpu_time = MetricComparison {
                baseline: baseline_usage.cpu_time_ms as f64,
                current: current_usage.cpu_time_ms as f64,
                absolute_change: (current_usage.cpu_time_ms - baseline_usage.cpu_time_ms) as f64,
                percentage_change: ((current_usage.cpu_time_ms - baseline_usage.cpu_time_ms) as f64 / baseline_usage.cpu_time_ms as f64) * 100.0,
                significance: StatisticalSignificance::NotSignificant,
            };
        }
        
        Ok(())
    }
}

/// P2-Issue7: Comparison analyzer trait
pub trait ComparisonAnalyzer: Send + Sync {
    /// Analyzer name
    fn name(&self) -> String;
    
    /// Analyze comparison
    async fn analyze(&self, comparison: &mut ValidationComparison) -> Result<()>;
}

/// P2-Issue7: Regression analyzer
pub struct RegressionAnalyzer;

impl RegressionAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonAnalyzer for RegressionAnalyzer {
    fn name(&self) -> String {
        "regression".to_string()
    }
    
    async fn analyze(&self, comparison: &mut ValidationComparison) -> Result<()> {
        let mut regressions = Vec::new();
        
        // Check for performance regressions
        if matches!(comparison.summary.performance_change.assessment, PerformanceAssessment::Regressed) {
            regressions.push(Regression {
                id: format!("perf_regress_{}", chrono::Utc::now().timestamp_nanos()),
                regression_type: RegressionType::Performance,
                description: "Performance regression detected".to_string(),
                severity: RegressionSeverity::Medium,
                affected_components: vec!["validation".to_string()],
                estimated_impact: RegressionImpact::Moderate,
            });
        }
        
        // Check for functional regressions
        if matches!(comparison.summary.status_change, StatusChange::Regressed) {
            regressions.push(Regression {
                id: format!("func_regress_{}", chrono::Utc::now().timestamp_nanos()),
                regression_type: RegressionType::Functional,
                description: "Functional regression detected".to_string(),
                severity: RegressionSeverity::High,
                affected_components: vec!["validation".to_string()],
                estimated_impact: RegressionImpact::Major,
            });
        }
        
        comparison.analysis.regression_analysis = RegressionAnalysis {
            regressions,
            severity_distribution: HashMap::new(),
            impact_score: 0.0,
            root_causes: Vec::new(),
        };
        
        Ok(())
    }
}

/// P2-Issue7: Improvement analyzer
pub struct ImprovementAnalyzer;

impl ImprovementAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonAnalyzer for ImprovementAnalyzer {
    fn name(&self) -> String {
        "improvement".to_string()
    }
    
    async fn analyze(&self, comparison: &mut ValidationComparison) -> Result<()> {
        let mut improvements = Vec::new();
        
        // Check for performance improvements
        if matches!(comparison.summary.performance_change.assessment, PerformanceAssessment::Improved) {
            improvements.push(Improvement {
                id: format!("perf_improve_{}", chrono::Utc::now().timestamp_nanos()),
                improvement_type: ImprovementType::Performance,
                description: "Performance improvement detected".to_string(),
                magnitude: ImprovementMagnitude::Moderate,
                affected_components: vec!["validation".to_string()],
                confidence: 0.8,
            });
        }
        
        comparison.analysis.improvement_analysis = ImprovementAnalysis {
            improvements,
            category_distribution: HashMap::new(),
            impact_score: 0.0,
            confidence: 0.8,
        };
        
        Ok(())
    }
}

/// P2-Issue7: Trend analyzer
pub struct TrendAnalyzer;

impl TrendAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonAnalyzer for TrendAnalyzer {
    fn name(&self) -> String {
        "trend".to_string()
    }
    
    async fn analyze(&self, comparison: &mut ValidationComparison) -> Result<()> {
        // Simple trend analysis based on single comparison
        let trend_direction = if matches!(comparison.summary.performance_change.assessment, PerformanceAssessment::Improved) {
            TrendDirection::Improving
        } else if matches!(comparison.summary.performance_change.assessment, PerformanceAssessment::Regressed) {
            TrendDirection::Degrading
        } else {
            TrendDirection::Stable
        };
        
        comparison.analysis.trend_analysis = TrendAnalysis {
            performance_trends: vec![Trend {
                metric_name: "execution_time".to_string(),
                direction: trend_direction,
                strength: 0.5,
                duration: Duration::from_secs(1),
                data_points: vec![],
            }],
            quality_trends: Vec::new(),
            resource_trends: Vec::new(),
            overall_direction: trend_direction,
            confidence: 0.5,
        };
        
        Ok(())
    }
}

/// P2-Issue7: Comparison reporter trait
pub trait ComparisonReporter: Send + Sync {
    /// Reporter name
    fn name(&self) -> String;
    
    /// Check if format is supported
    fn supports_format(&self, format: ReportFormat) -> bool;
    
    /// Generate report
    async fn generate_report(
        &self,
        comparison: &ValidationComparison,
        format: ReportFormat,
    ) -> Result<String>;
}

/// P2-Issue7: JSON comparison reporter
pub struct JsonComparisonReporter;

impl JsonComparisonReporter {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonReporter for JsonComparisonReporter {
    fn name(&self) -> String {
        "json".to_string()
    }
    
    fn supports_format(&self, format: ReportFormat) -> bool {
        matches!(format, ReportFormat::Json)
    }
    
    async fn generate_report(
        &self,
        comparison: &ValidationComparison,
        _format: ReportFormat,
    ) -> Result<String> {
        Ok(serde_json::to_string_pretty(comparison)?)
    }
}

/// P2-Issue7: HTML comparison reporter
pub struct HtmlComparisonReporter;

impl HtmlComparisonReporter {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonReporter for HtmlComparisonReporter {
    fn name(&self) -> String {
        "html".to_string()
    }
    
    fn supports_format(&self, format: ReportFormat) -> bool {
        matches!(format, ReportFormat::Html)
    }
    
    async fn generate_report(
        &self,
        comparison: &ValidationComparison,
        _format: ReportFormat,
    ) -> Result<String> {
        let html = format!(
            r#"
<!DOCTYPE html>
<html>
<head>
    <title>Validation Comparison Report</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        .header {{ background-color: #f0f0f0; padding: 10px; border-radius: 5px; }}
        .summary {{ margin: 20px 0; }}
        .diff {{ margin: 10px 0; padding: 10px; border-left: 3px solid #ccc; }}
        .improved {{ border-left-color: green; }}
        .regressed {{ border-left-color: red; }}
    </style>
</head>
<body>
    <div class="header">
        <h1>Validation Comparison Report</h1>
        <p>Generated: {}</p>
        <p>Comparison ID: {}</p>
    </div>
    
    <div class="summary">
        <h2>Summary</h2>
        <p>Status Change: {:?}</p>
        <p>Overall Assessment: {:?}</p>
    </div>
    
    <div class="details">
        <h2>Details</h2>
        <p>Performance Change: {:.1}%</p>
        <p>Issue Count Change: {}</p>
    </div>
</body>
</html>
            "#,
            comparison.timestamp,
            comparison.id,
            comparison.summary.status_change,
            comparison.summary.overall_assessment,
            comparison.summary.performance_change.execution_time_change_percent,
            comparison.summary.issue_count_change.total_change
        );
        
        Ok(html)
    }
}

/// P2-Issue7: Markdown comparison reporter
pub struct MarkdownComparisonReporter;

impl MarkdownComparisonReporter {
    pub fn new() -> Self {
        Self
    }
}

impl ComparisonReporter for MarkdownComparisonReporter {
    fn name(&self) -> String {
        "markdown".to_string()
    }
    
    fn supports_format(&self, format: ReportFormat) -> bool {
        matches!(format, ReportFormat::Markdown)
    }
    
    async fn generate_report(
        &self,
        comparison: &ValidationComparison,
        _format: ReportFormat,
    ) -> Result<String> {
        let markdown = format!(
            r#"# Validation Comparison Report

**Generated:** {}  
**Comparison ID:** {}

## Summary

- **Status Change:** {:?}
- **Overall Assessment:** {:?}

## Performance

- **Execution Time Change:** {:.1}%
- **Memory Change:** {:.1}%
- **CPU Change:** {:.1}%

## Issues

- **Total Change:** {}
- **New Issues:** {}
- **Resolved Issues:** {}

## Analysis

- **Regressions:** {}
- **Improvements:** {}
"#,
            comparison.timestamp,
            comparison.id,
            comparison.summary.status_change,
            comparison.summary.overall_assessment,
            comparison.summary.performance_change.execution_time_change_percent,
            comparison.summary.performance_change.memory_change_percent,
            comparison.summary.performance_change.cpu_change_percent,
            comparison.summary.issue_count_change.total_change,
            comparison.issue_comparison.new_issues.len(),
            comparison.issue_comparison.resolved_issues.len(),
            comparison.analysis.regression_analysis.regressions.len(),
            comparison.analysis.improvement_analysis.improvements.len()
        );
        
        Ok(markdown)
    }
}

impl Default for ComparisonSummary {
    fn default() -> Self {
        Self {
            status_change: StatusChange::NoChange,
            performance_change: PerformanceChange {
                execution_time_change_percent: 0.0,
                memory_change_percent: 0.0,
                cpu_change_percent: 0.0,
                assessment: PerformanceAssessment::NoSignificantChange,
            },
            issue_count_change: IssueCountChange {
                total_change: 0,
                critical_change: 0,
                error_change: 0,
                warning_change: 0,
                info_change: 0,
            },
            resource_change: ResourceChange {
                memory_change_mb: 0.0,
                cpu_time_change_sec: 0.0,
                disk_io_change_mb: 0.0,
                network_io_change_mb: 0.0,
                file_handles_change: 0,
            },
            overall_assessment: ComparisonAssessment::NoSignificantChanges,
        }
    }
}

impl Default for PerformanceComparison {
    fn default() -> Self {
        Self {
            execution_time: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            memory_usage: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            cpu_usage: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            disk_io: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            network_io: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            overall_score: 0.0,
        }
    }
}

impl Default for IssueComparison {
    fn default() -> Self {
        Self {
            new_issues: Vec::new(),
            resolved_issues: Vec::new(),
            persisting_issues: Vec::new(),
            severity_changes: Vec::new(),
            category_changes: Vec::new(),
        }
    }
}

impl Default for ResourceUsageComparison {
    fn default() -> Self {
        Self {
            memory: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            cpu_time: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            disk_io: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            network_io: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            file_handles: MetricComparison {
                baseline: 0.0,
                current: 0.0,
                absolute_change: 0.0,
                percentage_change: 0.0,
                significance: StatisticalSignificance::NotSignificant,
            },
            efficiency_score: 0.0,
        }
    }
}

impl Default for ComparisonAnalysis {
    fn default() -> Self {
        Self {
            regression_analysis: RegressionAnalysis {
                regressions: Vec::new(),
                severity_distribution: HashMap::new(),
                impact_score: 0.0,
                root_causes: Vec::new(),
            },
            improvement_analysis: ImprovementAnalysis {
                improvements: Vec::new(),
                category_distribution: HashMap::new(),
                impact_score: 0.0,
                confidence: 0.0,
            },
            trend_analysis: TrendAnalysis {
                performance_trends: Vec::new(),
                quality_trends: Vec::new(),
                resource_trends: Vec::new(),
                overall_direction: TrendDirection::Stable,
                confidence: 0.0,
            },
            recommendations: Vec::new(),
            risk_assessment: RiskAssessment {
                overall_risk: RiskLevel::Low,
                risk_factors: Vec::new(),
                mitigation_strategies: Vec::new(),
                risk_score: 0.0,
            },
        }
    }
}
