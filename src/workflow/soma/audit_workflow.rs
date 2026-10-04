//! `WorkflowDefinition` semantic audit producing the published SOMA
//! diagnostics. Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-validate/src/workflow.rs).

use std::collections::{BTreeMap, BTreeSet};

use super::contracts::{
    BodyItem, ConstraintKind, EffectExport, GovernanceConstraint, OperationDefinition,
    PortDefinition, WorkflowContext, WorkflowDefinition,
};
use super::types::{FAILURE_VARIANTS, OutcomeVariant, SUCCESS_VARIANT, type_in_vocabulary};
use super::{Diagnostic, SupportedVersion};

/// Root authority ceiling shared by every scope (the workflow's authority
/// profile is the only ceiling — composites declare grants, never ceilings).
struct Ceilings<'a> {
    tool_keys: BTreeSet<&'a str>,
    readable: BTreeSet<&'a str>,
    writable: BTreeSet<&'a str>,
    secret_names: BTreeSet<&'a str>,
}

/// One validation scope: the root workflow body or one composite's body.
struct ScopeEnv<'a> {
    /// Items of this scope (operations and containers).
    items: &'a [BodyItem],
    /// RFC 6901 prefix ending in `/body` for this scope (root `"/body"`,
    /// nested `"/body/1/body"`). Owned: cloned per recursion level; depth
    /// is document-bounded, the simplest borrow-safe form.
    ptr_prefix: String,
    /// Declared ports of this scope (root: workflow ports; nested: container ports).
    input_ports: &'a [PortDefinition],
    output_ports: &'a [PortDefinition],
    /// `SOMA-EXP-0007` effect gate: root uses `workflow.effectExports`,
    /// nested scopes use their container's (key-presence gated).
    effect_exports: Option<&'a Vec<EffectExport>>,
    /// `SOMA-EXP-0007` context gate: root uses `workflow.context`,
    /// nested scopes use their container's (discloses-presence gated).
    context: Option<&'a WorkflowContext>,
    /// Constraints evaluated for this scope (root's / container's).
    constraints: &'a [GovernanceConstraint],
    /// AUTH-0002 active iff this scope's owner is composite
    /// (root: `workflow.is_composite()`; nested: always).
    is_composite: bool,
    /// The scope owner's `authorityImports` (root / container).
    authority_imports: &'a [String],
}

impl WorkflowDefinition {
    pub fn is_composite(&self) -> bool {
        self.kind == Some(super::types::WorkflowKind::Composite)
    }

    /// Full semantic audit. Returns diagnostics in stable (sorted) order;
    /// empty means the workflow satisfies every check.
    ///
    /// E4/X07 Slice 2: root-only checks run once here, per-scope checks run
    /// in [`audit_scope`] for the root body and for every nested composite
    /// body, and document-wide checks run over ids collected at any depth.
    pub fn audit(&self, supported: &SupportedVersion) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = Vec::new();

