//! Generic governed node execution interface (issue #117, slice 1).
//!
//! Wraps node execution behind one mandatory ordered gate pipeline. Every
//! effectful path passes through the same nine gates:
//!
//! 1. resolve a declared capability;
//! 2. validate typed arguments and compatibility;
//! 3. authorize against effective authority/policy;
//! 4. apply constraint declarations (resource/scope);
//! 5. execute;
//! 6. verify the result and resulting state;
//! 7. redact protected material;
//! 8. retain required evidence;
//! 9. append the attributable result after artifacts are durable.
//!
//! Delegation (code mode, nested nodes, tool bridges, external adapters)
//! plugs in at gate 5 but cannot skip or reorder gates: the stage machine
//! rejects illegal transitions and the runner is the only entry point.
//!
//! Lite-owned types here are runtime continuity artifacts; where they
//! mirror published SOMA++ families (`NodeManifestV1`, `NodeResultV1`,
//! outcome variants) the mapping is explicit via `node_contracts`.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::workflow::node_contracts::{NodeManifestV1, NodeResultV1};
use crate::workflow::policy::{EffectiveExecutionSnapshotV1, resolve_effective};
use crate::workflow::redaction::Redactor;

/// The nine ordered gates, as an explicit state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GateStage {
    CapabilityResolved,
    ArgsValidated,
    Authorized,
    Constrained,
    Executed,
    Verified,
    Redacted,
    EvidenceRetained,
    Journaled,
}

impl GateStage {
    /// The next legal stage; terminal at Journaled.
    pub const fn next(self) -> Option<GateStage> {
        match self {
            GateStage::CapabilityResolved => Some(GateStage::ArgsValidated),
            GateStage::ArgsValidated => Some(GateStage::Authorized),
            GateStage::Authorized => Some(GateStage::Constrained),
            GateStage::Constrained => Some(GateStage::Executed),
            GateStage::Executed => Some(GateStage::Verified),
            GateStage::Verified => Some(GateStage::Redacted),
            GateStage::Redacted => Some(GateStage::EvidenceRetained),
            GateStage::EvidenceRetained => Some(GateStage::Journaled),
            GateStage::Journaled => None,
        }
    }
}

/// Transition law for the gate pipeline: only the immediate next stage is
/// legal; anything else (skip-ahead, backwards, repeat-advance) is rejected.
pub fn validate_gate_transition(from: GateStage, to: GateStage) -> Result<()> {
    if from.next() == Some(to) {
        Ok(())
    } else {
        bail!("illegal gate transition {from:?} -> {to:?}: gates must run in order");
    }
}

// ---------------------------------------------------------------------------
// Capability registry
// ---------------------------------------------------------------------------

/// A declared capability: typed handler plus its declared argument keys.
pub struct Capability {
    /// Required top-level argument names (gate 2 validation).
    pub required_args: &'static [&'static str],
    #[allow(clippy::type_complexity)]
    pub handler: Box<dyn Fn(&serde_json::Value) -> Result<String> + Send + Sync>,
}

impl Capability {
    pub fn deterministic(
        required_args: &'static [&'static str],
        handler: impl Fn(&serde_json::Value) -> Result<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            required_args,
            handler: Box::new(handler),
        }
    }
}

/// Registry of declared capabilities (gate 1 resolution source).
#[derive(Default)]
pub struct CapabilityRegistry {
    entries: BTreeMap<String, Capability>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn declare(&mut self, name: impl Into<String>, capability: Capability) {
        self.entries.insert(name.into(), capability);
    }

    /// Gate 1: resolve a declared capability; unknown names fail closed.
    pub fn resolve(&self, name: &str) -> Result<&Capability> {
        self.entries
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("SOMA-AUTH-0005: capability {name:?} not declared"))
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// One durable journal entry (gate 9), digest-chained to the previous.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalEntryV1 {
    pub sequence: u64,
    pub identity_key: String,
    pub stage: String,
    pub detail_digest: String,
    pub prev_entry_digest: Option<String>,
    pub entry_digest: String,
}

/// Terminal outcome of a governed node run.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunOutcome {
    pub result: NodeResultV1,
    /// Digest-bound evidence reference: the journaled terminal entry whose
    /// chain root is this run's identity key.
    pub evidence_entry: JournalEntryV1,
}

