//! CLI for the approval-controlled patch workflow.
//!
//! Drives `prometheos_lite::workflow` end to end:
//! propose -> dry-run -> approve -> apply -> report.
//!
//! `generate` is the #78 slice: it routes a `PatchProvider` through the same
//! governed path, so generated patches are treated as hostile input and pass
//! through every existing gate.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use prometheos_lite::config::AppConfig;
use prometheos_lite::harness::patch_provider::{
    MockProposalMode, MockProposalProvider, PatchProvider, PatchProviderContext, ProviderRegistry,
};
use prometheos_lite::workflow::evaluate::{self, EvaluationConfig, TaskManifest};
use prometheos_lite::workflow::{
    self, AuthorityLevel, GenerateScope, ProviderRouteInfo, sanitize_provider_route,
};

/// Map a string (e.g. from `PROMETHEOS_MOCK_MODE`) to a mock provider mode.
fn provider_mode_from_str(s: &str) -> MockProposalMode {
    match s.to_ascii_lowercase().as_str() {
        "outofscope" | "out_of_scope" => MockProposalMode::OutOfScope,
        "forbidden" => MockProposalMode::Forbidden,
        "dependency" => MockProposalMode::Dependency,
        "absolute" => MockProposalMode::Absolute,
        "traversal" => MockProposalMode::Traversal,
        "malformed" => MockProposalMode::Malformed,
        "empty" => MockProposalMode::Empty,
        "failing" => MockProposalMode::Failing,
        _ => MockProposalMode::Safe,
    }
}

#[derive(Debug, Parser)]
pub struct WorkflowCommand {
    #[command(subcommand)]
    command: WorkflowSubcommand,
}

#[derive(Debug, Subcommand)]
enum WorkflowSubcommand {
    /// Analyze the repo, lock scope, and record a proposed patch artifact.
    Propose {
        /// Repository root to operate on.
        #[arg(long)]
        repo: PathBuf,
        /// Goal description.
        #[arg(short, long)]
        goal: String,
        /// Authority level: review | propose | assist | execute.
        #[arg(long, default_value = "propose")]
        authority: String,
        /// Path to a unified-diff patch file to propose.
        #[arg(long)]
        patch: PathBuf,
        /// Allowed repo-relative path prefixes (repeatable).
        #[arg(long = "allowed")]
        allowed: Vec<String>,
        /// Forbidden repo-relative path prefixes (repeatable).
        #[arg(long = "forbidden")]
        forbidden: Vec<String>,
        /// Allow dependency-manifest changes (Cargo.toml, package.json, ...).
        #[arg(long)]
        allow_deps: bool,
        /// Maximum changed files before blocking.
        #[arg(long)]
        max_files: Option<usize>,
        /// Maximum changed lines before blocking.
        #[arg(long)]
        max_lines: Option<usize>,
    },
    /// Validate the proposal in an isolated Git worktree.
    DryRun {
        /// Repository root.
        #[arg(long)]
        repo: PathBuf,
        /// Workflow id returned by `propose`.
        id: String,
        /// Optional validation command (run inside the worktree via `sh -c`).
        #[arg(long)]
        validate: Option<String>,
    },
    /// Record explicit approval for the proposal's patch hash.
    Approve {
        /// Repository root.
        #[arg(long)]
        repo: PathBuf,
        /// Workflow id.
        id: String,
        /// Patch hash to approve (must match the proposal).
        #[arg(long = "patch-hash")]
        patch_hash: String,
        /// Approver identity.
        #[arg(long, default_value = "operator")]
        approver: String,
    },
    /// Apply the approved patch to the user's tree after checkpoint + scope re-check.
    Apply {
        /// Repository root.
        #[arg(long)]
        repo: PathBuf,
        /// Workflow id.
        id: String,
        /// Patch hash (must match approval + proposal).
        #[arg(long = "patch-hash")]
        patch_hash: String,
        /// Optional validation command (run in the repo via `sh -c`).
        #[arg(long)]
        validate: Option<String>,
        /// Disable automatic rollback on validation failure.
        #[arg(long)]
        no_rollback: bool,
    },
    /// Generate a governed proposal through a PatchProvider (no patch file needed).
    ///
    /// The configured provider (or `--provider mock` for offline/deterministic runs)
    /// produces candidate edits; those are rendered to a unified diff, treated as
    /// hostile input, and routed through the same propose -> dry-run -> approve ->
    /// apply -> report path as `propose`.
    Generate {
        /// Repository root to operate on.
        #[arg(long)]
        repo: PathBuf,
        /// Goal description.
        #[arg(short, long)]
        goal: String,
        /// Authority level: review | propose | assist | execute.
        #[arg(long, default_value = "assist")]
        authority: String,
        /// Allowed repo-relative path prefixes (repeatable).
        #[arg(long = "allowed")]
        allowed: Vec<String>,
        /// Forbidden repo-relative path prefixes (repeatable).
        #[arg(long = "forbidden")]
        forbidden: Vec<String>,
        /// Allow dependency-manifest changes (Cargo.toml, package.json, ...).
        #[arg(long)]
        allow_deps: bool,
        /// Maximum changed files before blocking.
        #[arg(long)]
        max_files: Option<usize>,
        /// Maximum changed lines before blocking.
        #[arg(long)]
        max_lines: Option<usize>,
        /// Validation command recorded with the proposal (run at dry-run/apply).
        #[arg(long)]
        validate: Option<String>,
        /// Provider source: `config` (default) uses the configured provider, or
        /// `mock` for the deterministic offline provider.
        #[arg(long, default_value = "config")]
        provider: String,
    },
    /// Print a JSON report for a workflow id.
    Report {
        /// Repository root.
        #[arg(long)]
        repo: PathBuf,
        /// Workflow id.
        id: String,
    },
    /// Fast Governed Loop V1: automated evaluation pipeline.
    ///
    /// Takes a task from definition through REVIEW_GATE, producing a trustworthy
    /// evidence bundle. The human still makes the final correctness decision.
    ///
    /// Accepts either a path to a JSON manifest file (--manifest) or inline
    /// arguments (--repo, --goal, etc.).
    Evaluate {
        /// Path to a JSON task manifest file.
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Repository root to operate on (used with inline args).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Goal description (used with inline args).
        #[arg(short, long)]
        goal: Option<String>,
        /// Task ID (used with inline args; auto-generated if omitted).
        #[arg(long)]
        task_id: Option<String>,
        /// Authority level: review | propose | assist | execute.
        #[arg(long, default_value = "propose")]
        authority: String,
        /// Allowed repo-relative path prefixes (repeatable).
        #[arg(long = "allowed")]
        allowed: Vec<String>,
        /// Forbidden repo-relative path prefixes (repeatable).
        #[arg(long = "forbidden")]
        forbidden: Vec<String>,
        /// Allow dependency-manifest changes.
        #[arg(long)]
        allow_deps: bool,
        /// Maximum changed files before blocking.
        #[arg(long)]
        max_files: Option<usize>,
        /// Maximum changed lines before blocking.
        #[arg(long)]
        max_lines: Option<usize>,
        /// Validation command (run in the isolated worktree).
        #[arg(long)]
        validate: Option<String>,
        /// Provider source: `config` or `mock`.
        #[arg(long, default_value = "mock")]
        provider: String,
        /// Minimum free disk space in bytes.
        #[arg(long, default_value = "104857600")]
        min_disk_bytes: u64,
        /// Output the JSON evidence bundle to stdout.
        #[arg(long)]
        json: bool,
    },
}

