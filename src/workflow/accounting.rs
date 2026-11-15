//! Execution accounting for governed routed attempts (issue #133, Slice C).
//!
//! A single bounded vertical path: bind **real** usage/latency/cost evidence
//! to actual governed route dispatches, then aggregate honestly.
//!
//! Rules this module enforces:
//!
//! * **Observed, never estimated.** Usage numbers are taken verbatim from
//!   the capability's result document. If the provider reports none, nothing
//!   is invented: the field is `Unknown`, explicitly.
//! * **Measured latency.** `latencyMs` recorded in dispatch is the wall-clock
//!   time between dispatch start and outcome binding at the seam, provenance
//!   labelled `measured-by-route-dispatch`.
//! * **Cost only from an authorized, versioned price book.** The caller
//!   injects a `PricingBook` carrying an explicit `source` and `version`; the
//!   derived cost names that source/version/currency and reports what it was
//!   derived from (`prompt`+`completion` direction rates, `total` flat
//!   rate). No rate, no cost.
//! * **Unknown is not zero.** Missing usage or missing pricing produce
//!   `Unknown`, not 0. Partial usage is recorded as partial; aggregates keep
//!   `known`/`unknown` coverage counters and never produce a single
//!   seemingly-exact number from incomplete coverage.
//! * **Idempotency.** A replayed attempt binds on `idempotency_key` AND the
//!   terminal journal entry digest. The same key with a different journal
//!   digest is a hard conflict — nothing is recorded and the ledger refuses.
//!   The same key with the same digest is an idempotent no-op.
//! * **Failures and cancellations are typed and accounted.** An attempt that
//!   did not produce a verified cost/usage source accrues no tokens and no
//!   cost — unknown, explicitly. It never erases a prior journal entry.
//! * **No secret or payload leakage.** Records carry digests, identity and
//!   numeric fields only — never raw args, prompts, outputs, or secrets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::workflow::node_runner::JournalEntryV1;
use crate::workflow::route_dispatch::RoutedDispatchOutcome;
use crate::workflow::route_resolution::RouteResolution;
use crate::workflow::soma::contracts::Budgets;

pub const ACCOUNTING_SCHEMA_VERSION: &str = "1.0.0";
pub const PRICING_BOOK_SCHEMA_VERSION: &str = "1.0.0";

// ---------------------------------------------------------------------------
// Token usage (reported verbatim from the capability result document)
// ---------------------------------------------------------------------------

