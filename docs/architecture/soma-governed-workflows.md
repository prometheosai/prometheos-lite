# SOMA Governed Workflow Architecture

## Status

Proposed cross-cutting architecture for the graph-orchestration roadmap. This document defines ownership and contracts; it does not promote experimental runtime surfaces to the stable alpha product.

## Objective

PrometheOS Lite should execute portable, model-independent workflows whose data flow, authority, review gates, evidence, and resumability can be validated before execution.

The architecture separates responsibilities deliberately:

| Component | Responsibility |
|---|---|
| SOMA | Governed workflow semantics: operations, authority, gates, transitions, and lawful execution. |
| SOMA++ | Versioned portable workflow and message contracts. |
| PrometheOS Lite | Compilation, provider/harness routing, tool execution, validation, review, apply, rollback, and runtime evidence. |
| Mnemosyne | Durable decisions, workflow state, checkpoints, provenance, retrieval, and cross-implementation resume. |
| Foundry | Conformance, capability, policy, context, continuity, and task-success evaluation. |

## Canonical workflow representation

The source of truth is a versioned workflow AST, not chat history and not a visual graph.

The canonical representation must support deterministic JSON serialization and schema validation. Other views are projections of the same AST:

- compact model-facing representation;
- human-readable plan;
- graph visualization;
- executable plan;
- evidence timeline;
- review report.

The runtime must reject unsupported versions, unresolved references, invalid ports, and unsafe transitions before any effectful node runs.

## Recursive composite workflows

A `CompositeWorkflow` encapsulates a group of operations behind typed input and output ports.

A composite may contain other composites. Internal operations are private unless data or authority is exposed explicitly through the composite boundary.

Required properties:

- typed input and output ports;
- stable composite identity and version;
- private internal node identifiers;
- explicit imported authority and exported effects;
- bounded context visibility;
- independent validation and testing;
- deterministic expansion into an execution plan.

This enables a complex governed change workflow to appear externally as one operation while retaining internal assessment, planning, generation, validation, integrity, and review stages.

## Authority profiles

Every operation declares an `AuthorityProfile` rather than inheriting unlimited runtime capability.

The profile should include:

- execution freedom: deterministic, constrained model, scoped agent, or human decision;
- allowed tools and operations;
- readable and writable scopes;
- network and provider policy;
- secret-access policy;
- source-mutation authority;
- cost, token, duration, and retry budgets;
- review requirements before effects;
- escalation and abstention behavior.

Authority is part of the compiled plan and durable evidence. Provider or harness selection cannot expand it.

## Typed outcomes

Operations return typed outcomes rather than ambiguous nulls or implicit exceptions.

Minimum outcome family:

- `Produced<T>`;
- `Skipped { reason }`;
- `Blocked { gate }`;
- `Failed { evidence }`;
- `Cancelled { actor, reason }`;
- `ReviewRequired { request }`.

Downstream operations declare which outcomes they accept. The compiler rejects routes that would silently reinterpret failure, denial, cancellation, or skipped work as successful output.

## Governance compiler

Before execution, the compiler validates data, authority, policy, and graph structure.

Required rule examples:

- source application requires a human-approved validated proposal;
- restricted data cannot reach a disallowed provider;
- review-only workflows cannot include source mutation;
- memory writes require provenance;
- public API changes require contract validation and human review;
- destructive operations require explicit authority and rollback strategy;
- model-produced content cannot directly trigger irreversible effects;
- composite boundaries cannot leak undeclared data or authority;
- parallel branches cannot write overlapping scopes without an explicit conflict strategy.

Diagnostics must be stable, machine-readable, source-located, and suitable for Foundry conformance suites.

## Durable suspension and resume

Human review and external waits are native suspension points.

A suspension record must preserve:

- workflow and AST version;
- current operation and completed operations;
- repository identity and revision;
- input and output digests;
- authority snapshot;
- decisions and review request;
- provider, model, and harness provenance;
- evidence references;
- resume compatibility and expiration policy.

A compatible provider or harness may resume from this state without hidden chat or process-local history.

## Context isolation

Each operation receives the smallest authorized context needed for its declared purpose.

Composite boundaries provide a natural context boundary. Internal implementation details are not automatically visible to sibling or parent operations. Context compilation must record included sources, omitted material, budget, and policy decisions.

## Parallel execution

The initial implementation remains sequential until portable state, typed outcomes, suspension, and governance compilation are proven.

Later parallel execution may be enabled only when the compiler can prove or require declarations for:

- independent inputs;
- non-overlapping write scopes;
- deterministic join semantics;
- cancellation behavior;
- evidence ordering;
- conflict detection and recovery.

## Immediate implementation scope

1. Versioned canonical workflow AST.
2. Recursive composites with typed ports and private scopes.
3. Authority profiles and governance constraints.
4. Typed outcomes.
5. Governance compiler and stable diagnostics.
6. Durable suspension and resume contracts.
7. Deterministic projections from the canonical AST.
8. Foundry conformance and continuity fixtures.

## Future development

The following remain registered but are not on the current critical path:

- visual workflow authoring and synchronized graph editing;
- a custom compact textual syntax beyond canonical JSON/typed Rust models;
- an AI workflow-authoring assistant;
- a public node or workflow marketplace;
- infrastructure provisioning represented as first-class workflow nodes;
- generated native source code per workflow;
- broad general-purpose business automation;
- automatic parallel scheduling beyond proven conflict-free branches.

These features require the canonical AST, compiler, governance model, and evaluation framework to be stable first.

## Product boundary

SOMA is not a competing runtime. PrometheOS Lite remains the runtime and developer-facing product. SOMA defines lawful workflow semantics, while SOMA++ defines the portable contracts used by the runtime, Mnemosyne, Foundry, providers, and execution harnesses.
