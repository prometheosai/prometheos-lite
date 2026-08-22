use anyhow::{Context, Result, bail};
use std::path::Path;
use std::str::FromStr;

use crate::harness::patch_provider::{PatchProvider, PatchProviderContext};
use crate::workflow::{
    AuthorityLevel, GenerateScope, ProposalArtifact, ProviderRouteInfo, is_git_repo,
    sanitize_provider_route,
};

use super::cancellation::CancellationToken;
use super::cleanup::cleanup_worktree;
use super::evidence::{
    EvidenceBundle, ProposalRecord, ProviderProvenanceRecord, ValidationRecord, new_bundle,
    new_bundle_from_identity, prepare_evidence_dir, read_integrity_artifact,
    read_validation_artifact, write_bundle, write_integrity_artifact, write_validation_artifact,
};
use super::generation::{classify_generation_error, load_proposal_from_repo};
use super::heartbeat::HeartbeatSession;
use super::identity::{
    EvaluationState, ExecutionIdentity, GovernanceScopeSnapshot, TaskManifest,
    compute_identity_key, evidence_dir_for, hash_str, now_iso, persist_execution_identity,
};
use super::integrity::{git_rev_parse_head, verify_repo_integrity};
use super::preflight::{run_preflight, run_validation_preflight};
use super::recovery::{RecoveryDisposition, determine_recovery_disposition, recover_evaluation};
use super::registry::{
    FenceToken, LeaseConfig, OwnershipObservation, ProposalState, ReserveResult, TakeoverResult,
    fenced_finalize, is_entry_stale, is_entry_stale_at, lookup_entry, proposal_state_for_state,
    read_registry, release_reservation, transition_entry, try_reserve, try_take_ownership_cas,
};
use super::resource::{CLASSIFICATION_DISK, ResourceLimits};
use super::validation::{
    classify_dry_run_error, classify_validation_failure, failure_to_terminal_state,
    run_isolated_validation,
};
use crate::workflow::redaction::{Redactor, collect_known_secrets};
use crate::workflow::retention::{ProtectedReferences, reclaim_orphan_artifacts};

// ---------------------------------------------------------------------------
// Pipeline orchestration
// ---------------------------------------------------------------------------

/// Configuration for a single evaluation run.
pub struct EvaluationConfig {
    pub manifest: TaskManifest,
    pub provider: Box<dyn PatchProvider>,
    pub route_info: Option<ProviderRouteInfo>,
    /// Lease and heartbeat configuration. Defaults are conservative for
    /// production; tests should inject short durations.
    pub lease_config: LeaseConfig,
}

impl EvaluationConfig {
    pub fn new(manifest: TaskManifest, provider: Box<dyn PatchProvider>) -> Self {
        Self {
            manifest,
            provider,
            route_info: None,
            lease_config: LeaseConfig::default(),
        }
    }
}
/// Repo-relative evidence references used as journal `evidence_ref` values.
struct EvidenceRefs {
    /// The evidence directory, referenced by in-progress events.
    dir: String,
    /// The final `evidence.json`, referenced by terminal events.
    final_json: String,
}

impl EvidenceRefs {
    fn of(repo: &Path, evidence_dir: &Path) -> Self {
        Self {
            dir: super::durable::repo_relative_path(repo, evidence_dir),
            final_json: super::durable::repo_relative_path(
                repo,
                &evidence_dir.join("evidence.json"),
            ),
        }
    }
}

/// Perform a durable, journaled state transition under the current fence.
///
/// The journal event is the authoritative durable record; the identity
/// document and checkpoint are derived views (written fail-closed) afterwards.
fn durable_transition(
    repo: &Path,
    identity_path: &Path,
    run_id: &str,
    identity_key: &str,
    repository_revision: &str,
    fence: &FenceToken,
    to_state: EvaluationState,
    proposal_ref: Option<String>,
    failure_classification: Option<String>,
    evidence_ref: Option<String>,
) -> Result<u64> {
    super::journal::record_transition(
        repo,
        identity_path,
        run_id,
        identity_key,
        to_state,
        proposal_ref,
        failure_classification,
        repository_revision,
        evidence_ref,
        fence,
    )
}

/// Durably record a cooperative cancellation at the state where the run
/// stopped: a same-state journal event classified `"cancelled"`.
///
/// The reservation is intentionally KEPT (its heartbeat stops, so it goes
/// stale). This fences further writes: a fresh run can never silently restart
/// from scratch and violate append-only journal continuity, and the event
/// gives a later run the audit trail it needs to resume safely.
fn record_cancellation(
    repo: &Path,
    identity_path: &Path,
    run_id: &str,
    identity_key: &str,
    repository_revision: &str,
    fence: &FenceToken,
    state: EvaluationState,
    proposal_ref: Option<String>,
    evidence_ref: Option<String>,
) -> Result<u64> {
    durable_transition(
        repo,
        identity_path,
        run_id,
        identity_key,
        repository_revision,
        fence,
        state,
        proposal_ref,
        Some("cancelled".to_string()),
        evidence_ref,
    )
}
/// Run the full evaluation pipeline and return the evidence bundle.
///
/// This is the primary entry point for the `workflow evaluate` command.
/// Cancellation is not requested (the run is allowed to complete).
pub async fn evaluate(config: EvaluationConfig) -> Result<EvidenceBundle> {
    evaluate_with_cancellation(config, CancellationToken::new()).await
}