/// Token usage as reported by the provider. A missing field means *unknown*,
/// never zero. At least one field is `Some` for a `Known` record; otherwise
/// use `UsageAccounting::Unknown` with an explicit reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl TokenUsage {
    fn any_known(&self) -> bool {
        self.prompt_tokens.is_some()
            || self.completion_tokens.is_some()
            || self.total_tokens.is_some()
    }

    fn is_consistent(&self) -> Result<(), String> {
        match (
            self.prompt_tokens,
            self.completion_tokens,
            self.total_tokens,
        ) {
            (Some(p), Some(c), Some(t)) => {
                let sum = p.checked_add(c).ok_or_else(|| {
                    format!("promptTokens ({p}) + completionTokens ({c}) overflows")
                })?;
                if sum != t {
                    return Err(format!(
                        "provider reported inconsistent token usage: promptTokens {p} + completionTokens {c} != totalTokens {t}"
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Provider-reported token usage, explicitly classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde(tag = "state")]
pub enum UsageAccounting {
    /// At least one usage field was reported and validated. Unreported
    /// fields remain unknown.
    Known(TokenUsage),
    /// The provider produced no usable usage (absent, null, malformed,
    /// or inconsistent with the reported total).
    Unknown { reason: String },
}

// ---------------------------------------------------------------------------
// Cost accounting (only from an authorized, versioned pricing source)
// ---------------------------------------------------------------------------

/// How a cost was derived, so reviewers can audit the derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostMethod {
    /// prompt_rate * prompt_tokens + completion_rate * completion_tokens
    DirectedRates,
    /// flat_rate * total_tokens
    FlatTotalRate,
}

/// A derived monetary cost bound to the exact pricing inputs used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CostComputation {
    /// Integer minor units of the currency (e.g. cents). Exact arithmetic.
    pub minor_units: u64,
    pub currency: String,
    pub pricing_source: String,
    pub pricing_version: String,
    pub method: CostMethod,
    pub derivation_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde(tag = "state")]
pub enum CostAccounting {
    Known(CostComputation),
    /// No trustworthy derivation. `reason` says exactly why.
    Unknown {
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Pricing book — caller-authorized, versioned, explicit provenance
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PricingEntry {
    pub provider: String,
    pub model: String,
    /// ISO 4217 three-letter uppercase currency.
    pub currency: String,
    /// Minor units per 1_000 prompt tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_minor_per_1k: Option<u64>,
    /// Minor units per 1_000 completion tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_minor_per_1k: Option<u64>,
    /// Minor units per 1_000 total tokens (used when directed rates absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flat_minor_per_1k: Option<u64>,
}

/// Authorized, versioned price book, injected by the execution authority
/// (never by the task). Invalid entries fail closed at load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PricingBook {
    pub source: String,
    pub version: String,
    pub schema_version: String,
    pub entries: Vec<PricingEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingBookDefect(pub String);

impl PricingBook {
    /// Strict, fail-closed load of a price book: non-empty source/version,
    /// ISO 4217 shape, deduped (provider, model, currency) rows.
    pub fn from_json(text: &str) -> Result<Self, PricingBookDefect> {
        let book: PricingBook = serde_json::from_str(text)
            .map_err(|error| PricingBookDefect(format!("price book does not parse: {error}")))?;
        book.validate()?;
        Ok(book)
    }

    pub fn validate(&self) -> Result<(), PricingBookDefect> {
        if self.schema_version != PRICING_BOOK_SCHEMA_VERSION {
            return Err(PricingBookDefect(format!(
                "unsupported price book schemaVersion {:?} (supported {PRICING_BOOK_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.source.trim().is_empty() {
            return Err(PricingBookDefect(
                "price book `source` must be non-empty".to_string(),
            ));
        }
        if self.version.trim().is_empty() {
            return Err(PricingBookDefect(
                "price book `version` must be non-empty".to_string(),
            ));
        }
        if self.entries.is_empty() {
            return Err(PricingBookDefect(
                "price book `entries` must be non-empty".to_string(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if entry.provider.trim().is_empty()
                || entry.model.trim().is_empty()
                || entry.currency.trim().is_empty()
            {
                return Err(PricingBookDefect(
                    "price entry provider/model/currency must be non-empty".to_string(),
                ));
            }
            if entry.currency.len() != 3 || !entry.currency.bytes().all(|b| b.is_ascii_uppercase())
            {
                return Err(PricingBookDefect(format!(
                    "price entry currency {:?} is not uppercase ISO 4217 shape",
                    entry.currency
                )));
            }
            let key = (
                entry.provider.as_str(),
                entry.model.as_str(),
                entry.currency.as_str(),
            );
            if !seen.insert(key) {
                return Err(PricingBookDefect(format!(
                    "duplicate price entry for {key:?}"
                )));
            }
            if entry.prompt_minor_per_1k.is_none()
                && entry.completion_minor_per_1k.is_none()
                && entry.flat_minor_per_1k.is_none()
            {
                return Err(PricingBookDefect(format!(
                    "price entry for {key:?} carries no rates"
                )));
            }
        }
        Ok(())
    }

    fn find(&self, provider: &str, model: &str, currency: &str) -> Option<&PricingEntry> {
        self.entries.iter().find(|entry| {
            entry.provider == provider && entry.model == model && entry.currency == currency
        })
    }

    /// Derive a known cost, or an honest reason why it is unknown. Unknown is
    /// never zero. The price book is re-validated at the point of use, so a
    /// book built directly through public fields is held to the same rules.
    ///
    /// Rounding convention: directed-rate costs are computed from the SUM
    /// (`(prompt * prompt_rate + completion * completion_rate) / 1000`) with
    /// a single integer division toward zero (floor for nonnegatives);
    /// flat-total costs scale `(total * flat_rate) / 1000` likewise. No
    /// per-component rounding — every cent of misalignment is accounted.
    pub fn derive_cost(
        &self,
        usage: &TokenUsage,
        provider: &str,
        model: &str,
        currency: &str,
    ) -> Result<CostComputation, CostDefect> {
        self.validate()
            .map_err(|defect| CostDefect::BookInvalid { reason: defect.0 })?;
        let Some(entry) = self.find(provider, model, currency) else {
            return Err(CostDefect::NoPricingMatch {
                provider: provider.to_string(),
                model: model.to_string(),
                currency: currency.to_string(),
            });
        };
        if let (Some(p_rate), Some(c_rate)) =
            (entry.prompt_minor_per_1k, entry.completion_minor_per_1k)
        {
            let prompt = usage.prompt_tokens.ok_or(CostDefect::PartialUsage {
                missing: "promptTokens".to_string(),
            })?;
            let completion = usage.completion_tokens.ok_or(CostDefect::PartialUsage {
                missing: "completionTokens".to_string(),
            })?;
            let prompt_units = (prompt as u128)
                .checked_mul(p_rate as u128)
                .ok_or(CostDefect::Overflow)?;
            let completion_units = (completion as u128)
                .checked_mul(c_rate as u128)
                .ok_or(CostDefect::Overflow)?;
            let total_units = prompt_units
                .checked_add(completion_units)
                .ok_or(CostDefect::Overflow)?;
            let total = u64::try_from(total_units / 1_000).map_err(|_| CostDefect::Overflow)?;
            let tokens = prompt.checked_add(completion).ok_or(CostDefect::Overflow)?;
            return Ok(CostComputation {
                minor_units: total,
                currency: entry.currency.clone(),
                pricing_source: self.source.clone(),
                pricing_version: self.version.clone(),
                method: CostMethod::DirectedRates,
                derivation_tokens: tokens,
            });
        }
        if let Some(f_rate) = entry.flat_minor_per_1k {
            let total = usage.total_tokens.ok_or(CostDefect::PartialUsage {
                missing: "totalTokens".to_string(),
            })?;
            let total_units = (total as u128)
                .checked_mul(f_rate as u128)
                .ok_or(CostDefect::Overflow)?;
            let cost = u64::try_from(total_units / 1_000).map_err(|_| CostDefect::Overflow)?;
            return Ok(CostComputation {
                minor_units: cost,
                currency: entry.currency.clone(),
                pricing_source: self.source.clone(),
                pricing_version: self.version.clone(),
                method: CostMethod::FlatTotalRate,
                derivation_tokens: total,
            });
        }
        Err(CostDefect::EntryRatesUnusable)
    }
}

/// Why a cost could not be known. Used as honest refusals, not errors that
/// fabricate a zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CostDefect {
    NoPricingMatch {
        provider: String,
        model: String,
        currency: String,
    },
    PartialUsage {
        missing: String,
    },
    Overflow,
    EntryRatesUnusable,
    /// The price book failed validation at the point of use.
    BookInvalid {
        reason: String,
    },
}

impl std::fmt::Display for CostDefect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CostDefect::NoPricingMatch {
                provider,
                model,
                currency,
            } => write!(
                f,
                "no pricing for provider {provider:?} model {model:?} currency {currency:?}"
            ),
            CostDefect::PartialUsage { missing } => {
                write!(
                    f,
                    "usage is partially known; cannot price without {missing}"
                )
            }
            CostDefect::Overflow => write!(f, "cost or usage derivation overflows"),
            CostDefect::EntryRatesUnusable => write!(f, "price entry carries no usable rates"),
            CostDefect::BookInvalid { reason } => write!(f, "price book invalid at use: {reason}"),
        }
    }
}
impl std::error::Error for CostDefect {}

// ---------------------------------------------------------------------------
// Over-budget evidence (post-effect: tokens/cost vs ceilings)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverBudgetEvidence {
    pub dimension: String,
    pub observed: u64,
    pub ceiling: u64,
    /// What imposed the ceiling (e.g. "effective-route-budget").
    pub ceiling_source: String,
}

// ---------------------------------------------------------------------------
// Per-attempt accounting record
// ---------------------------------------------------------------------------

/// The outcome class, which selects the required binding semantics.
///
/// * `Completed` — a sealed routed attempt exists; BOTH `attempt_digest`
///   and `journal_digest` are REQUIRED.
/// * `Failed` / `Cancelled` — the routed operation did NOT complete through
///   a sealed attempt; `attempt_digest` MUST be absent (a snapshot digest
///   masquerades nothing), and `journal_digest` is present only when the
///   dispatch produced a partial journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcomeKind", rename_all = "camelCase")]
pub enum AttemptOutcomeAccounting {
    Completed,
    Failed { kind: String },
    Cancelled { kind: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptAccountingV1 {
    pub schema_version: String,
    pub accounting_id: String,
    pub graph_id: String,
    pub node_id: String,
    pub idempotency_key: String,
    /// Digest of the sealed routed attempt; REQUIRED for `Completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_digest: Option<String>,
    /// Digest of the terminal journal entry. REQUIRED for `Completed`;
    /// absent when the dispatch produced no journal entry (pre-effect failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_digest: Option<String>,
    pub harness_id: String,
    pub harness_revision: String,
    pub provider_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub route_set_id: String,
    pub usage: UsageAccounting,
    /// Wall-clock ms measured at the dispatch boundary.
    pub latency_ms: u64,
    pub cost: CostAccounting,
    pub outcome: AttemptOutcomeAccounting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over_budget: Option<OverBudgetEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl AttemptAccountingV1 {
    pub fn try_sealed(mut self) -> anyhow::Result<Self> {
        self.content_digest = None;
        self.content_digest = Some(self.try_compute_digest()?);
        Ok(self)
    }

    fn try_compute_digest(&self) -> anyhow::Result<String> {
        let mut value = serde_json::to_value(self).context("accounting record serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::{bail, ensure};
        ensure!(
            self.schema_version == ACCOUNTING_SCHEMA_VERSION,
            "unsupported accounting schema version {} (supported {ACCOUNTING_SCHEMA_VERSION})",
            self.schema_version
        );
        ensure!(
            !self.idempotency_key.is_empty(),
            "idempotencyKey must be non-empty"
        );
        ensure!(
            !self.route_set_id.is_empty(),
            "routeSetId must be non-empty"
        );
        ensure!(!self.graph_id.is_empty(), "graphId must be non-empty");
        ensure!(!self.node_id.is_empty(), "nodeId must be non-empty");
        ensure!(!self.harness_id.is_empty(), "harnessId must be non-empty");
        ensure!(
            !self.harness_revision.is_empty(),
            "harnessRevision must be non-empty"
        );
        match self.outcome {
            AttemptOutcomeAccounting::Completed => {
                let attempt = self.attempt_digest.as_deref().unwrap_or("");
                let journal = self.journal_digest.as_deref().unwrap_or("");
                ensure!(
                    !attempt.is_empty(),
                    "Completed attempt requires attemptDigest"
                );
                ensure!(
                    !journal.is_empty(),
                    "Completed attempt requires journalDigest"
                );
                ensure!(
                    crate::workflow::route_resolution::is_hex64(attempt),
                    "attemptDigest is not lower hex sha256"
                );
                ensure!(
                    crate::workflow::route_resolution::is_hex64(journal),
                    "journalDigest is not lower hex sha256"
                );
            }
            AttemptOutcomeAccounting::Failed { .. }
            | AttemptOutcomeAccounting::Cancelled { .. } => {
                ensure!(
                    self.attempt_digest.is_none(),
                    "failed/cancelled attempts must not claim an attempt digest (a snapshot digest is not an attempt)"
                );
                if let Some(journal) = &self.journal_digest {
                    ensure!(
                        crate::workflow::route_resolution::is_hex64(journal),
                        "journalDigest is not lower hex sha256"
                    );
                }
            }
        }
        let expected = self.try_compute_digest()?;
        match &self.content_digest {
            Some(actual) if *actual == expected => Ok(()),
            _ => bail!("attempt accounting contentDigest does not verify"),
        }
    }
}

use anyhow::Context;

// ---------------------------------------------------------------------------
// Building accounting records from actual routed dispatches
// ---------------------------------------------------------------------------

/// The graph/node coordinates of a routed attempt (caller-provided —
/// accounts describe work, they don't plan it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountingContext {
    pub graph_id: String,
    pub node_id: String,
    /// Currency to price this attempt in, when a price book is applied.
    pub currency: Option<String>,
}

/// Path-specific reader for the governed harness result document. Each
/// supported model harness emits exactly the same governed-success shape
/// (see `local_model`, `remote_service`, `external_subject`):
///
///   {"success": true, "status": "ok",
///    "output": {"harnessId": …, "capability": …,
///               "content": {…provider-reported document incl. "usage"…},
///               "latencyMs": …, "requestId": …, "correlationId": …}}
///
/// The usage facts, if any, live at `output.content.usage`. Any other path —
/// including a nested "usage" inside tool payloads or prompt text — is not
/// the provider's usage report and is never read as one. Documents that
/// don't match this envelope are Unknown("usage provenance unavailable").
fn provider_usage_path(
    document: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    document
        .get("output")
        .and_then(serde_json::Value::as_object)
        .and_then(|output| output.get("content"))
        .and_then(serde_json::Value::as_object)
        .and_then(|content| content.get("usage"))
        .and_then(serde_json::Value::as_object)
}

/// Parse the provider's usage report, fail-closed on malformed numeric
/// fields. Every present field must be an unsigned integer; any present
/// field of any other type is a typed refusal, not a silent None.
fn parse_reported_usage(output: &str) -> Result<UsageAccounting, String> {
    let document: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| format!("result document does not parse: {error}"))?;

    if document.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
        return Ok(UsageAccounting::Unknown {
            reason: "provider success document is missing success=true (provenance unavailable)"
                .to_string(),
        });
    }
    let Some(usage) = provider_usage_path(&document) else {
        return Ok(UsageAccounting::Unknown {
            reason:
                "provider usage provenance unavailable (no usage object at output.content.usage)"
                    .to_string(),
        });
    };
    let field = |key: &str| -> Result<Option<u64>, String> {
        match usage.get(key) {
            None => Ok(None),
            Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Number(number)) => match number.as_u64() {
                Some(value) => Ok(Some(value)),
                None => Err(format!(
                    "usage.{key} must be an unsigned 64-bit integer, got {number}"
                )),
            },
            Some(other) => Err(format!("usage.{key} must be a number, got {other}")),
        }
    };
    let prompt = field("promptTokens")?;
    let completion = field("completionTokens")?;
    let total = field("totalTokens")?;
    let usage = TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
    };
    usage.is_consistent()?;
    if !usage.any_known() {
        return Ok(UsageAccounting::Unknown {
            reason: "provider returned usage with no usable fields".to_string(),
        });
    }
    Ok(UsageAccounting::Known(usage))
}

pub fn account_routed_failure(
    resolution: &RouteResolution,
    failure_kind: &str,
    context: &AccountingContext,
    step_id: &str,
    journal_entry: Option<JournalEntryV1>,
    latency_ms: u64,
) -> AttemptAccountingV1 {
    // A failed operation through routed dispatch is NOT a sealed routed
    // attempt. `attempt_digest` is absent by construction; `journal_digest`
    // is populated if the dispatch produced a partial journal entry.
    AttemptAccountingV1 {
        schema_version: ACCOUNTING_SCHEMA_VERSION.to_string(),
        accounting_id: format!("acct/{}/{}/{}", context.graph_id, context.node_id, step_id),
        graph_id: context.graph_id.clone(),
        node_id: context.node_id.clone(),
        idempotency_key: step_id.to_string(),
        attempt_digest: None,
        journal_digest: journal_entry
            .as_ref()
            .map(|entry| entry.entry_digest.clone()),
        harness_id: resolution.snapshot.selected.harness_id.clone(),
        harness_revision: resolution.snapshot.selected.harness_revision.clone(),
        provider_identity: resolution.snapshot.selected.provider_identity.clone(),
        model: resolution.snapshot.selected.model.clone(),
        route_set_id: resolution.snapshot.route_set_id.clone(),
        usage: UsageAccounting::Unknown {
            reason: format!("attempt {failure_kind}; provider usage unavailable"),
        },
        latency_ms,
        cost: CostAccounting::Unknown {
            reason: format!("attempt {failure_kind}; cost cannot be derived"),
        },
        outcome: if failure_kind == "cancelled" {
            AttemptOutcomeAccounting::Cancelled {
                kind: failure_kind.to_string(),
            }
        } else {
            AttemptOutcomeAccounting::Failed {
                kind: failure_kind.to_string(),
            }
        },
        over_budget: None,
        content_digest: None,
    }
    .try_sealed()
    .expect("failure accounting records are Lite-constructed and cannot fail to seal")
}

fn over_budget_from_tokens(
    usage: &UsageAccounting,
    budget: Option<&Budgets>,
) -> Option<OverBudgetEvidence> {
    let UsageAccounting::Known(usage) = usage else {
        return None;
    };
    let tokens =
        usage
            .total_tokens
            .or_else(|| match (usage.prompt_tokens, usage.completion_tokens) {
                (Some(p), Some(c)) => p.checked_add(c),
                _ => None,
            })?;
    let ceiling_tokens = budget.and_then(|b| b.tokens.as_ref().and_then(|n| n.as_u64()))?;
    if tokens > ceiling_tokens {
        return Some(OverBudgetEvidence {
            dimension: "tokens".to_string(),
            observed: tokens,
            ceiling: ceiling_tokens,
            ceiling_source: "effective-route-budget".to_string(),
        });
    }
    None
}

/// Successful routed attempt -> accounting record, fail-closed on malformed
/// or inconsistent provider reports and on arithmetic overflow. Refusal
/// happens before any new effect; it never re-runs the attempt.
pub fn account_routed_outcome(
    outcome: &RoutedDispatchOutcome,
    resolution: &RouteResolution,
    context: &AccountingContext,
    pricing: Option<&PricingBook>,
) -> Result<AttemptAccountingV1, AccountingError> {
    let attempt = &outcome.attempt;
    let snapshot = &resolution.snapshot;

    // The sealed routed attempt must itself verify — we are about to price
    // against it; a broken attempt is not a pricing source.
    outcome.attempt.verify().map_err(|error| {
        AccountingError::AttributionMismatch(format!(
            "sealed routed attempt does not verify: {error}"
        ))
    })?;

    if outcome.outcome.harness_id != snapshot.selected.harness_id {
        return Err(AccountingError::AttributionMismatch(format!(
            "outcome harness {:?} is not the snapshot-selected harness {:?}",
            outcome.outcome.harness_id, snapshot.selected.harness_id
        )));
    }
    if outcome.outcome.evidence_entry_digest != attempt.journal_entry.entry_digest {
        return Err(AccountingError::AttributionMismatch(format!(
            "outcome evidence digest {:?} does not equal the attempt's journal digest {:?}",
            outcome.outcome.evidence_entry_digest, attempt.journal_entry.entry_digest
        )));
    }
    let expected_snapshot_digest = snapshot.content_digest.as_deref().unwrap_or_default();
    if attempt.snapshot_digest != expected_snapshot_digest {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt snapshot digest {:?} does not equal the resolution snapshot digest {:?}",
            attempt.snapshot_digest, expected_snapshot_digest
        )));
    }
    let expected_evidence_digest = resolution
        .evidence
        .content_digest
        .as_deref()
        .unwrap_or_default();
    if attempt.route_evidence_digest != expected_evidence_digest {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt route-evidence digest {:?} does not equal the resolution evidence digest {:?}",
            attempt.route_evidence_digest, expected_evidence_digest
        )));
    }
    if attempt.decision_digest != snapshot.decision_digest {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt decision digest {:?} does not equal the resolution decision digest {:?}",
            attempt.decision_digest, snapshot.decision_digest
        )));
    }
    if attempt.harness_id != snapshot.selected.harness_id {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt harness id {:?} does not equal the snapshot-selected harness id {:?}",
            attempt.harness_id, snapshot.selected.harness_id
        )));
    }
    if attempt.harness_id != snapshot.selected.harness_id {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt harness id {:?} does not equal the snapshot-selected harness id {:?}",
            attempt.harness_id, snapshot.selected.harness_id
        )));
    }
    if attempt.provider_identity != snapshot.selected.provider_identity {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt provider {:?} does not equal the snapshot-selected provider {:?}",
            attempt.provider_identity, snapshot.selected.provider_identity
        )));
    }
    if attempt.model != snapshot.selected.model {
        return Err(AccountingError::AttributionMismatch(format!(
            "attempt model {:?} does not equal the snapshot-selected model {:?}",
            attempt.model, snapshot.selected.model
        )));
    }

    // Re-verify the journal entry digest against its contents; the journal
    // is attacker-influenced input at this layer.
    crate::workflow::route_dispatch::verify_journal_entry(&attempt.journal_entry).map_err(
        |error| {
            AccountingError::AttributionMismatch(format!("journal entry does not verify: {error}"))
        },
    )?;

    let usage = parse_reported_usage(&outcome.outcome.output)
        .map_err(AccountingError::InconsistentProviderReport)?;
    let model = snapshot.selected.model.clone();
    let provider = resolution.snapshot.selected.provider_identity.as_str();
    let cost = derive_cost_or_unknown(
        pricing,
        &usage,
        provider,
        model.as_deref().unwrap_or(""),
        context,
    );
    let over_budget = over_budget_from_tokens(
        &usage,
        resolution
            .snapshot
            .effective_profile
            .effective
            .budgets
            .as_ref(),
    );

    let journal_digest_str = outcome.attempt.journal_entry.entry_digest.clone();
    let journal_digest_prefix = &journal_digest_str[..16.min(journal_digest_str.len())];

    AttemptAccountingV1 {
        schema_version: ACCOUNTING_SCHEMA_VERSION.to_string(),
        accounting_id: format!(
            "acct/{}/{}/{}/{}",
            context.graph_id,
            context.node_id,
            outcome.attempt.idempotency_key,
            journal_digest_prefix
        ),
        graph_id: context.graph_id.clone(),
        node_id: context.node_id.clone(),
        idempotency_key: outcome.attempt.idempotency_key.clone(),
        attempt_digest: outcome.attempt.content_digest.clone(),
        journal_digest: Some(journal_digest_str.clone()),
        harness_id: snapshot.selected.harness_id.clone(),
        harness_revision: snapshot.selected.harness_revision.clone(),
        provider_identity: snapshot.selected.provider_identity.clone(),
        model,
        route_set_id: snapshot.route_set_id.clone(),
        usage,
        latency_ms: outcome.latency_ms,
        cost,
        outcome: AttemptOutcomeAccounting::Completed,
        over_budget,
        content_digest: None,
    }
    .try_sealed()
    .map_err(|error| AccountingError::Internal(error.to_string()))
}

