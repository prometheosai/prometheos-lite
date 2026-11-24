"""State-independent security regressions for the pinned collector integration.

This does NOT authenticate anything, does NOT dispatch the workflow and never
writes a real policy pin. The tests accept TWO legitimate pin states:

- PREAPPROVAL: both pin files name the previously approved workflow digest;
  the repository policy gate is expected to fail for the changed workflow.
- APPROVED: both pin files name the reviewed executing-workflow digest; the
  repository policy gate passes.

The historical introduction digest remains provenance only. Mixed, missing,
unknown or mismatched pins are rejected. Every test passes in both legitimate
snapshots because the on-disk state is classified, not assumed.
"""

import hashlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "pr341-windows-acceptance.yml"
CONFIG_PINS = ROOT / "scripts" / "reviewed_config_digests.json"
POLICY = ROOT / "scripts" / "local_ci_policy.py"

REVIEWED_PR341_SHA = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce"
REVIEWED_COLLECTOR_SHA = "debd59d5de62b7c4bb6e30d9d9c797d68a8e3b68"
COLLECTOR_CONTRACT_BLOB = "41d06dda3ce49eb8fc7d5378ef35b604e869a568"
RESOLVER_SOURCE_BLOB = "6f9a8254c19aaf67aae487ee9f2ee53fbb1e546a"
RESOLVER_EMBEDDED_SHA256 = "fa3a0b71bd623f8e1a5d3078f0cb845a0df93a7974fac5cdd755cc2a9247aaa4"
HISTORICAL_WORKFLOW_DIGEST = "b45b5ed1c912cef4d14251563fe2366f76c1d28287ce2e2291ef84f55d01b84c"
PREVIOUS_APPROVED_DIGEST = "65dcce0bf11b3791b2dc180bfbd9d010fed97fb6682862d6a559895edf63db1a"
EXPECTED_CURRENT_DIGEST = "824793f326c4136696eab0b62c9c3a740a558167038edcb1720e6d2c90d2af13"
APPROVED_CHECKOUT_ACTION = "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683"
EXPECTED_WORKFLOW_REF = (
    "prometheosai/prometheos-lite/.github/workflows/"
    "pr341-windows-acceptance.yml@refs/heads/main"
)

CONFIG_WORKFLOW_PIN_RE = (
    r'"\.github/workflows/pr341-windows-acceptance\.yml"\s*:\s*"(?P<digest>[0-9a-f]{64})"'
)
POLICY_WORKFLOW_PIN_RE = r'APPROVED_WORKFLOW_SHA256\s*=\s*"(?P<digest>[0-9a-f]{64})"'
POLICY_SELF_PIN_RE = r'"scripts/local_ci_policy\.py"\s*:\s*"(?P<digest>[0-9a-f]{64})"'


def normalized_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def config_pin(text: str) -> str | None:
    match = re.search(CONFIG_WORKFLOW_PIN_RE, text)
    return match.group("digest") if match else None


def policy_pin(text: str) -> str | None:
    match = re.search(POLICY_WORKFLOW_PIN_RE, text)
    return match.group("digest") if match else None


def set_pin(text: str, pattern: str, digest: str) -> str:
    updated, count = re.subn(
        pattern,
        lambda m: m.group(0).replace(m.group("digest"), digest),
        text,
    )
    if count != 1:
        raise AssertionError(f"expected exactly one pin for {pattern!r}, found {count}")
    return updated


def pin_state(config: str | None, policy: str | None) -> str:
    """Classify the two active workflow pins without assuming the date."""
    if config is None or policy is None:
        return "MISSING"
    if config != policy:
        return "MIXED"
    if config == PREVIOUS_APPROVED_DIGEST:
        return "PREAPPROVAL"
    if config == EXPECTED_CURRENT_DIGEST:
        return "APPROVED"
    if config == HISTORICAL_WORKFLOW_DIGEST:
        return "UNAPPROVED"
    return "UNKNOWN"


def validate_pin_state(config: str | None, policy: str | None) -> list[str]:
    state = pin_state(config, policy)
    if state in ("PREAPPROVAL", "APPROVED", "UNAPPROVED"):
        return []
    if state == "MISSING":
        return ["a required workflow pin is missing"]
    if state == "MIXED":
        return ["workflow pins are mixed or inconsistent between the two files"]
    return ["workflow pins name an unknown digest"]


