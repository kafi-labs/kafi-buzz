# Codebase Inventory & Gap Analysis

**Status:** Measured Baseline  
**Date:** September 2026  

---

## 1. Measured Overview

All line counts and file tallies below were measured directly from the respective workspaces.

| Codebase / Subsystem | Path | File Count | Lines of Code (LOC) | Primary Stack | Current State |
|---|---|---|---|---|---|
| **Buzz Web Client** | `web/src/` | 51 | 4,277 | Vite + React 19 + TanStack Router | Repo browser + invite flow; read-only Nostr client (no live WS, no publish) |
| **Sibling Biosv3** | `biosv3/src/` | 16 | 1,906 | Next.js 15 App Router + React 19 | Chat prototype; persistent WS, NIP-42, #h subscription, kind-9 publish (Milestone 1 green) |
| **Desktop Messages Feature** | `desktop/src/features/messages/` | 114 | 45,738 | Tauri 2 + React 19 + TanStack Query | Full-featured chat (threads, reactions, voice notes, media, cards, quotes, edit/delete) |
| **Desktop Channels Feature** | `desktop/src/features/channels/` | 76 | 23,399 | Tauri 2 + React 19 + TanStack Query | Channel lifecycle, unread tracking, permissions, welcome dialogs, metadata |

---

## 2. Biosv3 Module Inventory & Disposition

`biosv3` (at `/Users/fitrakacamarga/project/kafi/biosv3/src`) contains 16 files total (1,906 LOC). Below is the granular disposition for every single file:

| File Path | LOC | Category | Disposition | Detailed Technical Rationale |
|---|---|---|---|---|
| `src/lib/buzz/client.ts` | 461 | Protocol / Client | **Port & Adapt** | Core persistent WebSocket client. Handles connect lifecycle, NIP-42 AUTH challenge handling, subscription state map (`ActiveSub`), `#h` channel scoping, EOSE tracking, buffered event replay, and `sendMessage` kind-9 publish with `OK` promise resolution. **Adaptation required:** enhance reconnect backoff, integrate with `web/src/shared/lib/relay-url.ts`, support dynamic kind lists from codegen `kinds.ts`. |
| `src/lib/buzz/client.test.ts` | 347 | Tests | **Port as-is** | Comprehensive unit tests using mock WebSocket factory covering: connection states, auth challenge-response, unauthenticated grace period, subscription multiplexing, EOSE replay, publishing ack/nack. Port directly to `web/src/shared/lib/__tests__/buzz-client.test.ts` (or Vitest suite). |
| `src/lib/buzz/signer.ts` | 94 | Protocol / Crypto | **Port & Adapt** | Implements `Signer` interface, `EphemeralSigner` (using `nostr-tools/pure`), `buildKind9Event`, and `buildAuthEvent`. **Adaptation required:** unify with `web/src/shared/lib/nostr-signer.ts` so that `Nip07Signer` and `PersistentKeySigner` implement the same `Signer` interface. |
| `src/lib/buzz/types.ts` | 51 | Protocol / Types | **Port as-is** | Clean TypeScript interfaces: `NostrEvent`, `UnsignedNostrEvent`, `SignedNostrEvent`, `NostrFilter`, `ConnectionState`, `TimelineListener`, `PublishAck`. Merge into `web/src/shared/lib/nostr-types.ts`. |
| `src/lib/buzz/kinds.ts` | 20 | Constants | **Drop (Use Codegen)** | Hardcoded kind constants (`KIND_CHANNEL_MESSAGE = 9`, `KIND_EPHEMERAL_CHAT = 40002`, `KIND_REACTION = 7`, `KIND_AUTH = 22242`). Drop in favor of `web/scripts/gen-kinds.mjs` generated `web/src/shared/constants/kinds.ts`. |
| `src/lib/buzz/index.ts` | 26 | Barrel Export | **Port as-is** | Re-exports client, signer, kinds, and types. |
| `src/components/ChannelView.tsx` | 192 | UI Component | **Drop / Rewrite** | Next.js `"use client"` container. Hardcodes single channel (`DEFAULT_CHANNEL = "lobby"`), uses local React state instead of routing/URL parameters, and mixes layout with protocol setup. Rewrite natively as TanStack Route component at `web/src/app/routes/channels.$channelId.tsx`. |
| `src/components/MessageBubble.tsx` | 140 | UI Component | **Adapt (Extract logic)** | Message rendering component. Contains useful inline markdown tokenizer (`renderInlineMarkdown`) and message alignment logic. Adapt into `web/src/features/chat/ui/MessageBubble.tsx` using `web/` design tokens and components. |
| `src/components/MessageComposer.tsx` | 102 | UI Component | **Adapt** | Textarea input with Enter-to-send, Shift+Enter newline, and disabled state during submit. Adapt to use `web/src/shared/ui/input.tsx` or `textarea`, styled with Tailwind and connected to route params. |
| `src/components/StoryboardCard.tsx` | 146 | UI Component (Kafi) | **Drop (Out of Scope)** | Custom card renderer for Kafi-specific `id.kafi.storyboard` payloads. Out of scope for general Buzz OSS web chat client (can be an optional extension later). |
| `src/shared/storyboard.ts` | 163 | Parser / Validator | **Drop (Out of Scope)** | Storyboard JSON / fenced code block parsing and validation. Kafi-specific product feature. |
| `src/shared/personas.ts` | 53 | Constants | **Drop (Out of Scope)** | Kafi AI persona metadata (Maya, Budi, etc.). Drop for OSS Buzz client. |
| `src/shared/index.ts` | 21 | Barrel Export | **Drop** | Exports storyboard and persona helpers. |
| `src/app/page.tsx` | 5 | Next.js Route | **Drop** | Next.js root page rendering `<ChannelView />`. Drop; route structure is managed by TanStack Router. |
| `src/app/layout.tsx` | 51 | Next.js Layout | **Drop** | Next.js root layout with font and metadata declarations. `web/` already has `web/src/app/App.tsx` and `web/src/app/routes/root.tsx`. |
| `src/app/globals.css` | 34 | Styles | **Drop** | Custom Kafi palette (`--kafi-forest`, `--kafi-warm`, etc.). Drop; `web/src/shared/styles/globals.css` already provides full Catppuccin theme variables. |

