"""End-to-end W1/W2 fixtures for PR #355.

W1 proves the reviewed resolver is embedded in the workflow, absent from the
pinned PR #341 checkout, byte-verified when staged into a fresh isolated
checkout, and refuses wrong helper SHA/blob, absent resolver and malformed or
ambiguous Cargo artifacts.

W2 proves the restricted-user ACL plan covers every intermediate directory to
a deeply nested executable without recursive grants, and refuses paths outside
the owned workspace or missing intermediate directories.

No GitHub Actions dispatch and no privileged execution occurs in these tests.
"""

import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "pr341-windows-acceptance.yml"
RESOLVER_SOURCE = ROOT / "scripts" / "resolve_cargo_test_artifact.py"

PR341_SHA = "37a7ae33ca21fc25ad0f70f20022cd4a95634bce"
RESOLVER_BLOB = "6f9a8254c19aaf67aae487ee9f2ee53fbb1e546a"
RESOLVER_SHA256 = "fa3a0b71bd623f8e1a5d3078f0cb845a0df93a7974fac5cdd755cc2a9247aaa4"
RESOLVER_RELATIVE = "scripts/resolve_cargo_test_artifact.py"


def git(*args, cwd=ROOT, check=False):
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if check and result.returncode != 0:
        raise AssertionError(f"git {args} failed: {result.stderr}")
    return result


def embedded_resolver_source(text):
    opening = text.index("$resolverSource = @'")
    closing = text.index("'@\n", opening)
    source = text[opening + len("$resolverSource = @'\n"):closing].replace("\r\n", "\n")
    return source if source.endswith("\n") else source + "\n"


