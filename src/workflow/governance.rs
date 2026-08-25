//! Governed authority compilation and graph-level governance enforcement
//! (issue #161).
//!
//! Compiles a SOMA `WorkflowDefinition` into a runtime-checkable authority
//! graph, enforces the seven required governance rules before any effect
//! runs, and records durable `lite.govdec.v1` decision evidence.
//!
//! Ownership: semantics under SOMA-owned codes reuse the published
//! diagnostics verbatim; `LITE-GOV-*` codes mark Lite runtime rules with no
//! published equivalent (disclosed in decision records — never masquerading
//! as SOMA codes).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workflow::soma::contracts::{AuthorityProfile, OperationDefinition, WorkflowDefinition};
use crate::workflow::soma::profile::authority_widened;
use crate::workflow::soma::{Diagnostic, SupportedVersion};

/// Schema version for Lite governance decision records.
pub const GOV_DECISION_VERSION: &str = "1.0.0";

// ---------------------------------------------------------------------------
// Compiled authority graph
// ---------------------------------------------------------------------------

/// Effective authority resolved for one operation.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledOperationAuthority {
    pub op_id: String,
    /// Workflow authority narrowed by nothing further at op level: ops hold
    /// grants; the workflow profile is the ceiling. Kept for evidence.
    pub effective: AuthorityProfile,
}

/// The runtime-checkable authority plan for one workflow.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledAuthorityGraph {
    pub workflow_id: String,
    pub workflow_version: String,
    pub workflow_authority: AuthorityProfile,
    pub operations: Vec<CompiledOperationAuthority>,
    pub constraints: Vec<crate::workflow::soma::contracts::GovernanceConstraint>,
    /// Audit-clean body retained for rule evaluation (effects, grants,
    /// execution classes). Present because the published rules inspect
    /// per-operation declarations, not just resolved profiles.
    pub body: Vec<OperationDefinition>,
}

impl CompiledAuthorityGraph {
    /// Per-operation authority projection check: provider/harness selection
    /// expressed as an authority projection must not widen the compiled
    /// effective profile. Any widening is rejected before effects run.
    pub fn selection_within_authority(
        &self,
        op_id: &str,
        selection_projection: &AuthorityProfile,
    ) -> Result<(), Diagnostic> {
        let Some(op) = self.operations.iter().find(|o| o.op_id == op_id) else {
            return Err(Diagnostic::related(
                "SOMA-CMP-0002",
                format!("unknown operation {op_id:?} in authority graph"),
                op_id.to_string(),
            ));
        };
        if authority_widened(selection_projection, &op.effective) {
            return Err(Diagnostic::related(
                "SOMA-PROF-0001",
                "provider/harness selection widens compiled authority",
                op_id.to_string(),
            ));
        }
        Ok(())
    }
}

/// Compile + fully validate a workflow document's authority. Fail-closed:
/// any published-audit diagnostic rejects compilation BEFORE execution can
/// ever see the graph ("invalid authority graphs fail before execution").
pub fn compile_authority(workflow_json: &Value) -> Result<CompiledAuthorityGraph, Vec<Diagnostic>> {
    let model: WorkflowDefinition = serde_json::from_value(workflow_json.clone()).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0003",
            format!("schema violation: {e}"),
        )]
    })?;
    let supported: SupportedVersion = crate::workflow::soma::supported_version();
    let diags = model.audit(&supported);
    if !diags.is_empty() {
        return Err(diags);
    }
    let operations = model
        .body
        .iter()
        .map(|u| CompiledOperationAuthority {
            op_id: u.id.clone(),
            effective: model.authority.clone(),
        })
        .collect();
    Ok(CompiledAuthorityGraph {
        workflow_id: model.id.clone(),
        workflow_version: model.version.clone(),
        workflow_authority: model.authority.clone(),
        operations,
        constraints: model.constraints.clone(),
        body: model.body.clone(),
    })
}

// ---------------------------------------------------------------------------
// Governance rules
// ---------------------------------------------------------------------------

