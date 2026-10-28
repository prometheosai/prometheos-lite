#!/usr/bin/env python3
"""MG2 orphan CLI detection regression tests.

These tests verify the MG2 orphan CLI surface detector behaves correctly:
- A. A new unmanifested CLI command module is rejected
- B. An unmanifested Command::new() public command is rejected
- C. An unmanifested Subcommand enum variant is rejected
- D. The corresponding correctly manifested/approved forms pass
- E. Unrelated legitimate modules do not produce MG2 false positives
- F. MG2 baseline missing fails closed
"""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator

ROOT = Path(__file__).resolve().parents[1]
GUARD = ROOT / "scripts" / "gb_static_guard.py"
PIN_REL = "scripts/reviewed_config_digests.json"

BASELINE_FILES = [
    "reviewed_authority_baselines.json",
    "authority_pub_surface_baseline.json",
    "authority_resolver_coverage_manifest.json",
    "gate_completion_registry.json",
    "reviewed_authorization_surfaces.json",
    "reviewed_config_digests.json",
]


def lf_normalized_digest(path: Path) -> str:
    """LF-normalized digest, matching scripts/local_ci_policy.py."""
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read().replace(b"\r\n", b"\n")).hexdigest()


@contextmanager
def sandbox(*, git: bool = True) -> Iterator[Path]:
    """A disposable repository root holding the real guard and pin set."""
    work = Path(tempfile.mkdtemp(prefix="prometheos-mg2-test-"))
    try:
        (work / "scripts").mkdir(parents=True, exist_ok=True)
        (work / "src" / "cli" / "commands").mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / "scripts" / "gb_static_guard.py", work / "scripts" / "gb_static_guard.py")
        # Copy all required baseline files
        for f in BASELINE_FILES:
            shutil.copy2(ROOT / "scripts" / f, work / "scripts" / f)
        if git:
            subprocess.run(["git", "init", "-q"], cwd=str(work), capture_output=True, check=True)
            track(work)
        yield work
    finally:
        shutil.rmtree(work, ignore_errors=True)


def track(root: Path, *relatives: str) -> None:
    if not relatives:
        subprocess.run(["git", "add", "-A", "."], cwd=str(root), capture_output=True, check=False)
        return
    for relative in relatives:
        subprocess.run(["git", "add", "--", relative], cwd=str(root), capture_output=True, check=False)


def run_guard(root: Path) -> tuple[int, str]:
    # Run guard on repo root (not src/ subdirectory)
    result = subprocess.run(
        [sys.executable, str(root / "scripts" / "gb_static_guard.py"), "."],
        capture_output=True,
        text=True,
        cwd=str(root),
    )
    return result.returncode, result.stdout + result.stderr


def expect_pass(label: str, root: Path) -> None:
    code, output = run_guard(root)
    if code == 0:
        print(f"PASS accept: {label}")
    else:
        print(f"FAIL accept: {label} was REJECTED (exit {code})")
        for line in output.strip().splitlines():
            print(f"    {line}")
        raise SystemExit(1)


def expect_fail(label: str, root: Path, *must_contain: str) -> None:
    code, output = run_guard(root)
    if code == 0:
        print(f"FAIL reject: {label} was ACCEPTED (fails open)")
        raise SystemExit(1)
    missing = [needle for needle in must_contain if needle not in output]
    if "Traceback" in output:
        missing.append("no unhandled traceback")
    if missing:
        print(f"FAIL reject: {label} did not report {missing}")
        for line in output.strip().splitlines():
            print(f"    {line}")
        raise SystemExit(1)
    print(f"PASS reject: {label}")


def update_baseline_and_pin(root: Path, module_name: str) -> None:
    """Add a module to the CLI surface baseline and update the pin digest."""
    # Update the baseline
    baselines_path = root / "scripts" / "reviewed_authority_baselines.json"
    baselines = json.loads(baselines_path.read_text(encoding="utf-8"))
    if "cli_surface_baseline" not in baselines:
        baselines["cli_surface_baseline"] = {"modules": [], "verbs": []}
    if "modules" not in baselines["cli_surface_baseline"]:
        baselines["cli_surface_baseline"]["modules"] = []
    if module_name not in baselines["cli_surface_baseline"]["modules"]:
        baselines["cli_surface_baseline"]["modules"].append(module_name)
    baselines_path.write_text(json.dumps(baselines, indent=2), encoding="utf-8")
    
    # Update the pin file digest
    pin_path = root / "scripts" / "reviewed_config_digests.json"
    pins = json.loads(pin_path.read_text(encoding="utf-8"))
    pinned_key = "scripts/reviewed_authority_baselines.json"
    if "files" in pins:
        pins["files"][pinned_key] = lf_normalized_digest(baselines_path)
    pin_path.write_text(json.dumps(pins, indent=2), encoding="utf-8")


