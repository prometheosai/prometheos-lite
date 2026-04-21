# Project Management Setup

This repository is configured to support consistent issue/PR flow and CI quality gates.

## What is configured

- CI workflow at `.github/workflows/ci.yml` with:
  - `cargo fmt --check`
  - `cargo clippy -D warnings`
  - `cargo test`
- Issue templates:
  - `.github/ISSUE_TEMPLATE/feature.yml`
  - `.github/ISSUE_TEMPLATE/task.yml`
  - `.github/ISSUE_TEMPLATE/bug.yml`
  - `.github/ISSUE_TEMPLATE/config.yml`
- PR template: `.github/pull_request_template.md`
- Code owners: `.github/CODEOWNERS`
- Contributor and review standards: `CONTRIBUTING.md`

## PRD Issue Sync

Use `scripts/sync-prd-issues-to-project.ps1` to create PRD issues and add them to org project `prometheosai/3`.

### Usage

```powershell
.\scripts\sync-prd-issues-to-project.ps1 -GithubToken "<YOUR_GITHUB_TOKEN>"
```

### Token requirements

Use a GitHub token with permissions for:
- repository issues write access
- organization project write access

The script creates 21 issues (from the PRD) and adds each to Project 3.

## Recommended GitHub branch protection (manual)

Configure on `main`:

1. Require pull request before merging.
2. Require approvals (at least 1).
3. Require status checks:
   - `Rust Checks`
4. Dismiss stale approvals when new commits are pushed.
5. Require conversation resolution before merge.
6. Restrict force pushes and branch deletion.