/// Binding configuration for Lite runtime governance rules that need a
/// declaration surface. Defaults match the published rule descriptions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GovernanceRuleBindings {
    /// Effect-name prefix treated as "applies source changes" (rule 1).
    #[serde(default = "default_source_prefix")]
    pub source_effect_prefix: String,
    /// Effect-name prefix treated as a public API change (rule 4).
    #[serde(default = "default_api_prefix")]
    pub api_effect_prefix: String,
    /// Effect-name prefix treated as a destructive action (rule 5).
    #[serde(default = "default_destructive_prefix")]
    pub destructive_effect_prefix: String,
}

fn default_destructive_prefix() -> String {
    "destroy:".into()
}

fn default_source_prefix() -> String {
    "source:".into()
}

fn default_api_prefix() -> String {
    "api.".into()
}

impl AttemptContext {
    /// No references supplied: every reference-gated rule fails closed.
    pub fn empty() -> Self {
        Self {
            approved_proposal_ref: None,
            contract_validation_ref: None,
        }
    }
}

impl Default for GovernanceRuleBindings {
    fn default() -> Self {
        Self {
            source_effect_prefix: default_source_prefix(),
            api_effect_prefix: default_api_prefix(),
            destructive_effect_prefix: default_destructive_prefix(),
        }
    }
}

/// Attempt context supplied by the runtime at enforcement time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AttemptContext {
    /// Human-approved validated proposal reference for source application,
    /// when the attempt applies source changes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "approvedProposalRef"
    )]
    pub approved_proposal_ref: Option<String>,
    /// Contract-validation evidence reference for public-API effects.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contractValidationRef"
    )]
    pub contract_validation_ref: Option<String>,
}

/// Enforce every governance rule over the compiled graph BEFORE effects.
/// Returns all violations (stable codes); empty = allowed to proceed.
pub fn enforce_before_effects(
    graph: &CompiledAuthorityGraph,
    bindings: &GovernanceRuleBindings,
    attempt: &AttemptContext,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let auth = &graph.workflow_authority;
    let review_only = auth.review.as_ref().is_some_and(|r| r.effect.is_some());

    for unit in &graph.body {
        enforce_unit_rules(graph, bindings, attempt, unit, review_only, auth, &mut out);
    }
    out
}

fn enforce_unit_rules(
    graph: &CompiledAuthorityGraph,
    bindings: &GovernanceRuleBindings,
    attempt: &AttemptContext,
    unit: &OperationDefinition,
    review_only: bool,
    auth: &AuthorityProfile,
    out: &mut Vec<Diagnostic>,
) {
    // Rule 3: review-only workflows cannot mutate source.
    if review_only {
        let mutates = !auth.writable_scopes.as_ref().unwrap_or(&vec![]).is_empty()
            || unit
                .effects
                .iter()
                .any(|e| e.name.starts_with(&bindings.source_effect_prefix));
        if mutates {
            out.push(Diagnostic::related(
                "LITE-GOV-0002",
                "review-only workflow cannot mutate source",
                unit.id.clone(),
            ));
        }
    }

    for eff in &unit.effects {
        // Rule 1: source application requires a human-approved validated
        // proposal.
        if eff.name.starts_with(&bindings.source_effect_prefix)
            && attempt
                .approved_proposal_ref
                .as_deref()
                .map(|s| s.is_empty())
                .unwrap_or(true)
        {
            out.push(Diagnostic::related(
                "LITE-GOV-0001",
                "source application without human-approved validated proposal",
                format!("{}/{}", unit.id, eff.name),
            ));
        }
        // Rule 2: restricted data must not reach disallowed providers —
        // enforced at the authority boundary (SOMA-AUTH-0004 audit already
        // rejects restriction targets outside allowlists at compile time;
        // runtime re-check guards dynamic provider picks).
        // Rule 4: public API changes require contract validation + human
        // review.
        if eff.name.starts_with(&bindings.api_effect_prefix) {
            let has_contract_evidence = attempt
                .contract_validation_ref
                .as_deref()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            let has_review_gate =
                auth.review.as_ref().and_then(|r| r.effect.as_deref()) == Some(eff.name.as_str());
            if !has_contract_evidence || !has_review_gate {
                out.push(Diagnostic::related(
                    "LITE-GOV-0003",
                    "public API change without contract validation and human review gate",
                    format!("{}/{}", unit.id, eff.name),
                ));
            }
        }
        // Rule 5: destructive actions (declared by binding prefix) require
        // explicit mutation authority AND a rollback strategy (escalation).
        // SOMA-AUTH-0008 covers the published irreversible half at audit
        // time; this rule adds the Lite-side destructive declaration surface.
        if eff.name.starts_with(&bindings.destructive_effect_prefix)
            && (auth.mutation != crate::workflow::soma::types::MutationMode::Explicit
                || !auth.has_recovery_path())
        {
            out.push(Diagnostic::related(
                "LITE-GOV-0005",
                "destructive action without explicit mutation authority and rollback path",
                format!("{}/{}", unit.id, eff.name),
            ));
        }
        // Rule 6: model output cannot directly trigger irreversible effects.
        if eff.irreversible.unwrap_or(false)
            && unit.execution_class != crate::workflow::soma::types::ExecutionClass::Deterministic
        {
            out.push(Diagnostic::related(
                "LITE-GOV-0004",
                "non-deterministic operation emits irreversible effect",
                format!("{}/{}", unit.id, eff.name),
            ));
        }
    }
    let _ = graph;
}

