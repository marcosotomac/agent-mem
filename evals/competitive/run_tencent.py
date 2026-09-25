#!/usr/bin/env python3
"""Run TencentDB Agent Memory's L0 API on the shared retrieval corpus.

This measures raw conversation storage/search with extraction disabled, matching
Mem0's infer=False mode. L1 extraction and agent task success are separate jobs.
"""

from __future__ import annotations

import http.client
import json
import os
import random
import statistics
import time
import uuid

from run_local import FACTS, KEYWORDS, MARKER, corpus, pctl


def main() -> None:
    entries, queries = corpus(int(os.environ.get("BENCH_DISTRACTORS", "200")))
    host = os.environ.get("TENCENT_BENCH_HOST", "localhost")
    port = int(os.environ.get("TENCENT_BENCH_PORT", "18420"))
    session = "competitive-" + uuid.uuid4().hex
    scope = {"team_id": "competitive", "agent_id": "competitive",
             "user_id": "competitive", "session_id": session}
    client = http.client.HTTPConnection(host, port, timeout=30)
    headers = {"Content-Type": "application/json", "Authorization": "Bearer benchmark-local",
               "x-tdai-service-id": "default"}

    def request(path: str, payload: dict) -> dict:
        client.request("POST", path, body=json.dumps(payload), headers=headers)
        response = client.getresponse()
        data = json.loads(response.read())
        if response.status != 200 or data.get("code") != 0:
            raise RuntimeError(f"{path}: HTTP {response.status}, code={data.get('code')}, "
                               f"message={data.get('message')}")
        return data["data"]

    start = time.perf_counter()
    for marker, value in entries:
        request("/v3/conversation/add", {**scope, "messages": [
            {"role": "user", "content": f"{marker}: {value}"}]})
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
            data = request("/v3/conversation/search", {**scope, "query": query, "limit": 5})
            elapsed_ms = (time.perf_counter_ns() - started) / 1e6
            times.append(elapsed_ms)
            ranked = [MARKER.search(item.get("content", "")) for item in data["messages"]]
            ranked = [match.group() for match in ranked if match]
            rank = ranked.index(expected) + 1 if expected in ranked else None
            raw.append({"query": query, "expected": expected, "rank": rank,
                        "returned": ranked, "output_bytes": len(json.dumps(data).encode()),
                        "query_ms": elapsed_ms})
        for repeat in range(1, 5):
            extra = profile_queries.copy()
            random.Random(42 + repeat).shuffle(extra)
            for query, _ in extra:
                started = time.perf_counter_ns()
                request("/v3/conversation/search", {**scope, "query": query, "limit": 5})
                times.append((time.perf_counter_ns() - started) / 1e6)
        results.append({"name": "tencentdb", "query_profile": profile,
                        "records": len(entries), "queries": len(profile_queries),
                        "recall_at_5": sum(x["rank"] is not None for x in raw) / len(raw),
                        "mrr": sum(1 / x["rank"] for x in raw if x["rank"]) / len(raw),
                        "ingest_seconds": ingest,
                        "query_ms_p50": statistics.median(times),
                        "query_ms_p95": pctl(times, .95),
                        "query_ms_p99": pctl(times, .99), "cases": raw,
                        "latency_samples_ms": times})
    client.close()
    print(json.dumps({"method": "TencentDB MemoryCore v3 HTTP L0 conversation add/search; "
                                "extraction disabled; persistent HTTP connection",
                      "environment": {"platform": os.uname().sysname,
                                      "machine": os.uname().machine,
                                      "distractors": len(entries) - len(FACTS)},
                      "results": results}, indent=2))


if __name__ == "__main__":
    main()
