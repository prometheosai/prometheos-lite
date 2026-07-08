# PrometheOS ReviewOnly — Planned GitHub Automation Flow

This document describes the ReviewOnly GitHub automation flow.

ReviewOnly is the PrometheOS-native equivalent of Claude Code's automatic PR review mode. It provides read-only diff review via a GitHub Action that posts structured review comments.

## Implementation status

- **v0 (implemented):** deterministic, read-only reviewer. No external models are invoked. It runs as `.github/workflows/prometheos-reviewonly.yml` and uses `scripts/reviewonly/reviewonly.mjs` to inspect the PR metadata/diff and post one structured ReviewOnly report comment.
- No LLM-powered review yet. The deterministic v0 is the first automation step so the governance layer can be proven before any model or provider dependency is introduced.

## Level 1: ReviewOnly

| Property       | Value                                    |
| -------------- | ---------------------------------------- |
| Trigger        | `pull_request` opened/synchronize/reopened |
| Permissions    | `contents: read`, `pull-requests: write` |
| Writes         | review comments only                     |
| No commits     | yes                                      |
| No branch writes | yes                                    |
| No merge       | yes                                      |
| No issue-to-PR | yes                                      |
| No dependency changes | yes                              |
| No code modification | yes                              |
| Model invoked  | no (v0 is deterministic)                 |

### v0 implementation

- Workflow: `.github/workflows/prometheos-reviewonly.yml`
- Script: `scripts/reviewonly/reviewonly.mjs` (Node, no npm package, no model)
- Behavior: reads PR metadata + diff, reads repo agent/safety docs, produces one structured ReviewOnly report comment (deduped across pushes), and exits 0 so it never blocks CI.
- Blocker-level checks: dependency files changed without approval, CI/workflow weakening, secrets/credentials, conflict markers, promotion/overclaim of experimental surfaces, benchmark claims without evidence.
- Warning-level checks: >5 files, >200 net lines, runtime/API/frontend paths touched, missing verification for the touched area, docs-only claim with runtime files changed.
- The bot posts a normal comment; it does **not** open a formal blocking review, commit, write branches, or merge.

## Reviewer behavior

When triggered, the reviewer should:

1. Read `AGENTS.md`.
2. Read Loop Engineering Protocol.
3. Read Safety Gates.
4. Inspect PR diff.
5. Check product boundary risks.
6. Check verification evidence.
7. Check if PR exceeds budget.
8. Check if docs overclaim maturity.
9. Check if frontend/API/autonomous surfaces are promoted accidentally.
10. Post findings grouped by severity.

## Finding severities

| Severity    | Meaning                                      |
|-------------|----------------------------------------------|
| Blocker     | Must be resolved before merge                |
| Warning     | Should be reviewed and addressed             |
| Suggestion  | Optional improvement                         |
| Question    | Clarification needed                         |

## Confidence labels

| Label             | Meaning                                      |
|-------------------|----------------------------------------------|
| High confidence   | Clear violation of protocol or safety gate   |
| Medium confidence | Likely issue, may need human interpretation  |
| Low confidence    | Possible issue, needs human review           |

## ReviewOnly output format

```markdown
## PrometheOS ReviewOnly Report

Mode: ReviewOnly

Scope:
- Files reviewed:
- Lines changed:
- Budget status:

Findings:
- Blockers:
- Warnings:
- Suggestions:
- Questions:

Verification evidence:
- Claimed checks:
- Missing checks:
- CI status:

Product boundary check:
- Stable alpha:
- Experimental surfaces:
- Overclaim risk:

Recommendation:
- Approve after human review
- Request changes
- Wait for CI
- Needs scope clarification
```

## Stop conditions

The reviewer should stop and flag if any of the following apply:

- Diff too large.
- Source-of-truth conflict.
- Missing verification for changed area.
- Possible CI weakening.
- Possible stable alpha scope change.
- Possible frontend/API/autonomous promotion.
- Dependency changes without approval.
- Secrets or credentials.

## Human review

ReviewOnly does not replace human review. All PRs still require human approval before merge.
