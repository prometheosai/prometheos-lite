//! Shared test helpers for the GovernancePermit slice (T5).
//!
//! Each integration-test crate is standalone, so a governed NodeRunner
//! needs a permit, and a permit needs an audit-clean workflow. Building
//! that workflow per test file would scatter the same contract; this
//! module holds it once.

use prometheos_lite::workflow::governance_permit::GovernancePermit;

/// Scope union granted by the shared fixture workflow. The conformance
/// manifests declare subsets of these scopes; the runtime scope binding
/// (`GovernancePermit::ensure_request`) refuses any manifest scope that
/// exceeds the reviewed grant, so widening a manifest past this union
/// must fail closed.
const READABLE_SCOPES: &[&str] = &["repo://evaluation", "repo://x", "repo://fixture"];
const WRITABLE_SCOPES: &[&str] = &["work://evaluation", "work://y", "repo://fixture"];

/// A minimal audit-clean workflow whose body units carry exactly the given
/// operation ids and grant exactly the given capabilities (identically for
/// every unit; each capability must also appear in the workflow ceiling's
/// `tools` map so `reduce_unit_authority` accepts the grant). Every unit
/// consumes the boundary input `goal` and emits the boundary output
/// `result`: with no inter-unit dataflow there is no edge for the cycle
/// detector to traverse, and every output lands on a boundary port so no
/// unit is an orphan. A multi-unit composite is thus well-formed under
/// every audit rule without needing a dataflow chain.
pub fn workflow_text(ids: &[&str], capabilities: &[&str]) -> String {
    let mut authority: Vec<String> = READABLE_SCOPES
        .iter()
        .map(|scope| format!("\"readable:{scope}\""))
        .chain(
            WRITABLE_SCOPES
                .iter()
                .map(|scope| format!("\"writable:{scope}\"")),
        )
        .collect();
    authority.extend(capabilities.iter().map(|cap| format!("{cap:?}")));
    let authority_json = authority.join(", ");
    let tools_json = capabilities
        .iter()
        .map(|cap| format!("{cap:?}: []"))
        .collect::<Vec<_>>()
        .join(", ");
    let units: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{
                  "schemaVersion": "1.1.0", "version": "1.1.0", "id": "{id}",
                  "executionClass": "deterministic",
                  "inputs": [{{"name": "goal", "type": "string", "acceptedOutcomes": ["Produced"]}}],
                  "outputs": [{{"name": "result", "type": "string", "emits": ["Produced"]}}],
                  "authority": [{authority_json}],
                  "effects": [], "uses": [], "secrets": [], "context": []
                }}"#,
                id = id,
                authority_json = authority_json
            )
        })
        .collect();
    format!(
        r#"{{
  "schemaVersion": "1.1.0", "version": "1.1.0", "id": "lite-governed",
  "name": "Governed", "kind": "atomic",
  "inputPorts": [{{"name": "goal", "direction": "input", "type": "string",
                   "cardinality": "single", "requiredness": "required"}}],
  "outputPorts": [{{"name": "result", "direction": "output", "type": "string",
                    "cardinality": "single", "requiredness": "required"}}],
  "body": [{}],
  "authority": {{
    "executionClass": "deterministic", "mutation": "none", "tools": {{{tools_json}}},
    "readableScopes": ["repo://evaluation", "repo://x", "repo://fixture"],
    "writableScopes": ["work://evaluation", "work://y", "repo://fixture"],
    "networkPolicy": {{"default": "deny"}}, "providerPolicy": {{"allowlist": []}},
    "secrets": []
  }}
}}"#,
        units.join(","),
        tools_json = tools_json
    )
}

/// Issue a permit for a workflow whose body units carry exactly `ids` and
/// grant exactly `capabilities`, bound to its own computed reviewed identity.
pub fn permit_for(ids: &[&str], capabilities: &[&str]) -> GovernancePermit {
    let text = workflow_text(ids, capabilities);
    let identity = prometheos_lite::workflow::governance_compiler::compile_workflow_text(&text)
        .expect("audit-clean workflow compiles")
        .canonicalization
        .sha256;
    GovernancePermit::issue(&text, &identity).expect("permit issues for the reviewed identity")
}