/// Run the full evaluation pipeline with cooperative cancellation support.
///
/// Cancellation is a distinct control-flow signal, NOT a failure. The pipeline
/// stops at the next safe point, durably records where it stopped (a
/// same-state journal event classified `"cancelled"`), stops its heartbeat,
/// fences further writes, and never deletes the proposal, evidence, or any
/// durable state. A later run resumes from the authoritative journal position.
pub async fn evaluate_with_cancellation(
    config: EvaluationConfig,
    token: CancellationToken,
) -> Result<EvidenceBundle> {
    config
        .lease_config
        .validate()
        .context("invalid lease configuration")?;
    let repo = config.manifest.repo.clone();

    if !is_git_repo(&repo) {
        bail!("not a git repository: {}", repo.display());
    }

    let commit_at_start = git_rev_parse_head(&repo)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let evidence_dir = config
        .manifest
        .evidence_dir
        .clone()
        .unwrap_or_else(|| evidence_dir_for(&repo, &run_id));
    prepare_evidence_dir(&evidence_dir)?;
    let refs = EvidenceRefs::of(&repo, &evidence_dir);

    let governance_scope = GovernanceScopeSnapshot {
        allowed_paths: config.manifest.allowed_paths.clone(),
        forbidden_paths: config.manifest.forbidden_paths.clone(),
        allow_dependency_changes: config.manifest.allow_dependency_changes,
        max_files_changed: config.manifest.max_files_changed,
        max_lines_changed: config.manifest.max_lines_changed,
        authority: config.manifest.authority.clone(),
        validation_command: config.manifest.validation_command.clone(),
    };

    let identity = ExecutionIdentity {
        run_id: run_id.clone(),
        task_id: config.manifest.task_id.clone(),
        repo: repo.display().to_string(),
        repo_pin: commit_at_start.clone(),
        model: config
            .route_info
            .as_ref()
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| "mock".to_string()),
        provider: config.provider.name().to_string(),
        governance_scope: governance_scope.clone(),
        created_at: now_iso(),
        state: EvaluationState::Created,
    };

    // Persist identity before any model call (exactly-once gate).
    let identity_path = persist_execution_identity(&evidence_dir, &identity)?;

    // Compute deterministic identity key for resume lookup.
    let identity_key = compute_identity_key(
        &config.manifest.task_id,
        &repo,
        &commit_at_start,
        config.provider.name(),
        identity.model.as_str(),
        &governance_scope,
        &config.manifest.validation_command,
    );

    // ---- Atomic reservation gate ----
    // Try to reserve the identity. If another process holds it, wait/reuse.
    let fence = match try_reserve(&repo, &identity_key, &run_id)
        .context("failed to attempt identity reservation")?
    {
        ReserveResult::Owned(token) => token,
        ReserveResult::AlreadyExists => {
            return match wait_and_reuse(
                &repo,
                &commit_at_start,
                &run_id,
                &config.manifest,
                &config,
                &evidence_dir,
                &governance_scope,
                &identity_key,
                &config.lease_config,
                &token,
            )
            .await
            {
                Ok(bundle) => Ok(bundle),
                // A stale Reserved/Generating entry was reclaimed and released;
                // restart the pipeline fresh so generation re-runs (exactly-once
                // still holds because the previous attempt left no proposal).
                Err(e) if e.to_string().contains("caller should retry") => {
                    Box::pin(evaluate_with_cancellation(config, token.clone())).await
                }
                Err(e) => Err(e),
            };
        }
    };

    // ---- Stage: Heartbeat ----
    // Start the heartbeat immediately after reservation so a long preflight (or
    // any stall before generation) keeps the entry live. A live `Reserved`
    // owner must never be falsely reclaimed because of immutable reservation
    // metadata. The heartbeat spans the entire nonterminal lifecycle and is
    // stopped on every exit path.
    let mut heartbeat = HeartbeatSession::start(
        repo.clone(),
        identity_key.clone(),
        fence.clone(),
        config.lease_config.heartbeat_interval,
        "ownership lost: registry entry claimed by another worker",
    );

    // ---- Stage: Preflight ----
    let preflight = run_preflight(&repo, &commit_at_start, &config.manifest, &evidence_dir);
    let mut bundle = new_bundle(&identity, &commit_at_start, &repo, &evidence_dir);

    if let Err(e) = &preflight {
        // A typed disk breach persists typed durable evidence (resource_kind,
        // configured reserve, observed free bytes, stage) and terminates as an
        // InfraBlocked resource outcome Ã¢â‚¬â€ not a generic preflight failure.
        if let Some(breach) = e.downcast_ref::<super::preflight::DiskPreflightBreach>() {
            let redactor = Redactor::new().with_known_secrets(&collect_known_secrets(&repo));
            let msg = redactor.redact(&e.to_string());
            let rec = ValidationRecord::resource_failure(
                config
                    .manifest
                    .validation_command
                    .as_deref()
                    .map(|c| redactor.redact(c)),
                CLASSIFICATION_DISK,
                &msg,
                now_iso(),
                now_iso(),
                Some("disk"),
                Some(&breach.required_bytes.to_string()),
                breach.observed_free_bytes.map(|v| v.to_string()).as_deref(),
                Some("preflight_disk"),
            );
            write_validation_artifact(&evidence_dir, &rec)?;
            bundle.validation = Some(rec);
            bundle.failure_classification = Some(CLASSIFICATION_DISK.to_string());
            bundle.final_state = EvaluationState::InfraBlocked.outcome_label().to_string();
            bundle.completed_at = now_iso();
            // Failure evidence is durable BEFORE the terminal event references it.
            write_bundle(&evidence_dir, &bundle)?;
            // Journal edge legality: a preflight disk breach is journaled as
            // PreflightBlocked carrying the typed resource classification;
            // recovery maps that classification to InfraBlocked.
            durable_transition(
                &repo,
                &identity_path,
                &run_id,
                &identity_key,
                &commit_at_start,
                &fence,
                EvaluationState::PreflightBlocked,
                None,
                Some(CLASSIFICATION_DISK.to_string()),
                // refs.dir exists by now and is repo-relative; final_json may
                // not exist yet at this point (its frozen form can be
                // absolute), and a terminal event must carry a resolvable
                // repository-relative reference.
                Some(refs.dir.clone()),
            )
            .context("failed to persist PreflightBlocked (disk) transition")?;
            heartbeat.shutdown("").await?;
            release_reservation(&repo, &identity_key, &fence)?;
            return Ok(bundle);
        }
        bundle.failure_classification = Some("preflight_blocked".to_string());
        bundle.final_state = EvaluationState::PreflightBlocked
            .outcome_label()
            .to_string();
        bundle.completed_at = now_iso();
        // Failure evidence is durable BEFORE the terminal event references it.
        write_bundle(&evidence_dir, &bundle)?;
        durable_transition(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::PreflightBlocked,
            None,
            Some("preflight_blocked".to_string()),
            Some(refs.final_json.clone()),
        )
        .context("failed to persist PreflightBlocked transition")?;
        // Release the reservation only after the failure history is recorded.
        heartbeat.shutdown("").await?;
        release_reservation(&repo, &identity_key, &fence)?;
        return Ok(bundle);
    }
    let _preflight = preflight.unwrap();
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::PreflightPassed,
        None,
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist PreflightPassed transition")?;

    // ---- Stage: Generate (with heartbeat) ----
    // Safe point: honour cancellation before the provider is invoked.
    if token.is_cancelled() {
        record_cancellation(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::PreflightPassed,
            None,
            Some(refs.dir.clone()),
        )?;
        // Stop the renewal task so the reservation becomes reclaimable rather
        // than kept alive by a detached heartbeat after this run bails.
        heartbeat
            .shutdown("evaluation cancelled before generation")
            .await?;
        bail!("evaluation cancelled by user request before generation");
    }
    transition_entry(
        &repo,
        &identity_key,
        ProposalState::Generating,
        None,
        &fence,
    )
    .context("failed to transition to Generating state")?;
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::Generating,
        None,
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist Generating transition")?;
    let scope = GenerateScope {
        allowed_paths: config.manifest.allowed_paths.clone(),
        forbidden_paths: config.manifest.forbidden_paths.clone(),
        allow_dependency_changes: config.manifest.allow_dependency_changes,
        max_files_changed: config.manifest.max_files_changed,
        max_lines_changed: config.manifest.max_lines_changed,
    };
    let patch_context = PatchProviderContext {
        task: config.manifest.goal.clone(),
        ..Default::default()
    };

    // Race generation against heartbeat failure so we can abort early if
    // ownership is lost or the heartbeat I/O fails.
    let gen_fut = crate::workflow::generate_proposal(
        &repo,
        &config.manifest.goal,
        AuthorityLevel::from_str(&config.manifest.authority)?,
        config.provider.as_ref(),
        patch_context,
        &scope,
        config.route_info.clone(),
        config.manifest.validation_command.clone(),
    );
    tokio::pin!(gen_fut);

    let mut hb_rx = heartbeat.status_receiver();
    let gen_result = loop {
        tokio::select! {
            result = &mut gen_fut => {
                break result;
            }
            _ = hb_rx.changed() => {
                if let Some(msg) = hb_rx.borrow().as_ref() {
                    bail!("heartbeat failure during generation: {msg}");
                }
            }
            // Safe point: cancellation may race an in-flight provider call. The
            // provider future is dropped; its outcome is uncertain, so the run
            // records a same-state "cancelled" event at Generating. Recovery
            // later reports GenerationOutcomeUnknown instead of re-invoking.
            _ = token.cancelled() => {
                record_cancellation(
                    &repo,
                    &identity_path,
                    &run_id,
                    &identity_key,
                    &commit_at_start,
                    &fence,
                    EvaluationState::Generating,
                    None,
                    Some(refs.dir.clone()),
                )?;
                heartbeat.shutdown(" evaluation cancelled during generation").await?;
                bail!("evaluation cancelled by user request during generation");
            }
        }
    };

    let gen_result = match gen_result {
        Ok(r) => {
            // Check heartbeat Ã¢â‚¬â€ ownership must still be held after generation.
            heartbeat.check("")?;
            r
        }
        Err(e) => {
            // Stop heartbeat and check for heartbeat errors first.
            heartbeat.shutdown("").await?;

            let msg = e.to_string();
            let classification = classify_generation_error(&msg);
            bundle.failure_classification = Some(classification.clone());
            bundle.final_state = EvaluationState::GenerationFailed
                .outcome_label()
                .to_string();
            bundle.completed_at = now_iso();
            // Failure evidence durable, then the terminal failure event, then
            // release the reservation.
            write_bundle(&evidence_dir, &bundle)?;
            durable_transition(
                &repo,
                &identity_path,
                &run_id,
                &identity_key,
                &commit_at_start,
                &fence,
                EvaluationState::GenerationFailed,
                None,
                Some(classification),
                Some(refs.final_json.clone()),
            )
            .context("failed to persist GenerationFailed transition")?;
            release_reservation(&repo, &identity_key, &fence)
                .context("failed to release reservation after generation failure")?;
            return Ok(bundle);
        }
    };

    // Proposal artifact is published and loadable BEFORE the ProposalGenerated
    // event is recorded (durability before visibility).
    let proposal = load_proposal_from_repo(&repo, &gen_result.id)?;
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::ProposalGenerated,
        Some(gen_result.id.clone()),
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist ProposalGenerated transition")?;

    // Register the proposal in the registry.
    transition_entry(
        &repo,
        &identity_key,
        ProposalState::ProposalGenerated,
        Some(&gen_result.id),
        &fence,
    )
    .context("failed to transition registry to ProposalGenerated")?;

    // Safe point: the proposal is durable and published. Cancellation here is
    // clean Ã¢â‚¬â€ a later run resumes from the durable proposal (ResumeFromProposal)
    // and never re-invokes generation.
    if token.is_cancelled() {
        record_cancellation(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::ProposalGenerated,
            Some(gen_result.id.clone()),
            Some(refs.dir.clone()),
        )?;
        heartbeat
            .shutdown(" evaluation cancelled after proposal generation")
            .await?;
        bail!("evaluation cancelled by user request after proposal generation");
    }

    bundle.proposal = Some(ProposalRecord {
        id: proposal.id.clone(),
        patch_hash: gen_result.patch_hash.clone(),
        changed_files: proposal.changed_files.clone(),
        added_lines: proposal.added_lines,
        removed_lines: proposal.removed_lines,
        base_sha: proposal.base_sha.clone(),
    });
    bundle.provider_provenance = ProviderProvenanceRecord {
        implementation: config.provider.name().to_string(),
        model: config.route_info.as_ref().and_then(|r| r.model.clone()),
        route: config
            .route_info
            .as_ref()
            .and_then(|r| r.route.clone())
            .and_then(|u| sanitize_provider_route(&u)),
        generated_at: Some(now_iso()),
        input_digest: Some(hash_str(&config.manifest.goal)),
        patch_hash: Some(gen_result.patch_hash.clone()),
    };

    // ---- Stage: Governance verification ----
    // Governance is already enforced by `generate_proposal` Ã¢â€ â€™ `propose_with_meta`.
    // Record that it passed.
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::GovernancePassed,
        Some(gen_result.id.clone()),
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist GovernancePassed transition")?;

    // Safe point: cancellation before validation starts. A later run resumes
    // validation (ResumeValidation) on the preserved proposal.
    if token.is_cancelled() {
        record_cancellation(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::GovernancePassed,
            Some(gen_result.id.clone()),
            Some(refs.dir.clone()),
        )?;
        heartbeat
            .shutdown(" evaluation cancelled before validation")
            .await?;
        bail!("evaluation cancelled by user request before validation");
    }

    // ---- Stage: Isolated dry-run validation (with heartbeat) ----
    transition_entry(
        &repo,
        &identity_key,
        ProposalState::Validating,
        Some(&gen_result.id),
        &fence,
    )
    .context("failed to transition to Validating state")?;
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::Validating,
        Some(gen_result.id.clone()),
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist Validating transition")?;

    let limits = ResourceLimits::from_environment()
        .context("failed to resolve resource limits from environment")?
        .with_manifest_disk(config.manifest.min_disk_bytes);
    let known_secrets = collect_known_secrets(&repo);
    let validation_result = run_isolated_validation(
        &repo,
        &gen_result.id,
        config.manifest.validation_command.as_deref(),
        &evidence_dir,
        &token,
        &limits,
        &known_secrets,
    )
    .await;

    // Check heartbeat after validation Ã¢â‚¬â€ must still own the entry.
    heartbeat.check("")?;

    // Safe point: cancellation after validation finished. The run records a
    // same-state "cancelled" event at Validating; a later run resumes
    // validation on the preserved proposal.
    if token.is_cancelled() {
        record_cancellation(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::Validating,
            Some(gen_result.id.clone()),
            Some(refs.dir.clone()),
        )?;
        heartbeat
            .shutdown(" evaluation cancelled during validation")
            .await?;
        bail!("evaluation cancelled by user request during validation");
    }

    // The validation record must be durable BEFORE the ValidationComplete
    // journal event references it.
    match &validation_result {
        Ok(vr) => {
            bundle.validation = Some(vr.clone());
            write_validation_artifact(&evidence_dir, vr)?;
            if vr.validation_passed {
                bundle.failure_classification =
                    Some("validation_passed_review_required".to_string());
            } else {
                let class = classify_validation_failure(vr);
                bundle.failure_classification = Some(class);
            }
        }
        Err(e) => {
            // Redact the diagnostic message before persisting it; provider/process
            // errors must never write a raw secret into the evidence bundle.
            let redactor = Redactor::new().with_known_secrets(&known_secrets);
            let msg = redactor.redact(&e.to_string());
            // A typed disk breach carries its own classification and evidence;
            // everything else falls back to free-text classification with no
            // typed resource fields.
            let (classification, kind, configured, observed, stage) =
                if let Some(breach) = e.downcast_ref::<super::preflight::DiskPreflightBreach>() {
                    (
                        CLASSIFICATION_DISK.to_string(),
                        Some("disk"),
                        Some(breach.required_bytes.to_string()),
                        breach.observed_free_bytes.map(|v| v.to_string()),
                        Some("preflight_disk"),
                    )
                } else {
                    (classify_dry_run_error(&msg), None, None, None, None)
                };
            // Durably record the failure BEFORE the ValidationComplete journal
            // event references it, so a resource/integrity rejection is never
            // lost and recovery maps it correctly.
            let cmd = config
                .manifest
                .validation_command
                .as_deref()
                .map(|c| redactor.redact(c));
            let rec = ValidationRecord::resource_failure(
                cmd,
                &classification,
                &msg,
                now_iso(),
                now_iso(),
                kind,
                configured.as_deref(),
                observed.as_deref(),
                stage,
            );
            write_validation_artifact(&evidence_dir, &rec)?;
            bundle.validation = Some(rec);
            bundle.failure_classification = Some(classification.clone());
        }
    }
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::ValidationComplete,
        Some(gen_result.id.clone()),
        bundle.failure_classification.clone(),
        Some(refs.dir.clone()),
    )
    .context("failed to persist ValidationComplete transition")?;

    // Safe point: validation is durable but the terminal event has not been
    // published. A later run resumes finalization (integrity + publication)
    // WITHOUT re-running validation.
    //
    // Test-only pause point: when a deterministic test installs a park barrier
    // on the token, the run parks here so the test can cancel the token at a
    // known, stable position (after ValidationComplete is durably journaled) and
    // then release. No-op in production (no barrier installed).
    token.park_at_safe_point().await;
    if token.is_cancelled() {
        record_cancellation(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::ValidationComplete,
            Some(gen_result.id.clone()),
            Some(refs.dir.clone()),
        )?;
        heartbeat
            .shutdown(" evaluation cancelled after validation complete")
            .await?;
        bail!("evaluation cancelled by user request after validation complete");
    }

    // ---- Stage: Repository integrity ----
    // Integrity verification RUNS before integrity is declared, and its record
    // is durable BEFORE the IntegrityVerified event.
    let integrity = verify_repo_integrity(&repo, &commit_at_start, &gen_result.id);
    bundle.integrity = Some(integrity.clone());
    write_integrity_artifact(&evidence_dir, &integrity)?;
    durable_transition(
        &repo,
        &identity_path,
        &run_id,
        &identity_key,
        &commit_at_start,
        &fence,
        EvaluationState::IntegrityVerified,
        Some(gen_result.id.clone()),
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist IntegrityVerified transition")?;

    // Safe point: integrity is durable but the terminal event has not been
    // published. A later run publishes the terminal outcome WITHOUT re-running
    // validation or integrity.
    //
    // Test-only pause point: see the earlier safe point. Parks the run here so a
    // test can cancel deterministically after IntegrityVerified is durably
    // journaled.
    token.park_at_safe_point().await;
    if token.is_cancelled() {
        record_cancellation(
            &repo,
            &identity_path,
            &run_id,
            &identity_key,
            &commit_at_start,
            &fence,
            EvaluationState::IntegrityVerified,
            Some(gen_result.id.clone()),
            Some(refs.dir.clone()),
        )?;
        heartbeat
            .shutdown(" evaluation cancelled after integrity verified")
            .await?;
        bail!("evaluation cancelled by user request after integrity verified");
    }

    // Resolve the terminal outcome.
    let terminal_state = if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
        EvaluationState::IntegrityFailed
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
            EvaluationState::ReviewGate
        } else {
            let terminal = failure_to_terminal_state(fc);
            bundle.final_state = terminal.outcome_label().to_string();
            terminal
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        EvaluationState::ReviewGate
    };

    // ---- Stage: Cleanup ----
    let cleanup = cleanup_worktree(&repo, &gen_result.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    if let Ok(head) = git_rev_parse_head(&repo) {
        bundle.repo_pin_after = head;
    }

    // Fenced finalization: evidence Ã¢â€ â€™ terminal event Ã¢â€ â€™ identity Ã¢â€ â€™ checkpoint Ã¢â€ â€™
    // registry, all under the registry lock. The terminal event references the
    // final evidence that was just written durably.
    fenced_finalize(
        &repo,
        &identity_key,
        &fence,
        &gen_result.id,
        &evidence_dir,
        &bundle,
        &heartbeat.status_receiver(),
        &identity_path,
        &run_id,
        &commit_at_start,
        terminal_state,
        bundle.failure_classification.clone(),
    )
    .context("fenced finalization failed")?;

    // Orphan reclamation of stale evaluation artifacts. Only unreferenced orphans
    // older than the retention window are removed (together with their checksum
    // sidecars). Authoritative state is explicitly protected so reclamation can
    // never delete referenced evidence: the registry + journal control metadata,
    // every referenced proposal directory (keyed by proposal id in the registry)
    // and evidence directory, and the run we just produced. Protection is built
    // fail-closed: if it cannot be computed, reclamation is SKIPPED entirely
    // rather than risk deleting authoritative state.
    match build_retention_protection(&repo, &identity_key, &refs.dir) {
        Ok(protection) => {
            let _ = reclaim_orphan_artifacts(
                &repo,
                std::time::Duration::from_secs(7 * 24 * 3600),
                &protection,
            );
        }
        Err(e) => {
            // Fail closed: refuse to delete anything if we cannot enumerate what
            // must be preserved.
            eprintln!("retention skipped: could not build protected set: {e}");
        }
    }

    // Stop heartbeat and check for errors that occurred during finalization.
    heartbeat.shutdown("").await?;

    Ok(bundle)
}