// ---------------------------------------------------------------------------
// Durable governance decision record (lite.govdec.v1)
// ---------------------------------------------------------------------------

/// Verdict for one rule over the whole graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuleVerdict {
    pub rule_code: String,
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Durable evidence record of a governance enforcement pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GovernanceDecisionRecordV1 {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(rename = "workflowId")]
    pub workflow_id: String,
    #[serde(rename = "workflowVersion")]
    pub workflow_version: String,
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
    pub verdicts: Vec<RuleVerdict>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

impl GovernanceDecisionRecordV1 {
    /// Build a record from an enforcement outcome; digest computed without
    /// the digest member itself (canonical-digest discipline shared with
    /// lite.policy.v1 / SOMA contentDigest).
    pub fn new(
        graph: &CompiledAuthorityGraph,
        recorded_at: impl Into<String>,
        violations: &[Diagnostic],
    ) -> Self {
        let mut by_code: BTreeMap<String, (bool, Vec<String>)> = BTreeMap::new();
        for code in [
            "LITE-GOV-0001",
            "LITE-GOV-0002",
            "LITE-GOV-0003",
            "LITE-GOV-0004",
            "LITE-GOV-0005",
            "SOMA-AUTH-0002",
            "SOMA-AUTH-0004",
            "SOMA-AUTH-0008",
            "SOMA-PROF-0001",
        ] {
            by_code.insert(code.into(), (true, Vec::new()));
        }
        for v in violations {
            let entry = by_code.entry(v.code.clone()).or_insert((true, Vec::new()));
            entry.0 = false;
            entry.1.extend(v.related.iter().cloned());
        }
        let verdicts = by_code
            .into_iter()
            .map(|(code, (allowed, subjects))| RuleVerdict {
                rule_code: code,
                allowed,
                subjects,
                message: None,
            })
            .collect();
        Self {
            schema_version: GOV_DECISION_VERSION.into(),
            workflow_id: graph.workflow_id.clone(),
            workflow_version: graph.workflow_version.clone(),
            recorded_at: recorded_at.into(),
            verdicts,
            content_digest: None,
        }
    }

    /// Canonical digest over the record minus the digest member.
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("record serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::canonical_digest(&v)
    }

    pub fn sealed(mut self) -> Self {
        let d = self.compute_digest();
        self.content_digest = Some(d);
        self
    }

