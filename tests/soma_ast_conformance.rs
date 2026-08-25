//! SOMA++ conformance harness (issue #159): drives every digest-pinned
//! fixture from the vendored v1.1 bundle through the Lite validator and
//! asserts the published expected diagnostic codes.
//!
//! Ground truth: `vendored/soma/v1.1/fixtures/manifest.json` — the normative
//! fixture set published by `prometheosai/soma`. Valid fixtures must produce
//! zero diagnostics; invalid fixtures must produce every pinned code.
//! Parse-level refusals map to SOMA-CMP-0003; duplicate keys to
//! SOMA-CMP-0007.

use prometheos_lite::workflow::soma::validate_artifact_text;

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

#[derive(serde::Deserialize)]
struct FixturePin {
    path: String,
    kind: String,
    #[serde(rename = "expected_codes")]
    expected_codes: Vec<String>,
    #[serde(rename = "artifact")]
    artifact_kind: String,
    sha256: String,
}

#[derive(serde::Deserialize)]
struct FixtureManifest {
    fixtures: Vec<FixturePin>,
}

fn fixture_manifest() -> FixtureManifest {
    let raw = std::fs::read_to_string(format!("{VENDORED}/fixtures/manifest.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// Map a validator outcome to the diagnostic codes a fixture expects,
/// including the parse-refusal mappings (CMP-0007 / CMP-0003).
fn outcome_codes(
    result: Result<Vec<prometheos_lite::workflow::soma::Diagnostic>, String>,
) -> Vec<String> {
    match result {
        Ok(diags) => diags.iter().map(|d| d.code.clone()).collect(),
        Err(msg) => {
            if msg.contains("duplicate object key") {
                vec!["SOMA-CMP-0007".into()]
            } else {
                vec!["SOMA-CMP-0003".into()]
            }
        }
    }
}

#[test]
fn all_published_fixtures_conform() {
    let manifest = fixture_manifest();
    assert!(
        manifest.fixtures.len() >= 60,
        "fixture corpus unexpectedly small: {}",
        manifest.fixtures.len()
    );
    let mut checked_valid = 0usize;
    let mut checked_invalid = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for fx in &manifest.fixtures {
        let path = format!("{VENDORED}/{}", fx.path);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{}: unreadable: {e}", fx.path));
                continue;
            }
        };
        let result = validate_artifact_text(&fx.artifact_kind, &text);
        let codes = outcome_codes(result);
        if fx.kind == "valid" {
            checked_valid += 1;
            let unexpected: Vec<&String> = codes
                .iter()
                .filter(|c| !fx.expected_codes.contains(c))
                .collect();
            if !unexpected.is_empty() {
                failures.push(format!(
                    "{}: valid fixture produced diagnostics {codes:?}",
                    fx.path
                ));
            }
        } else {
            checked_invalid += 1;
            for expected in &fx.expected_codes {
                if !codes.contains(expected) {
                    failures.push(format!(
                        "{}: missing expected code {expected}; produced {codes:?}",
                        fx.path
                    ));
                }
            }
        }
    }

    assert_eq!(
        failures,
        Vec::<String>::new(),
        "conformance failures (valid={checked_valid}, invalid={checked_invalid}):\n{}",
        failures.join("\n")
    );
    assert!(checked_valid >= 10 && checked_invalid >= 40);
}

#[test]
fn canonical_digests_reproduce_fixture_pins() {
    // The fixture manifest pins sha256 over the CANONICAL bytes of each
    // fixture; Lite's canonicalizer must reproduce them byte-for-byte.
    // The sanctioned all-zeros placeholder pin (wf-cmp-0007) is skipped:
    // that artifact intentionally carries a duplicate key and cannot have
    // trustworthy canonical bytes.
    use prometheos_lite::workflow::soma::canonical_digest;
    let manifest = fixture_manifest();
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for fx in &manifest.fixtures {
        if fx.sha256 == "0".repeat(64) {
            continue;
        }
        let path = format!("{VENDORED}/{}", fx.path);
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let computed = canonical_digest(&value);
        checked += 1;
        if computed != fx.sha256 {
            failures.push(format!("{}: got {computed} want {}", fx.path, fx.sha256));
        }
    }
    assert_eq!(
        failures,
        Vec::<String>::new(),
        "canonical digest mismatches over {checked} fixtures:\n{}",
        failures.join("\n")
    );
}
