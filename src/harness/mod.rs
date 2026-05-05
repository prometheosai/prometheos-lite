pub mod acceptance;
pub mod adversarial_validation;
pub mod artifacts;
pub mod attempt_pool;
pub mod benchmark;
pub mod completion;
pub mod confidence;
pub mod edit_protocol;
pub mod environment;
pub mod evidence;
pub mod execution_loop;
pub mod failure;
pub mod file_control;
pub mod git_checkpoint;
pub mod golden_paths;
pub mod knowledge_cache;
pub mod minimality;
pub mod mode_policy;
pub mod model_strategy;
pub mod observability;
pub mod patch_applier;
pub mod patch_provider;
pub mod permissions;
pub mod regression_memory;
pub mod repair_loop;
pub mod repo_intelligence;
pub mod report;
pub mod reproduction;
pub mod review;
pub mod risk;
pub mod runtime_tools;
pub mod sandbox;
pub mod scaling;
pub mod selection;
pub mod semantic_diff;
pub mod temp_workspace;
pub mod time_travel;
pub mod trajectory;
pub mod validation;
pub mod verification;
pub mod workspace;
pub mod work_integration;

pub use acceptance::{
    AcceptanceCompiler, AcceptanceCriterion, CompiledAcceptanceCriteria, CriterionPriority,
    CriterionStatus, VerificationMethod, compile_acceptance_criteria,
    compile_acceptance_criteria_with_env, get_verification_summary, update_criterion_status,
};
pub use adversarial_validation::{
    AdversarialTestSuite, EdgeCaseTest, EdgeCaseType, Property, PropertyTest, TestPriority,
    generate_adversarial_tests,
};
pub use artifacts::{
    ArtifactGenerator, ArtifactKind, ArtifactMetadata, CompressionType, HarnessArtifact,
    format_artifact_summary, generate_completion_artifact,
};
pub use benchmark::{
    AntiOverfittingReport, BenchmarkConfig, BenchmarkResult, BenchmarkRunner, BenchmarkSuite,
    BenchmarkTest, ComparisonResult, MetricResult, MetricType, TestType, create_benchmark_runner,
    format_anti_overfitting_report, format_benchmark_result, format_comparison,
};
pub use completion::{
    CompletionDecision, CompletionEvaluator, CompletionEvidence, ConfidenceEvidence, PatchEvidence,
    ProcessEvidence, ReviewEvidence, RiskEvidence, SemanticEvidence, ValidationEvidence,
    VerificationEvidence, create_evidence_from_components, evaluate_completion,
    format_completion_decision,
};
pub use confidence::{
    ConfidenceCalibrator, ConfidenceClassification, ConfidenceFactor, ConfidenceScore,
    ConfidenceWeights, FactorImpact, compute_confidence,
};
pub use edit_protocol::*;
pub use environment::*;
pub use execution_loop::*;
pub use failure::{
    FailureCategory, FailureContext, FailureDetails, FailureKind, analyze_failure_pattern,
    classify_command_failure, classify_patch_failure, classify_validation_failure,
    create_failure_details, format_failure_report,
};
pub use file_control::*;
pub use git_checkpoint::{
    GitCheckpoint, GitCheckpointManager, RepoInfo, RollbackStrategy, get_repo_info, is_git_repo,
};
pub use golden_paths::{
    GoldenPath, GoldenPathRegistry, PathCategory, PathComplexity, PathMatch, PathStats, PathStep,
    StepType, create_golden_path_registry, format_path_match,
};
pub use knowledge_cache::{
    CacheKey, CacheScope, CacheValue, KnowledgeCacheManager, TaskLocalKnowledgeCache,
};
pub use minimality::{
    ChangeType, FileChange, MinimalityConfig, MinimalityEnforcer, MinimalityStats,
    MinimalityViolation, PatchAnalysis, UnrelatedChange, ViolationSeverity,
    analyze_patch_minimality, enforce_patch_minimality, format_analysis_report,
};
pub use model_strategy::{
    ComplexityLevel, ModelProfile, ModelRecommendation, ModelStrategyEngine, ModelTier,
    TaskRequirements, UrgencyLevel, format_recommendation, select_model_for_task,
};
pub use observability::{
    ExecutionStatus, HarnessMetrics, ObservabilityCollector, ObservabilitySummary, OperationSpan,
    SpanEvent, SpanStatus, create_collector, format_metrics_report,
};
pub use patch_applier::*;
pub use patch_provider::{
    AggregatePatchProvider, AttemptOutcome, AttemptRecord, GenerateRequest, GenerateResponse,
    HeuristicPatchProvider, LlmPatchProvider, PatchProvider, PatchProviderContext,
    ProviderCandidate, ProviderCapabilities, RepairResponse, RiskEstimate,
};
pub use permissions::{
    Permission, PermissionCheck, PermissionGrant, PermissionLedger, PermissionScope,
    PermissionStats, create_permissive_ledger, create_restrictive_ledger, create_standard_ledger,
};
pub use regression_memory::{
    FailurePattern, FailureType, LearningResult, MemoryStats, PatternMatch, RegressionMemory,
    SuccessfulSolution, create_regression_memory, format_memory_stats,
};
pub use repair_loop::{
    RepairAttempt, RepairLoop, RepairRequest, RepairResult, RepairStrategy, format_repair_report,
    run_repair_loop,
};
pub use repo_intelligence::*;
pub use reproduction::{
    FixType, ReproductionMode, ReproductionRequest, ReproductionResult, generate_reproduction_test,
};
pub use review::{
    ReviewEngine, ReviewIssue, ReviewIssueType, ReviewReport, ReviewSeverity, format_review_report,
    generate_review_report, has_critical_issues, review_diff, review_file,
};
pub use risk::{
    OverridePolicy, OverrideResult, RiskAssessment, RiskCategory, RiskEngine, RiskLevel,
    RiskReason, assess_risk, format_risk_assessment,
};
pub use runtime_tools::{
    IssueSeverity, RuntimeTool, RuntimeToolRegistry, ToolExecution, ToolIssue, ToolResult,
    ToolStats, ToolType, create_tool_registry, format_tool_result,
};
pub use sandbox::*;
pub use scaling::{
    AttemptPriority, AttemptResult, ResourceAllocation, ResourceLimits, ScalingConfig,
    ScalingDecision, ScalingEngine, create_scaling_engine, format_scaling_report,
};
pub use selection::{
    PatchCandidate, ScoredCandidate, SelectionCriteria, SelectionEngine, rank_patches,
    select_best_patch,
};
pub use semantic_diff::{
    ApiChange, AuthChange, DatabaseChange, DependencyChange, RiskLevel as SemanticRiskLevel,
    SemanticDiff, analyze_semantic_diff, has_breaking_changes, requires_approval,
};
pub use time_travel::{
    Breakpoint, DebugState, DiffView, FileState, SessionStats, TimePoint, TimeTravelDebugger,
    TimeTravelSession, VariableState, create_time_travel_debugger, format_debug_state,
    format_diff_view,
};
pub use validation::*;
pub use verification::{
    VerificationAssessment, VerificationLevel, VerificationStrength, assess_verification_strength,
    format_verification_assessment,
};
pub use work_integration::HarnessWorkContextService;
