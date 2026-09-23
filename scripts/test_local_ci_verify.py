#!/usr/bin/env python3
"""Regression tests for the repository-owned verifier (`local_ci.py verify`).

The verifier must FAIL CLOSED: fabricated evidence files — wrong schema,
empty/missing/duplicate/unknown/malformed checks, commands that do not
normalize to the EXACT specification array (extra flags like --no-run
included), invalid durations (negative, boolean, NaN, infinity), empty or
foreign toolchain provenance, wrong commit, dirty tree, tampered digest —
must all be rejected, never vacuously accepted. The generation-time
self-check must refuse to write evidence with the same defects. The
frontend suite must be verifiable (with --require-frontend).

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

CORE = [entry["name"] for entry in local_ci.SUITE_SPEC["core"]]
PLATFORM = [entry["name"] for entry in local_ci.SUITE_SPEC["platform"]]
SMOKE = [entry["name"] for entry in local_ci.SUITE_SPEC["smoke"]]
FRONTEND = [entry["name"] for entry in local_ci.SUITE_SPEC["frontend"]]


def spec_command(name: str) -> list[str]:
    """Denormalize a spec entry into a concrete spec-compliant command:
    the identity token becomes a concrete program; placeholders stay
    (normalize_command maps them back). Unknown names get a placeholder
    command; the verifier rejects them on count/order first."""
    entry = local_ci.SPEC_BY_NAME.get(name)
    if entry is None:
        return ["true"]
    normalized = [str(item) for item in entry["command"]]
    token, args = normalized[0], normalized[1:]
    program = local_ci.TOKEN_PROGRAMS[token]
    return [program] + args


def spec_index(suite: str, name: str) -> int:
    return [entry["name"] for entry in local_ci.SUITE_SPEC[suite]].index(name)


def make_check(name: str, command: list[str] | None = None) -> dict[str, object]:
    return {
        "name": name,
        "command": command if command is not None else spec_command(name),
        "seconds": 1.0,
        "status": "passed",
    }


def write(tmp: Path, payload: object, name: str) -> str:
    target = tmp / name
    if isinstance(payload, (dict, list)):
        target.write_text(json.dumps(payload), encoding="utf-8")
    else:
        target.write_text(str(payload), encoding="utf-8")
    return str(target)


def expect_reject(
    label: str,
    paths: list[str],
    commit: str = COMMIT,
    platforms: list[str] | None = None,
    frontend: bool = False,
) -> None:
    try:
        local_ci.verify_evidence(paths, commit, platforms or [], frontend)
    except RuntimeError as error:
        if label in str(error) or "rejected" in str(error):
            print(f"PASS reject: {label}")
            return
        print(f"FAIL: {label} rejected with unexpected message: {error}")
        sys.exit(1)
    print(f"FAIL: {label} was ACCEPTED (verifier fails open)")
    sys.exit(1)


def expect_accept(
    label: str,
    paths: list[str],
    commit: str = COMMIT,
    platforms: list[str] | None = None,
    frontend: bool = False,
) -> None:
    try:
        local_ci.verify_evidence(paths, commit, platforms or [], frontend)
    except RuntimeError as error:
        print(f"FAIL: {label} was rejected: {error}")
        sys.exit(1)
    print(f"PASS accept: {label}")


def expect_generation_refusal(label: str, suite: str, checks: list[dict[str, object]]) -> None:
    """Generation-time regressions: the runner must refuse to WRITE evidence
    whose checks violate the shared specification. validate_checks_against_spec
    raises before any bytes are written."""
    try:
        local_ci.validate_checks_against_spec(suite, checks)
    except RuntimeError as error:
        print(f"PASS generation refusal: {label} ({error})")
        return
    print(f"FAIL: {label} was ACCEPTED by the generation self-check (fails open)")
    sys.exit(1)


def core_checks_with(index: int, **mutations: object) -> list[dict[str, object]]:
    checks = [make_check(name) for name in CORE]
    checks[index].update(mutations)
    return checks


def main() -> int:
    with tempfile.TemporaryDirectory() as work:
        tmp = Path(work)

        # ------------------------------------------------------------------
        # File-level and structural failures
        # ------------------------------------------------------------------
        expect_reject("file not found", [str(tmp / "missing.json")])

        bogus = write(tmp, "this is not json {", "bogus.json")
        expect_reject("malformed JSON", [bogus])

        top_list = write(tmp, [make_record_core()], "top_list.json")
        expect_reject("top level not an object", [top_list])

        # ------------------------------------------------------------------
        # Digest and provenance failures
        # ------------------------------------------------------------------
        no_digest = make_record_core(digest=False)
        no_digest_path = write(tmp, no_digest, "no_digest.json")
        expect_reject("missing evidenceDigest", [no_digest_path])

        tampered = make_record_core()
        tampered_path = write(tmp, tampered, "tampered.json")
        text = Path(tampered_path).read_text(encoding="utf-8").replace('"rustfmt"', '"rustfm"')
        Path(tampered_path).write_text(text, encoding="utf-8")
        expect_reject("digest mismatch", [tampered_path])

        wrong_commit = make_record_core(commit="f" * 40)
        wrong_commit_path = write(tmp, wrong_commit, "wrong_commit.json")
        expect_reject("wrong commit", [wrong_commit_path])

        dirty = make_record_core(workingTreeClean=False)
        dirty_path = write(tmp, dirty, "dirty.json")
        expect_reject("dirty tree", [dirty_path])

        bad_ts = make_record_core(completedAt="not-a-timestamp")
        bad_ts_path = write(tmp, bad_ts, "bad_ts.json")
        expect_reject("non-ISO completedAt", [bad_ts_path])

        empty_rustc = make_record_core(rustc="")
        empty_rustc_path = write(tmp, empty_rustc, "empty_rustc.json")
        expect_reject("empty rustc provenance", [empty_rustc_path])

        foreign_rustc = make_record_core(rustc="gcc 13.2.0")
        foreign_rustc_path = write(tmp, foreign_rustc, "foreign_rustc.json")
        expect_reject("foreign rustc provenance", [foreign_rustc_path])

        empty_cargo = make_record_core(cargo="")
        empty_cargo_path = write(tmp, empty_cargo, "empty_cargo.json")
        expect_reject("empty cargo provenance", [empty_cargo_path])

        foreign_cargo = make_record_core(cargo="make 4.4")
        foreign_cargo_path = write(tmp, foreign_cargo, "foreign_cargo.json")
        expect_reject("foreign cargo provenance", [foreign_cargo_path])

        empty_python = make_record_core(python="")
        empty_python_path = write(tmp, empty_python, "empty_python.json")
        expect_reject("empty python provenance", [empty_python_path])

        empty_arch = make_record_core(architecture="")
        empty_arch_path = write(tmp, empty_arch, "empty_arch.json")
        expect_reject("empty architecture provenance", [empty_arch_path])

        # ------------------------------------------------------------------
        # Schema failures
        # ------------------------------------------------------------------
        wrong_schema = make_record_core(schemaVersion="prometheos.local-verification.v2")
        wrong_schema_path = write(tmp, wrong_schema, "wrong_schema.json")
        expect_reject("wrong schemaVersion", [wrong_schema_path])

        missing_key = make_record_core()
        missing_key.pop("rustc")
        missing_key["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(missing_key)
        ).hexdigest()
        missing_key_path = write(tmp, missing_key, "missing_key.json")
        expect_reject("missing required key", [missing_key_path])

        extra_key = make_record_core(hostname="owned-machine")
        extra_key_path = write(tmp, extra_key, "extra_key.json")
        expect_reject("unknown top-level key", [extra_key_path])

        unknown_suite = make_record_core()
        unknown_suite["suite"] = "bogus-suite"
        unknown_suite["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(unknown_suite)
        ).hexdigest()
        unknown_suite_path = write(tmp, unknown_suite, "unknown_suite.json")
        expect_reject("unknown suite", [unknown_suite_path])

        unknown_platform = make_record_core(platform="plan9")
        unknown_platform_path = write(tmp, unknown_platform, "unknown_platform.json")
        expect_reject("unknown platform", [unknown_platform_path])

        # ------------------------------------------------------------------
        # Check-set failures: empty, missing, unknown, duplicate, malformed
        # ------------------------------------------------------------------
        empty_checks = make_record_core([])
        empty_checks_path = write(tmp, empty_checks, "empty_checks.json")
        expect_reject("empty checks list", [empty_checks_path])

        checks_not_list = make_record_core(checks="all good")
        checks_not_list_path = write(tmp, checks_not_list, "checks_not_list.json")
        expect_reject("checks not a list", [checks_not_list_path])

        missing_check = make_record_core([name for name in CORE if name != "clippy"])
        missing_check_path = write(tmp, missing_check, "missing_check.json")
        expect_reject("missing required check", [missing_check_path])

        unknown_check = make_record_core(CORE + ["hosted actions"])
        unknown_check_path = write(tmp, unknown_check, "unknown_check.json")
        expect_reject("unknown check", [unknown_check_path])

        duplicate_check = make_record_core(CORE + ["rustfmt"])
        duplicate_check_path = write(tmp, duplicate_check, "duplicate_check.json")
        expect_reject("duplicate check", [duplicate_check_path])

        reordered = make_record_core([CORE[-1]] + CORE[:-1])
        reordered_path = write(tmp, reordered, "reordered.json")
        expect_reject("check order mismatch", [reordered_path])

        failed_check = make_record_core()
        failed_check["checks"][0]["status"] = "failed"
        failed_check["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(failed_check)
        ).hexdigest()
        failed_path = write(tmp, failed_check, "failed_check.json")
        expect_reject("non-passed check", [failed_path])

        malformed_check = make_record_core()
        del malformed_check["checks"][0]["seconds"]
        malformed_check["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(malformed_check)
        ).hexdigest()
        malformed_path = write(tmp, malformed_check, "malformed_check.json")
        expect_reject("malformed check", [malformed_path])

        check_not_object = make_record_core()
        check_not_object["checks"][0] = "rustfmt passed"
        check_not_object["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(check_not_object)
        ).hexdigest()
        check_object_path = write(tmp, check_not_object, "check_not_object.json")
        expect_reject("check entry not an object", [check_object_path])

        extra_check_key = make_record_core()
        extra_check_key["checks"][0]["notes"] = "trust me"
        extra_check_key["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(extra_check_key)
        ).hexdigest()
        extra_check_key_path = write(tmp, extra_check_key, "extra_check_key.json")
        expect_reject("unknown check key", [extra_check_key_path])

        bad_command = make_record_core()
        bad_command["checks"][0]["command"] = "rustfmt --check"
        bad_command["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(bad_command)
        ).hexdigest()
        bad_command_path = write(tmp, bad_command, "bad_command.json")
        expect_reject("command not a list", [bad_command_path])

        # ------------------------------------------------------------------
        # Command-specification failures: exact normalized command binding
        # ------------------------------------------------------------------
        all_tests = spec_index("core", "all tests")

        all_true = make_record_core()
        for check in all_true["checks"]:
            check["command"] = ["true"]
        all_true["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(all_true)
        ).hexdigest()
        all_true_path = write(tmp, all_true, "all_true.json")
        expect_reject("all-true commands", [all_true_path])

        no_run = make_record_core()
        no_run["checks"][all_tests]["command"] = spec_command("all tests") + ["--no-run"]
        no_run["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(no_run)
        ).hexdigest()
        no_run_path = write(tmp, no_run, "no_run.json")
        expect_reject("--no-run appended to all tests", [no_run_path])

        skip_flag = make_record_core()
        skip_flag["checks"][all_tests]["command"] = spec_command("all tests") + ["--skip", "locking_tests"]
        skip_flag["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(skip_flag)
        ).hexdigest()
        skip_path = write(tmp, skip_flag, "skip_flag.json")
        expect_reject("--skip appended to all tests", [skip_path])

        clippy_index = spec_index("core", "clippy")
        extra_flag = make_record_core()
        extra_flag["checks"][clippy_index]["command"] = spec_command("clippy") + ["--no-deps"]
        extra_flag["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(extra_flag)
        ).hexdigest()
        extra_flag_path = write(tmp, extra_flag, "extra_flag.json")
        expect_reject("extra clippy flag", [extra_flag_path])

        wrong_program = make_record_core()
        wrong_program["checks"][0]["command"] = ["make", "fmt", "--all", "--", "--check"]
        wrong_program["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(wrong_program)
        ).hexdigest()
        wrong_program_path = write(tmp, wrong_program, "wrong_program.json")
        expect_reject("wrong program identity", [wrong_program_path])

        missing_args = make_record_core()
        missing_args["checks"][3]["command"] = ["cargo", "build"]  # release build without --release
        missing_args["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(missing_args)
        ).hexdigest()
        missing_args_path = write(tmp, missing_args, "missing_args.json")
        expect_reject("missing required argument", [missing_args_path])

        python_check_as_cargo = make_record_core()
        python_check_as_cargo["checks"][9]["command"] = ["cargo", "scripts/test_local_ci_verify.py"]
        python_check_as_cargo["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(python_check_as_cargo)
        ).hexdigest()
        python_cargo_path = write(tmp, python_check_as_cargo, "python_as_cargo.json")
        expect_reject("python check run by cargo", [python_cargo_path])

        impostor = make_record_suite("smoke")
        installed_version = spec_index("smoke", "installed version")
        impostor["checks"][installed_version]["command"] = [
            "C:/x/.local-ci/install/bin/impostor.exe",
            "--version",
        ]
        impostor["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(impostor)
        ).hexdigest()
        impostor_path = write(tmp, impostor, "impostor_binary.json")
        core_ok = write(tmp, make_record_core(), "core_ok.json")
        platform_ok = write(tmp, make_record_suite("platform"), "platform_ok.json")
        expect_reject(
            "binary check with wrong basename",
            [core_ok, platform_ok, impostor_path],
        )

        outside_root = make_record_suite("smoke")
        outside_root["checks"][installed_version]["command"] = ["prometheos", "--version"]
        outside_root["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(outside_root)
        ).hexdigest()
        outside_root_path = write(tmp, outside_root, "outside_root.json")
        expect_reject(
            "binary outside install root",
            [core_ok, platform_ok, outside_root_path],
        )

        # ------------------------------------------------------------------
        # Duration validation: finite, non-negative, non-boolean
        # ------------------------------------------------------------------
        for label, value in (
            ("negative duration", -1.0),
            ("boolean duration", True),
            ("NaN duration", float("nan")),
            ("infinite duration", float("inf")),
        ):
            record = make_record_core()
            record["checks"][0]["seconds"] = value
            record["evidenceDigest"] = hashlib.sha256(
                local_ci.canonical_bytes(record)
            ).hexdigest()
            path = write(tmp, record, f"duration_{label.split()[0]}.json")
            expect_reject(label, [path])

        # ------------------------------------------------------------------
        # Generation-time regressions: the runner refuses to WRITE bad evidence
        # ------------------------------------------------------------------
        expect_generation_refusal(
            "--no-run in generated all tests",
            "core",
            core_checks_with(all_tests, command=spec_command("all tests") + ["--no-run"]),
        )
        expect_generation_refusal(
            "negative duration in generated checks",
            "core",
            core_checks_with(0, seconds=-5.0),
        )
        expect_generation_refusal(
            "boolean duration in generated checks",
            "core",
            core_checks_with(0, seconds=True),
        )
        expect_generation_refusal(
            "all-true commands in generated checks",
            "core",
            [make_check(name, ["true"]) for name in CORE],
        )
        expect_generation_refusal(
            "failed status in generated checks",
            "core",
            core_checks_with(0, status="failed"),
        )
        expect_generation_refusal(
            "unknown check key in generated checks",
            "core",
            [dict(make_check(name), notes="x") for name in CORE],
        )
        try:
            local_ci.validate_checks_against_spec("core", [make_check(name) for name in CORE])
        except RuntimeError as error:
            print(f"FAIL: generation self-check rejected VALID checks: {error}")
            sys.exit(1)
        print("PASS generation acceptance: valid spec-shaped checks pass the self-check")

        # ------------------------------------------------------------------
        # Cross-file failures
        # ------------------------------------------------------------------
        core_a = write(tmp, make_record_core(), "core_a.json")
        core_b = write(tmp, make_record_core(), "core_b.json")
        platform_a = write(tmp, make_record_suite("platform"), "platform_a.json")
        smoke_a = write(tmp, make_record_suite("smoke"), "smoke_a.json")
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
        # Frontend suite support
        # ------------------------------------------------------------------
        frontend_ok = write(tmp, make_record_suite("frontend"), "frontend_ok.json")
        expect_accept(
            "valid four-suite evidence incl. frontend",
            [core_a, platform_a, smoke_a, frontend_ok],
        )

        expect_reject(
            "missing frontend evidence",
            [core_a, platform_a, smoke_a],
            frontend=True,
        )

        frontend_true = make_record_suite("frontend")
        for check in frontend_true["checks"]:
            check["command"] = ["true"]
        frontend_true["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(frontend_true)
        ).hexdigest()
        frontend_true_path = write(tmp, frontend_true, "frontend_true.json")
        expect_reject(
            "frontend all-true commands",
            [core_a, platform_a, smoke_a, frontend_true_path],
        )

        frontend_missing = make_record_suite(
            "frontend", [name for name in FRONTEND if name != "frontend lint"]
        )
        frontend_missing_path = write(tmp, frontend_missing, "frontend_missing.json")
        expect_reject(
            "frontend missing check",
            [core_a, platform_a, smoke_a, frontend_missing_path],
        )

        # ------------------------------------------------------------------
        # The valid three-suite configuration (frontend optional by default)
        # ------------------------------------------------------------------
        expect_accept("valid three-suite evidence", [core_a, platform_a, smoke_a])
        expect_accept(
            "valid evidence with platform requirement",
            [core_a, platform_a, smoke_a],
            platforms=["windows"],
        )

    print("\nPASS: all verifier regressions behave fail-closed")
    return 0


# --- record helpers -----------------------------------------------------------

def make_record_core(
    names: list[str] | None = None,
    *,
    digest: bool = True,
    **overrides: object,
) -> dict[str, object]:
    names = names if names is not None else CORE
    record: dict[str, object] = {
        "schemaVersion": SCHEMA,
        "commit": COMMIT,
        "suite": "core",
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


def make_record_suite(
    suite: str,
    names: list[str] | None = None,
    *,
    digest: bool = True,
) -> dict[str, object]:
    spec_names = [entry["name"] for entry in local_ci.SUITE_SPEC[suite]]
    names = names if names is not None else spec_names
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
    if digest:
        record["evidenceDigest"] = hashlib.sha256(
            local_ci.canonical_bytes(record)
        ).hexdigest()
    return record


if __name__ == "__main__":
    raise SystemExit(main())