fn derive_cost_or_unknown(
    pricing: Option<&PricingBook>,
    usage: &UsageAccounting,
    provider: &str,
    model: &str,
    context: &AccountingContext,
) -> CostAccounting {
    let Some(pricing) = pricing else {
        return CostAccounting::Unknown {
            reason: "no authorized pricing book was supplied; refusing to invent a cost"
                .to_string(),
        };
    };
    let Some(currency) = context.currency.clone() else {
        return CostAccounting::Unknown {
            reason: "no currency specified for this attempt; refusing to guess one".to_string(),
        };
    };
    let UsageAccounting::Known(usage) = usage else {
        return CostAccounting::Unknown {
            reason:
                "usage is unknown or partially reported; refusing to invent a cost for partial data"
                    .to_string(),
        };
    };
    match pricing.derive_cost(usage, provider, model, &currency) {
        Ok(computation) => CostAccounting::Known(computation),
        Err(defect) => CostAccounting::Unknown {
            reason: defect.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum AccountingError {
    /// The provider's usage report is internally inconsistent; recording it
    /// would invent a false fact. The attempt's outcome/evidence is untouched.
    InconsistentProviderReport(String),
    /// Attempt, route snapshot, decision, outcome and journal do not refer to
    /// the same actual execution. Refused before any cost fact is derived.
    AttributionMismatch(String),
    /// A checked arithmetic operation in aggregation would overflow. Never
    /// reports a saturated value as if it were exact.
    AggregationOverflow { field: String },
    /// An accounting fact cannot be sealed/digested (never fabricated).
    Internal(String),
}

impl std::fmt::Display for AccountingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountingError::InconsistentProviderReport(reason) => {
                write!(f, "provider usage report refused: {reason}")
            }
            AccountingError::AttributionMismatch(reason) => {
                write!(f, "outcome does not match the resolved route: {reason}")
            }
            AccountingError::AggregationOverflow { field } => {
                write!(f, "aggregation would overflow on field {field}")
            }
            AccountingError::Internal(reason) => write!(f, "internal error: {reason}"),
        }
    }
}
impl std::error::Error for AccountingError {}

