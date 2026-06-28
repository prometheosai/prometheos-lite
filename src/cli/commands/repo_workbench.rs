//! Local repo workbench MVP command.
//!
//! This module intentionally keeps the first Repo Workbench path small and file-backed.
//! It does not mutate the target repository. Suggested patches are staged as artifacts and
//! require explicit approval before any future writer is allowed to apply them.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};
use uuid::Uuid;
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Parser)]
pub struct RepoWorkbenchCommand {
    #[command(subcommand)]
    command: RepoWorkbenchSubcommand,
}

#[derive(Debug, Subcommand)]
enum RepoWorkbenchSubcommand {
    /// Create a local repo WorkContext
    Create {
        /// Repository root to analyze
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Goal for the workbench run
        #[arg(long)]
        goal: String,
        /// Work mode. MVP supports review-first behavior.
        #[arg(long, default_value = "review")]
        mode: String,
        /// Optional title. Defaults to a short title derived from the goal.
        #[arg(long)]
        title: Option<String>,
    },
    /// Run the read-only risky-code review workflow
    Run {
        /// WorkContext ID
        id: String,
    },
    /// Show WorkContext status
    Status {
        /// WorkContext ID
        id: String,
    },
    /// List artifacts for a WorkContext
    Artifacts {
        /// WorkContext ID
        id: String,
    },
    /// Approve a staged artifact. This records approval only; it does not write repo files.
    Approve {
        /// Artifact ID
        artifact_id: String,
        /// Optional WorkContext ID. If omitted, the current repo store is searched.
        #[arg(long)]
        work_id: Option<String>,
    },
    /// Continue a WorkContext from saved memory
    Continue {
        /// WorkContext ID
        id: String,
    },
    /// Inspect persisted MVP memory
    Memory {
        #[command(subcommand)]
        command: MemorySubcommand,
    },
}