/// Request to execute one governed node.
pub struct NodeRunRequest<'a> {
    pub manifest: &'a NodeManifestV1,
    pub local_restrictions: &'a crate::workflow::policy::LocalRestrictions,
    pub capability: String,
    pub args: serde_json::Value,
    /// Stable identity for idempotency (gate law: same identity => same
    /// outcome without re-execution).
    pub idempotency_key: String,
    /// Known secrets for gate 7 redaction.
    pub known_secrets: Vec<String>,
}

/// The generic governed node runner.
#[derive(Default)]
pub struct NodeRunner {
    registry: CapabilityRegistry,
    completed: BTreeMap<String, NodeRunOutcome>,
    journal: Vec<JournalEntryV1>,
}

impl NodeRunner {
    pub fn new(registry: CapabilityRegistry) -> Self {
        Self {
            registry,
            completed: BTreeMap::new(),
            journal: Vec::new(),
        }
    }

    pub fn journal(&self) -> &[JournalEntryV1] {
        &self.journal
    }

    /// Execute through all nine gates. Idempotent on `idempotency_key`.
    pub fn execute(&mut self, req: NodeRunRequest<'_>) -> Result<NodeRunOutcome> {
        if let Some(cached) = self.completed.get(&req.idempotency_key) {
            return Ok(cached.clone());
        }
        let mut stage = GateStage::CapabilityResolved;

        // Gate 1: resolve declared capability.
        let capability = self.registry.resolve(&req.capability)?;

        // Gate 2: validate typed arguments / compatibility.
        if let Some(obj) = req.args.as_object() {
            for key in capability.required_args {
                if !obj.contains_key(*key) {
                    bail!("SOMA-CMP-0003: missing required argument {key:?}");
                }
            }
        } else if !capability.required_args.is_empty() {
            bail!("SOMA-CMP-0003: arguments must be an object");
        }
        advance(&mut stage, GateStage::ArgsValidated)?;

        // Gate 3: authorize against effective authority/policy.
        let snapshot: EffectiveExecutionSnapshotV1 = resolve_effective(
            req.manifest,
            req.local_restrictions,
            crate::workflow::evaluate::now_iso(),
        )?;
        let writable_scope = req.manifest.writable_scopes.first().cloned();
        let provider: Option<String> = None;
        if let Err(violation) = crate::workflow::policy::enforce_before_effects(
            &snapshot,
            /* attempt (1-based) */ 1,
            writable_scope.as_deref(),
            provider.as_deref(),
        ) {
            bail!("SOMA-AUTH-0003/0009: policy rejected before effects: {violation}");
        }
        advance(&mut stage, GateStage::Authorized)?;

        // Gate 4: constraint declarations present (structural). Deep
        // sandboxed enforcement remains the orchestrator's resource layer
        // (evaluate/validation.rs); the runner refuses to proceed when the
        // manifest carries no retry budget because every effectful run must
        // be bounded by declaration.
        if req.manifest.retry.max_attempts == 0 {
            bail!("SOMA-AUTH-0009: unbounded retry budget refused");
        }
        advance(&mut stage, GateStage::Constrained)?;

        // Gate 5: execute (the ONLY delegation point).
        let raw_output = (capability.handler)(&req.args)?;
        advance(&mut stage, GateStage::Executed)?;

        // Gate 6: verify result/state — non-empty output.
        if raw_output.trim().is_empty() {
            bail!("SOMA-CMP-0003: empty node output failed verification");
        }
        let outcome_kind = crate::workflow::node_contracts::OutcomeKind::Completed;
        advance(&mut stage, GateStage::Verified)?;

        // Gate 7: redact protected material.
        let redactor = Redactor::with_known_secrets(Redactor::new(), &req.known_secrets);
        let redacted_output = redactor.redact(&raw_output);
        advance(&mut stage, GateStage::Redacted)?;

        // Gate 8: retain required evidence (digest-bound artifact first).
        let artifact_digest =
            crate::workflow::artifact_integrity::sha256_hex(redacted_output.as_bytes());
        if artifact_digest.len() != 64 {
            bail!("evidence digest malformed");
        }
        advance(&mut stage, GateStage::EvidenceRetained)?;

        // Gate 9: append attributable result in durable order.
        let now = crate::workflow::evaluate::now_iso();
        let result = NodeResultV1 {
            schema_version: crate::workflow::node_contracts::NODE_CONTRACT_VERSION.into(),
            node_id: req.manifest.node_id.clone(),
            outcome: outcome_kind,
            reason: String::new(),
            outputs: req.manifest.outputs.clone(),
            evidence_refs: vec![crate::workflow::memory_contracts::EvidenceReferenceV1 {
                id: format!("ev-{}", req.idempotency_key),
                event_digest: String::new(), // sealed below after journaling
                artifact_digest: artifact_digest.clone(),
                artifact_kind: "node-output".into(),
                produced_by: "lite.node-runner".into(),
                produced_at: Some(now.clone()),
            }],
            memory_reads_executed: 0,
            memory_writes_executed: 0,
            work_state_ref: req.manifest.work_state_ref.clone(),
            failure_classification: None,
            started_at: now.clone(),
            completed_at: now,
            result_digest: String::new(),
        };
        advance(&mut stage, GateStage::Journaled)?;
        let prev = self.journal.last().map(|e| e.entry_digest.clone());
        let detail_digest = result.compute_digest()?;
        let mut entry = JournalEntryV1 {
            sequence: self.journal.len() as u64,
            identity_key: req.idempotency_key.clone(),
            stage: format!("{stage:?}"),
            detail_digest,
            prev_entry_digest: prev,
            entry_digest: String::new(),
        };
        entry.entry_digest = journal_entry_digest(&entry);
        self.journal.push(entry.clone());

        // Seal the evidence reference to the journal entry, then seal the
        // result digest itself so terminal results always reference durable
        // evidence (acceptance: terminal => durable evidence reference).
        let mut final_result = result;
        if let Some(ev) = final_result.evidence_refs.first_mut() {
            ev.event_digest = entry.entry_digest.clone();
        }
        final_result.result_digest = final_result.compute_digest()?;

        let outcome = NodeRunOutcome {
            result: final_result,
            evidence_entry: entry,
        };
        self.completed
            .insert(req.idempotency_key.clone(), outcome.clone());
        Ok(outcome)
    }
}

