# Buzz Web Chat Client Specification

**Status:** Plan & Architectural Specification  
**Author:** WEB-CHAT PLANNING Lane (`buzz-architect` orchestration)  
**Date:** September 2026  
**Target Worktree:** `web/` in `block/buzz`  

---

## The Verdict

**"Port the LIB, rewrite the UI natively in `web/`."** Folding `biosv3` into `buzz`'s existing `web/` package is significantly faster than starting from scratch, but only if we draw a strict line between protocol logic and presentation. `biosv3`'s `src/lib/buzz/` provides a working, tested (9/9 tests green) TypeScript data client for persistent WebSocket connections, NIP-42 authentication challenge-response, `#h`-scoped REQ timeline subscriptions with EOSE tracking, and kind-9 message publishing. However, `biosv3`'s UI is written for Next.js App Router using hardcoded single-channel state and Kafi-specific branding. `web/` already runs Vite, React 19, `@tanstack/react-router`, Tailwind, and Biome. The winning path ports and matures `biosv3`'s 973 LOC protocol layer directly into `web/src/shared/lib/` or `web/src/features/chat/`, while rebuilding the client UI natively using `web/`'s TanStack Router and UI primitives.

---

## Executive Answers to Core Planning Questions

### 1. Is folding biosv3 in actually cheaper than writing chat natively in web/?
**Yes, by approximately 40% (saving 5–7 engineering days), provided we port the library and rewrite the UI.**
* **What is reusable as-is/adapted:** `biosv3/src/lib/buzz/client.ts`, `signer.ts`, `types.ts`, `kinds.ts`, and `client.test.ts` (973 LOC total). This cleanly solves the hardest protocol mechanics that `web/` currently lacks: persistent WebSocket state machine, NIP-42 challenge-response on reconnect, multi-channel `#h` subscriptions, EOSE backfill replay, and event publishing ack handlers (`OK` tracking).
* **What cannot be ported directly:** The Next.js UI components (`ChannelView.tsx`, `MessageBubble.tsx`, `MessageComposer.tsx`, `StoryboardCard.tsx` — 580 LOC). They use `"use client"`, rely on Next.js environment variables, lack routing, and carry Kafi-specific styling (`bg-kafi-warm`, `text-kafi-forest`). Attempting to force Next.js patterns into `web/`'s Vite + TanStack Router tree would cause impedance mismatch and tech debt. Rewriting the UI in TanStack Router using existing `web/src/shared/ui/` primitives takes ~3 days and yields a cohesive application.

### 2. What is the smallest thing that is genuinely 'Buzz in a browser'?
**Phase 1 MVP: Single-channel authenticated live chat (`/channels/$channelId`).**
* **User Experience:** A user navigates to `/channels/lobby` (or `/channels/general`), authenticates via NIP-07 extension or a persisted browser key, sees the historical message timeline load via NIP-01 REQ, receives real-time incoming kind-9 messages via live WebSocket subscription, and can publish new kind-9 messages via the message composer.
* **Done-When:** An engineer can open `http://localhost:3000/channels/lobby` in Chrome and chat back-and-forth in real time with the Buzz desktop app or `buzz-cli` connected to the same relay, with zero dropped messages, correct author display, and markdown rendering.

### 3. What does the relay need to change, exactly which files, for routes to serve in production?
**Two files in `crates/buzz-relay` must be updated:**
1. `crates/buzz-relay/src/router.rs`:
   * Update `should_serve_spa(path, serve_git_web_gui)` / `is_git_web_gui_path(path)` to recognize client routes (`/`, `/channels`, `/channels/*`, `/dms`, `/dms/*`, `/settings`, `/members`).
   * Update `nip11_or_ws_handler` to serve `index.html` on browser requests to `/` when the web client is enabled.
2. `crates/buzz-relay/src/config.rs`:
   * Generalize `serve_git_web_gui: bool` to `serve_web_client: bool` (or alias `BUZZ_SERVE_WEB_CLIENT` alongside `BUZZ_SERVE_GIT_WEB_GUI` for backward compatibility).
   * Update config parsing and validation.

---

## Document Map

| Document | Purpose |
|---|---|
| [`01-inventory.md`](./01-inventory.md) | Measured codebase inventory: `web/` vs `biosv3` vs `desktop/`, file-by-file disposition table (port / adapt / drop). |
| [`02-architecture.md`](./02-architecture.md) | Target architecture for `web/`, TanStack Router integration, live subscription lifecycle, identity/signing models, and relay route gating. |
| [`03-plan.md`](./03-plan.md) | Phased implementation plan (Phases 0–5) with explicit `DONE-WHEN` verification criteria and day-by-day effort estimates. |
| [`04-risks.md`](./04-risks.md) | Technical and operational risks, failure modes, and open questions requiring owner decisions. |

---

## Core Tenets & Constraints

1. **Relay is Single Source of Truth:** No secondary databases or proxy backends. The web client communicates strictly via Nostr over WebSocket (NIP-01, NIP-42, NIP-29) and HTTP bridge (NIP-98).
2. **Channel Tagging Discipline:** All channel messages MUST use `h` tags (NIP-29 group scoping), NEVER `e` tags for channel identification.
3. **Explicit Query Kinds:** All relay REQ queries MUST explicitly specify `kinds` (e.g. `[9, 40002, 7, 39000]`) to avoid triggering the relay p-gate and receiving 403 Forbidden.
4. **Zero Drift on Event Kinds:** Kinds must align with `crates/buzz-core/src/kind.rs` using the automated codegen `web/scripts/gen-kinds.mjs`.
5. **Honest Scope Pricing:** Desktop chat is 69,137 LOC. Web client is NOT attempting 1:1 parity with desktop on day one; it delivers a lean, reliable web chat client focused on core collaboration.
