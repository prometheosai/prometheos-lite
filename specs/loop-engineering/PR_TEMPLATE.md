# PR Template

Use this template for all loop-engineering PRs.

```markdown
## Summary

<!-- Concise description of what this PR does and why. -->

## Mode

<!-- Task Mode or Epic Completion Mode. -->

## Approved scope

<!-- Link to the approved issue, comment, or prompt that authorized this work. -->

## Tasks completed

<!-- List of completed bounded tasks. -->

## Files changed

<!-- List of files changed with brief reason for each. -->

## Safety boundary

<!--
Confirm no stable alpha changes, no autonomous execution promotion,
no dependency additions, no API/runtime behavior changes outside scope.
-->

## Verification

<!--
Checklist of verification steps run and their results.
Do not claim checks that were not run.
-->

- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo test`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cd frontend && npm ci && npm run build` (if frontend touched)

## Known limitations

<!-- Any known limitations, soft warnings, or partial coverage. -->

## Follow-ups

<!-- Tasks for future PRs, if any. -->

## Human review gate

<!--
Leave this section for the human reviewer.
This PR requires human approval before merge.
-->
```
