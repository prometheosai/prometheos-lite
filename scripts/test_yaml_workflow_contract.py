"""YAML red/green and effective here-string contract tests for the workflow."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts import workflow_yaml_contract as contract


def temp_dir(label):
    preferred = Path("D:/opencode-temp")
    return Path(tempfile.mkdtemp(prefix=f"prometheos-{label}-", dir=str(preferred) if preferred.is_dir() else None))


class YamlWorkflowContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.doc = contract.parse_workflow()
        cls.script = contract.build_step_run(cls.doc)
        cls.source = contract.embedded_resolver_source(cls.script)

    def test_green_parse_trigger_and_jobs(self):
        trigger = self.doc.get("on", self.doc.get(True))
        self.assertEqual(trigger, {"workflow_dispatch": None})
        self.assertEqual(set(self.doc["jobs"]), {"windows-owner-registry", "collector-facts"})
        self.assertEqual(self.doc["permissions"], {"contents": "read"})
        for job in self.doc["jobs"].values():
            self.assertEqual(job["runs-on"], "windows-2022")

    def test_effective_resolver_byte_identity(self):
        self.assertEqual(contract.resolver_sha256(self.source), contract.RESOLVER_SHA256)
        self.assertEqual(self.source, contract.reviewed_resolver_source())
        self.assertIn(contract.RESOLVER_SHA256, self.script)

    def test_red_column_zero_python_is_rejected_by_yaml_parser(self):
        tmp = temp_dir("yaml-red")
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        lines = contract.WORKFLOW.read_text(encoding="utf-8").splitlines()
        opening = next(i for i, line in enumerate(lines) if "$resolverSource = @'" in line)
        closing = next(i for i in range(opening + 1, len(lines)) if lines[i].strip() == "'@")
        for i in range(opening + 1, closing + 1):
            lines[i] = lines[i][10:] if lines[i].startswith(" " * 10) else lines[i]
        red = tmp / "column-zero.yml"
        red.write_text("\n".join(lines) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "implicit mapping pair"):
            contract.parse_workflow(red)

    def test_powershell_materializes_exact_resolver(self):
        if not shutil.which("pwsh"):
            self.skipTest("pwsh not available")
        tmp = temp_dir("ps-materialize")
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        ps = tmp / "materialize.ps1"
        ps.write_text(contract.materialization_script(self.script), encoding="utf-8")
        env = dict(os.environ, RUNNER_TEMP=str(tmp))
        result = subprocess.run(
            ["pwsh", "-NoProfile", "-Command", f". '{ps}'; (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $env:RUNNER_TEMP 'resolve_cargo_test_artifact.py')).Hash.ToLowerInvariant()"],
            capture_output=True,
            text=True,
            env=env,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), contract.RESOLVER_SHA256)
        materialized = tmp / "resolve_cargo_test_artifact.py"
        self.assertEqual(materialized.read_bytes().decode("utf-8"), self.source)

    def _run_materialized(self, tmp, payload, exe=None):
        resolver = tmp / "resolver.py"
        resolver.write_text(self.source, encoding="utf-8", newline="\n")
        cargo = tmp / "cargo.json"
        cargo.write_text(payload, encoding="utf-8")
        return subprocess.run(
            [sys.executable, str(resolver), "--input", str(cargo)], capture_output=True, text=True
        )

    def test_materialized_resolver_accepts_valid_cargo_json(self):
        tmp = temp_dir("resolver-valid")
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        exe = tmp / ".cargo-target/debug/build/id/out/prometheos_lite-1.exe"
        exe.parent.mkdir(parents=True, exist_ok=True)
        exe.write_bytes(b"MZ")
        record = {
            "reason": "compiler-artifact",
            "target": {"name": "prometheos_lite", "kind": ["lib"], "test": True},
            "profile": {"test": True},
            "executable": str(exe),
            "filenames": [str(exe)],
        }
        result = self._run_materialized(tmp, json.dumps(record) + "\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(exe.absolute()))

    def test_materialized_resolver_rejects_invalid_artifacts(self):
        tmp = temp_dir("resolver-invalid")
        self.addCleanup(lambda: shutil.rmtree(tmp, ignore_errors=True))
        self.assertNotEqual(self._run_materialized(tmp, "{not json}\n").returncode, 0)
        base = {
            "reason": "compiler-artifact",
            "target": {"name": "prometheos_lite", "kind": ["lib"], "test": True},
            "profile": {"test": True},
        }
        wrong = tmp / "wrong.exe"
        wrong.write_bytes(b"MZ")
        self.assertNotEqual(
            self._run_materialized(
                tmp,
                json.dumps({**base, "target": {**base["target"], "name": "other"}, "executable": str(wrong)}) + "\n",
            ).returncode,
            0,
        )
        self.assertNotEqual(
            self._run_materialized(tmp, json.dumps({**base, "executable": str(tmp / "absent.exe")}) + "\n").returncode,
            0,
        )
        first, second = tmp / "a.exe", tmp / "b.exe"
        first.write_bytes(b"MZ")
        second.write_bytes(b"MZ")
        self.assertNotEqual(
            self._run_materialized(
                tmp,
                json.dumps({**base, "executable": str(first)}) + "\n" + json.dumps({**base, "executable": str(second)}) + "\n",
            ).returncode,
            0,
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
