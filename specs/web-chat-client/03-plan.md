# Phased Implementation Plan

**Status:** Implementation Roadmap  
**Date:** September 2026  
**Total Estimated Effort:** 10.5 working days (~2.1 engineer-weeks)  

---

## Plan Overview

Each phase is designed to be **independently testable and shippable**. At no point should `web/` be left in a broken or half-migrated state. Existing repo browsing and invite flows remain green across all phases.

```
┌────────────────────────────────────────────────────────────────────────┐
│ Phase 0: Relay Route Gate & Kind Codegen Sync (0.5 days)               │
│ - Rust relay SPA allowlist expanded for /channels/*, /settings         │
│ - gen-kinds.mjs wired for chat event kinds                             │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│ Phase 1: Real-Time Protocol & Data Layer (1.5 days)                     │
│ - Port biosv3 BuzzClient & test suite into web/src/shared/lib/         │
│ - Unify Signer seam (NIP-07, local nsec, ephemeral)                   │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│ Phase 2: MVP Chat Surface — Single Channel (2.5 days) [BUZZ IN BROWSER]│
│ - /channels/$channelId TanStack route                                  │
│ - Message timeline, markdown bubbles, message composer                 │
│ - Bidirectional real-time messaging with desktop & CLI clients         │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│ Phase 3: Channel Navigation, Sidebar & Discovery (2.0 days)            │
│ - Multi-channel sidebar layout (/channels layout route)                │
│ - Channel metadata queries (kind 39000), channel switcher              │
│ - Header with member count & connection status badge                   │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│ Phase 4: Thread Replies, Reactions & Optimistic UI (2.0 days)          │
│ - Kind-7 reaction bar & aggregate display                              │
│ - Kind-9 NIP-10 thread reply view & badges                             │
│ - Optimistic message sending with status ticks                         │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────────┐
│ Phase 5: PWA App Shell, Offline Cache & Polish (2.0 days)              │
│ - Web App Manifest, ServiceWorker offline caching                      │
│ - IndexedDB recent message cache                                       │
│ - Production build verification & E2E smoke tests                      │
└────────────────────────────────────────────────────────────────────────┘
```

---

## Phase 0: Relay Route Gate & Kind Codegen Sync

### Scope & Tasks
1. **Relay Route Allowlist:**
   * Modify `crates/buzz-relay/src/router.rs` to allow `/channels`, `/channels/*`, `/dms`, `/dms/*`, `/settings`, `/members`.
   * Update `crates/buzz-relay/src/config.rs` to add `BUZZ_SERVE_WEB_CLIENT` / `serve_web_client`.
   * Add unit tests in `crates/buzz-relay/src/router.rs` asserting all web chat routes return the SPA index when configured.
2. **Kind Codegen Sync:**
   * Integrate `web/scripts/gen-kinds.mjs` into `package.json` (`pnpm gen:kinds` / `pnpm check:kinds`).
   * Add chat kinds to `WANTED`: `KIND_CHANNEL_MESSAGE` (9), `KIND_EPHEMERAL_CHAT` (40002), `KIND_REACTION` (7), `KIND_CHANNEL_METADATA` (39000), `KIND_AUTH` (22242).
   * Generate `web/src/shared/constants/kinds.ts`.

### Files Modified / Added
* `crates/buzz-relay/src/router.rs`
* `crates/buzz-relay/src/config.rs`
* `web/scripts/gen-kinds.mjs`
* `web/src/shared/constants/kinds.ts`
* `web/package.json`

### Effort Estimate
**0.5 working days.** (Rust router modification is ~40 lines + test updates; script and codegen is ~80 lines).

### DONE-WHEN
* `cargo test -p buzz-relay` passes with new route tests.
* Running `pnpm -C web check:kinds` exits with status 0.
* Starting the relay with `BUZZ_WEB_DIR=web/dist` and curling `http://localhost:3000/channels/lobby` with `Accept: text/html` returns HTTP 200 with `index.html`.

---

## Phase 1: Real-Time Protocol & Data Layer (`BuzzClient` in `web/`)

### Scope & Tasks
1. **Port `BuzzClient`:**
   * Create `web/src/shared/lib/buzz-client.ts` adapted from `biosv3/src/lib/buzz/client.ts`.
   * Add exponential backoff reconnection logic.
   * Enforce `#h` channel tag scoping and explicit kind list on all subscriptions.
   * Wire `relayUrl` resolution using `web/src/shared/lib/relay-url.ts`.