#[derive(Debug, Subcommand)]
enum MemorySubcommand {
    /// Show memory for a WorkContext
    Show {
        /// WorkContext ID
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkbenchContext {
    id: String,
    title: String,
    goal: String,
    mode: String,
    repo_path: PathBuf,
    status: String,
    phase: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    repo_summary: RepoSummary,
    artifacts: Vec<ArtifactRef>,
    decisions: Vec<DecisionRecord>,
    next_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RepoSummary {
    project_type: String,
    files_scanned: usize,
    candidate_files: Vec<FileSummary>,
    ignored_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileSummary {
    path: String,
    bytes: u64,
    lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactRef {
    id: String,
    kind: String,
    title: String,
    path: PathBuf,
    status: String,
    requires_approval: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecisionRecord {
    id: String,
    artifact_id: String,
    decision: String,
    approved: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct RiskFinding {
    id: String,
    file: String,
    line: usize,
    risk: &'static str,
    category: &'static str,
    summary: String,
    recommendation: String,
}

impl RepoWorkbenchCommand {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            RepoWorkbenchSubcommand::Create { repo, goal, mode, title } => {
                let repo_path = normalize_repo_path(&repo)?;
                let summary = scan_repo(&repo_path)?;
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let title = title.unwrap_or_else(|| title_from_goal(&goal));
                let mut context = WorkbenchContext {
                    id: id.clone(),
                    title,
                    goal,
                    mode,
                    repo_path: repo_path.clone(),
                    status: "draft".to_string(),
                    phase: "intake".to_string(),
                    created_at: now,
                    updated_at: now,
                    repo_summary: summary,
                    artifacts: Vec::new(),
                    decisions: Vec::new(),
                    next_action: Some("Run `prometheos repo run <work_id>` to perform read-only risk review.".to_string()),
                };

                save_context(&context)?;
                write_memory(&context, &[])?;

                println!("Created Repo Workbench WorkContext");
                println!("  ID: {}", context.id);
                println!("  Title: {}", context.title);
                println!("  Repo: {}", context.repo_path.display());
                println!("  Mode: {}", context.mode);
                println!("  Project type: {}", context.repo_summary.project_type);
                println!("  Candidate files: {}", context.repo_summary.candidate_files.len());
                println!("  Next: prometheos repo run {}", context.id);
            }
            RepoWorkbenchSubcommand::Run { id } => {
                let mut context = load_context_from_current_repo(&id)?;
                context.status = "in_progress".to_string();
                context.phase = "review".to_string();
                context.repo_summary = scan_repo(&context.repo_path)?;

                let findings = analyze_risks(&context.repo_path, &context.repo_summary)?;
                let report = render_risk_report(&context, &findings);
                let patch = render_patch_suggestions(&context, &findings);

                let report_artifact = write_artifact(
                    &context,
                    "risk-report",
                    "Risk Review Report",
                    "ready",
                    false,
                    &report,
                )?;
                let patch_artifact = write_artifact(
                    &context,
                    "suggested-patch",
                    "Suggested Patch Plan",
                    "awaiting_approval",
                    true,
                    &patch,
                )?;

                upsert_artifact(&mut context, report_artifact);
                upsert_artifact(&mut context, patch_artifact);
                context.status = "awaiting_approval".to_string();
                context.phase = "approval".to_string();
                context.next_action = context
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.requires_approval && artifact.status == "awaiting_approval")
                    .map(|artifact| format!("Review `{}` and approve with `prometheos repo approve {}`.", artifact.title, artifact.id));
                context.updated_at = Utc::now();

                save_context(&context)?;
                write_memory(&context, &findings)?;

                println!("Repo Workbench run complete");
                println!("  WorkContext: {}", context.id);
                println!("  Status: {}", context.status);
                println!("  Files considered: {}", context.repo_summary.candidate_files.len());
                println!("  Findings: {}", findings.len());
                println!("  Artifacts: {}", context.artifacts.len());
                if let Some(next) = &context.next_action {
                    println!("  Next: {}", next);
                }
            }
            RepoWorkbenchSubcommand::Status { id } => {
                let context = load_context_from_current_repo(&id)?;
                print_status(&context);
            }
            RepoWorkbenchSubcommand::Artifacts { id } => {
                let context = load_context_from_current_repo(&id)?;
                println!("Artifacts for {}:", context.id);
                if context.artifacts.is_empty() {
                    println!("  No artifacts yet. Run `prometheos repo run {}` first.", context.id);
                }
                for artifact in &context.artifacts {
                    println!("  {}", artifact.id);
                    println!("    Title: {}", artifact.title);
                    println!("    Kind: {}", artifact.kind);
                    println!("    Status: {}", artifact.status);
                    println!("    Requires approval: {}", artifact.requires_approval);
                    println!("    Path: {}", artifact.path.display());
                }
            }
            RepoWorkbenchSubcommand::Approve { artifact_id, work_id } => {
                let mut context = if let Some(work_id) = work_id {
                    load_context_from_current_repo(&work_id)?
                } else {
                    find_context_by_artifact(&artifact_id)?
                };

                let artifact = context
                    .artifacts
                    .iter_mut()
                    .find(|artifact| artifact.id == artifact_id)
                    .with_context(|| format!("Artifact `{}` not found", artifact_id))?;

                if !artifact.requires_approval {
                    println!("Artifact `{}` does not require approval; recording acknowledgement anyway.", artifact.id);
                }

                artifact.status = "approved".to_string();
                context.decisions.push(DecisionRecord {
                    id: Uuid::new_v4().to_string(),
                    artifact_id: artifact_id.clone(),
                    decision: "approved_for_future_application".to_string(),
                    approved: true,
                    created_at: Utc::now(),
                });
                context.status = "approved".to_string();
                context.phase = "ready_to_apply".to_string();
                context.next_action = Some(
                    "Approval recorded. MVP does not apply patches automatically; future writer must consume the approved artifact explicitly."
                        .to_string(),
                );
                context.updated_at = Utc::now();
                save_context(&context)?;
                write_memory(&context, &[])?;

                println!("Approval recorded");
                println!("  WorkContext: {}", context.id);
                println!("  Artifact: {}", artifact_id);
                println!("  Safety: no repository files were modified by this command");
            }
            RepoWorkbenchSubcommand::Continue { id } => {
                let context = load_context_from_current_repo(&id)?;
                println!("Continuing Repo Workbench WorkContext");
                print_status(&context);
                println!();
                println!("Memory:");
                println!("{}", load_memory(&context)?);
                if let Some(next) = &context.next_action {
                    println!();
                    println!("Recommended next action: {}", next);
                }
            }
            RepoWorkbenchSubcommand::Memory { command } => match command {
                MemorySubcommand::Show { id } => {
                    let context = load_context_from_current_repo(&id)?;
                    println!("{}", load_memory(&context)?);
                }
            },
        }

        Ok(())
    }
}

fn normalize_repo_path(repo: &Path) -> Result<PathBuf> {
    let path = if repo.is_absolute() {
        repo.to_path_buf()
    } else {
        std::env::current_dir()?.join(repo)
    };
    let path = path
        .canonicalize()
        .with_context(|| format!("Repository path does not exist: {}", path.display()))?;
    if !path.is_dir() {
        anyhow::bail!("Repository path is not a directory: {}", path.display());
    }
    Ok(path)
}

fn title_from_goal(goal: &str) -> String {
    let trimmed = goal.trim();
    if trimmed.len() <= 48 {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..48])
}

fn store_root(repo_path: &Path) -> PathBuf {
    repo_path.join(".prometheos-lite").join("workbench")
}

fn contexts_dir(repo_path: &Path) -> PathBuf {
    store_root(repo_path).join("contexts")
}

fn artifacts_dir(repo_path: &Path, work_id: &str) -> PathBuf {
    store_root(repo_path).join("artifacts").join(work_id)
}

fn memory_dir(repo_path: &Path) -> PathBuf {
    store_root(repo_path).join("memory")
}

fn context_path(context: &WorkbenchContext) -> PathBuf {
    contexts_dir(&context.repo_path).join(format!("{}.json", context.id))
}

fn load_context_from_current_repo(id: &str) -> Result<WorkbenchContext> {
    let repo_path = normalize_repo_path(Path::new("."))?;
    let path = contexts_dir(&repo_path).join(format!("{}.json", id));
    let json = fs::read_to_string(&path)
        .with_context(|| format!("WorkContext `{}` not found at {}", id, path.display()))?;
    let context = serde_json::from_str(&json)?;
    Ok(context)
}

fn find_context_by_artifact(artifact_id: &str) -> Result<WorkbenchContext> {
    let repo_path = normalize_repo_path(Path::new("."))?;
    let dir = contexts_dir(&repo_path);
    for entry in fs::read_dir(&dir).with_context(|| format!("Context directory not found: {}", dir.display()))? {
        let entry = entry?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let json = fs::read_to_string(entry.path())?;
        let context: WorkbenchContext = serde_json::from_str(&json)?;
        if context.artifacts.iter().any(|artifact| artifact.id == artifact_id) {
            return Ok(context);
        }
    }
    anyhow::bail!("Artifact `{}` not found in current repo workbench store", artifact_id)
}

fn save_context(context: &WorkbenchContext) -> Result<()> {
    fs::create_dir_all(contexts_dir(&context.repo_path))?;
    let json = serde_json::to_string_pretty(context)?;
    fs::write(context_path(context), json)?;
    Ok(())
}

fn scan_repo(repo_path: &Path) -> Result<RepoSummary> {
    let mut files = Vec::new();
    let mut ignored_dirs = vec![
        ".git".to_string(),
        "target".to_string(),
        "node_modules".to_string(),
        "dist".to_string(),
        "build".to_string(),
        ".prometheos-lite".to_string(),
    ];

    for entry in WalkDir::new(repo_path)
        .into_iter()
        .filter_entry(|entry| !is_ignored(entry))
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_candidate_file(path) {
            continue;
        }
        let metadata = fs::metadata(path)?;
        if metadata.len() > 200_000 {
            continue;
        }
        let content = fs::read_to_string(path).unwrap_or_default();
        let relative = path
            .strip_prefix(repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        files.push(FileSummary {
            path: relative,
            bytes: metadata.len(),
            lines: content.lines().count(),
        });
        if files.len() >= 80 {
            break;
        }
    }

    ignored_dirs.sort();
    ignored_dirs.dedup();

    Ok(RepoSummary {
        project_type: detect_project_type(repo_path),
        files_scanned: files.len(),
        candidate_files: files,
        ignored_dirs,
    })
}

fn is_ignored(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    matches!(
        name.as_ref(),
        ".git" | "target" | "node_modules" | "dist" | "build" | ".prometheos-lite"
    )
}

fn is_candidate_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    if matches!(
        file_name,
        "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "README.md"
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "cpp" | "c" | "h" | "toml" | "yaml" | "yml")
    )
}

fn detect_project_type(repo_path: &Path) -> String {
    if repo_path.join("Cargo.toml").exists() {
        "rust".to_string()
    } else if repo_path.join("package.json").exists() {
        "node".to_string()
    } else if repo_path.join("pyproject.toml").exists() || repo_path.join("requirements.txt").exists() {
        "python".to_string()
    } else if repo_path.join("go.mod").exists() {
        "go".to_string()
    } else {
        "unknown".to_string()
    }
}

fn analyze_risks(repo_path: &Path, summary: &RepoSummary) -> Result<Vec<RiskFinding>> {
    let mut findings = Vec::new();

    for file in &summary.candidate_files {
        let path = repo_path.join(&file.path);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            let line_number = idx + 1;
            let lower = line.to_lowercase();

            let finding = if line.contains(".unwrap()") || line.contains("unwrap(") {
                Some((
                    "Medium",
                    "panic-risk",
                    "Potential panic from unwrap".to_string(),
                    "Replace unwrap with structured error handling or a safe fallback.".to_string(),
                ))
            } else if line.contains(".expect(") || line.contains("panic!(") {
                Some((
                    "Medium",
                    "panic-risk",
                    "Explicit panic path in application code".to_string(),
                    "Return a typed error or convert the panic into a controlled failure path.".to_string(),
                ))
            } else if lower.contains("todo!") || lower.contains("fixme") || lower.contains("todo:") {
                Some((
                    "Low",
                    "unfinished-work",
                    "Unfinished implementation marker".to_string(),
                    "Convert the marker into a tracked task or complete the implementation before release.".to_string(),
                ))
            } else if looks_like_hardcoded_secret(&lower) {
                Some((
                    "High",
                    "secret-risk",
                    "Possible hardcoded secret or credential".to_string(),
                    "Move the value into environment-backed configuration or a secret manager.".to_string(),
                ))
            } else if lower.contains("eval(") {
                Some((
                    "High",
                    "code-execution-risk",
                    "Dynamic eval call detected".to_string(),
                    "Avoid eval or strictly constrain and validate input before execution.".to_string(),
                ))
            } else if lower.contains("shell=true") {
                Some((
                    "High",
                    "command-injection-risk",
                    "Shell execution with shell=True detected".to_string(),
                    "Use argument arrays and avoid shell interpolation for subprocess execution.".to_string(),
                ))
            } else if lower.contains("innerhtml") {
                Some((
                    "Medium",
                    "xss-risk",
                    "Direct innerHTML usage detected".to_string(),
                    "Prefer safe DOM APIs or sanitize trusted markup before insertion.".to_string(),
                ))
            } else {
                None
            };

            if let Some((risk, category, summary, recommendation)) = finding {
                findings.push(RiskFinding {
                    id: format!("F-{:03}", findings.len() + 1),
                    file: file.path.clone(),
                    line: line_number,
                    risk,
                    category,
                    summary,
                    recommendation,
                });
            }
        }
    }

