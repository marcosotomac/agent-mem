# End-to-end agent benchmark

This suite replaces synthetic agent-quality claims with model executions against
real GitHub issues. Harbor supplies the external repositories, task instructions,
isolated environments, and independent SWE-bench Verified tests. Generated jobs
and trials are evidence; this directory intentionally contains no claimed result.

## Experimental contract

- **Capability:** use persistent engineering history without repeating known bad
  approaches or spending unnecessary context and latency.
- **Question:** fix one verified issue from each of Astropy, Django, and SymPy.
- **Environment:** the same Codex agent version, model, task, resource policy, and
  constructed prior-attempt fixture for every memory product. The native baseline
  receives no external memory. Memory products receive identical facts through
  their real MCP servers.
- **Success:** the independent task verifier passes. Tokens and agent latency come
  from Harbor's recorded agent stats. A repeated error is the same normalized
  verifier-failure signature appearing in more than one repetition.

The matrix uses two models, four baselines (`native`, `agent-mem`, ProjectMem, and
the official MCP memory server), three repositories, and three repetitions: 72
trials. Package, agent, Harbor, and the SWE-bench Verified dataset digest are
pinned; retain `config.json`, `lock.json`, trajectories, verifier logs, and
`result.json` with every published report.

The history fixture is explicitly constructed and contains no reference patches
or hidden-test answers. It models previously rejected classes of change. It must
not be described as observed production history.

## Run

Validation generates every job and asks Harbor to parse it without starting a
container or spending model tokens:

```bash
python3 evals/run_matrix.py
```

After configuring the model-provider credentials required by Harbor, execute the
paid matrix explicitly:

```bash
python3 evals/run_matrix.py --execute
```

Narrow smoke runs before the full campaign:

```bash
python3 evals/run_matrix.py --execute --baseline native --model openai/gpt-5.2-2025-12-11 \
  --attempts 1 --task-glob '*astropy__astropy-*'
python3 evals/run_matrix.py --execute --baseline agent-mem --model openai/gpt-5.2-2025-12-11 \
  --attempts 1 --task-glob '*astropy__astropy-*'
```

The first local trial populates `evals/.cache/` with the pinned Codex/NVM
installation. Later isolated containers mount that cache and skip repeated
agent installation. The runner also reuses uv's package-download cache for the
SWE-bench verifier; task repositories and result directories remain isolated.
For the `agent-mem` baseline, the runner also builds the current source in a
pinned slim Linux/amd64 Rust container and mounts that binary into SWE-bench. This
prevents host binaries (for example macOS/ARM) from invalidating MCP startup.
The build is reused until `Cargo.toml`, `Cargo.lock`, or a Rust source file changes.

Once both smokes complete without infrastructure errors, use a unique immutable
campaign id for the full 72 trials. Job directories are never overwritten:

```bash
python3 evals/run_matrix.py --execute --campaign v1-3-0
python3 evals/report.py --campaign v1-3-0 --gate
python3 evals/package_evidence.py --campaign v1-3-0
```

Summarize retained evidence:

```bash
python3 evals/summarize.py > evals/results.csv
```

The campaign reporter writes tracked Markdown and JSON summaries under
`evals/published/`. Its gate requires all 72 trials, zero infrastructure errors,
complete success/token/latency evidence, observed agent-mem success no lower than
native in every model/task cell, required MCP tool calls in every memory trial,
and fewer aggregate tokens than native. This is
an observed release gate, not a claim of statistical non-inferiority. The legacy
CSV summarizer exits nonzero if tokens or latency are absent; missing evidence is
never replaced by an estimate. Infrastructure failures (build, credential,
adapter, timeout, or verifier failure) are not scored as agent failures and must
be reported separately from task success.

## Fairness and reporting

- Randomize or interleave job order when capacity or provider load changes.
- Keep temperature/reasoning settings identical within a model.
- Report per-cell success rate, median and p95 input/output tokens, median and p95
  agent latency, repeated-error rate, and infrastructure-error count.
- Publish raw Harbor jobs or a content-addressed archive, model identifiers, host
  OS/architecture, Docker version, and the exact commit of this repository.
- Do not claim statistical superiority from three repetitions. Show the raw cells
  and uncertainty; increase repetitions before making product claims.
