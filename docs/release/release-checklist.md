# Release Checklist

## Pre-release verification

- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo test`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] Repo Workbench golden-path CI passes
- [ ] `cargo install --path . --force`
- [ ] `prometheos --version`
- [ ] First-value workflow runs from installed binary against `fixtures/repo-workbench/rust-risky`
- [ ] Linux install smoke CI passes
- [ ] Optional manual local endpoint smoke test run (see [Ornith manual validation guide](../guides/ornith-manual-validation.md))

## Documentation

- [ ] README install section is accurate
- [ ] `docs/guides/install.md` is accurate
- [ ] `docs/guides/zero-to-first-value.md` is accurate
- [ ] Safety model is documented
- [ ] `docs/release/alpha-notes.md` is accurate
- [ ] demo transcript is accurate
- [ ] README links to alpha notes
- [ ] Local model compatibility docs are accurate
- [ ] No unsupported model integration is claimed

## Safety

- [ ] `work run` does not modify source files (verified by CI golden-path)
- [ ] `work approve` records approval only
- [ ] Local workbench state (`.prometheos-lite/`) is ignored by `.gitignore`

## Versioning

- [ ] Confirm `Cargo.toml` version
- [ ] Update version if needed
- [ ] Tag release only after CI passes

## Known non-goals for first alpha

- no Brain integration
- no Mnemosyne integration
- no cloud sync
- no automatic patch application
- no plugin marketplace
- no crates.io publishing
- no Homebrew formula

## Post-release

- [ ] Push git tag
- [ ] Write release notes summarizing changes
- [ ] Verify CI workflow runs against the tag

## v1.6.1-alpha.1 manual release checklist

- [ ] `main` is green
- [ ] CI passes
- [ ] Linux Install Smoke passes
- [ ] Repo Workbench Golden Path passes
- [ ] `cargo install --path . --force` verified
- [ ] `prometheos --version` verified
- [ ] first-value workflow verified
- [ ] generated artifacts include provenance
- [ ] release notes reviewed
- [ ] GitHub Release draft reviewed
- [ ] tag created only after approval
