//! `ExecutionProfile` models, the fail-closed authority comparator, and the
//! profile audit. Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-validate/{comparator,profile,numeric}).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::contracts::{AuthorityProfile, Budgets};
use super::types::{Cmp, ExecutionClass, MutationMode, NetworkDefault, number_cmp, sum_numbers};
use super::{Diagnostic, SupportedVersion};

fn rank_execution(c: ExecutionClass) -> u8 {
    match c {
        ExecutionClass::Deterministic => 0,
        ExecutionClass::ModelAssisted => 1,
        ExecutionClass::OpenEnded => 2,
    }
}

fn rank_mutation(m: MutationMode) -> u8 {
    match m {
        MutationMode::None_ => 0,
        MutationMode::Explicit => 1,
    }
}

fn rank_net(d: NetworkDefault) -> u8 {
    match d {
        NetworkDefault::Deny => 0,
        NetworkDefault::Allow => 1,
    }
}

fn set(v: &Option<Vec<String>>) -> BTreeSet<&str> {
    v.iter().flatten().map(String::as_str).collect()
}

fn budget_expanded(eff: &Budgets, dec: &Budgets) -> bool {
    for dim in Budgets::DIMENSIONS {
        if let Some(val) = eff.get(dim) {
            let Some(bound) = dec.get(dim) else {
                // Effective dimension with no declared upper bound.
                return true;
            };
            // Fail closed: an incomparable budget lexeme is a widening.
            match number_cmp(val, bound) {
                Cmp::Greater | Cmp::Incomparable => return true,
                Cmp::NotGreater => {}
            }
        }
    }
    false
}

/// True when `eff` widens `dec` on any tracked dimension.
pub fn authority_widened(eff: &AuthorityProfile, dec: &AuthorityProfile) -> bool {
    if rank_execution(eff.execution_class) > rank_execution(dec.execution_class)
        || rank_mutation(eff.mutation) > rank_mutation(dec.mutation)
    {
        return true;
    }

    for (e, d) in [
        (&eff.readable_scopes, &dec.readable_scopes),
        (&eff.writable_scopes, &dec.writable_scopes),
    ] {
        if !set(e).is_subset(&set(d)) {
            return true;
        }
    }

    if let Some(et) = &eff.tools {
        let empty: Vec<String> = Vec::new();
        let dt = dec.tools.as_ref();
        for (key, vals) in et {
            let declared = dt.and_then(|m| m.get(key)).unwrap_or(&empty);
            let ev: BTreeSet<&str> = vals.iter().map(String::as_str).collect();
            let dv: BTreeSet<&str> = declared.iter().map(String::as_str).collect();
            if !ev.is_subset(&dv) {
                return true;
            }
        }
    }

    if let Some(en) = &eff.network_policy {
        match dec.network_policy.as_ref() {
            None => return true,
            Some(dn) => {
                if rank_net(en.default) > rank_net(dn.default) {
                    return true;
                }
            }
        }
        if net_allowlist(Some(en)).is_subset(&net_allowlist(dec.network_policy.as_ref())) {
            // subset holds — keep checking
        } else {
            return true;
        }
    }

    if !prov_allowlist(eff.provider_policy.as_ref())
        .is_subset(&prov_allowlist(dec.provider_policy.as_ref()))
    {
        return true;
    }

    if secret_expanded(eff.secrets.as_deref(), dec.secrets.as_deref()) {
        return true;
    }

    if obligation_weakened(
        eff.escalation.as_ref().map(|e| e.to.as_str()),
        dec.escalation.as_ref().map(|e| e.to.as_str()),
    ) {
        return true;
    }
    if review_weakened(eff.review.as_ref(), dec.review.as_ref()) {
        return true;
    }
    if abstention_loosened(eff.abstention.as_ref(), dec.abstention.as_ref()) {
        return true;
    }

    match (&eff.budgets, &dec.budgets) {
        (Some(e), Some(d)) if budget_expanded(e, d) => return true,
        (Some(_), None) => return true,
        _ => {}
    }
    false
}

