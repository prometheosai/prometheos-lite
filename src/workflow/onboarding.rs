//! E6/I05 (#134) — repository onboarding and actionable diagnostics.
//!
//! Onboarding is a read-only survey followed by a reviewable proposal:
//! detect repository languages, package systems, test commands, protected
//! paths, dependency manifests, Git state, and a likely validation
//! strategy — every detection carrying explicit confidence and the
//! evidence it was derived from. The proposal is a suggestion document
//! that enables no authority: no scopes, capabilities, or tools appear
//! anywhere in it, and nothing consumes it as a grant. Unsupported or
//! conflicting setups stop with typed, actionable diagnostics
//! (`SOMA-ONB-*`, pinned in the repository-owned extension registry).
//! User edits override detection explicitly and are recorded as override
//! rows; the apply step requires explicit approval, and nothing is
//! written until it is given.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::workflow::governance_compiler::remediation_for;
use crate::workflow::repo_index::IndexedRepository;
use crate::workflow::soma::{Diagnostic, try_canonical_digest};

/// Schema version of the Lite onboarding contracts.
pub const ONBOARDING_SCHEMA_VERSION: &str = "lite.onboarding.v1";

/// One package system this onboarding pass knows how to detect and
/// derive a validation strategy for. Presence of the marker file is
/// high-confidence evidence; the derived test command is medium
/// confidence until a repository pins it explicitly.
struct KnownSystem {
    /// Repository-relative marker file whose presence detects the system.
    marker: &'static str,
    /// Detection name recorded in the survey.
    system: &'static str,
    /// Default test command for the system.
    test_command: &'static [&'static str],
    /// Validation-strategy identifier recorded in the proposal.
    strategy: &'static str,
    /// Optional lockfile that also counts as a dependency manifest.
    lockfile: Option<&'static str>,
    /// Whether the marker must deep-parse (TOML/JSON) versus
    /// presence-only detection.
    parse: bool,
}

const KNOWN_SYSTEMS: &[KnownSystem] = &[
    KnownSystem {
        marker: "Cargo.toml",
        system: "cargo",
        test_command: &["cargo", "test"],
        strategy: "cargo-test",
        lockfile: Some("Cargo.lock"),
        parse: true,
    },
    KnownSystem {
        marker: "package.json",
        system: "npm",
        test_command: &["npm", "test"],
        strategy: "npm-test",
        lockfile: Some("package-lock.json"),
        parse: true,
    },
    KnownSystem {
        marker: "pyproject.toml",
        system: "python",
        test_command: &["pytest"],
        strategy: "pytest",
        lockfile: None,
        parse: true,
    },
    KnownSystem {
        marker: "requirements.txt",
        system: "python",
        test_command: &["python", "-m", "unittest"],
        strategy: "unittest",
        lockfile: None,
        parse: false,
    },
    KnownSystem {
        marker: "go.mod",
        system: "go",
        test_command: &["go", "test", "./..."],
        strategy: "go-test",
        lockfile: None,
        parse: false,
    },
    KnownSystem {
        marker: "pom.xml",
        system: "maven",
        test_command: &["mvn", "test"],
        strategy: "maven-test",
        lockfile: None,
        parse: false,
    },
    KnownSystem {
        marker: "build.gradle",
        system: "gradle",
        test_command: &["gradle", "test"],
        strategy: "gradle-test",
        lockfile: None,
        parse: false,
    },
];

/// One detected fact: the value, how confident the detection is
/// (`high` = manifest-verified, `medium` = derived/inferred), and the
/// repo-relative evidence it was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectionRowV1 {
    pub field: String,
    pub value: serde_json::Value,
    pub confidence: String,
    pub evidence: Vec<String>,
}

/// The read-only survey result: repository identity plus every
/// detection row, digest-bound so the apply step can detect both survey
/// tampering and repository drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingSurveyV1 {
    pub schema_version: String,
    pub repo_root: String,
    pub revision: String,
    pub dirty: bool,
    pub rows: Vec<DetectionRowV1>,
    /// Canonical digest over this survey minus the digest member itself.
    pub survey_digest: String,
}

/// One suggested test command, derived per detected package system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestCommandV1 {
    pub system: String,
    pub command: Vec<String>,
}

