# Roadmap — Buzz as a standalone web platform, Intelligence Platform as its AI harness

**Status:** written 2026-07-27, on branch `feat/intel-turn-quota`.
**Scope decision (user, 2026-07-27):** this is a **dedicated platform**. It does **not** integrate
with Kafi.

## What "no Kafi" removes, and what it obliges

Out of scope: Tower OIDC/SSO, the `kafi_agent` / `kafi_cart` clients, Cart/Book/CFO, and
`kafi-signer` as the identity path.

That deletes a cross-repo dependency and an availability coupling — Buzz no longer needs Tower to be
up for a user to sign in. The obligation it creates: **Buzz must own identity itself.** It already
does. The invite flow enrols a NIP-07 identity for browser access and is e2e-tested
(`web/tests/e2e/smoke.spec.ts`). This is a foundation to build on, not a gap to fill.

If web credential custody is ever needed, it must be decided on Buzz's own terms as a Buzz-native
component. `kafi-signer` remains useful as *prior art on the threat model* — its README is honest
that "the operator can sign as any user" — but not as a dependency.

## Verified starting point

Everything below is confirmed against the live host, not inferred:

- Intel harness live on `vm-buzz-relay-dev-wren`, audited across three adversarial passes. Fixed:
  a quota bypass (any client got a fresh budget by reconnecting), truncated and empty answers
  published as complete, cancellation that took up to 9.5 minutes, and two latent secret leaks.
- **Owner identity restored and proven end to end (2026-07-27).** Prompt `3d341f9e…` →
  reply *"The result of 17 times 23 is 391."* The agent is a working coworker on the real gateway.
- Desktop config complete: roster picker in create **and** edit; the gateway URL field, which was
  inert, now propagates.
- Web read-only inventory console at `/intelligence`, served behind `BUZZ_SERVE_INTEL_CONSOLE`
  (default off).
- Real-AI e2e suite (`e2e_intel_coworker.rs`) — opt-in, property-based assertions.

## Phase 1 — Deployable web app *(unblocked today)*

1. Dedicated VM `vm-buzz-web-<short>`: own DB, own signing secrets, per exe.dev rules.
2. Deploy relay + web bundle. The root `Dockerfile` already builds both (stage 4 is the vite
   bundle). Enable `BUZZ_SERVE_GIT_WEB_GUI=true` and `BUZZ_SERVE_INTEL_CONSOLE=true`.
3. **Verify every route serves from the relay, not just from a local build.** `/`, `/repos*`,
   `/invite/<code>`, `/intelligence`.
4. Deliverable: a clickable `<name>.exe.xyz`.

**Two traps, both hit already on this branch:**
- A route can build, pass e2e, and still 404 — the web e2e serves the SPA directly, while the relay
  has its own path allowlist. Check the server that will serve it in production.
- One public port per VM. The relay serves WS **and** HTTP on 3000. Do not `share port` a second
  service; doing so silently made 3000 private and broke the agent.

## Phase 2 — Identity, decided inside Buzz

| Path | Reach | Cost |
|---|---|---|
| **Invite + NIP-07** *(today)* | users with a browser extension | none — built and tested |
| **NIP-46 remote signer** | key stays with the user, no extension | ecosystem-standard; needs a signer app to exist |
| **Buzz-native custody** | anyone, no key handling | operator can sign as any user; a crown-jewel service |

**Recommendation:** ship Phase 1 on invite + NIP-07 and *measure*. That is a real product for
technical users now. Move only on evidence of users who cannot install an extension — and then
prefer **NIP-46**, because custody creates an always-on security liability that a standalone
platform has no natural owner for. Custody is a decision about who carries a pager, not a feature.

## Phase 3 — Intel on web

Read-only inventory now (agreed and shipped). Configuration stays on desktop, where credentials sit
at rest at `0600` and are never published — a property a broadcast relay cannot offer.

**The constraint that shapes any future work here:** custody solves *holding a key*. It does **not**
let a browser write another machine's local config. The agent still executes somewhere with local
state. So the real question is **where agents execute** — and for a dedicated platform the honest
answer is hosted agent execution, which is a far larger commitment than a config page. Do not let a
"web config" ticket smuggle that in.

## Phase 4 — Real-AI e2e

Opt-in (`#[ignore]`), never in default CI because each turn spends money. Assertions are properties,
not strings: exact-answer arithmetic, multi-turn memory, emoji integrity. wren is now a valid target.

## Phase 5 — Own the relay image

wren currently runs upstream `ghcr.io/block/buzz:main`. A standalone platform needs images built
from this fork; otherwise every deploy inherits whatever upstream shipped — including the
`mesh_demo` breakage encountered here.

## Sequencing

Phase 1 → measure → Phase 2 **only if reach demands it**. Phases 3–5 are independent of that path.

## The operating rule this branch earned

Ten defects here were not broken logic but **artifacts misdescribing themselves**: a README
asserting the inverse of the behaviour, a health suite measuring only installability, a labelled
field that configured nothing, a comment describing a dropdown never built, and a console reported
as shipped that 404'd. Each passed review *because it looked deliberate*.

**Verify a safety property at the call site — never from the name, the comment, or the test beside
it.**
