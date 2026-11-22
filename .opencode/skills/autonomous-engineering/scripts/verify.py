#!/usr/bin/env python3
"""Fail-closed local verification with exact-revision and evidence integrity.

Authorization comes from reviewed argument templates and trusted executable
resolution, never from contract-supplied argv alone. Output is bounded WHILE a
command runs. This is NOT an OS sandbox.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import common as cm

SCHEMA = "autonomous-evidence/3"
SKILL = Path(__file__).resolve().parent.parent
POLICY = SKILL / "policies" / "approved-commands.json"
STATUSES = ("passed", "failed", "skipped", "untested", "invalidated")


class VerifyError(cm.LoopError):
    pass


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def load_policy() -> tuple[dict[str, Any], str]:
    if not POLICY.is_file():
        raise VerifyError("missing approved-command policy")
    data = cm.read_json(POLICY)
    if data.get("schema") != "autonomous-verification-policy/2":
        raise VerifyError("wrong verification policy schema")
    for key in ("approved_patterns", "max_commands", "default_timeout_seconds",
                "max_timeout_seconds", "max_output_bytes", "read_chunk_bytes",
                "process_tree_termination_guaranteed", "no_shell",
                "allowed_environment_keys", "sandbox", "required_negative_control_ids"):
        if key not in data:
            raise VerifyError(f"policy missing {key}")
    sandbox = data["sandbox"]
    if sandbox.get("require_sandbox") and sandbox.get("mode") == "unavailable":
        raise VerifyError("mandatory sandbox requested but isolation is unavailable")
    if data["no_shell"] is not True:
        raise VerifyError("policy must forbid shell execution")
    return data, cm.sha256_file(POLICY)


def load_contract(path: str) -> tuple[dict[str, Any], str]:
    data = cm.read_json(path)
    if data.get("schema") != "autonomous-task/2":
        raise VerifyError("wrong task contract schema")
    for key in ("task_id", "worktree", "commands"):
        if key not in data:
            raise VerifyError(f"contract missing {key}")
    if not isinstance(data["commands"], list) or not data["commands"]:
        raise VerifyError("contract must declare at least one command")
    return data, cm.sha256_file(path)


def authorize(command: dict[str, Any], policy: dict[str, Any],
              forbidden_roots: list[str]) -> dict[str, Any]:
    """Return the reviewed pattern authorizing this command, or fail closed."""
    if not isinstance(command, dict):
        raise VerifyError("command entry must be an object")
    pid = command.get("pattern_id")
    argv = command.get("argv")
    if not isinstance(pid, str) or not isinstance(argv, list) or not argv:
        raise VerifyError("command needs pattern_id and argv")
    if any(not isinstance(a, str) or "\x00" in a for a in argv):
        raise VerifyError("invalid argv")
    dangerous = ";&|`$<>()\n\r"
    if any(any(ch in a for ch in dangerous) for a in argv):
        raise VerifyError("shell metacharacter in verification argv")
    if any(a.startswith("/") or (len(a) > 1 and a[1] == ":") or ".." in a for a in argv[1:]):
        raise VerifyError("absolute path or parent traversal in verification argv")
    pattern = next((p for p in policy["approved_patterns"] if p.get("id") == pid), None)
    if pattern is None:
        raise VerifyError(f"command has no reviewed pattern: {pid}")
    if pattern.get("match") != "exact" or list(pattern.get("argv", [])) != list(argv):
        raise VerifyError(f"argv does not match reviewed template {pid}")
    resolved = cm.trusted_executable(argv[0], forbidden_roots)
    expected = pattern.get("resolved_path")
    if expected and Path(expected).resolve() != Path(resolved).resolve():
        raise VerifyError(f"executable path does not match reviewed template {pid}")
    return pattern


def revision(repo: str, sha: str) -> tuple[str, str]:
    head = cm.git(repo, "rev-parse", "HEAD")
    tree = cm.git(repo, "rev-parse", "HEAD^{tree}")
    if head != sha:
        raise VerifyError(f"worktree HEAD {head} != candidate {sha}")
    return head, tree


def result_counts(results: list[dict[str, Any]]) -> dict[str, int]:
    return {s: sum(1 for r in results if r.get("status") == s) for s in STATUSES}


def validate_evidence(data: Any, policy_sha: str, *, candidate: str | None = None,
                      tree: str | None = None, task_id: str | None = None) -> dict[str, Any]:
    """Full structural + binding validation with recomputed counts. No trust in summary."""
    if not isinstance(data, dict) or data.get("schema") != SCHEMA:
        raise VerifyError("wrong or missing evidence schema")
    required = {"schema", "task_id", "candidate_sha", "tree_sha", "pre_head", "post_head",
                "pre_tree", "post_tree", "pre_clean", "post_clean", "policy_sha256",
                "contract_sha256", "resource_limits", "mandatory_ids", "results",
                "summary", "green", "started_at", "finished_at",
                "process_tree_termination_guaranteed", "verification_seconds",
                "worktree", "repository", "branch", "check_classes", "negative_control_ids",
                "required_negative_control_ids", "sandbox_mode"}
    if set(data) != required:
        raise VerifyError(f"evidence fields mismatch: missing={sorted(required - set(data))} "
                          f"extra={sorted(set(data) - required)}")
    if candidate and data["candidate_sha"] != candidate:
        raise VerifyError("evidence bound to a different candidate")
    if tree and data["tree_sha"] != tree:
        raise VerifyError("evidence bound to a different tree")
    if task_id and data["task_id"] != task_id:
        raise VerifyError("evidence bound to a different task")
    for key in ("candidate_sha", "tree_sha", "pre_head", "post_head", "pre_tree", "post_tree"):
        cm.validate_sha(data[key], key)
    cm.validate_sha256(data["policy_sha256"], "policy_sha256")
    cm.validate_sha256(data["contract_sha256"], "contract_sha256")
    if data["policy_sha256"] != policy_sha:
        raise VerifyError("policy digest does not match current reviewed policy")
    live_wt = Path(str(data["worktree"])).resolve()
    try:
        live_repo = cm.repo_root(str(data["repository"]))
    except (OSError, subprocess.CalledProcessError) as exc:
        raise VerifyError("evidence repository cannot be resolved") from exc
    if not cm.git_ok(str(live_wt), "rev-parse", "--is-inside-work-tree"):
        raise VerifyError("evidence worktree is missing")
    if Path(cm.git(str(live_wt), "rev-parse", "--show-toplevel")).resolve() != live_wt:
        raise VerifyError("evidence worktree path does not match its git root")
    wt_common = Path(cm.git(str(live_wt), "rev-parse", "--path-format=absolute",
                            "--git-common-dir")).resolve()
    repo_common = Path(cm.git(str(live_repo), "rev-parse", "--path-format=absolute",
                              "--git-common-dir")).resolve()
    if wt_common != repo_common:
        raise VerifyError("evidence worktree belongs to a different repository")
    if cm.git(str(live_wt), "rev-parse", "--abbrev-ref", "HEAD") != data["branch"]:
        raise VerifyError("evidence branch does not match the worktree")
    required_nc = set(data["required_negative_control_ids"])
    present_nc = set(data["negative_control_ids"])
    if not required_nc <= present_nc:
        raise VerifyError("a required negative control is missing")
    if data["pre_head"] != data["candidate_sha"] or data["post_head"] != data["candidate_sha"]:
        raise VerifyError("evidence HEAD drift")
    if data["pre_tree"] != data["tree_sha"] or data["post_tree"] != data["tree_sha"]:
        raise VerifyError("evidence tree drift")
    if data["pre_clean"] is not True or data["post_clean"] is not True:
        raise VerifyError("worktree was not clean for verification")
    if not isinstance(data["results"], list) or not data["results"]:
        raise VerifyError("evidence has no results")
    if not isinstance(data["mandatory_ids"], list) or not data["mandatory_ids"]:
        raise VerifyError("evidence has no mandatory checks")
    results = data["results"]
    counts = result_counts(results)
    if data["summary"] != counts:
        raise VerifyError("forged result summary")
    seen: set[str] = set()
    for r in results:
        if not isinstance(r, dict) or set(r) < {"id", "argv", "status", "exit_code",
                                                 "duration_seconds", "stdout_sha256",
                                                 "stderr_sha256", "termination"}:
            raise VerifyError("malformed result entry")
        if r["id"] in seen:
            raise VerifyError("duplicate check id")
        seen.add(r["id"])
        if r["status"] not in STATUSES:
            raise VerifyError("invalid result status")
        cm.validate_sha256(r["stdout_sha256"], "stdout_sha256")
        cm.validate_sha256(r["stderr_sha256"], "stderr_sha256")
    classes = data["check_classes"]
    if not isinstance(classes, dict) or set(classes) != seen:
        raise VerifyError("check_classes must cover every result exactly once")
    if any(v not in {"mandatory", "optional", "conditional"} for v in classes.values()):
        raise VerifyError("unrecognized check class")
    mandatory = set(data["mandatory_ids"])
    if not mandatory <= seen:
        raise VerifyError("a mandatory check has no result")
    if any(classes[mid] != "mandatory" for mid in mandatory):
        raise VerifyError("a mandatory check is not classified mandatory")
    if not all(next(r for r in results if r["id"] == mid)["status"] == "passed"
               for mid in mandatory):
        raise VerifyError("a mandatory check did not pass")
    if counts["passed"] == 0 or counts["failed"] or counts["untested"] or counts["skipped"] or counts["invalidated"]:
        raise VerifyError("verification is not green")
    if data["green"] is not True:
        raise VerifyError("evidence is not marked green")
    return data


def run_one(command: dict[str, Any], cwd: str, policy: dict[str, Any],
            forbidden_roots: list[str]) -> dict[str, Any]:
    pattern = authorize(command, policy, forbidden_roots)
    timeout = int(command.get("timeout_seconds", policy["default_timeout_seconds"]))
    if timeout <= 0 or timeout > policy["max_timeout_seconds"]:
        raise VerifyError("timeout outside policy bounds")
    # Controlled environment: only reviewed keys pass through.
    allowed = set(policy["allowed_environment_keys"])
    env = {k: v for k, v in os.environ.items() if k in allowed}
    env["PYTHONDONTWRITEBYTECODE"] = "1"  # keep the verified worktree clean
    run = cm.run_bounded(list(command["argv"]), cwd, timeout,
                         int(policy["max_output_bytes"]), env)
    status = "passed" if run["exit_code"] == 0 and run["termination"] == "none" else "failed"
    return {
        "id": command["id"], "argv": list(command["argv"]), "pattern_id": pattern["id"],
        "status": status, "exit_code": run["exit_code"],
        "duration_seconds": run["duration_seconds"],
        "termination": run["termination"],
        "stdout_sha256": cm.sha256_text(run["stdout"]),
        "stderr_sha256": cm.sha256_text(run["stderr"]),
    }


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["run", "validate-evidence"])
    p.add_argument("--repo", required=True)
    p.add_argument("--contract")
    p.add_argument("--output")
    p.add_argument("--candidate")
    p.add_argument("--task-id")
    args = p.parse_args(argv)
    try:
        policy, policy_sha = load_policy()
        if args.action == "validate-evidence":
            validate_evidence(cm.read_json(args.output), policy_sha,
                              candidate=args.candidate, task_id=args.task_id)
            print("EVIDENCE_OK")
            return 0

        contract, contract_sha = load_contract(str(args.contract))
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
        if not cm.worktree_clean(cwd):
            raise VerifyError("worktree is dirty before verification")
        pre_head, pre_tree = revision(cwd, candidate)
        started_clock = time.monotonic()
        commands = contract["commands"]
        if len(commands) > policy["max_commands"]:
            raise VerifyError("too many commands")
        forbidden = [cwd, root]
        results = [run_one(c, cwd, policy, forbidden) for c in commands]
        if not cm.worktree_clean(cwd):
            raise VerifyError("worktree became dirty during verification")
        post_head, post_tree = revision(cwd, candidate)
        counts = result_counts(results)
        mandatory = [c["id"] for c in commands]
        green = (counts["passed"] == len(results) and counts["passed"] > 0
                 and pre_head == post_head == candidate and pre_tree == post_tree == tree)
        evidence = {
            "schema": SCHEMA,
            "task_id": contract["task_id"],
            "candidate_sha": candidate,
            "tree_sha": tree,
            "pre_head": pre_head, "post_head": post_head,
            "pre_tree": pre_tree, "post_tree": post_tree,
            "pre_clean": True, "post_clean": True,
            "policy_sha256": policy_sha,
            "contract_sha256": contract_sha,
            "resource_limits": {
                "max_commands": policy["max_commands"],
                "max_timeout_seconds": policy["max_timeout_seconds"],
                "max_output_bytes": policy["max_output_bytes"],
            },
            "mandatory_ids": mandatory,
            "worktree": cwd,
            "repository": root,
            "branch": cm.git(cwd, "rev-parse", "--abbrev-ref", "HEAD"),
            "check_classes": {c["id"]: c.get("class", "mandatory") for c in commands},
            "negative_control_ids": [c["id"] for c in commands
                                     if c.get("class") == "conditional"],
            "required_negative_control_ids": list(policy["required_negative_control_ids"]),
            "sandbox_mode": policy["sandbox"]["mode"],
            "process_tree_termination_guaranteed":
                bool(policy["process_tree_termination_guaranteed"]),
            "started_at": now(), "finished_at": now(),
            "verification_seconds": round(time.monotonic() - started_clock, 3),
            "results": results, "summary": counts, "green": green,
        }
        cm.atomic_json_write(str(args.output), evidence)
        print(json.dumps(counts, sort_keys=True))
        if not green:
            return 1
        return 0
    except (cm.LoopError, OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"VERIFY_ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