    Ok(findings)
}

fn looks_like_hardcoded_secret(lower: &str) -> bool {
    let mentions_secret = ["api_key", "apikey", "secret", "password", "token"]
        .iter()
        .any(|needle| lower.contains(needle));
    mentions_secret && lower.contains('=') && !lower.contains("env") && !lower.contains("example")
}

fn render_risk_report(context: &WorkbenchContext, findings: &[RiskFinding]) -> String {
    let mut out = String::new();
    out.push_str("# Risk Review\n\n");
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "PrometheOS Lite reviewed `{}` and found {} risk finding(s) across {} candidate file(s).\n\n",
        context.repo_path.display(),
        findings.len(),
        context.repo_summary.candidate_files.len()
    ));
    out.push_str("## WorkContext\n\n");
    out.push_str(&format!("- ID: `{}`\n", context.id));
    out.push_str(&format!("- Goal: {}\n", context.goal));
    out.push_str(&format!("- Mode: {}\n", context.mode));
    out.push_str(&format!("- Project type: {}\n\n", context.repo_summary.project_type));

    if findings.is_empty() {
        out.push_str("## Findings\n\nNo obvious risky patterns were found by the MVP heuristic scanner. This does not mean the repo is safe; it means the boring little scanner did not catch anything obvious.\n");
        return out;
    }

    out.push_str("## Findings\n\n");
    for finding in findings {
        out.push_str(&format!("### {}. {}\n\n", finding.id, finding.summary));
        out.push_str(&format!("- File: `{}`\n", finding.file));
        out.push_str(&format!("- Line: {}\n", finding.line));
        out.push_str(&format!("- Risk: {}\n", finding.risk));
        out.push_str(&format!("- Category: {}\n", finding.category));
        out.push_str(&format!("- Suggested fix: {}\n\n", finding.recommendation));
    }

    out
}