/// The proposed configuration — suggestions only. This struct
/// deliberately has no authority-bearing member: no scopes, no
/// capabilities, no tools. Editing it is the human review surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedConfigV1 {
    pub test_commands: Vec<TestCommandV1>,
    pub validation_strategy: Vec<String>,
    pub protected_paths: Vec<String>,
    pub dependency_manifests: Vec<String>,
    pub languages: Vec<String>,
}

/// One explicit user override of a detected proposal field: the field,
/// what detection proposed, and what the human chose instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverrideRowV1 {
    pub field: String,
    pub detected: serde_json::Value,
    pub user: serde_json::Value,
}

/// The reviewable onboarding manifest: detection evidence (the survey
/// block, detection-owned and immutable by the human), the proposal the
/// human edits, the recorded overrides, and the approval bit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingManifestV1 {
    pub schema_version: String,
    pub survey: OnboardingSurveyV1,
    pub proposed: ProposedConfigV1,
    #[serde(default)]
    pub overrides: Vec<OverrideRowV1>,
    #[serde(default)]
    pub approved: bool,
}

/// One typed onboarding refusal with its curated remediation.
fn onb(code: &'static str, message: String) -> Vec<Diagnostic> {
    let mut d = Diagnostic::new(code, message);
    d.remediation = Some(remediation_for(code));
    vec![d]
}

/// Canonical digest over a survey value with the digest member removed.
fn survey_digest_of(value: &serde_json::Value) -> String {
    let mut v = value.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("surveyDigest");
    }
    try_canonical_digest(&v).expect("Lite-constructed survey digests")
}

/// Recompute a survey's digest from its own members.
fn recomputed_survey_digest(survey: &OnboardingSurveyV1) -> String {
    let value = serde_json::to_value(survey).expect("survey serializes");
    survey_digest_of(&value)
}

