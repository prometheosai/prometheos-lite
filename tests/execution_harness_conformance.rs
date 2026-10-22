//! Adapter conformance for the governed execution-harness seam
//! (issue #154, slice A).
//!
//! Two DISTINCT implementations — an in-process `native` worker and a
//! `protocol-bridged` worker driven by a recorded script — pass the same
//! reusable conformance kit. Neither is a real external adapter: this slice
//! proves the seam is implementable twice and that the declaration/behaviour
//! agreement is testable, without pretending Claude Code/Codex/Goose support
//! exists.
//!
//! The suite also proves the two structural laws:
//!
//! 1. an adapter cannot invoke a capability outside its declaration, and the
//!    refusal happens BEFORE the governed runner is touched;
//! 2. the catalogue is generated from declarations, so it is byte-identical
//!    regardless of registration order, and duplicate/unsealed declarations
//!    are refused.

mod common;

use anyhow::{Result, bail};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessCheckpointV1, HarnessContinuation,
    HarnessDeclarationV1, HarnessDispatch, HarnessEvent, HarnessFamily, HarnessOutcome,
    HarnessRegistry, HarnessStep, conformance_report,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{
    Capability, CapabilityRegistry, NodeRunOutcome, NodeRunner,
};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::soma::capability::{
    Identity, Locality, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
};

use common::permit_for;

const NATIVE_CAP: &str = "harness.native.probe";
const RELAY_CAP: &str = "harness.protocol.relay";
const NODE_ID: &str = "harness.probe";

/// A syntactically valid portable-state digest. The seam binds the digest,
/// never the state itself.
fn portable_digest() -> String {
    "0123456789abcdef".repeat(4)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn capability_set(
    harness_id: &str,
    name: &str,
    revision: &str,
    export: bool,
    import: bool,
    resume: bool,
) -> RuntimeCapabilitySet {
    RuntimeCapabilitySet {
        id: format!("{harness_id}.caps"),
        schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
        version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        declared_at: "2026-10-22T00:00:00Z".to_string(),
        spec_versions: vec!["1.1.0".to_string()],
        adapter_families: Vec::new(),
        effect_classes: vec!["process-exec".to_string()],
        workspace: None,
        checkpoint_import: Some(import),
        checkpoint_export: Some(export),
        resume_support: Some(resume),
        event_streaming: Some(true),
        read_projection: Some(true),
        locality: Some(Locality::Local),
        privacy_classes: Vec::new(),
        max_structured_input: None,
        max_structured_output: None,
        budget_ceilings: None,
        evidence_attachment: Some(true),
        validation: Some(true),
        approval_escalation: Some(false),
    }
}

fn declaration(
    harness_id: &str,
    name: &str,
    revision: &str,
    family: HarnessFamily,
    declared: &[&str],
    claimed: &[&str],
    export: bool,
    import: bool,
    resume: bool,
) -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: harness_id.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        family,
        declared_capabilities: declared.iter().map(|c| c.to_string()).collect(),
        claimed_capabilities: claimed.iter().map(|c| c.to_string()).collect(),
        capabilities: capability_set(harness_id, name, revision, export, import, resume),
        content_digest: None,
    }
    .sealed()
}

// ---------------------------------------------------------------------------
// Implementation 1: in-process native worker (no checkpoint support)
// ---------------------------------------------------------------------------

struct NativeHarness {
    declaration: HarnessDeclarationV1,
}