fn render_patch_suggestions(context: &WorkbenchContext, findings: &[RiskFinding]) -> String {
    let mut out = String::new();
    out.push_str("# Suggested Patch Plan\n\n");
    out.push_str("> Safety note: this MVP artifact is a staged suggestion only. No repository files were modified.\n\n");
    out.push_str(&format!("WorkContext: `{}`\n\n", context.id));

    if findings.is_empty() {
        out.push_str("No patch suggestions were generated because the MVP risk scanner found no obvious risky patterns.\n");
        return out;
    }

    out.push_str("## Recommended Changes\n\n");
    for finding in findings.iter().take(20) {
        out.push_str(&format!("### {}: `{}` line {}\n\n", finding.id, finding.file, finding.line));
        out.push_str(&format!("Risk: {} / {}\n\n", finding.risk, finding.category));
        out.push_str(&format!("Suggested change: {}\n\n", finding.recommendation));
    }

    out.push_str("## Approval\n\n");
    out.push_str("Approve this artifact when the suggestions are acceptable. The current MVP records approval only; an explicit future writer must apply changes.\n");
    out
}

fn write_artifact(
    context: &WorkbenchContext,
    kind: &str,
    title: &str,
    status: &str,
    requires_approval: bool,
    content: &str,
) -> Result<ArtifactRef> {
    let id = Uuid::new_v4().to_string();
    let dir = artifacts_dir(&context.repo_path, &context.id);
    fs::create_dir_all(&dir)?;
    let filename = format!("{}-{}.md", kind, &id[..8]);
    let path = dir.join(filename);
    fs::write(&path, content)?;
    Ok(ArtifactRef {
        id,
        kind: kind.to_string(),
        title: title.to_string(),
        path,
        status: status.to_string(),
        requires_approval,
        created_at: Utc::now(),
    })
}

