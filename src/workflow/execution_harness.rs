//! Governed execution-harness seam (issue #154, slice A).
//!
//! A harness is a REPLACEABLE WORKER. PrometheOS Lite owns the durable work
//! state, evidence, decisions, workspace references, and policy; a harness
//! owns none of them. This module defines the Rust-native seam between the
//! two, plus the conformance foundation every adapter must pass.
//!
//! The seam is deliberately thin:
//!
//! - An adapter publishes a sealed [`HarnessDeclarationV1`] whose identity
//!   and capability claims map explicitly onto the published SOMA++
//!   `HarnessAdapter` fields and onto the SPEC 007 [`RuntimeCapabilitySet`].
//!   Lite declares no competing semantics and no Lite-only authority field.
//! - The ONLY sanctioned effect path is [`HarnessDispatch::call`]. It
//!   delegates to the governed [`NodeRunner`] nine-gate pipeline and refuses
//!   any capability outside the adapter's declaration, so an adapter cannot
//!   reach an effect around #117/#118. The governed context (manifest,
//!   restrictions, secrets) is fixed by the CALLER; the adapter chooses only
//!   which of its declared capabilities to invoke.
//! - Checkpoints carry a PORTABLE state digest, never process/PTY/chat
//!   state, and resume refuses a checkpoint that does not bind this harness,
//!   this revision, and this portable state.
//!
//! Slice A establishes the seam, the declaration-driven registry, and the
//! conformance kit. It does NOT claim that any concrete external adapter
//! (Claude Code, Codex, Goose, OpenCode, Aider, …) is implemented, and it
//! does not add a transport.
//!
//! `lite.harness.v1` is Lite-owned. Mapping to SOMA#80 happens through the
//! published document contracts already implemented in
//! [`crate::workflow::soma::adapters`]; this seam is the Rust execution
//! boundary, not a second protocol definition.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_runner::{NodeRunOutcome, NodeRunRequest, NodeRunner};
use crate::workflow::policy::LocalRestrictions;
use crate::workflow::soma::capability::{
    CompatibilityDecision, DecisionStatus, Identity, NegotiationInputs, RuntimeCapabilitySet,
    SUPPORTED_CAPABILITY_SCHEMA_VERSION, Substitution, WorkRequirements, resolve,
};
use crate::workflow::soma::contracts::AuthorityProfile;
use crate::workflow::soma::profile::Boundary;
use crate::workflow::workspace::{
    AdapterKind, WORKSPACE_SCHEMA_VERSION, WorkspaceManifestV1, WorkspaceMode,
};

/// Schema version of the harness seam contracts.
pub const HARNESS_SCHEMA_VERSION: &str = "1.0.0";

/// Canonical harness declarations owned by the implementation.
///
/// These are the authoritative source declarations from which the capability
/// catalog is generated. They represent the harness declaration seam families
/// (native, protocol) and correspond to the adapter implementations in the
/// conformance tests. There is no production HarnessRegistry in src/; these
/// declarations are the implementation-owned inventory, not a registry of
/// production executables.
pub fn canonical_harness_declarations() -> Vec<HarnessDeclarationV1> {
    use crate::workflow::soma::capability::Locality;
    vec![
        HarnessDeclarationV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: "lite.harness.native".to_string(),
            identity: Identity {
                name: "lite-native".to_string(),
                revision: "v1".to_string(),
            },
            family: HarnessFamily::Native,
            declared_capabilities: vec!["harness.native.probe".to_string()],
            claimed_capabilities: vec!["harness.native.probe".to_string()],
            capabilities: RuntimeCapabilitySet {
                id: "lite.harness.native.caps".to_string(),
                schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
                version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
                identity: Identity {
                    name: "lite-native".to_string(),
                    revision: "v1".to_string(),
                },
                declared_at: "2026-10-22T00:00:00Z".to_string(),
                spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
                adapter_families: Vec::new(),
                effect_classes: vec!["process-exec".to_string()],
                workspace: None,
                checkpoint_import: Some(false),
                checkpoint_export: Some(false),
                resume_support: Some(false),
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
            },
            content_digest: None,
        }
        .sealed(),
        HarnessDeclarationV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: "lite.harness.protocol".to_string(),
            identity: Identity {
                name: "lite-protocol".to_string(),
                revision: "v1".to_string(),
            },
            family: HarnessFamily::ProtocolBridged,
            declared_capabilities: vec!["harness.protocol.relay".to_string()],
            claimed_capabilities: vec!["harness.protocol.relay".to_string()],
            capabilities: RuntimeCapabilitySet {
                id: "lite.harness.protocol.caps".to_string(),
                schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
                version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
                identity: Identity {
                    name: "lite-protocol".to_string(),
                    revision: "v1".to_string(),
                },
                declared_at: "2026-10-22T00:00:00Z".to_string(),
                spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
                adapter_families: Vec::new(),
                effect_classes: vec!["process-exec".to_string()],
                workspace: None,
                checkpoint_import: Some(true),
                checkpoint_export: Some(true),
                resume_support: Some(true),
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
            },
            content_digest: None,
        }
        .sealed(),
    ]
}

/// Schema version of a conformance report.
pub const HARNESS_CONFORMANCE_VERSION: &str = "1.0.0";

/// True when `s` is a lowercase 64-hex digest.
fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Canonical digest of any serializable value, as a `Result`.
///
/// Used on evidence paths, where a digest failure must fail closed instead of
/// panicking.
fn canonical_json_digest<T: Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value).context("value serializes")?;
    crate::workflow::soma::try_canonical_digest(&value)
        .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
}

// ---------------------------------------------------------------------------
// Declaration
// ---------------------------------------------------------------------------

/// Implementation family of an adapter. Closed set: a new family is a schema
/// change, not an adapter-local string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessFamily {
    Native,
    Subprocess,
    LocalModel,
    RemoteService,
    ProtocolBridged,
}

impl HarnessFamily {
    /// The exact string emitted in declarations, catalogues and evidence.
    pub const fn as_str(self) -> &'static str {
        match self {
            HarnessFamily::Native => "native",
            HarnessFamily::Subprocess => "subprocess",
            HarnessFamily::LocalModel => "local-model",
            HarnessFamily::RemoteService => "remote-service",
            HarnessFamily::ProtocolBridged => "protocol-bridged",
        }
    }
}

