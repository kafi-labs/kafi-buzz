/**
 * Persistent browser client for a Buzz relay.
 *
 * `nostr-client.ts` is strictly one-shot (open → REQ → EOSE → close). Chat
 * needs a connection that stays open: multiplexed long-lived REQ
 * subscriptions, an in-session AUTH re-challenge handled without tearing
 * down those subscriptions, and reconnection with jittered exponential
 * backoff after an unexpected close.
 */

import { makeAuthEvent } from "nostr-tools/nip42";
import { KINDS } from "@/shared/constants/kinds";
import type { Signer } from "./nostr-signer";
import type {
  ConnectionListener,
  ConnectionState,
  NostrEvent,
  NostrFilter,
  PublishAck,
  TimelineListener,
  Unsub,
} from "./nostr-types";
import { relayWsUrl } from "./relay-url";

export interface BuzzClientOptions {
  /** WebSocket relay URL. Defaults to `relayWsUrl()` (same-origin, or `VITE_RELAY_URL`). */
  relayUrl?: string;
  signer: Signer;
  /** Milliseconds to wait for an AUTH challenge before sending an unauthenticated REQ. Default 100. */
  authGraceMs?: number;
  /** Minimum reconnect backoff delay in milliseconds. Default 500. */
  minReconnectDelayMs?: number;
  /** Maximum reconnect backoff delay in milliseconds. Default 10_000. */
  maxReconnectDelayMs?: number;
  /** Factory for tests — inject a mock WebSocket. */
  webSocketFactory?: (url: string) => WebSocket;
}

/**
 * Channel timeline kinds. Fixed and explicit by construction — a filter
 * without `kinds` trips the relay's p-gate (403). Never build a timeline
 * filter without this set.
 */
const TIMELINE_KINDS: readonly number[] = [
  KINDS.STREAM_MESSAGE,
  KINDS.STREAM_MESSAGE_V2,
  KINDS.REACTION,
];

interface ActiveSub {
  subId: string;
  filter: NostrFilter;
  listeners: Set<TimelineListener>;
  eoseListeners: Set<() => void>;
  events: NostrEvent[];
  eose: boolean;
}

