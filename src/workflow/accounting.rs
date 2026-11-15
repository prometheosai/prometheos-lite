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

    fn is_consistent(&self) -> bool {
        match (
            self.prompt_tokens,
            self.completion_tokens,
            self.total_tokens,
        ) {
            (Some(p), Some(c), Some(t)) => p.saturating_add(c) == t,
            _ => true,
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
    /// never zero. Empty denominator (no rates) is refused at load.
    pub fn derive_cost(
        &self,
        usage: &TokenUsage,
        provider: &str,
        model: &str,
        currency: &str,
    ) -> Result<CostComputation, CostDefect> {
        let Some(entry) = self.find(provider, model, currency) else {
            return Err(CostDefect::NoPricingMatch {
                provider: provider.to_string(),
                model: model.to_string(),
                currency: currency.to_string(),
            });
        };
        let scale = |tokens: u64, per_1k: u64| -> Result<u64, CostDefect> {
            let scaled = (tokens as u128)
                .checked_mul(per_1k as u128)
                .ok_or(CostDefect::Overflow)?;
            u64::try_from(scaled / 1_000).map_err(|_| CostDefect::Overflow)
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
            let p_cost = scale(prompt, p_rate)?;
            let c_cost = scale(completion, c_rate)?;
            let total = p_cost.checked_add(c_cost).ok_or(CostDefect::Overflow)?;
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
            let cost = scale(total, f_rate)?;
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
    pub attempt_digest: String,
    pub journal_digest: String,
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
        for (label, value) in [
            ("attemptDigest", &self.attempt_digest),
            ("journalDigest", &self.journal_digest),
        ] {
            if !value.is_empty() {
                ensure!(
                    crate::workflow::route_resolution::is_hex64(value),
                    "{label} is not lower hex sha256"
                );
            }
        }
        // An empty journal digest marks "no terminal journal entry" (a
        // pre-effect failure); it is not a valid digest, but the record is
        // still bound by its attempt/snapshot digest.
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

/// Deterministic recursive lookup: the first "usage" key in BTreeMap key
/// order, anywhere in the document. Harness documents nest their result
/// under varying depths (`output`, `content`, ...), so the search covers
/// every shape without an "output unwrap" guess.
fn find_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|child| find_key(child, key))
        }
        _ => None,
    }
}

/// Parse the capability result document's usage section, fail-closed on
/// malformed or inconsistent reports (total != prompt + completion).
fn parse_reported_usage(output: &str) -> Result<UsageAccounting, String> {
    let document: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| format!("result document does not parse: {error}"))?;
    let usage = find_key(&document, "usage");
    let Some(usage) = usage else {
        return Ok(UsageAccounting::Unknown {
            reason: "provider returned no usage section".to_string(),
        });
    };
    if usage.is_null() {
        return Ok(UsageAccounting::Unknown {
            reason: "provider returned null usage".to_string(),
        });
    }
    let obj = usage
        .as_object()
        .ok_or_else(|| "usage section is not an object".to_string())?;
    let prompt = obj.get("promptTokens").and_then(serde_json::Value::as_u64);
    let completion = obj
        .get("completionTokens")
        .and_then(serde_json::Value::as_u64);
    let total = obj.get("totalTokens").and_then(serde_json::Value::as_u64);
    let usage = TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
    };
    if !usage.is_consistent() {
        return Err(format!(
            "provider reported inconsistent token usage (prompt={:?} completion={:?} total={:?})",
            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
        ));
    }
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
    let attempt_digest = resolution
        .snapshot
        .content_digest
        .clone()
        .unwrap_or_default();
    let journal_digest = journal_entry
        .as_ref()
        .map(|entry| entry.entry_digest.clone())
        .unwrap_or_default();
    AttemptAccountingV1 {
        schema_version: ACCOUNTING_SCHEMA_VERSION.to_string(),
        accounting_id: format!("acct/{}/{}/{}", context.graph_id, context.node_id, step_id),
        graph_id: context.graph_id.clone(),
        node_id: context.node_id.clone(),
        idempotency_key: step_id.to_string(),
        attempt_digest,
        journal_digest,
        harness_id: resolution.snapshot.selected.harness_id.clone(),
        harness_revision: snapshot_revision(resolution),
        provider_identity: resolution.snapshot.selected.provider_identity.clone(),
        model: snapshot_model(resolution),
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
    let usage = parse_reported_usage(&outcome.outcome.output)
        .map_err(AccountingError::InconsistentProviderReport)?;
    let model = snapshot_model(resolution);
    let (provider, _) = (resolution.snapshot.selected.provider_identity.as_str(), ());
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
    // Defensive re-verification of the attempt the record binds to: if the
    // caller hands in a structurally valid but tampered attempt, we refuse
    // to fabricate a binding instead of refusing it.
    outcome
        .attempt
        .verify()
        .map_err(|error| AccountingError::Internal(format!("attempt does not verify: {error}")))?;

    AttemptAccountingV1 {
        schema_version: ACCOUNTING_SCHEMA_VERSION.to_string(),
        accounting_id: format!(
            "acct/{}/{}/{}",
            context.graph_id, context.node_id, outcome.attempt.idempotency_key
        ),
        graph_id: context.graph_id.clone(),
        node_id: context.node_id.clone(),
        idempotency_key: outcome.attempt.idempotency_key.clone(),
        attempt_digest: outcome.attempt.content_digest.clone().unwrap_or_default(),
        journal_digest: outcome.attempt.journal_entry.entry_digest.clone(),
        harness_id: outcome.attempt.harness_id.clone(),
        harness_revision: resolution.snapshot.selected.harness_revision.clone(),
        provider_identity: outcome.attempt.provider_identity.clone(),
        model,
        route_set_id: resolution.snapshot.route_set_id.clone(),
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

fn snapshot_model(resolution: &RouteResolution) -> Option<String> {
    resolution.snapshot.selected.model.clone()
}

fn snapshot_revision(resolution: &RouteResolution) -> String {
    resolution.snapshot.selected.harness_revision.clone()
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
    /// An accounting fact cannot be sealed/digested (never fabricated).
    Internal(String),
}

impl std::fmt::Display for AccountingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountingError::InconsistentProviderReport(reason) => {
                write!(f, "provider usage report refused: {reason}")
            }
            AccountingError::Internal(reason) => write!(f, "internal error: {reason}"),
        }
    }
}
impl std::error::Error for AccountingError {}