        // ROOT-ONLY — SOMA-CMP-0001: unsupported versions.
        for v in [&self.schema_version, &self.version] {
            if let Ok(parsed) = super::types::SemVer::parse(v)
                && !parsed.is_compatible_with(supported)
            {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0001",
                    format!("artifact version {v} newer than bundle"),
                ));
            }
        }

        // ROOT-ONLY — SOMA-CMP-0004: the declared content digest must equal
        // the canonical digest of this artifact with the digest field itself
        // excluded. serde serialization is recursive, so this digest already
        // covers nested bytes.
        if let Some(declared) = &self.content_digest {
            match serde_json::to_value(self) {
                Err(_) => out.push(Diagnostic::new(
                    "SOMA-CMP-0004",
                    "artifact cannot be serialized for digest verification",
                )),
                Ok(mut value) => {
                    if let Some(obj) = value.as_object_mut() {
                        obj.remove("contentDigest");
                    }
                    // Fail closed: an uncomputable digest can never verify —
                    // report it under the same code, never substitute a value.
                    match super::canonical::try_canonical_digest(&value) {
                        Ok(computed) => {
                            if computed != declared.as_str() {
                                out.push(Diagnostic::new(
                                    "SOMA-CMP-0004",
                                    "declared contentDigest does not verify",
                                ));
                            }
                        }
                        Err(e) => {
                            out.push(Diagnostic::new(
                                "SOMA-CMP-0004",
                                format!("contentDigest cannot be recomputed ({e})"),
                            ));
                        }
                    }
                }
            }
        }

        let authority = &self.authority;

        // ROOT-ONLY — SOMA-AUTH-0010
        if authority
            .abstention
            .as_ref()
            .and_then(|a| a.behavior.as_deref())
            == Some("abstain")
            && !authority.has_recovery_path()
        {
            out.push(Diagnostic::new(
                "SOMA-AUTH-0010",
                "abstained review with no re-approval path",
            ));
        }

        // ROOT-ONLY — SOMA-AUTH-0004
        if let Some(restrictions) = &authority.content_restrictions {
            let providers = authority
                .provider_policy
                .iter()
                .flat_map(|p| p.allowlist.iter().flatten());
            let network = authority
                .network_policy
                .iter()
                .flat_map(|n| n.allowlist.iter().flatten());
            let allowed: BTreeSet<&str> = providers.chain(network).map(String::as_str).collect();
            for r in restrictions {
                if !allowed.contains(r.to.as_str()) {
                    out.push(Diagnostic::new(
                        "SOMA-AUTH-0004",
                        format!("restricted content routed to prohibited provider {}", r.to),
                    ));
                }
            }
        }

        let ceilings = Ceilings {
            tool_keys: self.authority.tool_keys().into_iter().collect(),
            readable: self
                .authority
                .readable_scopes
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
            writable: self
                .authority
                .writable_scopes
                .iter()
                .flatten()
                .map(String::as_str)
                .collect(),
            secret_names: self.authority.declared_secret_names().into_iter().collect(),
        };
        let root_env = ScopeEnv {
            items: &self.body,
            ptr_prefix: "/body".to_string(),
            input_ports: &self.input_ports,
            output_ports: &self.output_ports,
            effect_exports: self.effect_exports.as_ref(),
            context: self.context.as_ref(),
            constraints: &self.constraints,
            is_composite: self.is_composite(),
            authority_imports: &self.authority_imports,
        };
        audit_scope(self, &root_env, &ceilings, &mut out);

        // DOCUMENT-WIDE — recursive id collection over ops AND containers,
        // at any depth.
        let mut doc_ids: Vec<String> = Vec::new();
        collect_body_ids(&self.body, &mut doc_ids);
        let ids: Vec<&str> = doc_ids.iter().map(String::as_str).collect();

        // SOMA-CMP-0002 — references resolve against {own id} ∪ document ids.
        {
            let known: BTreeSet<&str> = std::iter::once(self.id.as_str())
                .chain(ids.iter().copied())
                .collect();
            for r in &self.references {
                if !known.contains(r.as_str()) {
                    out.push(Diagnostic::new(
                        "SOMA-CMP-0002",
                        format!("unknown reference {r}"),
                    ));
                }
            }
        }

        // SOMA-EXP-0001 — self-containment, document-wide.
        if self.is_composite() && ids.contains(&self.id.as_str()) {
            out.push(Diagnostic::new(
                "SOMA-EXP-0001",
                "transitive self-containment",
            ));
        }

        // SOMA-EXP-0003 — duplicate body-item ids at any depth. The message
        // keeps its flat-workflow wording (the code is what gates the
        // composite case; the invalid fixture suite pins the bytes).
        {
            let dupes: BTreeSet<&str> = ids
                .iter()
                .filter(|id| ids.iter().filter(|o| *o == *id).count() > 1)
                .copied()
                .collect();
            for d in dupes {
                out.push(Diagnostic::new(
                    "SOMA-EXP-0003",
                    format!("duplicate operation id {d}"),
                ));
            }
        }

        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }
}

/// Document-wide id collection over operations AND containers, at any depth.
fn collect_body_ids(items: &[BodyItem], acc: &mut Vec<String>) {
    for item in items {
        acc.push(item.id().to_string());
        if let BodyItem::Composite(c) = item {
            collect_body_ids(&c.body, acc);
        }
    }
}

