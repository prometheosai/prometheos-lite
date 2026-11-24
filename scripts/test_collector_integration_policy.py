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
- the workflow content digest changed, so a Principal re-pin is required.
"""

import hashlib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "pr341-windows-acceptance.yml"

REVIEWED_PR341_SHA = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce"
REVIEWED_COLLECTOR_SHA = "96d9cbcefdeb09955c1fc7b50734d260263813f0"
HISTORICAL_WORKFLOW_DIGEST = "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c"
APPROVED_CHECKOUT_ACTION = "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683"
EXPECTED_WORKFLOW_REF = (
    "prometheosai/prometheos-lite/.github/workflows/"
    "pr341-windows-acceptance.yml@refs/heads/main"
)


def normalized_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


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

    def test_review_digest_change_requires_principal_repin(self):
        self.assertNotEqual(normalized_digest(WORKFLOW), HISTORICAL_WORKFLOW_DIGEST)
        # The pin files are deliberately NOT updated by this focused PR: the
        # policy gate must keep failing until a Principal re-pins the digest.
        pins = (ROOT / "scripts" / "reviewed_config_digests.json").read_text(encoding="utf-8")
        self.assertIn(HISTORICAL_WORKFLOW_DIGEST, pins)
        policy = (ROOT / "scripts" / "local_ci_policy.py").read_text(encoding="utf-8")
        self.assertIn(HISTORICAL_WORKFLOW_DIGEST, policy)


if __name__ == "__main__":
    unittest.main(verbosity=2)
