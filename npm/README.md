# agent-mem

NPX launcher for the native `agent-mem` binary. The launcher downloads the matching GitHub
release for the current platform, verifies its published SHA-256 checksum, caches it locally,
and forwards all arguments and stdio without altering the MCP JSON-RPC stream.

```bash
npx agent-mem --help
npx agent-mem mcp
```

Project documentation: https://github.com/marcosotomac/agent-mem