/// Validate one scope — the root workflow body or one composite body — under
/// the root authority ceiling, then recurse into nested composite bodies.
/// Each block keeps its flat-document diagnostics verbatim; only the data
/// source becomes scope-local.
fn audit_scope(
    wf: &WorkflowDefinition,
    env: &ScopeEnv<'_>,
    ceilings: &Ceilings<'_>,
    out: &mut Vec<Diagnostic>,
) {
    let authority = &wf.authority;

    // (1) Per-unit loop: authority grants, uses/secrets, effects/context.
    for (body_index, item) in env.items.iter().enumerate() {
        let BodyItem::Operation(unit) = item else {
            continue;
        };
        let ptr = format!("{}/{}", env.ptr_prefix, body_index);
        audit_unit_authority(
            ceilings,
            unit,
            &ptr,
            env.is_composite,
            env.authority_imports,
            out,
        );
        // SOMA-AUTH-0005 / 0006 / 0007 / 0008 / EXP-0007
        for cap in &unit.uses {
            if !ceilings.tool_keys.is_empty() && !ceilings.tool_keys.contains(cap.as_str()) {
                out.push(
                    Diagnostic::related(
                        "SOMA-AUTH-0005",
                        "operation outside allowed set",
                        unit.id.clone(),
                    )
                    .with_source(ptr.clone(), Some(unit.id.clone())),
                );
            }
        }
        for sec in &unit.secrets {
            if !ceilings.secret_names.contains(sec.as_str()) {
                out.push(
                    Diagnostic::related(
                        "SOMA-AUTH-0006",
                        "secret outside declared policy",
                        unit.id.clone(),
                    )
                    .with_source(ptr.clone(), Some(unit.id.clone())),
                );
            }
        }
        for eff in &unit.effects {
            if eff.review.unwrap_or(false)
                && authority.review.as_ref().and_then(|r| r.effect.as_deref())
                    != Some(eff.name.as_str())
            {
                let subject = format!("{}/{}", unit.id, eff.name);
                out.push(
                    Diagnostic::related(
                        "SOMA-AUTH-0007",
                        "review-gated effect without covering review gate",
                        subject.clone(),
                    )
                    .with_source(ptr.clone(), Some(subject)),
                );
            }
            if eff.irreversible.unwrap_or(false)
                && authority.mutation != super::types::MutationMode::Explicit
                && !authority.has_recovery_path()
            {
                let subject = format!("{}/{}", unit.id, eff.name);
                out.push(
                    Diagnostic::related(
                        "SOMA-AUTH-0008",
                        "irreversible effect without explicit mutation or recovery",
                        subject.clone(),
                    )
                    .with_source(ptr.clone(), Some(subject)),
                );
            }
            // SOMA-EXP-0007 (effects), gated on KEY PRESENCE of this scope's
            // `effectExports`.
            if let Some(exports) = env.effect_exports
                && !exports.iter().any(|e| e.name == eff.name)
            {
                let subject = format!("{}/{}", unit.id, eff.name);
                out.push(
                    Diagnostic::related(
                        "SOMA-EXP-0007",
                        "undeclared effect crossing",
                        subject.clone(),
                    )
                    .with_source(ptr.clone(), Some(subject)),
                );
            }
        }
        for c in &unit.context {
            if let Some(ctx) = env.context {
                // Gate on key PRESENCE (discloses is Some), not on
                // non-empty list.
                if ctx.discloses.is_some() && !ctx.discloses.as_ref().unwrap().contains(c) {
                    let subject = format!("{}/{}", unit.id, c);
                    out.push(
                        Diagnostic::related(
                            "SOMA-EXP-0007",
                            "undeclared context crossing",
                            subject.clone(),
                        )
                        .with_source(ptr.clone(), Some(subject)),
                    );
                }
            }
        }
    }

    // (2) SOMA-CMP-0005 — this scope's boundary types vs each item's ports
    // (operations check `inputs`/`outputs`; containers check
    // `inputPorts`/`outputPorts`).
    let in_types: BTreeMap<&str, &str> = env
        .input_ports
        .iter()
        .map(|p| (p.name.as_str(), p.ty.as_str()))
        .collect();
    let out_types: BTreeMap<&str, &str> = env
        .output_ports
        .iter()
        .map(|p| (p.name.as_str(), p.ty.as_str()))
        .collect();
    for (body_index, item) in env.items.iter().enumerate() {
        let ptr = format!("{}/{}", env.ptr_prefix, body_index);
        let id = item.id().to_string();
        match item {
            BodyItem::Operation(unit) => {
                for inp in &unit.inputs {
                    if let Some(t) = in_types.get(inp.name.as_str())
                        && *t != inp.ty
                    {
                        out.push(
                            Diagnostic::related(
                                "SOMA-CMP-0005",
                                "port/edge type mismatch",
                                id.clone(),
                            )
                            .with_source(ptr.clone(), Some(id.clone())),
                        );
                    }
                }
                for o in &unit.outputs {
                    if let Some(t) = out_types.get(o.name.as_str())
                        && *t != o.ty
                    {
                        out.push(
                            Diagnostic::related(
                                "SOMA-CMP-0005",
                                "port/edge type mismatch",
                                id.clone(),
                            )
                            .with_source(ptr.clone(), Some(id.clone())),
                        );
                    }
                }
            }
            BodyItem::Composite(c) => {
                for p in &c.input_ports {
                    if let Some(t) = in_types.get(p.name.as_str())
                        && *t != p.ty
                    {
                        out.push(
                            Diagnostic::related(
                                "SOMA-CMP-0005",
                                "port/edge type mismatch",
                                id.clone(),
                            )
                            .with_source(ptr.clone(), Some(id.clone())),
                        );
                    }
                }
                for p in &c.output_ports {
                    if let Some(t) = out_types.get(p.name.as_str())
                        && *t != p.ty
                    {
                        out.push(
                            Diagnostic::related(
                                "SOMA-CMP-0005",
                                "port/edge type mismatch",
                                id.clone(),
                            )
                            .with_source(ptr.clone(), Some(id.clone())),
                        );
                    }
                }
            }
        }
    }

    // (3) SOMA-CMP-0006 vocabulary — this scope's boundary ports plus every
    // scope item's port types (op inputs/outputs; container ports).
    let mut check_type = |t: &str| {
        if !type_in_vocabulary(t) {
            out.push(Diagnostic::new(
                "SOMA-CMP-0006",
                format!("type {t:?} outside vocabulary"),
            ));
        }
    };
    for t in env
        .input_ports
        .iter()
        .chain(env.output_ports.iter())
        .map(|p| p.ty.as_str())
    {
        check_type(t);
    }
    for item in env.items {
        match item {
            BodyItem::Operation(unit) => {
                for t in unit
                    .inputs
                    .iter()
                    .map(|i| i.ty.as_str())
                    .chain(unit.outputs.iter().map(|o| o.ty.as_str()))
                {
                    check_type(t);
                }
            }
            BodyItem::Composite(c) => {
                for t in c
                    .input_ports
                    .iter()
                    .map(|p| p.ty.as_str())
                    .chain(c.output_ports.iter().map(|p| p.ty.as_str()))
                {
                    check_type(t);
                }
            }
        }
    }

    // (4) Scope-local dependency helpers: producers also include container
    // output ports (container outputs feed parent-scope consumers);
    // consumed also includes container input ports.
    let mut producers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut consumed: BTreeSet<&str> = BTreeSet::new();
    for item in env.items {
        match item {
            BodyItem::Operation(unit) => {
                for o in &unit.outputs {
                    producers
                        .entry(o.name.as_str())
                        .or_default()
                        .insert(unit.id.as_str());
                }
                consumed.extend(unit.inputs.iter().map(|i| i.name.as_str()));
            }
            BodyItem::Composite(c) => {
                for o in &c.output_ports {
                    producers
                        .entry(o.name.as_str())
                        .or_default()
                        .insert(c.id.as_str());
                }
                consumed.extend(c.input_ports.iter().map(|p| p.name.as_str()));
            }
        }
    }
    let boundary_out: BTreeSet<&str> = env.output_ports.iter().map(|p| p.name.as_str()).collect();
    let boundary_in: BTreeSet<&str> = env.input_ports.iter().map(|p| p.name.as_str()).collect();

    // (5) SOMA-EXP-0006: required input shadowed by optional output port.
    for p in env.input_ports {
        if p.is_required()
            && env
                .output_ports
                .iter()
                .any(|o| o.name == p.name && !o.is_required())
        {
            out.push(Diagnostic::new(
                "SOMA-EXP-0006",
                format!("required input '{}' fed by optional output", p.name),
            ));
        }
    }

    // (6) SOMA-EXP-0005: dead inputs — operations check `inputs`,
    // containers check `inputPorts` (a boundary input must be fed by a
    // producer or the scope's own boundary).
    for (body_index, item) in env.items.iter().enumerate() {
        let ptr = format!("{}/{}", env.ptr_prefix, body_index);
        match item {
            BodyItem::Operation(unit) => {
                for inp in &unit.inputs {
                    if !producers.contains_key(inp.name.as_str())
                        && !boundary_in.contains(inp.name.as_str())
                    {
                        let subject = format!("{}/{}", unit.id, inp.name);
                        out.push(
                            Diagnostic::related(
                                "SOMA-EXP-0005",
                                "required input not fed/defaulted",
                                subject.clone(),
                            )
                            .with_source(ptr.clone(), Some(subject)),
                        );
                    }
                }
            }
            BodyItem::Composite(c) => {
                for p in &c.input_ports {
                    if !producers.contains_key(p.name.as_str())
                        && !boundary_in.contains(p.name.as_str())
                    {
                        let subject = format!("{}/{}", c.id, p.name);
                        out.push(
                            Diagnostic::related(
                                "SOMA-EXP-0005",
                                "required input not fed/defaulted",
                                subject.clone(),
                            )
                            .with_source(ptr.clone(), Some(subject)),
                        );
                    }
                }
            }
        }
    }

    // (7) SOMA-EXP-0004: orphan operations. Containers are never orphans —
    // they are declared boundary structure, and `consumed` already includes
    // their input ports.
    for (body_index, item) in env.items.iter().enumerate() {
        let BodyItem::Operation(unit) = item else {
            continue;
        };
        if unit.inputs.is_empty() {
            let produced: BTreeSet<&str> = unit.outputs.iter().map(|o| o.name.as_str()).collect();
            if produced.is_disjoint(&consumed) && produced.is_disjoint(&boundary_out) {
                out.push(
                    Diagnostic::related(
                        "SOMA-EXP-0004",
                        "operation unreachable from any boundary",
                        unit.id.clone(),
                    )
                    .with_source(
                        format!("{}/{}", env.ptr_prefix, body_index),
                        Some(unit.id.clone()),
                    ),
                );
            }
        }
    }

    // (8) SOMA-EXP-0002: dataflow cycle in this scope; `has_cycle` reads
    // each item's consumer names (op `inputs` / container `inputPorts`).
    if has_cycle(env.items, &producers) {
        out.push(Diagnostic::new("SOMA-EXP-0002", "operation-edge cycle"));
    }

    // (9) SOMA-OUT-0001 / 0002 — operations consume; the producer side
    // scans same-scope OPERATION outputs only (container ports declare no
    // `emits`; the `emitted.is_empty()` skip keeps parity).
    for (body_index, item) in env.items.iter().enumerate() {
        let BodyItem::Operation(unit) = item else {
            continue;
        };
        let ptr = format!("{}/{}", env.ptr_prefix, body_index);
        for inp in &unit.inputs {
            let accepted: BTreeSet<OutcomeVariant> =
                inp.accepted_outcomes.iter().copied().collect();
            let emitted: BTreeSet<OutcomeVariant> = env
                .items
                .iter()
                .filter_map(BodyItem::as_operation)
                .flat_map(|src| src.outputs.iter())
                .filter(|o| o.name == inp.name)
                .flat_map(|o| o.emits.iter().flatten())
                .copied()
                .collect();
            if emitted.is_empty() {
                continue;
            }
            let failure_into_success = emitted.iter().any(|v| FAILURE_VARIANTS.contains(v))
                && !accepted.is_empty()
                && accepted == BTreeSet::from([SUCCESS_VARIANT]);
            if failure_into_success {
                let subject = format!("{}/{}", unit.id, inp.name);
                out.push(
                    Diagnostic::related(
                        "SOMA-OUT-0001",
                        "failure-like outcome coerced to success",
                        subject.clone(),
                    )
                    .with_source(ptr.clone(), Some(subject)),
                );
            } else if emitted.iter().any(|v| !accepted.contains(v)) {
                let subject = format!("{}/{}", unit.id, inp.name);
                out.push(
                    Diagnostic::related(
                        "SOMA-OUT-0002",
                        "upstream outcome not in accept set",
                        subject.clone(),
                    )
                    .with_source(ptr.clone(), Some(subject)),
                );
            }
        }
    }

    // (10) Governance constraints scoped to this body's ids.
    let scope_ids: Vec<&str> = env.items.iter().map(BodyItem::id).collect();
    out.extend(audit_governance_scope(wf, env.constraints, scope_ids));

    // (11) Recursion: each nested composite body is its own scope.
    for (i, item) in env.items.iter().enumerate() {
        let BodyItem::Composite(c) = item else {
            continue;
        };
        let child = ScopeEnv {
            items: &c.body,
            ptr_prefix: format!("{}/{}/body", env.ptr_prefix, i),
            input_ports: &c.input_ports,
            output_ports: &c.output_ports,
            effect_exports: c.effect_exports.as_ref(),
            context: c.context.as_ref(),
            constraints: &c.constraints,
            is_composite: true,
            authority_imports: &c.authority_imports,
        };
        audit_scope(wf, &child, ceilings, out);
    }
}