// ---------------------------------------------------------------------------
// Aggregation: per-node, per-graph, with honest coverage semantics
// ---------------------------------------------------------------------------

/// Aggregated token usage across attempts. `complete` is true only when
/// every contributing attempt reported total usage; otherwise the totals
/// are partial and `unknown_attempts` / `partial_attempts` say so.
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
/// together; `mixed_currencies` lists every currency that contributed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CostAggregate {
    pub currencies: BTreeMap<String, u128>,
    pub known_attempts: BTreeMap<String, u64>,
    pub unknown_attempts: u64,
    pub mixed_currencies: Vec<String>,
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

/// Strict coverage arithmetic — nothing is silently zeroed.
fn add_usage(target: &mut UsageAggregate, u: &TokenUsage) {
    if let Some(p) = u.prompt_tokens {
        target.prompt_tokens = target.prompt_tokens.saturating_add(p);
    }
    if let Some(c) = u.completion_tokens {
        target.completion_tokens = target.completion_tokens.saturating_add(c);
    }
    if let Some(t) = u.total_tokens {
        target.total_tokens = target.total_tokens.saturating_add(t);
    }
}

fn usage_complete(u: &TokenUsage) -> bool {
    u.prompt_tokens.is_some() && u.completion_tokens.is_some() && u.total_tokens.is_some()
}

fn aggregate_usage(records: &[&AttemptAccountingV1]) -> UsageAggregate {
    let mut agg = UsageAggregate::default();
    for record in records {
        match &record.usage {
            UsageAccounting::Known(usage) => {
                add_usage(&mut agg, usage);
                if usage_complete(usage) {
                    agg.known_attempts += 1;
                } else {
                    agg.partial_attempts += 1;
                }
            }
            UsageAccounting::Unknown { .. } => agg.unknown_attempts += 1,
        }
    }
    agg.complete = records.iter().all(|r| match &r.usage {
        UsageAccounting::Known(u) => usage_complete(u),
        UsageAccounting::Unknown { .. } => false,
    });
    agg
}

fn aggregate_cost(records: &[&AttemptAccountingV1]) -> CostAggregate {
    let mut agg = CostAggregate::default();
    for record in records {
        match &record.cost {
            CostAccounting::Known(computation) => {
                let entry = agg
                    .currencies
                    .entry(computation.currency.clone())
                    .or_insert(0);
                *entry = entry.saturating_add(computation.minor_units as u128);
                *agg.known_attempts
                    .entry(computation.currency.clone())
                    .or_insert(0) += 1;
            }
            CostAccounting::Unknown { .. } => agg.unknown_attempts += 1,
        }
    }
    // Mixed-currency buckets: every contributing currency is reported so no
    // caller sums unrelated units together.
    for currency in agg.currencies.keys() {
        agg.mixed_currencies.push(currency.clone());
    }
    agg
}