// ---------------------------------------------------------------------------
// Aggregation: per-node, per-graph, with honest coverage semantics
// ---------------------------------------------------------------------------

/// Checked u64 accumulator: never saturates. Overflow is an explicit, typed
/// failure naming the field, never a silently clipped total.
#[inline]
fn checked_u64_add(acc: &mut u64, value: u64, field: &str) -> Result<(), AccountingError> {
    *acc = acc
        .checked_add(value)
        .ok_or_else(|| AccountingError::AggregationOverflow {
            field: field.to_string(),
        })?;
    Ok(())
}

/// Checked u128 accumulator (here for currencies: exact minor units never
/// wrap even at extreme totals).
#[inline]
fn checked_u128_add(acc: &mut u128, value: u128, field: &str) -> Result<(), AccountingError> {
    *acc = acc
        .checked_add(value)
        .ok_or_else(|| AccountingError::AggregationOverflow {
            field: field.to_string(),
        })?;
    Ok(())
}

/// Aggregated token usage across attempts. `complete` is true only when
/// every contributing attempt reported total usage and every sum was
/// representable; otherwise the totals are partial and the coverage counters
/// say so. A saturated total is never presented as an exact observation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageAggregate {
    pub complete: bool,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub known_attempts: u64,
    pub partial_attempts: u64,
    pub unknown_attempts: u64,
}

