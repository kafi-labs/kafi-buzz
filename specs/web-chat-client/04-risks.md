# Risks, Failure Modes & Open Decisions

**Status:** Risk Assessment & Governance  
**Date:** September 2026  

---

## 1. Technical & Architectural Failure Modes

### Risk 1: Relay Route 404 Gate Desync
* **Failure Scenario:** A developer implements new frontend routes in TanStack Router (e.g. `/channels/lobby`, `/settings`). The routes work flawlessly in `pnpm dev` (Vite dev server) and pass standalone frontend tests because Vite serves `index.html` for all paths. However, once deployed behind `buzz-relay`, refreshing the page returns `404 Not Found` because `crates/buzz-relay/src/router.rs` has a strict SPA allowlist.
* **Impact:** High. Breaks direct navigation, bookmarking, and link sharing in production.
* **Mitigation:**
  1. Execute Phase 0 (Relay Route Gate update) FIRST.
  2. Add an automated Rust test in `crates/buzz-relay/tests/` asserting that every client route returns `200 OK` with `index.html`.
  3. Include a full-stack integration test in `crates/buzz-test-client` testing direct URL navigation against a live relay binary.

### Risk 2: P-Gate 403 Rejections (Unscoped Query Filters)
* **Failure Scenario:** A frontend component issues a REQ query omitting the `kinds` array (e.g. `["REQ", "sub", { "#h": ["lobby"] }]`). The relay treats open-ended queries as an authorization breach, triggers the p-gate check, and closes the subscription with a 403 error.
* **Impact:** Critical. The channel timeline fails to load silently.
* **Mitigation:**
  1. Strict TypeScript typing in `NostrFilter`: make `kinds: number[]` a mandatory property in `buzz-client.ts` subscription APIs.
  2. The `BuzzClient.subscribeTimeline` method internally injects `[KINDS.CHANNEL_MESSAGE, KINDS.EPHEMERAL_CHAT, KINDS.REACTION]` unconditionally.

### Risk 3: Tag Scoping Confusion (`h` vs `e` Tags)
* **Failure Scenario:** A developer writes code using `["e", channelId]` for channel scoping instead of `["h", channelId]`. In NIP-29, `h` represents the group/channel ID, whereas `e` represents event/message IDs (e.g. thread root/reply).
* **Impact:** High. Messages published with `e` tags will not be indexed as channel messages by the relay, causing them to disappear from the channel feed.
* **Mitigation:**
  1. Use helper builder functions (`buildKind9Event`) exclusively; ban manual event construction in UI components.
  2. Include lint rules / type checks ensuring `h` tag presence on all kind-9 event payloads.

### Risk 4: Browser Key Orphaning & Ephemeral Identity Churn
* **Failure Scenario:** If the web app uses purely in-memory ephemeral keys, every browser refresh creates a new Nostr pubkey. The user loses their identity, mentions fail, and membership checks break. Conversely, if we mandate a NIP-07 browser extension (Alby/nos2x), conversion drops for non-crypto users.
* **Impact:** Medium-High. Poor onboarding or broken session continuity.
* **Mitigation:**
  1. Implement a three-tier signer strategy:
     - Tier 1: NIP-07 extension if available (`window.nostr`).
     - Tier 2: Persisted browser keypair generated on first visit and stored in `localStorage` under `buzz:nsec`.
     - Tier 3: In-memory ephemeral key for guest read-only access.
  2. Provide an explicit "Export Key / Import Key" dialog in `/settings` so users can back up or transfer their browser identity.

### Risk 5: The Desktop Parity Trap (Scope Explosion)
* **Failure Scenario:** The team attempts to port all desktop chat features (69,137 LOC: voice recording, nested thread flyouts, rich text editor, blossom video player, agent observer frames) into the first web client release.
* **Impact:** Critical. Stalls the project for months and produces an unstable web client.
* **Mitigation:**
  1. Strictly bound Phase 2 (MVP) to: Read & Write Kind-9 Messages in a Channel.
  2. Treat all advanced features (threads, voice, video, agent metrics) as incremental layers according to the phased roadmap.

---

## 2. Open Questions Requiring Owner Decision

Below are the 5 architectural forks that require explicit product/owner direction:

### Question 1: Default Landing Route (`/`)
* **Context:** Today, when `BUZZ_SERVE_GIT_WEB_GUI` is true, navigating to `https://relay.example.com/` displays the Git Repositories index page. Once the web chat client lands, what should the root URL `/` serve?
* **Options:**
  1. **Option A (Chat First):** `/` redirects to `/channels/general` (or the first available channel). Repos are accessible via a `/repos` link in the top bar. (Recommended for community workspaces).
  2. **Option B (Workspace Picker):** `/` displays a welcome dashboard with quick links to Channels, Repositories, and Settings.
  3. **Option C (Configurable):** Controlled by an environment variable (`BUZZ_WEB_DEFAULT_ROUTE=/channels`).
* **Recommendation:** Option A (Chat First) — matches desktop behavior where opening the app lands directly in the default channel.

### Question 2: Identity Model for Non-Extension Users
* **Context:** Power users will use NIP-07 extensions, but mainstream users will not have extensions installed.
* **Options:**
  1. **Option A (Local Storage nsec):** Generate a keypair on first visit, persist in `localStorage`, and use for all signing. Add an export banner. (Recommended for OSS self-hosted).
  2. **Option B (Extension Required for Write):** Require NIP-07 to post; allow ephemeral keys for read-only.
* **Recommendation:** Option A — gives a frictionless, Slack-like web experience out of the box.

### Question 3: Custodial Keys & NIP-46 (Tower OIDC) Timeline
* **Context:** `biosv3` designed a custodial key model using Tower OIDC + NIP-46 remote signing for Indonesian SMBs.
* **Options:**
  1. **Option A (Defer to Phase 6+):** Build the OSS client with NIP-07 + LocalKeySigner first; leave the `Signer` interface open for NIP-46 remote signers later.
  2. **Option B (Include in Milestone 1):** Build the NIP-46 client handler in Phase 1.
* **Recommendation:** Option A — ship core web chat first with zero external dependencies; add NIP-46/Tower as a pluggable `Signer` implementation in a dedicated follow-up.

### Question 4: PWA Web Push Notifications Strategy
* **Context:** The relay has a `buzz-push-gateway` pipeline configured for mobile. Web push requires VAPID key pairs and ServiceWorker push subscriptions.
* **Options:**
  1. **Option A (Standard Web Push in Phase 5):** Implement standard Web Push API in ServiceWorker connecting to relay push endpoints.
  2. **Option B (Defer Web Push):** Ship web client with in-app audio/badge notifications; defer background OS web push to a dedicated notification epic.
* **Recommendation:** Option B for Phase 1–4, evaluating Option A in Phase 5 based on mobile push gateway maturity.

### Question 5: Handling Kafi / Intel Custom Event Payloads (e.g. Storyboard Cards)
* **Context:** `biosv3` includes UI for `id.kafi.storyboard` JSON payloads. Should `block/buzz`'s `web/` client support custom card renderers?
* **Options:**
  1. **Option A (Pluggable Message Attachments):** `MessageBubble.tsx` includes an extensible custom payload registry where plugins can register custom card renderers for specific JSON tags or kinds.
  2. **Option B (Pure Markdown in Core):** Core `web/` renders standard markdown + code blocks; custom product cards remain in specialized forks or plugins.
* **Recommendation:** Option A — provides a clean extension seam for Kafi and future agent-generated UI components without cluttering core code.