let subCounter = 0;
function nextSubId(prefix: string): string {
  subCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${subCounter}`;
}

/**
 * Build an unsigned kind-9 channel message. Channel scoping is via the `h`
 * tag (NIP-29); replies use a NIP-10 `e` marker tag.
 */
function buildChannelMessageEvent(opts: {
  channelId: string;
  content: string;
  replyToId?: string;
}): { kind: number; created_at: number; tags: string[][]; content: string } {
  const tags: string[][] = [["h", opts.channelId]];
  if (opts.replyToId) {
    tags.push(["e", opts.replyToId, "", "reply"]);
  }
  return {
    kind: KINDS.STREAM_MESSAGE,
    created_at: Math.floor(Date.now() / 1000),
    tags,
    content: opts.content,
  };
}

export class BuzzClient {
  private readonly relayUrl: string;
  private readonly signer: Signer;
  private readonly authGraceMs: number;
  private readonly minReconnectDelayMs: number;
  private readonly maxReconnectDelayMs: number;
  private readonly webSocketFactory: (url: string) => WebSocket;

  private ws: WebSocket | null = null;
  private state: ConnectionState = "idle";
  private authEventId: string | null = null;
  private unauthenticatedReqTimer: ReturnType<typeof setTimeout> | null = null;
  private connectPromise: Promise<void> | null = null;
  private connectResolve: (() => void) | null = null;
  private connectReject: ((err: Error) => void) | null = null;
  private explicitDisconnect = false;
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  private readonly subs = new Map<string, ActiveSub>();
  private readonly channelSubs = new Map<string, string>(); // channelId → subId
  private readonly pendingPublish = new Map<
    string,
    { resolve: (ack: PublishAck) => void; reject: (err: Error) => void }
  >();
  private readonly connectionListeners = new Set<ConnectionListener>();

  constructor(options: BuzzClientOptions) {
    this.relayUrl = options.relayUrl ?? relayWsUrl();
    this.signer = options.signer;
    this.authGraceMs = options.authGraceMs ?? 100;
    this.minReconnectDelayMs = options.minReconnectDelayMs ?? 500;
    this.maxReconnectDelayMs = options.maxReconnectDelayMs ?? 10_000;
    this.webSocketFactory =
      options.webSocketFactory ?? ((url) => new WebSocket(url));
  }

  getConnectionState(): ConnectionState {
    return this.state;
  }

  onConnectionChange(listener: ConnectionListener): Unsub {
    this.connectionListeners.add(listener);
    return () => {
      this.connectionListeners.delete(listener);
    };
  }

  private setState(state: ConnectionState, detail?: string): void {
    this.state = state;
    for (const listener of this.connectionListeners) {
      try {
        listener(state, detail);
      } catch {
        // listener errors must not break the client
      }
    }
  }

  /**
   * Open the WebSocket, complete NIP-42 AUTH if challenged, resolve when
   * ready. Safe to call while already connected or connecting — returns the
   * same in-flight/settled promise.
   */
  connect(): Promise<void> {
    if (this.state === "connected" && this.ws?.readyState === WebSocket.OPEN) {
      return Promise.resolve();
    }
    if (this.connectPromise) {
      return this.connectPromise;
    }

    // A fresh connect() call (initial or caller-initiated) re-arms
    // reconnect-on-close; only disconnect() should permanently disable it.
    this.explicitDisconnect = false;

    this.connectPromise = new Promise<void>((resolve, reject) => {
      this.connectResolve = resolve;
      this.connectReject = reject;
      this.authEventId = null;
      this.setState("connecting");

      try {
        this.ws = this.webSocketFactory(this.relayUrl);
      } catch (err) {
        this.failConnect(
          err instanceof Error
            ? err
            : new Error("WebSocket construction failed"),
        );
        if (!this.explicitDisconnect) {
          this.scheduleReconnect();
        }
        return;
      }

      this.ws.addEventListener("open", () => {
        this.setState("authenticating");
        // Wait briefly for AUTH challenge; open relays may not send one.
        this.unauthenticatedReqTimer = setTimeout(() => {
          this.unauthenticatedReqTimer = null;
          this.markReady();
        }, this.authGraceMs);
      });

      this.ws.addEventListener("message", (msg) => {
        void this.handleMessage(String(msg.data));
      });

      this.ws.addEventListener("error", () => {
        this.failConnect(new Error("WebSocket connection failed"));
      });

      this.ws.addEventListener("close", () => {
        const wasConnecting = this.connectResolve != null;
        this.ws = null;
        this.clearAuthTimer();
        this.connectPromise = null;
        if (wasConnecting) {
          this.failConnect(new Error("WebSocket closed before ready"));
        } else {
          this.setState("disconnected");
        }
        if (!this.explicitDisconnect) {
          this.scheduleReconnect();
        }
      });
    });

    return this.connectPromise;
  }

  /**
   * Tear down: reject in-flight publishes, drop subscriptions, close the
   * socket, and cancel any pending reconnect. Terminal until `connect()` (or
   * a subscribe/publish call) is invoked again.
   */
  disconnect(): void {
    this.explicitDisconnect = true;
    this.clearAuthTimer();
    this.clearReconnectTimer();
    this.reconnectAttempt = 0;
    for (const [id, pending] of this.pendingPublish) {
      pending.reject(new Error("Client disconnected"));
      this.pendingPublish.delete(id);
    }
    this.subs.clear();
    this.channelSubs.clear();
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        // ignore
      }
      this.ws = null;
    }
    this.connectPromise = null;
    this.connectResolve = null;
    this.connectReject = null;
    this.setState("disconnected");
  }

  /**
   * Subscribe to a channel timeline: explicit kinds, `#h` = [channelId].
   * Historical EVENTs arrive until EOSE, then live EVENTs stream. The
   * subscription is automatically re-sent after a reconnect.
   */
  subscribeTimeline(
    channelId: string,
    onEvent: TimelineListener,
    options?: { limit?: number; onEose?: () => void },
  ): Unsub {
    const existingSubId = this.channelSubs.get(channelId);
    if (existingSubId) {
      const sub = this.subs.get(existingSubId);
      if (sub) {
        sub.listeners.add(onEvent);
        if (options?.onEose) {
          if (sub.eose) {
            options.onEose();
          } else {
            sub.eoseListeners.add(options.onEose);
          }
        }
        // Replay buffered historical events for late joiners.
        for (const ev of sub.events) {
          onEvent(ev);
        }
        return () => {
          sub.listeners.delete(onEvent);
          if (options?.onEose) sub.eoseListeners.delete(options.onEose);
          if (sub.listeners.size === 0) {
            this.closeSub(existingSubId);
            this.channelSubs.delete(channelId);
          }
        };
      }
    }

    const subId = nextSubId("tl");
    const filter: NostrFilter = {
      kinds: [...TIMELINE_KINDS],
      "#h": [channelId],
      limit: options?.limit ?? 50,
    };
    const sub: ActiveSub = {
      subId,
      filter,
      listeners: new Set([onEvent]),
      eoseListeners: new Set(options?.onEose ? [options.onEose] : []),
      events: [],
      eose: false,
    };
    this.subs.set(subId, sub);
    this.channelSubs.set(channelId, subId);

    if (this.state === "connected") {
      this.sendReq(subId, filter);
    } else {
      // markReady() sends a REQ for every still-registered subscription once
      // the connection is ready, including this one — do not also send it
      // here, or the relay gets the same REQ twice. A failed connect()
      // surfaces to whoever called connect()/sendMessage(); this attempt
      // only needs to not become an unhandled rejection.
      void this.connect().catch(() => {});
    }

    return () => {
      sub.listeners.delete(onEvent);
      if (options?.onEose) sub.eoseListeners.delete(options.onEose);
      if (sub.listeners.size === 0) {
        this.closeSub(subId);
        this.channelSubs.delete(channelId);
      }
    };
  }

  /** Buffered events for a channel (post-connect backfill). */
  getTimeline(channelId: string): NostrEvent[] {
    const subId = this.channelSubs.get(channelId);
    if (!subId) return [];
    return this.subs.get(subId)?.events.slice() ?? [];
  }

  /**
   * Sign and publish a kind-9 channel message, waiting for the relay `OK`.
   */
  async sendMessage(
    channelId: string,
    content: string,
    replyToId?: string,
  ): Promise<PublishAck> {
    if (this.state !== "connected") {
      await this.connect();
    }
    const unsigned = buildChannelMessageEvent({
      channelId,
      content,
      replyToId,
    });
    const signed = await this.signer.signEvent(unsigned);
    return this.publish(signed);
  }

  /** Low-level publish of an already-signed event. */
  publish(event: NostrEvent): Promise<PublishAck> {
    return new Promise((resolve, reject) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        reject(new Error("Not connected"));
        return;
      }
      this.pendingPublish.set(event.id, { resolve, reject });
      this.ws.send(JSON.stringify(["EVENT", event]));
    });
  }

  async getPublicKey(): Promise<string> {
    return this.signer.getPublicKey();
  }

  // ── internals ──────────────────────────────────────────────────────────

  private clearAuthTimer(): void {
    if (this.unauthenticatedReqTimer) {
      clearTimeout(this.unauthenticatedReqTimer);
      this.unauthenticatedReqTimer = null;
    }
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  /** Jittered exponential backoff: 500ms, 1s, 2s, ... capped at 10s. */
  private scheduleReconnect(): void {
    if (this.reconnectTimer) return;
    const exponential = this.minReconnectDelayMs * 2 ** this.reconnectAttempt;
    const capped = Math.min(exponential, this.maxReconnectDelayMs);
    const jittered = capped / 2 + Math.random() * (capped / 2);
    this.reconnectAttempt += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      void this.connect().catch(() => {
        // failConnect already recorded the failure. A resulting close event
        // (or a synchronous construction failure above) schedules the next
        // attempt — nothing further to do here.
      });
    }, jittered);
  }

  private markReady(): void {
    this.setState("connected");
    this.reconnectAttempt = 0;
    const resolve = this.connectResolve;
    this.connectResolve = null;
    this.connectReject = null;
    // Keep connectPromise so concurrent connect() calls resolve to the same
    // settled promise while still connected.
    if (resolve) resolve();

    // Re-send every subscription on this (possibly new) socket.
    for (const sub of this.subs.values()) {
      this.sendReq(sub.subId, sub.filter);
    }
  }

  private failConnect(err: Error): void {
    this.clearAuthTimer();
    this.setState("error", err.message);
    const reject = this.connectReject;
    this.connectResolve = null;
    this.connectReject = null;
    this.connectPromise = null;
    if (reject) reject(err);
  }

  private sendReq(subId: string, filter: NostrFilter): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    this.ws.send(JSON.stringify(["REQ", subId, filter]));
  }

  private closeSub(subId: string): void {
    this.subs.delete(subId);
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(["CLOSE", subId]));
    }
  }

  private async handleMessage(raw: string): Promise<void> {
    let data: unknown;
    try {
      data = JSON.parse(raw);
    } catch {
      return;
    }
    if (!Array.isArray(data) || data.length === 0) return;

    const type = data[0];

    if (type === "AUTH" && typeof data[1] === "string") {
      await this.handleAuthChallenge(data[1]);
      return;
    }

    if (type === "OK" && typeof data[1] === "string") {
      const eventId = data[1];
      const ok = data[2] === true;
      const message = typeof data[3] === "string" ? data[3] : undefined;

      if (eventId === this.authEventId) {
        if (ok) {
          this.clearAuthTimer();
          if (this.connectResolve) {
            // Fresh connect (including a reconnect): this socket has not
            // sent any REQ yet — send them now.
            this.markReady();
          } else {
            // Mid-session re-challenge: the socket and its subscriptions
            // were never closed, so there is nothing to resend.
            this.setState("connected");
          }
        } else {
          this.failConnect(
            new Error(message ?? "Relay authentication failed."),
          );
        }
        return;
      }

      const pending = this.pendingPublish.get(eventId);
      if (pending) {
        this.pendingPublish.delete(eventId);
        pending.resolve({ id: eventId, ok, message });
      }
      return;
    }

    if (type === "EVENT" && typeof data[1] === "string" && data[2]) {
      const subId = data[1];
      const event = data[2] as NostrEvent;
      const sub = this.subs.get(subId);
      if (!sub) return;
      // Dedupe by id
      if (!sub.events.some((e) => e.id === event.id)) {
        sub.events.push(event);
      }
      for (const listener of sub.listeners) {
        try {
          listener(event);
        } catch {
          // ignore listener errors
        }
      }
      return;
    }

    if (type === "EOSE" && typeof data[1] === "string") {
      const sub = this.subs.get(data[1]);
      if (!sub || sub.eose) return;
      sub.eose = true;
      for (const listener of sub.eoseListeners) {
        try {
          listener();
        } catch {
          // ignore
        }
      }
      return;
    }

    if (type === "CLOSED" && typeof data[1] === "string") {
      const subId = data[1];
      const reason =
        typeof data[2] === "string" ? data[2] : "subscription closed";
      const sub = this.subs.get(subId);
      if (sub) {
        this.subs.delete(subId);
        for (const [ch, id] of this.channelSubs) {
          if (id === subId) this.channelSubs.delete(ch);
        }
        // Surface as connection detail only if still connecting.
        if (this.connectReject) {
          this.failConnect(new Error(reason));
        }
      }
      return;
    }

    // NOTICE — informational; ignore for now.
  }

  private async handleAuthChallenge(challenge: string): Promise<void> {
    this.clearAuthTimer();
    this.setState("authenticating");
    try {
      const template = makeAuthEvent(this.relayUrl, challenge);
      const signed = await this.signer.signEvent(template);
      this.authEventId = signed.id;
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        this.failConnect(new Error("Socket closed during AUTH"));
        return;
      }
      this.ws.send(JSON.stringify(["AUTH", signed]));
    } catch (err) {
      this.failConnect(
        err instanceof Error
          ? err
          : new Error("Failed to sign relay authentication."),
      );
    }
  }
}

export function createBuzzClient(options: BuzzClientOptions): BuzzClient {
  return new BuzzClient(options);
}