def main() -> int:
    # A. A new unmanifested CLI command module is rejected
    with sandbox() as root:
        (root / "src" / "cli" / "commands").mkdir(parents=True, exist_ok=True)
        (root / "src" / "cli" / "commands" / "new_unmanifested.rs").write_text(
            'pub fn run() { println!("unmanifested") }\n', encoding="utf-8"
        )
        track(root)
        code, output = run_guard(root)
        if code != 0 and "mg2_unmanifested_entry_point" in output:
            print("PASS reject: MG2: new unmanifested CLI command module is rejected")
        else:
            print(f"FAIL reject: MG2: new unmanifested CLI command module is rejected")
            print(f"  exit={code}, output={output[:500]}")
            return 1

    # B. An unmanifested Command::new() public command is rejected
    with sandbox() as root:
        (root / "src" / "cli" / "commands" / "new_command.rs").write_text(
            'fn run() { Command::new("unmanifested_cmd").get_matches(); }\n', encoding="utf-8"
        )
        track(root)
        code, output = run_guard(root)
        if code != 0 and "mg2_unmanifested_entry_point" in output:
            print("PASS reject: MG2: unmanifested Command::new() public command is rejected")
        else:
            print(f"FAIL reject: MG2: unmanifested Command::new() public command is rejected")
            print(f"  exit={code}, output={output[:500]}")
            return 1

    # C. An unmanifested Subcommand enum variant is rejected
    with sandbox() as root:
        (root / "src" / "cli" / "commands" / "new_subcommand.rs").write_text(
            '#[derive(clap::Subcommand)]\nenum Commands {\n    UnmanifestedCmd,\n}\n', encoding="utf-8"
        )
        track(root)
        code, output = run_guard(root)
        if code != 0 and "mg2_unmanifested_entry_point" in output:
            print("PASS reject: MG2: unmanifested Subcommand enum variant is rejected")
        else:
            print(f"FAIL reject: MG2: unmanifested Subcommand enum variant is rejected")
            print(f"  exit={code}, output={output[:500]}")
            return 1

    # D. Correctly manifested/approved CLI module passes (requires baseline update + re-pin)
    with sandbox() as root:
        update_baseline_and_pin(root, "approved_new_module")
        
        # Create the approved module
        (root / "src" / "cli" / "commands" / "approved_new_module.rs").write_text(
            'pub fn run() { println!("approved") }\n', encoding="utf-8"
        )
        track(root)
        code, output = run_guard(root)
        if code == 0:
            print("PASS accept: MG2: correctly manifested CLI module passes")
        else:
            print(f"FAIL: MG2 positive test failed")
            print(f"  exit={code}, output={output[:500]}")
            return 1

    # E. Unrelated legitimate module does not produce MG2 false positive
    with sandbox() as root:
        (root / "src" / "cli" / "commands" / "mod.rs").write_text(
            '#[derive(clap::Subcommand)]\nenum Commands {\n    Bench,\n    Apply,\n}\n', encoding="utf-8"
        )
        track(root)
        code, output = run_guard(root)
        if code == 0:
            print("PASS accept: MG2: unrelated legitimate module does not produce false positive")
        else:
            if "mg2_unmanifested_entry_point" in output:
                print(f"FAIL: MG2 produced false positive on existing command")
                return 1
            else:
                print(f"FAIL: unexpected error")
                return 1

    # F. MG2 baseline missing fails closed (gb_baseline_unavailable for missing data file)
    with sandbox() as root:
        (root / "scripts" / "authority_pub_surface_baseline.json").unlink()
        track(root)
        # The guard should fail with gb_baseline_unavailable for missing data file
        code, output = run_guard(root)
        if code != 0 and "gb_baseline_unavailable" in output and "authority_pub_surface_baseline" in output:
            print("PASS reject: MG2: baseline missing fails closed (gb_baseline_unavailable)")
        else:
            print(f"FAIL reject: MG2: baseline missing fails closed")
            print(f"  exit={code}, output={output[:500]}")
            return 1

    print("\nPASS: all MG2 regression tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())