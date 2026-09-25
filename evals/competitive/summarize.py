#!/usr/bin/env python3
"""Validate raw competitive runs and build a small plot-ready summary."""

from __future__ import annotations

import json
import math
from pathlib import Path
import statistics

from run_local import pctl


ROOT = Path(__file__).resolve().parent / "results"
SOURCES = ("local-natural.json", "local-keyword.json", "mem0.json", "tencent.json")
PRODUCTS = {"agent-mem", "engram", "projectmem", "mem0", "tencentdb"}


def main() -> None:
    summary = []
    seen = set()
    for filename in SOURCES:
        document = json.loads((ROOT / filename).read_text())
        if document["environment"]["distractors"] != 200:
            raise ValueError(f"{filename}: expected 200 distractors")
        for result in document["results"]:
            name = result["name"]
            profile = result.get("query_profile", document["environment"].get("query_profile"))
            key = (name, profile)
            if key in seen:
                raise ValueError(f"duplicate run: {key}")
            seen.add(key)
            expected_queries = 20 if profile == "natural" else 10
            cases = result["cases"]
            times = result["latency_samples_ms"]
            if result["records"] != 210 or len(cases) != expected_queries:
                raise ValueError(f"{filename}: incomplete corpus or cases for {key}")
            if len(times) != expected_queries * 5 or any(t <= 0 for t in times):
                raise ValueError(f"{filename}: incomplete latency evidence for {key}")
            for case in cases:
                rank = case["rank"]
                returned = case["returned"]
                if rank is not None and rank <= 5 and (
                    len(returned) < rank or returned[rank - 1] != case["expected"]
                ):
                    raise ValueError(f"{filename}: inconsistent rank for {case['query']}")
            recall = sum(c["rank"] is not None and c["rank"] <= 5 for c in cases) / len(cases)
            mrr = sum(1 / c["rank"] for c in cases
                      if c["rank"] is not None and c["rank"] <= 5) / len(cases)
            checks = (("recall_at_5", recall), ("mrr", mrr),
                      ("query_ms_p50", statistics.median(times)),
                      ("query_ms_p95", pctl(times, .95)),
                      ("query_ms_p99", pctl(times, .99)))
            for field, expected in checks:
                if not math.isclose(result[field], expected, rel_tol=1e-9):
                    raise ValueError(f"{filename}: {field} mismatch for {key}")
            summary.append({"name": name, "profile": profile,
                            "records": result["records"], "queries": len(cases),
                            "latency_samples": len(times), "recall_at_5": recall,
                            "mrr_at_5": mrr, "ingest_seconds": result["ingest_seconds"],
                            "query_ms_p50": result["query_ms_p50"],
                            "query_ms_p95": result["query_ms_p95"],
                            "query_ms_p99": result["query_ms_p99"],
                            "peak_rss_kib": result.get("peak_rss_kib"),
                            "stored_bytes": result.get("stored_bytes"),
                            "source": filename, "method": document["method"]})
    required = {(name, profile) for name in PRODUCTS for profile in ("natural", "keyword")}
    if seen != required:
        raise ValueError(f"missing runs: {sorted(required - seen)}")
    (ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    for row in summary:
        print(f"{row['name']:11} {row['profile']:7} recall@5={row['recall_at_5']:.2f} "
              f"mrr@5={row['mrr_at_5']:.3f} p50={row['query_ms_p50']:.3f}ms "
              f"p95={row['query_ms_p95']:.3f}ms")


if __name__ == "__main__":
    main()
