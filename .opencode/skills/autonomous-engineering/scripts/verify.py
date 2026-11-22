#!/usr/bin/env python3
"""Deterministic local verification with fail-closed, exact-revision evidence.

Only commands explicitly declared in the task contract AND present in the
approved-command policy are executed. There is no denylist and no shell.
The candidate HEAD and tree are checked before and after every run; any drift
invalidates the evidence. Only digests of output are stored.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import common as cm

SCHEMA = "autonomous-evidence/2"
SKILL = Path(__file__).resolve().parent.parent
POLICY = SKILL / "policies" / "approved-commands.json"


class VerifyError(cm.LoopError):
    pass


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def load_policy() -> tuple[dict[str, Any], str]:
    if not POLICY.is_file():
        raise VerifyError("missing approved-command policy")
    data = cm.read_json(POLICY)
    if data.get("schema") != "autonomous-verification-policy/1":
        raise VerifyError("wrong verification policy schema")
    return data, cm.sha256_file(POLICY)


def load_contract(path: str) -> dict[str, Any]:
    data = cm.read_json(path)
    if data.get("schema") != "autonomous-task/2":
        raise VerifyError("wrong task contract schema")
    for key in ("task_id", "worktree", "commands"):
        if key not in data:
            raise VerifyError(f"contract missing {key}")
    if not isinstance(data["commands"], list) or not data["commands"]:
        raise VerifyError("contract must declare at least one command")
    return data


def guard(argv: list[str], policy: dict[str, Any]) -> None:
    if not isinstance(argv, list) or not argv or not all(isinstance(x, str) for x in argv):
        raise VerifyError("command argv must be a non-empty list of strings")
    if any("\x00" in part for part in argv):
        raise VerifyError("NUL byte in command")
    program = Path(argv[0]).stem  # handles python.exe / python3 on Windows
    approved = policy["approved_executables"]
    if program not in approved:
        raise VerifyError(f"command not approved by policy: {program}")
    if program == "git":
        sub = next((a for a in argv[1:] if not a.startswith("-")), "")
        if sub not in policy["approved_git_subcommands"]:
            raise VerifyError(f"git subcommand not approved: {sub}")


def revision(repo: str, sha: str) -> tuple[str, str]:
    head = cm.git(repo, "rev-parse", "HEAD")
    tree = cm.git(repo, "rev-parse", "HEAD^{tree}")
    if head != sha:
        raise VerifyError(f"worktree HEAD {head} != candidate {sha}")
    return head, tree


def run_one(command: dict[str, Any], cwd: str, policy: dict[str, Any]) -> dict[str, Any]:
    argv = command["argv"]
    guard(argv, policy)
    timeout = int(command.get("timeout_seconds", policy["default_timeout_seconds"]))
    if timeout <= 0 or timeout > policy["max_timeout_seconds"]:
        raise VerifyError("timeout outside policy bounds")
    started = time.monotonic()
    try:
        proc = subprocess.run(
            argv, cwd=cwd, capture_output=True, text=True,
            timeout=timeout, check=False,
        )
        code: int | None = proc.returncode
        out, err = proc.stdout, proc.stderr
    except subprocess.TimeoutExpired:
        code, out, err = None, "", "TIMEOUT"
    duration = round(time.monotonic() - started, 3)
    cap = int(policy["max_output_bytes"])
    oversized = len(out) + len(err) > cap
    status = "passed" if code == 0 and not oversized else "failed"
    return {
        "id": command["id"],
        "argv": argv,
        "status": status,
        "exit_code": code,
        "duration_seconds": duration,
        "reason": "output_cap_exceeded" if oversized else ("timeout" if code is None else ""),
        "stdout_sha256": cm.sha256_text(out[:cap]),
        "stderr_sha256": cm.sha256_text(err[:cap]),
    }


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["run", "validate-evidence"])
    p.add_argument("--repo", required=True)
    p.add_argument("--contract")
    p.add_argument("--output")
    p.add_argument("--candidate")
    p.add_argument("--only")
    p.add_argument("--skip")
    p.add_argument("--skip-reason", default="operator excluded with recorded policy")
    args = p.parse_args(argv)
    try:
        if args.action == "validate-evidence":
            data = cm.read_json(args.output)
            if data.get("schema") != SCHEMA:
                raise VerifyError("wrong evidence schema")
            print("EVIDENCE_OK")
            return 0

        policy, policy_sha = load_policy()
        contract = load_contract(str(args.contract))
        candidate = args.candidate or cm.git(args.repo, "rev-parse", "HEAD")
        cm.validate_sha(candidate, "candidate")
        cm.git(args.repo, "cat-file", "-e", f"{candidate}^{{commit}}")
        tree = cm.git(args.repo, "rev-parse", f"{candidate}^{{tree}}")
        cwd = str(Path(contract["worktree"]).resolve())
        root = str(cm.repo_root(args.repo).resolve())
        if not cm.git_ok(cwd, "rev-parse", "--is-inside-work-tree"):
            raise VerifyError("contract worktree is missing or not a git worktree")
        if Path(cm.git(cwd, "rev-parse", "--show-toplevel")).resolve() != Path(cwd).resolve():
            raise VerifyError("contract worktree path is not its git root")
        wt_common = Path(cm.git(cwd, "rev-parse", "--path-format=absolute",
                                "--git-common-dir")).resolve()
        repo_common = Path(cm.git(root, "rev-parse", "--path-format=absolute",
                                  "--git-common-dir")).resolve()
        if wt_common != repo_common:
            raise VerifyError("contract worktree does not belong to this repository")
        pre_head, pre_tree = revision(cwd, candidate)
        commands = contract["commands"]
        if len(commands) > policy["max_commands"]:
            raise VerifyError("too many commands")
        selected = set(args.only.split(",")) if args.only else None
        skipped = set(args.skip.split(",")) if args.skip else set()
        declared = {c["id"] for c in commands}
        if selected and not selected <= declared:
            raise VerifyError("--only contains unknown check ids")
        if skipped and not skipped <= declared:
            raise VerifyError("--skip contains unknown check ids")
        results: list[dict[str, Any]] = []
        started_at = now()
        for command in commands:
            cid = command["id"]
            if cid in skipped:
                item = {"id": cid, "argv": command["argv"], "status": "skipped",
                        "exit_code": None, "reason": args.skip_reason,
                        "duration_seconds": 0.0,
                        "stdout_sha256": "0" * 64, "stderr_sha256": "0" * 64}
            elif selected is not None and cid not in selected:
                item = {"id": cid, "argv": command["argv"], "status": "untested",
                        "exit_code": None, "reason": "not selected by --only",
                        "duration_seconds": 0.0,
                        "stdout_sha256": "0" * 64, "stderr_sha256": "0" * 64}
            else:
                item = run_one(command, cwd, policy)
            results.append(item)
        post_head, post_tree = revision(cwd, candidate)
        summary = {s: sum(1 for r in results if r["status"] == s)
                   for s in ("passed", "failed", "skipped", "untested", "invalidated")}
        evidence = {
            "schema": SCHEMA,
            "task_id": contract["task_id"],
            "candidate_sha": candidate,
            "tree_sha": tree,
            "pre_head": pre_head,
            "post_head": post_head,
            "pre_tree": pre_tree,
            "post_tree": post_tree,
            "policy_sha256": policy_sha,
            "resource_limits": {
                "max_commands": policy["max_commands"],
                "max_timeout_seconds": policy["max_timeout_seconds"],
                "max_output_bytes": policy["max_output_bytes"],
            },
            "started_at": started_at,
            "finished_at": now(),
            "results": results,
            "summary": summary,
        }
        cm.atomic_json_write(str(args.output), evidence)
        print(json.dumps(summary, sort_keys=True))
        if summary["failed"] or summary["untested"]:
            return 1
        return 0
    except (cm.LoopError, OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"VERIFY_ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
