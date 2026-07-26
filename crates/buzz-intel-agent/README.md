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

### LLM turn quota (cost bound)

| Env | Default | Meaning |
|---|---|---|
| `INTEL_MAX_TURNS_PER_WINDOW` | `30` | Paid gateway turns allowed per window, per scope. **`0` disables the quota** (it does not mean "deny all"). |
| `INTEL_QUOTA_WINDOW_SECS` | `3600` | Window length. Fixed window: the first turn starts it, and it resets once the window elapses. |

Scope is `{relay host}/{channel uuid}/{intel agent}` — one budget per channel per
agent. When a prompt carries no channel the ACP session id is used instead, so a
harness session cannot bypass the budget by omitting the channel.

**This is enabled by default.** It bounds spend on a surface that previously had
none: the relay rate-limits *protocol admission* (WS connects and EVENT ingest,
via `buzz_pubsub::rate_limiter::RedisRateLimiter`), but one admitted mention can
still cost a full LLM turn. Nothing between the harness and the gateway bounded
that. Raise the limit or set it to `0` if an agent is expected to run hot.

When the quota is hit the adapter **does not call the gateway at all** — no
session create, no message — posts a visible message to the channel saying what
the limit is and when it resets, and ends the turn with stop reason `refusal`.
A silent stop would be indistinguishable from an outage.

`INTEL_MAX_TOKENS_PER_WINDOW` is **not implemented**: the gateway's SSE frames
carry no token usage (`MESSAGE_EVENT_TYPE_*` has no usage field), so the adapter
has no token count to meter. Adding it requires a gateway-side change to report
usage; the turn counter is the honest bound available today.