def validate_postapproval_bindings(config: str | None, policy: str | None, actual: str) -> list[str]:
    """APPROVED-state only: every active binding must name the executing digest."""
    errors = []
    if config != actual:
        errors.append("reviewed_config_digests.json workflow pin is stale or missing")
    if policy != actual:
        errors.append("local_ci_policy.py workflow pin is stale or missing")
    if config is not None and policy is not None and config != policy:
        errors.append("workflow digest bindings are inconsistent between the two pin files")
    return errors


class CollectorIntegrationPolicy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.config_text = CONFIG_PINS.read_text(encoding="utf-8")
        cls.policy_text = POLICY.read_text(encoding="utf-8")

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
        self.assertNotIn("96d9cbcefdeb09955c1fc7b50734d260263813f0", self.text)
        self.assertIn("git hash-object", self.text)
        self.assertIn("path: collector-src", self.text)
        self.assertEqual(self.text.count("persist-credentials: false"), 3)
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
        self.assertIn("--message-format=json", self.text)
        self.assertIn("resolve_cargo_test_artifact.py", self.text)
        self.assertIn(RESOLVER_EMBEDDED_SHA256, self.text)
        self.assertNotIn(".cargo-target\\debug\\deps", self.text)
        self.assertNotIn("LastWriteTime", self.text)

    def test_embedded_resolver_and_least_privilege_traversal(self):
        # W1: the resolver is embedded in the workflow so the pinned PR #341
        # checkout need not contain it. Extract it from the YAML-parsed
        # effective PowerShell run scalar, not raw workflow text.
        from scripts import workflow_yaml_contract as contract

        embedded = contract.embedded_resolver_source(
            contract.build_step_run(contract.parse_workflow())
        )
        self.assertEqual(
            hashlib.sha256(embedded.encode("utf-8")).hexdigest(), RESOLVER_EMBEDDED_SHA256
        )
        self.assertIn("GetRelativePath", self.text)
        self.assertIn("Missing intermediate test-binary directory", self.text)
        self.assertIn("outside the owned workspace", self.text)
        self.assertIn("ACL_TRAVERSE=", self.text)
        # No recursive grant on the build tree; only the owned evidence dir
        # keeps the bounded (OI)(CI)F grant.
        self.assertEqual(self.text.count("(OI)(CI)RX"), 0)
        self.assertEqual(self.text.count("(OI)(CI)F"), 1)
        self.assertIn("if ($testProc.ExitCode -ne 0)", self.text)
        self.assertIn("Restricted-token test SKIPPED", self.text)

    def test_active_workflow_identity_is_bound_without_self_approval(self):
        self.assertIn("Bind active workflow identity", self.text)
        self.assertIn(EXPECTED_WORKFLOW_REF, self.text)
        self.assertIn("ACTIVE_WORKFLOW_IDENTITY=REVIEWED_WORKFLOW_REF_BOUND", self.text)
        self.assertIn(
            "ACTIVE_WORKFLOW_DIGEST=BOUND_BY_PRINCIPAL_POLICY_PIN_NOT_ASSERTED_HERE",
            self.text,
        )
        self.assertNotIn(normalized_digest(WORKFLOW), self.text)
        for line in self.text.splitlines():
            if HISTORICAL_WORKFLOW_DIGEST in line:
                self.assertIn("provenance only", line)
        self.assertNotIn("HISTORICAL_INTRODUCTION_DIGEST=" + HISTORICAL_WORKFLOW_DIGEST, self.text)

    def test_collector_run_is_read_only_and_claims_only(self):
        self.assertIn("test_collector_collects_real_nonprivileged_facts", self.text)
        self.assertIn("PR341_EVIDENCE_OUT", self.text)
        self.assertIn("NONPRIVILEGED_CLAIMS_ONLY_UNVERIFIED", self.text)

    def test_pin_state_is_state_independent(self):
        """Passes identically before and after the Principal's re-pin."""
        self.assertEqual(normalized_digest(WORKFLOW), EXPECTED_CURRENT_DIGEST)
        config = config_pin(self.config_text)
        policy = policy_pin(self.policy_text)
        self.assertIn(pin_state(config, policy), ("PREAPPROVAL", "APPROVED"))
        self.assertEqual(validate_pin_state(config, policy), [])
        self.assertEqual(config, policy)

    def test_preapproval_and_approved_states_are_both_legitimate(self):
        self.assertEqual(
            pin_state(PREVIOUS_APPROVED_DIGEST, PREVIOUS_APPROVED_DIGEST), "PREAPPROVAL"
        )
        self.assertEqual(
            pin_state(EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST), "APPROVED"
        )
        self.assertEqual(
            validate_pin_state(PREVIOUS_APPROVED_DIGEST, PREVIOUS_APPROVED_DIGEST), []
        )
        self.assertEqual(
            validate_pin_state(EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST), []
        )
        # PREAPPROVAL is legitimate but must fail the postapproval consistency
        # check for the new workflow; APPROVED must pass it.
        self.assertTrue(
            validate_postapproval_bindings(
                PREVIOUS_APPROVED_DIGEST, PREVIOUS_APPROVED_DIGEST, EXPECTED_CURRENT_DIGEST
            )
        )
        self.assertEqual(
            validate_postapproval_bindings(
                EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST
            ),
            [],
        )

    def test_mixed_missing_unknown_and_mismatched_pins_are_rejected(self):
        for config, policy in [
            (PREVIOUS_APPROVED_DIGEST, EXPECTED_CURRENT_DIGEST),
            (EXPECTED_CURRENT_DIGEST, PREVIOUS_APPROVED_DIGEST),
            (None, None),
            (EXPECTED_CURRENT_DIGEST, None),
            (None, PREVIOUS_APPROVED_DIGEST),
            ("f" * 64, "f" * 64),
        ]:
            with self.subTest(config=config, policy=policy):
                self.assertNotEqual(pin_state(config, policy), "APPROVED")
                self.assertTrue(validate_pin_state(config, policy))

    def test_stale_pin_negative_control_is_preserved(self):
        self.assertTrue(validate_postapproval_bindings(None, None, EXPECTED_CURRENT_DIGEST))
        self.assertTrue(
            validate_postapproval_bindings(
                EXPECTED_CURRENT_DIGEST, PREVIOUS_APPROVED_DIGEST, EXPECTED_CURRENT_DIGEST
            )
        )
        self.assertTrue(
            validate_postapproval_bindings(
                PREVIOUS_APPROVED_DIGEST, EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST
            )
        )
        self.assertEqual(
            validate_postapproval_bindings(
                EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST, EXPECTED_CURRENT_DIGEST
            ),
            [],
        )

    def test_postapproval_policy_gate_passes_in_isolated_disposable_copy(self):
        """Simulate a Principal re-pin in a disposable copy; real pins untouched."""
        preferred_base = Path("D:/opencode-temp")
        base = str(preferred_base) if preferred_base.is_dir() else None
        temporary = Path(tempfile.mkdtemp(prefix="prometheos-postapproval-", dir=base))
        try:
            repo = temporary / "repo"
            shutil.copytree(
                ROOT,
                repo,
                ignore=shutil.ignore_patterns(".git", ".cargo-target", "node_modules"),
            )
            config_path = repo / "scripts" / "reviewed_config_digests.json"
            policy_path = repo / "scripts" / "local_ci_policy.py"
            config = config_path.read_text(encoding="utf-8")
            policy = policy_path.read_text(encoding="utf-8")
            config = set_pin(config, CONFIG_WORKFLOW_PIN_RE, EXPECTED_CURRENT_DIGEST)
            policy = set_pin(policy, POLICY_WORKFLOW_PIN_RE, EXPECTED_CURRENT_DIGEST)
            policy_path.write_text(policy, encoding="utf-8")
            # Changing local_ci_policy.py requires its own reviewed pin update.
            policy_self_digest = hashlib.sha256(
                policy_path.read_bytes().replace(b"\r\n", b"\n")
            ).hexdigest()
            config = set_pin(config, POLICY_SELF_PIN_RE, policy_self_digest)
            config_path.write_text(config, encoding="utf-8")

            self.assertEqual(config_pin(config_path.read_text(encoding="utf-8")), EXPECTED_CURRENT_DIGEST)
            self.assertEqual(policy_pin(policy_path.read_text(encoding="utf-8")), EXPECTED_CURRENT_DIGEST)

            result = subprocess.run(
                [sys.executable, "scripts/local_ci_policy.py"],
                cwd=repo,
                capture_output=True,
                text=True,
            )
            self.assertEqual(
                result.returncode,
                0,
                f"postapproval policy gate failed: {result.stdout}\n{result.stderr}",
            )
        finally:
            shutil.rmtree(temporary, ignore_errors=True)

        # No real pin was modified by this test.
        self.assertEqual(config_pin(CONFIG_PINS.read_text(encoding="utf-8")), config_pin(self.config_text))
        self.assertEqual(policy_pin(POLICY.read_text(encoding="utf-8")), policy_pin(self.policy_text))


if __name__ == "__main__":
    unittest.main(verbosity=2)
