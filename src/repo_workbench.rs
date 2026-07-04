use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchContext {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub mode: String,
    pub repo_path: PathBuf,
    pub status: String,
    pub phase: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repo_summary: RepoSummary,
    pub artifacts: Vec<ArtifactRef>,
    pub decisions: Vec<DecisionRecord>,
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoSummary {
    pub project_type: String,
    pub files_scanned: usize,
    pub candidate_files: Vec<FileSummary>,
    pub ignored_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSummary {
    pub path: String,
    pub bytes: u64,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactProvenance {
    pub generator: String,
    pub generation_mode: String,
    pub model_invoked: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub provider_kind: Option<String>,
    pub local_provider: Option<bool>,
    pub work_id: String,
    pub artifact_type: String,
    pub created_at: DateTime<Utc>,
}

impl ArtifactProvenance {
    pub fn deterministic(work_id: &str, artifact_type: &str) -> Self {
        Self {
            generator: "repo_workbench".to_string(),
            generation_mode: "deterministic_static_analysis".to_string(),
            model_invoked: false,
            provider: None,
            model: None,
            provider_kind: None,
            local_provider: None,
            work_id: work_id.to_string(),
            artifact_type: artifact_type.to_string(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub path: PathBuf,
    pub status: String,
    pub requires_approval: bool,
    pub created_at: DateTime<Utc>,
    pub provenance: ArtifactProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: String,
    pub artifact_id: String,
    pub decision: String,
    pub approved: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RiskFinding {
    pub id: String,
    pub file: String,
    pub line: usize,
    pub risk: &'static str,
    pub category: &'static str,
    pub summary: String,
    pub recommendation: String,
}

pub fn create_repo_workbench_context(
    repo_path: &Path,
    goal: &str,
    mode: &str,
    title: Option<String>,
) -> Result<WorkbenchContext> {
    let repo_path = normalize_repo_path(repo_path)?;
    let summary = scan_repo(&repo_path)?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let title = title.unwrap_or_else(|| title_from_goal(goal));
    let context = WorkbenchContext {
        id: id.clone(),
        title,
        goal: goal.to_string(),
        mode: mode.to_string(),
        repo_path: repo_path.clone(),
        status: "draft".to_string(),
        phase: "intake".to_string(),
        created_at: now,
        updated_at: now,
        repo_summary: summary,
        artifacts: Vec::new(),
        decisions: Vec::new(),
        next_action: Some(
            "Run `prometheos repo run <work_id>` to perform read-only risk review.".to_string(),
        ),
    };

    save_context(&context)?;
    write_memory(&context, &[])?;
    // Register in cwd-level index for cross-directory discovery
    let _ = register_context(&id, &repo_path);

    Ok(context)
}

pub fn run_repo_workbench_context(context: &mut WorkbenchContext) -> Result<()> {
    context.status = "in_progress".to_string();
    context.phase = "review".to_string();
    context.repo_summary = scan_repo(&context.repo_path)?;

    let findings = analyze_risks(&context.repo_path, &context.repo_summary)?;
    let report = render_risk_report(context, &findings);
    let patch = render_patch_suggestions(context, &findings);

    let report_artifact = write_artifact(
        context,
        "risk-report",
        "Risk Review Report",
        "ready",
        false,
        &report,
    )?;
    let patch_artifact = write_artifact(
        context,
        "suggested-patch",
        "Suggested Patch Plan",
        "awaiting_approval",
        true,
        &patch,
    )?;

    upsert_artifact(context, report_artifact);
    upsert_artifact(context, patch_artifact);
    context.status = "awaiting_approval".to_string();
    context.phase = "approval".to_string();
    context.next_action = context
        .artifacts
        .iter()
        .find(|artifact| artifact.requires_approval && artifact.status == "awaiting_approval")
        .map(|artifact| {
            format!(
                "Review `{}` and approve with `prometheos repo approve {}`.",
                artifact.title, artifact.id
            )
        });
    context.updated_at = Utc::now();

    save_context(context)?;
    write_memory(context, &findings)?;

    Ok(())
}

/// Look up a context file path by searching ancestors and the local registry.
fn resolve_context_path(id: &str) -> Option<PathBuf> {
    let filename = format!("{}.json", id);

    // 1. Check registry (cwd-level index of work_id -> repo_path)
    if let Some(path) = resolve_from_registry(id)
        && path.exists()
    {
        return Some(path);
    }

    // 2. Walk parent directories for a local .prometheos-lite/workbench/contexts/
    let cwd = std::env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor
            .join(".prometheos-lite")
            .join("workbench")
            .join("contexts")
            .join(&filename);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Read the cwd-level registry mapping work_id -> absolute repo path.
fn resolve_from_registry(id: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let registry_path = cwd
        .join(".prometheos-lite")
        .join("workbench")
        .join("registry.json");
    let json = fs::read_to_string(registry_path).ok()?;
    let registry: std::collections::HashMap<String, String> = serde_json::from_str(&json).ok()?;
    let repo_path_str = registry.get(id)?;
    let path = Path::new(repo_path_str)
        .join(".prometheos-lite")
        .join("workbench")
        .join("contexts")
        .join(format!("{}.json", id));
    Some(path)
}

/// Record the work_id -> repo_path mapping in the cwd-level registry.
fn register_context(id: &str, repo_path: &Path) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let registry_dir = cwd.join(".prometheos-lite").join("workbench");
    let registry_path = registry_dir.join("registry.json");

    fs::create_dir_all(&registry_dir)?;

    let mut registry: std::collections::HashMap<String, String> = if registry_path.exists() {
        let json = fs::read_to_string(&registry_path)?;
        serde_json::from_str(&json).unwrap_or_default()
    } else {
        std::collections::HashMap::new()
    };

    registry.insert(id.to_string(), repo_path.to_string_lossy().to_string());
    fs::write(&registry_path, serde_json::to_string_pretty(&registry)?)?;
    Ok(())
}

pub fn load_context(id: &str) -> Result<WorkbenchContext> {
    let path = resolve_context_path(id).ok_or_else(|| {
        anyhow::anyhow!(
            "WorkContext `{}` not found. Searched registry and parent directories from {} for `.prometheos-lite/workbench/contexts/`",
            id,
            std::env::current_dir().unwrap_or_default().display()
        )
    })?;
    let json =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let context = serde_json::from_str(&json)?;
    Ok(context)
}

pub fn find_context_by_artifact(artifact_id: &str) -> Result<WorkbenchContext> {
    let cwd = std::env::current_dir()?;

    // Search cwd-level registry first
    let registry_path = cwd
        .join(".prometheos-lite")
        .join("workbench")
        .join("registry.json");
    if let Ok(json) = fs::read_to_string(&registry_path)
        && let Ok(registry) =
            serde_json::from_str::<std::collections::HashMap<String, String>>(&json)
    {
        for repo_path_str in registry.values() {
            let dir = Path::new(repo_path_str)
                .join(".prometheos-lite")
                .join("workbench")
                .join("contexts");
            if !dir.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let json = match fs::read_to_string(entry.path()) {
                        Ok(j) => j,
                        _ => continue,
                    };
                    let context: WorkbenchContext = match serde_json::from_str(&json) {
                        Ok(c) => c,
                        _ => continue,
                    };
                    if context
                        .artifacts
                        .iter()
                        .any(|artifact| artifact.id == artifact_id)
                    {
                        return Ok(context);
                    }
                }
            }
        }
    }

    // Fallback: walk parent directories for local contexts/ dirs
    for ancestor in cwd.ancestors() {
        let dir = ancestor
            .join(".prometheos-lite")
            .join("workbench")
            .join("contexts");
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let json = match fs::read_to_string(entry.path()) {
                Ok(j) => j,
                _ => continue,
            };
            let context: WorkbenchContext = match serde_json::from_str(&json) {
                Ok(c) => c,
                _ => continue,
            };
            if context
                .artifacts
                .iter()
                .any(|artifact| artifact.id == artifact_id)
            {
                return Ok(context);
            }
        }
    }
    anyhow::bail!(
        "Artifact `{}` not found in any repo workbench store",
        artifact_id
    )
}

pub fn repo_workbench_context_exists(id: &str) -> bool {
    resolve_context_path(id).is_some()
}

pub fn get_artifacts(context: &WorkbenchContext) -> &[ArtifactRef] {
    &context.artifacts
}

pub fn approve_artifact(context: &mut WorkbenchContext, artifact_id: &str) -> Result<()> {
    let artifact = context
        .artifacts
        .iter_mut()
        .find(|artifact| artifact.id == artifact_id)
        .with_context(|| format!("Artifact `{}` not found", artifact_id))?;

    if !artifact.requires_approval {
        println!(
            "Artifact `{}` does not require approval; recording acknowledgement anyway.",
            artifact.id
        );
    }

    artifact.status = "approved".to_string();
    context.decisions.push(DecisionRecord {
        id: Uuid::new_v4().to_string(),
        artifact_id: artifact_id.to_string(),
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
    save_context(context)?;
    write_memory(context, &[])?;
    Ok(())
}

pub fn load_memory(context: &WorkbenchContext) -> Result<String> {
    let path = memory_dir(&context.repo_path).join(format!("{}.md", context.id));
    fs::read_to_string(&path).with_context(|| format!("Memory not found at {}", path.display()))
}

pub fn print_status(context: &WorkbenchContext) {
    println!("Repo Workbench WorkContext");
    println!("  ID: {}", context.id);
    println!("  Title: {}", context.title);
    println!("  Goal: {}", context.goal);
    println!("  Repo: {}", context.repo_path.display());
    println!("  Status: {}", context.status);
    println!("  Phase: {}", context.phase);
    println!("  Project type: {}", context.repo_summary.project_type);
    println!(
        "  Candidate files: {}",
        context.repo_summary.candidate_files.len()
    );
    println!("  Artifacts: {}", context.artifacts.len());
    println!("  Decisions: {}", context.decisions.len());
    if let Some(next) = &context.next_action {
        println!("  Next: {}", next);
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
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if matches!(
        file_name,
        "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "README.md"
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "go"
                | "java"
                | "cpp"
                | "c"
                | "h"
                | "toml"
                | "yaml"
                | "yml"
        )
    )
}

fn detect_project_type(repo_path: &Path) -> String {
    if repo_path.join("Cargo.toml").exists() {
        "rust".to_string()
    } else if repo_path.join("package.json").exists() {
        "node".to_string()
    } else if repo_path.join("pyproject.toml").exists()
        || repo_path.join("requirements.txt").exists()
    {
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
                    "Return a typed error or convert the panic into a controlled failure path."
                        .to_string(),
                ))
            } else if lower.contains("todo!") || lower.contains("fixme") || lower.contains("todo:")
            {
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
                    "Move the value into environment-backed configuration or a secret manager."
                        .to_string(),
                ))
            } else if lower.contains("eval(") {
                Some((
                    "High",
                    "code-execution-risk",
                    "Dynamic eval call detected".to_string(),
                    "Avoid eval or strictly constrain and validate input before execution."
                        .to_string(),
                ))
            } else if lower.contains("shell=true") {
                Some((
                    "High",
                    "command-injection-risk",
                    "Shell execution with shell=True detected".to_string(),
                    "Use argument arrays and avoid shell interpolation for subprocess execution."
                        .to_string(),
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
    out.push_str("## Provenance\n\n");
    out.push_str("- Generator: Repo Workbench\n");
    out.push_str("- Generation mode: deterministic static analysis\n");
    out.push_str("- Model invoked: no\n");
    out.push_str("- Provider: none\n");
    out.push_str("- Model: none\n\n");
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
    out.push_str(&format!(
        "- Project type: {}\n\n",
        context.repo_summary.project_type
    ));

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
    out.push_str("## Provenance\n\n");
    out.push_str("- Generator: Repo Workbench\n");
    out.push_str("- Generation mode: deterministic static analysis\n");
    out.push_str("- Model invoked: no\n");
    out.push_str("- Provider: none\n");
    out.push_str("- Model: none\n\n");
    out.push_str(&format!("WorkContext: `{}`\n\n", context.id));

    if findings.is_empty() {
        out.push_str("No patch suggestions were generated because the MVP risk scanner found no obvious risky patterns.\n");
        return out;
    }

    out.push_str("## Recommended Changes\n\n");
    for finding in findings.iter().take(20) {
        out.push_str(&format!(
            "### {}: `{}` line {}\n\n",
            finding.id, finding.file, finding.line
        ));
        out.push_str(&format!(
            "Risk: {} / {}\n\n",
            finding.risk, finding.category
        ));
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
    let now = Utc::now();
    Ok(ArtifactRef {
        id,
        kind: kind.to_string(),
        title: title.to_string(),
        path,
        status: status.to_string(),
        requires_approval,
        created_at: now,
        provenance: ArtifactProvenance {
            generator: "repo_workbench".to_string(),
            generation_mode: "deterministic_static_analysis".to_string(),
            model_invoked: false,
            provider: None,
            model: None,
            provider_kind: None,
            local_provider: None,
            work_id: context.id.clone(),
            artifact_type: kind.to_string(),
            created_at: now,
        },
    })
}

fn upsert_artifact(context: &mut WorkbenchContext, artifact: ArtifactRef) {
    context
        .artifacts
        .retain(|existing| existing.kind != artifact.kind);
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
    out.push_str(&format!(
        "- Project type: {}\n",
        context.repo_summary.project_type
    ));
    out.push_str(&format!(
        "- Candidate files: {}\n",
        context.repo_summary.candidate_files.len()
    ));
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

    fs::write(
        memory_dir(&context.repo_path).join(format!("{}.md", context.id)),
        out,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn temp_repo() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write_file(dir: &Path, relative: &str, content: &str) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
    }

    fn make_context(repo_path: &Path) -> WorkbenchContext {
        WorkbenchContext {
            id: "test-id".to_string(),
            title: "Test".to_string(),
            goal: "Find issues".to_string(),
            mode: "review".to_string(),
            repo_path: repo_path.to_path_buf(),
            status: "draft".to_string(),
            phase: "intake".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            repo_summary: RepoSummary::default(),
            artifacts: Vec::new(),
            decisions: Vec::new(),
            next_action: None,
        }
    }

    fn make_artifact_ref(
        id: &str,
        kind: &str,
        title: &str,
        path: &str,
        status: &str,
        requires_approval: bool,
    ) -> ArtifactRef {
        let now = Utc::now();
        ArtifactRef {
            id: id.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            path: PathBuf::from(path),
            status: status.to_string(),
            requires_approval,
            created_at: now,
            provenance: ArtifactProvenance {
                generator: "repo_workbench".to_string(),
                generation_mode: "deterministic_static_analysis".to_string(),
                model_invoked: false,
                provider: None,
                model: None,
                provider_kind: None,
                local_provider: None,
                work_id: "test-id".to_string(),
                artifact_type: kind.to_string(),
                created_at: now,
            },
        }
    }

    // ── scan_repo ─────────────────────────────────────────────────────────

    #[test]
    fn test_scan_repo_ignores_ignored_dirs() {
        let dir = temp_repo();
        let root = dir.path();

        write_file(root, "Cargo.toml", "[package]\nname = \"test\"\n");
        write_file(root, ".git/HEAD", "ref: refs/heads/main\n");
        write_file(root, "target/debug/test.o", "");
        write_file(root, "node_modules/pkg/index.js", "// dep");
        write_file(root, ".prometheos-lite/workbench/contexts/test.json", "{}");

        let summary = scan_repo(root).unwrap();
        let paths: Vec<&str> = summary
            .candidate_files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(
            paths.contains(&"Cargo.toml"),
            "Cargo.toml should be scanned"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".git")),
            ".git should be ignored"
        );
        assert!(
            !paths.iter().any(|p| p.contains("target/")),
            "target should be ignored"
        );
        assert!(
            !paths.iter().any(|p| p.contains("node_modules/")),
            "node_modules should be ignored"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".prometheos-lite")),
            ".prometheos-lite should be ignored"
        );
    }

    // ── detect_project_type ───────────────────────────────────────────────

    #[test]
    fn test_detect_project_type_rust() {
        let dir = temp_repo();
        write_file(dir.path(), "Cargo.toml", "[package]\n");
        assert_eq!(detect_project_type(dir.path()), "rust");
    }

    #[test]
    fn test_detect_project_type_node() {
        let dir = temp_repo();
        write_file(dir.path(), "package.json", "{}");
        assert_eq!(detect_project_type(dir.path()), "node");
    }

    #[test]
    fn test_detect_project_type_python_pyproject() {
        let dir = temp_repo();
        write_file(dir.path(), "pyproject.toml", "[project]\n");
        assert_eq!(detect_project_type(dir.path()), "python");
    }

    #[test]
    fn test_detect_project_type_python_requirements() {
        let dir = temp_repo();
        write_file(dir.path(), "requirements.txt", "requests\n");
        assert_eq!(detect_project_type(dir.path()), "python");
    }

    #[test]
    fn test_detect_project_type_go() {
        let dir = temp_repo();
        write_file(dir.path(), "go.mod", "module test\n");
        assert_eq!(detect_project_type(dir.path()), "go");
    }

    #[test]
    fn test_detect_project_type_unknown() {
        let dir = temp_repo();
        assert_eq!(detect_project_type(dir.path()), "unknown");
    }

    // ── is_candidate_file ─────────────────────────────────────────────────

    #[test]
    fn test_is_candidate_file_extensions() {
        assert!(is_candidate_file_inner("main.rs"));
        assert!(is_candidate_file_inner("lib.ts"));
        assert!(is_candidate_file_inner("app.tsx"));
        assert!(is_candidate_file_inner("index.js"));
        assert!(is_candidate_file_inner("app.jsx"));
        assert!(is_candidate_file_inner("script.py"));
        assert!(is_candidate_file_inner("main.go"));
        assert!(is_candidate_file_inner("App.java"));
        assert!(is_candidate_file_inner("lib.cpp"));
        assert!(is_candidate_file_inner("lib.c"));
        assert!(is_candidate_file_inner("lib.h"));
        assert!(is_candidate_file_inner("conf.toml"));
        assert!(is_candidate_file_inner("config.yaml"));
        assert!(is_candidate_file_inner("config.yml"));
        assert!(!is_candidate_file_inner("image.png"));
        assert!(!is_candidate_file_inner("data.json"));
        assert!(!is_candidate_file_inner("file.md"));
    }

    #[test]
    fn test_is_candidate_file_special_names() {
        assert!(is_candidate_file_inner("Cargo.toml"));
        assert!(is_candidate_file_inner("package.json"));
        assert!(is_candidate_file_inner("pyproject.toml"));
        assert!(is_candidate_file_inner("go.mod"));
        assert!(is_candidate_file_inner("README.md"));
    }

    fn is_candidate_file_inner(name: &str) -> bool {
        let path = Path::new(name);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if matches!(
            file_name,
            "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "README.md"
        ) {
            return true;
        }
        matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some(
                "rs" | "ts"
                    | "tsx"
                    | "js"
                    | "jsx"
                    | "py"
                    | "go"
                    | "java"
                    | "cpp"
                    | "c"
                    | "h"
                    | "toml"
                    | "yaml"
                    | "yml"
            )
        )
    }

    // ── analyze_risks ─────────────────────────────────────────────────────

    fn make_summary_with_file(repo_path: &Path, relative: &str) -> RepoSummary {
        let metadata = fs::metadata(repo_path.join(relative)).unwrap();
        RepoSummary {
            project_type: "rust".to_string(),
            files_scanned: 1,
            candidate_files: vec![FileSummary {
                path: relative.to_string(),
                bytes: metadata.len(),
                lines: fs::read_to_string(repo_path.join(relative))
                    .unwrap()
                    .lines()
                    .count(),
            }],
            ignored_dirs: vec![],
        }
    }

    #[test]
    fn test_analyze_risks_detects_unwrap() {
        let dir = temp_repo();
        write_file(
            dir.path(),
            "src/main.rs",
            "fn main() { let x = data.unwrap(); }",
        );
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.summary.contains("unwrap")),
            "should detect unwrap"
        );
    }

    #[test]
    fn test_analyze_risks_detects_expect() {
        let dir = temp_repo();
        write_file(
            dir.path(),
            "src/main.rs",
            "fn main() { let x = data.expect(\"msg\"); }",
        );
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.summary.contains("panic")),
            "should detect expect"
        );
    }

    #[test]
    fn test_analyze_risks_detects_panic() {
        let dir = temp_repo();
        write_file(dir.path(), "src/main.rs", "fn main() { panic!(\"boom\"); }");
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.summary.contains("panic")),
            "should detect panic"
        );
    }

    #[test]
    fn test_analyze_risks_detects_todo_marker() {
        let dir = temp_repo();
        write_file(dir.path(), "src/main.rs", "fn main() { let _ = todo! }");
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.category == "unfinished-work"),
            "should detect todo marker"
        );
    }

    #[test]
    fn test_analyze_risks_detects_fixme_marker() {
        let dir = temp_repo();
        write_file(dir.path(), "src/main.rs", "fn main() { let _ = fixme }");
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.category == "unfinished-work"),
            "should detect fixme marker"
        );
    }

    #[test]
    fn test_analyze_risks_detects_hardcoded_secret() {
        let dir = temp_repo();
        write_file(
            dir.path(),
            "src/main.rs",
            "fn main() { let api_key = \"sk-123\"; }",
        );
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.category == "secret-risk"),
            "should detect hardcoded secret"
        );
    }

    #[test]
    fn test_analyze_risks_detects_eval() {
        let dir = temp_repo();
        write_file(dir.path(), "src/main.rs", "fn main() { eval(user_input); }");
        let summary = make_summary_with_file(dir.path(), "src/main.rs");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.category == "code-execution-risk"),
            "should detect eval"
        );
    }

    #[test]
    fn test_analyze_risks_detects_shell_true() {
        let dir = temp_repo();
        write_file(dir.path(), "script.py", "subprocess.run(cmd, shell=True)");
        let summary = make_summary_with_file(dir.path(), "script.py");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.category == "command-injection-risk"),
            "should detect shell=True"
        );
    }

    #[test]
    fn test_analyze_risks_detects_innerhtml() {
        let dir = temp_repo();
        write_file(
            dir.path(),
            "app.js",
            "document.getElementById('x').innerHTML = html;",
        );
        let summary = make_summary_with_file(dir.path(), "app.js");
        let findings = analyze_risks(dir.path(), &summary).unwrap();
        assert!(
            findings.iter().any(|f| f.category == "xss-risk"),
            "should detect innerHTML"
        );
    }

    // ── render_risk_report ───────────────────────────────────────────────

    #[test]
    fn test_render_risk_report_creates_output() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let findings = vec![RiskFinding {
            id: "F-001".to_string(),
            file: "src/main.rs".to_string(),
            line: 5,
            risk: "Medium",
            category: "panic-risk",
            summary: "Potential panic from unwrap".to_string(),
            recommendation: "Replace unwrap".to_string(),
        }];
        let report = render_risk_report(&context, &findings);
        assert!(report.contains("# Risk Review"), "should have title");
        assert!(
            report.contains("## Provenance"),
            "should have provenance section"
        );
        assert!(
            report.contains("Model invoked: no"),
            "should state no model"
        );
        assert!(
            report.contains("deterministic"),
            "should reference deterministic analysis"
        );
        assert!(report.contains("F-001"), "should contain finding ID");
        assert!(report.contains("Potential panic"), "should contain summary");
    }

    #[test]
    fn test_render_risk_report_empty() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let report = render_risk_report(&context, &[]);
        assert!(
            report.contains("No obvious risky patterns"),
            "empty case message"
        );
    }

    // ── render_patch_suggestions ──────────────────────────────────────────

    #[test]
    fn test_render_patch_suggestions_creates_output() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let findings = vec![RiskFinding {
            id: "F-001".to_string(),
            file: "src/main.rs".to_string(),
            line: 5,
            risk: "Medium",
            category: "panic-risk",
            summary: "Potential panic".to_string(),
            recommendation: "Replace with error handling".to_string(),
        }];
        let patch = render_patch_suggestions(&context, &findings);
        assert!(
            patch.contains("# Suggested Patch Plan"),
            "should have title"
        );
        assert!(
            patch.contains("## Provenance"),
            "should have provenance section"
        );
        assert!(patch.contains("Model invoked: no"), "should state no model");
        assert!(patch.contains("F-001"), "should contain finding ID");
        assert!(patch.contains("Approval"), "should mention approval");
    }

    #[test]
    fn test_render_patch_suggestions_empty() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let patch = render_patch_suggestions(&context, &[]);
        assert!(patch.contains("No patch suggestions"), "empty case message");
    }

    // ── write_artifact ────────────────────────────────────────────────────

    #[test]
    fn test_write_artifact_creates_file() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let artifact = write_artifact(
            &context,
            "risk-report",
            "Test Report",
            "ready",
            false,
            "# Report",
        )
        .unwrap();
        assert!(artifact.path.exists(), "artifact file should exist on disk");
        let content = fs::read_to_string(&artifact.path).unwrap();
        assert_eq!(content.trim(), "# Report");
        assert_eq!(artifact.kind, "risk-report");
        assert_eq!(artifact.status, "ready");
        assert!(!artifact.requires_approval);
    }

    #[test]
    fn test_write_artifact_requires_approval() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let artifact = write_artifact(
            &context,
            "suggested-patch",
            "Patch",
            "awaiting_approval",
            true,
            "# Patch",
        )
        .unwrap();
        assert!(artifact.requires_approval);
        assert_eq!(artifact.status, "awaiting_approval");
    }

    // ── upsert_artifact ───────────────────────────────────────────────────

    #[test]
    fn test_upsert_artifact_replaces_by_kind() {
        let mut context = make_context(Path::new("/tmp"));
        let a1 = make_artifact_ref("id-1", "risk-report", "Old", "/tmp/a.md", "ready", false);
        let a2 = make_artifact_ref("id-2", "risk-report", "New", "/tmp/b.md", "ready", false);
        context.artifacts.push(a1);
        upsert_artifact(&mut context, a2);
        assert_eq!(
            context.artifacts.len(),
            1,
            "old artifact should be replaced"
        );
        assert_eq!(
            context.artifacts[0].id, "id-2",
            "new artifact should be present"
        );
    }

    #[test]
    fn test_upsert_artifact_appends_different_kind() {
        let mut context = make_context(Path::new("/tmp"));
        let a1 = make_artifact_ref("id-1", "risk-report", "Report", "/tmp/a.md", "ready", false);
        let a2 = make_artifact_ref(
            "id-2",
            "suggested-patch",
            "Patch",
            "/tmp/b.md",
            "awaiting_approval",
            true,
        );
        context.artifacts.push(a1);
        upsert_artifact(&mut context, a2);
        assert_eq!(
            context.artifacts.len(),
            2,
            "different kinds should both be present"
        );
    }

    // ── write_memory ──────────────────────────────────────────────────────

    #[test]
    fn test_write_memory_includes_required_fields() {
        let dir = temp_repo();
        let mut context = make_context(dir.path());
        context.id = "mem-test-id".to_string();
        context.goal = "Find issues".to_string();
        context.status = "awaiting_approval".to_string();
        context.next_action = Some("Approve the patch".to_string());
        let now = Utc::now();
        context.artifacts.push(ArtifactRef {
            id: "art-1".to_string(),
            kind: "risk-report".to_string(),
            title: "Report".to_string(),
            path: PathBuf::from("report.md"),
            status: "ready".to_string(),
            requires_approval: false,
            created_at: now,
            provenance: ArtifactProvenance {
                generator: "repo_workbench".to_string(),
                generation_mode: "deterministic_static_analysis".to_string(),
                model_invoked: false,
                provider: None,
                model: None,
                provider_kind: None,
                local_provider: None,
                work_id: "mem-test-id".to_string(),
                artifact_type: "risk-report".to_string(),
                created_at: now,
            },
        });

        write_memory(&context, &[]).unwrap();

        let memory_path = memory_dir(dir.path()).join("mem-test-id.md");
        assert!(memory_path.exists(), "memory file should exist");
        let content = fs::read_to_string(&memory_path).unwrap();
        assert!(
            content.contains("mem-test-id"),
            "should contain work context ID"
        );
        assert!(content.contains("Find issues"), "should contain goal");
        assert!(
            content.contains(&dir.path().to_string_lossy().to_string()),
            "should contain repo path"
        );
        assert!(
            content.contains("awaiting_approval"),
            "should contain status"
        );
        assert!(content.contains("Report"), "should contain artifact info");
        assert!(
            content.contains("Approve the patch"),
            "should contain next action"
        );
    }

    #[test]
    fn test_write_memory_with_findings() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let findings = vec![RiskFinding {
            id: "F-001".to_string(),
            file: "src/main.rs".to_string(),
            line: 5,
            risk: "Medium",
            category: "panic-risk",
            summary: "Potential panic".to_string(),
            recommendation: "Fix it".to_string(),
        }];
        write_memory(&context, &findings).unwrap();
        let memory_path = memory_dir(dir.path()).join("test-id.md");
        let content = fs::read_to_string(&memory_path).unwrap();
        assert!(content.contains("F-001"), "should contain finding ID");
        assert!(
            content.contains("Potential panic"),
            "should contain finding summary"
        );
    }

    // ── approve flow ──────────────────────────────────────────────────────

    #[test]
    fn test_approve_updates_status() {
        let dir = temp_repo();
        let mut context = make_context(dir.path());
        let artifact_id = "approve-test-id".to_string();
        let now = Utc::now();
        context.artifacts.push(ArtifactRef {
            id: artifact_id.clone(),
            kind: "suggested-patch".to_string(),
            title: "Patch".to_string(),
            path: PathBuf::from("patch.md"),
            status: "awaiting_approval".to_string(),
            requires_approval: true,
            created_at: now,
            provenance: ArtifactProvenance {
                generator: "repo_workbench".to_string(),
                generation_mode: "deterministic_static_analysis".to_string(),
                model_invoked: false,
                provider: None,
                model: None,
                provider_kind: None,
                local_provider: None,
                work_id: "test-id".to_string(),
                artifact_type: "suggested-patch".to_string(),
                created_at: now,
            },
        });

        let artifact = context
            .artifacts
            .iter_mut()
            .find(|a| a.id == artifact_id)
            .unwrap();
        artifact.status = "approved".to_string();
        context.decisions.push(DecisionRecord {
            id: "dec-1".to_string(),
            artifact_id: artifact_id.clone(),
            decision: "approved_for_future_application".to_string(),
            approved: true,
            created_at: Utc::now(),
        });
        context.status = "approved".to_string();
        context.phase = "ready_to_apply".to_string();

        assert_eq!(artifact.status, "approved");
        assert_eq!(context.status, "approved");
        assert_eq!(context.decisions.len(), 1);
        assert!(context.decisions[0].approved);
    }

    // ── title_from_goal ───────────────────────────────────────────────────

    #[test]
    fn test_title_from_goal_short() {
        let title = title_from_goal("Hello");
        assert_eq!(title, "Hello");
    }

    #[test]
    fn test_title_from_goal_truncates_long() {
        let long = "a".repeat(100);
        let title = title_from_goal(&long);
        assert_eq!(title.len(), 51);
        assert!(title.ends_with("..."));
    }

    // ── looks_like_hardcoded_secret ───────────────────────────────────────

    #[test]
    fn test_looks_like_hardcoded_secret_detects() {
        assert!(looks_like_hardcoded_secret("let api_key = \"xxx\""));
        assert!(looks_like_hardcoded_secret("password = \"hunter2\""));
        assert!(looks_like_hardcoded_secret("secret = \"s3cr3t\""));
        assert!(looks_like_hardcoded_secret("token = \"abc\""));
        assert!(looks_like_hardcoded_secret("apikey = \"xyz\""));
    }

    #[test]
    fn test_looks_like_hardcoded_secret_ignores_env() {
        assert!(!looks_like_hardcoded_secret("api_key = env(\"VAR\")"));
        assert!(!looks_like_hardcoded_secret("password = \"example\""));
        assert!(!looks_like_hardcoded_secret("fn main() {}"));
    }

    // ── normalize_repo_path ───────────────────────────────────────────────

    #[test]
    fn test_normalize_repo_path_absolute() {
        let dir = temp_repo();
        let result = normalize_repo_path(dir.path());
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_normalize_repo_path_nonexistent() {
        let result = normalize_repo_path(Path::new("/nonexistent/path/12345"));
        assert!(result.is_err());
    }

    // ── store_root / paths ────────────────────────────────────────────────

    #[test]
    fn test_store_root_path() {
        let p = Path::new("/repo");
        let root = store_root(p);
        assert_eq!(root, Path::new("/repo/.prometheos-lite/workbench"));
    }

    #[test]
    fn test_contexts_dir_path() {
        let p = Path::new("/repo");
        assert_eq!(
            contexts_dir(p),
            Path::new("/repo/.prometheos-lite/workbench/contexts")
        );
    }

    #[test]
    fn test_artifacts_dir_path() {
        let p = Path::new("/repo");
        assert_eq!(
            artifacts_dir(p, "wid"),
            Path::new("/repo/.prometheos-lite/workbench/artifacts/wid")
        );
    }

    #[test]
    fn test_memory_dir_path() {
        let p = Path::new("/repo");
        assert_eq!(
            memory_dir(p),
            Path::new("/repo/.prometheos-lite/workbench/memory")
        );
    }

    // ── provenance ──────────────────────────────────────────────────────────

    #[test]
    fn test_write_artifact_includes_provenance() {
        let dir = temp_repo();
        let context = make_context(dir.path());
        let artifact = write_artifact(
            &context,
            "risk-report",
            "Test Report",
            "ready",
            false,
            "# Report",
        )
        .unwrap();
        assert_eq!(artifact.provenance.generator, "repo_workbench");
        assert_eq!(
            artifact.provenance.generation_mode,
            "deterministic_static_analysis"
        );
        assert!(!artifact.provenance.model_invoked);
        assert!(artifact.provenance.provider.is_none());
        assert!(artifact.provenance.model.is_none());
        assert_eq!(artifact.provenance.work_id, "test-id");
        assert_eq!(artifact.provenance.artifact_type, "risk-report");
    }

    #[test]
    fn test_artifact_provenance_deterministic_constructor() {
        let provenance = ArtifactProvenance::deterministic("test-work", "risk-report");
        assert_eq!(provenance.generator, "repo_workbench");
        assert!(!provenance.model_invoked);
        assert!(provenance.provider.is_none());
        assert!(provenance.model.is_none());
        assert_eq!(provenance.work_id, "test-work");
        assert_eq!(provenance.artifact_type, "risk-report");
    }

    #[test]
    fn test_provenance_is_serialized_in_artifact_ref() {
        let now = Utc::now();
        let artifact = ArtifactRef {
            id: "test-id".to_string(),
            kind: "risk-report".to_string(),
            title: "Test".to_string(),
            path: PathBuf::from("/tmp/test.md"),
            status: "ready".to_string(),
            requires_approval: false,
            created_at: now,
            provenance: ArtifactProvenance {
                generator: "repo_workbench".to_string(),
                generation_mode: "deterministic_static_analysis".to_string(),
                model_invoked: false,
                provider: None,
                model: None,
                provider_kind: None,
                local_provider: None,
                work_id: "wid".to_string(),
                artifact_type: "risk-report".to_string(),
                created_at: now,
            },
        };
        let json = serde_json::to_string(&artifact).unwrap();
        assert!(json.contains("provenance"));
        assert!(json.contains("deterministic_static_analysis"));
        assert!(json.contains("\"model_invoked\":false"));
        let deserialized: ArtifactRef = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.provenance.model_invoked);
        assert!(deserialized.provenance.provider.is_none());
    }
}
