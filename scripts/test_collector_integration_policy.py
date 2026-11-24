"""Deterministic security regressions for the pinned collector integration.

This does NOT authenticate anything and does NOT dispatch the workflow. It only
asserts the *shape* of the single approved manual workflow after adding the
read-only collector job:

- production source stays pinned to the reviewed PR 341 head;
- the collector is sourced only from a separately pinned 40-hex commit;
- both source identities (and PostCSS) are verified before collector code runs;
- the staged collector bytes are verified against the pinned git blob;
- the ACTIVE workflow identity is bound through the separately reviewed,
  deterministic policy digest, with no digest self-approval or circular
  collector/workflow digest dependency;
- manual-only, contents: read, no secrets/tokens are preserved;
- preapproval: pins still name the historical digest and are intentionally stale;
- postapproval: the executing workflow digest matches identical current pins in
  both pin files (verified in memory only — no pin file is written here);
- stale or inconsistent pins are rejected, and the historical digest remains
  provenance rather than an active execution pin.
"""

import hashlib
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "pr341-windows-acceptance.yml"
CONFIG_PINS = ROOT / "scripts" / "reviewed_config_digests.json"
POLICY = ROOT / "scripts" / "local_ci_policy.py"

REVIEWED_PR341_SHA = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce"
REVIEWED_COLLECTOR_SHA = "96d9cbcefdeb09955c1fc7b50734d260263813f0"
HISTORICAL_WORKFLOW_DIGEST = "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c"
EXPECTED_CURRENT_DIGEST = "53984301f49022049859218efb2ee2b44dd0e10902542f583ac7e19967cb30fe"
APPROVED_CHECKOUT_ACTION = "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683"
EXPECTED_WORKFLOW_REF = (
    "prometheosai/prometheos-lite/.github/workflows/"
    "pr341-windows-acceptance.yml@refs/heads/main"
)


def normalized_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def config_pin(text: str) -> str | None:
    match = re.search(
        r'"\.github/workflows/pr341-windows-acceptance\.yml"\s*:\s*"([0-9a-f]{64})"', text
    )
    return match.group(1) if match else None


def policy_pin(text: str) -> str | None:
    match = re.search(r'APPROVED_WORKFLOW_SHA256\s*=\s*"([0-9a-f]{64})"', text)
    return match.group(1) if match else None


def validate_postapproval_bindings(config: str | None, policy: str | None, actual: str) -> list[str]:
    """Postapproval state only: all active bindings must name the same digest.

    This is a pure document check. It never writes the pin files and does not
    assert that a Principal re-pin has already happened.
    """
    errors = []
    if config != actual:
        errors.append("reviewed_config_digests.json workflow pin is stale or missing")
    if policy != actual:
        errors.append("local_ci_policy.py workflow pin is stale or missing")
    if config is not None and policy is not None and config != policy:
        errors.append("workflow digest bindings are inconsistent between the two pin files")
    if config == HISTORICAL_WORKFLOW_DIGEST or policy == HISTORICAL_WORKFLOW_DIGEST:
        errors.append("historical introduction digest must not be an active execution pin")
    return errors


