#!/usr/bin/env python3
"""Local, portable benchmark recorder for the autonomous engineering loop.

Measures only what local evidence actually records. Never invents a baseline or
a cost figure. When no comparable baseline exists the result is INCONCLUSIVE.
"""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

import common as cm


def _elapsed(evidence: dict[str, Any]) -> float:
    try:
        a = datetime.fromisoformat(evidence["started_at"])
        b = datetime.fromisoformat(evidence["finished_at"])
        return max(0.0, (b - a).total_seconds())
    except (KeyError, ValueError):
        return 0.0


def analyze(evidence_files: list[Path], baseline_seconds: float | None) -> dict[str, Any]:
    tasks: list[dict[str, Any]] = []
    for path in evidence_files:
        try:
            ev = cm.read_json(path)
        except (OSError, ValueError, cm.LoopError):
            continue
        if ev.get("schema") != "autonomous-evidence/3":
            continue
        tasks.append({
            "task_id": ev.get("task_id"),
            "candidate_sha": ev.get("candidate_sha"),
            "verification_seconds": round(ev.get("verification_seconds", _elapsed(ev)), 3),
            "passed": ev.get("summary", {}).get("passed", 0),
            "green": ev.get("green") is True,
        })
    accepted = sum(1 for t in tasks if t["green"])
    total_seconds = sum(t["verification_seconds"] for t in tasks)
    hours = total_seconds / 3600 if total_seconds else 0.0
    result: dict[str, Any] = {
        "schema": "autonomous-benchmark/1",
        "tasks_observed": len(tasks),
        "accepted_tasks": accepted,
        "failed_tasks": len(tasks) - accepted,
        "total_verification_seconds": round(total_seconds, 3),
        "accepted_tasks_per_verification_hour": round(accepted / hours, 3) if hours else None,
        "baseline_seconds": baseline_seconds,
        "measured_speedup": None,
        "throughput_status": "INCONCLUSIVE",
        "model_calls_measured": False,
        "model_cost_measured": False,
        "note": "Verification time only. Planning, implementation, review, repair, "
                "model calls and cost are not measured by this local recorder.",
    }
    if baseline_seconds and total_seconds:
        result["measured_speedup"] = round(baseline_seconds / total_seconds, 3)
        result["throughput_status"] = "MEASURED_VERIFICATION_ONLY"
    return result


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["run"])
    p.add_argument("--evidence-dir", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--baseline-seconds", type=float)
    args = p.parse_args(argv)
    try:
        files = sorted(Path(args.evidence_dir).glob("*.json"))
        result = analyze(files, args.baseline_seconds)
        cm.atomic_json_write(args.output, result)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (OSError, ValueError, cm.LoopError) as exc:
        print(f"BENCHMARK_ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
