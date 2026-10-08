# Vendored SOMA v1.2 bundle — provenance

Source: `https://github.com/prometheosai/soma`, tree `spec/soma/v1.2`,
cloned at upstream commit `a1d62d0e8b72e0818909a34e4a6706fd6e463977`
(byte-exact sparse clone; files copied verbatim — no transformations).

## What this is

The SPEC 007 (Runtime Capability Negotiation and Execution Compatibility)
publication bundle, published as the backward-compatible **v1.2.0**
bundle: every v1.1-family artifact is byte-identical to the vendored
`v1.1` tree, plus the three new capability families
(`RuntimeCapabilitySet`, `WorkRequirements`, `CompatibilityDecision`
schemas), the 18 new digest-pinned fixtures (4 valid, 14 invalid), and
the v1.2 manifest. Upstream's own manifest pins are sha256 over the
CANONICAL CONTENT (`try_canonical_digest` renders of the parsed
fixtures), not raw file bytes — the same convention as the v1.1 bundle
(the Slice 1B probe finding). The raw-byte additivity between the
vendored v1.1 and v1.2 trees is enforced by an exhaustive test
(`tests/soma_capability_conformance.rs`): every v1.1 manifest path must
exist in both trees byte-identically, and the v1.2-only set must equal
exactly the 18 enumerated capability fixtures.

The `wf-cmp-0007` all-zeros manifest placeholder carries its documented
upstream convention (intentionally unparseable fixture; the digest is a
placeholder, never verified).

## Lock policy

`vendored/soma/**` is `-text` in `.gitattributes` (byte-stable on every
platform). Any change to files under this directory requires an explicit
reviewed bundle upgrade.