fn upsert_artifact(context: &mut WorkbenchContext, artifact: ArtifactRef) {
    context.artifacts.retain(|existing| existing.kind != artifact.kind);
    context.artifacts.push(artifact);
}

fn write_memory(context: &WorkbenchContext, findings: &[RiskFinding]) -> Result<()> {
    fs::create_dir_all(memory_dir(&context.repo_path))?;
    let mut out = String::new();
    out.push_str("# Repo Workbench Memory\n\n");
    out.push_str(&format!("- WorkContext: `{}`\n", context.id));
    out.push_str(&format!("- Title: {}\n", context.title));
    out.push_str(&format!("- Goal: {}\n", context.goal));
    out.push_str(&format!("- Repo: `{}`\n", context.repo_path.display()));
    out.push_str(&format!("- Status: {}\n", context.status));
    out.push_str(&format!("- Phase: {}\n", context.phase));
    out.push_str(&format!("- Project type: {}\n", context.repo_summary.project_type));
    out.push_str(&format!("- Candidate files: {}\n", context.repo_summary.candidate_files.len()));
    out.push_str(&format!("- Artifacts: {}\n", context.artifacts.len()));
    out.push_str(&format!("- Decisions: {}\n", context.decisions.len()));
    if let Some(next) = &context.next_action {
        out.push_str(&format!("- Next action: {}\n", next));
    }

    if !findings.is_empty() {
        out.push_str("\n## Latest Findings\n\n");
        for finding in findings.iter().take(20) {
            out.push_str(&format!(
                "- {} `{}`:{} [{}] {}\n",
                finding.id, finding.file, finding.line, finding.risk, finding.summary
            ));
        }
    }

    if !context.artifacts.is_empty() {
        out.push_str("\n## Artifacts\n\n");
        for artifact in &context.artifacts {
            out.push_str(&format!(
                "- `{}` {} ({}, status: {}, approval: {}) at `{}`\n",
                artifact.id,
                artifact.title,
                artifact.kind,
                artifact.status,
                artifact.requires_approval,
                artifact.path.display()
            ));
        }
    }

    fs::write(memory_dir(&context.repo_path).join(format!("{}.md", context.id)), out)?;
    Ok(())
}

fn load_memory(context: &WorkbenchContext) -> Result<String> {
    let path = memory_dir(&context.repo_path).join(format!("{}.md", context.id));
    fs::read_to_string(&path).with_context(|| format!("Memory not found at {}", path.display()))
}

fn print_status(context: &WorkbenchContext) {
    println!("Repo Workbench WorkContext");
    println!("  ID: {}", context.id);
    println!("  Title: {}", context.title);
    println!("  Goal: {}", context.goal);
    println!("  Repo: {}", context.repo_path.display());
    println!("  Status: {}", context.status);
    println!("  Phase: {}", context.phase);
    println!("  Project type: {}", context.repo_summary.project_type);
    println!("  Candidate files: {}", context.repo_summary.candidate_files.len());
    println!("  Artifacts: {}", context.artifacts.len());
    println!("  Decisions: {}", context.decisions.len());
    if let Some(next) = &context.next_action {
        println!("  Next: {}", next);
    }
}