/// Versioned declaration owned by ONE adapter implementation.
///
/// `declared_capabilities` is the universe the adapter may invoke through
/// [`HarnessDispatch`]; `claimed_capabilities` is the subset it currently
/// supports. A claim outside the declaration is refused (the same rule as
/// SOMA-ADAPT-0001), so an adapter cannot widen its authority surface by
/// publishing a claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessDeclarationV1 {
    pub schema_version: String,
    /// Stable Lite identity of this adapter.
    pub harness_id: String,
    /// Published SOMA++ `HarnessAdapter.identity` (name + revision).
    pub identity: Identity,
    pub family: HarnessFamily,
    /// Capabilities the adapter is allowed to invoke.
    pub declared_capabilities: Vec<String>,
    /// Capabilities the adapter currently claims to support.
    pub claimed_capabilities: Vec<String>,
    /// SPEC 007 declaration this adapter owns. Its `identity` must equal
    /// `identity` above; the conformance kit refuses a divergence rather than
    /// letting one document describe two implementations.
    pub capabilities: RuntimeCapabilitySet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl HarnessDeclarationV1 {
    /// Build + seal a declaration.
    pub fn sealed(mut self) -> Self {
        self.content_digest = None;
        let digest = self.compute_digest();
        self.content_digest = Some(digest);
        self
    }

    /// Canonical digest over every field except `contentDigest`.
    ///
    /// Fallible variant: used wherever the declaration may have been parsed
    /// from untrusted input, so a digest failure fails closed.
    pub fn try_compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("declaration serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    /// Canonical digest over every field except `contentDigest`.
    ///
    /// Panic-on-error wrapper retained for Lite-constructed declarations.
    /// Untrusted input must go through [`Self::try_compute_digest`].
    pub fn compute_digest(&self) -> String {
        let mut value = serde_json::to_value(self).expect("declaration serializes");
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .expect("canonical digest of a Lite-constructed declaration cannot fail")
    }

    /// Fail-closed parse: version gate, structural checks, claim law, digest
    /// verification. Refusals are input errors, never silent repair.
    /// Fail-closed validation of a sealed declaration.
    ///
    /// Returns a list of findings (empty = valid). Used by the conformance kit
    /// and by the Slice C drift guard.
    pub fn validate(&self) -> Vec<String> {
        let mut findings = Vec::new();
        if self.schema_version != HARNESS_SCHEMA_VERSION {
            findings.push(format!(
                "unsupported schema version {} (expected {HARNESS_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if self.harness_id.trim().is_empty() {
            findings.push("empty harnessId".to_string());
        }
        if self.identity.name.trim().is_empty() || self.identity.revision.trim().is_empty() {
            findings.push("empty identity name or revision".to_string());
        }
        if self.declared_capabilities.is_empty() {
            findings.push("no declared capabilities".to_string());
        }
        let declared_set: BTreeSet<&str> = self
            .declared_capabilities
            .iter()
            .map(String::as_str)
            .collect();
        if declared_set.len() != self.declared_capabilities.len() {
            findings.push("duplicate declared capability".to_string());
        }
        for c in &self.claimed_capabilities {
            if !declared_set.contains(c.as_str()) {
                findings.push(format!("claimed capability {c:?} not in declared set"));
            }
        }
        if self.capabilities.identity != self.identity {
            findings.push("capability identity diverges from declaration identity".to_string());
        }
        // Stale version check: spec_versions must be non-empty
        if self.capabilities.spec_versions.is_empty() {
            findings.push("no supported spec versions declared".to_string());
        }
        findings
    }

    /// Fail-closed parse: version gate, structural checks, claim law, digest
    /// verification. Refusals are input errors, never silent repair.
    pub fn parse_json(text: &str) -> Result<Self> {
        let d: Self = serde_json::from_str(text).context("harness declaration parse failed")?;
        if d.schema_version != HARNESS_SCHEMA_VERSION {
            bail!(
                "unsupported harness declaration schema version {} (supported {HARNESS_SCHEMA_VERSION})",
                d.schema_version
            );
        }
        if d.harness_id.trim().is_empty() {
            bail!("harness declaration requires a non-empty harnessId");
        }
        if d.identity.name.trim().is_empty() || d.identity.revision.trim().is_empty() {
            bail!("harness declaration requires a non-empty identity name and revision");
        }
        if d.declared_capabilities.is_empty() {
            bail!("harness declaration requires at least one declared capability");
        }
        let declared: BTreeSet<&str> = d.declared_capabilities.iter().map(String::as_str).collect();
        if declared.len() != d.declared_capabilities.len() {
            bail!("harness declaration lists a capability more than once");
        }
        if d.claimed_capabilities
            .iter()
            .any(|c| !declared.contains(c.as_str()))
        {
            bail!("harness declaration claims a capability outside its declared set");
        }
        if d.capabilities.identity != d.identity {
            bail!("harness declaration identity diverges from its capability declaration");
        }
        let audit = d.capabilities.audit();
        if let Some(first) = audit.first() {
            bail!(
                "harness capability declaration is not audit-clean: {} {}",
                first.code,
                first.message
            );
        }
        match &d.content_digest {
            None => bail!("harness declaration missing contentDigest"),
            Some(digest) if *digest != d.try_compute_digest()? => {
                bail!("harness declaration contentDigest does not verify")
            }
            Some(_) => {}
        }
        Ok(d)
    }
}

// ---------------------------------------------------------------------------
// Work, events, outcome
// ---------------------------------------------------------------------------

/// One bounded unit of work handed to an adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessStep {
    /// Stable step identity; also the idempotency key of the governed call.
    pub step_id: String,
    /// The declared capability this step is expected to invoke.
    pub capability: String,
    pub args: serde_json::Value,
}

/// One observable adapter event.
///
/// Events are OBSERVABLE execution state, never durable work truth: nothing
/// here is required to resume a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessEvent {
    pub sequence: u64,
    pub correlation_id: String,
    pub kind: String,
    pub detail_digest: String,
}

/// Typed terminal result of one governed step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessOutcome {
    pub harness_id: String,
    /// Gate-7-redacted effect output retained as evidence content.
    pub output: String,
    /// Digest of the journaled terminal entry (the evidence binding).
    pub evidence_entry_digest: String,
    pub events: Vec<HarnessEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl HarnessOutcome {
    pub fn sealed(mut self) -> Self {
        self.content_digest = None;
        let digest = self.compute_digest();
        self.content_digest = Some(digest);
        self
    }

    pub fn try_compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("outcome serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    pub fn compute_digest(&self) -> String {
        let mut value = serde_json::to_value(self).expect("outcome serializes");
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .expect("canonical digest of a Lite-constructed outcome cannot fail")
    }

    /// Fail-closed verification of a sealed outcome.
    pub fn verify(&self) -> Result<()> {
        match &self.content_digest {
            None => bail!("harness outcome missing contentDigest"),
            Some(digest) if *digest != self.try_compute_digest()? => {
                bail!("harness outcome contentDigest does not verify")
            }
            Some(_) => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Portable checkpoint
// ---------------------------------------------------------------------------

/// Portable continuation binding for one adapter.
///
/// The checkpoint does NOT carry process, PTY, chat or conversation state.
/// It binds the harness identity + revision to a digest of the portable work
/// state; the durable state itself lives in the store, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessCheckpointV1 {
    pub schema_version: String,
    pub harness_id: String,
    pub harness_revision: String,
    pub portable_state_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl HarnessCheckpointV1 {
    pub fn sealed(mut self) -> Self {
        self.content_digest = None;
        let digest = self.compute_digest();
        self.content_digest = Some(digest);
        self
    }

    pub fn try_compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("checkpoint serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    pub fn compute_digest(&self) -> String {
        let mut value = serde_json::to_value(self).expect("checkpoint serializes");
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .expect("canonical digest of a Lite-constructed checkpoint cannot fail")
    }

    /// Fail-closed verification: version, identity, digest shape, binding.
    pub fn verify(&self) -> Result<()> {
        if self.schema_version != HARNESS_SCHEMA_VERSION {
            bail!(
                "unsupported harness checkpoint schema version {} (supported {HARNESS_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        if self.harness_id.trim().is_empty() || self.harness_revision.trim().is_empty() {
            bail!("harness checkpoint missing harness identity or revision");
        }
        if !is_hex64(&self.portable_state_digest) {
            bail!("harness checkpoint portableStateDigest must be 64 lowercase hex");
        }
        match &self.content_digest {
            None => bail!("harness checkpoint missing contentDigest"),
            Some(digest) if *digest != self.try_compute_digest()? => {
                bail!("harness checkpoint contentDigest does not verify")
            }
            Some(_) => Ok(()),
        }
    }
}

/// Verified continuation returned by [`ExecutionHarness::resume`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessContinuation {
    pub harness_id: String,
    pub harness_revision: String,
    pub portable_state_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_step: Option<String>,
}

// ---------------------------------------------------------------------------
// The seam
// ---------------------------------------------------------------------------

/// A replaceable governed execution adapter.
///
/// Implementations receive work and a dispatch bound to the CALLER's governed
/// context. They cannot construct a second context, spawn an ungated effect,
/// or widen their declared capability surface.
#[async_trait::async_trait]
pub trait ExecutionHarness: Send + Sync {
    /// The sealed declaration this implementation owns.
    fn declaration(&self) -> &HarnessDeclarationV1;

    /// Execute one bounded step. Every effect must reach
    /// [`HarnessDispatch::call`].
    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome>;

    /// Export a portable continuation binding. The default REFUSES: an
    /// adapter that does not declare `checkpointExport` has no checkpoint.
    async fn checkpoint(
        &self,
        _outcome: &HarnessOutcome,
        _portable_state_digest: &str,
    ) -> Result<HarnessCheckpointV1> {
        bail!(
            "harness {:?} does not declare checkpointExport; no continuation can be exported",
            self.declaration().harness_id
        )
    }

    /// Verify a checkpoint against this implementation and the supplied
    /// portable state digest. The default REFUSES: an adapter that does not
    /// declare resume support cannot continue a run.
    async fn resume(
        &self,
        _checkpoint: &HarnessCheckpointV1,
        _portable_state_digest: &str,
    ) -> Result<HarnessContinuation> {
        bail!(
            "harness {:?} does not declare resume support; a checkpoint cannot be resumed",
            self.declaration().harness_id
        )
    }
}

/// The only sanctioned effect path for an adapter.
///
/// The caller fixes the governed context (manifest, local restrictions,
/// known secrets) and the runner. The adapter chooses only which DECLARED
/// capability to invoke, so the nine gates cannot be bypassed by an adapter
/// that "just needs to run a process".
pub struct HarnessDispatch<'a> {
    harness_id: String,
    declared: BTreeSet<String>,
    manifest: &'a NodeManifestV1,
    local_restrictions: &'a LocalRestrictions,
    known_secrets: Vec<String>,
    runner: &'a mut NodeRunner,
    calls: usize,
}

impl<'a> HarnessDispatch<'a> {
    pub fn new(
        harness: &dyn ExecutionHarness,
        manifest: &'a NodeManifestV1,
        local_restrictions: &'a LocalRestrictions,
        known_secrets: Vec<String>,
        runner: &'a mut NodeRunner,
    ) -> Self {
        let declaration = harness.declaration();
        Self {
            harness_id: declaration.harness_id.clone(),
            declared: declaration.declared_capabilities.iter().cloned().collect(),
            manifest,
            local_restrictions,
            known_secrets,
            runner,
            calls: 0,
        }
    }

    /// Number of governed calls this dispatch has issued.
    pub fn calls(&self) -> usize {
        self.calls
    }

    /// The authority-surface gate. Public so conformance can prove the
    /// refusal point itself, and so callers can pre-check a step.
    pub fn ensure_declared(&self, capability: &str) -> Result<()> {
        if !self.declared.contains(capability) {
            bail!(
                "harness {:?} invoked capability {capability:?} outside its declaration; \
                 an adapter is never an authority source",
                self.harness_id
            );
        }
        Ok(())
    }

    /// Invoke one declared capability through the governed runner.
    pub fn call(
        &mut self,
        capability: &str,
        args: serde_json::Value,
        idempotency_key: &str,
    ) -> Result<NodeRunOutcome> {
        self.ensure_declared(capability)?;
        self.calls += 1;
        let request = NodeRunRequest {
            manifest: self.manifest,
            local_restrictions: self.local_restrictions,
            capability: capability.to_string(),
            args,
            idempotency_key: idempotency_key.to_string(),
            known_secrets: self.known_secrets.clone(),
        };
        self.runner.execute(request)
    }
}

// ---------------------------------------------------------------------------
// Registry + generated catalogue
// ---------------------------------------------------------------------------

/// Declaration-driven registry of adapters.
///
/// There is no separate catalogue file to drift: the catalogue is generated
/// from the declarations the implementations themselves return, at the moment
/// it is asked for.
#[derive(Default)]
pub struct HarnessRegistry {
    entries: BTreeMap<String, Box<dyn ExecutionHarness>>,
}

impl HarnessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an adapter. A duplicate id, an unsealed declaration, or a
    /// declaration that does not verify is refused.
    pub fn register(&mut self, harness: Box<dyn ExecutionHarness>) -> Result<()> {
        let declaration = harness.declaration();
        if declaration.harness_id.trim().is_empty() {
            bail!("cannot register a harness with an empty id");
        }
        // Fail-closed: reject malformed declarations even if sealed.
        let findings = declaration.validate();
        if !findings.is_empty() {
            bail!(
                "cannot register harness {:?}: declaration is invalid: {}",
                declaration.harness_id,
                findings.join("; ")
            );
        }
        // Enforce canonical capability audit (SOMA SPEC 007).
        let audit = declaration.capabilities.audit();
        if let Some(first) = audit.first() {
            bail!(
                "cannot register harness {:?}: capability declaration is not audit-clean: {} {}",
                declaration.harness_id,
                first.code,
                first.message
            );
        }
        if declaration.content_digest.as_deref() != Some(&declaration.try_compute_digest()?) {
            bail!(
                "cannot register harness {:?}: declaration is not sealed or does not verify",
                declaration.harness_id
            );
        }
        if self.entries.contains_key(&declaration.harness_id) {
            bail!(
                "cannot register harness {:?}: id already registered",
                declaration.harness_id
            );
        }
        self.entries.insert(declaration.harness_id.clone(), harness);
        Ok(())
    }

    pub fn get(&self, harness_id: &str) -> Result<&dyn ExecutionHarness> {
        self.entries
            .get(harness_id)
            .map(|h| h.as_ref())
            .ok_or_else(|| anyhow::anyhow!("harness {harness_id:?} is not registered"))
    }

    /// Generate the catalogue from the registered declarations. Entries are
    /// ordered by `harnessId`, so two registries holding the same adapters
    /// produce byte-identical catalogues regardless of insertion order.
    pub fn catalog(&self) -> HarnessCatalog {
        let entries = self
            .entries
            .values()
            .map(|harness| {
                let d = harness.declaration();
                HarnessCatalogEntry {
                    harness_id: d.harness_id.clone(),
                    family: d.family,
                    name: d.identity.name.clone(),
                    revision: d.identity.revision.clone(),
                    declaration_digest: d.compute_digest(),
                    declared_capabilities: d.declared_capabilities.clone(),
                    claimed_capabilities: d.claimed_capabilities.clone(),
                    effect_classes: d.capabilities.effect_classes.clone(),
                    spec_versions: d.capabilities.spec_versions.clone(),
                    adapter_families: d
                        .capabilities
                        .adapter_families
                        .iter()
                        .map(|f| format!("{f:?}"))
                        .collect(),
                    workspace: d.capabilities.workspace,
                    max_structured_input: d.capabilities.max_structured_input.clone(),
                    max_structured_output: d.capabilities.max_structured_output.clone(),
                    budget_ceilings: d.capabilities.budget_ceilings.clone(),
                    locality: d.capabilities.locality,
                    checkpoint_export: d.capabilities.checkpoint_export,
                    checkpoint_import: d.capabilities.checkpoint_import,
                    resume_support: d.capabilities.resume_support,
                    event_streaming: d.capabilities.event_streaming,
                    read_projection: d.capabilities.read_projection,
                    privacy_classes: d.capabilities.privacy_classes.clone(),
                    evidence_attachment: d.capabilities.evidence_attachment,
                    validation: d.capabilities.validation,
                    approval_escalation: d.capabilities.approval_escalation,
                    unclaimed_capabilities: d
                        .declared_capabilities
                        .iter()
                        .filter(|c| !d.claimed_capabilities.contains(c))
                        .cloned()
                        .collect(),
                    over_claimed: d
                        .claimed_capabilities
                        .iter()
                        .any(|c| !d.declared_capabilities.contains(c)),
                }
            })
            .collect();
        HarnessCatalog {
            schema_version: HARNESS_SCHEMA_VERSION.into(),
            conformance_version: HARNESS_CONFORMANCE_VERSION.into(),
            entries,
            content_digest: None,
        }
        .sealed()
    }
}

// ---------------------------------------------------------------------------
// Selection (SPEC 007 negotiation binding)
// ---------------------------------------------------------------------------

/// Schema version of the harness-selection evidence.
pub const HARNESS_SELECTION_SCHEMA_VERSION: &str = "1.0.0";

/// A request to select exactly ONE registered harness for one work item.
///
/// The declaration is metadata; the decision is the published SPEC 007
/// negotiation over it. `pinned_harness_id` is a requirement, not a hint: when
/// it names no registered adapter the request is refused, and when it is
/// absent an ambiguity is refused rather than resolved by registration order.
pub struct HarnessSelectionRequest<'a> {
    pub selection_id: String,
    pub requirements: &'a WorkRequirements,
    pub authority_declared: &'a AuthorityProfile,
    pub authority_effective: &'a AuthorityProfile,
    /// Injected instant (RFC 3339 UTC-Z); the seam never reads a clock.
    pub evaluated_at: String,
    pub substitutions: Vec<Substitution>,
    pub pinned_harness_id: Option<&'a str>,
    /// The actual execution workspace (#171) that will receive the harness's effects.
    /// When present, selection enforces workspace compatibility and binds the
    /// evidence to this concrete workspace instance.
    pub workspace: Option<&'a WorkspaceManifestV1>,
}

/// Trusted inputs for semantic re-verification of selection evidence.
///
/// These are the values the verifier independently holds or can re-derive: the
/// registered adapter (hence its sealed declaration and capability set), the
/// work requirements, and the authority snapshots that were in force. They are
/// deliberately NOT read back out of the evidence, so a substituted snapshot
/// cannot validate itself.
pub struct HarnessSelectionTrust<'a> {
    pub harness: &'a dyn ExecutionHarness,
    pub requirements: &'a WorkRequirements,
    pub authority_declared: &'a AuthorityProfile,
    pub authority_effective: &'a AuthorityProfile,
    /// The original selection ID that was used to construct the request.
    pub selection_id: &'a str,
    /// The original evaluation instant that was used to construct the request.
    pub evaluated_at: &'a str,
    /// The original substitutions that were used to construct the request.
    pub substitutions: &'a [Substitution],
    /// The actual workspace (#171) the selection was bound to, if any.
    pub workspace: Option<&'a WorkspaceManifestV1>,
}

