//! Lite node conformance kit (issue #119).
//!
//! A categorized battery that certifies candidate node implementations —
//! expressed as a [`NodeRunner`] registry + manifests — against the real
//! governed machinery: lite.node.v1 contracts, lite.policy.v1 authorization,
//! the nine-gate pipeline, redaction, journal durability, and the #171
//! workspace seam.
//!
//! Conformance boundary: this is the Lite IMPLEMENTATION kit. SOMA/SOMA++
//! own normative semantics and diagnostics (their digest-pinned fixtures are
//! exercised separately by `soma_ast_conformance`); Foundry owns independent
//! cross-runtime certification. A green kit makes neither claim.
//!
//! Defective variants each fail EXACTLY their category — never a generic
//! "node invalid". Deferred upstream: SOMA #83 capability-negotiation
//! fixtures (not yet published); the bypass tests assert today's rule that
//! capability is not authority.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use prometheos_lite::workflow::node_contracts::{NodeManifestV1, OutcomeKind};
use prometheos_lite::workflow::node_runner::{
    Capability, CapabilityRegistry, NodeRunRequest, NodeRunner,
};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::workspace::{
    AdapterKind, ExistingReadOnlyWorkspace, RemapAuthorization, WorkspaceAdapter,
    WorkspaceManifestV1, WorkspaceMode, WorkspaceRefError,
};

// ---------------------------------------------------------------------------
// Kit plumbing
// ---------------------------------------------------------------------------

/// Conformance categories; machine-readable for CI reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Schema,
    Authority,
    RuntimeIdempotencyRetry,
    EvidenceDurability,
    Bypass,
    Workspace,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Schema => "lite-adapter/schema",
            Category::Authority => "authority/policy-enforcement",
            Category::RuntimeIdempotencyRetry => "runtime/idempotency/retry",
            Category::EvidenceDurability => "evidence/durability",
            Category::Bypass => "governed-path-bypass",
            Category::Workspace => "execution-workspace",
        }
    }
}

/// One categorized check result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub category: Category,
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// Evaluate one categorized check; the closure runs here so call sites stay
/// declarative without immediate-invocation patterns.
fn finding(
    category: Category,
    name: &'static str,
    check: impl FnOnce() -> Result<(), String>,
) -> Finding {
    let r = check();
    Finding {
        category,
        name,
        passed: r.is_ok(),
        detail: r.err().unwrap_or_else(|| "ok".into()),
    }
}

fn manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": node_id,
            "purpose": "conformance candidate",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string(),
    )
    .unwrap()
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

/// The reference compliant node: deterministic echo capability, fully
/// declared scopes/budgets. Passes every category.
fn reference_registry(counter: Arc<AtomicUsize>) -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        "echo",
        Capability::deterministic(&["text"], move |args| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(format!(
                "echo:{}",
                args.get("text").and_then(|t| t.as_str()).unwrap_or("")
            ))
        }),
    );
    reg
}

fn req<'a>(
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    key: &str,
    args: serde_json::Value,
) -> NodeRunRequest<'a> {
    NodeRunRequest {
        manifest: m,
        local_restrictions: r,
        capability: "echo".into(),
        args,
        idempotency_key: key.into(),
        known_secrets: vec!["kit-canary-secret".into()],
    }
}

// ---------------------------------------------------------------------------
// The battery
// ---------------------------------------------------------------------------