/// SOMA-AUTH-0001 / 0002 / 0003 over one body unit's authority grants.
///
/// `ptr` is the unit's RFC 6901 pointer within its scope (root
/// `/body/<i>`, nested `/body/<i>/body/<j>`), used to anchor each
/// diagnostic.
fn audit_unit_authority(
    ceilings: &Ceilings<'_>,
    unit: &OperationDefinition,
    ptr: &str,
    is_composite: bool,
    authority_imports: &[String],
    out: &mut Vec<Diagnostic>,
) {
    let imported: BTreeSet<&str> = authority_imports.iter().map(String::as_str).collect();
    for grant in &unit.authority {
        match grant.split_once(':') {
            Some(("readable", scope)) => {
                if !ceilings.readable.contains(scope) {
                    out.push(
                        Diagnostic::related(
                            "SOMA-AUTH-0003",
                            "readable scope not declared",
                            unit.id.clone(),
                        )
                        .with_source(ptr, Some(unit.id.clone())),
                    );
                }
            }
            Some(("writable", scope)) => {
                if !ceilings.writable.contains(scope) {
                    out.push(
                        Diagnostic::related(
                            "SOMA-AUTH-0003",
                            "writable scope not declared",
                            unit.id.clone(),
                        )
                        .with_source(ptr, Some(unit.id.clone())),
                    );
                }
            }
            _ => {
                if !ceilings.tool_keys.contains(grant.as_str()) {
                    out.push(
                        Diagnostic::related(
                            "SOMA-AUTH-0001",
                            "capability used but not granted",
                            unit.id.clone(),
                        )
                        .with_source(ptr, Some(unit.id.clone())),
                    );
                }
            }
        }
        // SOMA-AUTH-0002 (composite): flags ANY unit authority entry outside
        // the import set — including scope-prefixed grants; the prefix
        // exemption exists only in AUTH-0001. Active iff this scope's owner
        // is composite.
        if is_composite && !imported.contains(grant.as_str()) {
            out.push(
                Diagnostic::related(
                    "SOMA-AUTH-0002",
                    "composite exceeding imported authority",
                    unit.id.clone(),
                )
                .with_source(ptr, Some(unit.id.clone())),
            );
        }
    }
}