2. **Unified `Signer` Seam:**
   * Update `web/src/shared/lib/nostr-signer.ts` to export a clean `Signer` interface.
   * Implement `Nip07Signer`, `LocalKeySigner` (with `localStorage` persistence under `buzz:nsec`), and `EphemeralSigner`.
3. **Unit Test Suite:**
   * Port `biosv3/src/lib/buzz/client.test.ts` to `web/src/shared/lib/__tests__/buzz-client.test.ts`.
   * Verify mock WebSocket events, EOSE replay, auth challenge-response, publish ack, and subscription cleanup.

### Files Modified / Added
* `web/src/shared/lib/buzz-client.ts`
* `web/src/shared/lib/nostr-signer.ts`
* `web/src/shared/lib/nostr-types.ts`
* `web/src/shared/lib/__tests__/buzz-client.test.ts`

### Effort Estimate
**1.5 working days.** (Core client adaptation: 1 day; test suite port & signer integration: 0.5 days).

### DONE-WHEN
* `pnpm -C web test` executes all `buzz-client.test.ts` test cases green (covering connection, AUTH, `#h` filter, EOSE, and publish).
* Biome lint passes with zero errors: `pnpm -C web lint`.

---

## Phase 2: MVP Chat Surface — Single Channel (The "Buzz in a Browser" Milestone)

### Scope & Tasks
1. **Chat Route Definition:**
   * Create `web/src/app/routes/channels.$channelId.tsx` and register route in `web/src/app/routes.ts`.
   * Run router codegen to update `routeTree.gen.ts`.
2. **Channel Timeline UI (`features/chat/ui/ChannelTimeline.tsx`):**
   * Render chronologically sorted message list with autoscroll to bottom on new events.
   * Extract and adapt `MessageBubble.tsx` from `biosv3` with markdown formatting (bold, italics, code blocks).
   * Format author pubkey and timestamp using `web/src/shared/lib/pubkey.ts` and `relative-time.ts`.
3. **Message Composer UI (`features/chat/ui/MessageComposer.tsx`):**
   * Text input with auto-growing textarea, Enter to submit, Shift+Enter for newline.
   * Disabled state during signature/publish with inline error toast on reject.
4. **Active Client Context:**
   * Create `ChatClientProvider` context to supply the active `BuzzClient` and `Signer` across the UI tree.

### Files Modified / Added
* `web/src/app/routes/channels.$channelId.tsx`
* `web/src/app/routes.ts`
* `web/src/app/routeTree.gen.ts`
* `web/src/features/chat/context/ChatClientContext.tsx`
* `web/src/features/chat/ui/ChannelTimeline.tsx`
* `web/src/features/chat/ui/MessageBubble.tsx`
* `web/src/features/chat/ui/MessageComposer.tsx`

### Effort Estimate
**2.5 working days.** (Route wiring + Context: 0.5 days; Timeline & Markdown: 1.0 days; Composer & Publishing flow: 1.0 days).

### DONE-WHEN
* A user can open `http://localhost:3000/channels/lobby`, send a message "Hello from Web", and immediately see it appear in the Buzz desktop app in `#lobby`.
* Messages sent from `buzz-cli` (`buzz message send --channel lobby "Hello from CLI"`) stream into the browser tab in real time without refreshing.

---

## Phase 3: Channel Navigation, Sidebar & Discovery

### Scope & Tasks
1. **Chat Layout & Sidebar (`web/src/app/routes/channels.tsx`):**
   * Master-detail layout: Channel sidebar on the left (collapsible on mobile), active channel view on the right.
   * Header showing channel name, topic, member count, and connection status indicator (`connected`, `reconnecting`, `offline`).
2. **Channel Discovery & Metadata:**
   * Query channel metadata events (kind 39000) on mount to populate the channel list.
   * Channel switcher with active route highlight (`bg-accent` / `text-accent-foreground`).
3. **Empty States & Routing Fallbacks:**
   * `/channels` root index route automatically redirects to `/channels/general` (or first available channel).

### Files Modified / Added
* `web/src/app/routes/channels.tsx`
* `web/src/app/routes/channels.index.tsx`
* `web/src/features/chat/ui/ChannelSidebar.tsx`
* `web/src/features/chat/ui/ChannelHeader.tsx`
* `web/src/features/chat/hooks/useChannelList.ts`

