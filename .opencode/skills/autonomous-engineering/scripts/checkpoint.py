#!/usr/bin/env python3
"""Schema-closed durable checkpoint store for the autonomous loop.

Only declared fields are accepted, so private content cannot leak into state.
Every checkpoint records both commit and tree identity of the candidate.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import common as cm

SCHEMA = "autonomous-checkpoint/2"
STATES = {
    "DISCOVER", "SELECT", "CONTRACT", "WORKTREE", "IMPLEMENT", "VERIFY",
    "REVIEW", "REPAIR", "READY", "SAVE_EVIDENCE", "DONE", "BLOCKED",
}
FIELDS = {
    "schema", "task_id", "issue", "repository", "base_sha", "candidate_sha",
    "tree_sha", "worktree", "branch", "owner", "builder_context", "state",
    "attempt_count", "last_successful_step", "test_results", "review_result", "blockers",
    "next_safe_action",
}
REQUIRED = set(FIELDS)


def checkpoint_dir(repo: str) -> Path:
    return cm.repo_root(repo) / ".engineering" / "autonomous" / "checkpoints"


def checkpoint_path(repo: str, task_id: str) -> Path:
    if not isinstance(task_id, str) or not cm.TASK_ID_RE.fullmatch(task_id):
        raise cm.LoopError("task_id must contain only [A-Za-z0-9._-]")
    return checkpoint_dir(repo) / f"{task_id}.json"


def validate(data: Any, *, verify_repo: bool = False) -> None:
    if not isinstance(data, dict):
        raise cm.LoopError("checkpoint must be a JSON object")
    if set(data) - FIELDS:
        raise cm.LoopError(f"unknown fields: {sorted(set(data) - FIELDS)}")
    if set(data) != REQUIRED:
        raise cm.LoopError(f"missing fields: {sorted(REQUIRED - set(data))}")
    if data["schema"] != SCHEMA:
        raise cm.LoopError("wrong checkpoint schema")
    if not isinstance(data["task_id"], str) or not data["task_id"]:
        raise cm.LoopError("task_id must be a non-empty string")
    if not isinstance(data["issue"], int) or data["issue"] < 0:
        raise cm.LoopError("issue must be a non-negative integer (0 = tooling task)")
    for key in ("base_sha", "candidate_sha"):
        cm.validate_sha(data[key], key, allow_none=True)
    cm.validate_sha(data["tree_sha"], "tree_sha", allow_none=True)
    if data["state"] not in STATES:
        raise cm.LoopError(f"unknown state: {data['state']}")
    if not isinstance(data["attempt_count"], int) or data["attempt_count"] < 0:
        raise cm.LoopError("attempt_count must be a non-negative integer")
    for key in ("test_results", "review_result", "blockers"):
        if not isinstance(data[key], dict):
            raise cm.LoopError(f"{key} must be an object")
    if not isinstance(data["worktree"], str) or not data["worktree"]:
        raise cm.LoopError("worktree must be a non-empty string")
    if not isinstance(data["branch"], str) or not data["branch"]:
        raise cm.LoopError("branch must be a non-empty string")
    if not isinstance(data["owner"], str) or not data["owner"]:
        raise cm.LoopError("owner must be a non-empty string")
    if not isinstance(data["builder_context"], str) or not data["builder_context"]:
        raise cm.LoopError("builder_context must be a non-empty string")
    if verify_repo:
        root = str(cm.repo_root(str(data["repository"])))
        for key in ("base_sha", "candidate_sha"):
            sha = data[key]
            if sha and not cm.git_ok(root, "cat-file", "-e", f"{sha}^{{commit}}"):
                raise cm.LoopError(f"{key} does not resolve in repository: {sha}")
        tree = data["tree_sha"]
        if tree and not cm.git_ok(root, "cat-file", "-e", f"{tree}^{{tree}}"):
            raise cm.LoopError(f"tree_sha does not resolve in repository: {tree}")


def read(repo: str, task_id: str) -> dict[str, Any]:
    path = checkpoint_path(repo, task_id)
    if not path.is_file():
        raise cm.LoopError(f"no checkpoint: {path}")
    data = cm.read_json(path)
    validate(data)
    return data


def write(repo: str, data: dict[str, Any]) -> Path:
    validate(data)
    return cm.atomic_json_write(checkpoint_path(repo, data["task_id"]), data)


def blank(task_id: str, issue: int, repository: str, base_sha: str,
          tree_sha: str, worktree: str, branch: str, owner: str,
          builder_context: str) -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "task_id": task_id,
        "issue": issue,
        "repository": str(cm.repo_root(repository)),
        "base_sha": base_sha,
        "candidate_sha": base_sha,
        "tree_sha": tree_sha,
        "worktree": worktree,
        "branch": branch,
        "owner": owner,
        "builder_context": builder_context,
        "state": "DISCOVER",
        "attempt_count": 0,
        "last_successful_step": "checkpoint-init",
        "test_results": {},
        "review_result": {"verdict": "NOT_REVIEWED", "candidate_sha": None,
                          "reviewer": None, "review_context": None},
        "blockers": {},
        "next_safe_action": "select-eligible-issue",
    }


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    q = sub.add_parser("init")
    for name in ("repo", "task-id", "base-sha", "tree-sha", "worktree", "branch",
                 "owner", "builder-context"):
        q.add_argument("--" + name, required=True)
    q.add_argument("--issue", type=int, required=True)
    q = sub.add_parser("get")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q = sub.add_parser("patch")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--file", required=True)
    q = sub.add_parser("list")
    q.add_argument("--repo", required=True)
    q = sub.add_parser("validate")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--verify-repo", action="store_true")
    args = p.parse_args(argv)
    try:
        if args.cmd == "init":
            data = blank(args.task_id, args.issue, args.repo, args.base_sha,
                         args.tree_sha, args.worktree, args.branch, args.owner,
                         args.builder_context)
            print(write(args.repo, data))
        elif args.cmd == "get":
            import json
            print(json.dumps(read(args.repo, args.task_id), indent=2, sort_keys=True))
        elif args.cmd == "patch":
            data = read(args.repo, args.task_id)
            patch = cm.read_json(args.file)
            if set(patch) - FIELDS:
                raise cm.LoopError(f"patch contains unknown fields: {sorted(set(patch) - FIELDS)}")
            data.update(patch)
            print(write(args.repo, data))
        elif args.cmd == "list":
            for path in sorted(checkpoint_dir(args.repo).glob("*.json")):
                print(path.stem)
        elif args.cmd == "validate":
            validate(read(args.repo, args.task_id), verify_repo=args.verify_repo)
            print("CHECKPOINT_OK")
        return 0
    except (cm.LoopError, OSError, ValueError) as exc:
        print(f"CHECKPOINT_ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
