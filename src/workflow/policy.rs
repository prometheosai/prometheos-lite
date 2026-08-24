//! Lite runtime policy: effective-authority resolution + enforcement
//! (`lite.policy.v1`) for #118.
//!
//! OWNERSHIP: SOMA owns authority/escalation/irreversibility/governance-
//! predicate semantics (AuthorityProfile). Lite owns resolving the effective
//! authority for a concrete run, reducing it per local policy, enforcing the
//! result before effects occur, and recording durable decisions. Until
//! soma#80 publishes a canonical ExecutionProfile, the snapshot here is
//! explicitly **Lite-owned**; a versioned fail-closed mapping will follow
//! publication. No second authority taxonomy is defined.
//!
//! INVARIANT: resolution is MONOTONE-DECREASING. The effective snapshot's
//! scopes are always a subset of BOTH the manifest's and the local
//! restrictions'; denied lists only grow; budgets only shrink. Retry,
//! fallback, escalation, nested execution, or harness replacement can never
//! widen an existing snapshot.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::node_contracts::NodeManifestV1;

/// Version of the lite.policy contract family.
pub const POLICY_CONTRACT_VERSION: &str = "1.0.0";
pub const POLICY_CONTRACT_MAJOR: u64 = 1;

fn ensure_supported_policy_version(v: &str) -> Result<()> {
    let sv = crate::workflow::schema::SchemaVersion::parse(v)
        .with_context(|| format!("invalid lite.policy schema_version {v:?}"))?;
    let ceiling = crate::workflow::schema::SchemaVersion::new(
        POLICY_CONTRACT_MAJOR as u32,
        u32::MAX,
        u32::MAX,
    );
    if sv > ceiling {
        bail!("unsupported lite.policy contract version {v} (fail closed)");
    }
    Ok(())
}

/// Local (user/task/project) restrictions intersected with declared authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalRestrictions {
    /// Scope ceiling locally granted (subset ends up effective).
    pub readable_scopes: Vec<String>,
    pub writable_scopes: Vec<String>,
    /// Providers/harnesses prohibited by privacy/provider policy.
    #[serde(default)]
    pub denied_providers: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    /// Hard cap overriding any declared budget when smaller.
    #[serde(default)]
    pub token_budget_ceiling: Option<u64>,
    /// Global attempt ceiling.
    pub max_attempts: u32,
    /// Where governed handoffs go (Lite-owned label).
    pub escalation_target: String,
}

/// Immutable, versioned effective execution snapshot for one attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveExecutionSnapshotV1 {
    pub schema_version: String,
    pub snapshot_id: String,
    pub node_id: String,
    /// Intersection of manifest + local readable scopes.
    pub readable_scopes: Vec<String>,
    /// Intersection of manifest + local writable scopes.
    pub writable_scopes: Vec<String>,
    /// Effective token budget (min of declared vs ceiling).
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub denied_providers: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    pub max_attempts: u32,
    pub escalation_target: String,
    pub recorded_at: String,
}

impl EffectiveExecutionSnapshotV1 {
    pub fn parse_json(json: &str) -> Result<Self> {
        let s: Self = serde_json::from_str(json).context("failed to parse lite.policy snapshot")?;
        ensure_supported_policy_version(&s.schema_version)?;
        if s.snapshot_id.is_empty() || s.node_id.is_empty() {
            bail!("snapshot_id and node_id must not be empty");
        }
        Ok(s)
    }
}

/// Typed policy violation raised BEFORE an effect occurs.
#[derive(Debug)]
pub struct PolicyViolation {
    pub kind: PolicyViolationKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyViolationKind {
    Scope,
    Provider,
    AttemptsExhausted,
}

impl std::fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let k = match self.kind {
            PolicyViolationKind::Scope => "scope",
            PolicyViolationKind::Provider => "provider",
            PolicyViolationKind::AttemptsExhausted => "attempts-exhausted",
        };
        write!(f, "policy violation [{k}]: {}", self.detail)
    }
}
impl std::error::Error for PolicyViolation {}

