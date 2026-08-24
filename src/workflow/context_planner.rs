//! Execution-time ContextPlanner (#153): deterministic assembly of the final
//! ordered `ContextBundle` from one or more retrieval ports, under enforced
//! budgets, with a full planning audit.
//!
//! OWNERSHIP: Lite owns final context assembly. Backends (local repo index,
//! Mnemosyne adapters) only supply candidates; they never decide what the
//! model sees. Same inputs + same port results + same policy => the SAME
//! ordered bundle with the SAME digest.
//!
//! Privacy/provider policy binds upstream: ports already refuse unauthorized
//! scopes (typed errors), and denied providers are recorded in the audit for
//! evidence. The planner itself performs no provider invocation.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::{
    BackendKind, ContextBundle, MemoryQuery, MemoryRetrievalPort, OmittedEntry, OperationPolicy,
    assemble_context_bundle,
};

/// Model/harness capability profile: drives projection differences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProfileV1 {
    pub model_id: String,
    pub harness: String,
    /// Full model context window in tokens.
    pub context_window_tokens: u64,
    /// Tokens reserved for the model's response (excluded from context).
    pub reserved_output_tokens: u64,
}

/// Declared planner inputs (one attempt).
#[derive(Debug, Clone)]
pub struct PlannerInputsV1 {
    pub planner_id: String,
    pub query: MemoryQuery,
    /// Current repository revision for freshness classification.
    pub current_revision: Option<String>,
    pub profile: ModelProfileV1,
    /// Explicit budget override; defaults to window minus reserved output.
    pub token_budget_override: Option<u64>,
    /// Deterministic timestamp for reproducible bundles.
    pub executed_at: String,
}

/// Per-port execution status recorded in the audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortStatusV1 {
    pub port: String,
    pub backend: BackendKind,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub retrieved: usize,
}

/// Full planning audit (durable evidence).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningAuditV1 {
    pub schema_version: String,
    pub planner_id: String,
    pub model_id: String,
    pub effective_budget_tokens: u64,
    pub retrieved_total: usize,
    pub selected_total: usize,
    pub omitted_stale: usize,
    pub omitted_budget: usize,
    pub omitted_other: usize,
    pub ports: Vec<PortStatusV1>,
    pub digest_echo: String,
}

impl PlanningAuditV1 {
    pub fn parse_json(json: &str) -> Result<Self> {
        let a: Self = serde_json::from_str(json).context("failed to parse lite.planner audit")?;
        crate::workflow::memory_contracts::ensure_supported_contract_version(&a.schema_version)?;
        Ok(a)
    }
}

pub struct PlanOutcome {
    pub bundle: ContextBundle,
    pub audit: PlanningAuditV1,
}

fn effective_budget(inputs: &PlannerInputsV1) -> Result<u64> {
    if let Some(b) = inputs.token_budget_override {
        let window = inputs
            .profile
            .context_window_tokens
            .saturating_sub(inputs.profile.reserved_output_tokens);
        return Ok(b.min(window));
    }
    let eff = inputs
        .profile
        .context_window_tokens
        .saturating_sub(inputs.profile.reserved_output_tokens);
    if eff == 0 {
        bail_any("model profile leaves zero tokens for context");
    }
    Ok(eff)
}

fn bail_any(msg: &'static str) -> anyhow::Error {
    anyhow::anyhow!(msg)
}