/// Build the set of artifact paths reclamation must never delete.
///
/// Covers the control metadata (proposal registry + journal + checkpoint), every
/// proposal directory and evidence directory referenced by a live/completed
/// registry entry, every evidence directory referenced by a durable journal
/// event or checkpoint, and the run that was just produced. References are
/// resolved fail-closed via [`resolve_repo_relative`]: an out-of-repository
/// reference is a hard error (so the caller skips reclamation rather than delete
/// authoritative state with an incomplete protection set, and to refuse to
/// "protect" attacker-controlled out-of-repo paths). Returns `Err` if the
/// protected set cannot be determined so reclamation is skipped entirely.
fn build_retention_protection(
    repo: &Path,
    identity_key: &str,
    current_run_dir: &str,
) -> Result<ProtectedReferences> {
    let mut protection = ProtectedReferences::new();
    let wf = repo.join(".prometheos").join("workflow");
    // Control metadata.
    protection.insert(&wf.join("proposal_registry.json"))?;
    protection.insert_dir(&wf.join("journal"))?;
    protection.insert_dir(&wf.join("checkpoint"))?;
    // Referenced proposal + evidence directories from the registry. A missing
    // registry yields an empty entry set; a lagging one simply references dirs
    // that may not exist (insert_dir skips missing paths).
    let reg = read_registry(repo)?;
    for entry in reg.entries.values() {
        if let Some(pid) = &entry.proposal_id {
            // Fail closed: a hostile id must never escape the workflow dir.
            let p = super::durable::confined_workflow_dir(repo, pid)?;
            protection.insert_dir(&p)?;
        }
        if let Some(ed) = &entry.evidence_dir {
            // Fail closed: a referenced evidence dir MUST stay inside the repo.
            let p = super::durable::resolve_repo_relative(repo, ed)?;
            protection.insert_dir(&p)?;
        }
    }
    // Authoritative proposal directories recorded by durable journal events and
    // the checkpoint snapshot (the journal is the source of truth for what ran),
    // each confined to the workflow directory.
    for ev in super::journal::read_journal(repo, identity_key)? {
        if let Some(pr) = ev.proposal_ref.as_deref() {
            let p = super::durable::confined_workflow_dir(repo, pr)?;
            protection.insert_dir(&p)?;
        }
        if let Some(er) = ev.evidence_ref.as_deref() {
            protect_evidence_ref(&mut protection, repo, er)?;
        }
    }
    if let Some(cp) = super::checkpoint::read_checkpoint(repo, identity_key)? {
        if let Some(pr) = cp.proposal_ref.as_deref() {
            let p = super::durable::confined_workflow_dir(repo, pr)?;
            protection.insert_dir(&p)?;
        }
        if let Some(er) = cp.evidence_ref.as_deref() {
            protect_evidence_ref(&mut protection, repo, er)?;
        }
    }
    // The real portable work state (when one has been exported into the
    // workflow tree): every repo-local artifact it references is protected, and
    // any reference escaping the repository fails closed so reclamation is
    // SKIPPED rather than risk deleting referenced evidence.
    let pws_path = wf.join(crate::workflow::portable_state::PORTABLE_STATE_FILENAME);
    if pws_path.exists() {
        let text = std::fs::read_to_string(&pws_path)
            .with_context(|| format!("failed to read {}", pws_path.display()))?;
        let pws = crate::workflow::portable_state::from_json(&text)
            .with_context(|| format!("corrupt portable work state {}", pws_path.display()))?;
        protection.extend_from_portable_work_state(repo, &pws)?;
    }
    // The run we just produced.
    protection.insert_dir(&repo.join(current_run_dir))?;
    Ok(protection)
}

/// Resolve an `evidence_ref` (a repo-relative directory or `evidence.json` file)
/// and protect its directory, fail-closed if the reference escapes the repo.
fn protect_evidence_ref(
    protection: &mut ProtectedReferences,
    repo: &Path,
    evidence_ref: &str,
) -> Result<()> {
    let resolved = super::durable::resolve_repo_relative(repo, evidence_ref)?;
    let dir = if resolved.file_name().and_then(|n| n.to_str()) == Some("evidence.json") {
        resolved
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| resolved.clone())
    } else {
        resolved
    };
    protection.insert_dir(&dir)?;
    Ok(())
}
// ---------------------------------------------------------------------------
// Wait and reuse (exactly-once resume after concurrent or restart)
// ---------------------------------------------------------------------------