fn net_allowlist(n: Option<&super::contracts::NetworkPolicy>) -> BTreeSet<&str> {
    n.and_then(|n| n.allowlist.as_ref())
        .map(|a| a.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

fn prov_allowlist(p: Option<&super::contracts::ProviderPolicy>) -> BTreeSet<&str> {
    p.and_then(|p| p.allowlist.as_ref())
        .map(|a| a.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

fn secret_expanded(
    eff: Option<&[super::contracts::SecretGrant]>,
    dec: Option<&[super::contracts::SecretGrant]>,
) -> bool {
    let Some(grants) = eff else { return false };
    let declared: BTreeMap<&str, BTreeSet<&str>> = dec
        .map(|ds| {
            ds.iter()
                .map(|d| {
                    (
                        d.name.as_str(),
                        d.scopes
                            .as_ref()
                            .map(|s| s.iter().map(String::as_str).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    for g in grants {
        let Some(allowed) = declared.get(g.name.as_str()) else {
            return true; // undeclared secret
        };
        if let Some(scopes) = &g.scopes
            && !scopes.iter().all(|s| allowed.contains(s.as_str()))
        {
            return true;
        }
    }
    false
}

use std::collections::BTreeMap;

/// An imposed obligation dropped or altered by the effective side widens.
fn obligation_weakened(eff_to: Option<&str>, dec_to: Option<&str>) -> bool {
    match dec_to {
        None => false,
        Some(imposed) => eff_to != Some(imposed),
    }
}

fn review_weakened(
    eff: Option<&super::contracts::ReviewRequirement>,
    dec: Option<&super::contracts::ReviewRequirement>,
) -> bool {
    let Some(dec) = dec else { return false };
    let imposed = dec.effect.is_some() || dec.reviewer_policy.is_some();
    if !imposed {
        return false;
    }
    let Some(eff) = eff else { return true };
    if dec.effect.is_some() && eff.effect != dec.effect {
        return true;
    }
    if dec.reviewer_policy.is_some() && eff.reviewer_policy != dec.reviewer_policy {
        return true;
    }
    false
}

fn abstention_loosened(
    eff: Option<&super::contracts::Abstention>,
    dec: Option<&super::contracts::Abstention>,
) -> bool {
    let dec_behavior = dec.and_then(|a| a.behavior.as_deref());
    let eff_behavior = eff.and_then(|a| a.behavior.as_deref());
    matches!(dec_behavior, Some(b) if b != "allow") && eff_behavior == Some("allow")
}

// ---------------------------------------------------------------------------
// ExecutionProfile model + audit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Boundary {
    None,
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessBoundary {
    None,
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Boundaries {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Boundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<Boundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<Boundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessBoundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<ProcessBoundary>,
    /// secret boundary is none|read only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<Boundary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ApprovalRule {
    pub effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EvidencePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retain: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "minRetentionDays"
    )]
    pub min_retention_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RedactionPolicy {
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "protectedFields"
    )]
    pub protected_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetentionPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ttlDays")]
    pub ttl_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ValidationPolicy {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "requireEvidence"
    )]
    pub require_evidence: Option<bool>,
    /// Must always be true when present (fail-closed invariant).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "failClosed"
    )]
    pub fail_closed: Option<bool>,
}

/// One side of the projection (declared or effective).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProfileSide {
    pub authority: AuthorityProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundaries: Option<Boundaries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budgets: Option<Budgets>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProfileUnit {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budgets: Option<Budgets>,
}

/// The execution profile contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExecutionProfile {
    pub id: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub declared: ProfileSide,
    pub effective: ProfileSide,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<ProfileUnit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Budgets>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "approvalRules"
    )]
    pub approval_rules: Vec<ApprovalRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidencePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction: Option<RedactionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<RetentionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationPolicy>,
}

impl ExecutionProfile {
    pub fn audit(&self, supported: &SupportedVersion) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        for v in [&self.schema_version, &self.version] {
            if let Ok(parsed) = super::types::SemVer::parse(v)
                && !parsed.is_compatible_with(supported)
            {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0001",
                    format!("profile version {v} newer than bundle"),
                ));
            }
        }

        // SOMA-PROF-0001: authority widening / capability claims.
        if authority_widened(&self.effective.authority, &self.declared.authority) {
            out.push(Diagnostic::new(
                "SOMA-PROF-0001",
                "effective authority widens declared authority",
            ));
        }
        {
            let declared_caps: BTreeSet<&str> = self
                .declared
                .capabilities
                .iter()
                .map(String::as_str)
                .collect();
            if self
                .effective
                .capabilities
                .iter()
                .any(|c| !declared_caps.contains(c.as_str()))
            {
                out.push(Diagnostic::new(
                    "SOMA-PROF-0001",
                    "effective capability claim outside declared set",
                ));
            }
        }

        // Fail-closed validation policy invariant.
        if let Some(v) = &self.validation
            && v.fail_closed == Some(false)
        {
            out.push(Diagnostic::new(
                "SOMA-PROF-0001",
                "validation.failClosed must be true when present",
            ));
        }

        // Budget discipline.
        for dim in Budgets::DIMENSIONS {
            let Some(eff) = self.effective.budgets.as_ref().and_then(|b| b.get(dim)) else {
                continue;
            };
            match self.declared.budgets.as_ref().and_then(|b| b.get(dim)) {
                None => out.push(Diagnostic::new(
                    "SOMA-PROF-0002",
                    format!("effective budget {dim} has no declared upper bound"),
                )),
                Some(dec) => match number_cmp(eff, dec) {
                    Cmp::Greater => out.push(Diagnostic::new(
                        "SOMA-PROF-0002",
                        format!("effective budget {dim} exceeds declared limit"),
                    )),
                    Cmp::Incomparable => out.push(Diagnostic::new(
                        "SOMA-PROF-0002",
                        format!("effective budget {dim} is outside the number policy"),
                    )),
                    Cmp::NotGreater => {}
                },
            }
        }

        // SOMA-AUTH-0009: aggregate unit demand vs declared/hard capacity.
        for dim in Budgets::DIMENSIONS {
            let vals: Vec<&Number> = self
                .units
                .iter()
                .filter_map(|u| u.budgets.as_ref().and_then(|b| b.get(dim)))
                .collect();
            if vals.is_empty() {
                continue;
            }
            // Fail closed: an uncountable aggregate is itself a violation.
            let Some(total) = sum_numbers(vals) else {
                out.push(Diagnostic::new(
                    "SOMA-AUTH-0009",
                    format!("unit aggregate {dim} uncountable (overflow)"),
                ));
                continue;
            };
            if let Some(cap) = self.declared.budgets.as_ref().and_then(|b| b.get(dim))
                && number_cmp(&total, cap) == Cmp::Greater
            {
                out.push(Diagnostic::new(
                    "SOMA-AUTH-0009",
                    format!("unit aggregate {dim} exceeds declared budget"),
                ));
            }
            if let Some(hard) = self.limits.as_ref().and_then(|l| l.get(dim))
                && number_cmp(&total, hard) == Cmp::Greater
            {
                out.push(Diagnostic::new(
                    "SOMA-AUTH-0009",
                    format!("unit aggregate {dim} exceeds hard limit"),
                ));
            }
        }

        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }
}
