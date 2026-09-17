#!/usr/bin/env python3
"""Summarize Harbor evidence without guessing missing metrics."""

from __future__ import annotations

from collections import Counter
import csv
from datetime import datetime
import hashlib
import json
from pathlib import Path
import re
import sys


JOBS = Path(__file__).resolve().parent / "jobs"


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


def failure_signature(trial: Path, reward: float | None) -> str:
    if reward is None or reward > 0:
        return ""
    pieces = []
    for pattern in ("verifier/*stderr*", "verifier/*stdout*", "verifier/*.log"):
        for path in sorted(trial.glob(pattern)):
            text = path.read_text(encoding="utf-8", errors="replace")[-8000:]
            pieces.append(text)
    normalized = re.sub(r"0x[0-9a-fA-F]+|\d+(?:\.\d+)?s", "<volatile>", "\n".join(pieces))
    return hashlib.sha256(normalized.encode()).hexdigest()[:16] if normalized else "missing-evidence"


def duration_seconds(timing: dict | None):
    if not timing or not timing.get("started_at") or not timing.get("finished_at"):
        return None
    started = datetime.fromisoformat(timing["started_at"].replace("Z", "+00:00"))
    finished = datetime.fromisoformat(timing["finished_at"].replace("Z", "+00:00"))
    return (finished - started).total_seconds()


def main() -> int:
    rows = []
    for job_result_path in sorted(JOBS.glob("*/result.json")):
        job = json.loads(job_result_path.read_text())
        job_config = json.loads((job_result_path.parent / "config.json").read_text())
        job_name = job_result_path.parent.name
        parts = job_config.get("job_name", job_name).split("__")
        baseline = parts[1] if len(parts) > 2 else "unknown"
        model = job_config.get("agents", [{}])[0].get("model_name", "unknown")
        for trial_result_path in sorted(job_result_path.parent.glob("*/result.json")):
            trial = json.loads(trial_result_path.read_text())
            reward = number(trial, ("verifier_result", "rewards", "reward"), ("reward",))
            input_tokens = number(trial, ("agent_result", "n_input_tokens"))
            output_tokens = number(trial, ("agent_result", "n_output_tokens"))
            latency = duration_seconds(trial.get("agent_execution"))
            infrastructure_error = int(trial.get("exception_info") is not None)
            rows.append({
                "job": job_name,
                "baseline": baseline,
                "model": model,
                "task": trial.get("task_name", "unknown"),
                "trial": trial_result_path.parent.name,
                "success": int(reward > 0) if reward is not None and not infrastructure_error else "",
                "reward": reward if reward is not None else "",
                "input_tokens": input_tokens if input_tokens is not None else "",
                "output_tokens": output_tokens if output_tokens is not None else "",
                "latency_seconds": latency if latency is not None else "",
                "infrastructure_error": infrastructure_error,
                "failure_signature": failure_signature(
                    trial_result_path.parent,
                    None if infrastructure_error else reward,
                ),
            })
    if not rows:
        raise SystemExit(f"no Harbor trial evidence found under {JOBS}")
    signatures = Counter(
        (row["baseline"], row["model"], row["task"], row["failure_signature"])
        for row in rows
        if row["failure_signature"]
    )
    for row in rows:
        signature = row["failure_signature"]
        signature_key = (row["baseline"], row["model"], row["task"], signature)
        row["repeated_error"] = int(bool(signature) and signatures[signature_key] > 1)
    writer = csv.DictWriter(sys.stdout, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(rows)
    missing = [key for key in ("input_tokens", "output_tokens", "latency_seconds") if all(row[key] == "" for row in rows)]
    if missing:
        print(f"missing required Harbor metrics: {', '.join(missing)}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
