#!/usr/bin/env python3
"""Regression tests for the repository-owned verifier (`local_ci.py verify`).

The verifier must FAIL CLOSED: fabricated evidence files — wrong schema,
empty/missing/duplicate/unknown/malformed checks, wrong commit, dirty tree,
tampered digest — must all be rejected, never vacuously accepted.

Run: python scripts/test_local_ci_verify.py
Exit: 0 if all cases behave, 1 otherwise. Dependency-free (stdlib only).
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("local_ci", HERE / "local_ci.py")
local_ci = importlib.util.module_from_spec(_spec)
assert _spec.loader is not None
_spec.loader.exec_module(local_ci)

COMMIT = "0" * 40
SCHEMA = "prometheos.local-verification.v1"

CORE = [
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
]
PLATFORM = [
    "registry leases",
    "heartbeats",
    "recovery",
    "cross-process locking",
    "interruption recovery",
    "cancellation",
    "single-winner concurrency",
    "resource enforcement",
]
SMOKE = [
    "full-stack smoke",
    "approval-controlled patch smoke",
    "governed provider smoke",
    "provider governance",
    "golden path",
    "isolated install",
    "installed version",
    "installed help",
]

SUITE_NAMES = {"core": CORE, "platform": PLATFORM, "smoke": SMOKE}


def make_check(name: str) -> dict[str, object]:
    return {"name": name, "command": ["true"], "seconds": 1.0, "status": "passed"}


def make_record(
    suite: str,
    names: list[str] | None = None,
    *,
    digest: bool = True,
    **overrides: object,
) -> dict[str, object]:
    names = names if names is not None else SUITE_NAMES[suite]
    record: dict[str, object] = {
        "schemaVersion": SCHEMA,
        "commit": COMMIT,
        "suite": suite,
        "platform": "windows",
        "architecture": "amd64",
        "python": "3.10.11",
        "rustc": "rustc 1.98.0",
        "cargo": "cargo 1.98.0",
        "workingTreeClean": True,
        "completedAt": "2026-09-23T00:00:00+00:00",
        "checks": [make_check(name) for name in names],
    }
    record.update(overrides)
    if digest:
        record["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(record)
        ).hexdigest()
    return record


def write(tmp: Path, payload: object, name: str) -> str:
    target = tmp / name
    if isinstance(payload, (dict, list)):
        target.write_text(json.dumps(payload), encoding="utf-8")
    else:
        target.write_text(str(payload), encoding="utf-8")
    return str(target)


def expect_reject(label: str, paths: list[str], commit: str = COMMIT, platforms: list[str] | None = None) -> None:
    try:
        local_ci.verify_evidence(paths, commit, platforms or [])
    except RuntimeError as error:
        if label in str(error) or "rejected" in str(error):
            print(f"PASS reject: {label}")
            return
        print(f"FAIL: {label} rejected with unexpected message: {error}")
        sys.exit(1)
    print(f"FAIL: {label} was ACCEPTED (verifier fails open)")
    sys.exit(1)


def expect_accept(label: str, paths: list[str], commit: str = COMMIT, platforms: list[str] | None = None) -> None:
    try:
        local_ci.verify_evidence(paths, commit, platforms or [])
    except RuntimeError as error:
        print(f"FAIL: {label} was rejected: {error}")
        sys.exit(1)
    print(f"PASS accept: {label}")


def main() -> int:
    with tempfile.TemporaryDirectory() as work:
        tmp = Path(work)

        # ------------------------------------------------------------------
        # File-level and structural failures
        # ------------------------------------------------------------------
        expect_reject("file not found", [str(tmp / "missing.json")])

        bogus = write(tmp, "this is not json {", "bogus.json")
        expect_reject("malformed JSON", [bogus])

        top_list = write(tmp, [make_record("core")], "top_list.json")
        expect_reject("top level not an object", [top_list])

        # ------------------------------------------------------------------
        # Digest and provenance failures
        # ------------------------------------------------------------------
        no_digest = make_record("core", digest=False)
        no_digest_path = write(tmp, no_digest, "no_digest.json")
        expect_reject("missing evidenceDigest", [no_digest_path])

        tampered = make_record("core")
        tampered_path = write(tmp, tampered, "tampered.json")
        text = Path(tampered_path).read_text(encoding="utf-8").replace('"rustfmt"', '"rustfm"')
        Path(tampered_path).write_text(text, encoding="utf-8")
        expect_reject("digest mismatch", [tampered_path])

        wrong_commit = make_record("core", commit="f" * 40)
        wrong_commit_path = write(tmp, wrong_commit, "wrong_commit.json")
        expect_reject("wrong commit", [wrong_commit_path])

        dirty = make_record("core", workingTreeClean=False)
        dirty_path = write(tmp, dirty, "dirty.json")
        expect_reject("dirty tree", [dirty_path])

        bad_ts = make_record("core", completedAt="not-a-timestamp")
        bad_ts_path = write(tmp, bad_ts, "bad_ts.json")
        expect_reject("non-ISO completedAt", [bad_ts_path])

        # ------------------------------------------------------------------
        # Schema failures
        # ------------------------------------------------------------------
        wrong_schema = make_record("core", schemaVersion="prometheos.local-verification.v2")
        wrong_schema_path = write(tmp, wrong_schema, "wrong_schema.json")
        expect_reject("wrong schemaVersion", [wrong_schema_path])

        missing_key = make_record("core")
        missing_key.pop("rustc")
        missing_key["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(missing_key)
        ).hexdigest()
        missing_key_path = write(tmp, missing_key, "missing_key.json")
        expect_reject("missing required key", [missing_key_path])

        extra_key = make_record("core", hostname="owned-machine")
        extra_key_path = write(tmp, extra_key, "extra_key.json")
        expect_reject("unknown top-level key", [extra_key_path])

        unknown_suite = make_record("core")
        unknown_suite["suite"] = "bogus-suite"
        unknown_suite["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(unknown_suite)
        ).hexdigest()
        unknown_suite_path = write(tmp, unknown_suite, "unknown_suite.json")
        expect_reject("unknown suite", [unknown_suite_path])

        unknown_platform = make_record("core", platform="plan9")
        unknown_platform_path = write(tmp, unknown_platform, "unknown_platform.json")
        expect_reject("unknown platform", [unknown_platform_path])

        # ------------------------------------------------------------------
        # Check-set failures: empty, missing, unknown, duplicate, malformed
        # ------------------------------------------------------------------
        empty_checks = make_record("core", [])
        empty_checks_path = write(tmp, empty_checks, "empty_checks.json")
        expect_reject("empty checks list", [empty_checks_path])

        checks_not_list = make_record("core", checks="all good")
        checks_not_list_path = write(tmp, checks_not_list, "checks_not_list.json")
        expect_reject("checks not a list", [checks_not_list_path])

        missing_check = make_record("core", [name for name in CORE if name != "clippy"])
        missing_check_path = write(tmp, missing_check, "missing_check.json")
        expect_reject("missing required check", [missing_check_path])

        unknown_check = make_record("core", CORE + ["hosted actions"])
        unknown_check_path = write(tmp, unknown_check, "unknown_check.json")
        expect_reject("unknown check", [unknown_check_path])

        duplicate_check = make_record("core", CORE + ["rustfmt"])
        duplicate_check_path = write(tmp, duplicate_check, "duplicate_check.json")
        expect_reject("duplicate check", [duplicate_check_path])

        reordered = make_record("core", [CORE[-1]] + CORE[:-1])
        reordered_path = write(tmp, reordered, "reordered.json")
        expect_reject("check order mismatch", [reordered_path])

        failed_check = make_record("core")
        failed_check["checks"][0]["status"] = "failed"
        failed_check["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(failed_check)
        ).hexdigest()
        failed_path = write(tmp, failed_check, "failed_check.json")
        expect_reject("non-passed check", [failed_path])

        malformed_check = make_record("core")
        del malformed_check["checks"][0]["seconds"]
        malformed_check["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(malformed_check)
        ).hexdigest()
        malformed_path = write(tmp, malformed_check, "malformed_check.json")
        expect_reject("malformed check", [malformed_path])

        check_not_object = make_record("core")
        check_not_object["checks"][0] = "rustfmt passed"
        check_not_object["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(check_not_object)
        ).hexdigest()
        check_object_path = write(tmp, check_not_object, "check_not_object.json")
        expect_reject("check entry not an object", [check_object_path])

        extra_check_key = make_record("core")
        extra_check_key["checks"][0]["notes"] = "trust me"
        extra_check_key["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(extra_check_key)
        ).hexdigest()
        extra_check_key_path = write(tmp, extra_check_key, "extra_check_key.json")
        expect_reject("unknown check key", [extra_check_key_path])

        bad_command = make_record("core")
        bad_command["checks"][0]["command"] = "rustfmt --check"
        bad_command["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(bad_command)
        ).hexdigest()
        bad_command_path = write(tmp, bad_command, "bad_command.json")
        expect_reject("command not a list", [bad_command_path])

        # ------------------------------------------------------------------
        # Cross-file failures
        # ------------------------------------------------------------------
        core_a = write(tmp, make_record("core"), "core_a.json")
        core_b = write(tmp, make_record("core"), "core_b.json")
        platform_a = write(tmp, make_record("platform"), "platform_a.json")
        smoke_a = write(tmp, make_record("smoke"), "smoke_a.json")
        expect_reject(
            "duplicate platform+suite evidence",
            [core_a, core_b, platform_a, smoke_a],
        )

        expect_reject("missing required suite", [core_a, platform_a])

        expect_reject(
            "missing platform evidence",
            [core_a, platform_a, smoke_a],
            platforms=["darwin"],
        )

        # ------------------------------------------------------------------
        # The one valid configuration
        # ------------------------------------------------------------------
        expect_accept("valid three-suite evidence", [core_a, platform_a, smoke_a])
        expect_accept(
            "valid evidence with platform requirement",
            [core_a, platform_a, smoke_a],
            platforms=["windows"],
        )

    print("\nPASS: all verifier regressions behave fail-closed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