class CollectorIntegrationPolicy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")

    def test_manual_only_and_read_only_preserved(self):
        self.assertIn("workflow_dispatch:", self.text)
        self.assertNotIn("push:", self.text)
        self.assertNotIn("schedule:", self.text)
        self.assertIn("permissions:", self.text)
        self.assertIn("contents: read", self.text)
        self.assertNotRegex(self.text, r"\bsecrets\.")
        self.assertNotRegex(self.text, r"\bgithub\.token\b")

    def test_production_source_and_postcss_stay_pinned(self):
        self.assertIn(REVIEWED_PR341_SHA, self.text)
        self.assertIn("33ad091d26d8a9dc95ebdf616e217d985ec215b8", self.text)
        self.assertIn("HEAD:frontend/postcss.config.js", self.text)
        self.assertIn("Untrusted PostCSS configuration", self.text)

    def test_collector_is_sourced_only_from_pinned_commit(self):
        self.assertEqual(
            self.text.count(APPROVED_CHECKOUT_ACTION), 3,
            "one existing production checkout plus two in the collector job",
        )
        self.assertEqual(self.text.count(f"ref: {REVIEWED_PR341_SHA}"), 2)
        self.assertIn(f"ref: {REVIEWED_COLLECTOR_SHA}", self.text)
        self.assertIn("path: collector-src", self.text)
        self.assertEqual(self.text.count("persist-credentials: false"), 3)
        # No mutable branch ref may be used for the collector checkout.
        self.assertNotRegex(self.text, r"ref:\s*(main|master|HEAD|\$\{\{)")

    def test_identities_verified_before_collector_executes(self):
        verify = self.text.index("Verify both pinned source identities")
        bind = self.text.index("Bind active workflow identity")
        stage = self.text.index("Stage pinned collector contract")
        run = self.text.index("Run pinned nonprivileged collector")
        self.assertLess(verify, bind)
        self.assertLess(bind, stage)
        self.assertLess(stage, run)
        self.assertIn("Unexpected production source commit", self.text)
        self.assertIn("Unexpected collector commit", self.text)
        self.assertIn("Staged collector contract does not match the pinned source blob", self.text)

    def test_active_workflow_identity_is_bound_without_self_approval(self):
        self.assertIn("Bind active workflow identity", self.text)
        self.assertIn(EXPECTED_WORKFLOW_REF, self.text)
        self.assertIn("ACTIVE_WORKFLOW_IDENTITY=REVIEWED_WORKFLOW_REF_BOUND", self.text)
        self.assertIn(
            "ACTIVE_WORKFLOW_DIGEST=BOUND_BY_PRINCIPAL_POLICY_PIN_NOT_ASSERTED_HERE",
            self.text,
        )
        # No digest of the executing workflow is embedded in the workflow or
        # asserted as verified: re-pinning remains a Principal-only action.
        self.assertNotIn(normalized_digest(WORKFLOW), self.text)
        for line in self.text.splitlines():
            if HISTORICAL_WORKFLOW_DIGEST in line:
                self.assertIn("provenance only", line)
        # The historical digest is not presented as verification here.
        self.assertNotIn("HISTORICAL_INTRODUCTION_DIGEST=" + HISTORICAL_WORKFLOW_DIGEST, self.text)

    def test_collector_run_is_read_only_and_claims_only(self):
        self.assertIn("test_collector_collects_real_nonprivileged_facts", self.text)
        self.assertIn("PR341_EVIDENCE_OUT", self.text)
        self.assertIn("NONPRIVILEGED_CLAIMS_ONLY_UNVERIFIED", self.text)

    def test_preapproval_state_requires_principal_repin(self):
        # Preapproval: the workflow has new bytes but the pins still name the
        # historical introduction digest. No pin file is modified by this PR.
        actual = normalized_digest(WORKFLOW)
        self.assertEqual(actual, EXPECTED_CURRENT_DIGEST)
        self.assertNotEqual(actual, HISTORICAL_WORKFLOW_DIGEST)
        pins_text = CONFIG_PINS.read_text(encoding="utf-8")
        policy_text = POLICY.read_text(encoding="utf-8")
        self.assertEqual(config_pin(pins_text), HISTORICAL_WORKFLOW_DIGEST)
        self.assertEqual(policy_pin(policy_text), HISTORICAL_WORKFLOW_DIGEST)
        self.assertNotIn(actual, pins_text)
        self.assertNotIn(actual, policy_text)
        # The preapproval state is deliberately rejected by the postapproval
        # consistency rule; the policy gate must remain failing.
        self.assertTrue(validate_postapproval_bindings(config_pin(pins_text), policy_pin(policy_text), actual))

    def test_postapproval_pin_consistency_is_deterministic_in_memory(self):
        # Simulate ONLY the Principal's deliberate re-pin in memory. This does
        # NOT claim that a re-pin has happened and writes no files.
        actual = normalized_digest(WORKFLOW)
        pins_text = CONFIG_PINS.read_text(encoding="utf-8")
        policy_text = POLICY.read_text(encoding="utf-8")
        repinned_config = pins_text.replace(
            f'"{HISTORICAL_WORKFLOW_DIGEST}"', f'"{EXPECTED_CURRENT_DIGEST}"'
        )
        repinned_policy = policy_text.replace(
            f'"{HISTORICAL_WORKFLOW_DIGEST}"', f'"{EXPECTED_CURRENT_DIGEST}"'
        )
        config_digest = config_pin(repinned_config)
        policy_digest = policy_pin(repinned_policy)
        self.assertEqual(config_digest, actual)
        self.assertEqual(policy_digest, actual)
        self.assertEqual(config_digest, policy_digest)
        self.assertEqual(validate_postapproval_bindings(config_digest, policy_digest, actual), [])
        # Files on disk remain untouched: re-pinning is Principal-only.
        self.assertEqual(config_pin(CONFIG_PINS.read_text(encoding="utf-8")), HISTORICAL_WORKFLOW_DIGEST)
        self.assertEqual(policy_pin(POLICY.read_text(encoding="utf-8")), HISTORICAL_WORKFLOW_DIGEST)

    def test_stale_or_inconsistent_pins_are_rejected(self):
        actual = EXPECTED_CURRENT_DIGEST
        self.assertTrue(validate_postapproval_bindings(HISTORICAL_WORKFLOW_DIGEST, HISTORICAL_WORKFLOW_DIGEST, actual))
        self.assertTrue(validate_postapproval_bindings(None, None, actual))
        self.assertTrue(validate_postapproval_bindings(actual, HISTORICAL_WORKFLOW_DIGEST, actual))
        self.assertTrue(validate_postapproval_bindings(HISTORICAL_WORKFLOW_DIGEST, actual, actual))
        self.assertEqual(validate_postapproval_bindings(actual, actual, actual), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