/// Plan and assemble the final bundle from every available port.
///
/// Ports that fail with typed unavailability or staleness degrade to audit
/// entries + omitted reasons; they never abort the plan while at least one
/// port succeeds. If ALL ports fail, the last typed error propagates.
pub fn plan(inputs: &PlannerInputsV1, ports: &[&dyn MemoryRetrievalPort]) -> Result<PlanOutcome> {
    if ports.is_empty() {
        bail_any("no retrieval ports configured");
    }
    let budget = effective_budget(inputs)?;
    let mut merged: Vec<crate::workflow::memory_contracts::RawCandidate> = Vec::new();
    let mut omitted: Vec<OmittedEntry> = Vec::new();
    let mut statuses: Vec<PortStatusV1> = Vec::new();
    let mut last_err: Option<anyhow::Error> = None;

    for p in ports {
        match p.retrieve(&inputs.query) {
            Ok(rows) => {
                statuses.push(PortStatusV1 {
                    port: p.name().into(),
                    backend: p.backend(),
                    status: "ok".into(),
                    detail: None,
                    retrieved: rows.len(),
                });
                merged.extend(rows);
            }
            Err(e) => {
                let msg = format!("{e:#}");
                let st = if msg.contains("unavailable") {
                    "unavailable"
                } else {
                    "error"
                };
                statuses.push(PortStatusV1 {
                    port: p.name().into(),
                    backend: p.backend(),
                    status: st.into(),
                    detail: Some(msg.clone()),
                    retrieved: 0,
                });
                omitted.push(OmittedEntry {
                    memory_id: format!("port:{}", p.name()),
                    reason: format!("backend failed: {msg}"),
                });
                last_err = Some(e);
            }
        }
    }

    let ok_ports = statuses.iter().filter(|s| s.status == "ok").count();
    if ok_ports == 0 {
        return Err(last_err.unwrap_or_else(|| bail_any("all ports failed")));
    }

    // Merge candidates from all ports, then run shared enforcement once.
    let result = {
        // Re-use assemble_retrieval via an equivalent inline path so omitted
        // reasons stay uniform with single-port usage.
        let q = &inputs.query;
        let mut all = merged;
        all.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.memory_id.cmp(&b.memory_id))
        });
        let mut selected = Vec::new();
        let mut used: u64 = 0;
        let budget_tokens = budget;
        for c in all {
            if let Some(cur) = &inputs.current_revision
                && c.source_revision != *cur
            {
                omitted.push(OmittedEntry {
                    memory_id: c.memory_id.clone(),
                    reason: format!(
                        "stale revision: produced against {}, current {cur}",
                        c.source_revision
                    ),
                });
                continue;
            }
            let t = crate::workflow::memory_contracts::estimate_tokens(&c.content);
            if used.saturating_add(t) > budget_tokens {
                omitted.push(OmittedEntry {
                    memory_id: c.memory_id.clone(),
                    reason: "token budget exceeded".into(),
                });
                continue;
            }
            used += t;
            selected.push(c);
        }
        crate::workflow::memory_contracts::RetrievalResult {
            schema_version: "1.0.0".into(),
            query_id: q.query_id.clone(),
            candidates: selected
                .into_iter()
                .map(|c| crate::workflow::memory_contracts::RetrievalCandidate {
                    memory_id: c.memory_id,
                    kind: c.kind,
                    source_revision: c.source_revision,
                    evidence: c.evidence,
                    content: c.content,
                    relevance: c.relevance,
                })
                .collect(),
            omitted,
            token_estimate: used,
            policy: OperationPolicy {
                backend: if ports.len() > 1 {
                    BackendKind::Local
                } else {
                    ports[0].backend()
                },
                mutation: "none".into(),
                executed_at: inputs.executed_at.clone(),
            },
        }
    };

    let result_omitted = result.omitted.clone();
    let omitted_stale_count = |om: &Vec<OmittedEntry>| {
        om.iter()
            .filter(|o| o.reason.starts_with("stale revision"))
            .count()
    };
    let omitted_budget_count = |om: &Vec<OmittedEntry>| {
        om.iter()
            .filter(|o| o.reason == "token budget exceeded")
            .count()
    };
    let bundle = assemble_context_bundle(
        &format!("plan-{}", inputs.planner_id),
        &result.query_id,
        result.candidates,
        result_omitted.clone(),
        result.policy,
    )?;
    let audit = PlanningAuditV1 {
        schema_version: "1.0.0".into(),
        planner_id: inputs.planner_id.clone(),
        model_id: inputs.profile.model_id.clone(),
        effective_budget_tokens: budget,
        retrieved_total: statuses.iter().map(|s| s.retrieved).sum(),
        selected_total: bundle.blocks.len(),
        omitted_stale: omitted_stale_count(&result_omitted),
        omitted_budget: omitted_budget_count(&result_omitted),
        omitted_other: result_omitted.len()
            - omitted_stale_count(&result_omitted)
            - omitted_budget_count(&result_omitted),
        ports: statuses,
        digest_echo: bundle.digest.clone(),
    };
    Ok(PlanOutcome { bundle, audit })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::memory_contracts::{
        EvidenceReferenceV1, MemoryBackendUnavailable, MemoryKind, MemoryWrite,
    };

    struct StubPort {
        backend: BackendKind,
        rows: Vec<crate::workflow::memory_contracts::RawCandidate>,
        unavailable: bool,
    }
    impl MemoryRetrievalPort for StubPort {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn backend(&self) -> BackendKind {
            self.backend
        }
        fn retrieve(
            &self,
            _q: &MemoryQuery,
        ) -> Result<Vec<crate::workflow::memory_contracts::RawCandidate>> {
            if self.unavailable {
                return Err(anyhow::Error::new(MemoryBackendUnavailable {
                    backend: self.backend,
                    message: "offline".into(),
                }));
            }
            Ok(self.rows.clone())
        }
        fn write(&self, _: &MemoryWrite) -> Result<String> {
            Err(anyhow::Error::new(MemoryBackendUnavailable {
                backend: self.backend,
                message: "read-only".into(),
            }))
        }
    }

    fn row(
        id: &str,
        rel: f32,
        chars: usize,
        rev: &str,
    ) -> crate::workflow::memory_contracts::RawCandidate {
        crate::workflow::memory_contracts::RawCandidate {
            memory_id: id.into(),
            kind: MemoryKind::Decision,
            source_revision: rev.into(),
            evidence: EvidenceReferenceV1 {
                id: id.into(),
                event_digest: "a".repeat(64),
                artifact_digest: "b".repeat(64),
                artifact_kind: "repo-symbol".into(),
                produced_by: "test".into(),
                produced_at: None,
            },
            content: format!("content-{id} {}", "w".repeat(chars)),
            relevance: rel,
        }
    }

    fn inputs(model: &str, window: u64, budget: Option<u64>) -> PlannerInputsV1 {
        PlannerInputsV1 {
            planner_id: "pl-1".into(),
            query: MemoryQuery {
                schema_version: "1.0.0".into(),
                query_id: "q-1".into(),
                readable_scopes: vec!["repo://t".into()],
                text: "decisions about helper".into(),
                kinds: vec![],
                token_budget: None,
            },
            current_revision: Some("r1".into()),
            profile: ModelProfileV1 {
                model_id: model.into(),
                harness: "cli".into(),
                context_window_tokens: window,
                reserved_output_tokens: 100,
            },
            token_budget_override: budget,
            executed_at: "2026-08-24T00:00:00Z".into(),
        }
    }

    #[test]
    fn determinism_same_inputs_same_bundle() {
        let rows = vec![row("a", 0.9, 40, "r1"), row("b", 0.7, 60, "r1")];
        let p = StubPort {
            backend: BackendKind::Local,
            rows: rows.clone(),
            unavailable: false,
        };
        let inp = inputs("m1", 4096, None);
        let o1 = plan(&inp, &[&p]).unwrap();
        let o2 = plan(&inp, &[&p]).unwrap();
        assert_eq!(o1.bundle.digest, o2.bundle.digest);
        assert_eq!(o1.audit, o2.audit);
    }

    #[test]
    fn different_profiles_produce_different_projections() {
        let rows = vec![
            row("a", 0.9, 200, "r1"),
            row("b", 0.8, 200, "r1"),
            row("c", 0.7, 200, "r1"),
        ];
        let p = StubPort {
            backend: BackendKind::Local,
            rows,
            unavailable: false,
        };
        // Large model gets all three; small model only fits one.
        let big = inputs("big", 8192, Some(300));
        let small = inputs("small", 1024, Some(80));
        let ob = plan(&big, &[&p]).unwrap();
        let os = plan(&small, &[&p]).unwrap();
        assert!(ob.bundle.blocks.len() > os.bundle.blocks.len());
    }

    #[test]
    fn multi_port_partial_failure_degrades_to_audit() {
        let good_rows = vec![row("g1", 0.9, 40, "r1")];
        let good = StubPort {
            backend: BackendKind::Local,
            rows: good_rows,
            unavailable: false,
        };
        let bad = StubPort {
            backend: BackendKind::Mnemosyne,
            rows: vec![],
            unavailable: true,
        };
        let inp = inputs("mp-1", 4096, None);
        let outcome = plan(&inp, &[&good, &bad]).unwrap();
        assert_eq!(outcome.bundle.blocks.len(), 1);
        let failed = outcome
            .audit
            .ports
            .iter()
            .find(|s| s.status != "ok")
            .expect("bad port");
        assert_eq!(failed.status, "unavailable");
        assert!(
            outcome
                .bundle
                .omitted
                .iter()
                .any(|o| o.memory_id == "port:stub")
        );
    }

    #[test]
    fn stale_revision_omitted_with_reason() {
        let mut stale = row("old", 0.99, 20, "ancient");
        stale.source_revision = "ancient".into();
        let fresh = row("new", 0.5, 20, "r1");
        let p = StubPort {
            backend: BackendKind::Local,
            rows: vec![stale, fresh],
            unavailable: false,
        };
        let inp = inputs("st-1", 4096, None);
        let outcome = plan(&inp, &[&p]).unwrap();
        assert_eq!(outcome.bundle.blocks[0].memory_id, "new");
        assert!(
            outcome
                .bundle
                .omitted
                .iter()
                .any(|o| o.reason.starts_with("stale revision"))
        );
    }
}
