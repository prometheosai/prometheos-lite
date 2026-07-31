use anyhow::{Context, Result, bail};
use std::path::Path;
use std::str::FromStr;

use crate::harness::patch_provider::{PatchProvider, PatchProviderContext};
use crate::workflow::{
    AuthorityLevel, GenerateScope, ProposalArtifact, ProviderRouteInfo, is_git_repo,
    sanitize_provider_route,
};

use super::cleanup::cleanup_worktree;
use super::evidence::{
    EvidenceBundle, ProposalRecord, ProviderProvenanceRecord, find_existing_evidence, new_bundle,
    new_bundle_from_identity, write_bundle,
};
use super::generation::{classify_generation_error, load_proposal_from_repo};
use super::identity::{
    EvaluationState, ExecutionIdentity, GovernanceScopeSnapshot, TaskManifest,
    compute_identity_key, evidence_dir_for, hash_str, now_iso, update_identity_state,
};
use super::integrity::{git_rev_parse_head, verify_repo_integrity};
use super::preflight::{run_preflight, run_validation_preflight};
use super::registry::{
    FenceToken, LeaseConfig, ProposalState, ReserveResult, TakeoverResult, fenced_finalize,
    is_entry_stale, lookup_entry, release_reservation, renew_heartbeat, transition_entry,
    try_reserve, try_take_ownership,
};
use super::validation::{
    classify_dry_run_error, classify_validation_failure, failure_to_terminal_state,
    run_isolated_validation,
};

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
/// Run the full evaluation pipeline and return the evidence bundle.
///
/// This is the primary entry point for the `workflow evaluate` command.
pub async fn evaluate(config: EvaluationConfig) -> Result<EvidenceBundle> {
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
    std::fs::create_dir_all(&evidence_dir)
        .with_context(|| format!("failed to create evidence dir: {}", evidence_dir.display()))?;

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
    let identity_path = evidence_dir.join("execution_identity.json");
    std::fs::write(
        &identity_path,
        serde_json::to_string_pretty(&identity).context("failed to serialize identity")?,
    )
    .context("failed to persist execution identity")?;

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
            return wait_and_reuse(
                &repo,
                &commit_at_start,
                &run_id,
                &config.manifest,
                &config,
                &evidence_dir,
                &governance_scope,
                &identity_key,
                &config.lease_config,
            )
            .await;
        }
    };

    // ---- Stage: Preflight ----
    let preflight = run_preflight(&repo, &commit_at_start, &config.manifest, &evidence_dir);
    let mut bundle = new_bundle(&identity, &commit_at_start, &repo, &evidence_dir);

    if let Err(_e) = &preflight {
        bundle.failure_classification = Some("preflight_blocked".to_string());
        bundle.final_state = EvaluationState::PreflightBlocked
            .outcome_label()
            .to_string();
        bundle.completed_at = now_iso();
        write_bundle(&evidence_dir, &bundle)?;
        // Release the reservation so another process can retry.
        let _ = release_reservation(&repo, &identity_key, &fence);
        return Ok(bundle);
    }
    let _preflight = preflight.unwrap();
    update_identity_state(&identity_path, EvaluationState::PreflightPassed);

    // ---- Stage: Generate (with heartbeat) ----
    transition_entry(
        &repo,
        &identity_key,
        ProposalState::Generating,
        None,
        &fence,
    )
    .context("failed to transition to Generating state")?;
    update_identity_state(&identity_path, EvaluationState::Generating);
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

    // Spawn a heartbeat task that renews the lease while the pipeline is active.
    // The heartbeat runs from `Generating` through `Validating` and stops only
    // when the pipeline reaches a terminal or `ValidationComplete`.
    // Errors and ownership loss are propagated via the `heartbeat_status` channel.
    let heartbeat_repo = repo.clone();
    let heartbeat_key = identity_key.clone();
    let heartbeat_fence = fence.clone();
    let heartbeat_interval = config.lease_config.heartbeat_interval;
    let (heartbeat_stop_tx, mut heartbeat_stop_rx) = tokio::sync::watch::channel(false);
    let (heartbeat_status_tx, heartbeat_status_rx) =
        tokio::sync::watch::channel::<Option<String>>(None);
    let heartbeat_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(heartbeat_interval) => {}
                _ = heartbeat_stop_rx.changed() => {
                    if *heartbeat_stop_rx.borrow() {
                        break;
                    }
                }
            }
            if *heartbeat_stop_rx.borrow() {
                break;
            }
            match renew_heartbeat(&heartbeat_repo, &heartbeat_key, &heartbeat_fence) {
                Err(e) => {
                    let _ = heartbeat_status_tx.send(Some(format!("{e:#}")));
                    break;
                }
                Ok(false) => {
                    let _ = heartbeat_status_tx.send(Some(
                        "ownership lost: registry entry claimed by another worker".to_string(),
                    ));
                    break;
                }
                Ok(true) => {}
            }
        }
    });

    macro_rules! check_heartbeat {
        () => {
            if let Some(msg) = heartbeat_status_rx.borrow().as_ref() {
                bail!("heartbeat failure: {msg}");
            }
        };
    }

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

    let mut hb_rx = heartbeat_status_rx.clone();
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
        }
    };

    let gen_result = match gen_result {
        Ok(r) => {
            // Check heartbeat — ownership must still be held after generation.
            check_heartbeat!();
            r
        }
        Err(e) => {
            // Stop heartbeat and check for heartbeat errors first.
            let _ = heartbeat_stop_tx.send(true);
            let _ = heartbeat_handle.await;
            check_heartbeat!();

            let msg = e.to_string();
            let classification = classify_generation_error(&msg);
            bundle.failure_classification = Some(classification);
            bundle.final_state = EvaluationState::GenerationFailed
                .outcome_label()
                .to_string();
            bundle.completed_at = now_iso();
            // Release the reservation so another process can retry.
            release_reservation(&repo, &identity_key, &fence)
                .context("failed to release reservation after generation failure")?;
            write_bundle(&evidence_dir, &bundle)?;
            return Ok(bundle);
        }
    };

    update_identity_state(&identity_path, EvaluationState::ProposalGenerated);

    // Register the proposal in the registry.
    transition_entry(
        &repo,
        &identity_key,
        ProposalState::ProposalGenerated,
        Some(&gen_result.id),
        &fence,
    )
    .context("failed to transition registry to ProposalGenerated")?;

    let proposal = load_proposal_from_repo(&repo, &gen_result.id)?;
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
    // Governance is already enforced by `generate_proposal` → `propose_with_meta`.
    // Record that it passed.
    update_identity_state(&identity_path, EvaluationState::GovernancePassed);

    // ---- Stage: Isolated dry-run validation (with heartbeat) ----
    // Transition to Validating state so other workers know validation is live.
    transition_entry(
        &repo,
        &identity_key,
        ProposalState::Validating,
        Some(&gen_result.id),
        &fence,
    )
    .context("failed to transition to Validating state")?;
    update_identity_state(&identity_path, EvaluationState::Validating);

    let validation_result = run_isolated_validation(
        &repo,
        &gen_result.id,
        config.manifest.validation_command.as_deref(),
        &evidence_dir,
    );

    // Check heartbeat after validation — must still own the entry.
    check_heartbeat!();

    update_identity_state(&identity_path, EvaluationState::ValidationComplete);

    match &validation_result {
        Ok(vr) => {
            bundle.validation = Some(vr.clone());
            if vr.validation_passed {
                bundle.failure_classification =
                    Some("validation_passed_review_required".to_string());
            } else {
                let class = classify_validation_failure(vr);
                bundle.failure_classification = Some(class);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let classification = classify_dry_run_error(&msg);
            bundle.failure_classification = Some(classification);
        }
    }

    // ---- Stage: Repository integrity ----
    update_identity_state(&identity_path, EvaluationState::IntegrityVerified);
    let integrity = verify_repo_integrity(&repo, &commit_at_start, &gen_result.id);
    bundle.integrity = Some(integrity.clone());

    if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        } else {
            // Map the failure classification to a terminal state.
            bundle.final_state = failure_to_terminal_state(fc).outcome_label().to_string();
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
    }

    // ---- Stage: Cleanup ----
    let cleanup = cleanup_worktree(&repo, &gen_result.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    // Fill repo_pin_after on the returned bundle.
    if let Ok(head) = git_rev_parse_head(&repo) {
        bundle.repo_pin_after = head;
    }

    // Fenced finalization: acquire registry lock, revalidate ownership and
    // heartbeat health, write evidence, and transition to ValidationComplete
    // atomically — no takeover can occur between the checks and publication.
    fenced_finalize(
        &repo,
        &identity_key,
        &fence,
        &gen_result.id,
        &evidence_dir,
        &bundle,
        &heartbeat_status_rx,
    )
    .context("fenced finalization failed")?;

    // Stop heartbeat and check for errors that occurred during finalization.
    let _ = heartbeat_stop_tx.send(true);
    let _ = heartbeat_handle.await;
    check_heartbeat!();

    Ok(bundle)
}
// ---------------------------------------------------------------------------
// Wait and reuse (exactly-once resume after concurrent or restart)
// ---------------------------------------------------------------------------