/// Workspace evidence bound to a harness selection. Captures the concrete
/// #171 workspace instance so the selection decision can be verified against
/// the actual execution workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceEvidenceV1 {
    pub workspace_id: String,
    pub adapter: String,
    pub adapter_revision: String,
    pub mode: String,
    #[serde(rename = "manifestDigest")]
    pub manifest_digest: String,
}

/// Durable evidence that one harness revision was selected for one work item.
///
/// It binds the harness id, the published identity revision, the declaration
/// digest and the negotiation decision, so a later reader can re-verify the
/// selection without re-running it and without trusting a log line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessSelectionV1 {
    pub schema_version: String,
    pub selection_id: String,
    pub harness_id: String,
    pub identity: Identity,
    pub family: HarnessFamily,
    pub declaration_digest: String,
    pub capability_set_digest: String,
    pub requirements_digest: String,
    pub decision_id: String,
    pub decision_status: DecisionStatus,
    pub evaluated_at: String,
    pub authority_declared_digest: String,
    pub authority_effective_digest: String,
    pub decision_digest: String,
    pub substitutions: Vec<Substitution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceEvidenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl HarnessSelectionV1 {
    /// Build + seal selection evidence.
    ///
    /// Fallible: sealing is reachable from `select`, and a canonical-digest
    /// failure must surface as an error rather than a panic.
    pub fn try_sealed(mut self) -> Result<Self> {
        self.content_digest = None;
        let digest = self.try_compute_digest()?;
        self.content_digest = Some(digest);
        Ok(self)
    }

    pub fn try_compute_digest(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).context("selection serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .map_err(|e| anyhow::anyhow!("canonical digest failed: {e}"))
    }

    /// Fail-closed re-verification of durable evidence. A rewritten revision,
    /// declaration digest, or decision does not verify.
    pub fn verify(&self) -> Result<()> {
        if self.schema_version != HARNESS_SELECTION_SCHEMA_VERSION {
            bail!(
                "unsupported harness selection schema version {} (supported {HARNESS_SELECTION_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        for (label, value) in [
            ("declarationDigest", &self.declaration_digest),
            ("capabilitySetDigest", &self.capability_set_digest),
            ("requirementsDigest", &self.requirements_digest),
            ("authorityDeclaredDigest", &self.authority_declared_digest),
            ("authorityEffectiveDigest", &self.authority_effective_digest),
            ("decisionDigest", &self.decision_digest),
        ] {
            if !is_hex64(value) {
                bail!("HSEL-0005: harness selection {label} is not a 64-hex digest");
            }
        }
        if let Some(ws) = &self.workspace {
            if !is_hex64(&ws.manifest_digest) {
                bail!(
                    "HSEL-0005: harness selection workspace manifestDigest is not a 64-hex digest"
                );
            }
            if ws.workspace_id.trim().is_empty()
                || ws.adapter.trim().is_empty()
                || ws.adapter_revision.trim().is_empty()
                || ws.mode.trim().is_empty()
            {
                bail!("HSEL-0005: harness selection workspace evidence has empty required fields");
            }
        }
        // RFC 3339 parsing is delegated to `chrono`: it validates separators,
        // calendar ranges, month lengths and leap years, and refuses malformed
        // input. Evidence must be canonical UTC (`Z`), so a numeric zero offset
        // (`+00:00`) is refused as well.
        if !self.evaluated_at.ends_with('Z') {
            bail!("HSEL-0005: harness selection evaluatedAt must be UTC with a Z suffix");
        }
        // Reject non-canonical separators before chrono to prevent lenient parsing.
        // RFC 3339 requires YYYY-MM-DDTHH:MM:SSZ with '-' '-' 'T' ':' ':' in fixed positions.
        if self.evaluated_at.len() != 20
            || self.evaluated_at.chars().nth(4) != Some('-')
            || self.evaluated_at.chars().nth(7) != Some('-')
            || self.evaluated_at.chars().nth(10) != Some('T')
            || self.evaluated_at.chars().nth(13) != Some(':')
            || self.evaluated_at.chars().nth(16) != Some(':')
            || self.evaluated_at.chars().nth(19) != Some('Z')
        {
            bail!("HSEL-0005: harness selection evaluatedAt has invalid separators");
        }
        let evaluated_at = chrono::DateTime::parse_from_rfc3339(&self.evaluated_at)
            .map_err(|error| {
                anyhow::anyhow!(
                    "HSEL-0005: harness selection evaluatedAt is not a valid RFC 3339 timestamp: {error}"
                )
            })?;
        if evaluated_at.offset().local_minus_utc() != 0 {
            bail!("HSEL-0005: harness selection evaluatedAt is not UTC");
        }
        // `chrono` represents a leap second as a nanosecond overflow. Selection
        // evidence never records one, so refuse it rather than carry an
        // ambiguous second.
        if evaluated_at.timestamp_subsec_nanos() >= 1_000_000_000 {
            bail!("HSEL-0005: harness selection evaluatedAt must not use a leap second");
        }
        match &self.content_digest {
            None => bail!("HSEL-0005: harness selection evidence is unsealed"),
            Some(digest) if *digest != self.try_compute_digest()? => {
                bail!("HSEL-0005: harness selection contentDigest does not verify")
            }
            Some(_) => Ok(()),
        }
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn parse_json(text: &str) -> Result<Self> {
        let selection: Self =
            serde_json::from_str(text).context("harness selection parse failed")?;
        selection.verify()?;
        Ok(selection)
    }

    /// Semantic re-verification against trusted inputs.
    ///
    /// [`Self::verify`] establishes only integrity: the digests are well formed
    /// and the content digest covers the recorded bytes. It cannot establish
    /// that the recorded decision was actually produced from the claimed
    /// authority, capabilities and requirements, because those inputs are not
    /// carried in the evidence.
    ///
    /// `verify_against` closes that gap. It recomputes every negotiation-input
    /// digest from the trusted values, then re-runs [`resolve`] with the
    /// recorded evaluation instant and requires the reproduced decision to
    /// match the recorded status and canonical digest. A substituted authority
    /// snapshot, capability set, requirement set, evaluation instant or
    /// decision therefore fails closed with `HSEL-0008`.
    pub fn verify_against(&self, trust: &HarnessSelectionTrust<'_>) -> Result<()> {
        self.verify()?;

        let declaration = trust.harness.declaration();
        if declaration.harness_id != self.harness_id {
            bail!("HSEL-0008: selection harnessId does not match the trusted harness");
        }
        if declaration.identity != self.identity {
            bail!("HSEL-0008: selection identity does not match the trusted harness");
        }
        if declaration.family != self.family {
            bail!("HSEL-0008: selection family does not match the trusted harness");
        }
        if declaration.try_compute_digest()? != self.declaration_digest {
            bail!("HSEL-0008: selection declarationDigest does not match the trusted declaration");
        }
        if declaration
            .capabilities
            .canonical_digest()
            .map_err(|e| anyhow::anyhow!("HSEL-0008: {e}"))?
            != self.capability_set_digest
        {
            bail!(
                "HSEL-0008: selection capabilitySetDigest does not match the trusted capability set"
            );
        }
        if trust
            .requirements
            .canonical_digest()
            .map_err(|e| anyhow::anyhow!("HSEL-0008: {e}"))?
            != self.requirements_digest
        {
            bail!(
                "HSEL-0008: selection requirementsDigest does not match the trusted requirements"
            );
        }
        if canonical_json_digest(trust.authority_declared)? != self.authority_declared_digest {
            bail!(
                "HSEL-0008: selection authorityDeclaredDigest does not match the trusted authority"
            );
        }
        if canonical_json_digest(trust.authority_effective)? != self.authority_effective_digest {
            bail!(
                "HSEL-0008: selection authorityEffectiveDigest does not match the trusted authority"
            );
        }
        if self.decision_status != DecisionStatus::Compatible {
            bail!("HSEL-0008: selection evidence records a non-compatible decision");
        }

        // Verify that the evidence records the same negotiation context that
        // the trusted inputs claim. This prevents a resealed record from
        // passing verification against a different request.
        if self.selection_id != trust.selection_id {
            bail!("HSEL-0008: selection selectionId does not match the trusted selection");
        }
        if self.evaluated_at != trust.evaluated_at {
            bail!("HSEL-0008: selection evaluatedAt does not match the trusted evaluation instant");
        }
        if self.substitutions != trust.substitutions {
            bail!("HSEL-0008: selection substitutions do not match the trusted substitutions");
        }
        let expected_decision_id = format!("{}/{}", trust.selection_id, declaration.harness_id);
        if self.decision_id != expected_decision_id {
            bail!("HSEL-0008: selection decisionId does not match the trusted decision");
        }

        // Re-run the resolver with the trusted inputs and the recorded
        // evaluation instant. The decision must reproduce byte for byte.
        let decision = resolve(&NegotiationInputs {
            id: format!("{}/{}", trust.selection_id, declaration.harness_id),
            requirements: trust.requirements,
            capabilities: &declaration.capabilities,
            authority_declared: trust.authority_declared,
            authority_effective: trust.authority_effective,
            evaluated_at: trust.evaluated_at.to_string(),
            substitutions: trust.substitutions.to_vec(),
        })
        .map_err(|error| {
            anyhow::anyhow!("HSEL-0008: trusted inputs do not reproduce a decision: {error}")
        })?;
        if decision.status != self.decision_status {
            bail!("HSEL-0008: selection decisionStatus does not match the reproduced decision");
        }
        if canonical_json_digest(&decision)? != self.decision_digest {
            bail!("HSEL-0008: selection decisionDigest does not match the reproduced decision");
        }
        // Verify workspace binding when selection evidence includes a workspace reference.
        if self.workspace.is_some() {
            let Some(trust_workspace) = trust.workspace else {
                bail!(
                    "HSEL-0008: trusted inputs missing workspace reference but selection evidence includes one"
                );
            };
            let manifest = trust_workspace;
            if manifest.schema_version != WORKSPACE_SCHEMA_VERSION {
                bail!("HSEL-0008: trusted workspace schema version does not match");
            }
            // Build expected workspace evidence from the trusted manifest.
            let adapter_family_str = match manifest.adapter {
                AdapterKind::GitWorktree => "git-worktree",
                AdapterKind::ExistingReadOnly => "existing-read-only",
            };
            let mode_boundary = match manifest.mode {
                WorkspaceMode::ReadOnly => "read",
                WorkspaceMode::Writable => "write",
            };
            let manifest_digest = manifest.compute_digest();
            let expected = WorkspaceEvidenceV1 {
                workspace_id: manifest.workspace_id.clone(),
                adapter: adapter_family_str.to_string(),
                adapter_revision: manifest.adapter_revision.clone(),
                mode: mode_boundary.to_string(),
                manifest_digest,
            };
            match &self.workspace {
                Some(actual) if actual == &expected => {}
                _ => bail!(
                    "HSEL-0008: selection workspace evidence does not match the trusted workspace"
                ),
            }
        }
        Ok(())
    }
}

impl HarnessRegistry {
    /// Select exactly one registered harness for `request.requirements`.
    ///
    /// Fail-closed refusals, all before the adapter is touched:
    ///
    /// - `HSEL-0001` — no harness is registered;
    /// - `HSEL-0002` — the pinned harness is not registered;
    /// - `HSEL-0003` — no registered harness satisfies the requirements (the
    ///   negotiation diagnostics are carried in the message);
    /// - `HSEL-0004` — more than one harness satisfies them and the caller did
    ///   not pin one, so the choice would otherwise be registration order.
    pub fn select(&self, request: &HarnessSelectionRequest<'_>) -> Result<HarnessSelectionV1> {
        if request.selection_id.trim().is_empty() {
            bail!("HSEL-0006: harness selection requires a non-empty selection id");
        }
        if self.entries.is_empty() {
            bail!("HSEL-0001: no harness is registered, so no selection can be made");
        }

        let candidates: Vec<&dyn ExecutionHarness> = match request.pinned_harness_id {
            Some(harness_id) => {
                let harness = self
                    .entries
                    .get(harness_id)
                    .map(|h| h.as_ref())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "HSEL-0002: pinned harness {harness_id:?} is not registered"
                        )
                    })?;
                vec![harness]
            }
            None => self.entries.values().map(|h| h.as_ref()).collect(),
        };

        let mut compatible: Vec<(&dyn ExecutionHarness, CompatibilityDecision)> = Vec::new();
        let mut refusals: Vec<String> = Vec::new();

        // --- Workspace compatibility enforcement (#171).
        // Validate the workspace manifest integrity if a workspace is requested.
        let workspace_manifest = request.workspace;
        if let Some(manifest) = workspace_manifest {
            // Schema version gate.
            if manifest.schema_version != WORKSPACE_SCHEMA_VERSION {
                bail!(
                    "HSEL-0009: unsupported workspace schema version {} (expected {WORKSPACE_SCHEMA_VERSION})",
                    manifest.schema_version
                );
            }
            // Content-digest integrity gate (manifest must be sealed).
            let expected_digest = manifest.compute_digest();
            if manifest.content_digest.as_deref() != Some(&expected_digest) {
                bail!("HSEL-0009: workspace manifest contentDigest does not verify");
            }
            // Mode satisfaction gate: if the work requires a workspace boundary,
            // the manifest's mode must be sufficient.
            if let Some(req_boundaries) = request.requirements.workspace {
                let rank_req = match req_boundaries.workspace {
                    Some(Boundary::Write) => 2u8,
                    Some(Boundary::Read) => 1u8,
                    Some(Boundary::None) | None => 0,
                };
                let rank_manifest = match manifest.mode {
                    WorkspaceMode::Writable => 2u8,
                    WorkspaceMode::ReadOnly => 1u8,
                };
                if rank_manifest < rank_req {
                    bail!(
                        "HSEL-0009: workspace mode ({:?}) cannot satisfy required boundary ({})",
                        manifest.mode,
                        if rank_req == 2 { "writable" } else { "read" }
                    );
                }
            }
            }

        for harness in candidates {
            let declaration = harness.declaration();

            // Skip candidate if it doesn't support the required workspace adapter.
            if let Some(manifest) = workspace_manifest {
                let adapter_family_str = match manifest.adapter {
                    AdapterKind::GitWorktree => "git-worktree",
                    AdapterKind::ExistingReadOnly => "existing-read-only",
                };
                let manifest_rev = &manifest.adapter_revision;
                let harness_supports = declaration
                    .capabilities
                    .adapter_families
                    .iter()
                    .any(|f| f.family == adapter_family_str && f.revision == *manifest_rev);
                if !harness_supports {
                    refusals.push(format!(
                        "{} refused: workspace adapter {} / revision {} not declared",
                        declaration.harness_id, adapter_family_str, manifest_rev
                    ));
                    continue;
                }
            }

            let decision = resolve(&NegotiationInputs {
                id: format!("{}/{}", request.selection_id, declaration.harness_id),
                requirements: request.requirements,
                capabilities: &declaration.capabilities,
                authority_declared: request.authority_declared,
                authority_effective: request.authority_effective,
                evaluated_at: request.evaluated_at.clone(),
                substitutions: request.substitutions.clone(),
            })
            .map_err(|error| anyhow::anyhow!("HSEL-0003: {error}"))?;
            if decision.status == DecisionStatus::Incompatible
                || decision.status == DecisionStatus::CompatibleWithDegradation
            {
                let detail = decision
                    .diagnostics
                    .iter()
                    .map(|d| format!("{} {}", d.code, d.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                refusals.push(format!("{} refused: {detail}", declaration.harness_id));
            } else if decision.status == DecisionStatus::Compatible {
                compatible.push((harness, decision));
            } else {
                bail!(
                    "HSEL-0003: registered harness produced unexpected decision status {:?}",
                    decision.status
                );
            }
        }

        let (harness, decision) = match compatible.len() {
            1 => compatible.remove(0),
            0 => bail!(
                "HSEL-0003: no registered harness satisfies the work requirements ({})",
                refusals.join(" | ")
            ),
            _ => {
                let ids: Vec<&str> = compatible
                    .iter()
                    .map(|(h, _)| h.declaration().harness_id.as_str())
                    .collect();
                bail!(
                    "HSEL-0004: harness selection is ambiguous between {}; pin one explicitly",
                    ids.join(", ")
                )
            }
        };

        let declaration = harness.declaration();
        let authority_declared_digest = canonical_json_digest(request.authority_declared)?;
        let authority_effective_digest = canonical_json_digest(request.authority_effective)?;
        let decision_digest = canonical_json_digest(&decision)?;
        HarnessSelectionV1 {
            schema_version: HARNESS_SELECTION_SCHEMA_VERSION.to_string(),
            selection_id: request.selection_id.clone(),
            harness_id: declaration.harness_id.clone(),
            identity: declaration.identity.clone(),
            family: declaration.family,
            declaration_digest: declaration.try_compute_digest()?,
            capability_set_digest: declaration
                .capabilities
                .canonical_digest()
                .map_err(|e| anyhow::anyhow!("HSEL-0007: {e}"))?,
            requirements_digest: request
                .requirements
                .canonical_digest()
                .map_err(|e| anyhow::anyhow!("HSEL-0007: {e}"))?,
            decision_id: decision.id,
            decision_status: decision.status,
            evaluated_at: request.evaluated_at.clone(),
            authority_declared_digest,
            authority_effective_digest,
            decision_digest,
            substitutions: request.substitutions.clone(),
            workspace: workspace_manifest.map(|manifest| {
                // Build workspace evidence from the validated manifest.
                let adapter_family_str = match manifest.adapter {
                    AdapterKind::GitWorktree => "git-worktree",
                    AdapterKind::ExistingReadOnly => "existing-read-only",
                };
                let mode_boundary = match manifest.mode {
                    WorkspaceMode::ReadOnly => "read",
                    WorkspaceMode::Writable => "write",
                };
                let manifest_digest = manifest.compute_digest();
                WorkspaceEvidenceV1 {
                    workspace_id: manifest.workspace_id.clone(),
                    adapter: adapter_family_str.to_string(),
                    adapter_revision: manifest.adapter_revision.clone(),
                    mode: mode_boundary.to_string(),
                    manifest_digest,
                }
            }),
            content_digest: None,
        }
        .try_sealed()
    }
}

/// One adapter as published by the generated catalogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessCatalogEntry {
    pub harness_id: String,
    pub family: HarnessFamily,
    pub name: String,
    pub revision: String,
    pub declaration_digest: String,
    pub declared_capabilities: Vec<String>,
    pub claimed_capabilities: Vec<String>,
    // --- Capability surface (from RuntimeCapabilitySet) ---
    pub effect_classes: Vec<String>,
    pub spec_versions: Vec<String>,
    pub adapter_families: Vec<String>,
    // --- Workspace compatibility ---
    pub workspace: Option<crate::workflow::soma::profile::Boundaries>,
    // --- Structured I/O limits ---
    pub max_structured_input: Option<serde_json::Number>,
    pub max_structured_output: Option<serde_json::Number>,
    // --- Budget ceilings ---
    pub budget_ceilings: Option<crate::workflow::soma::contracts::Budgets>,
    pub locality: Option<crate::workflow::soma::capability::Locality>,
    pub checkpoint_export: Option<bool>,
    pub checkpoint_import: Option<bool>,
    pub resume_support: Option<bool>,
    pub event_streaming: Option<bool>,
    pub read_projection: Option<bool>,
    pub privacy_classes: Vec<String>,
    pub evidence_attachment: Option<bool>,
    pub validation: Option<bool>,
    pub approval_escalation: Option<bool>,
    // --- Limitations (derived, never granted authority) ---
    /// Capabilities declared but not claimed — the adapter's limitations.
    pub unclaimed_capabilities: Vec<String>,
    /// Whether the adapter claims more than it declares (must always be false
    /// for a valid declaration).
    pub over_claimed: bool,
}