---

## 3. Existing Buzz Web Client (`web/src/`) Inventory

`web/` currently has 51 files totaling 4,277 LOC.

### Assets & Foundation
* `web/src/main.tsx` (36 LOC) & `web/src/app/App.tsx` (7 LOC): Application entry point and root mounting.
* `web/src/app/router.tsx` & `routeTree.gen.ts` (161 LOC): TanStack Router setup.
* `web/src/shared/theme/` (118 LOC): `ThemeProvider.tsx` and `ThemeToggle.tsx` providing light/dark/system mode.
* `web/src/shared/ui/` (249 LOC): Reusable Radix / shadcn UI components: `badge`, `button`, `card`, `input`, `sonner`, `tooltip`.
* `web/src/shared/styles/globals.css` (100 LOC): Tailwind base styling and theme variables.

### Shared Nostr & Protocol Libraries (The Existing Base)
* `web/src/shared/lib/nostr-signer.ts` (106 LOC):
  * Supports NIP-07 browser extensions (`window.nostr`).
  * Generates page-lifetime ephemeral keypairs (`generateSecretKey()`).
  * Supports `requireNip07` flag for actions requiring durable identity.
  * **Gap:** Does not implement a unified `Signer` interface compatible with long-lived client instances; doesn't store keys in `localStorage` for non-NIP-07 session persistence.
* `web/src/shared/lib/nostr-client.ts` (175 LOC):
  * `queryEvents(wsUrl, filter)` opens a WS, sends NIP-42 AUTH if challenged, issues REQ, collects events until EOSE, then terminates the connection.
  * **Gap:** Designed exclusively for one-shot batch queries; completely lacks long-lived connection management, live event streaming, subscription multiplexing, and event publishing (`EVENT` frame with `OK` wait).
* `web/src/shared/lib/nip98.ts` (48 LOC): HTTP authorization header generation (used for git and admin REST endpoints).
* `web/src/shared/lib/relay-url.ts` (24 LOC): WebSocket relay URL resolution and sanitization.

### Feature Modules
* `web/src/features/invite/` (596 LOC across 3 files): Invite token parsing, policy display, and NIP-98 invite claiming.
* `web/src/features/repos/` (2,654 LOC across 16 files): Complete git repository explorer (tree view, blob viewer, commit history, refs, git-over-HTTP).

---

## 4. Scale & Complexity Comparison: Desktop vs Web

A critical finding from codebase analysis is the immense scale difference between the desktop chat implementation and the proposed web client:

```
┌────────────────────────────────────────────────────────┐
│ Desktop Chat Subsystems (69,137 LOC)                   │
│                                                        │
│ ├─ features/messages (45,738 LOC)                      │
│ │   ├─ Slate / Prosemirror rich-text message input     │
│ │   ├─ Nested thread flyouts & active thread sync      │
│ │   ├─ Inline voice memo recorder & audio visualizer   │
│ │   ├─ Blossom image/video upload & frame comments     │
│ │   ├─ Message search & local SQLite indexing          │
│ │   ├─ Event reaction aggregates & live emoji picker   │
│ │   └─ Agent turn observation & metric cards           │
│ │                                                      │
│ └─ features/channels (23,399 LOC)                      │
│     ├─ NIP-29 group permission & role management       │
│     ├─ Complex unread counts & scroll-marker sync      │
│     ├─ Channel create/edit/delete/archive modals       │
│     ├─ Ephemeral channel TTL management                │
│     └─ Welcome flow & bot auto-provisioning            │
└────────────────────────────────────────────────────────┘
                           ▲
                           │  ~10x - 15x scale gap
                           ▼
┌────────────────────────────────────────────────────────┐
│ Target Web Chat Client (Estimated ~3,000 - 3,500 LOC)  │
│                                                        │
│ ├─ Protocol & Data Layer: ~1,000 LOC                   │
│ │   (BuzzClient, Signer seam, Subscription manager)    │
│ ├─ Channel Navigation & Sidebar: ~600 LOC              │
│ ├─ Channel Timeline & Markdown: ~800 LOC               │
│ └─ Message Composer & Identity: ~600 LOC               │
└────────────────────────────────────────────────────────┘
```

### Strategic Takeaways from the Inventory:
1. **Do Not Port Desktop Directly:** Desktop chat contains thousands of lines of Tauri IPC bridges (`invoke(...)`), local SQLite database hooks, and platform-specific audio/windowing code that are completely invalid in a standard browser environment.
2. **Leverage Biosv3's Lean Protocol Architecture:** `biosv3` extracted the essential Nostr protocol minimum for real-time chat in under 1,000 LOC. That protocol core is sound and proven.
3. **Build Web Chat as a Co-Equal Feature in `web/`:** Place the new chat functionality in `web/src/features/chat/` and `web/src/app/routes/channels/`, living harmoniously alongside `web/src/features/repos/` and `web/src/features/invite/`.
