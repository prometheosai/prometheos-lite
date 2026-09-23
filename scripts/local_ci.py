#!/usr/bin/env python3
"""Repository-owned verification runner with commit-bound JSON evidence.

The SUITE_SPEC below is the single source of truth shared by the runner and
the verifier. Every check is bound to an EXACT normalized command array:
command[0] is normalized to its program-identity token (cargo / python /
bash / npm / node / binary) and environment-dependent values (the install
--root path) are normalized to placeholders, so evidence can be verified
across operating systems — but any extra flag (e.g. --no-run, --skip) or
missing flag breaks exact equality and is rejected. Durations must be
finite, non-negative, non-boolean numbers. Provenance fields must be
non-empty and truthful.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
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

SCHEMA_VERSION = "prometheos.local-verification.v1"
REQUIRED_SUITES = ("core", "platform", "smoke")
KNOWN_PLATFORMS = ("linux", "darwin", "windows")
CHECK_KEYS = frozenset({"name", "command", "seconds", "status"})

# ---------------------------------------------------------------------------
# Shared suite/command specification
#
# Single source of truth for BOTH sides of the evidence contract:
#   - the runner emits checks whose commands must normalize to their
#     entry's exact array (enforced at generation time; evidence that
#     violates the spec is refused before it is ever written), and
#   - the verifier rejects any evidence whose commands do not normalize to
#     the exact array — extra flags (--no-run, --skip, ...) included.
#
# Normalized form: command[0] becomes its identity token; the value after
# "--root" becomes "<root>" (the install root is environment-dependent).
# Everything else must match verbatim.
# ---------------------------------------------------------------------------

SUITE_SPEC: dict[str, list[dict[str, object]]] = {
    "core": [
        {"name": "rustfmt", "command": ["cargo", "fmt", "--all", "--", "--check"]},
        {"name": "clippy", "command": ["cargo", "clippy", "--all-targets", "--all-features", "--", "-D", "warnings"]},
        {"name": "doc tests", "command": ["cargo", "test", "--doc", "--all-features"]},
        {"name": "release build", "command": ["cargo", "build", "--release", "--all-features"]},
        {"name": "all tests", "command": ["cargo", "test", "--all-targets", "--all-features", "--", "--test-threads", "4"]},
        {"name": "guardrails", "command": ["cargo", "test", "--test", "guardrail_tests", "--test", "guardrail_integration_tests"]},
        {"name": "runtime policy", "command": ["cargo", "test", "--test", "runtime_policy_enforcement", "--quiet"]},
        {"name": "verification policy", "command": ["cargo", "test", "--test", "ci_enforcement_tests", "--quiet"]},
        {"name": "patch diagnostics", "command": ["cargo", "test", "--test", "patch_provider_diagnostics_tests", "--quiet"]},
        {"name": "verifier regressions", "command": ["python", "scripts/test_local_ci_verify.py"]},
        {"name": "repository policy", "command": ["python", "scripts/local_ci_policy.py"]},
    ],
    "platform": [
        {"name": "registry leases", "command": ["cargo", "test", "--lib", "workflow::evaluate::registry", "--quiet"]},
        {"name": "heartbeats", "command": ["cargo", "test", "--lib", "workflow::evaluate::heartbeat", "--quiet"]},
        {"name": "recovery", "command": ["cargo", "test", "--lib", "workflow::evaluate::recovery", "--quiet"]},
        {"name": "cross-process locking", "command": ["cargo", "test", "--test", "locking_tests", "--quiet"]},
        {"name": "interruption recovery", "command": ["cargo", "test", "--test", "interruption_recovery_tests", "--quiet"]},
        {"name": "cancellation", "command": ["cargo", "test", "--test", "cancellation_tests", "--quiet"]},
        {"name": "single-winner concurrency", "command": ["cargo", "test", "--test", "concurrency_tests", "--quiet"]},
        {"name": "resource enforcement", "command": ["cargo", "test", "--lib", "workflow::evaluate::validation", "--quiet", "--", "--nocapture"]},
    ],
    "smoke": [
        {"name": "full-stack smoke", "command": ["bash", "scripts/fullstack-smoke.sh"]},
        {"name": "approval-controlled patch smoke", "command": ["bash", "scripts/approval-controlled-patch-smoke.sh"]},
        {"name": "governed provider smoke", "command": ["bash", "scripts/provider-governed-proposal-smoke.sh"]},
        {"name": "provider governance", "command": ["cargo", "test", "--test", "provider_governed_proposal_tests", "--quiet"]},
        {"name": "golden path", "command": ["bash", "scripts/demo/repo-workbench-first-value.sh"]},
        {"name": "isolated install", "command": ["cargo", "install", "--path", ".", "--force", "--root", "<root>"]},
        {"name": "installed version", "command": ["binary", "--version"]},
        {"name": "installed help", "command": ["binary", "--help"]},
    ],
    "frontend": [
        {"name": "frontend install", "command": ["npm", "ci"]},
        {"name": "frontend build", "command": ["npm", "run", "build"]},
        {"name": "frontend lint", "command": ["npm", "run", "lint"]},
        {"name": "frontend smoke", "command": ["node", "scripts/smoke.mjs"]},
    ],
}

SPEC_BY_NAME: dict[str, dict[str, object]] = {
    entry["name"]: entry for entries in SUITE_SPEC.values() for entry in entries
}

# Concrete program used to denormalize an identity token in synthetic
# commands (used by the regression suite; the runner uses real paths).
TOKEN_PROGRAMS = {
    "cargo": "cargo",
    "python": "python",
    "bash": "bash",
    "npm": "npm",
    "node": "node",
    "binary": ".local-ci/install/bin/prometheos",
}


def _split_path(program: str) -> list[str]:
    """Split a possibly Windows-style program path, host-independently."""
    return [part for part in program.replace("\\", "/").split("/") if part]


def _program_token(program: str) -> str | None:
    """Environment-independent identity token for a command[0], or None."""
    parts = _split_path(program)
    base = parts[-1].lower() if parts else ""
    if base in {"cargo", "cargo.exe"}:
        return "cargo"
    if base in {"python", "python3", "python.exe", "python3.exe", "py", "py.exe"} or base.startswith("python"):
        return "python"
    if base in {"bash", "bash.exe"}:
        return "bash"
    if base in {"npm", "npm.cmd", "npm.exe"}:
        return "npm"
    if base in {"node", "node.exe"}:
        return "node"
    if base in {"prometheos", "prometheos.exe"}:
        lowered = {part.lower() for part in parts}
        if ".local-ci" in lowered and "install" in lowered:
            return "binary"
    return None


def normalize_command(command: list[str]) -> list[str] | None:
    """Normalize a command to its spec-comparable form, or None when the
    program identity is unrecognized.

    command[0] becomes its identity token; the value following "--root"
    becomes "<root>" (the isolated-install target is environment-dependent).
    Every other element must match the specification verbatim, so extra
    flags (e.g. --no-run, --skip) break equality.
    """
    if not command:
        return None
    token = _program_token(command[0])
    if token is None:
        return None
    args = list(command[1:])
    if "--root" in args:
        index = args.index("--root")
        if index + 1 >= len(args):
            return None
        args[index + 1] = "<root>"
    return [token] + args


def command_matches_spec(command: list[str], entry: dict[str, object]) -> bool:
    """True only when the command normalizes to the entry's EXACT array."""
    normalized = normalize_command(command)
    expected = [str(item) for item in entry["command"]]
    return normalized == expected