/// Wait for another process to complete its reservation, then reuse the result.
///
/// The durable journal is authoritative; the registry is only a derived
/// coordination snapshot and must never override it. Recovery derives the
/// single allowed disposition (see [`RecoveryDisposition`]) and this function
/// reconciles the registry to it Ã¢â‚¬â€ never above it:
/// - terminal Ã¢â€ â€™ return preserved evidence;
/// - ProposalGenerated/GovernancePassed/Validating (claimable) Ã¢â€ â€™ resume validation;
/// - no journal + stale Reserved Ã¢â€ â€™ clear and let the caller retry fresh;
/// - a live owner (fresh heartbeat) is never reclaimed Ã¢â‚¬â€ wait;
/// - Generating interrupted Ã¢â€ â€™ fail closed (GenerationOutcomeUnknown).
async fn wait_and_reuse(
    repo: &Path,
    commit_at_start: &str,
    run_id: &str,
    manifest: &TaskManifest,
    config: &EvaluationConfig,
    evidence_dir: &Path,
    governance_scope: &GovernanceScopeSnapshot,
    identity_key: &str,
    lease_config: &LeaseConfig,
    token: &CancellationToken,
) -> Result<EvidenceBundle> {
    // Run validation-specific preflight first.
    if let Err(pre) = run_validation_preflight(repo, commit_at_start, manifest, evidence_dir) {
        // A typed disk breach persists its typed durable record into the
        // evidence directory before failing. This wait/reuse path holds no
        // fence, so fabricating a terminal journal transition here would be
        // unsafe; a subsequent owned run re-validates normally and journals it.
        if let Some(breach) = pre.downcast_ref::<super::preflight::DiskPreflightBreach>() {
            let redactor = Redactor::new().with_known_secrets(&collect_known_secrets(repo));
            let msg = redactor.redact(&pre.to_string());
            let rec = ValidationRecord::resource_failure(
                manifest
                    .validation_command
                    .as_deref()
                    .map(|c| redactor.redact(c)),
                CLASSIFICATION_DISK,
                &msg,
                now_iso(),
                now_iso(),
                Some("disk"),
                Some(&breach.required_bytes.to_string()),
                breach.observed_free_bytes.map(|v| v.to_string()).as_deref(),
                Some("preflight_disk"),
            );
            write_validation_artifact(evidence_dir, &rec)?;
        }
        return Err(pre);
    }

    if token.is_cancelled() {
        bail!("cancelled by caller before waiting for live owner on identity {identity_key}");
    }

    let max_wait = std::time::Duration::from_secs(300); // 5 minutes
    let poll_interval = std::time::Duration::from_millis(500);
    let mut elapsed = std::time::Duration::ZERO;

    loop {
        let recovered = recover_evaluation(repo, identity_key, None, None)
            .context("durable state recovery failed; refusing to continue")?;
        let entry = lookup_entry(repo, identity_key);

        match &recovered {
            // No durable journal history exists. Without any durable record
            // there is no immortalized proposal to protect, so a stale Reserved
            // entry is safe to clear and let a fresh run start.
            None => match &entry {
                None => {
                    // The reservation vanished (another process released it).
                    // Let the caller retry with a fresh ownership attempt.
                    bail!(
                        "identity reservation was released by another process; caller should retry"
                    );
                }
                Some(e) => {
                    if matches!(e.state, ProposalState::Reserved)
                        && is_entry_stale_at(e, lease_config, chrono::Utc::now())
                            .with_context(|| "failed to check staleness for Reserved entry")?
                    {
                        // Safe to remove the stale reservation and start fresh.
                        // Use an observed-CAS takeover so a heartbeat renewal or
                        // state transition between the staleness check and the
                        // lock cannot silently steal a live owner.
                        let obs = OwnershipObservation {
                            owner_run_id: e.owner_run_id.clone(),
                            lease_epoch: e.lease_epoch,
                            state: e.state,
                        };
                        match try_take_ownership_cas(
                            repo,
                            identity_key,
                            run_id,
                            lease_config,
                            Some(&obs),
                        )? {
                            TakeoverResult::Taken(fence) => {
                                release_reservation(repo, identity_key, &fence).context(
                                    "failed to release stale reservation after takeover",
                                )?;
                                bail!(
                                    "stale Reserved entry cleared; caller should retry to start fresh"
                                );
                            }
                            TakeoverResult::StillLive | TakeoverResult::LostRace => {
                                // Heartbeat renewed (or ownership changed) between
                                // check and lock; wait for the owner.
                            }
                        }
                    }
                    // Any other entry without a journal: wait for the owner to
                    // make durable progress or release the reservation.
                }
            },
            Some(r) => {
                let disposition = determine_recovery_disposition(
                    r,
                    entry.as_ref(),
                    lease_config,
                    chrono::Utc::now(),
                );

                match disposition {
                    RecoveryDisposition::ReturnTerminalEvidence => {
                        // Terminal outcome: never transition backward. Return the
                        // preserved evidence; no provider call, no validation rerun,
                        // no duplicate terminal publication.
                        return return_journal_completed(
                            r,
                            repo,
                            commit_at_start,
                            run_id,
                            manifest,
                            config,
                            governance_scope,
                            identity_key,
                            lease_config,
                        )
                        .await;
                    }
                    RecoveryDisposition::ResumeFromProposal
                    | RecoveryDisposition::ResumeValidation
                    | RecoveryDisposition::ReconcileSnapshots => {
                        // Resume validation on the ORIGINAL proposal Ã¢â‚¬â€ never re-invoke
                        // generation. Take ownership only when the entry is claimable
                        // (stale or absent); a live owner is never reclaimed.
                        let fence = match &entry {
                            Some(owner_entry) => {
                                let obs = OwnershipObservation {
                                    owner_run_id: owner_entry.owner_run_id.clone(),
                                    lease_epoch: owner_entry.lease_epoch,
                                    state: owner_entry.state,
                                };
                                match try_take_ownership_cas(
                                    repo,
                                    identity_key,
                                    run_id,
                                    lease_config,
                                    Some(&obs),
                                )? {
                                    TakeoverResult::Taken(f) => Some(f),
                                    TakeoverResult::StillLive | TakeoverResult::LostRace => None,
                                }
                            }
                            None => match try_reserve(repo, identity_key, run_id)? {
                                ReserveResult::Owned(f) => Some(f),
                                ReserveResult::AlreadyExists => None,
                            },
                        };
                        if let Some(fence) = fence {
                            let proposal_id = r.proposal_ref.as_deref().context(
                        "journal records ProposalGenerated/Validating without a proposal reference",
                    )?;
                            let proposal = load_proposal_from_repo(repo, proposal_id)?;
                            return resume_validation(
                                repo,
                                commit_at_start,
                                run_id,
                                manifest,
                                config,
                                evidence_dir,
                                governance_scope,
                                &proposal,
                                identity_key,
                                &fence,
                                token,
                            )
                            .await;
                        }
                        // StillLive / LostRace / AlreadyExists race: fall through to wait.
                    }
                    RecoveryDisposition::ResumeAfterValidation
                    | RecoveryDisposition::ResumeFinalization => {
                        // Validation (and possibly integrity) has already completed
                        // durably; resume FINALIZATION only. Take ownership when the
                        // entry is claimable; a live owner is never reclaimed.
                        let fence = match &entry {
                            Some(owner_entry) => {
                                let obs = OwnershipObservation {
                                    owner_run_id: owner_entry.owner_run_id.clone(),
                                    lease_epoch: owner_entry.lease_epoch,
                                    state: owner_entry.state,
                                };
                                match try_take_ownership_cas(
                                    repo,
                                    identity_key,
                                    run_id,
                                    lease_config,
                                    Some(&obs),
                                )? {
                                    TakeoverResult::Taken(f) => Some(f),
                                    TakeoverResult::StillLive | TakeoverResult::LostRace => None,
                                }
                            }
                            None => match try_reserve(repo, identity_key, run_id)? {
                                ReserveResult::Owned(f) => Some(f),
                                ReserveResult::AlreadyExists => None,
                            },
                        };
                        if let Some(fence) = fence {
                            let proposal_id = r.proposal_ref.as_deref().context(
                                "journal records a late state without a proposal reference",
                            )?;
                            let proposal = load_proposal_from_repo(repo, proposal_id)?;
                            return resume_late_finalization(
                                repo,
                                commit_at_start,
                                run_id,
                                manifest,
                                config,
                                evidence_dir,
                                governance_scope,
                                &proposal,
                                identity_key,
                                &fence,
                                token,
                                matches!(disposition, RecoveryDisposition::ResumeFinalization),
                            )
                            .await;
                        }
                        // StillLive / LostRace / AlreadyExists race: fall through to wait.
                    }
                    RecoveryDisposition::FreshReservation
                    | RecoveryDisposition::ReclaimExpiredOwner => {
                        // The journal reached only an early state (Created/PreflightPassed)
                        // with no durable proposal. The journal is append-only and the
                        // pipeline resumes from the journal tail, so a fresh run cannot
                        // legally restart here; fail closed with an actionable message
                        // rather than risk a fork or duplicate work.
                        bail!(
                            "durable journal for {identity_key} records state {:?} with no durable \
                     proposal; generation cannot be restarted safely from the append-only \
                     journal tail. Manual resolution is required.",
                            r.state
                        );
                    }
                    RecoveryDisposition::GenerationOutcomeUnknown => {
                        // A crash/cancel during an in-flight provider call: the external
                        // provider may have already completed before the process died.
                        // Refuse to auto-restart generation; fail closed (auditable).
                        bail!(
                            "durable journal for {identity_key} records state {:?} after an \
                     interruption; the external provider may have already completed before \
                     the process died. Generation outcome is unknown; refusing to \
                     auto-restart generation. Manual recovery is required.",
                            r.state
                        );
                    }
                    RecoveryDisposition::WaitForLiveOwner => {
                        // A live (heartbeating) owner holds the entry; never reclaim a
                        // live lease. Wait for it to progress or release.
                    }
                    RecoveryDisposition::FailClosed(msg) => {
                        bail!("{msg}");
                    }
                }
            }
        }

        if elapsed >= max_wait {
            bail!(
                "timed out waiting for another process to complete \
                 identity reservation after {} seconds",
                max_wait.as_secs()
            );
        }
        // Wait for the live owner to progress or release, but bail promptly if
        // the caller cancels while we are parking.
        tokio::select! {
            _ = tokio::time::sleep(poll_interval) => {}
            _ = token.cancelled() => {
                bail!(
                    "cancelled by caller while waiting for live owner on identity {identity_key}"
                );
            }
        }
        elapsed += poll_interval;
    }
}
/// Load and validate a preserved evidence bundle for a completed run.
///
/// A proposal reference is required only when the durable journal recorded one.
/// Proposal-less terminal outcomes (`PreflightBlocked`, some `GenerationFailed`)
/// return their evidence directly Ã¢â‚¬â€ no proposal is loaded and no validation is
/// re-run. The bundle is returned byte-for-byte as written.
fn load_preserved_evidence(
    evidence_dir: &Path,
    proposal_id: Option<&str>,
) -> Result<EvidenceBundle> {
    if let Some(pid) = proposal_id {
        return super::evidence::find_existing_evidence(evidence_dir, pid)
            .context("completed journal evidence missing or proposal id mismatch");
    }
    let evidence_path = evidence_dir.join("evidence.json");
    if !evidence_path.exists() {
        bail!(
            "completed journal evidence is missing: {}",
            evidence_path.display()
        );
    }
    // Verify the #115-format checksum sidecar and path binding FIRST: untrusted
    // bytes must be authenticated before they reach any parser or migration. A
    // missing or corrupt sidecar fails closed.
    let bytes = crate::workflow::artifact_integrity::read_verified(
        evidence_dir,
        &evidence_path,
        crate::workflow::artifact_integrity::ArtifactKind::Evidence,
    )
    .with_context(|| format!("failed to read evidence {}", evidence_path.display()))?;
    // Validate the durable document format (immutable evidence is validated in
    // memory, never blindly rewritten) only after integrity is confirmed, and
    // operate on the verified bytes so migration never re-reads untrusted file
    // content.
    super::migration::migrate_document_bytes(
        &evidence_path,
        super::schema::DocumentType::EvidenceBundle,
        &bytes,
    )?;
    let bundle: EvidenceBundle = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "corrupt preserved evidence bundle {}",
            evidence_path.display()
        )
    })?;
    Ok(bundle)
}

