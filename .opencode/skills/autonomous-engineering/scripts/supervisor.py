#!/usr/bin/env python3
"""Bounded supervisor with protected transitions and authority checks.

Generic `advance` cannot cross review/completion gates: those require real
verifier evidence and an independent APPROVE verdict bound to the exact
candidate commit and tree.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

import checkpoint as ck
import common as cm
import verify as vf

VERDICTS = {"APPROVE", "REPAIR_REQUIRED", "BLOCKED"}
# Generic advance never authorizes a protected gate.
TRANSITIONS = {
    "DISCOVER": {"SELECT"},
    "SELECT": {"CONTRACT", "BLOCKED"},
    "CONTRACT": {"WORKTREE", "BLOCKED"},
    "WORKTREE": {"IMPLEMENT", "BLOCKED"},
    "IMPLEMENT": {"VERIFY", "BLOCKED"},
    "VERIFY": {"IMPLEMENT", "BLOCKED"},
    "REVIEW": {"REPAIR", "BLOCKED"},
    "REPAIR": {"VERIFY", "BLOCKED"},
    "READY": {"BLOCKED"},
    "SAVE_EVIDENCE": {"BLOCKED"},
    "DONE": set(),
    "BLOCKED": set(),
}
MAX_REPAIRS = 2


def load_contract(path: str) -> dict[str, Any]:
    data = cm.read_json(path)
    if data.get("schema") != "autonomous-task/2":
        raise cm.LoopError("wrong task contract schema")
    for key in ("task_id", "issue", "base_sha", "tree_sha", "worktree",
                "branch", "owner", "builder_context"):
        if key not in data:
            raise cm.LoopError(f"contract missing {key}")
    return data


def check_dependencies(contract: dict[str, Any]) -> None:
    for dep in contract.get("dependencies", []):
        if not dep.get("merged"):
            raise cm.LoopError(f"dependency {dep.get('issue')} not merged")


def check_live_revision(cp: dict[str, Any]) -> None:
    wt = str(cp["worktree"])
    if not cm.git_ok(wt, "rev-parse", "--is-inside-work-tree"):
        raise cm.LoopError("worktree missing")
    if cm.git(wt, "rev-parse", "HEAD") != cp["candidate_sha"]:
        raise cm.LoopError("worktree HEAD differs from checkpoint candidate")
    if cm.git(wt, "rev-parse", "HEAD^{tree}") != cp["tree_sha"]:
        raise cm.LoopError("worktree tree differs from checkpoint tree")
    if not cm.worktree_clean(wt):
        raise cm.LoopError("worktree is dirty")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    q = sub.add_parser("discover")
    q.add_argument("--repo", required=True)
    q = sub.add_parser("claim")
    q.add_argument("--repo", required=True)
    q.add_argument("--contract", required=True)
    q = sub.add_parser("status")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q = sub.add_parser("advance")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--to", required=True)
    q = sub.add_parser("register-candidate")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--candidate", required=True)
    q.add_argument("--tree", required=True)
    q = sub.add_parser("submit-review")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--candidate", required=True)
    q.add_argument("--tree", required=True)
    q.add_argument("--evidence", required=True)
    q = sub.add_parser("record-review")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--candidate", required=True)
    q.add_argument("--tree", required=True)
    q.add_argument("--verdict", required=True)
    q.add_argument("--reviewer", required=True)
    q.add_argument("--review-context", required=True)
    q.add_argument("--findings", default="")
    q = sub.add_parser("repair")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--candidate", required=True)
    q.add_argument("--tree", required=True)
    q = sub.add_parser("complete")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    q.add_argument("--evidence", required=True)
    q = sub.add_parser("recover")
    q.add_argument("--repo", required=True)
    q.add_argument("--task-id", required=True)
    sub.add_parser("selftest")
    args = p.parse_args(argv)
    try:
        if args.cmd == "discover":
            root = cm.repo_root(args.repo)
            print(json.dumps({
                "repository": str(root),
                "branch": cm.git(str(root), "rev-parse", "--abbrev-ref", "HEAD"),
                "head_sha": cm.git(str(root), "rev-parse", "HEAD"),
                "tree_sha": cm.git(str(root), "rev-parse", "HEAD^{tree}"),
            }, indent=2, sort_keys=True))
            return 0

        if args.cmd == "selftest":
            skill = Path(__file__).resolve().parent.parent
            return subprocess.run(
                [sys.executable, "-m", "unittest", "discover", "-s",
                 str(skill / "tests"), "-v"], check=False,
            ).returncode

        if args.cmd == "claim":
            contract = load_contract(args.contract)
            cm.validate_sha(contract["base_sha"], "base_sha")
            cm.validate_sha(contract["tree_sha"], "tree_sha")
            check_dependencies(contract)
            with cm.exclusive_lock(ck.checkpoint_dir(args.repo) / f"{contract['task_id']}.lock"):
                path = ck.checkpoint_path(args.repo, contract["task_id"])
                if path.exists():
                    data = ck.read(args.repo, contract["task_id"])
                    if data["owner"] != contract["owner"]:
                        raise cm.LoopError("task already claimed by a different owner")
                    print("ALREADY_DONE" if data["state"] == "DONE" else "RECLAIMED")
                else:
                    ck.write(args.repo, ck.blank(
                        contract["task_id"], contract["issue"], args.repo,
                        contract["base_sha"], contract["tree_sha"],
                        contract["worktree"], contract["branch"], contract["owner"],
                        contract["builder_context"], cm.sha256_file(args.contract),
                    ))
                    print("CLAIMED")
                return 0

        if args.cmd == "recover":
            data = ck.read(args.repo, args.task_id)
            ck.validate(data, verify_repo=True)
            if data["state"] == "DONE":
                print("NO_OP_ALREADY_DONE")
                return 0
            wt = Path(data["worktree"])
            if not wt.is_dir() or not cm.git_ok(str(wt), "rev-parse", "--is-inside-work-tree"):
                raise cm.LoopError("worktree missing or not a git worktree")
            check_live_revision(data)
            print("RESUME:" + str(data["next_safe_action"]))
            return 0

        with cm.exclusive_lock(ck.checkpoint_dir(args.repo) / f"{args.task_id}.lock"):
            data = ck.read(args.repo, args.task_id)
            ck.validate(data)

            if args.cmd == "status":
                print(json.dumps(data, indent=2, sort_keys=True))
                return 0

            if args.cmd == "advance":
                if args.to not in TRANSITIONS.get(data["state"], set()):
                    raise cm.LoopError(f"illegal transition {data['state']} -> {args.to}")
                data["state"] = args.to
                data["last_successful_step"] = f"advance:{args.to}"
                data["next_safe_action"] = f"continue-from-{args.to.lower()}"
                ck.write(args.repo, data)
                print(f"STATE:{args.to}")
                return 0

            if args.cmd == "register-candidate":
                cm.validate_sha(args.candidate, "candidate")
                cm.validate_sha(args.tree, "tree")
                if data["state"] not in {"IMPLEMENT", "VERIFY", "REPAIR"}:
                    raise cm.LoopError("candidate registration not allowed in this state")
                root = str(cm.repo_root(args.repo))
                cm.git(root, "cat-file", "-e", f"{args.candidate}^{{commit}}")
                cm.git(root, "cat-file", "-e", f"{args.tree}^{{tree}}")
                data["candidate_sha"] = args.candidate
                data["tree_sha"] = args.tree
                data["next_safe_action"] = "run-verification"
                ck.write(args.repo, data)
                print("CANDIDATE_REGISTERED")
                return 0

            if args.cmd == "submit-review":
                cm.validate_sha(args.candidate, "candidate")
                cm.validate_sha(args.tree, "tree")
                if data["state"] != "VERIFY":
                    raise cm.LoopError("review requires state VERIFY")
                if args.candidate != data["candidate_sha"] or args.tree != data["tree_sha"]:
                    raise cm.LoopError("candidate or tree differs from checkpoint")
                policy_sha = vf.load_policy()[1]
                evidence = vf.validate_evidence(cm.read_json(args.evidence), policy_sha,
                                                candidate=args.candidate, tree=args.tree,
                                                task_id=args.task_id)
                if evidence["contract_sha256"] != data["contract_sha256"]:
                    raise cm.LoopError("evidence is bound to a different contract")
                check_live_revision(data)
                data["state"] = "REVIEW"
                data["evidence_sha256"] = cm.sha256_file(args.evidence)
                data["review_result"] = {"verdict": "NOT_REVIEWED", "candidate_sha": None,
                                         "reviewer": None, "review_context": None}
                data["next_safe_action"] = "dispatch-independent-reviewer"
                ck.write(args.repo, data)
                print("READY_FOR_REVIEW")
                return 0

            if args.cmd == "record-review":
                if args.verdict not in VERDICTS:
                    raise cm.LoopError(f"verdict must be one of {sorted(VERDICTS)}")
                if data["state"] != "REVIEW":
                    raise cm.LoopError("no review is open")
                if args.candidate != data["candidate_sha"] or args.tree != data["tree_sha"]:
                    raise cm.LoopError("stale verdict: candidate or tree differs")
                check_live_revision(data)
                if not data["evidence_sha256"]:
                    raise cm.LoopError("no verified evidence bound to this task")
                if args.reviewer == data["owner"]:
                    raise cm.LoopError("builder cannot review its own work")
                if args.review_context == data["builder_context"]:
                    raise cm.LoopError("review must run in a separate context")
                data["review_result"] = {
                    "verdict": args.verdict, "candidate_sha": args.candidate,
                    "tree_sha": args.tree, "reviewer": args.reviewer,
                    "review_context": args.review_context, "findings": args.findings,
                }
                if args.verdict == "APPROVE":
                    data["state"] = "READY"
                    data["next_safe_action"] = "complete-with-bound-evidence"
                elif args.verdict == "BLOCKED":
                    data["state"] = "BLOCKED"
                    data["blockers"]["authority"] = args.findings or "unavailable authority"
                else:
                    if data["attempt_count"] >= MAX_REPAIRS:
                        data["state"] = "BLOCKED"
                        data["blockers"]["budget"] = "repair budget exhausted"
                    else:
                        data["attempt_count"] += 1
                        data["state"] = "REPAIR"
                        data["next_safe_action"] = "apply-concrete-repair"
                ck.write(args.repo, data)
                print(f"VERDICT:{args.verdict} STATE:{data['state']}")
                return 0

            if args.cmd == "repair":
                cm.validate_sha(args.candidate, "candidate")
                cm.validate_sha(args.tree, "tree")
                if data["state"] != "REPAIR":
                    raise cm.LoopError("repair allowed only after REPAIR_REQUIRED")
                if data["attempt_count"] > MAX_REPAIRS:
                    raise cm.LoopError("repair budget exhausted")
                root = str(cm.repo_root(args.repo))
                cm.git(root, "cat-file", "-e", f"{args.candidate}^{{commit}}")
                data["candidate_sha"] = args.candidate
                data["tree_sha"] = args.tree
                data["state"] = "VERIFY"
                data["test_results"] = {}
                data["evidence_sha256"] = None
                data["review_result"] = {"verdict": "NOT_REVIEWED", "candidate_sha": None,
                                         "reviewer": None, "review_context": None}
                data["next_safe_action"] = "rerun-verification"
                ck.write(args.repo, data)
                print("REPAIR_REGISTERED")
                return 0

            if args.cmd == "complete":
                if data["state"] != "READY":
                    raise cm.LoopError("completion requires READY after independent APPROVE")
                verdict = data["review_result"]
                if verdict.get("verdict") != "APPROVE":
                    raise cm.LoopError("no valid APPROVE verdict")
                if verdict.get("candidate_sha") != data["candidate_sha"] or verdict.get("tree_sha") != data["tree_sha"]:
                    raise cm.LoopError("approval bound to a different candidate")
                if not data["evidence_sha256"] or cm.sha256_file(args.evidence) != data["evidence_sha256"]:
                    raise cm.LoopError("evidence package differs from the approved one")
                policy_sha = vf.load_policy()[1]
                vf.validate_evidence(cm.read_json(args.evidence), policy_sha,
                                     candidate=data["candidate_sha"], tree=data["tree_sha"],
                                     task_id=args.task_id)
                check_live_revision(data)
                data["state"] = "DONE"
                data["last_successful_step"] = "complete:bound-evidence"
                data["next_safe_action"] = "select-next-eligible-issue"
                ck.write(args.repo, data)
                print("STATE:DONE")
                return 0

        raise cm.LoopError("unhandled command")
    except (cm.LoopError, OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"GUARD_ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
