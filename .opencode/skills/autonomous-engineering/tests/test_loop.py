#!/usr/bin/env python3
"""Qualification tests and negative controls for the autonomous engineering loop."""
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
        (self.repo / "README.md").write_text("hello\n", encoding="utf-8")
        git(self.repo, "add", "README.md")
        git(self.repo, "commit", "-q", "-m", "init")
        self.head = git(self.repo, "rev-parse", "HEAD")
        self.tree = git(self.repo, "rev-parse", "HEAD^{tree}")
        self.worktree = str(self.repo)

    def contract(self, task_id="TASK-001", owner="builder-a", context="ctx-builder", **extra):
        data = {
            "schema": "autonomous-task/2",
            "task_id": task_id, "issue": 0,
            "base_sha": self.head, "tree_sha": self.tree,
            "worktree": self.worktree, "branch": "main",
            "owner": owner, "builder_context": context,
            "allowed_scope": ["README.md"], "acceptance": ["test passes"],
            "commands": [{"id": "unit", "argv": [sys.executable, "-c", "print(1)"]}],
        }
        data.update(extra)
        path = Path(self.tmp.name) / f"{task_id}.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return data, str(path)

    def claim(self, **extra):
        data, path = self.contract(**extra)
        code = sup.main(["claim", "--repo", str(self.repo), "--contract", path])
        return data, path, code

    def checkpoint(self, task_id="TASK-001"):
        return ck.read(str(self.repo), task_id)


class DiscoveryTests(Base):
    def test_discover_reports_head_and_tree(self):
        out = subprocess.run(
            [sys.executable, str(SKILL / "scripts" / "supervisor.py"),
             "discover", "--repo", str(self.repo)],
            check=True, capture_output=True, text=True,
        ).stdout
        data = json.loads(out)
        self.assertEqual(data["head_sha"], self.head)
        self.assertEqual(data["tree_sha"], self.tree)

    def test_claim_creates_checkpoint(self):
        data, _, code = self.claim()
        self.assertEqual(code, 0)
        cp = self.checkpoint()
        self.assertEqual(cp["base_sha"], self.head)
        self.assertEqual(cp["tree_sha"], self.tree)
        self.assertEqual(cp["state"], "DISCOVER")


class NegativeControls(Base):
    def test_unknown_checkpoint_field_rejected(self):
        _, _, _ = self.claim()
        cp = self.checkpoint()
        cp["injected_secret"] = "x"
        with self.assertRaises(cm.LoopError):
            ck.write(str(self.repo), cp)

    def test_bad_sha_rejected(self):
        _, _, _ = self.claim()
        cp = self.checkpoint()
        cp["candidate_sha"] = "deadbeef"
        with self.assertRaises(cm.LoopError):
            ck.write(str(self.repo), cp)

    def test_duplicate_claim_different_owner_refused(self):
        _, path_a, _ = self.claim(owner="builder-a")
        code = sup.main(["claim", "--repo", str(self.repo), "--contract", path_a])
        self.assertEqual(code, 0)  # same owner reclaims
        data, path_b = self.contract(owner="builder-b")  # same task id, other owner
        code = sup.main(["claim", "--repo", str(self.repo), "--contract", path_b])
        self.assertEqual(code, 2)

    def test_already_done_is_noop(self):
        _, path, _ = self.claim()
        cp = self.checkpoint()
        cp["state"] = "DONE"
        ck.write(str(self.repo), cp)
        code = sup.main(["claim", "--repo", str(self.repo), "--contract", path])
        self.assertEqual(code, 0)
        self.assertEqual(self.checkpoint()["state"], "DONE")

    def test_illegal_transition_refused(self):
        self.claim()
        code = sup.main(["advance", "--repo", str(self.repo),
                         "--task-id", "TASK-001", "--to", "DONE"])
        self.assertEqual(code, 2)

    def test_reviewer_cannot_be_builder(self):
        self.claim()
        cp = self.checkpoint()
        cp["state"] = "REVIEW"
        ck.write(str(self.repo), cp)
        code = sup.main(["record-review", "--repo", str(self.repo),
                         "--task-id", "TASK-001", "--candidate", self.head,
                         "--tree", self.tree, "--verdict", "APPROVE",
                         "--reviewer", "builder-a", "--review-context", "ctx-other"])
        self.assertEqual(code, 2)

    def test_review_context_must_differ(self):
        self.claim()
        cp = self.checkpoint()
        cp["state"] = "REVIEW"
        ck.write(str(self.repo), cp)
        code = sup.main(["record-review", "--repo", str(self.repo),
                         "--task-id", "TASK-001", "--candidate", self.head,
                         "--tree", self.tree, "--verdict", "APPROVE",
                         "--reviewer", "reviewer-b", "--review-context", "ctx-builder"])
        self.assertEqual(code, 2)

    def test_stale_review_refused_after_new_commit(self):
        self.claim()
        (self.repo / "README.md").write_text("changed\n", encoding="utf-8")
        git(self.repo, "add", "README.md")
        git(self.repo, "commit", "-q", "-m", "second")
        cp = self.checkpoint()
        cp["state"] = "REVIEW"
        ck.write(str(self.repo), cp)
        code = sup.main(["record-review", "--repo", str(self.repo),
                         "--task-id", "TASK-001", "--candidate", self.head,
                         "--tree", self.tree, "--verdict", "APPROVE",
                         "--reviewer", "reviewer-b", "--review-context", "ctx-other"])
        self.assertEqual(code, 2)

    def test_repair_budget_exhaustion_blocks(self):
        self.claim()
        cp = self.checkpoint()
        cp["state"] = "REVIEW"
        cp["attempt_count"] = 2
        ck.write(str(self.repo), cp)
        code = sup.main(["record-review", "--repo", str(self.repo),
                         "--task-id", "TASK-001", "--candidate", self.head,
                         "--tree", self.tree, "--verdict", "REPAIR_REQUIRED",
                         "--reviewer", "reviewer-b", "--review-context", "ctx-other"])
        self.assertEqual(code, 0)
        self.assertEqual(self.checkpoint()["state"], "BLOCKED")

    def test_recover_wrong_sha_refused(self):
        self.claim()
        cp = self.checkpoint()
        cp["candidate_sha"] = "0" * 40
        cp["tree_sha"] = None
        ck.write(str(self.repo), cp)
        code = sup.main(["recover", "--repo", str(self.repo), "--task-id", "TASK-001"])
        self.assertEqual(code, 2)

    def test_recover_missing_worktree_refused(self):
        self.claim()
        cp = self.checkpoint()
        cp["worktree"] = str(Path(self.tmp.name) / "missing")
        ck.write(str(self.repo), cp)
        code = sup.main(["recover", "--repo", str(self.repo), "--task-id", "TASK-001"])
        self.assertEqual(code, 2)

    def test_partial_checkpoint_json_refused(self):
        self.claim()
        path = ck.checkpoint_path(str(self.repo), "TASK-001")
        path.write_text("{broken", encoding="utf-8")
        with self.assertRaises((ValueError, cm.LoopError)):
            ck.read(str(self.repo), "TASK-001")