fn has_cycle(body: &[BodyItem], producers: &BTreeMap<&str, BTreeSet<&str>>) -> bool {
    // Kahn's algorithm over INDEX-keyed dependency edges so id-sharing units
    // never collapse into one node.
    let n = body.len();
    let mut indegree = vec![0usize; n];
    let mut edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for (i, item) in body.iter().enumerate() {
        let mut deps: BTreeSet<usize> = BTreeSet::new();
        match item {
            BodyItem::Operation(unit) => {
                for inp in &unit.inputs {
                    if let Some(producers_of_name) = producers.get(inp.name.as_str()) {
                        for (j, other) in body.iter().enumerate() {
                            if producers_of_name.contains(&other.id()) {
                                deps.insert(j);
                            }
                        }
                    }
                }
            }
            BodyItem::Composite(c) => {
                for p in &c.input_ports {
                    if let Some(producers_of_name) = producers.get(p.name.as_str()) {
                        for (j, other) in body.iter().enumerate() {
                            if producers_of_name.contains(&other.id()) {
                                deps.insert(j);
                            }
                        }
                    }
                }
            }
        }
        indegree[i] = deps.len();
        for dep in deps {
            edges[dep].insert(i);
        }
    }
    let mut queue: Vec<usize> = (0..n).filter(|i| indegree[*i] == 0).collect();
    let mut visited = 0usize;
    while let Some(node) = queue.pop() {
        visited += 1;
        for nxt in edges[node].clone() {
            indegree[nxt] -= 1;
            if indegree[nxt] == 0 {
                queue.push(nxt);
            }
        }
    }
    visited < n
}