/// Resolve the immutable effective snapshot. Monotone-decreasing by
/// construction: every output scope appears in BOTH inputs; budgets take the
/// minimum; denial lists union. Fails closed when memory reads are declared
/// but the intersection leaves no readable scope.
pub fn resolve_effective(
    manifest: &NodeManifestV1,
    local: &LocalRestrictions,
    recorded_at: String,
) -> Result<EffectiveExecutionSnapshotV1> {
    let intersect = |a: &[String], b: &[String]| -> Vec<String> {
        let mut v: Vec<String> = a
            .iter()
            .filter(|s| b.iter().any(|t| t == *s))
            .cloned()
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let readable = intersect(&manifest.readable_scopes, &local.readable_scopes);
    let writable = intersect(&manifest.writable_scopes, &local.writable_scopes);
    if !manifest.memory_reads.is_empty() && readable.is_empty() {
        bail!(
            "node {} declares memory reads but scope intersection is empty (fail closed)",
            manifest.node_id
        );
    }
    let token_budget = match (manifest.token_budget, local.token_budget_ceiling) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let mut denied = local.denied_providers.clone();
    denied.sort();
    denied.dedup();
    let max_attempts = manifest.retry.max_attempts.min(local.max_attempts);
    Ok(EffectiveExecutionSnapshotV1 {
        schema_version: POLICY_CONTRACT_VERSION.to_string(),
        snapshot_id: format!(
            "snap-{}-{}",
            manifest.node_id,
            crate::workflow::evaluate::now_iso()
        ),
        node_id: manifest.node_id.clone(),
        readable_scopes: readable,
        writable_scopes: writable,
        token_budget,
        denied_providers: denied,
        forbidden_paths: local.forbidden_paths.clone(),
        max_attempts,
        escalation_target: local.escalation_target.clone(),
        recorded_at,
    })
}

/// Enforce the snapshot BEFORE an effect occurs. `attempt` is 1-based.
pub fn enforce_before_effects(
    snap: &EffectiveExecutionSnapshotV1,
    attempt: u32,
    requested_writable_scope: Option<&str>,
    requested_provider: Option<&str>,
) -> Result<()> {
    if attempt > snap.max_attempts {
        return Err(PolicyViolation {
            kind: PolicyViolationKind::AttemptsExhausted,
            detail: format!(
                "attempt {attempt} exceeds max_attempts {}",
                snap.max_attempts
            ),
        }
        .into());
    }
    if let Some(p) = requested_provider
        && snap.denied_providers.iter().any(|d| d == p)
    {
        return Err(PolicyViolation {
            kind: PolicyViolationKind::Provider,
            detail: format!("provider '{p}' is prohibited by privacy/provider policy"),
        }
        .into());
    }
    if let Some(s) = requested_writable_scope
        && !snap.writable_scopes.iter().any(|w| w == s)
    {
        return Err(PolicyViolation {
            kind: PolicyViolationKind::Scope,
            detail: format!("write scope '{s}' is outside the effective writable scopes"),
        }
        .into());
    }
    Ok(())
}

/// Durable record of one enforcement decision, bound to an immutable
/// snapshot via its canonical digest (recomputable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyDecisionRecordV1 {
    pub schema_version: String,
    pub snapshot_id: String,
    pub snapshot_digest: String,
    pub effect: String,
    pub allowed: bool,
    pub reason: String,
    pub recorded_at: String,
}

impl PolicyDecisionRecordV1 {
    pub fn new(
        snap: &EffectiveExecutionSnapshotV1,
        effect: &str,
        allowed: bool,
        reason: &str,
        recorded_at: String,
    ) -> Result<Self> {
        let digest_value = serde_json::to_value(snap).context("serialize snapshot")?;
        let digest = crate::workflow::memory_contracts::canonical_digest(&digest_value)?;
        Ok(Self {
            schema_version: POLICY_CONTRACT_VERSION.to_string(),
            snapshot_id: snap.snapshot_id.clone(),
            snapshot_digest: digest,
            effect: effect.to_string(),
            allowed,
            reason: reason.to_string(),
            recorded_at,
        })
    }

    pub fn parse_json(json: &str) -> Result<Self> {
        let r: Self = serde_json::from_str(json).context("failed to parse lite.policy decision")?;
        ensure_supported_policy_version(&r.schema_version)?;
        Ok(r)
    }
}

// Helper so the snapshot can expose identity parts without widening the
// public struct (kept private to this module's digest computation).
impl EffectiveExecutionSnapshotV1 {
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(
        read: Vec<&str>,
        write: Vec<&str>,
        budget: Option<u64>,
        attempts: u32,
    ) -> NodeManifestV1 {
        NodeManifestV1 {
            schema_version: "1.0.0".into(),
            node_id: "n1".into(),
            purpose: "p".into(),
            inputs: vec![],
            outputs: vec![],
            readable_scopes: read.into_iter().map(String::from).collect(),
            writable_scopes: write.into_iter().map(String::from).collect(),
            token_budget: budget,
            retry: crate::workflow::node_contracts::RetryPolicy {
                max_attempts: attempts,
                retryable_classes: vec!["infra".into()],
            },
            memory_reads: vec![],
            memory_writes: vec![],
            work_state_ref: None,
            next_route_hints: vec![],
        }
    }

    fn local(
        read: Vec<&str>,
        write: Vec<&str>,
        ceiling: Option<u64>,
        attempts: u32,
    ) -> LocalRestrictions {
        LocalRestrictions {
            readable_scopes: read.into_iter().map(String::from).collect(),
            writable_scopes: write.into_iter().map(String::from).collect(),
            denied_providers: vec!["secret-provider".into()],
            forbidden_paths: vec!["secrets/**".into()],
            token_budget_ceiling: ceiling,
            max_attempts: attempts,
            escalation_target: "human-review".into(),
        }
    }

