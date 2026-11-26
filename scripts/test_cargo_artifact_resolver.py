"""Deterministic fixtures for the exact Cargo test-artifact resolver.

These prove that resolution depends on compiler-artifact identity, not on a
hardcoded Cargo output directory or LastWriteTime. No hosted workflow run is
executed by these tests.
"""

import glob
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RESOLVER = ROOT / "scripts" / "resolve_cargo_test_artifact.py"


def artifact(path, *, name="prometheos_lite", kind=None, test=True, profile_test=True,
             reason="compiler-artifact"):
    return {
        "reason": reason,
        "target": {
            "name": name,
            "kind": ["lib"] if kind is None else kind,
            "test": test,
        },
        "profile": {"test": profile_test},
        "executable": str(path),
        "filenames": [str(path)],
    }


class CargoArtifactResolver(unittest.TestCase):
    def setUp(self):
        preferred = Path("D:/opencode-temp")
        base = str(preferred) if preferred.is_dir() else None
        self.tmp = Path(tempfile.mkdtemp(prefix="prometheos-artifact-", dir=base))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))

    def run_resolver(self, records):
        input_path = self.tmp / "cargo.json"
        input_path.write_text(
            "\n".join(json.dumps(record) for record in records) + "\n", encoding="utf-8"
        )
        return subprocess.run(
            [sys.executable, str(RESOLVER), "--input", str(input_path)],
            capture_output=True,
            text=True,
        )

    def make_exe(self, relative):
        path = self.tmp / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"MZ")
        return path

    def test_standard_deps_layout_resolves(self):
        exe = self.make_exe(".cargo-target/debug/deps/prometheos_lite-abc.exe")
        result = self.run_resolver([artifact(exe)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(exe.absolute()))

    def test_nonstandard_hosted_layout_resolves(self):
        exe = self.make_exe(
            ".cargo-target/debug/build/prometheus-lite/259f344e5f6eb474/out/prometheos_lite-259f.exe"
        )
        result = self.run_resolver([artifact(exe)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(exe.absolute()))

    def test_former_hardcoded_deps_assumption_fails_for_hosted_layout(self):
        # The hosted failure: the old workflow only searched debug/deps.
        self.make_exe(
            ".cargo-target/debug/build/prometheus-lite/259f344e5f6eb474/out/prometheos_lite-259f.exe"
        )
        old_search = glob.glob(
            str(self.tmp / ".cargo-target/debug/deps/prometheos_lite-*.exe")
        )
        self.assertEqual(old_search, [], "former hardcoded deps search must find nothing here")
        nonstandard = self.tmp / (
            ".cargo-target/debug/build/prometheus-lite/259f344e5f6eb474/out/prometheos_lite-259f.exe"
        )
        result = self.run_resolver([artifact(nonstandard)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(nonstandard.absolute()))

    def test_missing_executable_fails_closed(self):
        absent = self.tmp / ".cargo-target/debug/deps/prometheos_lite-missing.exe"
        result = self.run_resolver([artifact(absent)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not a real file", result.stderr)

    def test_directory_instead_of_file_fails_closed(self):
        directory = self.tmp / ".cargo-target/debug/deps/prometheos_lite-dir.exe"
        directory.mkdir(parents=True, exist_ok=True)
        result = self.run_resolver([artifact(directory)])
        self.assertNotEqual(result.returncode, 0)

    def test_malformed_json_fails_closed(self):
        input_path = self.tmp / "cargo.json"
        input_path.write_text("{ this is not json }\n", encoding="utf-8")
        result = subprocess.run(
            [sys.executable, str(RESOLVER), "--input", str(input_path)],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed Cargo JSON", result.stderr)

    def test_ambiguous_candidates_fail_closed(self):
        first = self.make_exe(".cargo-target/debug/deps/prometheos_lite-1.exe")
        second = self.make_exe(".cargo-target/debug/build/x/out/prometheos_lite-2.exe")
        result = self.run_resolver([artifact(first), artifact(second)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ambiguous", result.stderr)

    def test_wrong_target_identity_fails_closed(self):
        exe = self.make_exe(".cargo-target/debug/deps/other_lib-1.exe")
        result = self.run_resolver([artifact(exe, name="other_lib")])
        self.assertNotEqual(result.returncode, 0)

    def test_no_artifact_record_fails_closed(self):
        result = self.run_resolver([{"reason": "compiler-message", "message": {}}])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no matching compiler-artifact", result.stderr)

    def test_non_test_profile_fails_closed(self):
        exe = self.make_exe(".cargo-target/debug/deps/prometheos_lite-notest.exe")
        result = self.run_resolver([artifact(exe, profile_test=False)])
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