/// Return preserved evidence when the durable journal already records a
/// completed outcome, and reconcile a lagging registry entry to the terminal
/// proposal state when we can reclaim it.
async fn return_journal_completed(
    recovered: &super::recovery::RecoveredEvaluation,
    repo: &Path,
    _commit_at_start: &str,
    run_id: &str,
    _manifest: &TaskManifest,
    _config: &EvaluationConfig,
    _governance_scope: &GovernanceScopeSnapshot,
    identity_key: &str,
    lease_config: &LeaseConfig,
) -> Result<EvidenceBundle> {
    // If the registry entry is stale-owned and behind the journal, reclaim it
    // and reconcile to the terminal proposal state so future observers agree.
    // Terminal proposal-less outcomes still reconcile to the terminal registry
    // state (without a proposal id).
    if let Some(entry) = lookup_entry(repo, identity_key)
        && let Ok(true) = is_entry_stale(&entry, lease_config)
    {
        // Use an observed-CAS takeover so a concurrent renewal or transition
        // cannot be stolen; only a genuinely stale entry is reclaimed.
        let obs = OwnershipObservation {
            owner_run_id: entry.owner_run_id.clone(),
            lease_epoch: entry.lease_epoch,
            state: entry.state,
        };
        if let TakeoverResult::Taken(fence) =
            try_take_ownership_cas(repo, identity_key, run_id, lease_config, Some(&obs))?
        {
            let target = proposal_state_for_state(recovered.state)
                .unwrap_or(ProposalState::ValidationComplete);
            transition_entry(
                repo,
                identity_key,
                target,
                recovered.proposal_ref.as_deref(),
                &fence,
            )?;
        }
    }

    let evidence_dir = super::recovery::resolve_evidence_dir(
        repo,
        recovered
            .evidence_ref
            .as_deref()
            .context("completed journal state has no evidence reference")?,
    )
    .context("completed journal evidence reference cannot be resolved")?;

    // Return the preserved evidence. A proposal is loaded only when the journal
    // recorded one; proposal-less terminal outcomes return their evidence
    // directly. No provider call, no validation rerun, no duplicate publication.
    load_preserved_evidence(&evidence_dir, recovered.proposal_ref.as_deref())
        .context("failed to load preserved terminal evidence")
}
/// Resume validation from the ProposalGenerated (or later) state.
///
/// The caller must already own the entry (via `FenceToken`). Recovery is
/// re-run after takeover with the NEW fence so the authoritative journal
/// position is reconstructed and lagging snapshots are repaired under the lock.
async fn resume_validation(
    repo: &Path,
    commit_at_start: &str,
    run_id: &str,
    manifest: &TaskManifest,
    config: &EvaluationConfig,
    evidence_dir: &Path,
    governance_scope: &GovernanceScopeSnapshot,
    proposal: &ProposalArtifact,
    identity_key: &str,
    fence: &FenceToken,
    token: &CancellationToken,
) -> Result<EvidenceBundle> {
    let refs = EvidenceRefs::of(repo, evidence_dir);

    // Recover with the newly acquired fence, against the current revision.
    let recovered = recover_evaluation(repo, identity_key, Some(commit_at_start), Some(fence))
        .context("durable state recovery after takeover failed; refusing to resume")?;

    if let Some(recovered) = &recovered {
        super::recovery::ensure_resumable(
            EvaluationState::ProposalGenerated,
            &proposal.id,
            recovered,
        )
        .context("durable journal conflicts with resume")?;
    }

    // The journal is authoritative for where validation stands. Reconcile
    // idempotently if the previous run already entered Validating.
    let start_state = recovered
        .as_ref()
        .map(|r| r.state)
        .unwrap_or(EvaluationState::ProposalGenerated);

    // Reconcile the registry to the authoritative journal position so a
    // subsequent observer sees the correct state (data-driven mapping).
    let target = proposal_state_for_state(start_state).unwrap_or(ProposalState::Validating);
    if let Some(entry) = lookup_entry(repo, identity_key)
        && entry.state != target
    {
        transition_entry(repo, identity_key, target, Some(&proposal.id), fence)?;
    }

    let mut bundle = new_bundle_from_identity(
        run_id,
        &manifest.task_id,
        repo,
        commit_at_start,
        governance_scope,
        evidence_dir,
    );

    bundle.proposal = Some(ProposalRecord {
        id: proposal.id.clone(),
        patch_hash: proposal.patch_hash.clone(),
        changed_files: proposal.changed_files.clone(),
        added_lines: proposal.added_lines,
        removed_lines: proposal.removed_lines,
        base_sha: proposal.base_sha.clone(),
    });
    bundle.provider_provenance = ProviderProvenanceRecord {
        implementation: config.provider.name().to_string(),
        model: config.route_info.as_ref().and_then(|r| r.model.clone()),
        route: config
            .route_info
            .as_ref()
            .and_then(|r| r.route.clone())
            .and_then(|u| sanitize_provider_route(&u)),
        generated_at: None,
        input_digest: Some(hash_str(&manifest.goal)),
        patch_hash: Some(proposal.patch_hash.clone()),
    };

    // Persist a resumable identity snapshot (state = recovered position) so the
    // subsequent transitions have an owner-consistent identity to journal.
    let resumed_identity = ExecutionIdentity {
        run_id: run_id.to_string(),
        task_id: manifest.task_id.clone(),
        repo: repo.display().to_string(),
        repo_pin: commit_at_start.to_string(),
        model: config
            .route_info
            .as_ref()
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| "mock".to_string()),
        provider: config.provider.name().to_string(),
        governance_scope: governance_scope.clone(),
        created_at: now_iso(),
        state: start_state,
    };
    let identity_path = persist_execution_identity(evidence_dir, &resumed_identity)?;

    // Check ownership before starting validation.
    if let Some(entry) = lookup_entry(repo, identity_key)
        && (entry.owner_run_id != fence.owner_run_id || entry.lease_epoch != fence.lease_epoch)
    {
        bail!(
            "ownership lost before validation (found owner={}, epoch={})",
            entry.owner_run_id,
            entry.lease_epoch,
        );
    }

    // Transition to Validating (idempotent if the journal already says so).
    transition_entry(
        repo,
        identity_key,
        ProposalState::Validating,
        Some(&proposal.id),
        fence,
    )
    .context("failed to transition to Validating state in resume")?;
    durable_transition(
        repo,
        &identity_path,
        run_id,
        identity_key,
        commit_at_start,
        fence,
        EvaluationState::Validating,
        Some(proposal.id.clone()),
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist resume Validating transition")?;

    // Spawn a heartbeat task to protect validation from stale-reclaim.
    let mut hb = HeartbeatSession::start(
        repo.to_path_buf(),
        identity_key.to_string(),
        fence.clone(),
        config.lease_config.heartbeat_interval,
        "ownership lost during resume validation",
    );

    // Run validation on the existing proposal.
    let limits = ResourceLimits::from_environment()
        .context("failed to resolve resource limits from environment")?
        .with_manifest_disk(manifest.min_disk_bytes);
    let known_secrets = collect_known_secrets(repo);
    let validation_result = run_isolated_validation(
        repo,
        &proposal.id,
        manifest.validation_command.as_deref(),
        evidence_dir,
        token,
        &limits,
        &known_secrets,
    )
    .await;

    // Safe point: cancellation during resumed validation must be honoured the
    // same way as during a fresh validation. The run records a same-state
    // "cancelled" event at Validating and surfaces the cancellation as an
    // error, so a later run can resume validation on the preserved proposal.
    if token.is_cancelled() {
        record_cancellation(
            repo,
            &identity_path,
            run_id,
            identity_key,
            commit_at_start,
            fence,
            EvaluationState::Validating,
            Some(proposal.id.clone()),
            Some(refs.dir.clone()),
        )?;
        hb.shutdown(" evaluation cancelled during resume validation")
            .await?;
        bail!("evaluation cancelled by user request during validation");
    }

    // Validation record durable BEFORE the ValidationComplete event.
    match &validation_result {
        Ok(vr) => {
            bundle.validation = Some(vr.clone());
            write_validation_artifact(evidence_dir, vr)?;
            if vr.validation_passed {
                bundle.failure_classification =
                    Some("validation_passed_review_required".to_string());
            } else {
                let class = classify_validation_failure(vr);
                bundle.failure_classification = Some(class);
            }
        }
        Err(e) => {
            // Redact the diagnostic message before persisting it; provider/process
            // errors must never write a raw secret into the evidence bundle.
            let redactor = Redactor::new().with_known_secrets(&known_secrets);
            let msg = redactor.redact(&e.to_string());
            // A typed disk breach carries its own classification and evidence
            // (same as the fresh path).
            let (classification, kind, configured, observed, stage) =
                if let Some(breach) = e.downcast_ref::<super::preflight::DiskPreflightBreach>() {
                    (
                        CLASSIFICATION_DISK.to_string(),
                        Some("disk"),
                        Some(breach.required_bytes.to_string()),
                        breach.observed_free_bytes.map(|v| v.to_string()),
                        Some("preflight_disk"),
                    )
                } else {
                    (classify_dry_run_error(&msg), None, None, None, None)
                };
            // Durably record the failure BEFORE the ValidationComplete journal
            // event (same as the fresh path) so a resumed resource/integrity
            // rejection is never lost and recovery maps it correctly.
            let cmd = manifest
                .validation_command
                .as_deref()
                .map(|c| redactor.redact(c));
            let rec = ValidationRecord::resource_failure(
                cmd,
                &classification,
                &msg,
                now_iso(),
                now_iso(),
                kind,
                configured.as_deref(),
                observed.as_deref(),
                stage,
            );
            write_validation_artifact(evidence_dir, &rec)?;
            bundle.validation = Some(rec);
            bundle.failure_classification = Some(classification.clone());
        }
    }
    durable_transition(
        repo,
        &identity_path,
        run_id,
        identity_key,
        commit_at_start,
        fence,
        EvaluationState::ValidationComplete,
        Some(proposal.id.clone()),
        bundle.failure_classification.clone(),
        Some(refs.dir.clone()),
    )
    .context("failed to persist resume ValidationComplete transition")?;

    // Integrity verification runs before the event; its record is durable first.
    let integrity = verify_repo_integrity(repo, commit_at_start, &proposal.id);
    bundle.integrity = Some(integrity.clone());
    write_integrity_artifact(evidence_dir, &integrity)?;
    durable_transition(
        repo,
        &identity_path,
        run_id,
        identity_key,
        commit_at_start,
        fence,
        EvaluationState::IntegrityVerified,
        Some(proposal.id.clone()),
        None,
        Some(refs.dir.clone()),
    )
    .context("failed to persist resume IntegrityVerified transition")?;

    // Resolve the terminal outcome.
    let terminal_state = if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
        EvaluationState::IntegrityFailed
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
            EvaluationState::ReviewGate
        } else {
            let terminal = failure_to_terminal_state(fc);
            bundle.final_state = terminal.outcome_label().to_string();
            terminal
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        EvaluationState::ReviewGate
    };

    let cleanup = cleanup_worktree(repo, &proposal.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    if let Ok(head) = git_rev_parse_head(repo) {
        bundle.repo_pin_after = head;
    }

    // Fenced finalization under the registry lock: final evidence Ã¢â€ â€™ terminal
    // event Ã¢â€ â€™ identity Ã¢â€ â€™ checkpoint Ã¢â€ â€™ registry.
    fenced_finalize(
        repo,
        identity_key,
        fence,
        &proposal.id,
        evidence_dir,
        &bundle,
        &hb.status_receiver(),
        &identity_path,
        run_id,
        commit_at_start,
        terminal_state,
        bundle.failure_classification.clone(),
    )
    .context("fenced finalization failed during resume")?;

    // Stop heartbeat and check for errors.
    hb.shutdown(" during resume finalization").await?;

    Ok(bundle)
}

