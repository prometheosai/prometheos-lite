# PrometheOS Lite Roadmap

PrometheOS Lite is being built as a governed execution system for AI work: a task becomes a scoped proposal, is validated in isolation, produces durable evidence, and stops at an explicit human authority gate. The roadmap then evolves that reliable single loop into a graph of specialized governed nodes.

## Current position

- Pilot and governance foundation: complete
- Fast Governed Loop V1: final review in PR #101
- Production hardening: next
- Governed node contracts: planned
- Graph orchestration: planned
- Specialized node library: planned
- Developer product: planned
- External validation: planned
- Full PrometheOS integration: later

## Delivery sequence

```text
PR #101 merged
  -> durable single-loop runtime
  -> typed governed node contracts
  -> generic NodeRunner
  -> persistent sequential graph
  -> human gates and classified failure routing
  -> specialized implementation/validation/review nodes
  -> developer CLI/API/evidence interface
  -> parallel branches and joins
  -> external pilot and release candidate
  -> full PrometheOS integration
```

## Epics

| Epic | Tracker | Outcome | Dependency |
|---|---:|---|---|
| E1 | #102 | Ship Fast Governed Loop V1 | Current |
| E2 | #103 | Production-harden the loop runtime | E1 |
| E3 | #104 | Define governed node contracts | E2 |
| E4 | #105 | Build graph orchestration core | E3 |
| E5 | #106 | Build specialized governed nodes | E3 + E4 |
| E6 | #107 | Deliver the developer product | E4 + E5 |
| E7 | #108 | Validate on real repositories | E2-E6 |
| E8 | #109 | Integrate with full PrometheOS | E4+, production after E7 |

## E1: Fast Governed Loop V1

- #110 Complete final acceptance and merge of PR #101
- #111 Reconcile pilot records, diagnostics, and V1 release metadata

Existing work: PR #101, issue #90, PR #92, issues #74 and #75.

## E2: Production hardening

- #112 Decompose the evaluation runtime into bounded modules
- #113 Add durable state transitions, event journal, and schema migration
- #114 Harden cross-platform locking, leases, cancellation, and interruption recovery
- #115 Add evidence integrity, secret protection, resource limits, and retention

## E3: Governed node contracts

- #116 Define versioned `NodeManifest` and `NodeResult` schemas
- #117 Implement the generic governed `NodeRunner`
- #118 Add authority, tool permissions, context, retry, and escalation policies
- #119 Build the governed node conformance test kit

## E4: Graph orchestration core

- #120 Define `GraphManifest` and durable graph-run state
- #121 Implement deterministic sequential routing and bounded cycles
- #122 Add human gates, retry edges, and classified failure routing
- #123 Add resumable graph execution and graph-level cancellation
- #124 Add parallel branches, joins, resource locks, and graph evidence index

First target graph:

```text
Intake
  -> Discovery
  -> Planning
  -> Implementation
  -> Validation
  -> Independent Review
  -> Human Review Gate
```

## E5: Specialized governed node library

- #125 Intake, repository discovery, and planning nodes
- #126 Governed implementation and repair nodes
- #127 Test discovery, isolated validation, and infrastructure diagnostics
- #128 Security review, evidence audit, and independent correctness review
- #129 Documentation and release-preparation nodes

## E6: Developer-facing product

- #130 Stable CLI, project configuration, and workflow templates
- #131 Run inspector, evidence viewer, and human decision interface
- #132 Local API and durable execution event stream
- #133 Provider routing, policy profiles, and cost accounting
- #134 Repository onboarding and actionable diagnostics
- #135 Optional local web dashboard

The CLI and durable runtime remain authoritative. The dashboard is optional and must not introduce dashboard-only behavior.

## E7: External validation

- #136 Repository, task, and provider benchmark matrix
- #137 Interruption, concurrency, resource-pressure, and long-duration campaigns
- #138 Independent security and governance assessment
- #139 Human-review usability and ordinary-agent baseline comparison
- #140 External pilot and release-candidate evidence report

Core metrics include accepted proposal rate, false-success rate, duplicate-generation rate, resume success, evidence completeness, original-repository mutation rate, classification accuracy, human acceptance, repair cycles, latency, and cost per accepted task.

## E8: Full PrometheOS integration

- #141 Operator adapters with explicit consent and external-action evidence
- #142 Mnemosyne memory and Maestro session orchestration
- #143 Mentor and emotion-analysis governed node contracts
- #144 Governed voice input/output and real-time session orchestration
- #145 Policy-controlled edge/cloud routing and personal-state isolation

## Pull request policy

Planned implementation PRs are not opened before a branch contains real work. Each child issue should normally produce one focused PR, or a clearly documented small PR series when the issue requires staged migration.

Every implementation PR must:

1. Link its child issue and parent epic.
2. State approved scope and explicit non-goals.
3. Include verification commands and evidence.
4. Preserve the original repository and authority boundaries.
5. Stop at `REVIEW_GATE` unless a human explicitly authorizes the next action.
6. Update the child issue and epic checklist after merge.

## Definition of done

PrometheOS Lite reaches its intended product milestone when a developer can submit a scoped task and the system can:

1. Formalize the objective and authority.
2. Inspect and plan against the repository.
3. Generate exactly one governed proposal.
4. Validate it in isolation.
5. Route defects or infrastructure failures correctly.
6. Obtain an independent review.
7. Preserve complete evidence.
8. Stop for human approval.
9. Resume safely after interruption.
10. Apply or merge only with explicit authorization.