impl WorkflowCommand {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            WorkflowSubcommand::Propose {
                repo,
                goal,
                authority,
                patch,
                allowed,
                forbidden,
                allow_deps,
                max_files,
                max_lines,
            } => {
                let authority: AuthorityLevel = authority.parse()?;
                let patch_text = std::fs::read_to_string(&patch)
                    .with_context(|| format!("cannot read patch file {}", patch.display()))?;
                let id = workflow::propose(
                    &repo,
                    &goal,
                    authority,
                    &patch_text,
                    &allowed,
                    &forbidden,
                    allow_deps,
                    max_files,
                    max_lines,
                )?;
                println!("{id}");
                Ok(())
            }
            WorkflowSubcommand::DryRun { repo, id, validate } => {
                let passed = workflow::dry_run(&repo, &id, validate.as_deref())?;
                println!(
                    "dry-run {} for {}",
                    if passed { "PASSED" } else { "FAILED" },
                    id
                );
                Ok(())
            }
            WorkflowSubcommand::Approve {
                repo,
                id,
                patch_hash,
                approver,
            } => {
                workflow::approve(&repo, &id, &patch_hash, &approver)?;
                println!("approved {id}");
                Ok(())
            }
            WorkflowSubcommand::Apply {
                repo,
                id,
                patch_hash,
                validate,
                no_rollback,
            } => {
                workflow::apply(&repo, &id, &patch_hash, validate.as_deref(), !no_rollback)?;
                println!("applied {id}");
                Ok(())
            }
            WorkflowSubcommand::Report { repo, id } => {
                let report = workflow::report(&repo, &id)?;
                println!("{report}");
                Ok(())
            }
            WorkflowSubcommand::Evaluate {
                manifest,
                repo,
                goal,
                task_id,
                authority,
                allowed,
                forbidden,
                allow_deps,
                max_files,
                max_lines,
                validate,
                provider,
                min_disk_bytes,
                json,
            } => {
                let task_manifest = if let Some(manifest_path) = manifest {
                    let text = std::fs::read_to_string(&manifest_path).with_context(|| {
                        format!("cannot read manifest {}", manifest_path.display())
                    })?;
                    serde_json::from_str::<TaskManifest>(&text)
                        .context("failed to parse task manifest")?
                } else {
                    let repo_path = repo.ok_or_else(|| {
                        anyhow::anyhow!("--repo is required when --manifest is not provided")
                    })?;
                    let goal_str = goal.ok_or_else(|| {
                        anyhow::anyhow!("--goal is required when --manifest is not provided")
                    })?;
                    TaskManifest {
                        task_id: task_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                        goal: goal_str,
                        repo: repo_path,
                        allowed_paths: allowed,
                        forbidden_paths: forbidden,
                        allow_dependency_changes: allow_deps,
                        max_files_changed: max_files,
                        max_lines_changed: max_lines,
                        validation_command: validate,
                        provider: provider.clone(),
                        authority: authority.clone(),
                        min_disk_bytes,
                        evidence_dir: None,
                    }
                };

                // Select the provider.
                let (boxed, route_info): (
                    Box<dyn prometheos_lite::harness::patch_provider::PatchProvider>,
                    Option<ProviderRouteInfo>,
                ) = if task_manifest.provider == "mock" {
                    let mode = std::env::var("PROMETHEOS_MOCK_MODE")
                        .map(|s| provider_mode_from_str(&s))
                        .unwrap_or(MockProposalMode::Safe);
                    (Box::new(MockProposalProvider::with_mode(mode)), None)
                } else {
                    let config = AppConfig::load().context(
                        "failed to load provider configuration; set PROMETHEOS_PROVIDER/MODEL/BASE_URL or use --provider mock",
                    )?;
                    let registry =
                        prometheos_lite::harness::patch_provider::ProviderRegistry::from_config(
                            &config,
                        )?;
                    let route = ProviderRouteInfo {
                        model: Some(config.model.clone()),
                        route: sanitize_provider_route(&config.base_url),
                    };
                    (Box::new(registry), Some(route))
                };

                let eval_config = EvaluationConfig {
                    manifest: task_manifest,
                    provider: boxed,
                    route_info,
                };

                let bundle = evaluate::evaluate(eval_config).await?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&bundle)
                            .context("failed to serialize evidence bundle")?
                    );
                } else {
                    println!("Result: {}", bundle.final_state);
                    if let Some(ref fc) = bundle.failure_classification {
                        println!("Classification: {fc}");
                    }
                    if let Some(ref proposal) = bundle.proposal {
                        println!("Generation: completed exactly once");
                        println!("Proposal: {}", proposal.id);
                    } else {
                        println!("Generation: failed");
                    }
                    if let Some(ref validation) = bundle.validation {
                        println!(
                            "Test discovered: {}",
                            if validation.test_discovered {
                                "yes"
                            } else {
                                "no"
                            }
                        );
                        println!(
                            "Test executed: {}",
                            if validation.test_executed {
                                "yes"
                            } else {
                                "no"
                            }
                        );
                        println!(
                            "Validation: {}",
                            if validation.validation_passed {
                                "passed"
                            } else {
                                "failed"
                            }
                        );
                    }
                    if let Some(ref integrity) = bundle.integrity {
                        println!(
                            "Original repository: {}",
                            if integrity.original_commit_unchanged
                                && integrity.no_tracked_modifications
                            {
                                "unchanged"
                            } else {
                                "MODIFIED"
                            }
                        );
                    }
                    // Show evidence bundle path.
                    let evidence_path = PathBuf::from(&bundle.repo)
                        .join(".prometheos")
                        .join("evidence")
                        .join(&bundle.run_id);
                    println!("Evidence bundle: {}", evidence_path.display());
                }
                Ok(())
            }
            WorkflowSubcommand::Generate {
                repo,
                goal,
                authority,
                allowed,
                forbidden,
                allow_deps,
                max_files,
                max_lines,
                validate,
                provider,
            } => {
                let authority: AuthorityLevel = authority.parse()?;
                let scope = GenerateScope {
                    allowed_paths: allowed,
                    forbidden_paths: forbidden,
                    allow_dependency_changes: allow_deps,
                    max_files_changed: max_files,
                    max_lines_changed: max_lines,
                };
                let context = PatchProviderContext {
                    task: goal.clone(),
                    ..Default::default()
                };

                // Select the provider through the *existing* provider abstraction.
                // No model is invoked directly here.
                let (boxed, route_info): (Box<dyn PatchProvider>, Option<ProviderRouteInfo>) =
                    if provider == "mock" {
                        let mode = std::env::var("PROMETHEOS_MOCK_MODE")
                            .map(|s| provider_mode_from_str(&s))
                            .unwrap_or(MockProposalMode::Safe);
                        (Box::new(MockProposalProvider::with_mode(mode)), None)
                    } else {
                        let config = AppConfig::load().context(
                            "failed to load provider configuration; set PROMETHEOS_PROVIDER/MODEL/BASE_URL or use --provider mock",
                        )?;
                        let registry = ProviderRegistry::from_config(&config)?;
                        let route = ProviderRouteInfo {
                            model: Some(config.model.clone()),
                            route: sanitize_provider_route(&config.base_url),
                        };
                        (Box::new(registry), Some(route))
                    };

                let result = workflow::generate_proposal(
                    &repo,
                    &goal,
                    authority,
                    boxed.as_ref(),
                    context,
                    &scope,
                    route_info,
                    validate,
                )
                .await?;
                println!("{}", result.id);
                println!("patch_hash={}", result.patch_hash);
                Ok(())
            }
        }
    }
}
