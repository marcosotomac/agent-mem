#!/usr/bin/env python3
"""Run the same fixed retrieval corpus through Mem0 OSS with OpenAI defaults."""

from __future__ import annotations

import json
import os
from pathlib import Path
import random
import re
import statistics
import sys
import tempfile
import time

from run_local import FACTS, KEYWORDS, MARKER, corpus, pctl


def main() -> None:
    from mem0 import Memory

    entries, queries = corpus(int(os.environ.get("BENCH_DISTRACTORS", "200")))
    with tempfile.TemporaryDirectory(prefix="competitive-mem0-") as temp:
        os.environ["HOME"] = temp
        os.environ["MEM0_DIR"] = temp
        memory = Memory.from_config({
            "vector_store": {"provider": "qdrant", "config": {
                "path": str(Path(temp) / "qdrant"), "collection_name": "bench"}},
            "history_db_path": str(Path(temp) / "history.db"),
        })
        start = time.perf_counter()
        for marker, value in entries:
            memory.add(f"{marker}: {value}", user_id="bench", infer=False)
        ingest = time.perf_counter() - start
        results = []
        for profile, profile_queries in (
            ("natural", queries),
            ("keyword", [(query, f"BID{i:04d}") for i, query in enumerate(KEYWORDS)]),
        ):
            shuffled = profile_queries.copy()
            random.Random(42).shuffle(shuffled)
            raw = []
            times = []
            for query, expected in shuffled:
                started = time.perf_counter_ns()
                response = memory.search(query, top_k=5, filters={"user_id": "bench"},
                                         threshold=0)
                elapsed_ms = (time.perf_counter_ns() - started) / 1e6
                times.append(elapsed_ms)
                ranked = [MARKER.search(item.get("memory", "")) for item in response["results"]]
                ranked = [match.group() for match in ranked if match]
                rank = ranked.index(expected) + 1 if expected in ranked else None
                raw.append({"query": query, "expected": expected, "rank": rank,
                            "returned": ranked, "output_bytes": len(json.dumps(response).encode()),
                            "query_ms": elapsed_ms})
            for repeat in range(1, 5):
                extra = profile_queries.copy()
                random.Random(42 + repeat).shuffle(extra)
                for query, _ in extra:
                    started = time.perf_counter_ns()
                    memory.search(query, top_k=5, filters={"user_id": "bench"}, threshold=0)
                    times.append((time.perf_counter_ns() - started) / 1e6)
            results.append({"name": "mem0", "query_profile": profile,
                            "records": len(entries), "queries": len(profile_queries),
                            "recall_at_5": sum(x["rank"] is not None for x in raw) / len(raw),
                            "mrr": sum(1 / x["rank"] for x in raw if x["rank"]) / len(raw),
                            "ingest_seconds": ingest,
                            "query_ms_p50": statistics.median(times),
                            "query_ms_p95": pctl(times, .95),
                            "query_ms_p99": pctl(times, .99),
                            "stored_bytes": sum(p.stat().st_size for p in Path(temp).rglob("*")
                                                if p.is_file()),
                            "peak_rss_kib": int(re.search(
                                r"^VmHWM:\s+(\d+)", Path("/proc/self/status").read_text(),
                                re.MULTILINE).group(1)), "cases": raw,
                            "latency_samples_ms": times})
        print(json.dumps({"method": "Mem0 OSS Python API; OpenAI text-embedding-3-small; "
                                    "infer=False (raw memory, no LLM extraction)",
                          "environment": {"platform": os.uname().sysname,
                                          "machine": os.uname().machine,
                                          "distractors": len(entries) - len(FACTS)},
                          "results": results}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"mem0 benchmark failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        raise