def valid_duration(value: object) -> bool:
    """Durations must be finite, non-negative, non-boolean numbers."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    return math.isfinite(value) and value >= 0


def duration_problem(value: object) -> str | None:
    """Human-readable reason a duration is invalid, or None."""
    if isinstance(value, bool):
        return "seconds must not be a boolean"
    if not isinstance(value, (int, float)):
        return "seconds is not a number"
    if not math.isfinite(value):
        return "seconds must be finite"
    if value < 0:
        return "seconds must be non-negative"
    return None


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
        # Bounded test-thread count: every test still runs; the cap only
        # limits how many git-fixture tests perform .git/objects writes at
        # the same instant. Under full default parallelism (~host cores),
        # real-time AV scanning of fresh object files produces transient
        # "unable to write file .git/objects/...: Permission denied" races
        # that are environmental, not code defects (each affected test
        # passes standalone). The repo protocol already uses serialized
        # baselines; 4 preserves genuine concurrency (locking, heartbeat,
        # cancellation races) while staying under the contention threshold.
        ("all tests", ["cargo", "test", "--all-targets", "--all-features", "--", "--test-threads", "4"]),
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


def validate_checks_against_spec(suite: str, checks: list[dict[str, object]]) -> None:
    """Full generation-time self-check: the runner and the verifier share
    SUITE_SPEC, and evidence that does not satisfy it is refused BEFORE any
    bytes are written. Validates names in order, exact normalized commands,
    check shape, duration sanity, and pass status."""
    spec = SUITE_SPEC.get(suite)
    if spec is None:
        raise RuntimeError(f"unknown suite {suite!r}; refusing to write evidence")
    if len(checks) != len(spec):
        raise RuntimeError(
            f"generated {len(checks)} checks but the {suite} specification requires {len(spec)}"
        )
    for check, entry in zip(checks, spec):
        if frozenset(check) != CHECK_KEYS:
            raise RuntimeError(
                f"check keys must be exactly {sorted(CHECK_KEYS)}; refusing to write evidence"
            )
        if check["name"] != entry["name"]:
            raise RuntimeError(
                f"generated check {check['name']!r} does not match specification entry {entry['name']!r}"
            )
        if not command_matches_spec(list(check["command"]), entry):
            raise RuntimeError(
                f"generated command for {check['name']!r} does not match the exact normalized "
                f"command {entry['command']!r}"
            )
        problem = duration_problem(check["seconds"])
        if problem:
            raise RuntimeError(f"check {check['name']!r} {problem}; refusing to write evidence")
        if check["status"] != "passed":
            raise RuntimeError(
                f"check {check['name']!r} did not pass; refusing to write evidence"
            )


def write_evidence(suite: str, checks: list[dict[str, object]], dirty: bool) -> Path:
    # Generation-time self-check: nothing is written until every check
    # satisfies the shared specification exactly.
    validate_checks_against_spec(suite, checks)

    commit = capture("git", "rev-parse", "HEAD")
    record: dict[str, object] = {
        "schemaVersion": SCHEMA_VERSION,
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


def verify_evidence(
    paths: list[str],
    commit: str,
    required_platforms: list[str],
    require_frontend: bool = False,
) -> None:
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

        # Provenance: bound to the exact commit, clean tree, ISO timestamp,
        # and a REAL toolchain — empty or foreign provenance is fabricated.
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
        for key in ("python", "architecture"):
            value = record[key]
            if not isinstance(value, str) or not value.strip():
                reject(path, f"{key} provenance is empty")
        for key, prefix in (("rustc", "rustc"), ("cargo", "cargo")):
            value = record[key]
            if not isinstance(value, str) or not value.strip():
                reject(path, f"{key} provenance is empty")
            if not value.lstrip().lower().startswith(prefix):
                reject(path, f"{key} provenance {value!r} does not describe a {prefix} toolchain")

        suite = record["suite"]
        platform_name = record["platform"]
        if suite not in SUITE_SPEC:
            reject(path, f"unknown suite {suite!r}")
        if platform_name not in KNOWN_PLATFORMS:
            reject(path, f"unknown platform {platform_name!r}")

        # Checks: non-empty, well-formed, all passed, and every command must
        # normalize to the EXACT specification array — execution-suppressing
        # or behavior-changing extra flags break equality and are rejected.
        checks = record["checks"]
        if not isinstance(checks, list):
            reject(path, "checks is not a list")
        if not checks:
            reject(path, "empty checks list")
        spec = SUITE_SPEC[suite]
        if len(checks) != len(spec):
            reject(path, f"expected {len(spec)} checks for suite {suite!r}, found {len(checks)}")
        names: list[str] = []
        for check, entry in zip(checks, spec):
            if not isinstance(check, dict):
                reject(path, "check entry is not an object")
            if frozenset(check) != CHECK_KEYS:
                reject(path, f"check keys must be exactly {sorted(CHECK_KEYS)}")
            if not isinstance(check["name"], str) or not check["name"]:
                reject(path, "check name is not a non-empty string")
            if check["name"] != entry["name"]:
                reject(
                    path,
                    f"check order mismatch: expected {entry['name']!r}, found {check['name']!r}",
                )
            command = check["command"]
            if (
                not isinstance(command, list)
                or not command
                or not all(isinstance(part, str) for part in command)
            ):
                reject(path, f"check {check['name']!r} command is not a non-empty string list")
            if not command_matches_spec(command, entry):
                reject(
                    path,
                    f"check {check['name']!r} command does not normalize to the exact "
                    f"specification command {entry['command']!r}",
                )
            problem = duration_problem(check["seconds"])
            if problem:
                reject(path, f"check {check['name']!r} {problem}")
            if check["status"] != "passed":
                reject(path, f"check {check['name']!r} did not pass (status {check['status']!r})")
            names.append(check["name"])
        if len(names) != len(set(names)):
            duplicates = sorted({name for name in names if names.count(name) > 1})
            reject(path, f"duplicate checks {duplicates}")

        key = (platform_name, suite)
        if key in seen:
            reject(path, f"duplicate evidence for platform {platform_name!r} suite {suite!r}")
        seen.add(key)

    seen_suites = {suite for _, suite in seen}
    missing_suites = sorted(set(REQUIRED_SUITES) - seen_suites)
    if missing_suites:
        raise RuntimeError(f"missing required suites: {missing_suites}")
    if require_frontend and "frontend" not in seen_suites:
        raise RuntimeError("missing frontend evidence (--require-frontend)")
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
    verify_parser.add_argument(
        "--require-frontend",
        action="store_true",
        help="require frontend-suite evidence (use when frontend paths changed)",
    )
    verify_parser.add_argument("evidence", nargs="+")
    args = parser.parse_args()

    if args.command == "verify":
        verify_evidence(args.evidence, args.commit, args.require_platform, args.require_frontend)
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