/// Resume FINALIZATION after a late cancellation (issue #114).
///
/// The durable journal records `ValidationComplete` (or `IntegrityVerified`).
/// Validation is already durable and is NEVER re-run; integrity is verified
/// (read-only, idempotent) only when it had not already been recorded. The run
/// then publishes the terminal outcome exactly once and finalizes.
async fn resume_late_finalization(
    repo: &Path,
    commit_at_start: &str,
    run_id: &str,
    manifest: &TaskManifest,
    config: &EvaluationConfig,
    evidence_dir: &Path,
    governance_scope: &GovernanceScopeSnapshot,
    proposal: &ProposalArtifact,
    identity_key: &str,
    fence: &FenceToken,
    token: &CancellationToken,
    integrity_already_done: bool,
) -> Result<EvidenceBundle> {
    // Recover with the newly acquired fence against the current revision.
    let recovered = recover_evaluation(repo, identity_key, Some(commit_at_start), Some(fence))
        .context("durable state recovery after takeover failed; refusing to finalize")?;
    if let Some(recovered) = &recovered {
        super::recovery::ensure_resumable(
            EvaluationState::ValidationComplete,
            &proposal.id,
            recovered,
        )
        .context("durable journal conflicts with late resume")?;
    }

    // The original run's evidence lives in the durable directory recorded at
    // reservation, which may differ from this run's fresh `evidence_dir` (a
    // second/final crash creates a new directory). Resolve the *source* evidence
    // directory from the recovered durable reference and load durable artifacts
    // from there, so late finalization never reads a brand-new empty directory
    // and never silently switches evidence universes.
    let source_evidence_ref = recovered.as_ref().and_then(|r| r.evidence_ref.clone());
    let source_evidence_dir = match source_evidence_ref.as_deref() {
        Some(ref_str) => {
            super::recovery::resolve_evidence_dir(repo, ref_str).with_context(|| {
                format!("durable evidence_ref {ref_str} cannot be resolved for late finalization")
            })?
        }
        None => bail!(
            "late finalization has no durable evidence_ref; refusing to switch evidence universes"
        ),
    };

    let start_state = recovered
        .as_ref()
        .map(|r| r.state)
        .unwrap_or(EvaluationState::ValidationComplete);

    // Reconcile the registry observation to the journal position.
    let target = proposal_state_for_state(start_state).unwrap_or(ProposalState::ValidationComplete);
    if let Some(entry) = lookup_entry(repo, identity_key)
        && entry.state != target
    {
        transition_entry(repo, identity_key, target, Some(&proposal.id), fence)?;
    }

    let mut bundle = new_bundle_from_identity(
        run_id,
        &manifest.task_id,
        repo,
        commit_at_start,
        governance_scope,
        evidence_dir,
    );

    // Validation is durable; load it from the original run's evidence directory,
    // never re-run it.
    let vr = read_validation_artifact(&source_evidence_dir)
        .context("failed to read durable validation record for late finalization")?;
    bundle.validation = Some(vr.clone());
    if vr.validation_passed {
        bundle.failure_classification = Some("validation_passed_review_required".to_string());
    } else {
        bundle.failure_classification = Some(classify_validation_failure(&vr));
    }

    bundle.proposal = Some(ProposalRecord {
        id: proposal.id.clone(),
        patch_hash: proposal.patch_hash.clone(),
        changed_files: proposal.changed_files.clone(),
        added_lines: proposal.added_lines,
        removed_lines: proposal.removed_lines,
        base_sha: proposal.base_sha.clone(),
    });
    bundle.provider_provenance = ProviderProvenanceRecord {
        implementation: config.provider.name().to_string(),
        model: config.route_info.as_ref().and_then(|r| r.model.clone()),
        route: config
            .route_info
            .as_ref()
            .and_then(|r| r.route.clone())
            .and_then(|u| sanitize_provider_route(&u)),
        generated_at: None,
        input_digest: Some(hash_str(&manifest.goal)),
        patch_hash: Some(proposal.patch_hash.clone()),
    };

    // Persist a resumable identity snapshot (state = recovered position).
    let resumed_identity = ExecutionIdentity {
        run_id: run_id.to_string(),
        task_id: manifest.task_id.clone(),
        repo: repo.display().to_string(),
        repo_pin: commit_at_start.to_string(),
        model: config
            .route_info
            .as_ref()
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| "mock".to_string()),
        provider: config.provider.name().to_string(),
        governance_scope: governance_scope.clone(),
        created_at: now_iso(),
        state: start_state,
    };
    let identity_path = persist_execution_identity(evidence_dir, &resumed_identity)?;

    // Check ownership before finalizing.
    if let Some(entry) = lookup_entry(repo, identity_key)
        && (entry.owner_run_id != fence.owner_run_id || entry.lease_epoch != fence.lease_epoch)
    {
        bail!(
            "ownership lost before late finalization (found owner={}, epoch={})",
            entry.owner_run_id,
            entry.lease_epoch,
        );
    }

    // Spawn a heartbeat to protect finalization from stale-reclaim.
    let mut hb = HeartbeatSession::start(
        repo.to_path_buf(),
        identity_key.to_string(),
        fence.clone(),
        config.lease_config.heartbeat_interval,
        "ownership lost during late finalization",
    );

    // Integrity handling depends on whether it was already completed durably.
    //
    // - `ResumeFinalization` (integrity_already_done): the `IntegrityVerified`
    //   result is durable at `source_evidence_dir`. Consume it; do NOT recompute
    //   integrity merely because another process resumed.
    // - `ResumeAfterValidation`: integrity has not completed durably yet. Run it
    //   exactly once and persist the artifact into the *authoritative source*
    //   evidence directory so the `IntegrityVerified` journal event (which
    //   references that directory) actually contains the artifact.
    let integrity = if integrity_already_done {
        read_integrity_artifact(&source_evidence_dir)
            .context("failed to read durable integrity record for late finalization")?
    } else {
        let verified = verify_repo_integrity(repo, commit_at_start, &proposal.id);
        write_integrity_artifact(&source_evidence_dir, &verified)
            .context("failed to persist integrity artifact for late finalization")?;
        verified
    };
    bundle.integrity = Some(integrity.clone());
    if !integrity_already_done {
        durable_transition(
            repo,
            &identity_path,
            run_id,
            identity_key,
            commit_at_start,
            fence,
            EvaluationState::IntegrityVerified,
            Some(proposal.id.clone()),
            None,
            // Point the durable evidence reference at the ORIGINAL run's evidence
            // directory (the repo-relative reference recovered from the journal)
            // so a later run resumes from the authoritative validation artifact
            // rather than this run's fresh (empty) directory.
            recovered.as_ref().and_then(|r| r.evidence_ref.clone()),
        )
        .context("failed to persist resume IntegrityVerified transition")?;
    }

    // Resolve the terminal outcome.
    let terminal_state = if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
        EvaluationState::IntegrityFailed
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
            EvaluationState::ReviewGate
        } else {
            let terminal = failure_to_terminal_state(fc);
            bundle.final_state = terminal.outcome_label().to_string();
            terminal
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        EvaluationState::ReviewGate
    };

    let cleanup = cleanup_worktree(repo, &proposal.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    if let Ok(head) = git_rev_parse_head(repo) {
        bundle.repo_pin_after = head;
    }

    // Test-only pause point: a deterministic test can park the run here, before
    // the terminal event is published, to simulate a crash/interrupt during late
    // finalization and prove a later run reclaims and finishes. No-op in
    // production (no barrier installed).
    token.park_at_safe_point().await;

    // Fenced finalization under the registry lock: final evidence Ã¢â€ â€™ terminal
    // event Ã¢â€ â€™ identity Ã¢â€ â€™ checkpoint Ã¢â€ â€™ registry. No validation re-run.
    fenced_finalize(
        repo,
        identity_key,
        fence,
        &proposal.id,
        evidence_dir,
        &bundle,
        &hb.status_receiver(),
        &identity_path,
        run_id,
        commit_at_start,
        terminal_state,
        bundle.failure_classification.clone(),
    )
    .context("fenced finalization failed during late resume")?;

    hb.shutdown(" during late finalization").await?;

    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_config_new_uses_defaults_and_preserves_inputs() {
        let manifest = TaskManifest {
            task_id: "task-1".to_string(),
            goal: "goal".to_string(),
            repo: std::path::PathBuf::from("/tmp/repo"),
            allowed_paths: vec!["src/**".to_string()],
            forbidden_paths: vec![],
            allow_dependency_changes: false,
            max_files_changed: Some(5),
            max_lines_changed: None,
            validation_command: Some("cargo test".to_string()),
            provider: "noop".to_string(),
            authority: "propose".to_string(),
            min_disk_bytes: 100 * 1024 * 1024,
            evidence_dir: None,
        };
        let provider: Box<dyn PatchProvider> = Box::new(NoopProvider);
        let config = EvaluationConfig::new(manifest.clone(), provider);

        assert!(config.route_info.is_none());
        assert_eq!(
            config.lease_config.stale_reservation_timeout,
            LeaseConfig::default().stale_reservation_timeout
        );
        assert_eq!(
            config.lease_config.generation_lease_timeout,
            LeaseConfig::default().generation_lease_timeout
        );
        assert_eq!(
            config.lease_config.heartbeat_interval,
            LeaseConfig::default().heartbeat_interval
        );
        assert_eq!(config.manifest.task_id, "task-1");
        assert_eq!(config.manifest.goal, "goal");
        assert_eq!(config.provider.name(), "noop");
    }

    struct NoopProvider;

    #[async_trait::async_trait]
    impl PatchProvider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }

        async fn generate(
            &self,
            _request: crate::harness::patch_provider::GenerateRequest,
        ) -> anyhow::Result<crate::harness::patch_provider::GenerateResponse> {
            Ok(crate::harness::patch_provider::GenerateResponse {
                candidates: vec![],
                generation_time_ms: 0,
                provider_notes: None,
            })
        }
    }

    // ------------------------------------------------------------------
    // Orchestrator-level exact-once / typed-evidence harness
    // ------------------------------------------------------------------

    use crate::harness::edit_protocol::{CreateFileEdit, EditOperation};
    use crate::harness::patch_provider::{
        GenerateRequest, GenerateResponse, ProviderCandidate, RiskEstimate,
    };
    use crate::workflow::evaluate::integrity::git_rev_parse_head;
    use crate::workflow::evaluate::preflight::run_preflight;
    use crate::workflow::evaluate::recovery::recover_evaluation;
    use crate::workflow::evaluate::resource::{CLASSIFICATION_CPU, CLASSIFICATION_TIMEOUT};
    use std::process::Command as TestCommand;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrd};

    /// Serializes tests that mutate process-global environment variables.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn set_env(key: &str, value: &str) {
        unsafe { std::env::set_var(key, value) };
    }

    fn del_env(key: &str) {
        unsafe { std::env::remove_var(key) };
    }

    struct HarnessProvider {
        calls: std::sync::Arc<AtomicUsize>,
        error: Option<String>,
    }

    #[async_trait::async_trait]
    impl PatchProvider for HarnessProvider {
        fn name(&self) -> &str {
            "harness-mock"
        }

        async fn generate(&self, _request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
            self.calls.fetch_add(1, AtomicOrd::SeqCst);
            if let Some(err) = &self.error {
                return Err(anyhow::anyhow!("{err}"));
            }
            Ok(GenerateResponse {
                candidates: vec![ProviderCandidate {
                    edits: vec![EditOperation::CreateFile(CreateFileEdit {
                        file: std::path::PathBuf::from("docs/note.txt"),
                        content: "hello\n".to_string(),
                        executable: None,
                    })],
                    source: "mock".to_string(),
                    strategy: "whole_file".to_string(),
                    confidence: 99,
                    reasoning: "test candidate".to_string(),
                    estimated_risk: RiskEstimate::Low,
                }],
                generation_time_ms: 0,
                provider_notes: None,
            })
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        let out = TestCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git must be runnable");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_orch_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test"]);
        std::fs::write(repo.join("README.md"), "base\n").expect("write base file");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        dir
    }

    fn orch_manifest(
        repo: &Path,
        task_id: &str,
        validation_command: Option<String>,
        evidence_dir: Option<std::path::PathBuf>,
    ) -> TaskManifest {
        TaskManifest {
            task_id: task_id.to_string(),
            goal: "add a note file".to_string(),
            repo: repo.to_path_buf(),
            allowed_paths: vec!["docs/**".to_string()],
            forbidden_paths: vec![],
            allow_dependency_changes: false,
            max_files_changed: Some(5),
            max_lines_changed: None,
            validation_command,
            provider: "mock".to_string(),
            authority: "propose".to_string(),
            min_disk_bytes: 1024 * 1024,
            evidence_dir,
        }
    }

    fn marker_command(path: &Path) -> String {
        #[cfg(windows)]
        {
            // Wrapped so the preflight availability probe (`where`) resolves
            // the first token to a real executable, not a cmd builtin.
            format!("cmd /C echo run>>{}", path.display())
        }
        #[cfg(not(windows))]
        {
            format!("sh -c 'echo run >> {}'", path.display())
        }
    }

    fn marker_count(path: &Path) -> usize {
        std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }

    fn runaway_command() -> &'static str {
        #[cfg(windows)]
        {
            "ping -n 30 127.0.0.1 >nul"
        }
        #[cfg(not(windows))]
        {
            "sleep 30"
        }
    }

    fn cpu_burn_command() -> &'static str {
        #[cfg(windows)]
        {
            // Infinite cmd loop; wrapped so the preflight availability probe
            // (`where`) resolves the first token to a real executable.
            "cmd /C for /l %i in (0,0,1) do @rem"
        }
        #[cfg(not(windows))]
        {
            // Wrapped so the preflight availability probe (`which`) resolves
            // the first token to a real executable, not a shell keyword.
            "sh -c 'while true; do :; done'"
        }
    }

    #[test]
    fn orch_harness_repo_passes_fresh_preflight() {
        let dir = init_orch_repo();
        let repo = dir.path().join("repo");
        let ev = repo.join(".prometheos").join("workflow").join("ev-pf");
        std::fs::create_dir_all(&ev).unwrap();
        let manifest = orch_manifest(&repo, "pf", None, None);
        match run_preflight(&repo, "head", &manifest, &ev) {
            Ok(p) => {
                assert!(p.working_tree_clean, "{p:?}");
                assert!(p.disk_space_sufficient, "{p:?}");
                assert!(p.validation_command_available, "{p:?}");
                assert!(p.evidence_dir_writable, "{p:?}");
                assert!(p.credential_available, "{p:?}");
                assert!(p.governance_scope_valid, "{p:?}");
            }
            Err(e) => panic!("fresh harness repo must pass preflight: {e:#}"),
        }
    }

    #[tokio::test]
    async fn completed_run_is_reused_exactly_once_without_reexecution() {
        let dir = init_orch_repo();
        let repo = dir.path().join("repo");
        let marker = dir.path().join("marker.txt");
        let calls = std::sync::Arc::new(AtomicUsize::new(0));

        // Run 1: completes the full pipeline (provider + validation).
        let cfg1 = EvaluationConfig::new(
            orch_manifest(&repo, "oo-once", Some(marker_command(&marker)), None),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle1 = evaluate(cfg1)
            .await
            .expect("first run must complete successfully");
        if bundle1.validation.is_none() {
            panic!(
                "run1 produced no validation record; class={:?} state={:?}",
                bundle1.failure_classification, bundle1.final_state
            );
        }
        assert!(bundle1.validation.as_ref().unwrap().validation_passed);
        assert_eq!(calls.load(AtomicOrd::SeqCst), 1);
        assert_eq!(marker_count(&marker), 1);

        // Run 2: same identity (task/repo/commit/command/scope). The durable
        // journal is terminal, so this run must reuse the preserved evidence
        // WITHOUT calling the provider again and WITHOUT re-running
        // validation.
        let cfg2 = EvaluationConfig::new(
            orch_manifest(&repo, "oo-once", Some(marker_command(&marker)), None),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle2 = match evaluate(cfg2).await {
            Ok(b) => b,
            Err(e) => {
                let jroot = repo.join(".prometheos").join("workflow").join("journal");
                for f in walk_files(&jroot) {
                    if let Ok(t) = std::fs::read_to_string(&f) {
                        eprintln!("JRN {} => {t}", f.display());
                    }
                }
                panic!("reuse failed: {e:#}")
            }
        };
        assert_eq!(
            calls.load(AtomicOrd::SeqCst),
            1,
            "provider must not be re-invoked on reuse"
        );
        assert_eq!(
            marker_count(&marker),
            1,
            "validation command must not re-execute on reuse"
        );
        assert_eq!(
            bundle2.validation.as_ref().unwrap().validation_passed,
            bundle1.validation.as_ref().unwrap().validation_passed
        );
        assert_eq!(bundle2.run_id, bundle1.run_id, "preserved run identity");
    }

    #[tokio::test]
    async fn crash_after_validation_journal_fails_closed_without_rerun() {
        let dir = init_orch_repo();
        let repo = dir.path().join("repo");
        let marker = dir.path().join("marker2.txt");
        let calls = std::sync::Arc::new(AtomicUsize::new(0));

        // Run 1 parks deterministically at the post-IntegrityVerified safe
        // point (validation and integrity durable, terminal publication
        // pending), then is cancelled â€” a crash right before publication.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let token = CancellationToken::with_park_barrier(barrier.clone());
        let cfg1 = EvaluationConfig::new(
            orch_manifest(&repo, "crash-once", Some(marker_command(&marker)), None),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let j = tokio::spawn(evaluate_with_cancellation(cfg1, token.clone()));
        // The run parks twice (post-ValidationComplete, post-IntegrityVerified);
        // each 2-party generation pairs one park with one test wait.
        barrier.wait().await;
        barrier.wait().await;
        token.cancel();
        let r1 = j.await.expect("task joins");
        assert!(
            r1.is_err(),
            "cancelled mid-flight run must surface cancellation"
        );
        assert_eq!(
            calls.load(AtomicOrd::SeqCst),
            1,
            "provider executed exactly once"
        );
        assert_eq!(marker_count(&marker), 1, "validation executed exactly once");

        // Crash durability invariants: the journal tail records a cooperative
        // cancellation at ValidationComplete/IntegrityVerified with a
        // resolvable evidence reference, and the terminal evidence bundle was
        // NOT published. A later run can therefore resume publication without
        // any second execution (proven by completed_run_is_reused_*).
        let jroot = repo.join(".prometheos").join("workflow").join("journal");
        let mut tail_state = String::new();
        let mut tail_class = String::new();
        let mut tail_evidence = String::new();
        let mut files: Vec<_> = walk_files(&jroot);
        files.sort();
        if let Some(last) = files.last() {
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(last).unwrap()).unwrap();
            tail_state = v["to_state"].as_str().unwrap().to_string();
            tail_class = v["failure_classification"]
                .as_str()
                .unwrap_or("")
                .to_string();
            tail_evidence = v["evidence_ref"].as_str().unwrap_or("").to_string();
        }
        assert_eq!(tail_state, "integrity_verified", "journal tail state");
        assert_eq!(tail_class, "cancelled", "journal tail classification");
        assert!(
            !tail_evidence.is_empty(),
            "tail event keeps its evidence reference"
        );
        let ev_json = repo.join(&tail_evidence).join("evidence.json");
        assert!(
            !ev_json.exists(),
            "terminal evidence must not be published after the crash"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // serializes process-global env across the awaited run
    async fn cpu_breach_via_orchestrator_carries_typed_durable_evidence() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_env("PROMETHEOS_MAX_CPU_SECS", "1");
        set_env("PROMETHEOS_VALIDATION_TIMEOUT_SECS", "60");
        let dir = init_orch_repo();
        let repo = dir.path().join("repo");
        let calls = std::sync::Arc::new(AtomicUsize::new(0));

        let cfg1 = EvaluationConfig::new(
            orch_manifest(
                &repo,
                "cpu-orch",
                Some(cpu_burn_command().to_string()),
                None,
            ),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle1 = evaluate(cfg1)
            .await
            .expect("resource breach returns a completed, classified bundle");
        if bundle1.validation.is_none() {
            panic!(
                "cpu run produced no record; class={:?} state={:?}",
                bundle1.failure_classification, bundle1.final_state
            );
        }
        del_env("PROMETHEOS_MAX_CPU_SECS");
        del_env("PROMETHEOS_VALIDATION_TIMEOUT_SECS");

        let rec = bundle1.validation.as_ref().expect("durable record");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU),
            "cpu record: {rec:?}"
        );
        assert_eq!(rec.resource_kind.as_deref(), Some("cpu"));
        assert_eq!(rec.configured_limit.as_deref(), Some("1s"));
        assert_eq!(
            bundle1.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU)
        );

        // Reuse: second run returns the preserved typed evidence without any
        // new provider call.
        let cfg2 = EvaluationConfig::new(
            orch_manifest(
                &repo,
                "cpu-orch",
                Some(cpu_burn_command().to_string()),
                None,
            ),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle2 = evaluate(cfg2).await.expect("reuse must succeed");
        assert_eq!(calls.load(AtomicOrd::SeqCst), 1);
        assert_eq!(
            bundle2.failure_classification.as_deref(),
            Some(CLASSIFICATION_CPU)
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // serializes process-global env across the awaited run
    async fn timeout_breach_via_orchestrator_carries_typed_durable_evidence() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_env("PROMETHEOS_VALIDATION_TIMEOUT_SECS", "1");
        let dir = init_orch_repo();
        let repo = dir.path().join("repo");
        let calls = std::sync::Arc::new(AtomicUsize::new(0));

        let cfg1 = EvaluationConfig::new(
            orch_manifest(&repo, "tmo-orch", Some(runaway_command().to_string()), None),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle1 = evaluate(cfg1)
            .await
            .expect("timeout yields classified bundle");
        del_env("PROMETHEOS_VALIDATION_TIMEOUT_SECS");

        let rec = bundle1.validation.as_ref().expect("durable record");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_TIMEOUT)
        );
        assert_eq!(rec.resource_kind.as_deref(), Some("timeout"));
        assert_eq!(rec.configured_limit.as_deref(), Some("1s"));

        let cfg2 = EvaluationConfig::new(
            orch_manifest(&repo, "tmo-orch", Some(runaway_command().to_string()), None),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle2 = evaluate(cfg2).await.expect("reuse must succeed");
        assert_eq!(calls.load(AtomicOrd::SeqCst), 1);
        assert_eq!(
            bundle2.failure_classification.as_deref(),
            Some(CLASSIFICATION_TIMEOUT)
        );
    }

    #[tokio::test]
    async fn disk_breach_persists_typed_evidence_and_recovers_infra_blocked() {
        let dir = init_orch_repo();
        let repo = dir.path().join("repo");
        let calls = std::sync::Arc::new(AtomicUsize::new(0));

        let marker3 = dir.path().join("m3.txt");
        let mut manifest = orch_manifest(&repo, "disk-orch", Some(marker_command(&marker3)), None);
        manifest.min_disk_bytes = u64::MAX / 2; // absurd reserve Ã¢â€ â€™ typed breach
        let validation_cmd = manifest.validation_command.clone();

        let cfg1 = EvaluationConfig::new(
            manifest,
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let bundle1 = evaluate(cfg1)
            .await
            .expect("typed disk breach returns a completed, classified bundle");

        let rec = bundle1.validation.as_ref().expect("typed disk record");
        assert_eq!(
            rec.failure_classification.as_deref(),
            Some(CLASSIFICATION_DISK),
            "{rec:?}"
        );
        assert_eq!(rec.resource_kind.as_deref(), Some("disk"));
        assert_eq!(rec.stage.as_deref(), Some("preflight_disk"));
        assert_eq!(
            rec.configured_limit.as_deref(),
            Some((u64::MAX / 2).to_string()).as_deref()
        );
        assert!(
            rec.observed_value
                .as_ref()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                > 0,
            "observed free bytes must be present: {rec:?}"
        );
        // The provider was never invoked: preflight failed first.
        assert_eq!(calls.load(AtomicOrd::SeqCst), 0);

        // Recovery maps the journaled classification to InfraBlocked.
        let commit_at_start = git_rev_parse_head(&repo).unwrap();
        let governance_scope = GovernanceScopeSnapshot {
            allowed_paths: vec!["docs/**".to_string()],
            forbidden_paths: vec![],
            allow_dependency_changes: false,
            max_files_changed: Some(5),
            max_lines_changed: None,
            authority: "propose".to_string(),
            validation_command: validation_cmd.clone(),
        };
        let identity_key = compute_identity_key(
            "disk-orch",
            &repo,
            &commit_at_start,
            "harness-mock",
            "mock",
            &governance_scope,
            &validation_cmd,
        );
        let recovered = recover_evaluation(&repo, &identity_key, None, None)
            .expect("recovery reads")
            .expect("journal exists");
        // Journal edge legality: preflight-stage disk breach journals as
        // PreflightBlocked carrying the typed resource classification;
        // recovery consumers map that classification to InfraBlocked.
        assert_eq!(recovered.state, EvaluationState::PreflightBlocked);
        assert_eq!(
            recovered.last_failure_classification.as_deref(),
            Some(CLASSIFICATION_DISK)
        );
        assert_eq!(
            failure_to_terminal_state(CLASSIFICATION_DISK),
            EvaluationState::InfraBlocked
        );

        // The preserved evidence bundle carries the same typed record, loaded
        // through the integrity-checked path.
        {
            let ev_dir = repo.join(
                recovered
                    .evidence_ref
                    .as_deref()
                    .expect("terminal event keeps its evidence reference"),
            );
            let bytes = crate::workflow::artifact_integrity::read_verified(
                &ev_dir,
                &ev_dir.join("evidence.json"),
                crate::workflow::artifact_integrity::ArtifactKind::Evidence,
            )
            .expect("preserved bundle verifies");
            let preserved: crate::workflow::evaluate::evidence::EvidenceBundle =
                serde_json::from_slice(&bytes).expect("preserved bundle parses");
            let rec2 = preserved.validation.as_ref().expect("preserved record");
            assert_eq!(rec2.resource_kind.as_deref(), Some("disk"));
            assert_eq!(rec2.stage.as_deref(), Some("preflight_disk"));
        }

        // A retry of the same identity after the terminal failure is refused
        // by append-only journal continuity â€” it can never silently re-execute
        // (the provider counter stays at zero).
        let cfg2 = EvaluationConfig::new(
            orch_manifest(&repo, "disk-orch", validation_cmd.clone(), None),
            Box::new(HarnessProvider {
                calls: calls.clone(),
                error: None,
            }),
        );
        let err = evaluate(cfg2)
            .await
            .expect_err("retry after terminal journal must fail closed");
        assert!(
            format!("{err:#}").contains("continuity"),
            "unexpected retry failure: {err:#}"
        );
        assert_eq!(
            calls.load(AtomicOrd::SeqCst),
            0,
            "provider must never be invoked across the terminal boundary"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // serializes process-global env across the awaited run
    async fn provider_error_and_stderr_secrets_are_never_persisted() {
        const CANARY: &str = crate::workflow::redaction::SECRET_CANARY;
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_env("PROMETHEOS_KNOWN_SECRETS", CANARY);

        // Surface A: provider error text embedding the canary in plain,
        // URL-userinfo and query forms. The pipeline must persist surfaces
        // (identity, journal, registry, evidence bundle/markdown) with zero
        // canary occurrences.
        {
            let dir = init_orch_repo();
            let repo = dir.path().join("repo");
            let calls = std::sync::Arc::new(AtomicUsize::new(0));
            let cfg = EvaluationConfig::new(
                orch_manifest(&repo, "canary-gen", None, None),
                Box::new(HarnessProvider {
                    calls: calls.clone(),
                    error: Some(format!(
                        "upstream exploded key={CANARY} at https://u:{CANARY}@host/x?token={CANARY}"
                    )),
                }),
            );
            let bundle = evaluate(cfg)
                .await
                .expect("generation failure returns a classified bundle");
            assert_ne!(
                bundle.failure_classification.as_deref(),
                None,
                "generation failure must be classified"
            );
            let mut found = 0usize;
            let mut leaked_in: Vec<String> = Vec::new();
            let prom = repo.join(".prometheos");
            for entry in walk_files(&prom) {
                if let Ok(text) = std::fs::read_to_string(&entry) {
                    let n = text.matches(CANARY).count();
                    if n > 0 {
                        leaked_in.push(format!("{entry:?} x{n}"));
                    }
                    found += n;
                }
            }
            assert_eq!(
                found, 0,
                "canary leaked into persisted surfaces: {leaked_in:?}"
            );
        }

        // Surface B: validation STDERR echoing the canary at RUNTIME (the
        // command text itself stays clean â€” it is user input persisted in
        // proposal/identity artifacts). Raw logs, previews, markdown and
        // journal must all be redacted.
        {
            set_env("PROMETHEOS_TEST_CANARY", CANARY);
            let dir = init_orch_repo();
            let repo = dir.path().join("repo");
            #[cfg(windows)]
            let stderr_cmd: String =
                "cmd /C echo key=%PROMETHEOS_TEST_CANARY% 1>&2 & exit /b 3".to_string();
            #[cfg(not(windows))]
            let stderr_cmd: String =
                "sh -c 'echo key=$PROMETHEOS_TEST_CANARY >&2; exit 3'".to_string();
            let cfg = EvaluationConfig::new(
                orch_manifest(&repo, "canary-val", Some(stderr_cmd), None),
                Box::new(HarnessProvider {
                    calls: std::sync::Arc::new(AtomicUsize::new(0)),
                    error: None,
                }),
            );
            let bundle = evaluate(cfg)
                .await
                .expect("failed validation returns a classified bundle");
            let rec = bundle.validation.as_ref().expect("record");
            assert!(
                !rec.stderr_preview.contains(CANARY),
                "stderr preview leaked the canary"
            );
            let mut found = 0usize;
            let mut leaked_in: Vec<String> = Vec::new();
            let prom = repo.join(".prometheos");
            for entry in walk_files(&prom) {
                if let Ok(text) = std::fs::read_to_string(&entry) {
                    let n = text.matches(CANARY).count();
                    if n > 0 {
                        leaked_in.push(format!("{entry:?} x{n}"));
                    }
                    found += n;
                }
            }
            assert_eq!(
                found, 0,
                "canary leaked into persisted surfaces: {leaked_in:?}"
            );
            del_env("PROMETHEOS_TEST_CANARY");
        }

        del_env("PROMETHEOS_KNOWN_SECRETS");
    }

    fn walk_files(root: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(entries) = std::fs::read_dir(&d) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.is_file() {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // Retention protection wiring (real PWS source, confinement)
    // ------------------------------------------------------------------

    #[test]
    fn retention_protection_tolerates_missing_registry_and_confines_refs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();

        // Missing registry / journal / checkpoint entirely.
        build_retention_protection(repo, "key-x", "cur-run")
            .expect("missing control documents are tolerated");

        // A hostile proposal reference in the durable journal must fail closed.
        let jdir = repo
            .join(".prometheos")
            .join("workflow")
            .join("journal")
            .join("key-y");
        std::fs::create_dir_all(&jdir).unwrap();
        std::fs::write(
            jdir.join(format!("{:020}.json", 0)),
            serde_json::json!({
                "schema_version": "1.0.0", "event_id": "e0", "sequence": 0,
                "run_id": "r1", "identity_key": "key-y", "timestamp": "2026-01-01T00:00:00Z",
                "from_state": "created", "to_state": "generating",
                "proposal_ref": "../evil", "failure_classification": null,
                "owner_run_id": "r1", "lease_epoch": 1,
                "repository_revision": "abc", "evidence_ref": null, "checkpoint_ref": null
            })
            .to_string(),
        )
        .unwrap();
        let err = build_retention_protection(repo, "key-y", "cur-run")
            .expect_err("hostile proposal ref must fail closed");
        assert!(
            err.to_string().contains("escapes repository"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn retention_wires_the_real_portable_work_state_source() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let wf = repo.join(".prometheos").join("workflow");
        std::fs::create_dir_all(&wf).unwrap();
        let pws_path = wf.join(crate::workflow::portable_state::PORTABLE_STATE_FILENAME);

        // Corrupt PWS document: fail closed (reclamation skipped).
        std::fs::write(&pws_path, "{ definitely not json").unwrap();
        let err =
            build_retention_protection(repo, "k", "cur").expect_err("corrupt PWS must fail closed");
        assert!(err.to_string().contains("portable work state"), "{err}");

        // Valid PWS referencing an existing in-repo artifact: protected.
        let target = repo.join("artifacts").join("keep.bin");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "x").unwrap();
        std::fs::write(
            &pws_path,
            serde_json::json!({
                "schema_version": crate::workflow::schema::CURRENT_SCHEMA_VERSION.to_string_owned(),
                "work": {"work_id":"w","task_id":"t","objective":"o",
                         "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"},
                "repository": {"identity":"origin","branch":"main","revision":"abc"},
                "artifact_refs":[{"kind":"artifact","uri":"artifacts/keep.bin"}],
                "authority": {"authority":"propose","allow_dependency_changes":false},
                "compatibility": {"state_schema_version":
                    crate::workflow::schema::PORTABLE_WORK_STATE_SCHEMA_VERSION.to_string_owned()},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        let prot =
            build_retention_protection(repo, "k", "cur").expect("valid PWS must wire through");
        assert!(
            prot.contains(&target),
            "PWS-referenced artifact must be protected by the wired source"
        );
    }
}
