#!/usr/bin/env python3
"""Aggregate one immutable Harbor campaign and enforce its evidence contract."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import sys
import tomllib


EVALS = Path(__file__).resolve().parent
JOBS = EVALS / "jobs"
REQUIRED_MEMORY_TOOLS = {
    "agent-mem": {"mem_find"},
    "projectmem": {"get_summary", "precheck_file"},
    "official-mcp-memory": {"search_nodes"},
}


def number(mapping: dict, *paths: tuple[str, ...]):
    for path in paths:
        value = mapping
        for key in path:
            if not isinstance(value, dict) or key not in value:
                break
            value = value[key]
        else:
            if isinstance(value, (int, float)):
                return value
    return None


def duration_seconds(timing: dict | None):
    if not timing or not timing.get("started_at") or not timing.get("finished_at"):
        return None
    started = datetime.fromisoformat(timing["started_at"].replace("Z", "+00:00"))
    finished = datetime.fromisoformat(timing["finished_at"].replace("Z", "+00:00"))
    return (finished - started).total_seconds()


def failure_signature(trial_dir: Path, reward: float | None) -> str:
    if reward is None or reward > 0:
        return ""
    pieces = []
    for pattern in ("verifier/*stderr*", "verifier/*stdout*", "verifier/*.log"):
        for path in sorted(trial_dir.glob(pattern)):
            pieces.append(path.read_text(encoding="utf-8", errors="replace")[-8000:])
    normalized = re.sub(r"0x[0-9a-fA-F]+|\d+(?:\.\d+)?s", "<volatile>", "\n".join(pieces))
    return hashlib.sha256(normalized.encode()).hexdigest()[:16] if normalized else "missing-evidence"


def memory_tool_evidence(trial_dir: Path, baseline: str) -> tuple[list[str], bool | None]:
    required = REQUIRED_MEMORY_TOOLS.get(baseline)
    if required is None:
        return [], None
    trajectory_path = trial_dir / "agent" / "trajectory.json"
    try:
        trajectory = json.loads(trajectory_path.read_text())
    except (OSError, UnicodeError, json.JSONDecodeError):
        return [], False
    observed = set()
    for step in trajectory.get("steps", []):
        results = {
            result.get("source_call_id"): str(result.get("content", ""))
            for result in (step.get("observation", {}).get("results") or [])
            if isinstance(result, dict)
        }
        for call in step.get("tool_calls") or []:
            if not isinstance(call, dict):
                continue
            content = results.get(call.get("tool_call_id"), "")
            lowered = content.lower()
            if content and "error:" not in lowered and "no memories found" not in lowered:
                observed.add(call.get("function_name", "").rsplit("__", 1)[-1])
    matched = sorted(required & observed)
    return matched, required <= observed


def percentile(values: list[float], percentage: int):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * percentage / 100) - 1)]


def collect(campaign: str) -> list[dict]:
    rows = []
    suffix = f"__{campaign}"
    for job_dir in sorted(path for path in JOBS.iterdir() if path.is_dir() and path.name.endswith(suffix)):
        config_path = job_dir / "config.json"
        if not config_path.exists():
            continue
        config = json.loads(config_path.read_text())
        parts = config["job_name"].split("__")
        baseline = parts[1]
        model = config.get("agents", [{}])[0].get("model_name", "unknown")
        for result_path in sorted(job_dir.glob("*/result.json")):
            trial = json.loads(result_path.read_text())
            infrastructure_error = trial.get("exception_info") is not None
            reward = number(trial, ("verifier_result", "rewards", "reward"), ("reward",))
            input_tokens = number(trial, ("agent_result", "n_input_tokens"))
            output_tokens = number(trial, ("agent_result", "n_output_tokens"))
            memory_tools, memory_tool_used = memory_tool_evidence(result_path.parent, baseline)
            rows.append({
                "baseline": baseline,
                "model": model,
                "task": trial.get("task_name", "unknown"),
                "trial": result_path.parent.name,
                "infrastructure_error": infrastructure_error,
                "success": None if infrastructure_error or reward is None else reward > 0,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": None if input_tokens is None or output_tokens is None else input_tokens + output_tokens,
                "latency_seconds": duration_seconds(trial.get("agent_execution")),
                "failure_signature": failure_signature(result_path.parent, None if infrastructure_error else reward),
                "memory_tools": memory_tools,
                "memory_tool_used": memory_tool_used,
            })
    return rows


def aggregate(rows: list[dict]) -> list[dict]:
    repeated = Counter(
        (row["baseline"], row["model"], row["task"], row["failure_signature"])
        for row in rows if row["failure_signature"]
    )
    groups = defaultdict(list)
    for row in rows:
        groups[(row["baseline"], row["model"], row["task"])].append(row)
    result = []
    for key, group in sorted(groups.items()):
        valid = [row for row in group if not row["infrastructure_error"]]
        metrics = {}
        for field in ("input_tokens", "output_tokens", "total_tokens", "latency_seconds"):
            values = [row[field] for row in valid if row[field] is not None]
            metrics[f"{field}_p50"] = statistics.median(values) if values else None
            metrics[f"{field}_p95"] = percentile(values, 95)
        result.append({
            "baseline": key[0], "model": key[1], "task": key[2],
            "trials": len(group),
            "valid_trials": len(valid),
            "infrastructure_errors": len(group) - len(valid),
            "successes": sum(row["success"] is True for row in valid),
            "success_rate": (sum(row["success"] is True for row in valid) / len(valid)) if valid else None,
            "repeated_error_rate": (
                sum(repeated[(row["baseline"], row["model"], row["task"], row["failure_signature"])] > 1
                    for row in valid if row["failure_signature"]) / len(valid)
            ) if valid else None,
            "memory_tool_rate": (
                sum(row["memory_tool_used"] is True for row in valid) / len(valid)
                if key[0] in REQUIRED_MEMORY_TOOLS and valid else None
            ),
            **metrics,
        })
    return result


def evidence_gate(rows: list[dict], matrix: dict) -> list[str]:
    errors = []
    expected = len(matrix["baselines"]) * len(matrix["models"]) * len(matrix["task_globs"]) * matrix["attempts"]
    if len(rows) != expected:
        errors.append(f"expected {expected} trials, found {len(rows)}")
    infrastructure = sum(row["infrastructure_error"] for row in rows)
    if infrastructure:
        errors.append(f"found {infrastructure} infrastructure errors")
    missing = [row["trial"] for row in rows if not row["infrastructure_error"] and (
        row["success"] is None or row["total_tokens"] is None or row["latency_seconds"] is None
    )]
    if missing:
        errors.append(f"{len(missing)} valid trials lack success, token, or latency evidence")
    missing_memory = [
        row["trial"] for row in rows
        if not row["infrastructure_error"]
        and row["baseline"] in REQUIRED_MEMORY_TOOLS
        and row["memory_tool_used"] is not True
    ]
    if missing_memory:
        errors.append(f"{len(missing_memory)} valid memory trials lack required MCP tool evidence")

    cells = defaultdict(list)
    for row in rows:
        if not row["infrastructure_error"]:
            cells[(row["baseline"], row["model"], row["task"])].append(row)
    native_tokens = 0
    agent_tokens = 0
    comparisons = 0
    for model in matrix["models"]:
        tasks = sorted({row["task"] for row in rows if row["model"] == model})
        for task in tasks:
            native = cells[("native", model, task)]
            agent = cells[("agent-mem", model, task)]
            if len(native) != matrix["attempts"] or len(agent) != matrix["attempts"]:
                errors.append(f"incomplete native/agent-mem cell for {model} {task}")
                continue
            if sum(row["success"] is True for row in agent) < sum(row["success"] is True for row in native):
                errors.append(f"agent-mem observed success is below native for {model} {task}")
            if any(row["total_tokens"] is None for row in [*native, *agent]):
                continue
            native_tokens += sum(row["total_tokens"] for row in native)
            agent_tokens += sum(row["total_tokens"] for row in agent)
            comparisons += 1
    if comparisons and agent_tokens >= native_tokens:
        errors.append(f"no aggregate token saving: agent-mem={agent_tokens}, native={native_tokens}")
    return errors


def markdown(campaign: str, summary: list[dict], gate_errors: list[str]) -> str:
    lines = [
        f"# End-to-end benchmark: {campaign}", "",
        "This report contains observed results; three repetitions per cell do not establish statistical superiority.", "",
        f"Evidence gate: **{'PASS' if not gate_errors else 'FAIL'}**", "",
    ]
    if gate_errors:
        lines.extend(f"- {error}" for error in gate_errors)
        lines.append("")
    lines.extend([
        "| Baseline | Model | Task | Success | Memory tool | Input tokens p50/p95 | Total tokens p50/p95 | Latency p50/p95 | Infra | Repeated errors |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ])
    for row in summary:
        def pair(prefix: str) -> str:
            p50, p95 = row[f"{prefix}_p50"], row[f"{prefix}_p95"]
            return "n/a" if p50 is None else f"{p50:.1f}/{p95:.1f}"
        success = "n/a" if row["success_rate"] is None else f"{row['successes']}/{row['valid_trials']} ({row['success_rate']:.0%})"
        repeated = "n/a" if row["repeated_error_rate"] is None else f"{row['repeated_error_rate']:.0%}"
        memory = "n/a" if row["memory_tool_rate"] is None else f"{row['memory_tool_rate']:.0%}"
        lines.append(
            f"| {row['baseline']} | {row['model']} | {row['task']} | {success} | {memory} | "
            f"{pair('input_tokens')} | {pair('total_tokens')} | {pair('latency_seconds')} | "
            f"{row['infrastructure_errors']} | {repeated} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--campaign", required=True)
    parser.add_argument("--output-dir", type=Path, default=EVALS / "published")
    parser.add_argument("--gate", action="store_true", help="return nonzero unless the evidence contract passes")
    args = parser.parse_args()
    rows = collect(args.campaign)
    if not rows:
        raise SystemExit(f"no evidence for campaign {args.campaign!r}")
    with (EVALS / "matrix.toml").open("rb") as handle:
        matrix = tomllib.load(handle)
    summary = aggregate(rows)
    gate_errors = evidence_gate(rows, matrix)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    payload = {"campaign": args.campaign, "trials": rows, "summary": summary, "gate_errors": gate_errors}
    (args.output_dir / f"{args.campaign}.json").write_text(json.dumps(payload, indent=2) + "\n")
    report = markdown(args.campaign, summary, gate_errors)
    (args.output_dir / f"{args.campaign}.md").write_text(report)
    print(report, end="")
    return 2 if args.gate and gate_errors else 0


if __name__ == "__main__":
    sys.exit(main())
