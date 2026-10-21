#!/usr/bin/env python3
"""Adversarial regressions for the repository policy gate.

``scripts/local_ci_policy.py`` is a fail-closed control: it must reject
what it exists to reject, and it must never pass silently. Until now the
gate was only ever *executed* by the verification suite, never tested
against an adversarial input — which is how a missing pin file could
raise an unhandled traceback and an empty pin set could pass unnoticed.

Each case builds a throwaway sandbox (a copy of the real gate plus the
real pinned surfaces) and runs the gate against it. The repository itself
is never modified.

Run: python scripts/test_local_ci_policy.py
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "scripts" / "local_ci_policy.py"
PIN_REL = "scripts/reviewed_config_digests.json"

FAILURES: list[str] = []


@contextmanager
def sandbox() -> Iterator[Path]:
    """A disposable repository root holding the real gate and pin set."""
    work = Path(tempfile.mkdtemp(prefix="prometheos-policy-"))
    try:
        (work / "scripts").mkdir(parents=True, exist_ok=True)
        (work / "src" / "api").mkdir(parents=True, exist_ok=True)
        shutil.copy2(GATE, work / "scripts" / "local_ci_policy.py")
        pins = json.loads((ROOT / PIN_REL).read_text(encoding="utf-8"))["files"]
        for relative in pins:
            source = ROOT / relative
            target = work / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        shutil.copy2(ROOT / PIN_REL, work / PIN_REL)
        yield work
    finally:
        shutil.rmtree(work, ignore_errors=True)


def run_gate(root: Path) -> tuple[int, str]:
    result = subprocess.run(
        [sys.executable, str(root / "scripts" / "local_ci_policy.py")],
        capture_output=True,
        text=True,
        cwd=str(root),
    )
    return result.returncode, result.stdout + result.stderr


def expect_pass(label: str, root: Path) -> None:
    code, output = run_gate(root)
    if code == 0:
        print(f"PASS accept: {label}")
    else:
        FAILURES.append(label)
        print(f"FAIL accept: {label} was REJECTED (exit {code})")
        for line in output.strip().splitlines():
            print(f"    {line}")


def expect_fail(label: str, root: Path, *must_contain: str) -> None:
    code, output = run_gate(root)
    if code == 0:
        FAILURES.append(label)
        print(f"FAIL reject: {label} was ACCEPTED (fails open)")
        return
    missing = [needle for needle in must_contain if needle not in output]
    if "Traceback" in output:
        missing.append("no unhandled traceback")
    if missing:
        FAILURES.append(label)
        print(f"FAIL reject: {label} did not report {missing}")
        for line in output.strip().splitlines():
            print(f"    {line}")
        return
    print(f"PASS reject: {label}")


def main() -> int:
    with sandbox() as base:
        expect_pass("baseline: real pin set, unmodified surfaces", base)

    # -- the pin file itself is the reviewed record -----------------------
    with sandbox() as root:
        (root / PIN_REL).unlink()
        expect_fail(
            "missing pin file",
            root,
            "missing",
            "cannot be reviewed without it",
        )

    with sandbox() as root:
        (root / PIN_REL).write_text("{ this is not json", encoding="utf-8")
        expect_fail(
            "malformed pin file reports a message, not a traceback",
            root,
            "unreadable pin file",
        )

    with sandbox() as root:
        (root / PIN_REL).write_text('{"schemaVersion": "1.0.0"}', encoding="utf-8")
        expect_fail(
            "pin file without a 'files' object",
            root,
            "unreadable pin file",
        )

    with sandbox() as root:
        (root / PIN_REL).write_text("[]", encoding="utf-8")
        expect_fail(
            "pin file that is not a JSON object",
            root,
            "must be a JSON object",
        )

    with sandbox() as root:
        (root / PIN_REL).write_text(
            json.dumps({"schemaVersion": "1.0.0", "files": {}}, indent=2), encoding="utf-8"
        )
        expect_fail(
            "empty pin set is a violation, not a silent pass",
            root,
            "pin set is empty",
        )

    with sandbox() as root:
        (root / PIN_REL).write_text(
            json.dumps({"schemaVersion": "1.0.0", "files": {}}, indent=2), encoding="utf-8"
        )
        (root / "frontend" / "postcss.config.js").write_text(
            "module.exports = {}\n// contaminated\n", encoding="utf-8"
        )
        expect_fail(
            "empty pin set cannot mask a contaminated pinned surface",
            root,
            "pin set is empty",
        )

    # -- pinned surfaces ----------------------------------------------------
    with sandbox() as root:
        (root / "frontend" / "postcss.config.js").write_text(
            "module.exports = {}\n// changed\n", encoding="utf-8"
        )
        expect_fail(
            "changed pinned surface without a re-pin",
            root,
            "changed without a reviewed pin update",
        )

    with sandbox() as root:
        (root / "frontend" / "postcss.config.js").unlink()
        expect_fail(
            "deleted pinned surface",
            root,
            "pinned executable configuration is missing",
        )

    with sandbox() as root:
        (root / "frontend" / "package.json").write_text('{"name":"tampered"}', encoding="utf-8")
        expect_fail(
            "changed bundler input that the runner executes",
            root,
            "changed without a reviewed pin update",
        )

    # -- duplicate JSON keys ------------------------------------------------
    with sandbox() as root:
        (root / "config.json").write_text('{"a": 1, "a": 2}', encoding="utf-8")
        expect_fail(
            "duplicate key in a repository JSON document",
            root,
            "duplicate key",
        )

    with sandbox() as root:
        target = root / "tests" / "fixtures" / "slice3" / "invalid"
        target.mkdir(parents=True, exist_ok=True)
        (target / "new-invalid.json").write_text('{"b": 1, "b": 2}', encoding="utf-8")
        expect_fail(
            "new invalid fixture inside a previously exempt subtree",
            root,
            "duplicate key",
        )

    with sandbox() as root:
        target = root / "tests" / "fixtures" / "slice3" / "invalid"
        target.mkdir(parents=True, exist_ok=True)
        (target / "duplicate-key.json").write_text('{"a": 1, "a": 2}', encoding="utf-8")
        expect_pass(
            "the explicitly exempt fixture is not reported",
            root,
        )

    # -- vendored byte fidelity ---------------------------------------------
    with sandbox() as root:
        vendored = root / "vendored" / "soma" / "v1.1"
        vendored.mkdir(parents=True, exist_ok=True)
        (vendored / "payload.bin").write_bytes(b"line\r\nline\r\n")
        expect_fail(
            "CR bytes in a vendored file",
            root,
            "carries CR bytes",
        )

    with sandbox() as root:
        vendored = root / "vendored" / "soma" / "v1.1"
        vendored.mkdir(parents=True, exist_ok=True)
        (vendored / "payload.bin").write_bytes(b"line\nline\n")
        expect_pass("LF-only vendored file", root)

    # -- the API LLM boundary scans every root that exists -------------------
    with sandbox() as root:
        (root / "src" / "api" / "client.rs").write_text(
            "fn go() { llm_client.generate(); }\n", encoding="utf-8"
        )
        expect_fail(
            "LLM call boundary violation under src/api",
            root,
            "llm_client.generate",
        )

    with sandbox() as root:
        (root / "api").mkdir(parents=True, exist_ok=True)
        (root / "api" / "client.rs").write_text(
            "fn go() { llm_client.generate(); }\n", encoding="utf-8"
        )
        expect_fail(
            "LLM call boundary violation under a top-level api/",
            root,
            "llm_client.generate",
        )

    # -- hosted CI and LFS independence --------------------------------------
    with sandbox() as root:
        workflows = root / ".github" / "workflows"
        workflows.mkdir(parents=True, exist_ok=True)
        (workflows / "ci.yml").write_text("on: push\n", encoding="utf-8")
        expect_fail(
            "hosted CI workflow appears",
            root,
            "hosted-CI/LFS independence",
        )

    with sandbox() as root:
        (root / ".lfsconfig").write_text("[lfs]\n", encoding="utf-8")
        expect_fail(
            "LFS configuration appears",
            root,
            "hosted-CI/LFS independence",
        )

    # -- the gate's own PASS lines -------------------------------------------
    with sandbox() as base:
        code, output = run_gate(base)
        expected = (
            "PASS: fallback confinement and API LLM boundary",
            "PASS: hosted-CI independence (no actions, no external CI configs, no LFS)",
            "PASS: executable configuration review pins (build surfaces are reviewed changes)",
            "PASS: no duplicate JSON keys in repository documents",
            "PASS: vendored byte fidelity (LF-only upstream-locked bytes)",
        )
        missing = [line for line in expected if line not in output]
        if code == 0 and not missing:
            print("PASS accept: the gate reports every invariant it enforces")
        else:
            FAILURES.append("the gate reports every invariant it enforces")
            print(f"FAIL accept: the gate's report is incomplete (exit {code}, missing {missing})")

    if FAILURES:
        print(f"\nFAIL: {len(FAILURES)} policy-gate regressions behaved wrongly:")
        for item in FAILURES:
            print(f"  - {item}")
        return 1
    print("\nPASS: all policy-gate regressions behave fail-closed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