/// Generated, digest-sealed capability catalogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessCatalog {
    pub schema_version: String,
    pub conformance_version: String,
    pub entries: Vec<HarnessCatalogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl HarnessCatalog {
    pub fn sealed(mut self) -> Self {
        self.content_digest = None;
        let digest = self.compute_digest();
        self.content_digest = Some(digest);
        self
    }

    pub fn compute_digest(&self) -> String {
        let mut value = serde_json::to_value(self).expect("catalogue serializes");
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .expect("canonical digest of a Lite-constructed catalogue cannot fail")
    }

    /// Fail-closed validation of a generated catalog.
    ///
    /// Returns a list of findings (empty = valid). Used by the Slice C drift
    /// guard to detect malformed or inconsistent catalogs.
    pub fn validate(&self) -> Vec<String> {
        let mut findings = Vec::new();
        if self.schema_version != HARNESS_SCHEMA_VERSION {
            findings.push(format!(
                "unsupported schema version {} (expected {HARNESS_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if self.conformance_version != HARNESS_CONFORMANCE_VERSION {
            findings.push(format!(
                "unsupported conformance version {} (expected {HARNESS_CONFORMANCE_VERSION})",
                self.conformance_version
            ));
        }
        // Check for duplicate harness_ids
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            if !seen.insert(&entry.harness_id) {
                findings.push(format!("duplicate harnessId {:?}", entry.harness_id));
            }
            if entry.declaration_digest.len() != 64 {
                findings.push(format!(
                    "invalid declaration_digest for {:?}",
                    entry.harness_id
                ));
            }
            // Over-claimed check
            if entry.over_claimed {
                findings.push(format!(
                    "harness {:?} claims capabilities it does not declare",
                    entry.harness_id
                ));
            }
        }
        // Verify content digest if present
        if let Some(digest) = &self.content_digest {
            let computed = self.compute_digest();
            if *digest != computed {
                findings.push("contentDigest does not verify".to_string());
            }
        } else {
            findings.push("missing contentDigest".to_string());
        }
        // Entries must be sorted by harness_id for deterministic output
        let sorted: Vec<&str> = self.entries.iter().map(|e| e.harness_id.as_str()).collect();
        let mut sorted_copy = sorted.clone();
        sorted_copy.sort();
        if sorted != sorted_copy {
            findings.push("catalog entries are not sorted by harnessId".to_string());
        }
        findings
    }

    /// Verify the catalog against a set of trusted declarations.
    pub fn verify_against_declarations(
        &self,
        declarations: &[HarnessDeclarationV1],
    ) -> Vec<String> {
        let mut findings = Vec::new();

        if let Some(digest) = &self.content_digest {
            let computed = self.compute_digest();
            if *digest != computed {
                findings.push("catalog contentDigest does not verify".to_string());
            }
        } else {
            findings.push("catalog missing contentDigest".to_string());
        }
        let catalog_ids: BTreeSet<&str> =
            self.entries.iter().map(|e| e.harness_id.as_str()).collect();
        let decl_ids: BTreeSet<&str> = declarations.iter().map(|d| d.harness_id.as_str()).collect();

        // Missing entries
        for id in &decl_ids {
            if !catalog_ids.contains(id) {
                findings.push(format!(
                    "missing catalog entry for trusted declaration {id:?}"
                ));
            }
        }
        // Extra entries
        for id in &catalog_ids {
            if !decl_ids.contains(id) {
                findings.push(format!(
                    "catalog contains entry for unknown declaration {id:?}"
                ));
            }
        }
        // Verify each entry matches its declaration
        for entry in &self.entries {
            if let Some(decl) = declarations
                .iter()
                .find(|d| d.harness_id == entry.harness_id)
            {
                if entry.declaration_digest != decl.compute_digest() {
                    findings.push(format!(
                        "declarationDigest mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.name != decl.identity.name {
                    findings.push(format!("name mismatch for {:?}", entry.harness_id));
                }
                if entry.revision != decl.identity.revision {
                    findings.push(format!("revision mismatch for {:?}", entry.harness_id));
                }
                if entry.family != decl.family {
                    findings.push(format!("family mismatch for {:?}", entry.harness_id));
                }
                if entry.declared_capabilities != decl.declared_capabilities {
                    findings.push(format!(
                        "declaredCapabilities mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.claimed_capabilities != decl.claimed_capabilities {
                    findings.push(format!(
                        "claimedCapabilities mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.effect_classes != decl.capabilities.effect_classes {
                    findings.push(format!("effectClasses mismatch for {:?}", entry.harness_id));
                }
                let decl_adapter_families: Vec<String> = decl
                    .capabilities
                    .adapter_families
                    .iter()
                    .map(|f| format!("{f:?}"))
                    .collect();
                if entry.adapter_families != decl_adapter_families {
                    findings.push(format!(
                        "adapterFamilies mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                let decl_adapter_families: Vec<String> = decl
                    .capabilities
                    .adapter_families
                    .iter()
                    .map(|f| format!("{f:?}"))
                    .collect();
                if entry.adapter_families != decl_adapter_families {
                    findings.push(format!(
                        "adapterFamilies mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.spec_versions != decl.capabilities.spec_versions {
                    findings.push(format!("specVersions mismatch for {:?}", entry.harness_id));
                }
                if entry.privacy_classes != decl.capabilities.privacy_classes {
                    findings.push(format!(
                        "privacyClasses mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.workspace != decl.capabilities.workspace {
                    findings.push(format!("workspace mismatch for {:?}", entry.harness_id));
                }
                if entry.max_structured_input != decl.capabilities.max_structured_input {
                    findings.push(format!(
                        "maxStructuredInput mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.max_structured_output != decl.capabilities.max_structured_output {
                    findings.push(format!(
                        "maxStructuredOutput mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.budget_ceilings != decl.capabilities.budget_ceilings {
                    findings.push(format!(
                        "budgetCeilings mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.locality != decl.capabilities.locality {
                    findings.push(format!("locality mismatch for {:?}", entry.harness_id));
                }
                if entry.checkpoint_export != decl.capabilities.checkpoint_export {
                    findings.push(format!(
                        "checkpointExport mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.checkpoint_import != decl.capabilities.checkpoint_import {
                    findings.push(format!(
                        "checkpointImport mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.resume_support != decl.capabilities.resume_support {
                    findings.push(format!("resumeSupport mismatch for {:?}", entry.harness_id));
                }
                if entry.event_streaming != decl.capabilities.event_streaming {
                    findings.push(format!(
                        "eventStreaming mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.read_projection != decl.capabilities.read_projection {
                    findings.push(format!(
                        "readProjection mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.evidence_attachment != decl.capabilities.evidence_attachment {
                    findings.push(format!(
                        "evidenceAttachment mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                if entry.validation != decl.capabilities.validation {
                    findings.push(format!("validation mismatch for {:?}", entry.harness_id));
                }
                if entry.approval_escalation != decl.capabilities.approval_escalation {
                    findings.push(format!(
                        "approvalEscalation mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                let expected_unclaimed: Vec<String> = decl
                    .declared_capabilities
                    .iter()
                    .filter(|c| !decl.claimed_capabilities.contains(c))
                    .cloned()
                    .collect();
                if entry.unclaimed_capabilities != expected_unclaimed {
                    findings.push(format!(
                        "unclaimedCapabilities mismatch for {:?}",
                        entry.harness_id
                    ));
                }
                let expected_over_claimed = decl
                    .claimed_capabilities
                    .iter()
                    .any(|c| !decl.declared_capabilities.contains(c));
                if entry.over_claimed != expected_over_claimed {
                    findings.push(format!("overClaimed mismatch for {:?}", entry.harness_id));
                }
            }
        }
        findings
    }

    /// Deterministic JSON (entries already ordered by `harnessId`).
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

// ---------------------------------------------------------------------------
// Conformance kit
// ---------------------------------------------------------------------------

/// One conformance finding. An empty report means conformance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessFinding {
    pub code: &'static str,
    pub message: String,
}

/// Result of driving one adapter through the conformance kit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessConformanceReport {
    pub harness_id: String,
    pub findings: Vec<HarnessFinding>,
}

impl HarnessConformanceReport {
    pub fn passed(&self) -> bool {
        self.findings.is_empty()
    }
}

fn finding(code: &'static str, message: impl Into<String>) -> HarnessFinding {
    HarnessFinding {
        code,
        message: message.into(),
    }
}

/// Drive `harness` through the reusable conformance checks.
///
/// The kit EXECUTES one step through the caller's governed dispatch, so it
/// proves the adapter goes through the gates rather than only inspecting its
/// declaration. It also proves that the declaration and the behaviour agree:
/// an adapter that declares checkpoint export must be able to export, and one
/// that does not declare it must refuse.
pub async fn conformance_report(
    harness: &dyn ExecutionHarness,
    step: HarnessStep,
    dispatch: &mut HarnessDispatch<'_>,
    portable_state_digest: &str,
) -> HarnessConformanceReport {
    let declaration = harness.declaration();
    let mut findings = Vec::new();

    if declaration.schema_version != HARNESS_SCHEMA_VERSION {
        findings.push(finding(
            "HCONF-0001",
            format!(
                "unsupported declaration schema {}",
                declaration.schema_version
            ),
        ));
    }
    if declaration.content_digest.as_deref() != declaration.try_compute_digest().ok().as_deref() {
        findings.push(finding(
            "HCONF-0002",
            "declaration is not sealed or its contentDigest does not verify",
        ));
    }
    if declaration.identity != declaration.capabilities.identity {
        findings.push(finding(
            "HCONF-0003",
            "declaration identity diverges from the capability declaration identity",
        ));
    }
    let declared: BTreeSet<&str> = declaration
        .declared_capabilities
        .iter()
        .map(String::as_str)
        .collect();
    if declared.len() != declaration.declared_capabilities.len() {
        findings.push(finding(
            "HCONF-0004",
            "declared capabilities contain duplicates",
        ));
    }
    if declaration
        .claimed_capabilities
        .iter()
        .any(|c| !declared.contains(c.as_str()))
    {
        findings.push(finding(
            "HCONF-0005",
            "a claimed capability is outside the declared set",
        ));
    }
    if let Some(first) = declaration.capabilities.audit().first() {
        findings.push(finding(
            "HCONF-0006",
            format!(
                "capability declaration is not audit-clean: {} {}",
                first.code, first.message
            ),
        ));
    }
    if declaration.capabilities.resume_support == Some(true)
        && declaration.capabilities.checkpoint_import != Some(true)
    {
        findings.push(finding(
            "HCONF-0007",
            "resume is declared without checkpointImport",
        ));
    }

    // The refusal point itself: a capability outside the declaration must be
    // refused BEFORE the runner is touched.
    if dispatch
        .ensure_declared("__hconf_undeclared_probe__")
        .is_ok()
    {
        findings.push(finding(
            "HCONF-0008",
            "the dispatch accepted a capability outside the declaration",
        ));
    }

    let calls_before = dispatch.calls();
    let outcome = match harness.run(step, dispatch).await {
        Ok(outcome) => Some(outcome),
        Err(error) => {
            findings.push(finding("HCONF-0009", format!("run failed: {error}")));
            None
        }
    };

    if let Some(outcome) = &outcome {
        if outcome.harness_id != declaration.harness_id {
            findings.push(finding(
                "HCONF-0010",
                "outcome harnessId does not match the declaration",
            ));
        }
        if !is_hex64(&outcome.evidence_entry_digest) {
            findings.push(finding(
                "HCONF-0011",
                "outcome does not carry a journaled evidence entry digest",
            ));
        }
        if let Err(error) = outcome.verify() {
            findings.push(finding("HCONF-0012", error.to_string()));
        }
        let mut correlation: Option<&str> = None;
        for (index, event) in outcome.events.iter().enumerate() {
            if event.sequence != index as u64 {
                findings.push(finding(
                    "HCONF-0013",
                    format!(
                        "event {index} has non-contiguous sequence {}",
                        event.sequence
                    ),
                ));
                break;
            }
            if event.correlation_id.trim().is_empty() || event.kind.trim().is_empty() {
                findings.push(finding(
                    "HCONF-0014",
                    format!("event {index} has an empty correlationId or kind"),
                ));
                break;
            }
            match correlation {
                None => correlation = Some(event.correlation_id.as_str()),
                Some(first) if first != event.correlation_id => {
                    findings.push(finding(
                        "HCONF-0015",
                        format!("event {index} changed correlationId within one run"),
                    ));
                    break;
                }
                Some(_) => {}
            }
        }
        if dispatch.calls() == calls_before {
            findings.push(finding(
                "HCONF-0016",
                "run produced an outcome without a governed capability call",
            ));
        }
    }

    let declared_export = declaration.capabilities.checkpoint_export == Some(true);
    let checkpoint = match outcome.as_ref() {
        Some(outcome) => match harness.checkpoint(outcome, portable_state_digest).await {
            Ok(checkpoint) => {
                if declared_export {
                    if let Err(error) = checkpoint.verify() {
                        findings.push(finding("HCONF-0017", error.to_string()));
                    }
                    if checkpoint.harness_id != declaration.harness_id
                        || checkpoint.harness_revision != declaration.identity.revision
                        || checkpoint.portable_state_digest != portable_state_digest
                    {
                        findings.push(finding(
                            "HCONF-0018",
                            "checkpoint does not bind this harness, revision and portable state",
                        ));
                    }
                    Some(checkpoint)
                } else {
                    findings.push(finding(
                        "HCONF-0020",
                        "checkpoint succeeded although checkpointExport is not declared",
                    ));
                    None
                }
            }
            Err(error) => {
                if declared_export {
                    findings.push(finding(
                        "HCONF-0019",
                        format!("checkpointExport is declared but checkpoint failed: {error}"),
                    ));
                }
                None
            }
        },
        None => None,
    };

    let declared_resume = declaration.capabilities.resume_support == Some(true);
    let probe_checkpoint = checkpoint.unwrap_or_else(|| {
        HarnessCheckpointV1 {
            schema_version: HARNESS_SCHEMA_VERSION.into(),
            harness_id: declaration.harness_id.clone(),
            harness_revision: declaration.identity.revision.clone(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: None,
            content_digest: None,
        }
        .sealed()
    });
    let resumed = harness
        .resume(&probe_checkpoint, portable_state_digest)
        .await;
    match (declared_resume, resumed) {
        (true, Ok(continuation)) => {
            if continuation.harness_id != declaration.harness_id
                || continuation.harness_revision != declaration.identity.revision
                || continuation.portable_state_digest != portable_state_digest
            {
                findings.push(finding(
                    "HCONF-0021",
                    "resume returned a continuation that does not bind this harness and state",
                ));
            }
            let mut foreign = probe_checkpoint.clone();
            foreign.harness_revision = "revision-does-not-match".into();
            foreign.content_digest = None;
            let foreign = foreign.sealed();
            if harness
                .resume(&foreign, portable_state_digest)
                .await
                .is_ok()
            {
                findings.push(finding(
                    "HCONF-0022",
                    "resume accepted a checkpoint from a different revision",
                ));
            }
        }
        (true, Err(error)) => findings.push(finding(
            "HCONF-0023",
            format!("resume is declared but refused a matching checkpoint: {error}"),
        )),
        (false, Ok(_)) => findings.push(finding(
            "HCONF-0024",
            "resume succeeded although resume support is not declared",
        )),
        (false, Err(_)) => {}
    }

    HarnessConformanceReport {
        harness_id: declaration.harness_id.clone(),
        findings,
    }
}
