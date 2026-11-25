"""Deterministic regressions for the Windows provisioning-authority boundary.

No GitHub Actions dispatch, no privileged operation and no Stage 2 acceptance
happens here. REAL_HOST_EVIDENCE=NOT_RUN: the token-elevation observations,
restricted-token execution and ACL behaviour are exercised on a hosted Windows
runner only. This module tests the exact classification function extracted from
the reviewed workflow with pwsh against synthetic token states, and statically
checks that every skip/exit/ownership guard remains in the correct authority
context.

The workflow is read directly from its YAML block scalars with no new runtime
dependency. Set PROMETHEOS_WORKFLOW to exercise an alternative workflow file.
"""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = Path(
    os.environ.get(
        "PROMETHEOS_WORKFLOW",
        str(ROOT / ".github" / "workflows" / "pr341-windows-acceptance.yml"),
    )
)
SID_ELEVATED = "S-1-5-21-100-200-300-1000"
SID_RESTRICTED = "S-1-5-21-100-200-300-1001"

IDENTITY_STEP = "Record elevated Windows identity and prove elevated authority"
ELEVATED_STEP = "Elevated path - owner-registry unit and negative controls"
RESTRICTED_STEP = "Restricted-user path - real token proves unprivileged refusal"
CLEANUP_STEP = "Remove only the restricted identity owned by this run"
FINAL_STEP = "Final authority gate - fail closed on missing elevated token"


def step_run(step_name, text=None):
    """Return the dedented effective PowerShell scalar for one named step."""
    lines = (text if text is not None else WORKFLOW.read_text(encoding="utf-8")).splitlines()
    start = next(
        (
            i
            for i, line in enumerate(lines)
            if re.match(r"^\s*-\s*name:\s*" + re.escape(step_name) + r"\s*$", line)
        ),
        None,
    )
    if start is None:
        raise AssertionError(f"workflow step not found: {step_name}")
    run = next(
        (i for i in range(start + 1, len(lines)) if re.match(r"^\s*run:\s*\|\s*$", lines[i])),
        None,
    )
    if run is None:
        raise AssertionError(f"workflow step has no run block: {step_name}")
    indent = len(lines[run]) - len(lines[run].lstrip())
    body = []
    for line in lines[run + 1 :]:
        if line.strip() and (len(line) - len(line.lstrip())) <= indent:
            break
        body.append(line[indent + 2 :] if line.startswith(" " * (indent + 2)) else line.strip())
    return "\n".join(body)


def optional_step_run(step_name, text):
    try:
        return step_run(step_name, text)
    except AssertionError:
        return ""


def authority_function(script):
    lines = script.splitlines()
    start = next(
        (
            i
            for i, line in enumerate(lines)
            if line.strip().startswith("function Get-AuthorityClass")
        ),
        None,
    )
    if start is None:
        return ""
    depth = 0
    end = None
    for i in range(start, len(lines)):
        depth += lines[i].count("{") - lines[i].count("}")
        if depth == 0:
            end = i
            break
    if end is None:
        raise AssertionError("Get-AuthorityClass has unbalanced braces")
    return "\n".join(lines[start : end + 1])


class TokenAuthorityContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.identity = optional_step_run(IDENTITY_STEP, cls.text)
        cls.elevated = optional_step_run(ELEVATED_STEP, cls.text)
        cls.restricted = optional_step_run(RESTRICTED_STEP, cls.text)
        cls.cleanup = optional_step_run(CLEANUP_STEP, cls.text)
        cls.final = optional_step_run(FINAL_STEP, cls.text)
        cls.function = authority_function(cls.identity)

    def classify(self, token_elevated, admin_member, sid):
        if shutil.which("pwsh") is None:
            self.skipTest("pwsh not available; REAL_HOST_EVIDENCE=NOT_RUN")
        temp_root = os.environ.get("PROMETHEOS_TEST_TEMP") or tempfile.gettempdir()
        tmp = Path(tempfile.mkdtemp(prefix="prometheos-token-", dir=temp_root))
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        script = tmp / "classify.ps1"
        script.write_text(
            self.function
            + "\n"
            + f"Get-AuthorityClass -TokenIsElevated {token_elevated} "
            + f"-AdministratorGroupMember ${str(admin_member).lower()} -Sid '{sid}'\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            ["pwsh", "-NoProfile", "-NonInteractive", "-File", str(script)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    # 1. Administrator-group member with a non-elevated token.
    def test_admin_member_non_elevated_is_not_elevated(self):
        self.assertEqual(self.classify(0, True, SID_ELEVATED), "ADMIN_MEMBER_NON_ELEVATED")
        self.assertIn("EFFECTIVE_PROVISIONING_AUTHORITY=$effective", self.identity)
        self.assertNotIn("Runner token is not elevated", self.identity)

    # 2. Elevated process token.
    def test_elevated_process_token_is_granted_only_with_membership(self):
        self.assertEqual(self.classify(1, True, SID_ELEVATED), "ELEVATED")
        self.assertIn("GetTokenInformation($tokenHandle, 20", self.identity)
        self.assertIn("0x0008", self.identity)
        self.assertIn("$authority -eq 'ELEVATED'", self.identity)

    # 3. Restricted token.
    def test_restricted_token_is_unprivileged_and_proven_from_its_token(self):
        self.assertEqual(self.classify(0, False, SID_RESTRICTED), "UNPRIVILEGED")
        self.assertIn("TOKEN_IS_ELEVATED=$tokenIsElevated", self.restricted)
        self.assertIn("Pr341RestrictedTokenNative", self.restricted)
        self.assertIn("$restrictedElevated -ne '0'", self.restricted)

    # 4. Missing token information.
    def test_missing_token_information_fails_closed(self):
        self.assertEqual(self.classify(-1, True, SID_ELEVATED), "INVALID")
        for guard in (
            "OpenProcessToken did not return a process token",
            "GetTokenInformation(TokenElevation) was incomplete",
            "unexpected TokenIsElevated=",
            "authority classification is INVALID",
        ):
            self.assertIn(guard, self.identity)

    # 5. Incorrect authority classification.
    def test_incorrect_authority_classification_is_not_granted(self):
        self.assertEqual(self.classify(1, False, SID_ELEVATED), "ELEVATED_NON_ADMIN_MEMBER")
        self.assertEqual(self.classify(0, True, ""), "INVALID")
        self.assertEqual(self.classify(1, True, ""), "INVALID")
        self.assertIn("$effective = if ($authority -eq 'ELEVATED')", self.identity)

    # 6. Expected elevated SKIP marker is still checked in the right context.
    def test_expected_elevated_skip_marker_is_checked(self):
        self.assertIn("SKIP: elevated host; privileged provisioning acceptance NOT RUN", self.elevated)
        self.assertIn("Select-String -Path $log -Pattern $expectedSkip -SimpleMatch", self.elevated)
        self.assertLess(
            self.elevated.index("if ($effective -ne 'GRANTED')"),
            self.elevated.index("Expected elevated-host SKIP marker is absent"),
        )
        for name in (
            "reparse_point_or_junction_is_detected",
            "world_writable_directory_is_refused",
            "missing_file_security_cannot_be_verified",
            "acl_setup_failure_is_detected_instead_of_skipped",
            "concurrent_writers_share_one_mutex",
            "monotonic_update_refuses_rollback_revocation_reversal_and_owner_change",
            "revocation_tombstone_cannot_be_dropped_by_omission_then_readded",
        ):
            self.assertIn(name, self.elevated)

    # 7. Missing expected SKIP marker and unexpected SKIP are both refused.
    def test_missing_expected_skip_is_not_a_guard_deletion(self):
        self.assertIn("Expected elevated-host SKIP marker is absent", self.elevated)
        self.assertIn("Unexpected SKIP in elevated path", self.elevated)
        self.assertIn("ELEVATED_PATH=UNIT_CONTROLS_EXECUTED", self.elevated)
        self.assertIn("ELEVATED_PATH=NOT_RUN", self.elevated)

    # 8. Unexpected restricted SKIP.
    def test_unexpected_restricted_skip_is_refused(self):
        self.assertIn("Restricted-token test SKIPPED", self.restricted)
        self.assertIn("$restrictedOutput -match 'SKIP:'", self.restricted)

    # 9. Same elevated/restricted SID.
    def test_same_elevated_and_restricted_sid_is_refused(self):
        self.assertIn("SID=$sid", self.identity)
        self.assertIn("SID=$($id.User.Value)", self.restricted)
        self.assertIn("Restricted token reuses the elevated SID", self.restricted)
        self.assertIn("$restrictedSid -eq $elevatedSid", self.restricted)

    # 10. Restricted token unexpectedly administrator.
    def test_restricted_token_unexpectedly_administrator_is_refused(self):
        self.assertNotEqual(
            self.classify(1, True, SID_RESTRICTED), self.classify(0, False, SID_RESTRICTED)
        )
        self.assertIn("IDENTITY_IS_ADMIN=False", self.restricted)
        self.assertIn("Restricted token is elevated/administrator", self.restricted)
        self.assertIn("-Credential $credential", self.restricted)

    # 11. Nonzero restricted process exit.
    def test_nonzero_restricted_exit_is_refused(self):
        self.assertIn("if ($testProc.ExitCode -ne 0)", self.restricted)
        self.assertIn("Restricted-token test failed", self.restricted)
        self.assertIn("Restricted identity probe failed", self.restricted)

    # 12. Missing test output.
    def test_missing_test_output_is_refused(self):
        self.assertIn("Restricted identity probe produced no evidence", self.restricted)
        self.assertIn("Restricted-token test produced no evidence log", self.restricted)
        self.assertIn("unprivileged_provisioning_is_denied_before_any_write", self.restricted)

    # 13. Failed ACL traversal.
    def test_failed_acl_traversal_is_refused(self):
        for guard in (
            "Failed to grant restricted identity traverse",
            "Missing intermediate test-binary directory",
            "outside the owned workspace",
            "ACL_TRAVERSE=",
            "/grant \"${user}:(RX)\"",
        ):
            self.assertIn(guard, self.restricted)

    # 14. Ownership mismatch during cleanup.
    def test_cleanup_ownership_mismatch_is_refused(self):
        for guard in (
            "OWNED_BY_THIS_RUN=YES",
            "CLEANUP_NO_OWNERSHIP_RECEIPT",
            "CLEANUP_DELETED_OWNED_ACCOUNT",
            "CLEANUP_NO_RECEIPT",
            "^PR341R[0-9A-F]{8}$",
            "Remove-LocalUser -Name $name -ErrorAction Stop",
        ):
            self.assertIn(guard, self.restricted)
        self.assertIn("& $cleanupScript", self.cleanup)
        self.assertIn("NO_ACCOUNT_CREATED_BY_THIS_RUN", self.cleanup)

    # Boundary preservation and explicit NOT_RUN marking.
    def test_manual_only_boundary_and_real_host_evidence_not_run(self):
        self.assertIn("workflow_dispatch:", self.text)
        self.assertNotIn("push:", self.text)
        self.assertNotIn("schedule:", self.text)
        self.assertNotIn("continue-on-error", self.text)
        self.assertIn("ELEVATED_PATH=NOT_RUN", self.final)
        self.assertIn("elevated acceptance evidence NOT_RUN", self.final)
        self.assertIn("FINAL_AUTHORITY_GATE=ELEVATED_CONTEXT_PROVEN", self.final)
        self.assertIn("ELEVATED_PATH_BLOCKER=", self.identity)
        self.assertNotIn("Stage 2 acceptance PASSED", self.text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