/// Aggregated cost, bucketed per currency. Currencies are never summed
/// together; `mixed_currencies` lists every currency that contributed, and
/// `total_known_exact` is true only when every contributing attempt had a
/// known cost — an unknown cost prevents an exact currency total claim.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CostAggregate {
    pub currencies: BTreeMap<String, u128>,
    pub known_attempts: BTreeMap<String, u64>,
    pub unknown_attempts: u64,
    pub mixed_currencies: Vec<String>,
    /// Set false when ANY contributing cost is unknown; exact currency
    /// totals may then only be claimed for the known subset, not the whole.
    pub total_known_exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeAccounting {
    pub node_id: String,
    pub attempts: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub usage: UsageAggregate,
    pub cost: CostAggregate,
    pub latency_ms_total: u64,
    pub latency_ms_max: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphAccounting {
    pub schema_version: String,
    pub graph_id: String,
    pub nodes: Vec<NodeAccounting>,
    pub attempts: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub usage: UsageAggregate,
    pub cost: CostAggregate,
    pub latency_ms_total: u64,
    pub latency_ms_max: u64,
}

fn usage_complete(u: &TokenUsage) -> bool {
    u.prompt_tokens.is_some() && u.completion_tokens.is_some() && u.total_tokens.is_some()
}

fn aggregate_usage(records: &[&AttemptAccountingV1]) -> Result<UsageAggregate, AccountingError> {
    let mut agg = UsageAggregate::default();
    for record in records {
        match &record.usage {
            UsageAccounting::Known(usage) => {
                if let Some(p) = usage.prompt_tokens {
                    checked_u64_add(&mut agg.prompt_tokens, p, "promptTokens")?;
                }
                if let Some(c) = usage.completion_tokens {
                    checked_u64_add(&mut agg.completion_tokens, c, "completionTokens")?;
                }
                if let Some(t) = usage.total_tokens {
                    checked_u64_add(&mut agg.total_tokens, t, "totalTokens")?;
                }
                if usage_complete(usage) {
                    checked_u64_add(&mut agg.known_attempts, 1, "knownAttempts")?;
                } else {
                    checked_u64_add(&mut agg.partial_attempts, 1, "partialAttempts")?;
                }
            }
            UsageAccounting::Unknown { .. } => {
                checked_u64_add(&mut agg.unknown_attempts, 1, "unknownAttempts")?;
            }
        }
    }
    let mut all_complete = true;
    for record in records {
        match &record.usage {
            UsageAccounting::Known(u) => {
                if !usage_complete(u) {
                    all_complete = false;
                }
            }
            UsageAccounting::Unknown { .. } => all_complete = false,
        }
    }
    agg.complete = all_complete && !records.is_empty();
    Ok(agg)
}

fn aggregate_cost(records: &[&AttemptAccountingV1]) -> Result<CostAggregate, AccountingError> {
    let mut agg = CostAggregate {
        // Nothing is known until at least one cost fact exists; exactness is
        // earned, never presumed.
        total_known_exact: true,
        ..CostAggregate::default()
    };
    for record in records {
        match &record.cost {
            CostAccounting::Known(computation) => {
                let sum = agg
                    .currencies
                    .entry(computation.currency.clone())
                    .or_insert(0);
                checked_u128_add(sum, computation.minor_units as u128, "costMinorUnits")?;
                let known = agg
                    .known_attempts
                    .entry(computation.currency.clone())
                    .or_insert(0);
                checked_u64_add(known, 1, "costKnownAttempts")?;
                if !agg.mixed_currencies.contains(&computation.currency) {
                    agg.mixed_currencies.push(computation.currency.clone());
                }
            }
            CostAccounting::Unknown { .. } => {
                checked_u64_add(&mut agg.unknown_attempts, 1, "costUnknownAttempts")?;
                agg.total_known_exact = false;
            }
        }
    }
    agg.mixed_currencies.sort();
    Ok(agg)
}

fn outcome_counts(
    records: &[&AttemptAccountingV1],
) -> Result<(u64, u64, u64, u64), AccountingError> {
    let mut attempts = 0u64;
    let mut completed = 0u64;
    let mut failed = 0u64;
    let mut cancelled = 0u64;
    for record in records {
        checked_u64_add(&mut attempts, 1, "outcomeAttempts")?;
        match &record.outcome {
            AttemptOutcomeAccounting::Completed => checked_u64_add(&mut completed, 1, "completed")?,
            AttemptOutcomeAccounting::Failed { .. } => checked_u64_add(&mut failed, 1, "failed")?,
            AttemptOutcomeAccounting::Cancelled { .. } => {
                checked_u64_add(&mut cancelled, 1, "cancelled")?
            }
        }
    }
    Ok((attempts, completed, failed, cancelled))
}

fn latency_max(records: &[&AttemptAccountingV1]) -> u64 {
    records.iter().map(|r| r.latency_ms).max().unwrap_or(0)
}

fn aggregate_records(
    records: Vec<&AttemptAccountingV1>,
) -> Result<NodeAccounting, AccountingError> {
    let node_id = records
        .first()
        .map(|r| r.node_id.clone())
        .unwrap_or_default();
    let (attempts, completed, failed, cancelled) = outcome_counts(&records)?;
    let usage = aggregate_usage(&records)?;
    let cost = aggregate_cost(&records)?;
    let latency_ms_total = records.iter().try_fold(0u64, |acc, r| {
        acc.checked_add(r.latency_ms)
            .ok_or_else(|| AccountingError::AggregationOverflow {
                field: "latencyMsTotal".to_string(),
            })
    })?;
    let latency_ms_max = latency_max(&records);
    Ok(NodeAccounting {
        node_id,
        attempts,
        completed,
        failed,
        cancelled,
        usage,
        cost,
        latency_ms_total,
        latency_ms_max,
    })
}

/// Scope of a ledger binding: graph, node and idempotency key. The same key
/// MUST bind to one sealed record forever; different keys in the same graph
/// may name different outstanding attempts.
type LedgerScope = (String, String, String);

/// The sealed facts a binding commits to: journal digest, attempt digest and
/// the record's own content digest.
type LedgerBinding = (Option<String>, Option<String>, Option<String>);

/// A bounded in-memory ledger with journal-bound idempotency, scoped by
/// graph, node and idempotency key. The whole sealed accounting record must
/// match for a duplicate; anything else is a hard conflict that records
/// nothing.
#[derive(Debug, Default)]
pub struct AccountingLedger {
    records: Vec<AttemptAccountingV1>,
    bindings: BTreeMap<LedgerScope, LedgerBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// Record accepted and appended.
    Recorded,
    /// Identical sealed record observed before; nothing changed.
    Duplicate,
}

#[derive(Debug)]
pub enum LedgerError {
    /// Same binding triple observed with a different accounting record —
    /// usage, cost, latency or outcome moved. Fail closed: record nothing.
    IdempotencyConflict {
        key: String,
        graph_id: String,
        node_id: String,
        existing: Option<String>,
        incoming: Option<String>,
    },
    Internal(String),
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerError::IdempotencyConflict {
                graph_id,
                node_id,
                key,
                existing,
                incoming,
            } => write!(
                f,
                "idempotency key {key:?} in graph {graph_id:?} node {node_id:?} rebound to a different sealed accounting record (digest {existing:?} vs {incoming:?})"
            ),
            LedgerError::Internal(reason) => write!(f, "internal accounting error: {reason}"),
        }
    }
}
impl std::error::Error for LedgerError {}

