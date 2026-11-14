//! Governed route dispatch binding (issue #133, Slice B).
//!
//! Slice A resolved an eligible route before any effect and emitted a sealed
//! [`EffectiveExecutionSnapshotV1`]. Slice B connects that preflight to one
//! actual governed dispatch seam: it re-validates the resolved route against
//! the *dispatch-time* reality and then executes through the existing
//! [`HarnessDispatch`] boundary — the only adapter-effect path.
//!
//! Design rules (fail closed, no new runtime, no new authorization):
//!
//! * **The snapshot authorizes preflight, not eternity.** Trust freshness is
//!   re-checked at the injected dispatch instant, not the resolution instant.
//! * **Registry and manifest may not have moved.** The selected harness's
//!   declaration digest, published identity/revision, capability-set digest
//!   and family must match the sealed snapshot exactly.
//! * **Candidates may not have been swapped.** The candidate that the
//!   resolution selected is re-identified by value against the operator-owned
//!   set carried in the original request.
//! * **No fallback, ever.** A pinned or uniformly-selected route either
//!   executes exactly as resolved or the dispatch refuses; nothing here
//!   substitutes a different provider, model, harness or workspace.
//! * **Evidence is bound to the attempt.** The sealed [`RoutedAttemptV1`]
//!   carries the snapshot/decision/evidence/manifest digests and the exact
//!   final journal entry produced by the governed run, so a reader can bind
//!   the durable attempt to the exact route that authorized it.
//! * **Typed failures are preserved; no replay happens.** A failing effect
//!   returns [`RoutedDispatchFailure::ExecutionFailed`] with the journal
//!   state up to the failure; it is never re-executed from inside this
//!   seam, and the partial evidence is carried forward.
//!
//! Scope: bind, preflight-revalidate, dispatch, bind evidence. No Slice C
//! accounting, no provider integrations, no new dispatch machinery for
//! unrouted callers.

use serde::{Deserialize, Serialize};

use crate::workflow::execution_harness::{
    HarnessDispatch, HarnessOutcome, HarnessRegistry, HarnessStep, workspace_adapter_family,
};
use crate::workflow::node_runner::{JournalEntryV1, NodeRunner};
use crate::workflow::policy::LocalRestrictions;
use crate::workflow::route_resolution::{
    EffectiveExecutionSnapshotV1, RouteCandidateV1, RouteResolution, RouteResolutionRequest,
    is_hex64, validate_trust,
};
use crate::workflow::soma;
use crate::workflow::soma::capability::is_rfc3339_z;

pub const ROUTED_ATTEMPT_SCHEMA_VERSION: &str = "1.0.0";

