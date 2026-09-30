//! Conformance for the deterministic projection subsystem (E4/X07 Slice 1).

use prometheos_lite::workflow::projection::{PROJECTION_VERSION_V1, VersionedProjectionEnvelope};

#[test]
fn envelope_exposes_projection_v1_and_nests_payload() {
    assert_eq!(PROJECTION_VERSION_V1, "projection.v1");
    let env = VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: "1.1.0".to_string(),
        source_digest: "a".repeat(64),
        projection_digest: "b".repeat(64),
        payload: serde_json::json!({"id": "wf-base"}),
    };
    let value = serde_json::to_value(&env).expect("envelope serializes");
    let obj = value.as_object().expect("envelope is a JSON object");
    for key in [
        "projectionVersion",
        "schemaVersion",
        "sourceDigest",
        "projectionDigest",
        "payload",
    ] {
        assert!(obj.contains_key(key), "missing envelope key {key}");
    }
    assert_eq!(
        obj.len(),
        5,
        "payload must be one nested key, not flattened"
    );
    assert_eq!(
        obj["payload"]["id"],
        serde_json::json!("wf-base"),
        "payload must nest its value under `payload`"
    );
}

#[test]
fn envelope_rejects_unknown_fields() {
    let mut value = serde_json::json!({
        "projectionVersion": "projection.v1",
        "schemaVersion": "1.1.0",
        "sourceDigest": "a".repeat(64),
        "projectionDigest": "b".repeat(64),
        "payload": null
    });
    value["extra"] = serde_json::json!(1);
    let parsed = serde_json::from_value::<VersionedProjectionEnvelope<serde_json::Value>>(value);
    assert!(parsed.is_err(), "unknown envelope field must fail closed");
}
