# Threat model

## Scope and assets

`agent-mem` stores repository memory in `.agent-rules` and a local SQLite cache,
serves it to an MCP client, and installs release artifacts. The protected assets
are source code, client configuration, local files, secrets available to the
calling agent, memory integrity, and host resources.

## Trust boundaries

- Repository files, including `.agent-rules`, are untrusted. A clone, dependency,
  pull request, or merge can introduce adversarial text.
- MCP arguments are untrusted even when they originate from a local client.
- Recalled memory is data, not an instruction channel. The MCP client remains
  responsible for separating tool output from system/developer authority.
- The local user and explicitly invoked `agent-mem mcp install` command are
  trusted to authorize client configuration changes. `agent-mem init` does not
  edit client configuration.
- GitHub Actions and the release repository are part of the distribution trust
  boundary. Checksums detect archive corruption; provenance attestations bind
  artifacts to the release workflow.

## Principal threats and controls

### Prompt injection through `.agent-rules`

An attacker can store text such as instructions to run a command, ignore policy,
or exfiltrate a secret. `agent-mem` does not execute rule contents, fetch URLs
from them, or elevate them above normal tool data. The text format is reviewable
in Git, malformed encoded records are rejected atomically, and archived rules
are distinguishable from active ones. Imports from `.agent-rules` are recorded
with `untrusted` trust and `rules-file:.agent-rules` provenance; MCP context and
search label those values. Only the local CLI can promote a record to `reviewed`,
so a model cannot approve its own input through MCP. Consumers must treat
returned values as quoted evidence and validate any requested action against
higher-priority policy.
Semantic prompt-injection detection is intentionally not claimed: keyword filters
are bypassable and would create false confidence.

### Secret persistence

Writes and imports reject high-confidence credential shapes (private-key PEMs,
GitHub and AWS keys, JWTs, bearer credentials, and explicit secret assignments)
before opening a transaction. Rejection is atomic and errors never echo the
suspected credential. The scanner deliberately avoids entropy-only guesses and
keyword-only matches, so it reduces accidental persistence but cannot prove that
arbitrary text contains no secret. Upstream clients must still redact credentials
and use dedicated secret scanners for repository-wide assurance.

### Resource exhaustion and oversized writes

The store rejects keys over 512 bytes, values over 64 KiB, anchors over 4 KiB,
kinds and relation types over 64 bytes, and relation targets over 512 bytes. MCP
accepts at most 256 batch items, 1 MiB of aggregate batch fields, and 1 MiB per
newline-delimited JSON-RPC request. `.agent-rules` imports are capped at 32 MiB.
Validation completes before starting a write transaction, so rejected batches do
not partially mutate memory. MCP text responses are capped at 16 KiB and context
results at 50 records.

### Cross-project writes

Project-scoped MCP calls resolve an explicit registered id, unique name, or
canonical path. Ambiguous selection fails closed. The global MCP process does not
create a project database in its launch directory. Global scope is separate from
project scope.

### Unauthorized MCP mutation

`agent-mem mcp --read-only` and `AGENT_MEM_READ_ONLY=1` remove write tools from
discovery and reject write dispatches independently. This is a process-wide
capability boundary for clients that only need retrieval. Full-mode writes retain
the limits above and record their MCP scope as provenance.

### Configuration and filesystem mutation

Initialization is repository-local and does not modify AI-client configuration.
Client setup requires an explicit `mcp install` command, rejects malformed config,
creates a sibling backup, writes a sibling temporary file, and atomically replaces
the target where the platform permits it. Repository sync writes through an
exclusive temporary file and rename.

### Release substitution

NPX and the shell installer download over HTTPS and verify the selected archive
against `SHA256SUMS.txt` before extraction. The release workflow generates the
manifest from built artifacts and emits GitHub build-provenance attestations.
The NPX launcher never substitutes a binary found on `PATH`, version-checks its
version-specific cache, and accepts a development override only when its version
matches the package. Package and Homebrew metadata are version-gated in CI, and
release archives run clean-install round trips on Linux, macOS, and Windows
before publication. Checksums are integrity
controls, not protection from a compromised release account; verify the GitHub
attestation when that threat matters.

## Residual risks

- A compromised MCP client or model can misuse otherwise valid memory and tools.
- A malicious committer can add plausible but false rules that pass syntax checks.
- Local processes with the user's permissions can modify the database, rules,
  client config, binary, or checksum manifest.
- Checksums obtained from the same compromised release page do not provide an
  independent trust root; provenance verification reduces but does not eliminate
  supply-chain risk.

Report vulnerabilities through the private process in [SECURITY.md](SECURITY.md).
