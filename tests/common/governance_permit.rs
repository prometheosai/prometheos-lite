//! Shared test helpers for the GovernancePermit slice (T5).
//!
//! Each integration-test crate is standalone, so a governed NodeRunner
//! needs a permit, and a permit needs an audit-clean workflow. Building
//! that workflow per test file would scatter the same contract; this
//! module holds it once.

use prometheos_lite::workflow::governance_permit::GovernancePermit;

/// A minimal audit-clean workflow whose body units carry exactly the given
/// operation ids. Every unit consumes the boundary input `goal` and emits
/// the boundary output `result`: with no inter-unit dataflow there is no
/// edge for the cycle detector to traverse, and every output lands on a
/// boundary port so no unit is an orphan. A multi-unit composite is thus
/// well-formed under every audit rule without needing a dataflow chain.
pub fn workflow_text(ids: &[&str]) -> String {
    let units: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{
                  "schemaVersion": "1.1.0", "version": "1.1.0", "id": "{id}",
                  "executionClass": "deterministic",
                  "inputs": [{{"name": "goal", "type": "string", "acceptedOutcomes": ["Produced"]}}],
                  "outputs": [{{"name": "result", "type": "string", "emits": ["Produced"]}}],
                  "authority": ["readable:repo://evaluation", "writable:work://evaluation"],
                  "effects": [], "uses": [], "secrets": [], "context": []
                }}"#,
                id = id
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
    "executionClass": "deterministic", "mutation": "none", "tools": {{}},
    "readableScopes": ["repo://evaluation"], "writableScopes": ["work://evaluation"],
    "networkPolicy": {{"default": "deny"}}, "providerPolicy": {{"allowlist": []}},
    "secrets": []
  }}
}}"#,
        units.join(",")
    )
}

/// Issue a permit for a workflow whose body units carry exactly `ids`,
/// binding it to its own computed reviewed identity.
pub fn permit_for(ids: &[&str]) -> GovernancePermit {
    let text = workflow_text(ids);
    let identity = prometheos_lite::workflow::governance_compiler::compile_workflow_text(&text)
        .expect("audit-clean workflow compiles")
        .canonicalization
        .sha256;
    GovernancePermit::issue(&text, &identity).expect("permit issues for the reviewed identity")
}