### Effort Estimate
**2.0 working days.** (Layout & Responsive Sidebar: 1.0 days; Metadata queries & switching logic: 1.0 days).

### DONE-WHEN
* Sidebar displays list of public channels retrieved from the relay.
* Clicking between channels in the sidebar changes the URL (`/channels/random`, `/channels/engineering`) and instantly switches the active subscription and timeline.

---

## Phase 4: Thread Replies, Reactions & Optimistic UI

### Scope & Tasks
1. **Reactions (Kind 7):**
   * Message hover action bar with quick emoji reactions (👍, ❤️, 🚀, 🎉).
   * Aggregate kind-7 events by message ID and display reaction badge pills with counts.
2. **Thread Reply View (Kind 9 with `e` marker):**
   * NIP-10 thread tagging: reply button sets parent event ID.
   * Thread side-panel / flyout displaying the thread conversation root and replies.
3. **Optimistic Message UI:**
   * Immediately append sent messages to the local timeline with a pending spinner, confirmed upon relay `OK` ack.

### Files Modified / Added
* `web/src/features/chat/ui/ReactionPicker.tsx`
* `web/src/features/chat/ui/ReactionPills.tsx`
* `web/src/features/chat/ui/ThreadPanel.tsx`
* `web/src/features/chat/hooks/useReactions.ts`
* `web/src/features/chat/hooks/useThread.ts`

### Effort Estimate
**2.0 working days.** (Reactions aggregate & publish: 1.0 days; Thread flyout & NIP-10 wiring: 1.0 days).

### DONE-WHEN
* Clicking a reaction adds the emoji pill with count 1; clicking it from desktop updates the web view to count 2 live.
* Replying in a thread attaches the root `e` tag and opens the reply panel correctly.

---

## Phase 5: PWA App Shell, Offline Cache & Polish

### Scope & Tasks
1. **PWA Assets & Manifest:**
   * Configure `manifest.webmanifest` (app name, theme color, icons from `web/src/assets/`).
   * Add service worker for offline app-shell caching (HTML/JS/CSS assets).
2. **IndexedDB Message Cache:**
   * Cache recent channel timeline events in IndexedDB for instant render on reload before WebSocket connects.
3. **Full Build & E2E Validation:**
   * Run full check: `pnpm check`, `pnpm build`, `cargo test -p buzz-relay`.
   * Add Playwright web E2E test verifying channel navigation, message publishing, and reaction rendering.

### Files Modified / Added
* `web/public/manifest.webmanifest`
* `web/src/service-worker.ts`
* `web/src/shared/lib/indexeddb-cache.ts`
* `web/tests/e2e/chat.spec.ts`

### Effort Estimate
**2.0 working days.** (PWA manifest & ServiceWorker: 0.5 days; IndexedDB cache: 0.75 days; E2E tests & cleanup: 0.75 days).

### DONE-WHEN
* The web app is installable as a standalone PWA on Chromium browsers.
* Opening the app in airplane mode displays cached messages and a graceful "Offline — Reconnecting" banner.
* Full workspace CI (`just ci`) passes without warnings or failures.

---

## Effort & Schedule Rollup

| Phase | Description | Days | Cumulative Days | Shippable Artifact |
|---|---|---|---|---|
| **Phase 0** | Relay Route Gate & Kind Codegen | 0.5 | 0.5 | Rust relay serves `/channels/*`; kinds synced |
| **Phase 1** | Real-Time Protocol & Data Layer | 1.5 | 2.0 | Tested `BuzzClient` with WebSocket & AUTH in `web/` |
| **Phase 2** | MVP Chat Surface (Single Channel) | 2.5 | 4.5 | **Working Web Chat Client (Milestone 1)** |
| **Phase 3** | Channel Navigation & Sidebar | 2.0 | 6.5 | Full multi-channel workspace shell |
| **Phase 4** | Threading, Reactions & Optimistic UI | 2.0 | 8.5 | Rich chat interactions |
| **Phase 5** | PWA App Shell, Cache & Polish | 2.0 | 10.5 | Installable PWA with offline caching & CI E2E |
| **Total** | **End-to-End Delivery** | **10.5 days** | **10.5 days** | **Production-Ready Web Chat Client** |
