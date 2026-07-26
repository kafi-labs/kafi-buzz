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
| `INTEL_MAX_TURNS_PER_WINDOW` | `30` | Admitted logical ACP turns per window, per scope. **`0` disables the quota** (it does not mean "deny all"). |
| `INTEL_QUOTA_WINDOW_SECS` | `3600` | Window length. Fixed window: the first turn starts it, and it resets once the window elapses. |

Scope is `{relay host}/{channel uuid}/{intel agent}` — one budget per channel per
agent. All channel-less traffic for one relay host and agent shares a stable
`nochannel` budget. One noisy direct-ACP client can therefore exhaust that budget
for other channel-less clients; this is deliberate for a cost control, because
reconnecting must not grant a fresh allowance.

The quota bounds admitted logical ACP turns, not billable gateway operations.
One admitted turn can drive multiple gateway attempts through the no-frame retry
or session-gone recreation paths, so N admitted turns do not guarantee at most N
gateway operations. An admitted turn that later fails (401, 429, 503, timeout,
or cancellation after admission) still consumes its slot. This is deliberate:
after a mid-stream failure the adapter cannot know that the model did no billable
work.

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

### Incomplete gateway responses

A successful answer requires a terminal SSE frame and non-whitespace response
text. If the stream closes without `Done`, any accumulated partial text is
discarded. Missing terminal frames and empty completed answers produce a safe,
owner-visible platform error (including the request id when available) in the
ACP transcript and, when a channel exists, in the Buzz channel instead of
publishing a partial or silently ending the turn.