fn advance(current: &mut GateStage, to: GateStage) -> Result<()> {
    validate_gate_transition(*current, to)?;
    *current = to;
    Ok(())
}

fn journal_entry_digest(entry: &JournalEntryV1) -> String {
    let v = serde_json::to_value(entry).expect("journal entry serializes");
    crate::workflow::soma::canonical_digest(&v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::policy::LocalRestrictions;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn manifest() -> NodeManifestV1 {
        NodeManifestV1::parse_json(
            r#"{
            "schemaVersion": "1.0.0",
            "nodeId": "node-1",
            "purpose": "test node",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "tokenBudget": 100,
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        }"#,
        )
        .unwrap()
    }

    fn restrictions() -> LocalRestrictions {
        LocalRestrictions {
            readable_scopes: vec!["repo://x".into()],
            writable_scopes: vec!["work://y".into()],
            token_budget_ceiling: Some(500),
            denied_providers: vec![],
            forbidden_paths: vec![],
            max_attempts: 3,
            escalation_target: "human".into(),
        }
    }

    fn runner_with_counter(counter: Arc<AtomicUsize>) -> NodeRunner {
        let mut registry = CapabilityRegistry::new();
        registry.declare(
            "echo",
            Capability::deterministic(&["text"], move |args| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(format!(
                    "echo: {}",
                    args.get("text").and_then(|t| t.as_str()).unwrap_or("")
                ))
            }),
        );
        NodeRunner::new(registry)
    }

    fn request<'a>(
        manifest: &'a NodeManifestV1,
        local: &'a LocalRestrictions,
        key: &str,
        secrets: Vec<String>,
    ) -> NodeRunRequest<'a> {
        NodeRunRequest {
            manifest,
            local_restrictions: local,
            capability: "echo".into(),
            args: serde_json::json!({"text": format!("secret={}", secrets.first().cloned().unwrap_or_default())}),
            idempotency_key: key.into(),
            known_secrets: secrets,
        }
    }

    #[test]
    fn executes_through_all_gates_and_journals_in_order() {
        let m = manifest();
        let local = restrictions();
        let mut runner = runner_with_counter(Arc::new(AtomicUsize::new(0)));
        let outcome = runner
            .execute(request(&m, &local, "run-1", vec![]))
            .expect("clean run passes all nine gates");
        assert_eq!(
            outcome.result.outcome,
            crate::workflow::node_contracts::OutcomeKind::Completed
        );
        assert_eq!(runner.journal().len(), 1);
        let e = &runner.journal()[0];
        assert_eq!(e.sequence, 0);
        assert_eq!(e.stage, "Journaled");
        // Terminal results always reference durable evidence.
        let ev = outcome
            .result
            .evidence_refs
            .first()
            .expect("evidence bound");
        assert_eq!(ev.event_digest, e.entry_digest);
        assert!(!ev.artifact_digest.is_empty());
        assert_eq!(outcome.result.result_digest.len(), 64);
    }

    #[test]
    fn idempotent_for_same_identity() {
        let m = manifest();
        let local = restrictions();
        let counter = Arc::new(AtomicUsize::new(0));
        let mut runner = runner_with_counter(counter.clone());
        let a = runner
            .execute(request(&m, &local, "same-key", vec![]))
            .unwrap();
        let b = runner
            .execute(request(&m, &local, "same-key", vec![]))
            .unwrap();
        assert_eq!(a.result.result_digest, b.result.result_digest);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "handler must run exactly once"
        );
        assert_eq!(runner.journal().len(), 1, "no duplicate journal entries");
    }

    #[test]
    fn illegal_transitions_rejected() {
        assert!(
            validate_gate_transition(GateStage::CapabilityResolved, GateStage::Executed).is_err()
        );
        assert!(validate_gate_transition(GateStage::Executed, GateStage::Authorized).is_err());
        assert!(validate_gate_transition(GateStage::Journaled, GateStage::Journaled).is_err());
        assert!(validate_gate_transition(GateStage::Authorized, GateStage::Constrained).is_ok());
    }

    #[test]
    fn undeclared_capability_fails_closed_at_gate_1() {
        let m = manifest();
        let local = restrictions();
        let mut runner = runner_with_counter(Arc::new(AtomicUsize::new(0)));
        let mut req = request(&m, &local, "run-x", vec![]);
        req.capability = "ghost-cap".into();
        let err = runner.execute(req).unwrap_err().to_string();
        assert!(err.contains("SOMA-AUTH-0005"), "{err}");
        assert!(runner.journal().is_empty(), "failed runs never journal");
    }

    #[test]
    fn missing_args_fail_at_gate_2() {
        let m = manifest();
        let local = restrictions();
        let mut runner = runner_with_counter(Arc::new(AtomicUsize::new(0)));
        let mut req = request(&m, &local, "run-y", vec![]);
        req.args = serde_json::json!({"wrong": 1});
        let err = runner.execute(req).unwrap_err().to_string();
        assert!(err.contains("SOMA-CMP-0003"), "{err}");
    }

    #[test]
    fn scope_outside_policy_fails_at_gate_3_before_execution() {
        let m = manifest();
        let local = LocalRestrictions {
            readable_scopes: vec!["repo://x".into()],
            writable_scopes: vec!["other://z".into()], // no overlap with manifest
            token_budget_ceiling: None,
            denied_providers: vec![],
            forbidden_paths: vec![],
            max_attempts: 3,
            escalation_target: "human".into(),
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let mut runner = runner_with_counter(counter.clone());
        let err = runner
            .execute(request(&m, &local, "run-z", vec![]))
            .unwrap_err();
        assert!(
            err.to_string().contains("policy rejected before effects"),
            "{err}"
        );
        assert_eq!(counter.load(Ordering::SeqCst), 0, "handler never ran");
    }

    #[test]
    fn secrets_redacted_before_evidence_retention() {
        let m = manifest();
        let local = restrictions();
        let mut runner = runner_with_counter(Arc::new(AtomicUsize::new(0)));
        let outcome = runner
            .execute(request(&m, &local, "run-secret", vec!["hunter2".into()]))
            .unwrap();
        let serialized = serde_json::to_string(&outcome.result).unwrap();
        let journaled = serde_json::to_string(runner.journal()).unwrap();
        assert!(
            !serialized.contains("hunter2"),
            "secret leaked into evidence"
        );
        assert!(!journaled.contains("hunter2"), "secret leaked into journal");
        // Redaction happened BEFORE evidence retention: the retained
        // artifact digest is over the REDACTED bytes, not the raw output.
        let raw_digest = crate::workflow::artifact_integrity::sha256_hex(b"echo: secret=hunter2");
        let ev = outcome.result.evidence_refs.first().unwrap();
        assert_ne!(
            ev.artifact_digest, raw_digest,
            "retention used unredacted bytes"
        );
    }
}