// ---------------------------------------------------------------------------
// Governance predicate grammar
// ---------------------------------------------------------------------------

const GOV_LIST_FIELDS: [&str; 6] = [
    "readableScopes",
    "writableScopes",
    "networkPolicy.allowlist",
    "providerPolicy.allowlist",
    "secrets.names",
    "tools.keys",
];

const GOV_OPS: [&str; 5] = ["equals", "subset", "superset", "intersects", "excludes"];

fn gov_field_value(wf: &WorkflowDefinition, field: &str) -> Vec<String> {
    let auth = &wf.authority;
    match field {
        // An absent dimension defaults to `or []` and DECIDES — an absent
        // dimension never makes a constraint undecidable.
        "readableScopes" => auth.readable_scopes.clone().unwrap_or_default(),
        "writableScopes" => auth.writable_scopes.clone().unwrap_or_default(),
        "networkPolicy.allowlist" => auth
            .network_policy
            .as_ref()
            .and_then(|n| n.allowlist.clone())
            .unwrap_or_default(),
        "providerPolicy.allowlist" => auth
            .provider_policy
            .as_ref()
            .and_then(|p| p.allowlist.clone())
            .unwrap_or_default(),
        "secrets.names" => auth
            .declared_secret_names()
            .into_iter()
            .map(String::from)
            .collect(),
        "tools.keys" => auth.tool_keys().into_iter().map(String::from).collect(),
        _ => Vec::new(),
    }
}