/// Wait for another process to complete its reservation, then reuse the result.
///
/// This handles five cases:
/// 1. ValidationComplete → return preserved evidence.
/// 2. ProposalGenerated → take ownership, resume validation.
/// 3. Reserved stale → take ownership, run generation from scratch.
/// 4. Generating stale heartbeat → take ownership, run generation from scratch.
/// 5. Reserved/Generating fresh → wait and retry.
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
) -> Result<EvidenceBundle> {
    // Run validation-specific preflight first.
    run_validation_preflight(repo, commit_at_start, manifest, evidence_dir)?;

    let max_wait = std::time::Duration::from_secs(300); // 5 minutes
    let poll_interval = std::time::Duration::from_millis(500);
    let mut elapsed = std::time::Duration::ZERO;

    loop {
        let entry = lookup_entry(repo, identity_key);
        match entry {
            Some(e) if e.state == ProposalState::ValidationComplete => {
                // The other process finished validation. Return the preserved evidence.
                let proposal_id = e
                    .proposal_id
                    .as_deref()
                    .context("ValidationComplete entry missing proposal_id")?;
                let proposal = load_proposal_from_repo(repo, proposal_id)?;
                let original_evidence_dir_str = e
                    .evidence_dir
                    .as_deref()
                    .unwrap_or(evidence_dir.to_str().unwrap_or(""));
                return return_completed_evidence(
                    repo,
                    commit_at_start,
                    run_id,
                    manifest,
                    config,
                    Path::new(original_evidence_dir_str),
                    governance_scope,
                    &proposal,
                    proposal_id,
                    identity_key,
                )
                .await;
            }
            Some(e) if e.state == ProposalState::ProposalGenerated => {
                // Generation is done but validation hasn't started.
                // Take ownership atomically, then resume validation.
                match try_take_ownership(repo, identity_key, run_id, lease_config)? {
                    TakeoverResult::Taken(fence) => {
                        let proposal_id = e
                            .proposal_id
                            .as_deref()
                            .context("ProposalGenerated entry missing proposal_id")?;
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
                        )
                        .await;
                    }
                    TakeoverResult::StillLive => {
                        // Another worker owns it or is actively renewing. Retry.
                        tokio::time::sleep(poll_interval).await;
                        elapsed += poll_interval;
                        continue;
                    }
                }
            }
            Some(e) if e.state == ProposalState::Validating => {
                // Validation is in progress (or stalled). Check if stale.
                let stale = match is_entry_stale(&e, lease_config) {
                    Ok(s) => s,
                    Err(err) => {
                        bail!("failed to check staleness for Validating entry: {err}");
                    }
                };

                if stale {
                    match try_take_ownership(repo, identity_key, run_id, lease_config)? {
                        TakeoverResult::Taken(fence) => {
                            // Take ownership and resume validation with the existing proposal.
                            let proposal_id = e
                                .proposal_id
                                .as_deref()
                                .context("Validating stale entry missing proposal_id")?;
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
                            )
                            .await;
                        }
                        TakeoverResult::StillLive => {
                            // Another worker reclaimed it first. Continue waiting.
                            tokio::time::sleep(poll_interval).await;
                            elapsed += poll_interval;
                            continue;
                        }
                    }
                }
            }
            Some(e) => {
                // Still in Reserved or Generating state.
                let stale = match is_entry_stale(&e, lease_config) {
                    Ok(s) => s,
                    Err(err) => {
                        bail!(
                            "failed to check staleness for {} entry: {err}",
                            match e.state {
                                ProposalState::Reserved => "Reserved",
                                ProposalState::Generating => "Generating",
                                _ => "unknown",
                            }
                        );
                    }
                };

                if stale {
                    // Attempt to take ownership of the expired entry.
                    match try_take_ownership(repo, identity_key, run_id, lease_config)? {
                        TakeoverResult::Taken(fence) => {
                            // We now own the expired entry. Release and let the caller retry
                            // from scratch (or continue with the new owner).
                            release_reservation(repo, identity_key, &fence)
                                .context("failed to release stale entry after takeover")?;
                            bail!(
                                "stale {} entry reclaimed; caller should retry now owning it",
                                match e.state {
                                    ProposalState::Reserved => "Reserved",
                                    ProposalState::Generating => "Generating",
                                    _ => "entry",
                                }
                            );
                        }
                        TakeoverResult::StillLive => {
                            // Another worker reclaimed it first, or the entry is still live.
                            // Continue waiting.
                            tokio::time::sleep(poll_interval).await;
                            elapsed += poll_interval;
                            continue;
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
                tokio::time::sleep(poll_interval).await;
                elapsed += poll_interval;
            }
            None => {
                // Entry was removed (generation failed and reservation released).
                // Return an error so the caller can retry from scratch.
                bail!(
                    "identity reservation was released by another process \
                     (generation likely failed); caller should retry"
                );
            }
        }
    }
}
/// Return preserved evidence from a completed validation.
///
/// The original bundle is returned UNCHANGED — no field modification, no rewrite.
/// This ensures completed evidence is immutable and returned byte-for-byte.
async fn return_completed_evidence(
    _repo: &Path,
    _commit_at_start: &str,
    _run_id: &str,
    _manifest: &TaskManifest,
    _config: &EvaluationConfig,
    evidence_dir: &Path,
    _governance_scope: &GovernanceScopeSnapshot,
    _proposal: &ProposalArtifact,
    proposal_id: &str,
    _identity_key: &str,
) -> Result<EvidenceBundle> {
    // Load the preserved bundle from the original evidence directory.
    let existing_bundle = find_existing_evidence(evidence_dir, proposal_id).context(
        "ValidationComplete entry references evidence that is missing or corrupt; \
             cannot return preserved bundle",
    )?;

    // Return the original bundle unchanged. No modification, no rewrite.
    Ok(existing_bundle)
}
/// Resume validation from the ProposalGenerated state.
/// The caller must already own the entry (via `FenceToken`).
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
) -> Result<EvidenceBundle> {
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

    // Transition to Validating state to protect the live validation from takeover.
    transition_entry(
        repo,
        identity_key,
        ProposalState::Validating,
        Some(&proposal.id),
        fence,
    )
    .context("failed to transition to Validating state in resume")?;

    // Spawn a heartbeat task to protect validation from stale-reclaim.
    let hb_repo = repo.to_path_buf();
    let hb_key = identity_key.to_string();
    let hb_fence = fence.clone();
    let hb_interval = config.lease_config.heartbeat_interval;
    let (hb_stop_tx, mut hb_stop_rx) = tokio::sync::watch::channel(false);
    let (hb_status_tx, hb_status_rx) = tokio::sync::watch::channel::<Option<String>>(None);
    let hb_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(hb_interval) => {}
                _ = hb_stop_rx.changed() => {
                    if *hb_stop_rx.borrow() {
                        break;
                    }
                }
            }
            if *hb_stop_rx.borrow() {
                break;
            }
            match renew_heartbeat(&hb_repo, &hb_key, &hb_fence) {
                Err(e) => {
                    let _ = hb_status_tx.send(Some(format!("{e:#}")));
                    break;
                }
                Ok(false) => {
                    let _ = hb_status_tx
                        .send(Some("ownership lost during resume validation".to_string()));
                    break;
                }
                Ok(true) => {}
            }
        }
    });

    // Run validation on the existing proposal.
    let validation_result = run_isolated_validation(
        repo,
        &proposal.id,
        manifest.validation_command.as_deref(),
        evidence_dir,
    );

    match &validation_result {
        Ok(vr) => {
            bundle.validation = Some(vr.clone());
            if vr.validation_passed {
                bundle.failure_classification =
                    Some("validation_passed_review_required".to_string());
            } else {
                let class = classify_validation_failure(vr);
                bundle.failure_classification = Some(class);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let classification = classify_dry_run_error(&msg);
            bundle.failure_classification = Some(classification);
        }
    }

    let integrity = verify_repo_integrity(repo, commit_at_start, &proposal.id);
    bundle.integrity = Some(integrity.clone());

    if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        } else {
            bundle.final_state = failure_to_terminal_state(fc).outcome_label().to_string();
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
    }

    let cleanup = cleanup_worktree(repo, &proposal.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    if let Ok(head) = git_rev_parse_head(repo) {
        bundle.repo_pin_after = head;
    }

    // Fenced finalization: acquire registry lock, revalidate ownership and
    // heartbeat health, write evidence, and transition to ValidationComplete
    // atomically — no takeover can occur between the checks and publication.
    fenced_finalize(
        repo,
        identity_key,
        fence,
        &proposal.id,
        evidence_dir,
        &bundle,
        &hb_status_rx,
    )
    .context("fenced finalization failed")?;

    // Stop heartbeat and check for errors.
    let _ = hb_stop_tx.send(true);
    let _ = hb_handle.await;
    if let Some(msg) = hb_status_rx.borrow().as_ref() {
        bail!("heartbeat failure during resume finalization: {msg}");
    }

    Ok(bundle)
}
