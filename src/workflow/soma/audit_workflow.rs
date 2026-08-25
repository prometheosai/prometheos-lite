//! `WorkflowDefinition` semantic audit producing the published SOMA
//! diagnostics. Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-validate/src/workflow.rs).

use std::collections::{BTreeMap, BTreeSet};

use super::contracts::{
    ConstraintKind, GovernanceConstraint, OperationDefinition, WorkflowDefinition,
};
use super::types::{FAILURE_VARIANTS, OutcomeVariant, SUCCESS_VARIANT, type_in_vocabulary};
use super::{Diagnostic, SupportedVersion};

impl WorkflowDefinition {
    pub fn is_composite(&self) -> bool {
        self.kind == Some(super::types::WorkflowKind::Composite)
    }

    fn op_ids(&self) -> Vec<&str> {
        self.body.iter().map(|u| u.id.as_str()).collect()
    }

    /// Full semantic audit. Returns diagnostics in stable (sorted) order;
    /// empty means the workflow satisfies every check.
    pub fn audit(&self, supported: &SupportedVersion) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = Vec::new();

        // SOMA-CMP-0001: unsupported versions.
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

        // SOMA-CMP-0004: the declared content digest must equal the
        // canonical digest of this artifact with the digest field itself
        // excluded.
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
                    let computed = super::canonical::canonical_digest(&value);
                    if computed != declared.as_str() {
                        out.push(Diagnostic::new(
                            "SOMA-CMP-0004",
                            "declared contentDigest does not verify",
                        ));
                    }
                }
            }
        }

        let authority = &self.authority;
        let tool_keys: BTreeSet<&str> = authority.tool_keys().into_iter().collect();
        let readable: BTreeSet<&str> = authority
            .readable_scopes
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        let writable: BTreeSet<&str> = authority
            .writable_scopes
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        let secret_names: BTreeSet<&str> = authority.declared_secret_names().into_iter().collect();

        for unit in &self.body {
            audit_unit_authority(self, unit, &tool_keys, &readable, &writable, &mut out);
            // SOMA-AUTH-0005 / 0006 / 0007 / 0008 / EXP-0007
            for cap in &unit.uses {
                if !tool_keys.is_empty() && !tool_keys.contains(cap.as_str()) {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0005",
                        "operation outside allowed set",
                        unit.id.clone(),
                    ));
                }
            }
            for sec in &unit.secrets {
                if !secret_names.contains(sec.as_str()) {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0006",
                        "secret outside declared policy",
                        unit.id.clone(),
                    ));
                }
            }
            for eff in &unit.effects {
                if eff.review.unwrap_or(false)
                    && authority.review.as_ref().and_then(|r| r.effect.as_deref())
                        != Some(eff.name.as_str())
                {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0007",
                        "review-gated effect without covering review gate",
                        format!("{}/{}", unit.id, eff.name),
                    ));
                }
                if eff.irreversible.unwrap_or(false)
                    && authority.mutation != super::types::MutationMode::Explicit
                    && !authority.has_recovery_path()
                {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0008",
                        "irreversible effect without explicit mutation or recovery",
                        format!("{}/{}", unit.id, eff.name),
                    ));
                }
                // SOMA-EXP-0007 (effects), gated on KEY PRESENCE
                // (`effectExports` declared).
                if self.effect_exports.is_some()
                    && !self
                        .effect_exports
                        .iter()
                        .flatten()
                        .any(|e| e.name == eff.name)
                {
                    out.push(Diagnostic::related(
                        "SOMA-EXP-0007",
                        "undeclared effect crossing",
                        format!("{}/{}", unit.id, eff.name),
                    ));
                }
            }
            for c in &unit.context {
                if let Some(ctx) = &self.context {
                    // Gate on key PRESENCE (discloses is Some), not on
                    // non-empty list.
                    if ctx.discloses.is_some() && !ctx.discloses.as_ref().unwrap().contains(c) {
                        out.push(Diagnostic::related(
                            "SOMA-EXP-0007",
                            "undeclared context crossing",
                            format!("{}/{}", unit.id, c),
                        ));
                    }
                }
            }
        }

        // SOMA-AUTH-0010
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

        // SOMA-AUTH-0004
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

        // SOMA-CMP-0002
        {
            let known: BTreeSet<&str> = std::iter::once(self.id.as_str())
                .chain(self.op_ids())
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

        // SOMA-CMP-0005 + SOMA-CMP-0006
        let in_types: BTreeMap<&str, &str> = self
            .input_ports
            .iter()
            .map(|p| (p.name.as_str(), p.ty.as_str()))
            .collect();
        let out_types: BTreeMap<&str, &str> = self
            .output_ports
            .iter()
            .map(|p| (p.name.as_str(), p.ty.as_str()))
            .collect();
        for unit in &self.body {
            for inp in &unit.inputs {
                if let Some(t) = in_types.get(inp.name.as_str())
                    && *t != inp.ty
                {
                    out.push(Diagnostic::related(
                        "SOMA-CMP-0005",
                        "port/edge type mismatch",
                        unit.id.clone(),
                    ));
                }
            }
            for o in &unit.outputs {
                if let Some(t) = out_types.get(o.name.as_str())
                    && *t != o.ty
                {
                    out.push(Diagnostic::related(
                        "SOMA-CMP-0005",
                        "port/edge type mismatch",
                        unit.id.clone(),
                    ));
                }
            }
        }
        for t in self
            .input_ports
            .iter()
            .chain(self.output_ports.iter())
            .map(|p| &p.ty)
            .chain(
                self.body
                    .iter()
                    .flat_map(|u| u.inputs.iter().map(|i| &i.ty)),
            )
            .chain(
                self.body
                    .iter()
                    .flat_map(|u| u.outputs.iter().map(|o| &o.ty)),
            )
        {
            if !type_in_vocabulary(t) {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0006",
                    format!("type {t:?} outside vocabulary"),
                ));
            }
        }

        // SOMA-EXP-0001 / 0003
        if self.is_composite() && self.op_ids().contains(&self.id.as_str()) {
            out.push(Diagnostic::new(
                "SOMA-EXP-0001",
                "transitive self-containment",
            ));
        }
        {
            let ids = self.op_ids();
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

        // Dependency graph helpers
        let mut producers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for unit in &self.body {
            for o in &unit.outputs {
                producers
                    .entry(o.name.as_str())
                    .or_default()
                    .insert(unit.id.as_str());
            }
        }
        let consumed: BTreeSet<&str> = self
            .body
            .iter()
            .flat_map(|u| u.inputs.iter().map(|i| i.name.as_str()))
            .collect();
        let boundary_out: BTreeSet<&str> =
            self.output_ports.iter().map(|p| p.name.as_str()).collect();
        let boundary_in: BTreeSet<&str> =
            self.input_ports.iter().map(|p| p.name.as_str()).collect();

        // SOMA-EXP-0006: required input shadowed by optional output port
        for p in &self.input_ports {
            if p.is_required()
                && self
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

        // SOMA-EXP-0005: dead inputs
        for unit in &self.body {
            for inp in &unit.inputs {
                if !producers.contains_key(inp.name.as_str())
                    && !boundary_in.contains(inp.name.as_str())
                {
                    out.push(Diagnostic::related(
                        "SOMA-EXP-0005",
                        "required input not fed/defaulted",
                        format!("{}/{}", unit.id, inp.name),
                    ));
                }
            }
        }

        // SOMA-EXP-0004: orphan operations
        for unit in &self.body {
            if unit.inputs.is_empty() {
                let produced: BTreeSet<&str> =
                    unit.outputs.iter().map(|o| o.name.as_str()).collect();
                if produced.is_disjoint(&consumed) && produced.is_disjoint(&boundary_out) {
                    out.push(Diagnostic::related(
                        "SOMA-EXP-0004",
                        "operation unreachable from any boundary",
                        unit.id.clone(),
                    ));
                }
            }
        }

        // SOMA-EXP-0002: operation-edge cycle
        if has_cycle(&self.body, &producers) {
            out.push(Diagnostic::new("SOMA-EXP-0002", "operation-edge cycle"));
        }

        // SOMA-OUT-0001 / 0002
        for unit in &self.body {
            for inp in &unit.inputs {
                let accepted: BTreeSet<OutcomeVariant> =
                    inp.accepted_outcomes.iter().copied().collect();
                let emitted: BTreeSet<OutcomeVariant> = self
                    .body
                    .iter()
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
                    out.push(Diagnostic::related(
                        "SOMA-OUT-0001",
                        "failure-like outcome coerced to success",
                        format!("{}/{}", unit.id, inp.name),
                    ));
                } else if emitted.iter().any(|v| !accepted.contains(v)) {
                    out.push(Diagnostic::related(
                        "SOMA-OUT-0002",
                        "upstream outcome not in accept set",
                        format!("{}/{}", unit.id, inp.name),
                    ));
                }
            }
        }

        // Governance constraints
        out.extend(audit_governance(self));

        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }
}

