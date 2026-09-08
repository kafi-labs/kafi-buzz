# Target Architecture: Web Chat Client

**Status:** Technical Specification  
**Date:** September 2026  

---

## 1. System Architecture Overview

The Buzz Web Client transforms `web/` from a single-purpose repository viewer into a first-class, protocol-native web application for humans and agents. The application runs entirely within the user's browser, communicating directly with the `buzz-relay` over WebSocket and HTTP.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Browser Application (`web/` - Vite + React 19 + TanStack Router)            │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Route Tree: /, /invite/$code, /repos/*, /channels, /channels/$channelId │  │
│  └──────────────────────────────────┬────────────────────────────────────┘  │
│                                     │                                       │
│  ┌──────────────────────────────────┴────────────────────────────────────┐  │
│  │ Feature Layer: `features/chat/` (ChannelView, Timeline, Composer)     │  │
│  │ Feature Layer: `features/repos/` (RepoTree, BlobViewer, Commits)      │  │
│  └──────────────────────────────────┬────────────────────────────────────┘  │
│                                     │                                       │
│  ┌──────────────────────────────────┴────────────────────────────────────┐  │
│  │ Protocol & Data Layer: `BuzzClient` (Singleton per relay)             │  │
│  │  - WebSocket connection state machine (idle, connecting, ready, etc.) │  │
│  │  - Multiplexed NIP-01 REQ subscriptions with `#h` channel scoping     │  │
│  │  - NIP-42 AUTH challenge-response handler                             │  │
│  │  - Outgoing event publisher with `OK` promise settlement              │  │
│  └──────────────────────────────────┬────────────────────────────────────┘  │
│                                     │                                       │
│  ┌──────────────────────────────────┴────────────────────────────────────┐  │
│  │ Identity & Signing Seam: `Signer` interface                           │  │
│  │  - Nip07Signer (window.nostr extension)                               │  │
│  │  - LocalKeySigner (persisted nsec in localStorage)                    │  │
│  │  - EphemeralSigner (page-lifetime key for read-only / guest)          │  │
│  │  - [Future] RemoteSigner (NIP-46 / Tower OIDC custodial key)          │  │
│  └──────────────────────────────────┬────────────────────────────────────┘  │
└─────────────────────────────────────┼───────────────────────────────────────┘
                                      │ WebSocket (NIP-01, NIP-42, NIP-29)
                                      │ HTTP Bridge (NIP-98, Blossom)
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ `buzz-relay` (Axum Rust Server on port 3000)                                │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │ Axum Router & SPA Fallback Gate (`router.rs`)                          │  │
│  │  - Serves API routes (`/events`, `/query`, `/upload`, etc.)            │  │
│  │  - Upgrades WebSocket at `/` (with host-bound community lookup)       │  │
│  │  - Serves static assets (`/assets/*`) from `BUZZ_WEB_DIR`             │  │
│  │  - Serves `index.html` on client routes (`/channels/*`, `/repos/*`)   │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Next.js to Vite + TanStack Router Integration

### The Framework Gap
`biosv3` utilized Next.js 15 App Router server/client component boundaries (`"use client"`), Next.js file-system routing conventions (`src/app/page.tsx`), and Next.js environment injection (`process.env.NEXT_PUBLIC_*`).

In contrast, `web/` uses:
* **Bundler & Dev Server:** Vite (`vite.config.ts`), with `@vitejs/plugin-react` and `vite-tsconfig-paths`.
* **Routing:** `@tanstack/react-router` with code-based route definition in `src/app/routes.ts` generating `routeTree.gen.ts`.
* **Environment:** `import.meta.env.VITE_*` (with fallbacks to relative URLs).
* **Linter & Formatter:** Biome (`biome.json`), strictly configured.

### Pricing the Porting Cost
Attempting to keep Next.js would require maintaining two completely separate frontend builds or migrating `web/` to Next.js. That would break the existing lightweight static build that the Rust relay serves via `ServeDir`.