    #[test]
    fn resolution_is_monotone_decreasing() {
        let m = manifest(vec!["a", "b", "c"], vec!["w1", "w2"], Some(1000), 5);
        let l = local(vec!["b", "c", "d"], vec!["w2", "w3"], Some(400), 3);
        let s = resolve_effective(&m, &l, now()).unwrap();
        // Every effective scope must exist in BOTH inputs.
        for sc in &s.readable_scopes {
            assert!(m.readable_scopes.contains(sc) && l.readable_scopes.contains(sc));
        }
        for sc in &s.writable_scopes {
            assert!(m.writable_scopes.contains(sc) && l.writable_scopes.contains(sc));
        }
        assert_eq!(s.readable_scopes, vec!["b", "c"]);
        assert_eq!(s.writable_scopes, vec!["w2"]);
        assert_eq!(s.token_budget, Some(400), "min of declared vs ceiling");
        assert_eq!(s.max_attempts, 3, "min of manifest vs local");
        assert!(s.denied_providers.contains(&"secret-provider".to_string()));
    }

    #[test]
    fn reads_declared_with_empty_intersection_fail_closed() {
        let m = manifest(vec!["x"], vec![], None, 1);
        let m2 = NodeManifestV1 {
            memory_reads: vec![crate::workflow::memory_contracts::MemoryQuery {
                schema_version: "1.0.0".into(),
                query_id: "q".into(),
                readable_scopes: vec!["other".into()],
                text: "t".into(),
                kinds: vec![],
                token_budget: None,
            }],
            ..manifest(vec!["a"], vec![], None, 1)
        };
        let _ = m;
        let err = resolve_effective(&m2, &local(vec!["zzz"], vec![], None, 1), now()).unwrap_err();
        assert!(err.to_string().contains("fail closed"), "{err}");
    }

    #[test]
    fn enforce_rejects_out_of_scope_writes_before_effects() {
        let s = resolve_effective(
            &manifest(vec!["r"], vec!["w1"], None, 3),
            &local(vec!["r"], vec!["w1", "w9"], None, 3),
            now(),
        )
        .unwrap();
        assert!(enforce_before_effects(&s, 1, Some("w1"), None).is_ok());
        let err = enforce_before_effects(&s, 1, Some("w9"), None).unwrap_err();
        assert!(err.to_string().contains("[scope]"), "{err}");
    }

    #[test]
    fn denied_providers_are_blocked_before_effects() {
        let s = resolve_effective(
            &manifest(vec!["r"], vec![], None, 3),
            &local(vec!["r"], vec![], None, 3),
            now(),
        )
        .unwrap();
        assert!(enforce_before_effects(&s, 1, None, Some("openai")).is_ok());
        let err = enforce_before_effects(&s, 1, None, Some("secret-provider")).unwrap_err();
        assert!(err.to_string().contains("[provider]"), "{err}");
    }

    #[test]
    fn attempt_limits_prevent_unbounded_execution() {
        let s = resolve_effective(
            &manifest(vec!["r"], vec![], None, 2),
            &local(vec!["r"], vec![], None, 2),
            now(),
        )
        .unwrap();
        assert!(enforce_before_effects(&s, 2, None, None).is_ok());
        let err = enforce_before_effects(&s, 3, None, None).unwrap_err();
        assert!(err.to_string().contains("[attempts-exhausted]"), "{err}");
    }

    #[test]
    fn decision_record_binds_snapshot_digest_and_roundtrips() {
        let s = resolve_effective(
            &manifest(vec!["r"], vec!["w"], None, 2),
            &local(vec!["r"], vec!["w"], None, 2),
            now(),
        )
        .unwrap();
        let rec = PolicyDecisionRecordV1::new(
            &s,
            "memory.write",
            false,
            "scope outside effective set",
            now(),
        )
        .unwrap();
        assert!(!rec.allowed);
        assert_eq!(rec.snapshot_digest.len(), 64);
        let parsed =
            PolicyDecisionRecordV1::parse_json(&serde_json::to_string(&rec).unwrap()).unwrap();
        assert_eq!(parsed, rec);
        // Determinism: same snapshot => same digest.
        let rec2 = PolicyDecisionRecordV1::new(
            &s,
            "memory.write",
            false,
            "scope outside effective set",
            now(),
        )
        .unwrap();
        assert_eq!(rec.snapshot_digest, rec2.snapshot_digest);
    }

    #[test]
    fn future_major_policy_version_fails_closed() {
        let s = resolve_effective(
            &manifest(vec!["r"], vec![], None, 1),
            &local(vec!["r"], vec![], None, 1),
            now(),
        )
        .unwrap();
        let mut v = serde_json::to_value(&s).unwrap();
        v["schemaVersion"] = "5.0.0".into();
        let err = EffectiveExecutionSnapshotV1::parse_json(&v.to_string()).unwrap_err();
        assert!(err.to_string().contains("fail closed"), "{err}");
    }

    fn now() -> String {
        "2026-08-24T00:00:00Z".into()
    }
}