/// SOMA-AUTH-0001 / 0002 / 0003 over one body unit's authority grants.
fn audit_unit_authority(
    wf: &WorkflowDefinition,
    unit: &OperationDefinition,
    tool_keys: &BTreeSet<&str>,
    readable: &BTreeSet<&str>,
    writable: &BTreeSet<&str>,
    out: &mut Vec<Diagnostic>,
) {
    let imported: BTreeSet<&str> = wf.authority_imports.iter().map(String::as_str).collect();
    for grant in &unit.authority {
        match grant.split_once(':') {
            Some(("readable", scope)) => {
                if !readable.contains(scope) {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0003",
                        "readable scope not declared",
                        unit.id.clone(),
                    ));
                }
            }
            Some(("writable", scope)) => {
                if !writable.contains(scope) {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0003",
                        "writable scope not declared",
                        unit.id.clone(),
                    ));
                }
            }
            _ => {
                if !tool_keys.contains(grant.as_str()) {
                    out.push(Diagnostic::related(
                        "SOMA-AUTH-0001",
                        "capability used but not granted",
                        unit.id.clone(),
                    ));
                }
            }
        }
        // SOMA-AUTH-0002 (composite): flags ANY unit authority entry outside
        // the import set — including scope-prefixed grants; the prefix
        // exemption exists only in AUTH-0001.
        if wf.is_composite() && !imported.contains(grant.as_str()) {
            out.push(Diagnostic::related(
                "SOMA-AUTH-0002",
                "composite exceeding imported authority",
                unit.id.clone(),
            ));
        }
    }
}