impl AccountingLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// The scope of a binding: graph + node + idempotency key. Two different
    /// keys in the same graph/node may legitimately name different
    /// outstanding attempts; the same key MUST bind to one record forever.
    fn scope_of(record: &AttemptAccountingV1) -> LedgerScope {
        (
            record.graph_id.clone(),
            record.node_id.clone(),
            record.idempotency_key.clone(),
        )
    }

    pub fn record(&mut self, record: AttemptAccountingV1) -> Result<InsertOutcome, LedgerError> {
        record
            .verify()
            .map_err(|error| LedgerError::Internal(error.to_string()))?;
        let scope = Self::scope_of(&record);
        let incoming = (
            record.journal_digest.clone(),
            record.attempt_digest.clone(),
            record.content_digest.clone(),
        );
        match self.bindings.get(&scope) {
            Some(existing) if *existing == incoming => Ok(InsertOutcome::Duplicate),
            Some(existing) => Err(LedgerError::IdempotencyConflict {
                key: record.idempotency_key.clone(),
                graph_id: record.graph_id.clone(),
                node_id: record.node_id.clone(),
                existing: existing.2.clone(),
                incoming: record.content_digest.clone(),
            }),
            None => {
                self.bindings.insert(scope, incoming);
                self.records.push(record);
                Ok(InsertOutcome::Recorded)
            }
        }
    }

    pub fn records(&self) -> &[AttemptAccountingV1] {
        &self.records
    }

    /// Sum every attempt in `graph_id` into a graph-scoped aggregate.
    pub fn aggregate_graph(&self, graph_id: &str) -> Result<GraphAccounting, AccountingError> {
        let mut nodes: BTreeMap<String, Vec<&AttemptAccountingV1>> = BTreeMap::new();
        let mut records = Vec::new();
        for record in &self.records {
            if record.graph_id != graph_id {
                continue;
            }
            nodes
                .entry(record.node_id.clone())
                .or_default()
                .push(record);
            records.push(record);
        }
        let node_list: Vec<NodeAccounting> = nodes
            .into_values()
            .map(aggregate_records)
            .collect::<Result<_, _>>()?;
        let (attempts, completed, failed, cancelled) = outcome_counts(&records)?;
        let usage = aggregate_usage(&records)?;
        let cost = aggregate_cost(&records)?;
        let latency_ms_total = records.iter().try_fold(0u64, |acc, r| {
            acc.checked_add(r.latency_ms)
                .ok_or_else(|| AccountingError::AggregationOverflow {
                    field: "graphLatencyMsTotal".to_string(),
                })
        })?;
        let latency_ms_max = latency_max(&records);
        Ok(GraphAccounting {
            schema_version: ACCOUNTING_SCHEMA_VERSION.to_string(),
            graph_id: graph_id.to_string(),
            nodes: node_list,
            attempts,
            completed,
            failed,
            cancelled,
            usage,
            cost,
            latency_ms_total,
            latency_ms_max,
        })
    }

    pub fn over_budget_records(&self) -> Vec<&AttemptAccountingV1> {
        self.records
            .iter()
            .filter(|r| r.over_budget.is_some())
            .collect()
    }
}