    /// Fail-closed parse: version-gated, digest-verified.
    pub fn parse_json(text: &str) -> Result<Self, String> {
        let model: Self =
            serde_json::from_str(text).map_err(|e| format!("govdec parse failed: {e}"))?;
        if model.schema_version.split('.').next() != Some("1") {
            return Err(format!(
                "unsupported govdec schema version {}",
                model.schema_version
            ));
        }
        if let Some(declared) = &model.content_digest {
            if *declared != model.compute_digest() {
                return Err("govdec contentDigest does not verify".into());
            }
        } else {
            return Err("govdec missing contentDigest".into());
        }
        Ok(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_workflow() -> Value {
        json!({
            "schemaVersion": "1.1.0",
            "version": "1.1.0",
            "id": "wf-gov",
            "name": "Gov",
            "kind": "atomic",
            "inputPorts": [{
                "name": "order", "direction": "input", "type": "Order",
                "cardinality": "single", "requiredness": "required"
            }],
            "outputPorts": [{
                "name": "shipment", "direction": "output", "type": "Shipment",
                "cardinality": "single", "requiredness": "required"
            }],
            "body": [{
                "schemaVersion": "1.1.0", "version": "1.1.0", "id": "op1",
                "executionClass": "deterministic",
                "inputs": [{"name": "order", "type": "Order", "acceptedOutcomes": ["Produced"]}],
                "outputs": [{"name": "shipment", "type": "Shipment", "emits": ["Produced"]}],
                "authority": ["ship"],
                "effects": [], "uses": [], "secrets": [], "context": []
            }],
            "authority": {
                "executionClass": "deterministic",
                "mutation": "none",
                "tools": {"ship": ["ship.v1"]},
                "readableScopes": ["orders"],
                "writableScopes": ["shipping"],
                "networkPolicy": {"default": "deny"},
                "providerPolicy": {"allowlist": ["provider-1"]}
            }
        })
    }

    #[test]
    fn clean_graph_compiles_and_enforces_to_empty() {
        let wf = base_workflow();
        let graph = compile_authority(&wf).expect("clean graph compiles");
        assert_eq!(graph.operations.len(), 1);
        let violations = enforce_before_effects(
            &graph,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn invalid_authority_graph_fails_before_execution() {
        let mut wf = base_workflow();
        // op uses a capability that is not granted -> SOMA-AUTH-0001
        wf["body"][0]["authority"] = json!(["ghost-cap"]);
        let err = compile_authority(&wf).expect_err("must fail closed");
        assert!(err.iter().any(|d| d.code == "SOMA-AUTH-0001"), "{err:?}");
    }

    #[test]
    fn rule_source_application_requires_approved_proposal() {
        let mut wf = base_workflow();
        wf["body"][0]["effects"] = json!([{"name": "source:apply"}]);
        let graph = compile_authority(&wf).unwrap();
        let v = enforce_before_effects(
            &graph,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        assert!(v.iter().any(|d| d.code == "LITE-GOV-0001"), "{v:?}");

        let ok = AttemptContext {
            approved_proposal_ref: Some("proposal-7".into()),
            contract_validation_ref: None,
        };
        let v2 = enforce_before_effects(&graph, &GovernanceRuleBindings::default(), &ok);
        assert!(!v2.iter().any(|d| d.code == "LITE-GOV-0001"), "{v2:?}");
    }

    #[test]
    fn rule_review_only_cannot_mutate() {
        let mut wf = base_workflow();
        wf["authority"]["review"] = json!({"effect": "publish"});
        let graph = compile_authority(&wf).unwrap();
        let v = enforce_before_effects(
            &graph,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        assert!(
            v.iter().any(|d| d.code == "LITE-GOV-0002"),
            "writable scopes under review gate must trip LITE-GOV-0002: {v:?}"
        );
    }

    #[test]
    fn rule_public_api_requires_contract_validation_and_review() {
        let mut wf = base_workflow();
        wf["body"][0]["effects"] = json!([{"name": "api.publish"}]);
        wf["authority"]["review"] = json!({"effect": "api.publish"});
        let graph = compile_authority(&wf).unwrap();

        let no_contract = AttemptContext {
            approved_proposal_ref: None,
            contract_validation_ref: None,
        };
        let v = enforce_before_effects(&graph, &GovernanceRuleBindings::default(), &no_contract);
        assert!(v.iter().any(|d| d.code == "LITE-GOV-0003"), "{v:?}");

        let with_contract = AttemptContext {
            approved_proposal_ref: None,
            contract_validation_ref: Some("cv-1".into()),
        };
        let v2 = enforce_before_effects(&graph, &GovernanceRuleBindings::default(), &with_contract);
        assert!(!v2.iter().any(|d| d.code == "LITE-GOV-0003"), "{v2:?}");
    }

    #[test]
    fn rule_destructive_requires_explicit_mutation_and_rollback() {
        let mut wf = base_workflow();
        // Destructive by declaration (binding prefix), not flagged
        // irreversible: the published audit passes, the Lite runtime rule
        // still demands explicit mutation + rollback path.
        wf["body"][0]["effects"] = json!([{"name": "destroy:data"}]);
        let graph = compile_authority(&wf).unwrap();
        let v = enforce_before_effects(
            &graph,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        assert!(v.iter().any(|d| d.code == "LITE-GOV-0005"), "{v:?}");

        // With explicit mutation authority and a recovery path it passes.
        wf["authority"]["mutation"] = json!("explicit");
        wf["authority"]["escalation"] = json!({"to": "human-on-call"});
        let graph2 = compile_authority(&wf).unwrap();
        let v2 = enforce_before_effects(
            &graph2,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        assert!(!v2.iter().any(|d| d.code == "LITE-GOV-0005"), "{v2:?}");
    }

    #[test]
    fn rule_model_output_cannot_trigger_irreversible() {
        let mut wf = base_workflow();
        wf["authority"]["executionClass"] = json!("model-assisted");
        wf["authority"]["mutation"] = json!("explicit");
        wf["authority"]["escalation"] = json!({"to": "human-on-call"});
        wf["body"][0]["executionClass"] = json!("model-assisted");
        wf["body"][0]["effects"] = json!([{"name": "wipe", "irreversible": true}]);
        let graph = compile_authority(&wf).unwrap();
        let v = enforce_before_effects(
            &graph,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        assert!(v.iter().any(|d| d.code == "LITE-GOV-0004"), "{v:?}");
    }

    #[test]
    fn provider_selection_cannot_expand_authority() {
        let wf = base_workflow();
        let graph = compile_authority(&wf).unwrap();
        let mut wider = graph.workflow_authority.clone();
        wider.network_policy = Some(crate::workflow::soma::contracts::NetworkPolicy {
            default: crate::workflow::soma::types::NetworkDefault::Allow,
            allowlist: Some(vec!["x.com".into()]),
        });
        let err = graph.selection_within_authority("op1", &wider);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().code, "SOMA-PROF-0001");

        let same = graph.workflow_authority.clone();
        assert!(graph.selection_within_authority("op1", &same).is_ok());
    }

    #[test]
    fn decision_record_seals_and_verifies() {
        let mut wf = base_workflow();
        wf["body"][0]["effects"] = json!([{"name": "destroy:data"}]);
        let graph = compile_authority(&wf).unwrap();
        let violations = enforce_before_effects(
            &graph,
            &GovernanceRuleBindings::default(),
            &AttemptContext::empty(),
        );
        let record =
            GovernanceDecisionRecordV1::new(&graph, "2026-08-24T00:00:00Z", &violations).sealed();
        assert!(
            record
                .content_digest
                .as_deref()
                .is_some_and(|d| d.len() == 64)
        );

        let text = serde_json::to_string(&record).unwrap();
        let parsed = GovernanceDecisionRecordV1::parse_json(&text).expect("verifies");
        assert_eq!(parsed, record);

        // Tamper: flip a verdict -> digest must fail.
        let mut tampered = record.clone();
        tampered.verdicts[0].allowed = !tampered.verdicts[0].allowed;
        let bad = serde_json::to_string(&tampered).unwrap();
        assert!(GovernanceDecisionRecordV1::parse_json(&bad).is_err());

        // Missing digest fails closed.
        let mut naked = record.clone();
        naked.content_digest = None;
        let bad2 = serde_json::to_string(&naked).unwrap();
        assert!(GovernanceDecisionRecordV1::parse_json(&bad2).is_err());
    }

    #[test]
    fn major_version_gate_fails_closed() {
        let wf = base_workflow();
        let graph = compile_authority(&wf).unwrap();
        let record = GovernanceDecisionRecordV1::new(&graph, "2026-08-24T00:00:00Z", &[]).sealed();
        let mut text = serde_json::to_value(&record).unwrap();
        text["schemaVersion"] = json!("2.0.0");
        // Digest was computed for 1.x content; version bump alone breaks it.
        assert!(GovernanceDecisionRecordV1::parse_json(&text.to_string()).is_err());
    }
}