/// Sealed, digest-bound record of one resolved route's attempt.
///
/// It binds the durable journal evidence of the attempt to the exact route
/// that authorized it (snapshot/decision/evidence digests, manifest digest,
/// harness revision, provider/model identity). It is produced *after* the
/// governed effect settles; it is never a pre-effect authorization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutedAttemptV1 {
    pub schema_version: String,
    pub snapshot_digest: String,
    pub route_evidence_digest: String,
    pub decision_digest: String,
    pub manifest_digest: String,
    /// The idempotency identity of the governed attempt (the step id).
    pub idempotency_key: String,
    pub harness_id: String,
    pub provider_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_manifest_digest: Option<String>,
    /// Governor-dispatch call count for this attempt.
    pub governed_calls: u64,
    /// Outcome class (`completed`, or refused before any effect on failure
    /// paths that still produced evidence).
    pub outcome_kind: String,
    /// The terminal journal entry of the attempt, bound by its digest. On
    /// successful completion it always exists and verifies.
    pub journal_entry: JournalEntryV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl RoutedAttemptV1 {
    fn try_sealed(mut self) -> anyhow::Result<Self> {
        self.content_digest = None;
        self.content_digest = Some(self.try_compute_digest()?);
        Ok(self)
    }

    fn try_compute_digest(&self) -> anyhow::Result<String> {
        let mut value = serde_json::to_value(self).context("routed attempt serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    /// Fail-closed re-verification of the attempt. The journal entry digest
    /// is rebuilt from a zeroed entry and must match, and the entry's
    /// identity key must equal the attempt's idempotency key.
    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::{bail, ensure};
        ensure!(
            self.schema_version == ROUTED_ATTEMPT_SCHEMA_VERSION,
            "unsupported routed attempt schema version {} (supported {ROUTED_ATTEMPT_SCHEMA_VERSION})",
            self.schema_version
        );
        for (label, value) in [
            ("snapshotDigest", &self.snapshot_digest),
            ("routeEvidenceDigest", &self.route_evidence_digest),
            ("decisionDigest", &self.decision_digest),
            ("manifestDigest", &self.manifest_digest),
        ] {
            ensure!(is_hex64(value), "{label} is not a lower hex sha256 digest");
        }
        ensure!(
            is_hex64(&self.journal_entry.entry_digest),
            "journalEntry.entryDigest is not a lower hex sha256 digest"
        );
        verify_journal_entry(&self.journal_entry)?;
        ensure!(
            self.journal_entry.identity_key == self.idempotency_key,
            "journal identity key {:?} does not match attempt idempotency key {:?}",
            self.journal_entry.identity_key,
            self.idempotency_key
        );
        let expected = self.try_compute_digest()?;
        match &self.content_digest {
            Some(actual) if *actual == expected => Ok(()),
            _ => bail!("routed attempt contentDigest does not verify"),
        }
    }
}

/// The outcome of a routed dispatch: the governed harness outcome plus the
/// sealed, evidence-bound attempt record.
#[derive(Debug)]
pub struct RoutedDispatchOutcome {
    pub outcome: HarnessOutcome,
    pub attempt: RoutedAttemptV1,
}

/// The failure detail of a failed governed effect: typed error, call count
/// and whatever journal state existed when it failed.
#[derive(Debug)]
pub struct ExecutionFailure {
    pub source: anyhow::Error,
    pub calls: u64,
    pub journal_entry: Option<JournalEntryV1>,
}

/// Why a routed dispatch did or did not happen.
#[derive(Debug)]
pub enum RoutedDispatchFailure {
    /// Refused before any effect. The deterministic reason; no adapter ran,
    /// no capability handler ran.
    Refused(anyhow::Error),
    /// The governed effect itself failed. The typed error is preserved, the
    /// call count and whatever journal state exists are attached, and the
    /// seam does not retry.
    ExecutionFailed(Box<ExecutionFailure>),
}

impl RoutedDispatchFailure {
    fn refused(reason: impl Into<String>) -> Self {
        RoutedDispatchFailure::Refused(anyhow::anyhow!("{}", reason.into()))
    }

    fn failed(source: anyhow::Error, calls: u64, journal_entry: Option<JournalEntryV1>) -> Self {
        RoutedDispatchFailure::ExecutionFailed(Box::new(ExecutionFailure {
            source,
            calls,
            journal_entry,
        }))
    }
}

impl std::fmt::Display for RoutedDispatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoutedDispatchFailure::Refused(error) => write!(f, "routed dispatch refused: {error}"),
            RoutedDispatchFailure::ExecutionFailed(failure) => {
                write!(f, "governed effect failed: {}", failure.source)
            }
        }
    }
}

impl std::error::Error for RoutedDispatchFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RoutedDispatchFailure::Refused(error) => Some(error.as_ref()),
            RoutedDispatchFailure::ExecutionFailed(failure) => Some(failure.source.as_ref()),
        }
    }
}

/// Inputs for one routed dispatch. Everything here is caller-held; the seam
/// re-derives what it needs to trust.
pub struct RoutedDispatchRequest<'a> {
    /// The sealed resolution from [`resolve_route`](crate::workflow::route_resolution::resolve_route).
    pub resolution: &'a RouteResolution,
    /// The original resolution inputs (route set, candidates, requirements,
    /// authority, pin, workspace). They are re-checked, not re-run.
    pub original_request: &'a RouteResolutionRequest<'a>,
    /// The governed capability registry to execute through.
    pub registry: &'a HarnessRegistry,
    /// The permit-bound governed runner.
    pub runner: &'a mut NodeRunner,
    /// The node manifest under the reviewed permit.
    pub manifest: &'a crate::workflow::node_contracts::NodeManifestV1,
    /// The local policy restrictions for the runner's gate-3 decision.
    pub local_restrictions: &'a LocalRestrictions,
    /// Known secrets for gate-7 redaction.
    pub known_secrets: Vec<String>,
    /// Trusted dispatch instant (RFC 3339 UTC-Z), injected. Not a wall clock.
    pub dispatched_at: &'a str,
    /// The governed step to execute.
    pub step: HarnessStep,
}

