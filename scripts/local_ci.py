#!/usr/bin/env python3
"""Repository-owned verification runner with commit-bound JSON evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE_ROOT = ROOT / ".local-ci" / "evidence"


def capture(*command: str) -> str:
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def run(
    name: str,
    command: list[str],
    *,
    cwd: Path = ROOT,
    env: dict[str, str] | None = None,
) -> dict[str, object]:
    print(f"\n==> {name}\n    {' '.join(command)}", flush=True)
    started = time.monotonic()
    result = subprocess.run(command, cwd=cwd, env=env, check=False)
    duration = round(time.monotonic() - started, 3)
    if result.returncode:
        raise RuntimeError(f"{name} failed with exit code {result.returncode}")
    return {"name": name, "command": command, "seconds": duration, "status": "passed"}


def rust_core() -> list[dict[str, object]]:
    commands = [
        ("rustfmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--all-targets", "--all-features", "--", "-D", "warnings"]),
        ("doc tests", ["cargo", "test", "--doc", "--all-features"]),
        ("release build", ["cargo", "build", "--release", "--all-features"]),
        ("all tests", ["cargo", "test", "--all-targets", "--all-features"]),
        ("guardrails", ["cargo", "test", "--test", "guardrail_tests", "--test", "guardrail_integration_tests"]),
        ("runtime policy", ["cargo", "test", "--test", "runtime_policy_enforcement", "--quiet"]),
        ("verification policy", ["cargo", "test", "--test", "ci_enforcement_tests", "--quiet"]),
        ("patch diagnostics", ["cargo", "test", "--test", "patch_provider_diagnostics_tests", "--quiet"]),
        ("verifier regressions", [sys.executable, "scripts/test_local_ci_verify.py"]),
        ("repository policy", [sys.executable, "scripts/local_ci_policy.py"]),
    ]
    return [run(name, command) for name, command in commands]


def platform_suite() -> list[dict[str, object]]:
    commands = [
        ("registry leases", ["cargo", "test", "--lib", "workflow::evaluate::registry", "--quiet"]),
        ("heartbeats", ["cargo", "test", "--lib", "workflow::evaluate::heartbeat", "--quiet"]),
        ("recovery", ["cargo", "test", "--lib", "workflow::evaluate::recovery", "--quiet"]),
        ("cross-process locking", ["cargo", "test", "--test", "locking_tests", "--quiet"]),
        ("interruption recovery", ["cargo", "test", "--test", "interruption_recovery_tests", "--quiet"]),
        ("cancellation", ["cargo", "test", "--test", "cancellation_tests", "--quiet"]),
        ("single-winner concurrency", ["cargo", "test", "--test", "concurrency_tests", "--quiet"]),
        ("resource enforcement", ["cargo", "test", "--lib", "workflow::evaluate::validation", "--quiet", "--", "--nocapture"]),
    ]
    return [run(name, command) for name, command in commands]


def find_bash() -> str:
    """Locate a functional Bash. WSL bash.exe can be present but unusable
    (broken WSL installs fail with HCS mount errors); Git Bash is preferred on
    Windows. We probe each candidate with a trivial invocation."""
    on_windows = platform.system() == "Windows"
    candidates: list[str] = []
    if on_windows:
        # Prefer Git Bash over WSL `bash` because a broken WSL install causes
        # HCS/CreateInstance failures rather than working shell emulation.
        for p in [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files (x86)\Git\bin\bash.exe",
            rf"{os.environ.get('LOCALAPPDATA', '')}\Programs\Git\bin\bash.exe",
        ]:
            if Path(p).is_file():
                candidates.append(p)
    if shutil.which("bash"):
        candidates.append(shutil.which("bash") or "bash")

    for candidate in candidates:
        try:
            out = subprocess.run(
                [candidate, "-c", "true"],
                capture_output=True,
                timeout=15,
                check=False,
            )
            if out.returncode == 0:
                return candidate
        except (OSError, subprocess.SubprocessError):
            continue
    raise RuntimeError("smoke suite needs workable Bash parsing (Git Bash or WSL)")


def smoke_suite() -> list[dict[str, object]]:
    bash = find_bash()
    commands = [
        ("full-stack smoke", [bash, "scripts/fullstack-smoke.sh"]),
        ("approval-controlled patch smoke", [bash, "scripts/approval-controlled-patch-smoke.sh"]),
        ("governed provider smoke", [bash, "scripts/provider-governed-proposal-smoke.sh"]),
        ("provider governance", ["cargo", "test", "--test", "provider_governed_proposal_tests", "--quiet"]),
        ("golden path", [bash, "scripts/demo/repo-workbench-first-value.sh"]),
    ]
    checks = [run(name, command) for name, command in commands]
    install_root = ROOT / ".local-ci" / "install"
    install_env = dict(os.environ)
    install_env["CARGO_INSTALL_ROOT"] = str(install_root)
    checks.append(
        run(
            "isolated install",
            ["cargo", "install", "--path", ".", "--force", "--root", str(install_root)],
            env=install_env,
        )
    )
    binary_name = "prometheos.exe" if platform.system() == "Windows" else "prometheos"
    binary = install_root / "bin" / binary_name
    checks.append(run("installed version", [str(binary), "--version"]))
    checks.append(run("installed help", [str(binary), "--help"]))
    return checks


def frontend_suite() -> list[dict[str, object]]:
    frontend = ROOT / "frontend"
    commands = [
        ("frontend install", ["npm", "ci"]),
        ("frontend build", ["npm", "run", "build"]),
        ("frontend lint", ["npm", "run", "lint"]),
        ("frontend smoke", ["node", "scripts/smoke.mjs"]),
    ]
    return [run(name, command, cwd=frontend) for name, command in commands]


def canonical_bytes(value: dict[str, object]) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_evidence(suite: str, checks: list[dict[str, object]], dirty: bool) -> Path:
    commit = capture("git", "rev-parse", "HEAD")
    record: dict[str, object] = {
        "schemaVersion": "prometheos.local-verification.v1",
        "commit": commit,
        "suite": suite,
        "platform": platform.system().lower(),
        "architecture": platform.machine().lower(),
        "python": platform.python_version(),
        "rustc": capture("rustc", "--version"),
        "cargo": capture("cargo", "--version"),
        "workingTreeClean": not dirty,
        "completedAt": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
    }
    record["evidenceDigest"] = hashlib.sha256(canonical_bytes(record)).hexdigest()
    target = EVIDENCE_ROOT / commit / f"{record['platform']}-{suite}.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(canonical_bytes(record))
    print(f"\nPASS: evidence written to {target.relative_to(ROOT)}")
    print(f"SHA-256: {record['evidenceDigest']}")
    return target


def verify_evidence(paths: list[str], commit: str, required_platforms: list[str]) -> None:
    SCHEMA_VERSION = "prometheos.local-verification.v1"
    REQUIRED_SUITES = ("core", "platform", "smoke")
    KNOWN_PLATFORMS = ("linux", "darwin", "windows")
    # Every suite must present EXACTLY this check sequence: complete, no
    # missing, no unknown, no duplicates. Fabricated or truncated evidence
    # is rejected rather than accepted vacuously.
    EXPECTED_CHECKS = {
        "core": [
            "rustfmt",
            "clippy",
            "doc tests",
            "release build",
            "all tests",
            "guardrails",
            "runtime policy",
            "verification policy",
            "patch diagnostics",
            "verifier regressions",
            "repository policy",
        ],
        "platform": [
            "registry leases",
            "heartbeats",
            "recovery",
            "cross-process locking",
            "interruption recovery",
            "cancellation",
            "single-winner concurrency",
            "resource enforcement",
        ],
        "smoke": [
            "full-stack smoke",
            "approval-controlled patch smoke",
            "governed provider smoke",
            "provider governance",
            "golden path",
            "isolated install",
            "installed version",
            "installed help",
        ],
    }
    REQUIRED_KEYS = (
        "schemaVersion",
        "commit",
        "suite",
        "platform",
        "architecture",
        "python",
        "rustc",
        "cargo",
        "workingTreeClean",
        "completedAt",
        "checks",
    )
    CHECK_KEYS = frozenset({"name", "command", "seconds", "status"})

    def reject(path: Path, reason: str) -> None:
        raise RuntimeError(f"evidence rejected ({reason}): {path}")

    seen: set[tuple[str, str]] = set()
    for raw in paths:
        path = Path(raw)
        if not path.is_file():
            reject(path, "file not found")
        try:
            record = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            reject(path, f"unreadable or malformed JSON: {error}")
        if not isinstance(record, dict):
            reject(path, "top level is not a JSON object")

        # Integrity: digest over the canonical record minus the digest field.
        claimed = record.pop("evidenceDigest", None)
        if not isinstance(claimed, str) or len(claimed) != 64:
            reject(path, "missing or malformed evidenceDigest")
        actual = hashlib.sha256(canonical_bytes(record)).hexdigest()
        if claimed != actual:
            reject(path, "evidence digest mismatch (record tampered or hand-written)")

        # Schema: exact version, exact top-level key set, every key present.
        if record.get("schemaVersion") != SCHEMA_VERSION:
            reject(path, f"unsupported schemaVersion {record.get('schemaVersion')!r}")
        missing_keys = [key for key in REQUIRED_KEYS if key not in record]
        if missing_keys:
            reject(path, f"missing required keys {missing_keys}")
        extra_keys = sorted(set(record) - set(REQUIRED_KEYS))
        if extra_keys:
            reject(path, f"unknown top-level keys {extra_keys}")

        # Provenance: bound to the exact commit, clean tree, ISO timestamp.
        if record["commit"] != commit:
            reject(path, f"commit {record['commit']!r} does not match --commit {commit!r}")
        if record["workingTreeClean"] is not True:
            reject(path, "workingTreeClean is not true (dirty-tree evidence is non-mergeable)")
        if not isinstance(record["completedAt"], str):
            reject(path, "completedAt is not a string")
        try:
            datetime.fromisoformat(record["completedAt"])
        except ValueError:
            reject(path, "completedAt is not an ISO-8601 timestamp")

        suite = record["suite"]
        platform_name = record["platform"]
        if suite not in EXPECTED_CHECKS:
            reject(path, f"unknown suite {suite!r}")
        if platform_name not in KNOWN_PLATFORMS:
            reject(path, f"unknown platform {platform_name!r}")

        # Checks: non-empty, well-formed, all passed, exact expected set.
        checks = record["checks"]
        if not isinstance(checks, list):
            reject(path, "checks is not a list")
        if not checks:
            reject(path, "empty checks list")
        names: list[str] = []
        for check in checks:
            if not isinstance(check, dict):
                reject(path, "check entry is not an object")
            if frozenset(check) != CHECK_KEYS:
                reject(path, f"check keys must be exactly {sorted(CHECK_KEYS)}")
            if not isinstance(check["name"], str) or not check["name"]:
                reject(path, "check name is not a non-empty string")
            command = check["command"]
            if (
                not isinstance(command, list)
                or not command
                or not all(isinstance(part, str) for part in command)
            ):
                reject(path, f"check {check['name']!r} command is not a non-empty string list")
            if not isinstance(check["seconds"], (int, float)):
                reject(path, f"check {check['name']!r} seconds is not a number")
            if check["status"] != "passed":
                reject(path, f"check {check['name']!r} did not pass (status {check['status']!r})")
            names.append(check["name"])
        expected = EXPECTED_CHECKS[suite]
        if len(names) != len(set(names)):
            duplicates = sorted({name for name in names if names.count(name) > 1})
            reject(path, f"duplicate checks {duplicates}")
        missing = [name for name in expected if name not in names]
        if missing:
            reject(path, f"missing required checks {missing}")
        unknown = [name for name in names if name not in expected]
        if unknown:
            reject(path, f"unknown checks {unknown}")
        if names != expected:
            reject(path, f"check order mismatch: expected {expected}, found {names}")

        key = (platform_name, suite)
        if key in seen:
            reject(path, f"duplicate evidence for platform {platform_name!r} suite {suite!r}")
        seen.add(key)

    seen_suites = {suite for _, suite in seen}
    missing_suites = sorted(set(REQUIRED_SUITES) - seen_suites)
    if missing_suites:
        raise RuntimeError(f"missing required suites: {missing_suites}")
    missing_platforms = sorted(
        os_name for os_name in required_platforms if (os_name, "platform") not in seen
    )
    if missing_platforms:
        raise RuntimeError(f"missing platform evidence: {missing_platforms}")
    print(f"PASS: {len(paths)} evidence files verify for {commit}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    run_parser = sub.add_parser("run", help="run a local verification suite")
    run_parser.add_argument("--suite", choices=("core", "platform", "smoke", "frontend"), default="core")
    run_parser.add_argument("--allow-dirty", action="store_true")
    verify_parser = sub.add_parser("verify", help="verify collected repository evidence")
    verify_parser.add_argument("--commit", required=True)
    verify_parser.add_argument(
        "--require-platform",
        action="append",
        choices=("linux", "darwin", "windows"),
        default=[],
        help="require platform-suite evidence for this OS; repeat for a platform matrix",
    )
    verify_parser.add_argument("evidence", nargs="+")
    args = parser.parse_args()

    if args.command == "verify":
        verify_evidence(args.evidence, args.commit, args.require_platform)
        return 0
    if not shutil.which("cargo"):
        raise RuntimeError("cargo is required")
    initial_status = capture("git", "status", "--porcelain")
    dirty = bool(initial_status)
    if dirty and not args.allow_dirty:
        raise RuntimeError("working tree is dirty; commit first or use --allow-dirty for non-mergeable evidence")
    checks: list[dict[str, object]] = []
    if args.suite == "core":
        checks.extend(rust_core())
    if args.suite == "platform":
        checks.extend(platform_suite())
    if args.suite == "smoke":
        checks.extend(smoke_suite())
    if args.suite == "frontend":
        checks.extend(frontend_suite())
    if capture("git", "status", "--porcelain") != initial_status:
        raise RuntimeError("verification changed tracked or unignored files; evidence refused")
    write_evidence(args.suite, checks, dirty)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
