#!/usr/bin/env python3
"""Qualification and adversarial regression tests for the autonomous loop."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SKILL = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SKILL / "scripts"))
import checkpoint as ck  # noqa: E402
import common as cm  # noqa: E402
import supervisor as sup  # noqa: E402
import verify as vf  # noqa: E402


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], check=True,
                          capture_output=True, text=True).stdout.strip()


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.email", "test@example.invalid")
        git(self.repo, "config", "user.name", "Test")
        (self.repo / "calc.py").write_text("def add(a, b):\n    return a - b\n", encoding="utf-8")
        (self.repo / "test_calc.py").write_text(
            "import unittest\nfrom calc import add\n\n"
            "class T(unittest.TestCase):\n"
            "    def test_add(self):\n        self.assertEqual(add(2, 3), 5)\n",
            encoding="utf-8")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-q", "-m", "init")
        self.head = git(self.repo, "rev-parse", "HEAD")
        self.tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        # Keep checkpoints outside the verified worktree, as in real use.
        self.worktree_path = Path(self.tmp.name) / "wt"
        git(self.repo, "worktree", "add", "-q", "-b", "fix/test",
            str(self.worktree_path), "HEAD")
        self.worktree = str(self.worktree_path)

    def contract(self, task_id="TASK-001", owner="builder-a", context="ctx-builder", **extra):
        data = {
            "schema": "autonomous-task/2",
            "task_id": task_id, "issue": 0,
            "base_sha": self.head, "tree_sha": self.tree,
            "worktree": self.worktree, "branch": "master",
            "owner": owner, "builder_context": context,
            "allowed_scope": ["calc.py"], "acceptance": ["test passes"],
            "commands": [
                {"id": "unit", "pattern_id": "python-unittest-quiet",
                 "argv": ["python", "-m", "unittest", "-q"]},
                {"id": "diffcheck", "pattern_id": "git-diff-check",
                 "argv": ["git", "diff", "--check"]},
            ],
        }
        data.update(extra)
        path = Path(self.tmp.name) / f"{task_id}.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return data, str(path)

    def claim(self, **extra):
        data, path = self.contract(**extra)
        code = sup.main(["claim", "--repo", str(self.repo), "--contract", path])
        return data, path, code

    def cp(self, task_id="TASK-001"):
        return ck.read(str(self.repo), task_id)

    def advance_to_verify(self, task_id="TASK-001"):
        for state in ("SELECT", "CONTRACT", "WORKTREE", "IMPLEMENT", "VERIFY"):
            self.assertEqual(sup.main(["advance", "--repo", str(self.repo),
                                       "--task-id", task_id, "--to", state]), 0)

    def fix_and_register(self, task_id="TASK-001"):
        self._fix_count = getattr(self, "_fix_count", 0) + 1
        (self.worktree_path / "calc.py").write_text(
            f"def add(a, b):\n    return a + b  # variant {self._fix_count}\n",
            encoding="utf-8")
        git(self.worktree, "add", "calc.py")
        git(self.worktree, "commit", "-q", "-m", "fix add")
        head = git(self.worktree, "rev-parse", "HEAD")
        tree = git(self.worktree, "rev-parse", "HEAD^{tree}")
        self.assertEqual(sup.main(["register-candidate", "--repo", str(self.repo),
                                   "--task-id", task_id, "--candidate", head,
                                   "--tree", tree]), 0)
        return head, tree

    def green_evidence(self, task_id="TASK-001", head=None, tree=None):
        head = head or self.head
        tree = tree or self.tree
        out = str(Path(self.tmp.name) / f"evidence-{task_id}.json")
        code = vf.main(["run", "--repo", str(self.repo),
                        "--contract", str(Path(self.tmp.name) / f"{task_id}.json"),
                        "--output", out, "--candidate", head])
        return out, code

    def _evidence(self):
        head, tree, out = self.to_review()
        return cm.read_json(out), head, tree

    def to_review(self, task_id="TASK-001"):
        self.claim()
        self.advance_to_verify(task_id)
        head, tree = self.fix_and_register(task_id)
        out, code = self.green_evidence(task_id, head, tree)
        self.assertEqual(code, 0, out)
        self.assertEqual(sup.main(["submit-review", "--repo", str(self.repo),
                                   "--task-id", task_id, "--candidate", head,
                                   "--tree", tree, "--evidence", out]), 0)
        return head, tree, out

    def approve(self, head, tree, task_id="TASK-001"):
        return sup.main(["record-review", "--repo", str(self.repo), "--task-id", task_id,
                         "--candidate", head, "--tree", tree, "--verdict", "APPROVE",
                         "--reviewer", "reviewer-b", "--review-context", "ctx-reviewer"])


class DiscoveryTests(Base):
    def test_discover_reports_head_and_tree(self):
        out = subprocess.run([sys.executable, str(SKILL / "scripts" / "supervisor.py"),
                              "discover", "--repo", str(self.repo)],
                             check=True, capture_output=True, text=True).stdout
        data = json.loads(out)
        self.assertEqual((data["head_sha"], data["tree_sha"]), (self.head, self.tree))

    def test_claim_binds_contract_digest(self):
        _, _, code = self.claim()
        self.assertEqual(code, 0)
        self.assertEqual(len(self.cp()["contract_sha256"]), 64)


class SupervisorBypassTests(Base):
    def test_advance_bypass_to_ready_refused(self):
        self.claim(); self.advance_to_verify()
        self.assertEqual(sup.main(["advance", "--repo", str(self.repo),
                                   "--task-id", "TASK-001", "--to", "READY"]), 2)

    def test_advance_bypass_to_done_refused(self):
        self.claim(); self.advance_to_verify()
        self.assertEqual(sup.main(["advance", "--repo", str(self.repo),
                                   "--task-id", "TASK-001", "--to", "DONE"]), 2)

    def test_checkpoint_state_forgery_refused(self):
        self.claim(); self.advance_to_verify()
        with self.assertRaises(cm.LoopError):
            ck.patch(str(self.repo), self.cp(), {"state": "DONE"})
        cp = self.cp(); cp["state"] = "DONE"  # direct local edit
        with self.assertRaises(cm.LoopError):
            ck.validate(cp)

    def test_submit_review_without_evidence_refused(self):
        self.claim(); self.advance_to_verify()
        head, tree = self.fix_and_register()
        with self.assertRaises(SystemExit):
            sup.main(["submit-review", "--repo", str(self.repo), "--task-id", "TASK-001",
                      "--candidate", head, "--tree", tree])

    def test_complete_without_approval_refused(self):
        self.claim(); self.advance_to_verify()
        head, tree = self.fix_and_register()
        out, code = self.green_evidence(head=head, tree=tree)
        self.assertEqual(code, 0)
        code = sup.main(["complete", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--evidence", out])
        self.assertEqual(code, 2)


class EvidenceTests(Base):
    def _evidence(self):
        head, tree, out = self.to_review()
        return cm.read_json(out), head, tree

    def test_forged_evidence_refused(self):
        ev, head, tree = self._evidence()
        ev["summary"]["passed"] = 99
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_all_skipped_not_green(self):
        ev, head, tree = self._evidence()
        for r in ev["results"]:
            r["status"] = "skipped"
        ev["summary"] = vf.result_counts(ev["results"])
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_no_pass_not_green(self):
        ev, head, tree = self._evidence()
        ev["results"] = []
        ev["summary"] = vf.result_counts(ev["results"])
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_bad_policy_digest_refused(self):
        ev, head, tree = self._evidence()
        ev["policy_sha256"] = "0" * 64
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_dirty_worktree_invalidates_evidence(self):
        head, tree, out = self.to_review()
        (self.worktree_path / "untracked.txt").write_text("dirty", encoding="utf-8")
        self.assertFalse(cm.worktree_clean(self.worktree))
        code = sup.main(["complete", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--evidence", out])
        self.assertEqual(code, 2)
        (self.worktree_path / "untracked.txt").unlink()

    def test_stale_approval_refused(self):
        head, tree, out = self.to_review()
        self.assertEqual(self.approve(head, tree), 0)
        path = ck.checkpoint_path(str(self.repo), "TASK-001")
        forged = json.loads(path.read_text(encoding="utf-8"))
        forged["review_result"]["candidate_sha"] = "0" * 40
        path.write_text(json.dumps(forged), encoding="utf-8")
        code = sup.main(["complete", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--evidence", out])
        self.assertEqual(code, 2)

    def test_stale_review_after_new_commit_refused(self):
        self.claim(); self.advance_to_verify()
        head, tree = self.fix_and_register()
        out, code = self.green_evidence(head=head, tree=tree)
        self.assertEqual(code, 0)
        (self.worktree_path / "calc.py").write_text(
            "def add(a, b):\n    return a + b + 0\n", encoding="utf-8")
        git(self.worktree, "add", "calc.py")
        git(self.worktree, "commit", "-q", "-m", "later")
        code = sup.main(["submit-review", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--candidate", head, "--tree", tree, "--evidence", out])
        self.assertEqual(code, 2)


class CommandGuardTests(Base):
    def test_arbitrary_python_execution_rejected(self):
        policy = vf.load_policy()[0]
        with self.assertRaises(vf.VerifyError):
            vf.authorize({"pattern_id": "python-unittest-quiet",
                          "argv": [sys.executable, "-c", "print(1)"]},
                         policy, [str(self.repo)])

    def test_unsafe_git_options_rejected(self):
        policy = vf.load_policy()[0]
        with self.assertRaises(vf.VerifyError):
            vf.authorize({"pattern_id": "git-diff-check",
                          "argv": ["git", "-C", "/", "diff", "--check"]},
                         policy, [str(self.repo)])

    def test_unknown_pattern_rejected(self):
        policy = vf.load_policy()[0]
        with self.assertRaises(vf.VerifyError):
            vf.authorize({"pattern_id": "arbitrary", "argv": ["true"]},
                         policy, [str(self.repo)])

    def test_executable_spoofing_rejected(self):
        fake = self.worktree_path / "python.bat"
        fake.write_text("@echo spoofed\n", encoding="utf-8")
        old_path = os.environ.get("PATH", "")
        os.environ["PATH"] = str(self.worktree) + os.pathsep + old_path
        try:
            with self.assertRaises(cm.LoopError):
                cm.trusted_executable("python", [self.worktree])
        finally:
            os.environ["PATH"] = old_path

    def test_bounded_output_terminates_while_running(self):
        run = cm.run_bounded(
            [sys.executable, "-c", "print('x'*200000)"], self.worktree,
            timeout=10, max_output=1024)
        self.assertTrue(run["capped"])
        self.assertIn(run["termination"], ("output_cap", "process_killed",
                                           "process_only_terminated"))


class StateAndRepairTests(Base):
    def test_illegal_transition_refused(self):
        self.claim()
        self.assertEqual(sup.main(["advance", "--repo", str(self.repo),
                                   "--task-id", "TASK-001", "--to", "DONE"]), 2)

    def test_builder_cannot_review(self):
        head, tree, _ = self.to_review()
        code = sup.main(["record-review", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--candidate", head, "--tree", tree, "--verdict", "APPROVE",
                         "--reviewer", "builder-a", "--review-context", "ctx-other"])
        self.assertEqual(code, 2)

    def test_review_context_must_differ(self):
        head, tree, _ = self.to_review()
        code = sup.main(["record-review", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--candidate", head, "--tree", tree, "--verdict", "APPROVE",
                         "--reviewer", "reviewer-b", "--review-context", "ctx-builder"])
        self.assertEqual(code, 2)

    def test_generic_advance_cannot_enter_repair(self):
        head, tree, _ = self.to_review()
        code = sup.main(["advance", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--to", "REPAIR"])
        self.assertEqual(code, 2)
        self.assertEqual(self.cp()["state"], "REVIEW")
        self.assertEqual(self.cp()["attempt_count"], 0)

    def test_checkpoint_init_does_not_overwrite(self):
        _, path, code = self.claim()
        self.assertEqual(code, 0)
        # init on an existing task id is refused by the checkpoint store
        args = ["init", "--repo", str(self.repo), "--task-id", "TASK-001",
                     "--issue", "0", "--base-sha", self.head, "--tree-sha", self.tree,
                     "--worktree", self.worktree, "--branch", "master",
                     "--owner", "builder-a", "--builder-context", "ctx-builder",
                     "--contract-sha256", "0" * 64]
        self.assertEqual(ck.main(args), 2)

    def test_repair_budget_enforcement(self):
        head, tree, out = self.to_review()
        for _ in range(3):
            code = sup.main(["record-review", "--repo", str(self.repo),
                             "--task-id", "TASK-001", "--candidate", head, "--tree", tree,
                             "--verdict", "REPAIR_REQUIRED", "--reviewer", "r",
                             "--review-context", "ctx-r"])
            self.assertEqual(code, 0)
            if self.cp()["state"] == "BLOCKED":
                break
            head2, tree2 = self.fix_and_register()
            self.assertEqual(sup.main(["advance", "--repo", str(self.repo),
                                       "--task-id", "TASK-001", "--to", "VERIFY"]), 0)
            out, code = self.green_evidence(head=head2, tree=tree2)
            self.assertEqual(code, 0)
            self.assertEqual(sup.main(["submit-review", "--repo", str(self.repo),
                                       "--task-id", "TASK-001", "--candidate", head2,
                                       "--tree", tree2, "--evidence", out]), 0)
            head, tree = head2, tree2
        self.assertEqual(self.cp()["state"], "BLOCKED")

    def test_already_done_noop(self):
        _, _, _ = self.claim()
        cp = self.cp(); cp["state"] = "DONE"; cp["evidence_sha256"] = "a" * 64
        cp["review_result"] = {"verdict": "APPROVE", "candidate_sha": self.head,
                               "tree_sha": self.tree, "reviewer": "r",
                               "review_context": "c"}
        ck.write(str(self.repo), cp)
        self.assertEqual(sup.main(["recover", "--repo", str(self.repo),
                                   "--task-id", "TASK-001"]), 0)

    def test_recover_wrong_sha_refused(self):
        self.claim()
        cp = self.cp(); cp["candidate_sha"] = "0" * 40; cp["tree_sha"] = None
        ck.write(str(self.repo), cp)
        self.assertEqual(sup.main(["recover", "--repo", str(self.repo),
                                   "--task-id", "TASK-001"]), 2)

    def test_recover_missing_worktree_refused(self):
        self.claim()
        cp = self.cp(); cp["worktree"] = str(Path(self.tmp.name) / "missing")
        ck.write(str(self.repo), cp)
        self.assertEqual(sup.main(["recover", "--repo", str(self.repo),
                                   "--task-id", "TASK-001"]), 2)

    def test_partial_checkpoint_json_refused(self):
        self.claim()
        ck.checkpoint_path(str(self.repo), "TASK-001").write_text("{broken", encoding="utf-8")
        with self.assertRaises(ValueError):
            ck.read(str(self.repo), "TASK-001")


PROTECTED = {"REVIEW", "READY", "SAVE_EVIDENCE", "DONE"}


def assert_protected_gates_closed():
    """Model-check invariant: generic advance cannot authorize a protected gate."""
    for source, targets in sup.TRANSITIONS.items():
        overlap = set(targets) & PROTECTED
        if overlap:
            raise AssertionError(f"generic advance exposes {source} -> {sorted(overlap)}")


class RiskBasedVerificationTests(Base):
    def test_state_machine_model_check(self):
        assert_protected_gates_closed()
        self.assertTrue({"VERIFY", "REVIEW", "READY", "DONE"} <= set(sup.TRANSITIONS))

    def test_seeded_property_sequences_never_forge_completion(self):
        import random
        rng = random.Random(20261122)
        actions = ("advance", "register", "patch", "submit")
        for _ in range(20):
            task_id = f"PROP-{rng.randrange(100000)}"
            _, path, code = self.claim(task_id=task_id)
            self.assertEqual(code, 0)
            for _step in range(8):
                action = rng.choice(actions)
                if action == "advance":
                    sup.main(["advance", "--repo", str(self.repo), "--task-id", task_id,
                              "--to", rng.choice(list(ck.STATES))])
                elif action == "register":
                    sup.main(["register-candidate", "--repo", str(self.repo),
                              "--task-id", task_id, "--candidate", self.head,
                              "--tree", self.tree])
                elif action == "patch":
                    try:
                        ck.patch(str(self.repo), self.cp(task_id), {"state": "DONE"})
                    except cm.LoopError:
                        pass
                else:
                    sup.main(["submit-review", "--repo", str(self.repo), "--task-id", task_id,
                              "--candidate", self.head, "--tree", self.tree,
                              "--evidence", str(Path(self.tmp.name) / "missing.json")])
            cp = self.cp(task_id)
            self.assertNotEqual(cp["state"], "DONE")

    def test_mutation_detection_protected_gate(self):
        original = dict(sup.TRANSITIONS)
        try:
            sup.TRANSITIONS["VERIFY"] = original["VERIFY"] | {"READY"}
            with self.assertRaises(AssertionError):
                assert_protected_gates_closed()
        finally:
            sup.TRANSITIONS.clear()
            sup.TRANSITIONS.update(original)
        assert_protected_gates_closed()

    def test_fault_injection_timeout_terminates(self):
        run = cm.run_bounded([sys.executable, "-c", "import time; time.sleep(30)"],
                             self.worktree, timeout=1, max_output=1024)
        self.assertEqual(run["termination"], "timeout")
        self.assertLess(run["duration_seconds"], 5)


class HardeningTests(Base):
    def test_shell_metacharacters_rejected(self):
        policy = vf.load_policy()[0]
        with self.assertRaises(vf.VerifyError):
            vf.authorize({"pattern_id": "python-unittest-quiet",
                          "argv": ["python", "-m", "unittest; curl x"]},
                         policy, [self.worktree])

    def test_path_escape_rejected(self):
        policy = vf.load_policy()[0]
        with self.assertRaises(vf.VerifyError):
            vf.authorize({"pattern_id": "python-unittest-quiet",
                          "argv": ["python", "-m", "../../etc/passwd"]},
                         policy, [self.worktree])

    def test_mandatory_sandbox_fails_closed_when_unavailable(self):
        import common as _cm
        policy, _ = vf.load_policy()
        policy["sandbox"] = {"mode": "unavailable", "require_sandbox": True,
                             "filesystem_isolation_enforced": False,
                             "network_isolation_enforced": False}
        original = vf.POLICY
        probe = Path(self.tmp.name) / "policy-probe.json"
        probe.write_text(json.dumps(policy), encoding="utf-8")
        try:
            vf.POLICY = probe
            with self.assertRaises(vf.VerifyError):
                vf.load_policy()
        finally:
            vf.POLICY = original

    def test_evidence_from_another_worktree_rejected(self):
        ev, head, tree = self._evidence()
        ev["worktree"] = str(Path(self.tmp.name) / "other-wt")
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_evidence_from_another_repository_rejected(self):
        ev, head, tree = self._evidence()
        ev["repository"] = str(Path(self.tmp.name) / "other-repo")
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_duplicate_check_id_rejected(self):
        ev, head, tree = self._evidence()
        ev["results"].append(dict(ev["results"][0]))
        ev["summary"] = vf.result_counts(ev["results"])
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_unrecognized_status_rejected(self):
        ev, head, tree = self._evidence()
        ev["results"][0]["status"] = "maybe"
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_mandatory_skipped_rejected(self):
        ev, head, tree = self._evidence()
        ev["results"][0]["status"] = "skipped"
        ev["summary"] = vf.result_counts(ev["results"])
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_missing_negative_control_rejected(self):
        ev, head, tree = self._evidence()
        ev["required_negative_control_ids"] = ["neg-guard"]
        with self.assertRaises(vf.VerifyError):
            vf.validate_evidence(ev, vf.load_policy()[1], candidate=head, tree=tree)

    def test_partial_evidence_file_refused(self):
        head, tree, out = self.to_review()
        Path(out).write_text("{partial", encoding="utf-8")
        self.assertEqual(sup.main(["submit-review", "--repo", str(self.repo),
                                   "--task-id", "TASK-001", "--candidate", head,
                                   "--tree", tree, "--evidence", out]), 2)

    def test_reviewer_cannot_mutate_candidate(self):
        head, tree, out = self.to_review()
        (self.worktree_path / "calc.py").write_text("def add(a, b):\n    return 0\n",
                                                    encoding="utf-8")
        code = sup.main(["record-review", "--repo", str(self.repo), "--task-id", "TASK-001",
                         "--candidate", head, "--tree", tree, "--verdict", "APPROVE",
                         "--reviewer", "r", "--review-context", "ctx-r"])
        self.assertEqual(code, 2)

    def test_benchmark_reports_inconclusive_without_baseline(self):
        sys.path.insert(0, str(SKILL / "scripts"))
        import benchmark
        result = benchmark.analyze([], None)
        self.assertEqual(result["throughput_status"], "INCONCLUSIVE")
        self.assertIsNone(result["measured_speedup"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