/// Survey a repository — strictly read-only. Detects package systems
/// (marker-verified, deep-parsed where applicable), languages, test
/// commands, protected paths, dependency manifests, and Git state, each
/// with confidence and evidence. Fails closed with actionable
/// diagnostics when the setup is unsupported or conflicting.
pub fn survey_repository(repo_root: &Path) -> Result<OnboardingSurveyV1, Vec<Diagnostic>> {
    let display = repo_root.display();
    if !repo_root.join(".git").exists() {
        return Err(onb(
            "SOMA-ONB-0002",
            format!("{display} is not a git repository; onboarding requires one"),
        ));
    }
    let idx = IndexedRepository::build(repo_root).map_err(|e| {
        onb(
            "SOMA-ONB-0002",
            format!("git state at {display} is unreadable: {e}"),
        )
    })?;

    // Package systems: presence of a marker, deep-parsed where the
    // system defines a parseable manifest format. A present-but-broken
    // manifest is a conflicting setup and refuses.
    let mut systems: Vec<&'static str> = Vec::new();
    let mut test_commands: Vec<TestCommandV1> = Vec::new();
    let mut strategies: Vec<&'static str> = Vec::new();
    let mut manifests: Vec<String> = Vec::new();
    let mut test_evidence: Vec<String> = Vec::new();
    for known in KNOWN_SYSTEMS {
        let marker_path = repo_root.join(known.marker);
        if !marker_path.exists() {
            continue;
        }
        if known.parse {
            let text = std::fs::read_to_string(&marker_path).unwrap_or_default();
            let parsed = if known.marker.ends_with(".json") {
                serde_json::from_str::<serde_json::Value>(&text).is_ok()
            } else {
                text.parse::<toml::Table>().is_ok()
            };
            if !parsed {
                return Err(onb(
                    "SOMA-ONB-0003",
                    format!(
                        "{} is present but does not parse as a valid {}; \
                         fix the manifest or remove it",
                        known.marker,
                        if known.marker.ends_with(".json") {
                            "JSON document"
                        } else {
                            "TOML document"
                        }
                    ),
                ));
            }
            if known.system == "npm"
                && serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.get("scripts")
                            .and_then(|s| s.get("test"))
                            .and_then(|t| t.as_str())
                            .map(|_| ())
                    })
                    .is_some()
            {
                test_evidence.push("package.json scripts.test".to_string());
            }
        }
        systems.push(known.system);
        test_commands.push(TestCommandV1 {
            system: known.system.to_string(),
            command: known.test_command.iter().map(|s| s.to_string()).collect(),
        });
        strategies.push(known.strategy);
        manifests.push(known.marker.to_string());
        if let Some(lock) = known.lockfile
            && repo_root.join(lock).exists()
        {
            manifests.push(lock.to_string());
        }
    }
    if systems.is_empty() {
        let supported: Vec<&str> = KNOWN_SYSTEMS.iter().map(|k| k.marker).collect();
        return Err(onb(
            "SOMA-ONB-0001",
            format!(
                "no supported package system detected in {display}; \
                 supported markers: {}",
                supported.join(", ")
            ),
        ));
    }

    // Languages from the shared read-only index (manifest-verified).
    let mut languages: Vec<String> = idx.files.values().map(|f| f.language.clone()).collect();
    languages.sort();
    languages.dedup();

    // Protected paths: always the VCS directory and the tool's own
    // store; plus environment files commonly holding secrets.
    let mut protected_paths = vec![".git".to_string(), ".prometheos-lite".to_string()];
    for env in [".env", ".env.local"] {
        if repo_root.join(env).exists() {
            protected_paths.push(env.to_string());
        }
    }
    protected_paths.sort();
    protected_paths.dedup();

    if test_evidence.is_empty() {
        // The marker manifest is the evidence for a derived default.
        for system in &systems {
            if let Some(known) = KNOWN_SYSTEMS.iter().find(|k| k.system == *system) {
                test_evidence.push(known.marker.to_string());
            }
        }
    }
    test_evidence.sort();

    let mut systems_sorted = systems.clone();
    systems_sorted.sort();
    systems_sorted.dedup();

    let mut strategy_sorted = strategies.to_vec();
    strategy_sorted.sort();
    strategy_sorted.dedup();

    let mut manifests_sorted = manifests.clone();
    manifests_sorted.sort();
    manifests_sorted.dedup();

    let rows = vec![
        DetectionRowV1 {
            field: "package_systems".to_string(),
            value: serde_json::json!(systems_sorted),
            confidence: "high".to_string(),
            evidence: systems_sorted
                .iter()
                .map(|s| {
                    KNOWN_SYSTEMS
                        .iter()
                        .find(|k| k.system == *s)
                        .map(|k| k.marker.to_string())
                        .unwrap_or_default()
                })
                .collect(),
        },
        DetectionRowV1 {
            field: "test_commands".to_string(),
            value: serde_json::json!(test_commands),
            confidence: "medium".to_string(),
            evidence: test_evidence,
        },
        DetectionRowV1 {
            field: "validation_strategy".to_string(),
            value: serde_json::json!(strategy_sorted),
            confidence: "medium".to_string(),
            evidence: systems_sorted
                .iter()
                .map(|s| {
                    KNOWN_SYSTEMS
                        .iter()
                        .find(|k| k.system == *s)
                        .map(|k| k.marker.to_string())
                        .unwrap_or_default()
                })
                .collect(),
        },
        DetectionRowV1 {
            field: "protected_paths".to_string(),
            value: serde_json::json!(protected_paths),
            confidence: "high".to_string(),
            evidence: vec![".git".to_string()],
        },
        DetectionRowV1 {
            field: "dependency_manifests".to_string(),
            value: serde_json::json!(manifests_sorted),
            confidence: "high".to_string(),
            evidence: manifests_sorted.clone(),
        },
        DetectionRowV1 {
            field: "languages".to_string(),
            value: serde_json::json!(languages),
            confidence: "high".to_string(),
            evidence: idx.files.keys().take(1).cloned().collect::<Vec<_>>(),
        },
        DetectionRowV1 {
            field: "git_state".to_string(),
            value: serde_json::json!({
                "revision": idx.identity.revision,
                "dirty": idx.identity.dirty,
            }),
            confidence: "high".to_string(),
            evidence: vec![".git".to_string()],
        },
    ];

    let mut survey = OnboardingSurveyV1 {
        schema_version: ONBOARDING_SCHEMA_VERSION.to_string(),
        repo_root: display.to_string(),
        revision: idx.identity.revision,
        dirty: idx.identity.dirty,
        rows,
        survey_digest: String::new(),
    };
    let value = serde_json::to_value(&survey).expect("survey serializes");
    survey.survey_digest = survey_digest_of(&value);
    Ok(survey)
}

