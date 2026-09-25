# Competitive retrieval benchmark

This benchmark runs five actual memory implementations in isolated Linux/ARM64
Docker environments. It measures **retrieval of supplied engineering memories**.
The corpus is controlled: ten answer-bearing facts, 200 vocabulary-overlapping
distractors, 20 natural-language queries, and ten one-word queries. Each product
receives the same descriptive title, identifier, and fact text. Expected IDs are
declared before search; the first pass determines Recall@5 and MRR@5. Five
shuffled passes supply 100 natural-query and 50 keyword-query latency samples.

| Product | Source | Interface/configuration measured |
|---|---|---|
| agent-mem | this repository | `mem_set` / `mem_find`, warm MCP stdio server |
| Engram | [Gentleman-Programming/engram](https://github.com/Gentleman-Programming/engram) | `mem_save` / `mem_search` (`match_mode=any`), warm MCP stdio server |
| ProjectMem | [riponcm/projectmem](https://github.com/riponcm/projectmem) | `add_note` / `search_events`, warm MCP stdio server |
| Mem0 OSS | [mem0ai/mem0](https://github.com/mem0ai/mem0) | Python API, `infer=False`, local Qdrant, OpenAI `text-embedding-3-small` |
| TencentDB Agent Memory | [TencentCloud/TencentDB-Agent-Memory](https://github.com/TencentCloud/TencentDB-Agent-Memory) | MemoryCore v3 conversation L0 add/search over persistent HTTP, English BM25, extraction disabled |

Each write is one record per call. Process initialization and package installation
are outside the timed loop. The first query pass follows a fixed shuffled order;
four additional passes use fixed seeds. All runs use Linux/ARM64 on the same
Docker host, and the products are measured sequentially. Mem0 query time includes
the remote embedding API. TencentDB uses HTTP while the first three use MCP
stdio; the latency figures are deployed-path timings, not identical transport
kernel timings. `infer=False` and disabled extraction compare retrieval of supplied
facts; they do not test automatic extraction, contradiction resolution, code-task
success, or cost per solved issue. The separate [Harbor suite](../README.md)
addresses real coding tasks when run with the same agent/model across products.

## Source revisions

- Engram `facaba842db32b55d3a13a6ea16ea31968ae6ce6`
- Mem0 `94c3fe9f238f3dbf29c9ce98643bd71eb13077cd` (package 2.2.1)
- TencentDB `bd88cc83870bf9e7dbd2ec36aa13608d2295c7f4`
- ProjectMem `e8d73137acde6f091ef6196f88ba5eccf6eb0e8a` (package 0.3.3)

The source pins, image digest, local source commit, full query outcomes, and
latency samples accompany the [results](results/). No API key is stored there.

## Run

`run_local.py` expects `/opt/agent-mem`, `/opt/engram`, and ProjectMem installed
in a Linux container; it creates isolated temporary homes and Git repositories.
Set `BENCH_DISTRACTORS=200` and `BENCH_QUERY_PROFILE=natural` or `keyword`.
Build the binaries from the pinned sources for the same architecture. In this
campaign, agent-mem was built with Rust 1.89 and Engram with Go 1.25.1. The
three processes then run under Python 3.12.

`run_mem0.py` requires Mem0 2.2.1 and `OPENAI_API_KEY` at runtime. It creates
local Qdrant and history databases under a temporary directory, runs both query
profiles, and emits JSON. `run_tencent.py` requires a running MemoryCore v3
container, with its standalone config set to `host=0.0.0.0`, English BM25,
`memory.extraction.enabled=false`, and a local service token. It also emits both
profiles in one run. Keep provider keys outside this repository and pass them to
Docker by environment file.

Raw JSON uses the same fields for all products. Run `python3 summarize.py` after
placing `local-natural.json`, `local-keyword.json`, `mem0.json`, and `tencent.json`
under `results/`; it validates case counts, ranks, and recorded metrics before
writing `results/summary.json`. Run `uv run --with matplotlib plot.py` to render
the figure from that summary.
