//! Local repo workbench MVP command.
//!
//! Thin CLI wrapper around `prometheos_lite::repo_workbench` service functions.

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json;
use std::path::PathBuf;

use prometheos_lite::repo_workbench;

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
        /// Output JSON for machine-readable consumption
        #[arg(long)]
        json: bool,
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
    /// Survey a repository read-only and produce a reviewable onboarding
    /// proposal (detection confidence + evidence; never modifies project
    /// files; apply requires explicit approval)
    Onboarding {
        /// Repository root to survey
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Write the proposal manifest to this path (inside the tool's
        /// own .prometheos-lite store; project files are never touched)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Apply a human-edited proposal manifest after review
        #[arg(long)]
        apply: Option<PathBuf>,
        /// Record approval for the applied manifest (required with --apply)
        #[arg(long)]
        approve: bool,
        /// Output the manifest as JSON
        #[arg(long)]
        json: bool,
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

impl RepoWorkbenchCommand {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            RepoWorkbenchSubcommand::Create {
                repo,
                goal,
                mode,
                title,
                json,
            } => {
                let context =
                    repo_workbench::create_repo_workbench_context(&repo, &goal, &mode, title)?;

                if json {
                    let output = serde_json::json!({
                        "work_id": context.id,
                        "title": context.title,
                        "repo": context.repo_path.display().to_string(),
                        "mode": context.mode,
                        "status": context.status,
                        "project_type": context.repo_summary.project_type,
                        "candidate_files": context.repo_summary.candidate_files.len(),
                        "next": format!("prometheos repo run {}", context.id),
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    println!("Created Repo Workbench WorkContext");
                    println!("  ID: {}", context.id);
                    println!("  Title: {}", context.title);
                    println!("  Repo: {}", context.repo_path.display());
                    println!("  Mode: {}", context.mode);
                    println!("  Project type: {}", context.repo_summary.project_type);
                    println!(
                        "  Candidate files: {}",
                        context.repo_summary.candidate_files.len()
                    );
                    println!("  Next: prometheos repo run {}", context.id);
                }
            }
            RepoWorkbenchSubcommand::Run { id } => {
                let mut context = repo_workbench::load_context(&id)?;
                repo_workbench::run_repo_workbench_context(&mut context)?;

                let risk_report = context.artifacts.iter().find(|a| a.kind == "risk-report");
                let patch_artifact = context
                    .artifacts
                    .iter()
                    .find(|a| a.kind == "suggested-patch");

                println!("Repo Workbench run complete");
                println!("  WorkContext: {}", context.id);
                println!("  Status: {}", context.status);
                println!(
                    "  Files considered: {}",
                    context.repo_summary.candidate_files.len()
                );
                println!("  Findings: {}", context.artifacts.len());
                if let Some(report) = risk_report {
                    println!("  Risk report: {}", report.path.display());
                }
                if let Some(patch) = patch_artifact {
                    println!("  Suggested patch plan: {}", patch.path.display());
                }
                if let Some(next) = &context.next_action {
                    println!("  Next: {}", next);
                }
            }
            RepoWorkbenchSubcommand::Status { id } => {
                let context = repo_workbench::load_context(&id)?;
                repo_workbench::print_status(&context);
            }
            RepoWorkbenchSubcommand::Artifacts { id } => {
                let context = repo_workbench::load_context(&id)?;
                println!("Artifacts for {}:", context.id);
                if context.artifacts.is_empty() {
                    println!(
                        "  No artifacts yet. Run `prometheos repo run {}` first.",
                        context.id
                    );
                }
                for artifact in repo_workbench::get_artifacts(&context) {
                    println!("  {}", artifact.id);
                    println!("    Title: {}", artifact.title);
                    println!("    Kind: {}", artifact.kind);
                    println!("    Status: {}", artifact.status);
                    println!("    Requires approval: {}", artifact.requires_approval);
                    println!("    Path: {}", artifact.path.display());
                }
            }
            RepoWorkbenchSubcommand::Approve {
                artifact_id,
                work_id,
            } => {
                let mut context = if let Some(ref work_id) = work_id {
                    repo_workbench::load_context(work_id)?
                } else {
                    repo_workbench::find_context_by_artifact(&artifact_id)?
                };

                repo_workbench::approve_artifact(&mut context, &artifact_id)?;

                println!("Approval recorded");
                println!("  WorkContext: {}", context.id);
                println!("  Artifact: {}", artifact_id);
                println!("  Safety: no repository source files were modified");
                println!("  Next: prometheos repo continue {}", context.id);
            }
            RepoWorkbenchSubcommand::Continue { id } => {
                let context = repo_workbench::load_context(&id)?;
                println!("Continuing Repo Workbench WorkContext");
                repo_workbench::print_status(&context);
                println!();
                println!("Memory:");
                println!("{}", repo_workbench::load_memory(&context)?);
                if let Some(next) = &context.next_action {
                    println!();
                    println!("Recommended next action: {}", next);
                }
            }
            RepoWorkbenchSubcommand::Memory { command } => match command {
                MemorySubcommand::Show { id } => {
                    let context = repo_workbench::load_context(&id)?;
                    println!("{}", repo_workbench::load_memory(&context)?);
                }
            },
            RepoWorkbenchSubcommand::Onboarding {
                repo,
                out,
                apply,
                approve,
                json,
            } => onboarding::execute(&repo, out, apply, approve, json)?,
        }

        Ok(())
    }
}

/// E6/I05 (#134): onboarding command implementation. Survey is
/// read-only; the proposal is a suggestion that enables no authority;
/// apply requires explicit approval and writes only inside the tool's
/// own `.prometheos-lite` store — project files are never touched.
mod onboarding {
    use anyhow::{Context, Result, bail};
    use std::path::{Path, PathBuf};

    use prometheos_lite::workflow::onboarding::{
        OnboardingManifestV1, apply_user_manifest, manifest_from_survey, survey_repository,
    };
    use prometheos_lite::workflow::soma::Diagnostic;

    fn print_diagnostics(diags: &[Diagnostic]) {
        for d in diags {
            println!("  [{}] {}", d.code, d.message);
            if let Some(r) = &d.remediation {
                if let Some(action) = &r.action {
                    println!("    action: {action}");
                }
                println!("    {}", r.summary);
            }
        }
    }

    fn print_manifest_human(manifest: &OnboardingManifestV1) {
        println!("Onboarding proposal ({})", manifest.schema_version);
        println!("  Repository: {}", manifest.survey.repo_root);
        println!(
            "  Revision: {} (dirty: {})",
            manifest.survey.revision, manifest.survey.dirty
        );
        println!();
        for row in &manifest.survey.rows {
            println!(
                "  {:<20} [{}] {}",
                row.field,
                row.confidence,
                serde_json::to_string(&row.value).unwrap_or_default()
            );
            println!("      evidence: {}", row.evidence.join(", "));
        }
        println!();
        println!("Review this proposal, edit the proposed configuration if");
        println!("needed, then apply with explicit approval:");
        println!("  prometheos work onboarding --apply <path> --approve");
    }

    pub fn execute(
        repo: &Path,
        out: Option<PathBuf>,
        apply: Option<PathBuf>,
        approve: bool,
        json: bool,
    ) -> Result<()> {
        // Apply path: read the human-edited manifest, re-survey, apply.
        if let Some(path) = apply {
            let text = std::fs::read_to_string(&path).with_context(|| {
                format!(
                    "reading {} (generate one with `prometheos work onboarding`)",
                    path.display()
                )
            })?;
            let user: OnboardingManifestV1 = serde_json::from_str(&text)
                .with_context(|| format!("parsing {} as an onboarding manifest", path.display()))?;
            // The survey inside the manifest binds the repository it was
            // generated from; re-survey that root.
            let root = if repo == Path::new(".") {
                PathBuf::from(&user.survey.repo_root)
            } else {
                repo.to_path_buf()
            };
            let fresh = match survey_repository(&root) {
                Ok(s) => s,
                Err(diags) => {
                    println!("Onboarding refused — repository no longer surveys cleanly:");
                    print_diagnostics(&diags);
                    bail!("onboarding apply refused");
                }
            };
            // Regenerate the detection proposal bound to the user's
            // survey (same detection run as the original proposal).
            let proposal = manifest_from_survey(&fresh);
            match apply_user_manifest(&proposal, &user, &fresh, approve) {
                Ok(applied) => {
                    let applied_path = root.join(".prometheos-lite").join("onboarding.json");
                    if let Some(parent) = applied_path.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating {}", parent.display()))?;
                    }
                    std::fs::write(&applied_path, serde_json::to_string_pretty(&applied)?)
                        .with_context(|| format!("writing {}", applied_path.display()))?;
                    println!("Onboarding applied — approval recorded");
                    println!("  Manifest: {}", applied_path.display());
                    println!("  Overrides recorded: {}", applied.overrides.len());
                    for over in &applied.overrides {
                        println!(
                            "    {} -> {} (detected: {})",
                            over.field,
                            serde_json::to_string(&over.user).unwrap_or_default(),
                            serde_json::to_string(&over.detected).unwrap_or_default()
                        );
                    }
                    println!("  Safety: no project files were modified");
                }
                Err(diags) => {
                    println!("Onboarding apply refused:");
                    print_diagnostics(&diags);
                    bail!("onboarding apply refused");
                }
            }
            return Ok(());
        }

        // Survey path: read-only detection + reviewable proposal.
        let survey = match survey_repository(repo) {
            Ok(s) => s,
            Err(diags) => {
                println!("Onboarding refused — unsupported or conflicting repository setup:");
                print_diagnostics(&diags);
                bail!("onboarding refused");
            }
        };
        let manifest = manifest_from_survey(&survey);
        if json {
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        } else {
            print_manifest_human(&manifest);
        }
        let out_path = out.unwrap_or_else(|| {
            repo.join(".prometheos-lite")
                .join("onboarding-proposal.json")
        });
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&out_path, serde_json::to_string_pretty(&manifest)?)
            .with_context(|| format!("writing {}", out_path.display()))?;
        if json {
            eprintln!("proposal written to {}", out_path.display());
        } else {
            println!();
            println!("Proposal written to {}", out_path.display());
            println!("  (inside the tool's own store; no project files were modified)");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    use prometheos_lite::repo_workbench;

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

    #[test]
    fn test_work_create_repo_delegates() {
        let dir = temp_repo();
        write_file(dir.path(), "Cargo.toml", "[package]\nname = \"test\"\n");
        write_file(dir.path(), "src/main.rs", "fn main() {}");

        let context = repo_workbench::create_repo_workbench_context(
            dir.path(),
            "Find issues",
            "review",
            None,
        )
        .expect("create should succeed");

        assert_eq!(context.status, "draft");
        assert_eq!(context.phase, "intake");
        assert_eq!(context.repo_path, dir.path().canonicalize().unwrap());
        assert!(context.id.len() > 10);
    }

    #[test]
    fn test_work_create_repo_with_custom_title() {
        let dir = temp_repo();
        write_file(dir.path(), "Cargo.toml", "[package]\nname = \"test\"\n");

        let context = repo_workbench::create_repo_workbench_context(
            dir.path(),
            "Find issues",
            "review",
            Some("Custom Title".to_string()),
        )
        .expect("create should succeed");

        assert_eq!(context.title, "Custom Title");
    }

    #[test]
    fn test_context_exists_returns_false_for_bogus_id() {
        assert!(!repo_workbench::repo_workbench_context_exists(
            "nonexistent-id-12345"
        ));
    }

    #[test]
    fn test_find_context_by_artifact_fails_for_bogus_id() {
        let result = repo_workbench::find_context_by_artifact("bogus-artifact-id");
        assert!(result.is_err());
    }
}
