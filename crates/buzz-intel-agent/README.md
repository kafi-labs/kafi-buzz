# buzz-intel-agent

ACP adapter that bridges Buzz managed agents to Intelligence Platform agents.

- Speaks ACP (JSON-RPC 2.0 NDJSON) on stdio toward `buzz-acp`
- Speaks REST/SSE toward the intel gateway
- Posts replies to the Buzz relay via `buzz-sdk` (kind-9 stream messages)

See `specs/intel-agent-integration/04-adapter-spec.md` for the full contract.

## Quick start

```bash
export INTEL_GATEWAY_URL=https://intel-platform.exe.xyz
export INTEL_API_KEY=intel_...
export INTEL_AGENT=demo-agent
export BUZZ_RELAY_URL=http://localhost:3000
export BUZZ_PRIVATE_KEY=...

# Auth probe for desktop catalog
buzz-intel-agent --auth-probe

# List available intel agents (JSON)
buzz-intel-agent --list-agents

# Run as ACP server (stdio)
buzz-intel-agent
```

## Configuration

All settings are env-first; CLI flags mirror them. Required: `INTEL_GATEWAY_URL`,
`INTEL_API_KEY` (or `INTEL_API_KEY_FILE`), `INTEL_AGENT`. Injected by the harness:
`BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG`.
