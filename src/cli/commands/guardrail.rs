//! Guardrail CLI commands for managing interrupts, trust, and outbox

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum GuardrailCommand {
    /// List pending interrupts
    InterruptList {
        /// Run ID to filter interrupts
        #[arg(long)]
        run_id: Option<String>,
    },
    /// Approve an interrupt with a decision
    InterruptApprove {
        /// Interrupt ID to approve
        interrupt_id: String,
        /// Decision JSON
        #[arg(long)]
        decision: String,
    },
    /// Deny an interrupt
    InterruptDeny {
        /// Interrupt ID to deny
        interrupt_id: String,
    },
    /// List trust policies
    TrustList,
    /// Set trust level for a source
    TrustSet {
        /// Source identifier
        source: String,
        /// Trust level (trusted, local, community, external, untrusted)
        #[arg(long)]
        level: String,
    },
    /// List outbox entries
    OutboxList {
        /// Run ID to filter outbox entries
        #[arg(long)]
        run_id: Option<String>,
    },
}

#[derive(Debug, Parser)]
pub struct GuardrailCmd {
    #[command(subcommand)]
    command: GuardrailCommand,
}

impl GuardrailCmd {
    pub async fn execute(self) -> Result<()> {
        let GuardrailCmd { command } = self;
        match command {
            GuardrailCommand::InterruptList { run_id } => {
                let cmd = GuardrailCmd { command: GuardrailCommand::InterruptList { run_id } };
                cmd.interrupt_list().await
            }
            GuardrailCommand::InterruptApprove {
                interrupt_id,
                decision,
            } => {
                let cmd = GuardrailCmd {
                    command: GuardrailCommand::InterruptApprove {
                        interrupt_id,
                        decision,
                    },
                };
                cmd.interrupt_approve(interrupt_id, decision).await
            }
            GuardrailCommand::InterruptDeny { interrupt_id } => {
                let cmd = GuardrailCmd {
                    command: GuardrailCommand::InterruptDeny { interrupt_id },
                };
                cmd.interrupt_deny(interrupt_id).await
            }
            GuardrailCommand::TrustList => {
                let cmd = GuardrailCmd {
                    command: GuardrailCommand::TrustList,
                };
                cmd.trust_list().await
            }
            GuardrailCommand::TrustSet { source, level } => {
                let cmd = GuardrailCmd {
                    command: GuardrailCommand::TrustSet { source, level },
                };
                cmd.trust_set(source, level).await
            }
            GuardrailCommand::OutboxList { run_id } => {
                let cmd = GuardrailCmd {
                    command: GuardrailCommand::OutboxList { run_id },
                };
                cmd.outbox_list().await
            }
        }
    }

    async fn interrupt_list(self) -> Result<()> {
        let db_path = ".prometheos/runs.db";
        if !PathBuf::from(db_path).exists() {
            let interrupts = json!({
                "interrupts": [],
                "message": "Database not found"
            });
            println!("{}", serde_json::to_string_pretty(&interrupts)?);
            return Ok(());
        }

        let db = crate::db::repository::Db::new(db_path)?;
        let run_id_filter = match self.command {
            GuardrailCommand::InterruptList { run_id } => run_id,
            _ => None,
        };

        let interrupts = if let Some(run_id) = run_id_filter {
            db.list_pending_interrupts(&run_id)?
        } else {
            // List all pending interrupts across all runs
            db.list_all_pending_interrupts()?
        };

        let result = json!({
            "interrupts": interrupts.iter().map(|i| {
                json!({
                    "id": i.id,
                    "run_id": i.run_id,
                    "node_id": i.node_id,
                    "reason": i.reason,
                    "status": i.status,
                    "created_at": i.created_at.to_rfc3339()
                })
            }).collect::<Vec<_>>(),
            "count": interrupts.len()
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    async fn interrupt_approve(self, interrupt_id: String, decision: String) -> Result<()> {
        let db_path = ".prometheos/runs.db";
        if !PathBuf::from(db_path).exists() {
            anyhow::bail!("Database not found");
        }

        let db = crate::db::repository::Db::new(db_path)?;
        db.approve_interrupt(&interrupt_id, &decision)?;

        let result = json!({
            "interrupt_id": interrupt_id,
            "decision": decision,
            "status": "approved"
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    async fn interrupt_deny(self, interrupt_id: String) -> Result<()> {
        let db_path = ".prometheos/runs.db";
        if !PathBuf::from(db_path).exists() {
            anyhow::bail!("Database not found");
        }

        let db = crate::db::repository::Db::new(db_path)?;
        db.deny_interrupt(&interrupt_id)?;

        let result = json!({
            "interrupt_id": interrupt_id,
            "status": "denied"
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    async fn trust_list(self) -> Result<()> {
        let db_path = ".prometheos/runs.db";
        let mut policies = vec![
            {"source": "builtin", "level": "Trusted", "persistent": false},
            {"source": "local", "level": "Local", "persistent": false},
            {"source": "community", "level": "Community", "persistent": false},
            {"source": "external", "level": "External", "persistent": false},
            {"source": "unknown", "level": "Untrusted", "persistent": false},
        ];

        if std::path::Path::new(db_path).exists() {
            if let Ok(db) = crate::db::repository::Db::new(db_path) {
                if let Ok(db_policies) = crate::db::repository::TrustPolicyOperations::list_trust_policies(&db) {
                    policies = db_policies.iter().map(|p| {
                        json!({
                            "source": p.source,
                            "level": p.trust_level,
                            "require_approval": p.require_approval,
                            "persistent": true,
                            "updated_at": p.updated_at.to_rfc3339()
                        })
                    }).collect();
                }
            }
        }

        let result = json!({
            "policies": policies,
            "count": policies.len()
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    async fn trust_set(self, source: String, level: String) -> Result<()> {
        let db_path = ".prometheos/runs.db";
        if !std::path::Path::new(db_path).exists() {
            anyhow::bail!("Database not found");
        }

        let db = crate::db::repository::Db::new(db_path)?;
        
        // Determine if approval is required based on trust level
        let require_approval = matches!(level.to_lowercase().as_str(), "untrusted" | "external");
        
        let policy = crate::db::repository::TrustPolicyOperations::create_or_update_trust_policy(
            &db,
            &source,
            &level,
            require_approval,
        )?;

        let result = json!({
            "source": policy.source,
            "level": policy.trust_level,
            "require_approval": policy.require_approval,
            "updated_at": policy.updated_at.to_rfc3339(),
            "status": "updated"
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    async fn outbox_list(self) -> Result<()> {
        let db_path = ".prometheos/runs.db";
        if !PathBuf::from(db_path).exists() {
            let outbox = json!({
                "entries": [],
                "message": "Database not found"
            });
            println!("{}", serde_json::to_string_pretty(&outbox)?);
            return Ok(());
        }

        let db = crate::db::repository::Db::new(db_path)?;
        let run_id_filter = match self.command {
            GuardrailCommand::OutboxList { run_id } => run_id,
            _ => None,
        };

        let entries = if let Some(run_id) = run_id_filter {
            db.list_pending_outbox(&run_id)?
        } else {
            db.list_all_pending_outbox()?
        };

        let result = json!({
            "entries": entries.iter().map(|e| {
                json!({
                    "id": e.id,
                    "run_id": e.run_id,
                    "node_id": e.node_id,
                    "tool_name": e.tool_name,
                    "status": e.status,
                    "created_at": e.created_at.to_rfc3339()
                })
            }).collect::<Vec<_>>(),
            "count": entries.len()
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}