fn has_cycle(body: &[OperationDefinition], producers: &BTreeMap<&str, BTreeSet<&str>>) -> bool {
    // Kahn's algorithm over INDEX-keyed dependency edges so id-sharing units
    // never collapse into one node.
    let n = body.len();
    let mut indegree = vec![0usize; n];
    let mut edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for (i, unit) in body.iter().enumerate() {
        let mut deps: BTreeSet<usize> = BTreeSet::new();
        for inp in &unit.inputs {
            if let Some(producers_of_name) = producers.get(inp.name.as_str()) {
                for (j, other) in body.iter().enumerate() {
                    if producers_of_name.contains(&other.id.as_str()) {
                        deps.insert(j);
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

fn evaluate_gov(wf: &WorkflowDefinition, constraint: &GovernanceConstraint) -> Option<bool> {
    let (field, op, arg) = parse_predicate(&constraint.predicate)?;
    // Dynamic constraints need an evaluation point to be decidable statically.
    if constraint.kind == ConstraintKind::Dynamic && constraint.evaluation_point.is_none() {
        return None;
    }
    // Subject must reference known nodes only.
    if let Some(nodes) = &constraint.subject.node_set {
        let known: BTreeSet<&str> = wf.op_ids().into_iter().collect();
        if nodes.iter().any(|n| !known.contains(n.as_str())) {
            return None;
        }
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

fn audit_governance(wf: &WorkflowDefinition) -> Vec<Diagnostic> {
    let mut codes: BTreeSet<&'static str> = BTreeSet::new();
    let mut undecidable = false;
    let mut parsed: Vec<(String, String, serde_json::Value, BTreeSet<String>)> = Vec::new();

    for con in &wf.constraints {
        if con.kind == ConstraintKind::Dynamic && con.evaluation_point.is_none() {
            undecidable = true;
            continue;
        }
        let known_nodes: BTreeSet<&str> = wf.op_ids().into_iter().collect();
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
        match evaluate_gov(wf, con) {
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