fn outcome_counts(records: &[&AttemptAccountingV1]) -> (u64, u64, u64, u64) {
    let mut attempts = 0u64;
    let mut completed = 0u64;
    let mut failed = 0u64;
    let mut cancelled = 0u64;
    for record in records {
        attempts += 1;
        match &record.outcome {
            AttemptOutcomeAccounting::Completed => completed += 1,
            AttemptOutcomeAccounting::Failed { .. } => failed += 1,
            AttemptOutcomeAccounting::Cancelled { .. } => cancelled += 1,
        }
    }
    (attempts, completed, failed, cancelled)
}

fn aggregate_records(records: Vec<&AttemptAccountingV1>) -> NodeAccounting {
    let node_id = records
        .first()
        .map(|r| r.node_id.clone())
        .unwrap_or_default();
    let (attempts, completed, failed, cancelled) = outcome_counts(&records);
    let usage = aggregate_usage(&records);
    let cost = aggregate_cost(&records);
    let latency_ms_total = records
        .iter()
        .fold(0u64, |acc, r| acc.saturating_add(r.latency_ms));
    let latency_ms_max = records.iter().map(|r| r.latency_ms).max().unwrap_or(0);
    NodeAccounting {
        node_id,
        attempts,
        completed,
        failed,
        cancelled,
        usage,
        cost,
        latency_ms_total,
        latency_ms_max,
    }
}

/// A bounded in-memory ledger with journal-bound idempotency. Replay of the
/// same (key, journalDigest) is a no-op; the same key with a different
/// digest is a hard conflict and nothing is recorded.
#[derive(Debug, Default)]
pub struct AccountingLedger {
    records: Vec<AttemptAccountingV1>,
    /// idempotency key -> journal digest.
    bindings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// Record accepted and appended.
    Recorded,
    /// Same (key, journalDigest) recorded before; nothing changed.
    Duplicate,
}

#[derive(Debug)]
pub enum LedgerError {
    /// Same idempotency key bound to a different journal digest.
    IdempotencyConflict {
        key: String,
        existing_digest: String,
        incoming_digest: String,
    },
    Internal(String),
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerError::IdempotencyConflict {
                key,
                existing_digest,
                incoming_digest,
            } => write!(
                f,
                "idempotency key {key:?} already bound to journal digest {existing_digest}; refusing to bind it to {incoming_digest}"
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

    pub fn record(&mut self, record: AttemptAccountingV1) -> Result<InsertOutcome, LedgerError> {
        record
            .verify()
            .map_err(|error| LedgerError::Internal(error.to_string()))?;
        match self.bindings.get(&record.idempotency_key) {
            Some(existing) if *existing == record.journal_digest => Ok(InsertOutcome::Duplicate),
            Some(existing) => Err(LedgerError::IdempotencyConflict {
                key: record.idempotency_key.clone(),
                existing_digest: existing.clone(),
                incoming_digest: record.journal_digest.clone(),
            }),
            None => {
                self.bindings.insert(
                    record.idempotency_key.clone(),
                    record.journal_digest.clone(),
                );
                self.records.push(record);
                Ok(InsertOutcome::Recorded)
            }
        }
    }

    pub fn records(&self) -> &[AttemptAccountingV1] {
        &self.records
    }

    pub fn aggregate_graph(&self, graph_id: &str) -> GraphAccounting {
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
        let node_list: Vec<NodeAccounting> = nodes.into_values().map(aggregate_records).collect();
        let (attempts, completed, failed, cancelled) = outcome_counts(&records);
        let usage = aggregate_usage(&records);
        let cost = aggregate_cost(&records);
        let latency_ms_total = records
            .iter()
            .fold(0u64, |acc, r| acc.saturating_add(r.latency_ms));
        let latency_ms_max = records.iter().map(|r| r.latency_ms).max().unwrap_or(0);
        GraphAccounting {
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
        }
    }

    pub fn over_budget_records(&self) -> Vec<&AttemptAccountingV1> {
        self.records
            .iter()
            .filter(|r| r.over_budget.is_some())
            .collect()
    }
}