impl NativeHarness {
    fn new() -> Self {
        Self {
            declaration: declaration(
                "lite.harness.native",
                "lite-native",
                "v1",
                HarnessFamily::Native,
                &[NATIVE_CAP],
                &[NATIVE_CAP],
                false,
                false,
                false,
            ),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for NativeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        let outcome = dispatch.call(&step.capability, step.args, &step.step_id)?;
        let correlation = step.step_id.clone();
        Ok(HarnessOutcome {
            harness_id: self.declaration.harness_id.clone(),
            output: outcome.output.clone(),
            evidence_entry_digest: outcome.evidence_entry.detail_digest.clone(),
            events: vec![
                HarnessEvent {
                    sequence: 0,
                    correlation_id: correlation.clone(),
                    kind: "started".to_string(),
                    detail_digest: "0".repeat(64),
                },
                HarnessEvent {
                    sequence: 1,
                    correlation_id: correlation,
                    kind: "completed".to_string(),
                    detail_digest: "1".repeat(64),
                },
            ],
            content_digest: None,
        }
        .sealed())
    }
}

// ---------------------------------------------------------------------------
// Implementation 2: protocol-bridged worker (recorded script, portable
// checkpoint support)
// ---------------------------------------------------------------------------

/// Replays a recorded event script. This is the shape a transport-bridged
/// adapter will have: the seam fixes the lifecycle, the transport stays
/// outside it.
struct ScriptedProtocolHarness {
    declaration: HarnessDeclarationV1,
    script: Vec<String>,
}

impl ScriptedProtocolHarness {
    fn new(script: &[&str]) -> Self {
        Self {
            declaration: declaration(
                "lite.harness.protocol",
                "lite-protocol",
                "v1",
                HarnessFamily::ProtocolBridged,
                &[RELAY_CAP],
                &[RELAY_CAP],
                true,
                true,
                true,
            ),
            script: script.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for ScriptedProtocolHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        bail_if_foreign_capability(&step, &self.declaration)?;
        let outcome = dispatch.call(&step.capability, step.args, &step.step_id)?;
        let correlation = step.step_id.clone();
        let events = self
            .script
            .iter()
            .enumerate()
            .map(|(index, kind)| HarnessEvent {
                sequence: index as u64,
                correlation_id: correlation.clone(),
                kind: kind.clone(),
                detail_digest: format!("{index:064x}"),
            })
            .collect();
        Ok(HarnessOutcome {
            harness_id: self.declaration.harness_id.clone(),
            output: outcome.output.clone(),
            evidence_entry_digest: outcome.evidence_entry.detail_digest.clone(),
            events,
            content_digest: None,
        }
        .sealed())
    }

    async fn checkpoint(
        &self,
        outcome: &HarnessOutcome,
        portable_state_digest: &str,
    ) -> Result<HarnessCheckpointV1> {
        outcome.verify()?;
        if !is_hex64(portable_state_digest) {
            bail!("portable state digest must be 64 lowercase hex");
        }
        Ok(HarnessCheckpointV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: self.declaration.harness_id.clone(),
            harness_revision: self.declaration.identity.revision.clone(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: Some("next".to_string()),
            content_digest: None,
        }
        .sealed())
    }

    async fn resume(
        &self,
        checkpoint: &HarnessCheckpointV1,
        portable_state_digest: &str,
    ) -> Result<HarnessContinuation> {
        checkpoint.verify()?;
        if checkpoint.harness_id != self.declaration.harness_id {
            bail!("checkpoint belongs to a different harness");
        }
        if checkpoint.harness_revision != self.declaration.identity.revision {
            bail!("checkpoint belongs to a different revision");
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            bail!("checkpoint does not bind the supplied portable state");
        }
        Ok(HarnessContinuation {
            harness_id: self.declaration.harness_id.clone(),
            harness_revision: self.declaration.identity.revision.clone(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

fn bail_if_foreign_capability(
    step: &HarnessStep,
    declaration: &HarnessDeclarationV1,
) -> Result<()> {
    if !declaration
        .declared_capabilities
        .iter()
        .any(|c| c == &step.capability)
    {
        bail!("step names a capability outside the declaration");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Governed fixture
// ---------------------------------------------------------------------------

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "execution harness conformance",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string(),
    )
    .expect("manifest parses")
}

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://x".into()],
        writable_scopes: vec!["work://y".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 3,
        escalation_target: "human-review".into(),
    }
}

fn capability_registry() -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        NATIVE_CAP,
        Capability::deterministic(&["probe"], |args| {
            let probe = args.get("probe").and_then(|v| v.as_str()).unwrap_or("");
            Ok(format!("native:{probe}"))
        }),
    );
    registry.declare(
        RELAY_CAP,
        Capability::deterministic(&["probe"], |args| {
            let probe = args.get("probe").and_then(|v| v.as_str()).unwrap_or("");
            Ok(format!("relay:{probe}"))
        }),
    );
    registry
}

fn governed_runner() -> NodeRunner {
    NodeRunner::new(
        capability_registry(),
        permit_for(&[NODE_ID], &[NATIVE_CAP, RELAY_CAP]),
    )
}

fn step(capability: &str) -> HarnessStep {
    HarnessStep {
        step_id: format!("{capability}.step"),
        capability: capability.to_string(),
        args: serde_json::json!({"probe": "conformance"}),
    }
}

// ---------------------------------------------------------------------------
// Conformance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_distinct_implementations_pass_the_conformance_kit() {
    let manifest = manifest();
    let restrictions = restrictions();
    let digest = portable_digest();

    let native: Box<dyn ExecutionHarness> = Box::new(NativeHarness::new());
    let mut runner = governed_runner();
    let mut dispatch = HarnessDispatch::new(
        native.as_ref(),
        &manifest,
        &restrictions,
        Vec::new(),
        &mut runner,
    );
    let report =
        conformance_report(native.as_ref(), step(NATIVE_CAP), &mut dispatch, &digest).await;
    assert!(
        report.passed(),
        "native harness findings: {:?}",
        report.findings
    );
    assert_eq!(report.harness_id, "lite.harness.native");

    let scripted: Box<dyn ExecutionHarness> = Box::new(ScriptedProtocolHarness::new(&[
        "session.started",
        "turn.completed",
        "session.ended",
    ]));
    let mut runner = governed_runner();
    let mut dispatch = HarnessDispatch::new(
        scripted.as_ref(),
        &manifest,
        &restrictions,
        Vec::new(),
        &mut runner,
    );
    let report =
        conformance_report(scripted.as_ref(), step(RELAY_CAP), &mut dispatch, &digest).await;
    assert!(
        report.passed(),
        "protocol harness findings: {:?}",
        report.findings
    );
    assert_eq!(report.harness_id, "lite.harness.protocol");
}

#[tokio::test]
async fn declaration_and_behaviour_must_agree_on_continuity() {
    let manifest = manifest();
    let restrictions = restrictions();
    let digest = portable_digest();

    // This adapter declares resume support WITHOUT checkpoint import and
    // WITHOUT exporting anything. The kit must refuse the incoherent
    // declaration rather than accept the claim at face value.
    struct LiarHarness {
        declaration: HarnessDeclarationV1,
    }

    #[async_trait::async_trait]
    impl ExecutionHarness for LiarHarness {
        fn declaration(&self) -> &HarnessDeclarationV1 {
            &self.declaration
        }

        async fn run(
            &self,
            step: HarnessStep,
            dispatch: &mut HarnessDispatch<'_>,
        ) -> Result<HarnessOutcome> {
            let outcome = dispatch.call(&step.capability, step.args, &step.step_id)?;
            Ok(HarnessOutcome {
                harness_id: self.declaration.harness_id.clone(),
                output: outcome.output.clone(),
                evidence_entry_digest: outcome.evidence_entry.detail_digest.clone(),
                events: vec![HarnessEvent {
                    sequence: 0,
                    correlation_id: step.step_id.clone(),
                    kind: "claimed".to_string(),
                    detail_digest: "a".repeat(64),
                }],
                content_digest: None,
            }
            .sealed())
        }
    }

    let liar = LiarHarness {
        declaration: declaration(
            "lite.harness.liar",
            "liar",
            "v1",
            HarnessFamily::Subprocess,
            &[NATIVE_CAP],
            &[NATIVE_CAP],
            false,
            false,
            true,
        ),
    };
    let mut runner = governed_runner();
    let mut dispatch =
        HarnessDispatch::new(&liar, &manifest, &restrictions, Vec::new(), &mut runner);
    let report = conformance_report(&liar, step(NATIVE_CAP), &mut dispatch, &digest).await;
    let codes: Vec<&str> = report.findings.iter().map(|f| f.code).collect();
    assert!(
        codes.contains(&"HCONF-0007"),
        "incoherent resume declaration must be rejected: {:?}",
        report.findings
    );
    assert!(
        codes.contains(&"HCONF-0023"),
        "declared resume that cannot resume must be rejected: {:?}",
        report.findings
    );
    assert!(!report.passed());
}

// ---------------------------------------------------------------------------
// Structural laws
// ---------------------------------------------------------------------------

#[tokio::test]
async fn undeclared_capability_is_refused_before_the_runner_is_touched() {
    let manifest = manifest();
    let restrictions = restrictions();
    let native = NativeHarness::new();
    let mut runner = governed_runner();
    {
        let mut dispatch =
            HarnessDispatch::new(&native, &manifest, &restrictions, Vec::new(), &mut runner);
        let error = dispatch
            .call(
                "harness.not.declared",
                serde_json::json!({}),
                "undeclared-probe",
            )
            .expect_err("a capability outside the declaration must be refused");
        assert!(
            error.to_string().contains("outside its declaration"),
            "unexpected refusal: {error}"
        );
        assert_eq!(dispatch.calls(), 0, "no governed call may be issued");
    }
    assert!(
        runner.journal().is_empty(),
        "a refused call must not journal anything"
    );
}

#[tokio::test]
async fn registry_refuses_duplicates_and_unsealed_declarations() {
    struct UnsealedHarness {
        declaration: HarnessDeclarationV1,
    }

    #[async_trait::async_trait]
    impl ExecutionHarness for UnsealedHarness {
        fn declaration(&self) -> &HarnessDeclarationV1 {
            &self.declaration
        }

        async fn run(
            &self,
            _step: HarnessStep,
            _dispatch: &mut HarnessDispatch<'_>,
        ) -> Result<HarnessOutcome> {
            bail!("unsealed harness must never run")
        }
    }

    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(NativeHarness::new()))
        .expect("first registration succeeds");
    let duplicate = registry
        .register(Box::new(NativeHarness::new()))
        .expect_err("duplicate harness id must be refused");
    assert!(duplicate.to_string().contains("already registered"));

    let mut unsealed = declaration(
        "lite.harness.unsealed",
        "unsealed",
        "v1",
        HarnessFamily::Native,
        &[NATIVE_CAP],
        &[NATIVE_CAP],
        false,
        false,
        false,
    );
    unsealed.content_digest = None;
    let refused = registry
        .register(Box::new(UnsealedHarness {
            declaration: unsealed,
        }))
        .expect_err("an unsealed declaration must be refused");
    assert!(refused.to_string().contains("not sealed"));
}

#[tokio::test]
async fn catalogue_is_generated_and_order_independent() {
    let mut forward = HarnessRegistry::new();
    forward
        .register(Box::new(NativeHarness::new()))
        .expect("native registers");
    forward
        .register(Box::new(ScriptedProtocolHarness::new(&["a"])))
        .expect("protocol registers");

    let mut reverse = HarnessRegistry::new();
    reverse
        .register(Box::new(ScriptedProtocolHarness::new(&["a"])))
        .expect("protocol registers");
    reverse
        .register(Box::new(NativeHarness::new()))
        .expect("native registers");

    let a = forward.catalog();
    let b = reverse.catalog();
    assert_eq!(a.entries.len(), 2);
    assert_eq!(
        a.to_json().expect("catalogue serializes"),
        b.to_json().expect("catalogue serializes"),
        "catalogue must not depend on registration order"
    );
    assert_eq!(a.content_digest, b.content_digest);

    let ids: Vec<&str> = a.entries.iter().map(|e| e.harness_id.as_str()).collect();
    assert_eq!(ids, vec!["lite.harness.native", "lite.harness.protocol"]);
    assert_eq!(a.entries[0].family, HarnessFamily::Native);
    assert_eq!(a.entries[1].family, HarnessFamily::ProtocolBridged);
    assert_eq!(a.entries[1].checkpoint_export, Some(true));
    assert_eq!(a.entries[0].checkpoint_export, Some(false));
}

// Kept as a compile-time guard: the fixture functions above are only useful
// if the governed outcome type keeps carrying a journaled terminal entry.
#[allow(dead_code)]
fn outcome_evidence_binding(outcome: &NodeRunOutcome) -> &str {
    &outcome.evidence_entry.detail_digest
}
