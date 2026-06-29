//! Local repo workbench MVP command.
//!
//! Thin CLI wrapper around `prometheos_lite::repo_workbench` service functions.

use anyhow::Result;
use clap::{Parser, Subcommand};
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

impl RepoWorkbenchCommand {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            RepoWorkbenchSubcommand::Create {
                repo,
                goal,
                mode,
                title,
            } => {
                let context =
                    repo_workbench::create_repo_workbench_context(&repo, &goal, &mode, title)?;

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