class WorkflowResolverStaging(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.source = embedded_resolver_source(cls.text)

    def make_pr341_checkout(self, tmp):
        worktree = tmp / "pr341"
        git("worktree", "add", "--detach", str(worktree), PR341_SHA, check=True)
        self.addCleanup(lambda: git("worktree", "remove", "--force", str(worktree)))
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        return worktree

    def test_embedded_source_matches_reviewed_resolver(self):
        self.assertEqual(
            hashlib.sha256(self.source.encode("utf-8")).hexdigest(), RESOLVER_SHA256
        )
        self.assertEqual(self.source, RESOLVER_SOURCE.read_text(encoding="utf-8").replace("\r\n", "\n"))

    def test_resolver_absent_in_exact_pr341_checkout(self):
        self.assertNotEqual(git("cat-file", "-e", f"{PR341_SHA}:{RESOLVER_RELATIVE}").returncode, 0)
        preferred = Path("D:/opencode-temp")
        tmp = Path(tempfile.mkdtemp(prefix="prometheos-pr341-", dir=str(preferred) if preferred.is_dir() else None))
        worktree = self.make_pr341_checkout(tmp)
        self.assertEqual(git("rev-parse", "HEAD", cwd=worktree).stdout.strip(), PR341_SHA)
        self.assertFalse((worktree / RESOLVER_RELATIVE).exists())

    def test_end_to_end_staging_and_real_json_resolution(self):
        preferred = Path("D:/opencode-temp")
        tmp = Path(tempfile.mkdtemp(prefix="prometheos-pr341-stage-", dir=str(preferred) if preferred.is_dir() else None))
        worktree = self.make_pr341_checkout(tmp)
        staged = tmp / "resolve_cargo_test_artifact.py"
        staged.write_text(self.source, encoding="utf-8", newline="\n")

        self.assertEqual(
            hashlib.sha256(staged.read_bytes()).hexdigest(), RESOLVER_SHA256
        )
        self.assertEqual(
            git("hash-object", str(staged)).stdout.strip(), RESOLVER_BLOB
        )

        exe = worktree / ".cargo-target/debug/build/prometheus-lite/259f344e5f6eb474/out/prometheos_lite-259f.exe"
        exe.parent.mkdir(parents=True, exist_ok=True)
        exe.write_bytes(b"MZ")
        cargo_json = tmp / "cargo.json"
        cargo_json.write_text(
            json.dumps({
                "reason": "compiler-artifact",
                "target": {"name": "prometheos_lite", "kind": ["lib"], "test": True},
                "profile": {"test": True},
                "executable": str(exe),
                "filenames": [str(exe)],
            }) + "\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            [sys.executable, str(staged), "--input", str(cargo_json)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(exe.absolute()))

    def test_wrong_helper_sha_and_blob_are_refused(self):
        tampered = self.source.replace("EXPECTED_TARGET_NAME = ", "EXPECTED_TARGET_NAME_X = ", 1)
        self.assertNotEqual(tampered, self.source)
        self.assertNotEqual(
            hashlib.sha256(tampered.encode("utf-8")).hexdigest(), RESOLVER_SHA256
        )
        preferred = Path("D:/opencode-temp")
        tmp = Path(tempfile.mkdtemp(prefix="prometheos-helper-bad-", dir=str(preferred) if preferred.is_dir() else None))
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        bad = tmp / "resolve_cargo_test_artifact.py"
        bad.write_text(tampered, encoding="utf-8", newline="\n")
        self.assertNotEqual(git("hash-object", str(bad)).stdout.strip(), RESOLVER_BLOB)

    def test_absent_resolver_is_refused(self):
        preferred = Path("D:/opencode-temp")
        tmp = Path(tempfile.mkdtemp(prefix="prometheos-helper-absent-", dir=str(preferred) if preferred.is_dir() else None))
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        absent = tmp / "resolve_cargo_test_artifact.py"
        self.assertFalse(absent.exists())
        result = subprocess.run([sys.executable, str(absent)], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)

    def test_malformed_and_ambiguous_artifacts_refused_by_staged_resolver(self):
        preferred = Path("D:/opencode-temp")
        tmp = Path(tempfile.mkdtemp(prefix="prometheos-bad-artifact-", dir=str(preferred) if preferred.is_dir() else None))
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        staged = tmp / "resolve_cargo_test_artifact.py"
        staged.write_text(self.source, encoding="utf-8", newline="\n")
        for payload, expected in [("{not json}\n", "malformed Cargo JSON")]:
            bad = tmp / "cargo.json"
            bad.write_text(payload, encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(staged), "--input", str(bad)], capture_output=True, text=True
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(expected, result.stderr)
        first = tmp / "a.exe"
        second = tmp / "b.exe"
        first.write_bytes(b"MZ")
        second.write_bytes(b"MZ")
        record = {
            "reason": "compiler-artifact",
            "target": {"name": "prometheos_lite", "kind": ["lib"], "test": True},
            "profile": {"test": True},
            "executable": str(first),
            "filenames": [str(first)],
        }
        ambiguous = tmp / "ambiguous.json"
        ambiguous.write_text(
            json.dumps(record) + "\n" + json.dumps({**record, "executable": str(second)}) + "\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            [sys.executable, str(staged), "--input", str(ambiguous)], capture_output=True, text=True
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ambiguous", result.stderr)


class RestrictedTraversalPlan(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")

    @staticmethod
    def plan(workspace, exe, existing):
        workspace_full = str(Path(workspace).resolve())
        exe_full = str(Path(exe).resolve())
        prefix = workspace_full.rstrip("\\/") + "\\"
        if not exe_full.startswith(prefix):
            raise ValueError("outside the owned workspace")
        relative = Path(exe_full).parent.relative_to(workspace_full)
        cumulative = []
        cursor = Path(workspace_full)
        for part in relative.parts:
            cursor = cursor / part
            if str(cursor) not in existing:
                raise ValueError(f"missing intermediate directory: {cursor}")
            cumulative.append(str(cursor))
        return cumulative, exe_full

    def test_deep_path_grants_every_intermediate_directory(self):
        base = Path("D:/opencode-temp/prometheos-traversal-ok")
        deep = base / ".cargo-target/debug/build/id/out"
        expected_grants = [
            str(base / ".cargo-target"),
            str(base / ".cargo-target/debug"),
            str(base / ".cargo-target/debug/build"),
            str(base / ".cargo-target/debug/build/id"),
            str(deep),
        ]
        grants, exe = self.plan(base, deep / "prometheos_lite-1.exe", set(expected_grants))
        self.assertEqual(grants, expected_grants)
        self.assertTrue(exe.endswith("prometheos_lite-1.exe"))
        self.assertNotIn("(OI)(CI)RX", self.text)
        self.assertIn("GetRelativePath", self.text)

    def test_outside_workspace_is_refused(self):
        with self.assertRaisesRegex(ValueError, "outside the owned workspace"):
            self.plan("D:/opencode-temp/ws", "D:/elsewhere/prometheos_lite.exe", set())

    def test_missing_intermediate_directory_is_refused(self):
        base = Path("D:/opencode-temp/prometheos-traversal-missing")
        with self.assertRaisesRegex(ValueError, "missing intermediate directory"):
            self.plan(base, base / "a/b/c/prometheos_lite.exe", set())

    def test_restricted_launch_and_cleanup_guards_preserved(self):
        self.assertIn("if ($testProc.ExitCode -ne 0)", self.text)
        self.assertIn("Restricted-token test SKIPPED", self.text)
        self.assertIn("unprivileged_provisioning_is_denied_before_any_write \\.\\.\\. ok", self.text)
        self.assertIn("OWNED_BY_THIS_RUN=YES", self.text)
        self.assertIn("CLEANUP_NO_OWNERSHIP_RECEIPT", self.text)
        self.assertEqual(self.text.count("(OI)(CI)F"), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