class VerificationTests(Base):
    def test_allowlist_rejects_unapproved_command(self):
        data, path = self.contract(commands=[
            {"id": "bad", "argv": ["curl", "http://example.invalid"]}
        ])
        with self.assertRaises(cm.LoopError):
            vf.guard(["curl", "http://example.invalid"], vf.load_policy()[0])

    def test_verify_binds_head_tree_and_policy(self):
        data, path = self.contract()
        out = str(Path(self.tmp.name) / "evidence.json")
        code = vf.main(["run", "--repo", str(self.repo), "--contract", path,
                        "--output", out, "--candidate", self.head])
        self.assertEqual(code, 0)
        ev = cm.read_json(out)
        self.assertEqual(ev["candidate_sha"], self.head)
        self.assertEqual(ev["tree_sha"], self.tree)
        self.assertEqual(ev["pre_head"], ev["post_head"])
        self.assertEqual(ev["pre_tree"], ev["post_tree"])
        self.assertEqual(len(ev["policy_sha256"]), 64)

    def test_untested_check_is_not_green(self):
        _, path = self.contract()
        out = str(Path(self.tmp.name) / "evidence.json")
        code = vf.main(["run", "--repo", str(self.repo), "--contract", path,
                        "--output", out, "--only", "nonexistent"])
        self.assertEqual(code, 2)

    def test_missing_worktree_is_refused(self):
        _, path = self.contract(worktree=str(Path(self.tmp.name) / "missing"))
        out = str(Path(self.tmp.name) / "evidence.json")
        code = vf.main(["run", "--repo", str(self.repo), "--contract", path,
                        "--output", out])
        self.assertNotEqual(code, 0)

    def test_submit_review_requires_green_evidence(self):
        self.claim()
        cp = self.checkpoint()
        cp["state"] = "VERIFY"
        ck.write(str(self.repo), cp)
        _, path = self.contract()
        out = str(Path(self.tmp.name) / "evidence.json")
        vf.main(["run", "--repo", str(self.repo), "--contract", path,
                 "--output", out, "--candidate", self.head])
        code = sup.main(["submit-review", "--repo", str(self.repo),
                         "--task-id", "TASK-001", "--candidate", self.head,
                         "--tree", self.tree, "--evidence", out])
        self.assertEqual(code, 0)
        self.assertEqual(self.checkpoint()["state"], "REVIEW")


if __name__ == "__main__":
    unittest.main(verbosity=2)