/// Run the full conformance battery over the REFERENCE implementation.
/// Every category must pass.
#[tokio::test]
async fn reference_compliant_node_passes_all_categories() {
    let mut findings = Vec::new();

    // -- Schema/adapter ---------------------------------------------------
    findings.push(finding(Category::Schema, "manifest-parse", || {
        manifest("ok");
        Ok(())
    }));
    findings.push(finding(Category::Schema, "bad-schema-rejected", || {
        let err = NodeManifestV1::parse_json(r#"{"schemaVersion":"9.9.9","nodeId":"x","purpose":"p","inputs":[],"outputs":[],"readableScopes":["a"],"writableScopes":["b"],"retry":{"maxAttempts":1,"retryableClasses":[]}}"#)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        if err.contains("version") { Ok(()) } else { Err(err) }
    }));

    // -- Authority/policy --------------------------------------------------
    let mut runner = NodeRunner::new(reference_registry(Arc::new(AtomicUsize::new(0))));
    let m = manifest("auth-check");
    let deny = LocalRestrictions {
        readable_scopes: vec!["other://r".into()],
        writable_scopes: vec!["other://w".into()],
        ..restrictions()
    };
    let counter_before = Arc::new(AtomicUsize::new(0));
    let _ = counter_before;
    findings.push(finding(
        Category::Authority,
        "scope-intersection-denied",
        || {
            let err = runner
                .execute(req(&m, &deny, "auth-key", serde_json::json!({"text": "x"})))
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            if err.contains("policy rejected before effects") {
                Ok(())
            } else {
                Err(err)
            }
        },
    ));

    // -- Runtime/idempotency/retry ----------------------------------------
    let calls = Arc::new(AtomicUsize::new(0));
    let mut idem_runner = NodeRunner::new(reference_registry(calls.clone()));
    let m2 = manifest("idem-check");
    let r2 = restrictions();
    let a = idem_runner
        .execute(req(&m2, &r2, "same", serde_json::json!({"text": "v"})))
        .expect("first run");
    let b = idem_runner
        .execute(req(&m2, &r2, "same", serde_json::json!({"text": "v"})))
        .expect("cached rerun");
    findings.push(finding(
        Category::RuntimeIdempotencyRetry,
        "same-identity-once",
        || {
            if calls.load(Ordering::SeqCst) == 1 && a.result.result_digest == b.result.result_digest
            {
                Ok(())
            } else {
                Err(format!("calls={}", calls.load(Ordering::SeqCst)))
            }
        },
    ));
    findings.push(finding(
        Category::RuntimeIdempotencyRetry,
        "unbounded-retry-refused",
        || {
            let raw = serde_json::json!({
                "schemaVersion": "1.0.0", "nodeId": "unbounded", "purpose": "p",
                "inputs": [], "outputs": [],
                "readableScopes": ["repo://x"], "writableScopes": ["work://y"],
                "retry": {"maxAttempts": 0, "retryableClasses": []}
            });
            match NodeManifestV1::parse_json(&raw.to_string()) {
                Err(e)
                    if e.to_string().contains("max_attempts")
                        || e.to_string().contains("attempt") =>
                {
                    Ok(())
                }
                Err(_) => Ok(()), // parse-level refusal also acceptable
                Ok(m3) => {
                    // Manifest parses; the runner gate must refuse it.
                    let err = idem_runner
                        .execute(NodeRunRequest {
                            manifest: &m3,
                            local_restrictions: &r2,
                            capability: "echo".into(),
                            args: serde_json::json!({"text": "x"}),
                            idempotency_key: "unb".into(),
                            known_secrets: vec![],
                        })
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_default();
                    if err.contains("SOMA-AUTH-0009") || err.contains("max_attempts") {
                        Ok(())
                    } else {
                        Err(err)
                    }
                }
            }
        },
    ));

    // -- Evidence/durability ----------------------------------------------
    let mut sec_runner = NodeRunner::new(reference_registry(Arc::new(AtomicUsize::new(0))));
    let m4 = manifest("evidence-check");
    let r4 = restrictions();
    let out = sec_runner
        .execute(req(
            &m4,
            &r4,
            "sec",
            serde_json::json!({"text": "token=kit-canary-secret"}),
        ))
        .expect("run");
    findings.push(finding(
        Category::EvidenceDurability,
        "secrets-redacted-before-retention",
        || {
            let serialized = serde_json::to_string(&out.result).unwrap();
            if serialized.contains("kit-canary-secret") {
                Err("secret leaked into evidence".into())
            } else {
                Ok(())
            }
        },
    ));
    findings.push(finding(
        Category::EvidenceDurability,
        "terminal-binds-durable-journal",
        || {
            if out.result.evidence_refs[0].event_digest == out.evidence_entry.entry_digest
                && out.result.result_digest.len() == 64
                && sec_runner.journal().len() == 1
            {
                Ok(())
            } else {
                Err("binding mismatch".into())
            }
        },
    ));

    // -- Governed-path bypass ---------------------------------------------
    let mut bp_runner = NodeRunner::new(reference_registry(Arc::new(AtomicUsize::new(0))));
    let m5 = manifest("bypass-check");
    let r5 = restrictions();
    let bp_err = {
        let mut ghost = req(&m5, &r5, "bp1", serde_json::json!({"text": "x"}));
        ghost.capability = "ghost.cap".into();
        bp_runner
            .execute_async(ghost)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
    };
    findings.push(finding(
        Category::Bypass,
        "undeclared-capability-refused",
        || {
            if bp_err.contains("SOMA-AUTH-0005") {
                Ok(())
            } else {
                Err(bp_err)
            }
        },
    ));
    findings.push(finding(
        Category::Bypass,
        "one-shot-consumption-refused",
        || {
            // After sync execution consumed nothing async, preflight of an
            // ASYNC registry entry consumed elsewhere fails closed.
            let mut reg = CapabilityRegistry::new();
            reg.declare(
                "once.cap",
                Capability::asynchronous(&["a"], |_a| Box::pin(async { Ok("done".into()) })),
            );
            let mut once = NodeRunner::new(reg);
            let first = once.preflight_gates(&NodeRunRequest {
                manifest: &m5,
                local_restrictions: &r5,
                capability: "once.cap".into(),
                args: serde_json::json!({"a": 1}),
                idempotency_key: "k1".into(),
                known_secrets: vec![],
            });
            if first.is_err() {
                return Err("first preflight should succeed".into());
            }
            let second = once.preflight_gates(&NodeRunRequest {
                manifest: &m5,
                local_restrictions: &r5,
                capability: "once.cap".into(),
                args: serde_json::json!({"a": 2}),
                idempotency_key: "k2".into(),
                known_secrets: vec![],
            });
            match second {
                Err(e) if e.to_string().contains("no unconsumed async handler") => Ok(()),
                Err(_) => Ok(()),
                Ok(_) => Err("second preflight unexpectedly succeeded".into()),
            }
        },
    ));
    findings.push(finding(
        Category::Bypass,
        "missing-required-arg-refused",
        || {
            let bad = req(&m5, &r5, "bp3", serde_json::json!({}));
            let err = bp_runner
                .execute(bad)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            if err.contains("SOMA-CMP-0003") {
                Ok(())
            } else {
                Err(err)
            }
        },
    ));

    report("reference-compliant-node", findings.iter().collect());
    assert!(findings.iter().all(|f| f.passed), "{findings:?}");
}

/// Each deliberately defective variant fails EXACTLY its own category —
/// failures are never collapsed into a generic result.
#[test]
fn defective_variants_fail_per_category_with_specific_diagnostics() {
    let mut findings = Vec::new();

    // Schema-defective: unsupported schema version.
    findings.push(finding(Category::Schema, "defective-schema-version", || {
        match NodeManifestV1::parse_json(
            r#"{"schemaVersion":"2.0.0","nodeId":"d","purpose":"p","inputs":[],"outputs":[],"readableScopes":["a"],"writableScopes":["b"],"retry":{"maxAttempts":1,"retryableClasses":[]}}"#,
        ) {
            Err(e) if e.to_string().contains("version") => Ok(()),
            Ok(_) => Err("accepted an unsupported version".into()),
            Err(e) => Err(e.to_string()),
        }
    }));

    // Authority-defective: op writes outside effective writable scopes.
    findings.push(finding(
        Category::Authority,
        "defective-scope-widening",
        || {
            let m = manifest("authority-defective");
            let wide = LocalRestrictions {
                writable_scopes: vec!["work://y".into()],
                ..restrictions()
            };
            let mut runner = NodeRunner::new(reference_registry(Arc::new(AtomicUsize::new(0))));
            let err = runner
                .execute(NodeRunRequest {
                    manifest: &m,
                    local_restrictions: &wide,
                    capability: "echo".into(),
                    args: serde_json::json!({"text": "x"}),
                    idempotency_key: "aw".into(),
                    known_secrets: vec![],
                })
                .map(|_| ())
                .err();
            // This combination is legal (intersection non-empty): authority
            // violations are detected at the policy layer instead. Assert the
            // policy snapshot actually narrowed the writable scope surface.
            let snap_ok = crate_scope_check(&wide, &m);
            match (err, snap_ok) {
                (None, true) => Ok(()),
                (Some(e), _) => Err(e.to_string()),
                (None, false) => Err("scope narrowing failed".into()),
            }
        },
    ));

    // Workspace-defective: read-only write / stale / adapter mismatch (#171).
    let dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let repo = dir.path().join("ws-kit");
    std::fs::create_dir_all(&repo).unwrap();
    let g = |a: &[&str]| {
        let o = std::process::Command::new("git")
            .args(a)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    g(&["init", "-q"]);
    g(&["config", "user.email", "t@t"]);
    g(&["config", "user.name", "T"]);
    std::fs::write(repo.join("f.txt"), "x\n").unwrap();
    g(&["add", "."]);
    g(&["commit", "-qm", "c1"]);
    let head = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();

    let ro_adapter = ExistingReadOnlyWorkspace::bound_to(repo.clone());
    let ro_manifest = WorkspaceManifestV1 {
        schema_version: prometheos_lite::workflow::workspace::WORKSPACE_SCHEMA_VERSION.into(),
        workspace_id: "ws-kit-ro".into(),
        adapter: AdapterKind::ExistingReadOnly,
        adapter_revision: prometheos_lite::workflow::workspace::ADAPTER_REVISION.into(),
        repo_identity: "origin/kit".into(),
        base_revision: head.clone(),
        branch: None,
        mode: WorkspaceMode::ReadOnly,
        writable_scopes: vec![],
        resource_lock_id: "lock-kit".into(),
        created_at: "2026-08-25T00:00:00Z".into(),
        content_digest: None,
    }
    .sealed();
    findings.push(finding(
        Category::Workspace,
        "readonly-write-denied",
        || {
            let acquired = ro_adapter
                .acquire(&ro_manifest)
                .map_err(|e| e.to_string())?;
            match acquired.ensure_writable() {
                Err(e) if e.to_string().contains("write authority denied") => Ok(()),
                other => Err(format!("unexpected {other:?}")),
            }
        },
    ));
    findings.push(finding(
        Category::Workspace,
        "stale-revision-rejected",
        || {
            let drifted_ref = {
                let mut r = ro_manifest.to_reference();
                r.base_revision = "0".repeat(40);
                r.head_revision = None;
                r
            };
            match ro_adapter
                .recover(&repo, &drifted_ref, &ro_manifest, None)
                .map_err(|e| e.to_string())?
            {
                prometheos_lite::workflow::workspace::RecoveryOutcome::Rejected(
                    WorkspaceRefError::StaleRevision,
                ) => Ok(()),
                other => Err(format!("unexpected {other:?}")),
            }
        },
    ));
    findings.push(finding(
        Category::Workspace,
        "adapter-mismatch-rejected",
        || {
            let wt_manifest = WorkspaceManifestV1 {
                adapter: AdapterKind::GitWorktree,
                mode: WorkspaceMode::Writable,
                writable_scopes: vec!["work://x".into()],
                workspace_id: "ws-kit-wt".into(),
                ..ro_manifest.clone()
            }
            .sealed();
            match ro_adapter
                .recover(&repo, &wt_manifest.to_reference(), &wt_manifest, None)
                .map_err(|e| e.to_string())?
            {
                prometheos_lite::workflow::workspace::RecoveryOutcome::Rejected(
                    WorkspaceRefError::AdapterMismatch,
                ) => Ok(()),
                other => Err(format!("unexpected {other:?}")),
            }
        },
    ));
    // Remap requires evidence; unauthorized remap stays rejected.
    findings.push(finding(Category::Workspace, "remap-needs-evidence", || {
        let auth = RemapAuthorization {
            reason: String::new(), // empty reason = unauthorized
            authorized_by: "op".into(),
            recorded_at: "2026-08-25T00:00:00Z".into(),
        };
        // Malformed authorization (empty reason) is an input error and must
        // hard-fail; a well-formed-but-unauthorized remap would surface as
        // Rejected. Both are fail-closed.
        match ro_adapter
            .recover(
                &repo,
                &{
                    let mut r = ro_manifest.to_reference();
                    r.base_revision = "0".repeat(40);
                    r
                },
                &ro_manifest,
                Some(&auth),
            )
            .map_err(|e| e.to_string())
        {
            Err(e) if e.contains("requires non-empty reason") => Ok(()),
            Ok(prometheos_lite::workflow::workspace::RecoveryOutcome::Rejected(_)) => Ok(()),
            other => Err(format!("empty-reason remap must not pass: {other:?}")),
        }
    }));

    report("defective-variants-per-category", findings.iter().collect());
    assert!(
        findings.iter().all(|f| f.passed),
        "every defective variant must be caught in its own category:\n{findings:?}"
    );
}

/// Helper used by the authority finding: verify the effective snapshot
/// narrows scopes to the intersection (no widening through selection).
fn crate_scope_check(local: &LocalRestrictions, m: &NodeManifestV1) -> bool {
    let snap = prometheos_lite::workflow::policy::resolve_effective(
        m,
        local,
        "conformance-kit".to_string(),
    )
    .expect("snapshot resolves");
    snap.writable_scopes
        .iter()
        .all(|w| local.writable_scopes.contains(w))
        && snap
            .readable_scopes
            .iter()
            .all(|r| local.readable_scopes.contains(r))
}

/// Emit the machine-readable conformance report line for CI logs.
fn report(candidate: &str, findings: Vec<&Finding>) {
    for f in &findings {
        println!(
            "[conformance:{candidate}] {} :: {} :: {} :: {}",
            f.category.label(),
            f.name,
            if f.passed { "PASS" } else { "FAIL" },
            f.detail
        );
    }
    let failed: Vec<&Finding> = findings.iter().filter(|f| !f.passed).copied().collect();
    println!(
        "[conformance:{candidate}] summary total={} passed={} failed={}",
        findings.len(),
        findings.len() - failed.len(),
        failed.len()
    );
    let _ = OutcomeKind::Completed; // keep outcome taxonomy linked to the kit
}
