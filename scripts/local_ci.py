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
    seen: set[tuple[str, str]] = set()
    for raw in paths:
        path = Path(raw)
        record = json.loads(path.read_text(encoding="utf-8"))
        claimed = record.pop("evidenceDigest", None)
        actual = hashlib.sha256(canonical_bytes(record)).hexdigest()
        if claimed != actual:
            raise RuntimeError(f"evidence digest mismatch: {path}")
        if record.get("commit") != commit or not record.get("workingTreeClean"):
            raise RuntimeError(f"evidence is not clean and bound to {commit}: {path}")
        if any(check.get("status") != "passed" for check in record.get("checks", [])):
            raise RuntimeError(f"evidence contains a failed check: {path}")
        seen.add((str(record.get("platform")), str(record.get("suite"))))
    seen_suites = {suite for _, suite in seen}
    missing_suites = sorted({"core", "platform", "smoke"} - seen_suites)
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
