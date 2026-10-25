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
def sandbox(*, git: bool = True) -> Iterator[Path]:
    """A disposable repository root holding the real gate and pin set.

    By default the sandbox is a git repository with its files tracked, so
    the gate exercises the same tracked-file path it uses in the real
    repository. Pass ``git=False`` to exercise the filesystem fallback
    instead. Both paths are part of the gate's behaviour.
    """
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
        if git:
            subprocess.run(["git", "init", "-q"], cwd=str(work), capture_output=True, check=True)
            track(work)
        yield work
    finally:
        shutil.rmtree(work, ignore_errors=True)


def track(root: Path, *relatives: str) -> None:
    """Mark files as tracked, so the gate's tracked-file path sees them."""
    if not relatives:
        subprocess.run(
            ["git", "add", "-A", "."], cwd=str(root), capture_output=True, check=False
        )
        return
    for relative in relatives:
        subprocess.run(
            ["git", "add", "--", relative], cwd=str(root), capture_output=True, check=False
        )


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
        track(root, "config.json")
        expect_fail(
            "duplicate key in a tracked repository JSON document",
            root,
            "duplicate key",
        )

    with sandbox(git=False) as root:
        (root / "config.json").write_text('{"a": 1, "a": 2}', encoding="utf-8")
        expect_fail(
            "duplicate key in a JSON document found by the filesystem fallback",
            root,
            "duplicate key",
        )

    with sandbox() as root:
        target = root / "tests" / "fixtures" / "slice3" / "invalid"
        target.mkdir(parents=True, exist_ok=True)
        (target / "new-invalid.json").write_text('{"b": 1, "b": 2}', encoding="utf-8")
        track(root, "tests/fixtures/slice3/invalid/new-invalid.json")
        expect_fail(
            "new invalid fixture inside a previously exempt subtree",
            root,
            "duplicate key",
        )

    with sandbox() as root:
        target = root / "tests" / "fixtures" / "slice3" / "invalid"
        target.mkdir(parents=True, exist_ok=True)
        (target / "duplicate-key.json").write_text('{"a": 1, "a": 2}', encoding="utf-8")
        track(root, "tests/fixtures/slice3/invalid/duplicate-key.json")
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

    # The discriminating case: BOTH roots exist and the violation is in
    # src/api. A gate that scans only one of the two roots picks the
    # top-level api/ here and misses the violation, so this case is the
    # one that distinguishes "scans every root" from "scans one root".
    with sandbox() as root:
        (root / "api").mkdir(parents=True, exist_ok=True)
        (root / "api" / "clean.rs").write_text("fn go() {}\n", encoding="utf-8")
        (root / "src" / "api" / "client.rs").write_text(
            "fn go() { llm_client.generate(); }\n", encoding="utf-8"
        )
        expect_fail(
            "LLM call boundary violation under src/api while a top-level api/ also exists",
            root,
            "llm_client.generate",
        )

    with sandbox() as root:
        (root / "api").mkdir(parents=True, exist_ok=True)
        (root / "api" / "client.rs").write_text(
            "fn go() { LlmClient::generate(); }\n", encoding="utf-8"
        )
        (root / "src" / "api" / "clean.rs").write_text("fn go() {}\n", encoding="utf-8")
        expect_fail(
            "LLM call boundary violation under a top-level api/ while src/api also exists",
            root,
            "LlmClient::generate",
        )

    # -- filesystem-level pathologies are reported, not raised ---------------
    with sandbox() as root:
        (root / PIN_REL).unlink()
        (root / PIN_REL).mkdir(parents=True, exist_ok=True)
        expect_fail(
            "pin file is a directory",
            root,
            "unreadable pin file",
        )

    with sandbox() as root:
        pins = json.loads((root / PIN_REL).read_text(encoding="utf-8"))
        pins["files"]["scripts"] = "0" * 64
        (root / PIN_REL).write_text(json.dumps(pins, indent=2), encoding="utf-8")
        expect_fail(
            "pin entry names a directory",
            root,
            "unreadable",
        )

    with sandbox() as root:
        (root / "src" / "api" / "broken.rs").write_bytes(b"fn go() {}\n\xff\xfe invalid utf-8\n")
        expect_pass(
            "source file with invalid UTF-8 does not crash the gate",
            root,
        )

    # -- the fallback-confinement half of the boundary rule ------------------
    with sandbox() as root:
        (root / "src" / "other.rs").write_text(
            "fn go() { with_fallback_allowed(); }\n", encoding="utf-8"
        )
        expect_fail(
            "fallback confinement: with_fallback_allowed outside src/tools/repo.rs",
            root,
            "with_fallback_allowed",
        )

    with sandbox() as root:
        tools = root / "src" / "tools"
        tools.mkdir(parents=True, exist_ok=True)
        # src/tools/repo.rs is a G-B authority home: the sandbox must carry
        # its REVIEWED public surface, or the guard (correctly) reports the
        # baselined items as removals. The exemption under test is the
        # section-1 exclusion of this exact path, so the real content is
        # copied and the probe line appended (a private item, no surface
        # change).
        real = (ROOT / "src" / "tools" / "repo.rs").read_text(encoding="utf-8")
        (tools / "repo.rs").write_text(
            real + "\nfn go() { with_fallback_allowed(); }\n", encoding="utf-8"
        )
        expect_pass(
            "fallback confinement: with_fallback_allowed inside src/tools/repo.rs",
            root,
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

    # -- G-B static guard: CI enforcement ------------------------------------
    with sandbox() as root:
        (root / "src" / "flow").mkdir(parents=True, exist_ok=True)
        (root / "src" / "flow" / "adapter.rs").write_text(
            "pub use crate::tools::TrustRegistry;\n", encoding="utf-8"
        )
        expect_fail(
            "authority re-export outside a reviewed re-export file",
            root,
            "mg1_authority_reexport_outside_site",
        )

    with sandbox() as root:
        db = root / "src" / "db" / "repository"
        db.mkdir(parents=True, exist_ok=True)
        (db / "mod.rs").write_text("pub use trust_policies::*;\n", encoding="utf-8")
        expect_fail(
            "authority-namespace glob re-export (the adjudicated defect)",
            root,
            "mg1_glob_import_of_authority_namespace",
        )

    with sandbox() as root:
        db = root / "src" / "db" / "repository"
        db.mkdir(parents=True, exist_ok=True)
        (db / "mod.rs").write_text(
            "pub use crate::tools::TrustRegistry;\n", encoding="utf-8"
        )
        expect_fail(
            "a NEW authority symbol re-exported at a reviewed site",
            root,
            "mg1_new_authority_reexport",
        )

    with sandbox() as root:
        db = root / "src" / "db" / "repository"
        db.mkdir(parents=True, exist_ok=True)
        (db / "mod.rs").write_text(
            "pub use trust_policies::{TrustPolicyEntry, TrustPolicyOperations};\n",
            encoding="utf-8",
        )
        (root / "src" / "cli" / "runner").mkdir(parents=True, exist_ok=True)
        (root / "src" / "cli" / "runner" / "mod.rs").write_text(
            "mod flow_runner;\n#[cfg(test)]\nmod tests;\n", encoding="utf-8"
        )
        expect_pass(
            "the two adjudicated production fixes are guard-clean "
            "(enumerated re-export at a reviewed site; confined test module)",
            root,
        )

    with sandbox() as root:
        (root / "src" / "cli" / "runner").mkdir(parents=True, exist_ok=True)
        (root / "src" / "cli" / "runner" / "mod.rs").write_text(
            "mod flow_runner;\nmod tests;\n", encoding="utf-8"
        )
        expect_fail(
            "unconfined test module (the second adjudicated defect)",
            root,
            "mg1_unconfined_test_module",
        )

    with sandbox() as root:
        (root / "src" / "harness").mkdir(parents=True, exist_ok=True)
        (root / "src" / "harness" / "permissions.rs").write_text(
            "pub struct DeniedOperation {\n    pub why: String,\n}\n", encoding="utf-8"
        )
        expect_fail(
            "visibility widening of a baselined restricted item",
            root,
            "mg3_public_visibility_widening",
        )

    with sandbox() as root:
        (root / "src" / "harness").mkdir(parents=True, exist_ok=True)
        (root / "src" / "harness" / "permissions.rs").write_text(
            "pub (crate) struct DeniedOperation {\n    pub why: String,\n}\n",
            encoding="utf-8",
        )
        expect_fail(
            "visibility REMOVAL side: a baselined home item absent from the file",
            root,
            "mg3_public_surface_removal",
        )

    # -- G-B static guard: valid-Rust negative controls (no false positives) --
    with sandbox() as root:
        (root / "src" / "tools").mkdir(parents=True, exist_ok=True)
        # Private items only: the control is that authority-shaped TEXT in
        # comments, strings, raw strings and byte strings is not a finding.
        # (New PUBLIC items in a home are additions by design and are
        # exercised above.)
        (root / "src" / "tools" / "policy.rs").write_text(
            "//! pub use crate::tools::TrustRegistry; (doc mention)\n"
            "// pub use crate::tools::TrustRegistry;\n"
            "/* pub use crate::tools::TrustRegistry; */\n"
            "fn doc() -> &'static str {\n"
            "    \"pub use crate::tools::TrustRegistry;\"\n"
            "}\n"
            "fn raw() -> &'static str {\n"
            "    r#\"pub use crate::tools::TrustRegistry;\"#\n"
            "}\n"
            "fn bytes() -> &'static [u8] {\n"
            "    b\"pub use crate::tools::TrustRegistry;\"\n"
            "}\n"
            "fn notpub() { let notpub_use = 1; }\n",
            encoding="utf-8",
        )
        expect_pass(
            "authority patterns in comments, strings, raw strings and byte strings "
            "are not findings",
            root,
        )

    with sandbox() as root:
        (root / "src" / "harness").mkdir(parents=True, exist_ok=True)
        (root / "src" / "harness" / "permissions.rs").write_text(
            "pub (crate)  struct   DeniedOperation {\n    pub why: String,\n}\n",
            encoding="utf-8",
        )
        expect_fail(
            "whitespace-separated qualifiers canonicalize (still a removal on the "
            "single-item fixture, never a false widening)",
            root,
            "mg3_public_surface_removal",
        )

    with sandbox() as root:
        build = root / ".cargo-target" / "debug" / "build"
        build.mkdir(parents=True, exist_ok=True)
        (build / "generated.rs").write_text(
            "pub use crate::tools::TrustRegistry;\n", encoding="utf-8"
        )
        expect_pass(
            "generated build trees are outside the declared scan scope",
            root,
        )

    # -- G-B static guard: reviewed-data trust boundary ----------------------
    with sandbox() as root:
        # A JSON-valid but unreviewed baseline change is caught by the
        # CURATED PIN, not by the detector: the pin digest must move only
        # through the deliberate re-pin act.
        surfaces = root / "scripts" / "reviewed_authorization_surfaces.json"
        data = json.loads(surfaces.read_text(encoding="utf-8"))
        data["reviewed_reexport_files"].append("src/flow/adapter.rs")
        surfaces.write_text(json.dumps(data, indent=2), encoding="utf-8")
        expect_fail(
            "tampered reviewed-data file without a reviewed pin update",
            root,
            "changed without a reviewed pin update",
        )

    with sandbox() as root:
        # After the deliberate re-pin act the gate is green again. This is
        # the trust boundary in executable form: the re-pin is the review
        # artifact a human must inspect; the gate cannot itself authorize.
        surfaces = root / "scripts" / "reviewed_authorization_surfaces.json"
        data = json.loads(surfaces.read_text(encoding="utf-8"))
        data["reviewed_reexport_files"].append("src/flow/adapter.rs")
        surfaces.write_text(json.dumps(data, indent=2), encoding="utf-8")
        composite = root / "scripts" / "reviewed_authority_baselines.json"
        baselines = json.loads(composite.read_text(encoding="utf-8"))
        baselines["reexport_baseline"]["src/flow/adapter.rs"] = ["TrustRegistry"]
        composite.write_text(json.dumps(baselines, indent=2), encoding="utf-8")
        subprocess.run(
            [sys.executable, "scripts/review_config_digests.py"],
            cwd=str(root), capture_output=True, text=True, check=True,
        )
        (root / "src" / "flow").mkdir(parents=True, exist_ok=True)
        (root / "src" / "flow" / "adapter.rs").write_text(
            "pub use crate::tools::TrustRegistry;\n", encoding="utf-8"
        )
        expect_pass(
            "deliberate re-pin makes a reviewed surface change reviewable and "
            "enforceable (the re-pin is the human-review artifact)",
            root,
        )

    with sandbox() as root:
        pins = json.loads((root / PIN_REL).read_text(encoding="utf-8"))
        del pins["files"]["scripts/authority_pub_surface_baseline.json"]
        (root / PIN_REL).write_text(json.dumps(pins, indent=2), encoding="utf-8")
        expect_fail(
            "consumed baseline missing from the curated pin set",
            root,
            "gb_unreviewed_baseline_pin",
        )

    with sandbox() as root:
        (root / "scripts" / "authority_pub_surface_baseline.json").unlink()
        expect_fail(
            "missing reviewed baseline fails closed",
            root,
            "pinned executable configuration is missing",
        )

    with sandbox() as root:
        # Missing AND unpinned AND untracked: neither the curated pin check
        # nor the repository JSON scan covers it, so the failure must come
        # from the GUARD's own load - proving the guard fails closed rather
        # than silently scanning with no baseline.
        (root / "scripts" / "authority_pub_surface_baseline.json").unlink()
        subprocess.run(
            ["git", "rm", "--cached", "-q", "scripts/authority_pub_surface_baseline.json"],
            cwd=str(root), capture_output=True, check=False,
        )
        pins = json.loads((root / PIN_REL).read_text(encoding="utf-8"))
        del pins["files"]["scripts/authority_pub_surface_baseline.json"]
        (root / PIN_REL).write_text(json.dumps(pins, indent=2), encoding="utf-8")
        expect_fail(
            "missing reviewed baseline fails closed at the guard, not silently",
            root,
            "gb_baseline_unavailable",
        )

    with sandbox() as root:
        # Corrupt JSON: after the deliberate re-pin the pin check is green,
        # so the failure must come from the repository's own JSON-validity
        # gate - still fail-closed, one layer earlier than the guard.
        (root / "scripts" / "authority_pub_surface_baseline.json").write_text(
            "{ this is not json", encoding="utf-8"
        )
        subprocess.run(
            [sys.executable, "scripts/review_config_digests.py"],
            cwd=str(root), capture_output=True, text=True, check=True,
        )
        expect_fail(
            "corrupt reviewed baseline fails closed (repository JSON gate)",
            root,
            "Expecting property name",
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
            "PASS: G-B static guard (reviewed authority surfaces and baselines)",
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
