#!/usr/bin/env python3
"""Measure real MCP servers in one isolated Linux container.

The corpus has independently declared answers. It is a controlled retrieval
test, not a claim about agent task completion or automatic memory extraction.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import random
import re
import select
import statistics
import subprocess
import sys
import tempfile
import time


FACTS = [
    ("Validate JWT issuer and audience before accepting a token to prevent cross tenant access", ["JWT issuer audience cross tenant", "stop forged tenant token with issuer validation"]),
    ("Compare HMAC signatures and API tokens in constant time to block timing side channels", ["constant time HMAC signature comparison", "prevent token timing leak constant comparison"]),
    ("Start SQLite write transactions with BEGIN IMMEDIATE to prevent busy lock upgrade deadlocks", ["SQLite BEGIN IMMEDIATE writer deadlocks", "avoid lock upgrade failure with immediate transaction"]),
    ("Require an Idempotency Key on every mutating payment request so retries cannot duplicate charges", ["duplicate payment retry idempotency key", "prevent charging twice using payment idempotency"]),
    ("Use a single flight guard around cache misses to stop a thundering herd and cache stampede", ["single flight cache stampede herd", "collapse concurrent cache recomputation single flight"]),
    ("Persist domain events in a transactional outbox and publish after commit to avoid lost messages", ["lost event publish transactional outbox commit", "message disappears after commit use an outbox"]),
    ("Use expand and contract database migrations so mixed application versions remain compatible during rolling deploys", ["rolling deploy expand contract database migration", "database change compatible across old and new versions expand"]),
    ("Propagate trace and correlation identifiers across HTTP and queue boundaries", ["trace correlation identifiers queue HTTP", "follow request across worker logs correlation id"]),
    ("Redact access tokens passwords and personal data before structured logging", ["redact passwords tokens personal data logs", "keep secrets and PII out of logs with token redaction"]),
    ("Use cursor pagination with a stable tiebreaker instead of offset for mutable datasets", ["cursor pagination stable tiebreaker mutable dataset", "avoid duplicate rows while paging changing data with cursor"]),
]
MARKER = re.compile(r"BID\d{4}")
TITLES = [
    "gotcha/auth/issuer", "gotcha/auth/constant-time", "gotcha/db/writer-lock",
    "decision/payments/idempotency", "pattern/cache/single-flight",
    "pattern/queue/outbox", "pattern/deploy/expand-contract",
    "rule/observability/correlation", "rule/privacy/log-redaction",
    "decision/api/cursor-pagination",
]
KEYWORDS = ["issuer", "HMAC", "IMMEDIATE", "idempotency", "stampede",
            "outbox", "migrations", "correlation", "redact", "pagination"]


def corpus(distractors: int) -> tuple[list[tuple[str, str]], list[tuple[str, str]]]:
    entries = [(f"{TITLES[i]}/BID{i:04d}", value)
               for i, (value, _) in enumerate(FACTS)]
    for i in range(distractors):
        entries.append((f"background/service-{i:04d}/BID{i + 1000:04d}",
            f"Service {i} handles HTTP requests, database records, cache entries, tokens, "
            "queue messages, retries, logging, and rolling deployments"))
    queries = [(query, f"BID{i:04d}") for i, (_, variants) in enumerate(FACTS)
               for query in variants]
    return entries, queries


class McpProcess:
    def __init__(self, command: list[str], cwd: Path, env: dict[str, str], log: Path):
        self.log = log.open("wb")
        self.proc = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=self.log, bufsize=0)
        self.next_id = 1
        self.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                    "clientInfo": {"name": "agent-mem-competitive-eval", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        tools = self.request("tools/list", {}).get("tools", [])
        self.tools = {tool["name"] for tool in tools}

    def send(self, value: dict) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(value, separators=(",", ":")).encode() + b"\n")
        self.proc.stdin.flush()

    def request(self, method: str, params: dict, timeout: float = 45) -> dict:
        ident = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
        assert self.proc.stdout is not None
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"MCP server exited {self.proc.returncode}; see {self.log.name}")
            ready, _, _ = select.select([self.proc.stdout], [], [], min(1, deadline - time.monotonic()))
            if not ready:
                continue
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"MCP server closed stdout; see {self.log.name}")
            response = json.loads(line)
            if response.get("id") != ident:
                continue
            if "error" in response:
                raise RuntimeError(f"MCP protocol error: {response['error']}")
            return response.get("result", {})
        raise TimeoutError(f"MCP {method} timed out; see {self.log.name}")

    def call(self, name: str, arguments: dict) -> str:
        if name not in self.tools:
            raise RuntimeError(f"{name} missing from tools/list")
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        if response.get("isError"):
            raise RuntimeError(f"{name}: {response.get('content')}")
        return "\n".join(block.get("text", "") for block in response.get("content", [])
                         if block.get("type") == "text")

    def close(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.log.close()


def command(name: str, workspace: Path, env: dict[str, str]) -> list[str]:
    if name == "agent-mem":
        subprocess.run(["/opt/agent-mem", "init"], cwd=workspace, env=env,
                       check=True, stdout=subprocess.DEVNULL)
        return ["/opt/agent-mem", "mcp"]
    if name == "engram":
        return ["/opt/engram", "mcp"]
    if name == "projectmem":
        subprocess.run([sys.executable, "-m", "projectmem.cli", "init", "--no-hooks",
                        "--no-global", "--no-watch", "--no-backfill", "--no-claude-md",
                        "--no-stack-detect", "--no-mcp-config", "--no-structure"],
                       cwd=workspace, env=env, check=True, stdout=subprocess.DEVNULL)
        return [sys.executable, "-m", "projectmem.mcp_server", "--root", str(workspace)]
    raise ValueError(name)


def save_args(name: str, marker: str, value: str) -> tuple[str, dict]:
    if name == "agent-mem":
        return "mem_set", {"key": marker, "val": value, "scope": "project"}
    if name == "engram":
        return "mem_save", {"title": marker, "content": value, "type": "manual",
                            "capture_prompt": False}
    return "add_note", {"summary": f"{marker}: {value}"}


def find_args(name: str, query: str) -> tuple[str, dict]:
    if name == "agent-mem":
        return "mem_find", {"query": query, "scope": "project"}
    if name == "engram":
        return "mem_search", {"query": query, "limit": 5, "match_mode": "any"}
    return "search_events", {"query": query, "limit": 5}


def pctl(values: list[float], pct: float) -> float:
    values = sorted(values)
    return values[min(len(values) - 1, int((len(values) - 1) * pct))]


def disk_bytes(root: Path) -> int:
    return sum(path.stat().st_size for path in root.rglob("*") if path.is_file()
               and ".git" not in path.parts)


def run_one(name: str, entries: list[tuple[str, str]], queries: list[tuple[str, str]],
            root: Path) -> dict:
    workspace = root / name / "project"
    workspace.mkdir(parents=True)
    subprocess.run(["git", "init", "-q", str(workspace)], check=True)
    home = root / name / "home"
    home.mkdir()
    env = os.environ.copy()
    env.update({"HOME": str(home), "XDG_CONFIG_HOME": str(home / ".config"),
                "ENGRAM_DATA_DIR": str(root / name / "data"), "ENGRAM_PROJECT": "project",
                "PYTHONPATH": "/opt/projectmem/src"})
    server = McpProcess(command(name, workspace, env), workspace, env,
                        root / f"{name}.stderr.log")
    try:
        start = time.perf_counter()
        for marker, value in entries:
            tool, args = save_args(name, marker, value)
            server.call(tool, args)
        ingest = time.perf_counter() - start
        raw = []
        times = []
        shuffled = queries.copy()
        random.Random(42).shuffle(shuffled)
        for query, expected in shuffled:
            tool, args = find_args(name, query)
            started = time.perf_counter_ns()
            output = server.call(tool, args)
            elapsed_ms = (time.perf_counter_ns() - started) / 1e6
            times.append(elapsed_ms)
            ranked = list(dict.fromkeys(MARKER.findall(output)))
            rank = ranked.index(expected) + 1 if expected in ranked else None
            raw.append({"query": query, "expected": expected, "rank": rank,
                        "returned": ranked[:5], "output_bytes": len(output.encode()),
                        "query_ms": elapsed_ms})
        for repeat in range(1, 5):
            extra = queries.copy()
            random.Random(42 + repeat).shuffle(extra)
            for query, _ in extra:
                tool, args = find_args(name, query)
                started = time.perf_counter_ns()
                server.call(tool, args)
                times.append((time.perf_counter_ns() - started) / 1e6)
        mem = Path(f"/proc/{server.proc.pid}/status").read_text()
        peak = re.search(r"^VmHWM:\s+(\d+)", mem, re.MULTILINE)
        return {"name": name, "records": len(entries), "queries": len(queries),
                "recall_at_5": sum(x["rank"] is not None and x["rank"] <= 5 for x in raw) / len(raw),
                "mrr": sum(1 / x["rank"] for x in raw
                           if x["rank"] is not None and x["rank"] <= 5) / len(raw),
                "ingest_seconds": ingest, "query_ms_p50": statistics.median(times),
                "query_ms_p95": pctl(times, .95), "query_ms_p99": pctl(times, .99),
                "peak_rss_kib": int(peak.group(1)) if peak else None,
                "stored_bytes": disk_bytes(root / name), "cases": raw,
                "latency_samples_ms": times}
    finally:
        server.close()


def main() -> None:
    names = sys.argv[1:] or ["agent-mem", "engram", "projectmem"]
    entries, queries = corpus(int(os.environ.get("BENCH_DISTRACTORS", "200")))
    profile = os.environ.get("BENCH_QUERY_PROFILE", "natural")
    if profile == "keyword":
        queries = [(query, f"BID{i:04d}") for i, query in enumerate(KEYWORDS)]
    elif profile != "natural":
        raise ValueError(f"unknown query profile: {profile}")
    with tempfile.TemporaryDirectory(prefix="competitive-mcp-") as temp:
        root = Path(temp)
        results = []
        for name in names:
            print(f"running {name}", file=sys.stderr, flush=True)
            results.append(run_one(name, entries, queries, root))
        print(json.dumps({"method": "MCP stdio, warm server, one write per record, fixed corpus",
                          "environment": {"platform": os.uname().sysname, "machine": os.uname().machine,
                                          "distractors": len(entries) - len(FACTS),
                                          "query_profile": profile},
                          "results": results}, indent=2))


if __name__ == "__main__":
    main()
