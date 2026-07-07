# PrometheOS ReviewOnly — Planned GitHub Automation Flow

This document describes the future ReviewOnly GitHub automation flow.

ReviewOnly is the PrometheOS-native equivalent of Claude Code's automatic PR review mode. It provides read-only diff review via a GitHub Action that posts structured review comments.

This is a **planned design**. No automation has been implemented yet.

## Level 1: ReviewOnly

| Property       | Value                                    |
|----------------|------------------------------------------|
| Trigger        | `pull_request` opened/synchronize        |
| Permissions    | read-only diff review, comments only     |
| Writes         | review comments only                     |
| No commits     | yes                                      |
| No branch writes | yes                                    |
| No merge       | yes                                      |
| No issue-to-PR | yes                                      |
| No dependency changes | yes                              |
| No code modification | yes                              |

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