enum GovValue {
    Set(BTreeSet<String>),
    /// Scalar with the value as canonical lexeme; `None` = field absent.
    /// `numeric` marks number-domain fields (budget.*): a STRING argument can
    /// never equal a numeric value.
    Scalar {
        numeric: bool,
        val: Option<String>,
    },
}

impl WorkflowDefinition {
    fn gov_value(&self, field: &str) -> GovValue {
        match field {
            "networkPolicy.default" => GovValue::Scalar {
                numeric: false,
                val: self
                    .authority
                    .network_policy
                    .as_ref()
                    .map(|n| n.default.as_str().to_string()),
            },
            f if f.starts_with("budget.") => {
                let dim = &f["budget.".len()..];
                let v = self
                    .authority
                    .budgets
                    .as_ref()
                    .and_then(|b| b.get(dim))
                    .and_then(super::numeric_lexeme);
                GovValue::Scalar {
                    numeric: true,
                    val: v,
                }
            }
            list => GovValue::Set(gov_field_value(self, list).into_iter().collect()),
        }
    }
}

fn parse_predicate(p: &str) -> Option<(String, String, serde_json::Value)> {
    let mut parts = p.splitn(3, ' ');
    let field = parts.next()?;
    let op = parts.next()?;
    let raw = parts.next()?;
    if !GOV_OPS.contains(&op) {
        return None;
    }
    let known = GOV_LIST_FIELDS.contains(&field)
        || field == "networkPolicy.default"
        || field.starts_with("budget.");
    if !known {
        return None;
    }
    let arg: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some((field.to_string(), op.to_string(), arg))
}

fn evaluate_gov(
    wf: &WorkflowDefinition,
    constraint: &GovernanceConstraint,
    known_nodes: &BTreeSet<&str>,
) -> Option<bool> {
    let (field, op, arg) = parse_predicate(&constraint.predicate)?;
    // Dynamic constraints need an evaluation point to be decidable statically.
    if constraint.kind == ConstraintKind::Dynamic && constraint.evaluation_point.is_none() {
        return None;
    }
    // Subject must reference known nodes only (scope-local set supplied by
    // the caller).
    if let Some(nodes) = &constraint.subject.node_set
        && nodes.iter().any(|n| !known_nodes.contains(n.as_str()))
    {
        return None;
    }
    match wf.gov_value(&field) {
        GovValue::Scalar {
            numeric,
            val: actual,
        } => {
            if op != "equals" {
                return None;
            }
            match (&actual, &arg) {
                (None, _) if numeric => None, // absent budget -> undecidable
                (None, _) => Some(false),
                (Some(actual_lexeme), serde_json::Value::String(want)) => {
                    Some(!numeric && actual_lexeme == want)
                }
                (Some(actual_lexeme), serde_json::Value::Number(want)) => {
                    if !numeric {
                        return Some(false);
                    }
                    let want_lexeme = super::numeric_lexeme(want);
                    Some(actual_lexeme == want_lexeme.as_deref().unwrap_or_default())
                }
                _ => Some(false),
            }
        }
        GovValue::Set(actual) => {
            let arr = arg.as_array()?;
            if arr.iter().any(|x| !x.is_string()) {
                return None;
            }
            let arg_set: BTreeSet<String> = arr
                .iter()
                .filter_map(|x| x.as_str())
                .map(String::from)
                .collect();
            Some(match op.as_str() {
                "equals" => actual == arg_set,
                "subset" => actual.is_subset(&arg_set),
                "superset" => arg_set.is_subset(&actual),
                "intersects" => !actual.is_disjoint(&arg_set),
                "excludes" => actual.is_disjoint(&arg_set),
                _ => return None,
            })
        }
    }
}