fn verify_journal_entry(entry: &JournalEntryV1) -> anyhow::Result<()> {
    let mut clone = entry.clone();
    clone.entry_digest = String::new();
    let value = serde_json::to_value(&clone).context("journal entry serializes")?;
    let expected = soma::try_canonical_digest(&value)
        .map_err(|error| anyhow::anyhow!("journal entry canonical digest failed: {error}"))?;
    if expected != entry.entry_digest {
        anyhow::bail!("journal entry digest does not verify");
    }
    Ok(())
}

use anyhow::{Context, anyhow};

/// Resolve → revalidate → dispatch → bind.
///
/// Returns [`RoutedDispatchOutcome`] on success; [`RoutedDispatchFailure`]
/// otherwise. No adapter effect happens before every check passes.
pub async fn dispatch_resolved_route(
    req: RoutedDispatchRequest<'_>,
) -> Result<RoutedDispatchOutcome, RoutedDispatchFailure> {
    // ---- Trusted dispatch instant shape and monotonicity.
    if !is_rfc3339_z(req.dispatched_at) {
        return Err(RoutedDispatchFailure::refused(format!(
            "dispatchedAt {:?} is not a genuine RFC 3339 UTC-Z timestamp",
            req.dispatched_at
        )));
    }
    if req.dispatched_at < req.resolution.snapshot.evaluated_at.as_str() {
        return Err(RoutedDispatchFailure::refused(format!(
            "dispatchedAt {} precedes the snapshot's evaluatedAt {}; refusing a non-monotonic dispatch",
            req.dispatched_at, req.resolution.snapshot.evaluated_at
        )));
    }

    // ---- The snapshot and its evidence are the authorization. Both must
    // verify before anything else is considered.
    req.resolution.evidence.verify().map_err(|error| {
        RoutedDispatchFailure::refused(format!("route evidence does not verify: {error}"))
    })?;
    req.resolution.snapshot.verify().map_err(|error| {
        RoutedDispatchFailure::refused(format!("execution snapshot does not verify: {error}"))
    })?;

    let snapshot: &EffectiveExecutionSnapshotV1 = &req.resolution.snapshot;

    // ---- Registry identity: the harness must be present, sealed, and
    // exactly the declaration the snapshot attests to.
    let harness = req
        .registry
        .get(&snapshot.selected.harness_id)
        .map_err(|error| {
            RoutedDispatchFailure::refused(format!(
                "registry no longer serves the selected harness {:?}: {error}",
                snapshot.selected.harness_id
            ))
        })?;
    let declaration = harness.declaration();
    if declaration.harness_id != snapshot.selected.harness_id {
        return Err(RoutedDispatchFailure::refused(format!(
            "registry serves {:?} for selected id {:?}",
            declaration.harness_id, snapshot.selected.harness_id
        )));
    }
    if declaration.identity.revision != snapshot.selected.harness_revision {
        return Err(RoutedDispatchFailure::refused(format!(
            "selected harness {:?} published revision {:?} does not match snapshot revision {:?}; registry may have been mutated",
            declaration.harness_id,
            declaration.identity.revision,
            snapshot.selected.harness_revision
        )));
    }
    if declaration.family.as_str() != snapshot.selected.family {
        return Err(RoutedDispatchFailure::refused(format!(
            "selected harness family {:?} does not match snapshot family {:?}",
            declaration.family.as_str(),
            snapshot.selected.family
        )));
    }
    let declaration_digest = declaration.try_compute_digest().map_err(|error| {
        RoutedDispatchFailure::refused(format!("declaration digest failure: {error}"))
    })?;
    if declaration_digest != snapshot.declaration_digest {
        return Err(RoutedDispatchFailure::refused("selected harness declaration digest does not match the snapshot; registry may have been mutated".to_string()));
    }
    let capability_set_digest = declaration
        .capabilities
        .canonical_digest()
        .map_err(|error| {
            RoutedDispatchFailure::refused(format!("capability set digest failure: {error}"))
        })?;
    if capability_set_digest != snapshot.capability_set_digest {
        return Err(RoutedDispatchFailure::refused("selected harness capability set digest does not match the snapshot; registry may have been mutated".to_string()));
    }

    // ---- Candidate rebound: the operator-owned set must still contain the
    // exact selected candidate, by value.
    let candidate: &RouteCandidateV1 = req
        .original_request
        .candidates
        .iter()
        .find(|candidate| candidate.candidate_id == snapshot.selected.candidate_id)
        .ok_or_else(|| {
            RoutedDispatchFailure::refused(format!(
                "selected candidate {:?} is no longer present in the operator-owned route set",
                snapshot.selected.candidate_id
            ))
        })?;
    if candidate.harness_id != snapshot.selected.harness_id {
        return Err(RoutedDispatchFailure::refused(format!(
            "candidate {:?} now names harness {:?}; snapshot bound {:?}",
            candidate.candidate_id, candidate.harness_id, snapshot.selected.harness_id
        )));
    }
    if candidate.harness_revision != snapshot.selected.harness_revision {
        return Err(RoutedDispatchFailure::refused(format!(
            "candidate {:?} now names revision {:?}; snapshot bound revision {:?}",
            candidate.candidate_id, candidate.harness_revision, snapshot.selected.harness_revision
        )));
    }
    if candidate.provider_identity != snapshot.selected.provider_identity {
        return Err(RoutedDispatchFailure::refused(format!(
            "candidate {:?} now names provider {:?}; snapshot bound provider {:?}",
            candidate.candidate_id,
            candidate.provider_identity,
            snapshot.selected.provider_identity
        )));
    }
    if candidate.model != snapshot.selected.model {
        return Err(RoutedDispatchFailure::refused(format!(
            "candidate {:?} now names model {:?}; snapshot bound model {:?}",
            candidate.candidate_id, candidate.model, snapshot.selected.model
        )));
    }
    if candidate.priority != snapshot.selected.priority {
        return Err(RoutedDispatchFailure::refused(format!(
            "candidate {:?} now has priority {}; snapshot bound priority {}",
            candidate.candidate_id, candidate.priority, snapshot.selected.priority
        )));
    }
    if candidate.experimental != snapshot.selected.experimental {
        return Err(RoutedDispatchFailure::refused(format!(
            "candidate {:?} now has experimental flag {}; snapshot bound {}",
            candidate.candidate_id, candidate.experimental, snapshot.selected.experimental
        )));
    }
    match (&candidate.workspace, &snapshot.workspace) {
        (None, None) => {}
        (Some(binding), Some(evidence)) => {
            if workspace_adapter_family(binding.adapter) != evidence.adapter
                || binding.revision != evidence.adapter_revision
            {
                return Err(RoutedDispatchFailure::refused(format!(
                    "candidate {:?} workspace binding {:?}/{:?} does not match snapshot binding {:?}/{:?}",
                    candidate.candidate_id,
                    binding.adapter,
                    binding.revision,
                    evidence.adapter,
                    evidence.adapter_revision
                )));
            }
        }
        _ => {
            return Err(RoutedDispatchFailure::refused(format!(
                "candidate {:?} workspace binding and snapshot workspace disagree",
                candidate.candidate_id
            )));
        }
    }

    // ---- Trust freshness at the dispatch instant (the resolution's own
    // evaluatedAt is not a standing permission).
    validate_trust(&candidate.trust, req.dispatched_at)
        .map_err(|reason| RoutedDispatchFailure::Refused(anyhow!("{reason}")))?;

    // ---- Workspace re-verification: the route binds an exact workspace
    // manifest; the dispatch must see exactly that manifest.
    match (&req.original_request.workspace, &snapshot.workspace) {
        (None, None) => {}
        (Some(manifest), Some(evidence)) => {
            let digest = manifest.compute_digest();
            if digest != evidence.manifest_digest {
                return Err(RoutedDispatchFailure::refused(format!(
                    "workspace manifest changed since resolution (digest {digest} vs bound {})",
                    evidence.manifest_digest
                )));
            }
        }
        (None, Some(_)) => {
            return Err(RoutedDispatchFailure::refused(
                "route binds a workspace, but none was provided at dispatch".to_string(),
            ));
        }
        (Some(_), None) => {}
    }

    // ---- Step eligibility before the adapter runs.
    if !declaration
        .declared_capabilities
        .contains(&req.step.capability)
    {
        return Err(RoutedDispatchFailure::refused(format!(
            "step capability {:?} is not declared by selected harness {:?}; refusing before effects",
            req.step.capability, declaration.harness_id
        )));
    }

    // ---- Execute through the existing governed boundary.
    let mut dispatch = HarnessDispatch::new(
        harness,
        req.manifest,
        req.local_restrictions,
        req.known_secrets,
        req.runner,
    );
    let step_id = req.step.step_id.clone();
    let outcome = harness.run(req.step, &mut dispatch).await;
    let calls = dispatch.calls() as u64;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(source) => {
            let journal_entry = req.runner.journal().last().cloned();
            return Err(RoutedDispatchFailure::failed(source, calls, journal_entry));
        }
    };
    drop(dispatch);

    // ---- Bind the attempt to the durable journal evidence.
    let expected_digest = outcome.evidence_entry_digest.clone();
    let journal_entry = req
        .runner
        .journal()
        .iter()
        .rev()
        .find(|entry| entry.entry_digest == expected_digest)
        .cloned()
        .ok_or_else(|| RoutedDispatchFailure::failed(
            anyhow!(
                "governed dispatch returned evidence digest {expected_digest} but no journal entry matches it"
            ),
            calls,
            None,
        ))?;
    let manifest_value = serde_json::to_value(req.manifest).map_err(|error| {
        RoutedDispatchFailure::failed(
            anyhow::anyhow!("node manifest does not serialize: {error}"),
            calls,
            Some(journal_entry.clone()),
        )
    })?;
    let manifest_digest = soma::try_canonical_digest(&manifest_value).map_err(|error| {
        RoutedDispatchFailure::failed(
            anyhow::anyhow!("node manifest canonical digest failed: {error}"),
            calls,
            Some(journal_entry.clone()),
        )
    })?;
    let snapshot_digest = snapshot.content_digest.clone().ok_or_else(|| {
        RoutedDispatchFailure::failed(
            anyhow!("sealed snapshot is missing its contentDigest"),
            calls,
            Some(journal_entry.clone()),
        )
    })?;
    let attempt =
        RoutedAttemptV1 {
            schema_version: ROUTED_ATTEMPT_SCHEMA_VERSION.to_string(),
            snapshot_digest,
            route_evidence_digest: req.resolution.evidence.content_digest.clone().ok_or_else(
                || {
                    RoutedDispatchFailure::failed(
                        anyhow!("sealed route evidence is missing its contentDigest"),
                        calls,
                        Some(journal_entry.clone()),
                    )
                },
            )?,
            decision_digest: snapshot.decision_digest.clone(),
            manifest_digest,
            idempotency_key: step_id,
            harness_id: snapshot.selected.harness_id.clone(),
            provider_identity: snapshot.selected.provider_identity.clone(),
            model: snapshot.selected.model.clone(),
            workspace_manifest_digest: snapshot
                .workspace
                .as_ref()
                .map(|w| w.manifest_digest.clone()),
            governed_calls: calls,
            outcome_kind: "completed".to_string(),
            journal_entry,
            content_digest: None,
        }
        .try_sealed()
        .map_err(|error| RoutedDispatchFailure::failed(error, calls, None))?;

    Ok(RoutedDispatchOutcome { outcome, attempt })
}
