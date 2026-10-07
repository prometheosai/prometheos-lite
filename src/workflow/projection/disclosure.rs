//! Graph disclosure policy: validation, normalization, and semantics
//! (E4/X07 Slice 2 spec §6). Raw policies are never embedded in any
//! payload — only their normalized `policyDigest` reaches bytes.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::workflow::projection::graph::GraphRegistry;
use crate::workflow::soma::Diagnostic;

/// Caller-supplied, already-adjudicated policy for one
/// `project_graph_json` call (§6). The projector performs no principal
/// lookup; `None` and `Some(default)` are semantically identical.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GraphDisclosurePolicy {
    /// Rendered node ids (`s{NNNN}:{id}`) of composite boundaries whose
    /// direct contents (`children` + `internalEdges`) render.
    /// Non-cascading: each nested boundary needs its own entry.
    #[serde(default)]
    pub authorized_boundaries: Vec<String>,
    /// Rendered node ids of withheld boundaries that may emit integer
    /// `hiddenNodes`/`hiddenEdges`. Counts only — never content.
    #[serde(default)]
    pub count_authorization: Vec<String>,
}

/// Validate a raw policy against the registry and return its normalized
/// form: both arrays sorted lexicographically (§6 normalization — input
/// order never changes output bytes). Every refusal is `PROJ-0003`
/// (§6 rules 1-5), with deterministic diagnostics: duplicates per
/// array (sorted), then unknown/non-composite ids (sorted), then
/// ancestry (per authorized id, sorted, ancestors outermost first).
pub fn normalize_policy(
    registry: &GraphRegistry<'_>,
    policy: Option<&GraphDisclosurePolicy>,
) -> Result<GraphDisclosurePolicy, Vec<Diagnostic>> {
    let raw = policy.cloned().unwrap_or_default();
    let mut out: Vec<Diagnostic> = Vec::new();

    // Rule 3: duplicates within either array. Cross-array presence of
    // the same id stays legal (rule 5 note / §6 semantics).
    for (array, label) in [
        (&raw.authorized_boundaries, "authorizedBoundaries"),
        (&raw.count_authorization, "countAuthorization"),
    ] {
        let mut seen = BTreeSet::new();
        let mut dups = BTreeSet::new();
        for id in array {
            if !seen.insert(id.clone()) {
                dups.insert(id.clone());
            }
        }
        for id in dups {
            out.push(Diagnostic::new(
                "PROJ-0003",
                format!("duplicate id in {label}: {id}"),
            ));
        }
    }

    // Rules 1, 2, 5: every referenced id must exist and be composite.
    let unique: BTreeSet<&String> = raw
        .authorized_boundaries
        .iter()
        .chain(raw.count_authorization.iter())
        .collect();
    for id in unique {
        match registry.entry(id) {
            None => out.push(Diagnostic::new(
                "PROJ-0003",
                format!("unknown node id in disclosure policy: {id}"),
            )),
            Some(entry) if !entry.is_composite() => out.push(Diagnostic::new(
                "PROJ-0003",
                format!("disclosure policy targets a non-composite node: {id}"),
            )),
            Some(_) => {}
        }
    }

    // Rule 4: ancestry traversability — authorizing a boundary requires
    // every ancestor boundary to be authorized too (no hidden
    // descendant can surface through unauthorized parents).
    let authorized: BTreeSet<&String> = raw.authorized_boundaries.iter().collect();
    let mut authorized_sorted: Vec<&String> = raw.authorized_boundaries.iter().collect();
    authorized_sorted.sort();
    authorized_sorted.dedup();
    for id in authorized_sorted {
        let Some(entry) = registry.entry(id) else {
            continue; // rule 1 already reported
        };
        for ancestor in &entry.ancestors {
            if !authorized.contains(ancestor) {
                out.push(Diagnostic::new(
                    "PROJ-0003",
                    format!("boundary {id} is authorized without its ancestor {ancestor}"),
                ));
            }
        }
    }

    if !out.is_empty() {
        out.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.message.cmp(&b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        return Err(out);
    }

    let mut authorized_boundaries = raw.authorized_boundaries;
    let mut count_authorization = raw.count_authorization;
    authorized_boundaries.sort();
    count_authorization.sort();
    Ok(GraphDisclosurePolicy {
        authorized_boundaries,
        count_authorization,
    })
}

/// §8.2 preimage: the normalized (sorted, non-raw) policy bound to the
/// root source digest. `None` and empty normalize identically before
/// this, so they always produce the same `policyDigest`.
pub fn policy_digest_preimage(
    root_source_digest: &str,
    policy: &GraphDisclosurePolicy,
) -> serde_json::Value {
    serde_json::json!({
        "domain": "projection.graph.policy.v1",
        "graphSchemaVersion": crate::workflow::projection::graph::GRAPH_SCHEMA_VERSION,
        "rootSourceDigest": root_source_digest,
        "policy": {
            "authorizedBoundaries": policy.authorized_boundaries.clone(),
            "countAuthorization": policy.count_authorization.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::projection::digest_of;
    use crate::workflow::soma::contracts::WorkflowDefinition;

    const NESTED: &str = include_str!("../../../tests/fixtures/slice2/wf-nested.json");

    fn wf() -> WorkflowDefinition {
        serde_json::from_str(NESTED).expect("wf-nested parses")
    }

    fn policy(authorized: &[&str], counts: &[&str]) -> GraphDisclosurePolicy {
        GraphDisclosurePolicy {
            authorized_boundaries: authorized.iter().map(|s| s.to_string()).collect(),
            count_authorization: counts.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn none_and_empty_policy_normalize_identically() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("registry");
        let none = normalize_policy(&registry, None).expect("default policy");
        let empty =
            normalize_policy(&registry, Some(&GraphDisclosurePolicy::default())).expect("empty");
        assert_eq!(none, empty);
        assert!(none.authorized_boundaries.is_empty());
        assert!(none.count_authorization.is_empty());
    }

    #[test]
    fn policy_sorts_arrays_and_allows_cross_array_ids() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("registry");
        let raw = policy(&["s0001:flow", "s0000:inner"], &["s0001:flow"]);
        let norm = normalize_policy(&registry, Some(&raw)).expect("ancestry satisfied");
        assert_eq!(norm.authorized_boundaries, ["s0000:inner", "s0001:flow"]);
        assert_eq!(norm.count_authorization, ["s0001:flow"]);
    }

    #[test]
    fn policy_rejects_unknown_noncomposite_duplicates_and_missing_ancestry() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("registry");
        let refuse = |p: GraphDisclosurePolicy| {
            normalize_policy(&registry, Some(&p)).expect_err("policy must be refused")
        };

        let err = refuse(policy(&["s9999:ghost"], &[]));
        assert!(
            err.iter()
                .any(|d| d.code == "PROJ-0003" && d.message.contains("unknown node id")),
            "{err:?}"
        );

        let err = refuse(policy(&["s0000:prep"], &[]));
        assert!(
            err.iter()
                .any(|d| d.code == "PROJ-0003" && d.message.contains("non-composite")),
            "{err:?}"
        );

        let err = refuse(policy(&["s0001:flow", "s0001:flow"], &[]));
        assert!(
            err.iter().any(|d| d.code == "PROJ-0003"
                && d.message.contains("duplicate id in authorizedBoundaries")),
            "{err:?}"
        );

        let err = refuse(policy(&["s0000:inner"], &[]));
        assert!(
            err.iter()
                .any(|d| d.code == "PROJ-0003" && d.message.contains("without its ancestor")),
            "{err:?}"
        );

        let err = refuse(policy(&[], &["s0000:prep"]));
        assert!(
            err.iter()
                .any(|d| d.code == "PROJ-0003" && d.message.contains("non-composite")),
            "{err:?}"
        );
    }

    #[test]
    fn policy_digest_depends_only_on_sorted_content() {
        let wf = wf();
        let registry = GraphRegistry::build(&wf).expect("registry");
        let a = normalize_policy(
            &registry,
            Some(&policy(&["s0001:flow", "s0000:inner"], &[])),
        )
        .expect("valid");
        let b = normalize_policy(
            &registry,
            Some(&policy(&["s0000:inner", "s0001:flow"], &[])),
        )
        .expect("valid");
        let source = "f".repeat(64);
        let da = digest_of(&policy_digest_preimage(&source, &a)).expect("digest");
        let db = digest_of(&policy_digest_preimage(&source, &b)).expect("digest");
        assert_eq!(da, db, "input order must not change policyDigest");

        let c = normalize_policy(&registry, Some(&policy(&["s0001:flow"], &[]))).expect("valid");
        let dc = digest_of(&policy_digest_preimage(&source, &c)).expect("digest");
        assert_ne!(da, dc, "policy content must change policyDigest");
    }
}