/// Contradiction check between two same-field constraints.
fn contradiction(
    op_a: &str,
    arg_a: &serde_json::Value,
    set_a: &BTreeSet<String>,
    op_b: &str,
    arg_b: &serde_json::Value,
    set_b: &BTreeSet<String>,
) -> bool {
    if op_a == "equals" && op_b == "equals" {
        return arg_a != arg_b;
    }
    let pairs = [(op_a, op_b), (op_b, op_a)];
    if pairs.contains(&("subset", "superset")) && !set_b.is_subset(set_a) {
        return true;
    }
    if pairs.contains(&("subset", "intersects")) && set_a.is_disjoint(set_b) {
        return true;
    }
    if pairs.contains(&("superset", "excludes")) && !set_a.is_disjoint(set_b) {
        return true;
    }
    if pairs.contains(&("subset", "excludes")) && set_b.is_subset(set_a) {
        return true;
    }
    false
}

fn audit_governance_scope(
    wf: &WorkflowDefinition,
    constraints: &[GovernanceConstraint],
    scope_ids: Vec<&str>,
) -> Vec<Diagnostic> {
    let mut codes: BTreeSet<&'static str> = BTreeSet::new();
    let mut undecidable = false;
    let mut parsed: Vec<(String, String, serde_json::Value, BTreeSet<String>)> = Vec::new();
    let known_nodes: BTreeSet<&str> = scope_ids.into_iter().collect();

    for con in constraints {
        if con.kind == ConstraintKind::Dynamic && con.evaluation_point.is_none() {
            undecidable = true;
            continue;
        }
        if let Some(nodes) = &con.subject.node_set
            && nodes.iter().any(|n| !known_nodes.contains(n.as_str()))
        {
            undecidable = true;
            continue;
        }
        let Some((field, op, arg)) = parse_predicate(&con.predicate) else {
            undecidable = true;
            continue;
        };
        let is_list_field = GOV_LIST_FIELDS.contains(&field.as_str());
        if is_list_field && !arg.is_array() {
            undecidable = true;
            continue;
        }
        if !is_list_field && !arg.is_string() && !arg.is_number() {
            undecidable = true;
            continue;
        }
        match evaluate_gov(wf, con, &known_nodes) {
            Some(true) => {}
            Some(false) => {
                codes.insert("SOMA-GOV-0001");
            }
            None => {
                undecidable = true;
                continue;
            }
        }
        let set: BTreeSet<String> = arg
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        parsed.push((field, op, arg, set));
    }

    'outer: for i in 0..parsed.len() {
        for j in (i + 1)..parsed.len() {
            let (fa, oa, aa, sa) = &parsed[i];
            let (fb, ob, ab, sb) = &parsed[j];
            if fa == fb && contradiction(oa, aa, sa, ob, ab, sb) {
                codes.insert("SOMA-GOV-0002");
                break 'outer;
            }
        }
    }
    if codes.contains("SOMA-GOV-0002") {
        codes.remove("SOMA-GOV-0001"); // unsatisfiable set subsumes violations
    }
    if undecidable {
        codes.insert("SOMA-GOV-0003");
    }
    codes
        .into_iter()
        .map(|c| {
            Diagnostic::new(
                c,
                match c {
                    "SOMA-GOV-0001" => "governance constraint unsatisfied",
                    "SOMA-GOV-0002" => "declared constraint set unsatisfiable",
                    _ => "constraint satisfiability not decidable (fail closed)",
                },
            )
        })
        .collect()
}