/// Build the reviewable proposal manifest from a survey. The proposal
/// is derived deterministically from the detection rows and enables no
/// authority — it is a suggestion document for human review.
pub fn manifest_from_survey(survey: &OnboardingSurveyV1) -> OnboardingManifestV1 {
    let row = |field: &str| {
        survey
            .rows
            .iter()
            .find(|r| r.field == field)
            .map(|r| r.value.clone())
            .unwrap_or(serde_json::Value::Null)
    };
    let test_commands: Vec<TestCommandV1> =
        serde_json::from_value(row("test_commands")).expect("test_commands row is well-formed");
    OnboardingManifestV1 {
        schema_version: ONBOARDING_SCHEMA_VERSION.to_string(),
        survey: survey.clone(),
        proposed: ProposedConfigV1 {
            test_commands,
            validation_strategy: serde_json::from_value(row("validation_strategy"))
                .expect("validation_strategy row is well-formed"),
            protected_paths: serde_json::from_value(row("protected_paths"))
                .expect("protected_paths row is well-formed"),
            dependency_manifests: serde_json::from_value(row("dependency_manifests"))
                .expect("dependency_manifests row is well-formed"),
            languages: serde_json::from_value(row("languages"))
                .expect("languages row is well-formed"),
        },
        overrides: Vec::new(),
        approved: false,
    }
}

/// Apply a human-edited proposal. The survey block is detection-owned:
/// a user manifest whose survey no longer matches its own digest is
/// refused as tampered evidence, and one whose (honest) survey no
/// longer matches a fresh survey is refused as drift — regenerate the
/// proposal. Every difference between the detection proposal and the
/// user's configuration is recorded as an explicit override row.
/// Approval must be explicit in the reviewed document AND in the call.
pub fn apply_user_manifest(
    proposal: &OnboardingManifestV1,
    user: &OnboardingManifestV1,
    fresh: &OnboardingSurveyV1,
    approve: bool,
) -> Result<OnboardingManifestV1, Vec<Diagnostic>> {
    if recomputed_survey_digest(&user.survey) != user.survey.survey_digest {
        return Err(onb(
            "SOMA-ONB-0005",
            "the survey block is detection-owned evidence and was modified; \
             edit only the proposed configuration"
                .to_string(),
        ));
    }
    if user.survey.survey_digest != fresh.survey_digest {
        return Err(onb(
            "SOMA-ONB-0004",
            "the repository changed after this proposal was generated; \
             regenerate the onboarding proposal and reapply your edits"
                .to_string(),
        ));
    }
    if !user.approved || !approve {
        return Err(onb(
            "SOMA-ONB-0006",
            "applying an onboarding manifest requires explicit approval: \
             set approved to true in the reviewed document and pass --approve"
                .to_string(),
        ));
    }

    let fields: [(&str, &serde_json::Value, &serde_json::Value); 5] = [
        (
            "test_commands",
            &serde_json::to_value(&proposal.proposed.test_commands).unwrap(),
            &serde_json::to_value(&user.proposed.test_commands).unwrap(),
        ),
        (
            "validation_strategy",
            &serde_json::to_value(&proposal.proposed.validation_strategy).unwrap(),
            &serde_json::to_value(&user.proposed.validation_strategy).unwrap(),
        ),
        (
            "protected_paths",
            &serde_json::to_value(&proposal.proposed.protected_paths).unwrap(),
            &serde_json::to_value(&user.proposed.protected_paths).unwrap(),
        ),
        (
            "dependency_manifests",
            &serde_json::to_value(&proposal.proposed.dependency_manifests).unwrap(),
            &serde_json::to_value(&user.proposed.dependency_manifests).unwrap(),
        ),
        (
            "languages",
            &serde_json::to_value(&proposal.proposed.languages).unwrap(),
            &serde_json::to_value(&user.proposed.languages).unwrap(),
        ),
    ];
    let mut overrides = Vec::new();
    for (field, detected, user_value) in fields {
        if detected != user_value {
            overrides.push(OverrideRowV1 {
                field: field.to_string(),
                detected: detected.clone(),
                user: user_value.clone(),
            });
        }
    }
    Ok(OnboardingManifestV1 {
        schema_version: ONBOARDING_SCHEMA_VERSION.to_string(),
        survey: user.survey.clone(),
        proposed: user.proposed.clone(),
        overrides,
        approved: true,
    })
}