**Decision:** Maintain Vite + TanStack Router. Port the TypeScript logic into TanStack Router structures.
* **Cost:** Low for the protocol layer (pure TypeScript, zero Next.js dependencies).
* **Cost:** Moderate for the UI layer (~2.5 days to build clean TanStack Route components and layouts matching `web/`'s UI kit).

### Route Structure
The router in `web/src/app/routes.ts` will expand as follows:

```typescript
// web/src/app/routes.ts
import { route, rootRoute, index } from "@tanstack/virtual-file-routes";

export const routes = rootRoute("root.tsx", [
  index("index.tsx"),                       // Workspace home / channel redirect
  route("/invite/$code", "invite.$code.tsx"), // Invite accept flow
  route("/repos", "repos.tsx", [            // Git repo explorer
    index("repos.index.tsx"),
    route("/$repoId", "repos.$repoId.tsx", [
      index("repos.$repoId.index.tsx"),
      route("/blob/$", "repos.$repoId.blob.$.tsx"),
    ]),
  ]),
  route("/channels", "channels.tsx", [      // Chat client shell (sidebar + layout)
    index("channels.index.tsx"),            // Default channel / channel browser
    route("/$channelId", "channels.$channelId.tsx"), // Active channel view
  ]),
  route("/settings", "settings.tsx"),       // Identity, relay & UI settings
]);
```

---

## 3. Live Subscription & Protocol Layer (`BuzzClient`)

`web/`'s existing `nostr-client.ts` is strictly one-shot: it opens a WebSocket, queries historical events until `EOSE`, and immediately closes. A chat client requires an active, persistent connection.

We adapt `biosv3`'s `BuzzClient` into `web/src/shared/lib/buzz-client.ts` with the following architectural guarantees:

### A. Connection Lifecycle & State Machine
The client transitions through explicit states:
`idle` ➔ `connecting` ➔ `authenticating` ➔ `connected` ➔ `disconnected` / `error`

```
  ┌────────┐
  │  idle  │
  └───┬────┘
      │ connect()
      ▼
┌────────────┐
│ connecting │
└─────┬──────┘
      │ ws.onopen
      ▼
┌────────────────┐  AUTH challenge received
│ authenticating ├─────────────────────────┐
└─────┬──────────┘                         │
      │ 100ms grace (open relay)           │ signAuthEvent() & send ["AUTH", ...]
      ▼                                    ▼
┌───────────┐                       ┌──────────────┐
│ connected │◄──────────────────────┤  wait for OK │
└─────┬─────┘    OK received (true) └──────────────┘
      │
      │ ws.onclose / error / disconnect()
      ▼
┌──────────────┐
│ disconnected │ (Triggers reconnect with exponential backoff)
└──────────────┘
```

* **AUTH Challenge-Response:** On receiving `["AUTH", challenge]`, the client constructs a kind-22242 event referencing the relay URL and challenge string, requests a signature from the configured `Signer`, and transmits `["AUTH", signedEvent]`.
* **AUTH Re-Challenge Handling:** If the relay re-challenges during an active session, the client handles the challenge without tearing down active `REQ` subscriptions.
* **Automatic Reconnect with Backoff:** On unexpected socket closure, the client initiates reconnection using jittered exponential backoff (starting at 500ms, doubling up to a 10s maximum). Upon reconnection, all registered subscriptions in `this.subs` are automatically re-sent with `sendReq(subId, filter)`.
* **Clean Teardown:** Calling `disconnect()` immediately closes all active subscriptions with `["CLOSE", subId]`, rejects pending publish promises with a descriptive error, and closes the WebSocket.

### B. NIP-29 Group & Channel Scoping Discipline
* **Filter Scoping:** All channel timeline requests MUST scope via `#h` tags:
  ```json
  ["REQ", "tl-123", { "kinds": [9, 40002, 7], "#h": ["lobby"], "limit": 50 }]
  ```
* **Explicit Kinds:** Queries MUST explicitly enumerate kinds `[9, 40002, 7]`. Sending an open filter without `kinds` triggers the relay's p-gate security check and results in an immediate 403 / `CLOSED` rejection.
* **Kind Constants Source of Truth:** Kinds are imported from `web/src/shared/constants/kinds.ts` (generated from `crates/buzz-core/src/kind.rs` via `gen-kinds.mjs`).

### C. Timeline Deduplication & EOSE Handling
* Each subscription maintains an event buffer. Incoming `["EVENT", subId, event]` payloads are deduplicated by `event.id`.
* When `["EOSE", subId]` is received, historical loading is complete, marking the channel as live.
* Late-joining components subscribing to an existing channel subscription immediately receive a replay of buffered events before streaming live events.

---

## 4. Signing & Identity Architecture

A web chat client must support diverse user personas: power users with browser extensions, regular users wanting seamless browser persistence, and first-time guests.

```typescript
// web/src/shared/lib/nostr-signer.ts

export interface Signer {
  getPublicKey(): Promise<string>;
  signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent>;
  readonly isReadOnly?: boolean;
  readonly type: "nip07" | "local" | "ephemeral" | "remote";
}
```

### Signer Implementations:
1. **`Nip07Signer` (`type: "nip07"`):**
   * Uses `window.nostr` (Alby, nos2x, etc.).
   * Primary choice for sovereign desktop/browser users.
2. **`LocalKeySigner` (`type: "local"`):**
   * Stores a generated `nsec` in `localStorage` under `buzz:nsec`.
   * Allows persistent identity across reloads without requiring any browser extension.
   * Ideal for zero-friction web onboarding.
3. **`EphemeralSigner` (`type: "ephemeral"`):**
   * Keeps a generated `Uint8Array` secret key in memory for the life of the page.
   * Safe fallback for anonymous reading on open relays.
4. **`RemoteSigner` (`type: "remote"` - Future Seam):**
   * Implements NIP-46 client-side remote signing protocol (for custodial keys, Tower OIDC integration).

---

## 5. Server-Side Relay Route Gate (`crates/buzz-relay`)

### The Hard Production Gate
The Rust relay does not blindly serve `index.html` for arbitrary 404 paths. In `crates/buzz-relay/src/router.rs`, the fallback service explicitly gates SPA serving via `should_serve_spa(path, serve_git_web_gui)`:

```rust
// CURRENT IMPLEMENTATION (crates/buzz-relay/src/router.rs)
fn is_invite_landing_path(path: &str) -> bool {
    path.strip_prefix("/invite/")
        .is_some_and(|code| !code.is_empty() && !code.contains('/'))
}

fn is_git_web_gui_path(path: &str) -> bool {
    path == "/" || path == "/repos" || path.starts_with("/repos/")
}

fn should_serve_spa(path: &str, serve_git_web_gui: bool) -> bool {
    is_invite_landing_path(path) || (serve_git_web_gui && is_git_web_gui_path(path))
}
```

If a user navigates directly to `https://relay.example.com/channels/lobby`, the browser makes a GET request for `/channels/lobby`. Because `/channels/lobby` is not an invite path or a git GUI path, the relay returns **404 Not Found**.

### Required Relay Changes

#### 1. `crates/buzz-relay/src/config.rs`:
* Add `serve_web_client: bool` to `RelayConfig`.
* Parse `BUZZ_SERVE_WEB_CLIENT` (default `true` when `web_dir` is set, or aliased with `BUZZ_SERVE_GIT_WEB_GUI` for backwards compatibility).

#### 2. `crates/buzz-relay/src/router.rs`:
* Expand path detection to recognize web client paths:
  ```rust
  fn is_web_client_path(path: &str) -> bool {
      path == "/"
          || path == "/channels"
          || path.starts_with("/channels/")
          || path == "/dms"
          || path.starts_with("/dms/")
          || path == "/settings"
          || path == "/members"
          || is_git_web_gui_path(path)
  }

  fn should_serve_spa(path: &str, serve_web_client: bool) -> bool {
      is_invite_landing_path(path) || (serve_web_client && is_web_client_path(path))
  }
  ```
* Update `nip11_or_ws_handler` to serve `index.html` on HTML requests to `/` when `serve_web_client` is enabled.
* Update unit tests in `crates/buzz-relay/src/router.rs` to assert `/channels/general` and `/settings` are served.
