/**
 * NIP-46 remote signing ("bunker"): the browser never holds the user's
 * identity key. It holds one LOCAL, throwaway keypair — `buzz:bunker-client`
 * in `localStorage` — used only to encrypt the NIP-46 RPC channel to the
 * bunker. That is NOT the identity key: it cannot sign anything the bunker
 * didn't approve, and the identity key itself stays in the bunker (nsec.app,
 * Amber, a self-hosted Bunker46, …) and never reaches this browser. This is
 * the entire point of the owner ruling this module implements — no
 * `LocalKeySigner` fallback for durable membership, and no browser-held
 * custody smuggled back in under a different name. If a future reader sees
 * "secret in localStorage" here, this comment is why that is not the same
 * thing the ruling rejected.
 *
 * v1 scope: `bunker://` input only. `nostrconnect://` (client-initiated,
 * QR/deep-link) is deferred — nsec.app already emits usable `bunker://`
 * URIs today.
 */
import {
  BunkerSigner as ToolsBunkerSigner,
  parseBunkerInput,
  type BunkerPointer,
} from "nostr-tools/nip46";
import { generateSecretKey } from "nostr-tools/pure";
import { bytesToHex, hexToBytes } from "nostr-tools/utils";

import type {
  Signer,
  SignedNostrEvent,
  UnsignedNostrEvent,
} from "./nostr-signer";

const CLIENT_SECRET_STORAGE_KEY = "buzz:bunker-client-secret";
const BUNKER_POINTER_STORAGE_KEY = "buzz:bunker-pointer";

/** Time budget for the initial `connect()` handshake — a human may need to approve in their bunker app. */
const CONNECT_TIMEOUT_MS = 60_000;
/** Time budget for a single signing/read request once already connected. */
const REQUEST_TIMEOUT_MS = 30_000;

/**
 * Closed set of states for a bunker connection. "disconnected" is the only
 * value not named directly in the owner ruling — the neutral starting state
 * before any attempt, or after an explicit disconnect. Every other value is
 * a distinct, non-collapsible outcome: this axis answers "can I sign?",
 * never "may I post?" (relay membership) — see `describeBunkerBanner`.
 */
export type BunkerConnectionState =
  | "disconnected"
  | "connecting"
  | "awaiting-approval"
  | "connected"
  | "denied"
  | "unreachable"
  | "timed-out";

export type SignerFailureKind = "denied" | "unreachable" | "timed-out";

/**
 * Thrown by a bunker-backed signing/connect operation. `kind` is the same
 * closed set as the failure branches of `BunkerConnectionState`, so callers
 * (composer, invite-claim) can render the same three distinct messages the
 * house rule requires instead of one generic failure.
 */
export class SignerRequestError extends Error {
  readonly kind: SignerFailureKind;
  constructor(kind: SignerFailureKind, message: string) {
    super(message);
    this.name = "SignerRequestError";
    this.kind = kind;
  }
}

function loadOrCreateClientSecretKey(): Uint8Array {
  const stored =
    typeof window === "undefined"
      ? null
      : window.localStorage.getItem(CLIENT_SECRET_STORAGE_KEY);
  if (stored) {
    try {
      return hexToBytes(stored);
    } catch {
      // Corrupt value — fall through and mint a fresh one.
    }
  }
  const fresh = generateSecretKey();
  if (typeof window !== "undefined") {
    window.localStorage.setItem(CLIENT_SECRET_STORAGE_KEY, bytesToHex(fresh));
  }
  return fresh;
}

/** Read the last-connected bunker so a reload can reconnect without re-asking for a URI. */
export function loadPersistedBunkerPointer(): BunkerPointer | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem(BUNKER_POINTER_STORAGE_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof (parsed as { pubkey?: unknown }).pubkey === "string" &&
      Array.isArray((parsed as { relays?: unknown }).relays)
    ) {
      return parsed as BunkerPointer;
    }
  } catch {
    // Corrupt value — treat as absent rather than throwing on mount.
  }
  return null;
}

function persistBunkerPointer(bp: BunkerPointer): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(BUNKER_POINTER_STORAGE_KEY, JSON.stringify(bp));
}

/**
 * Forget the connected bunker. Deliberately does NOT clear the client
 * secret key — it is not identity material, and keeping it means a future
 * reconnect reuses the same RPC channel identity rather than forcing
 * re-approval at the bunker for no reason.
 */
export function clearPersistedBunker(): void {
  if (typeof window === "undefined") return;
  window.localStorage.removeItem(BUNKER_POINTER_STORAGE_KEY);
}

/**
 * Best-effort classification of a failed bunker round-trip into the closed
 * failure set, verified against nostr-tools 2.23.12's `nip46.js`: `Promise.any`
 * over every configured relay's publish rejects with an `AggregateError` when
 * ALL of them refuse the request (transport-level — "unreachable"); a
 * bunker-side explicit refusal instead rejects with the plain string/Error
 * the bunker sent back ("denied"). This is reading a library's *behavior*,
 * not a documented contract, so an ambiguous case fails toward "unreachable"
 * — a retriable, non-accusatory claim — rather than "denied", which asserts
 * a human decision we did not actually observe.
 */
function isUnreachableRejection(error: unknown): boolean {
  // Checked by `.name` rather than `instanceof AggregateError` — the same
  // runtime check, but one that doesn't need AggregateError in the TS lib
  // target, and is robust even if a bundler polyfills the constructor.
  return error instanceof Error && error.name === "AggregateError";
}

async function raceWithClassification<T>(
  promise: Promise<T>,
  timeoutMs: number,
): Promise<T> {
  // Attach a no-op handler so a late settlement after we've already timed
  // out doesn't surface as an unhandled rejection; Promise.race below still
  // observes the original promise independently.
  promise.catch(() => {});
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () =>
        reject(
          new SignerRequestError(
            "timed-out",
            "Your signer did not respond in time.",
          ),
        ),
      timeoutMs,
    );
  });
  try {
    return await Promise.race([promise, timeout]);
  } catch (error) {
    if (error instanceof SignerRequestError) throw error;
    if (isUnreachableRejection(error)) {
      throw new SignerRequestError(
        "unreachable",
        "Could not reach your signer.",
      );
    }
    throw new SignerRequestError(
      "denied",
      error instanceof Error
        ? error.message
        : "Your signer refused this request.",
    );
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** The slice of `nostr-tools`' `BunkerSigner` that `Nip46Signer` actually uses — narrowed so tests can pass a plain fake instead of a real relay-backed connection. */
export interface BunkerLike {
  getPublicKey(): Promise<string>;
  signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent>;
  close(): Promise<void>;
}

/** Wraps a connected `nostr-tools` `BunkerSigner` as a Buzz `Signer`. */
export class Nip46Signer implements Signer {
  readonly type = "bunker" as const;
  private readonly bunker: BunkerLike;
  private readonly requestTimeoutMs: number;

  constructor(bunker: BunkerLike, requestTimeoutMs = REQUEST_TIMEOUT_MS) {
    this.bunker = bunker;
    this.requestTimeoutMs = requestTimeoutMs;
  }

  getPublicKey(): Promise<string> {
    return raceWithClassification(
      this.bunker.getPublicKey(),
      this.requestTimeoutMs,
    );
  }

  async signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent> {
    return raceWithClassification(
      this.bunker.signEvent(event),
      this.requestTimeoutMs,
    );
  }

  /** Local teardown only — does not revoke the session at the bunker. */
  disconnect(): Promise<void> {
    return this.bunker.close();
  }
}

/**
 * Connect to a bunker from a previously-parsed pointer (reconnect-on-load
 * path) or a freshly-entered `bunker://` URI (`connectBunker` below).
 * `onStateChange` drives the closed enum: "connecting" immediately,
 * "awaiting-approval" if the bunker signals an `auth_url` step mid-flow,
 * then a terminal "connected" or one of the three failure states.
 */
export async function connectToBunkerPointer(
  bp: BunkerPointer,
  onStateChange: (state: BunkerConnectionState) => void,
): Promise<Nip46Signer> {
  onStateChange("connecting");
  const clientSecretKey = loadOrCreateClientSecretKey();
  const bunker = ToolsBunkerSigner.fromBunker(clientSecretKey, bp, {
    onauth: () => onStateChange("awaiting-approval"),
  });
  try {
    await raceWithClassification(bunker.connect(), CONNECT_TIMEOUT_MS);
  } catch (error) {
    await bunker.close().catch(() => {});
    onStateChange(error instanceof SignerRequestError ? error.kind : "denied");
    throw error;
  }
  persistBunkerPointer(bp);
  onStateChange("connected");
  return new Nip46Signer(bunker);
}

/** Parse a `bunker://` URI (or NIP-05 identifier) and connect. Throws a plain `Error` — not a state transition — on malformed input, before any connection attempt starts. */
export async function connectBunker(
  input: string,
  onStateChange: (state: BunkerConnectionState) => void,
): Promise<Nip46Signer> {
  const bp = await parseBunkerInput(input.trim());
  if (!bp) {
    throw new Error("That doesn't look like a valid bunker:// URI.");
  }
  return connectToBunkerPointer(bp, onStateChange);
}

export interface BunkerBanner {
  tone: "info" | "warning" | "error";
  label: string;
}

/**
 * What to show for a given bunker connection state. Mirrors
 * `describeConnectionBanner` in `chat/lib/view-state.ts`: `null` only for
 * the fully-idle and fully-connected cases, every other state surfaces
 * something rather than disappearing.
 */
export function describeBunkerBanner(
  state: BunkerConnectionState,
): BunkerBanner | null {
  switch (state) {
    case "disconnected":
    case "connected":
      return null;
    case "connecting":
      return { tone: "info", label: "Connecting to your signer…" };
    case "awaiting-approval":
      return {
        tone: "info",
        label: "Waiting for you to approve this in your signer app…",
      };
    case "denied":
      return { tone: "error", label: "Your signer refused this request." };
    case "unreachable":
      return { tone: "error", label: "Could not reach your signer." };
    case "timed-out":
      return {
        tone: "warning",
        label: "Your signer did not respond in time.",
      };
    default:
      // A future BunkerConnectionState value must surface, never silently
      // disappear — same rule chat/lib/view-state.ts enforces.
      return { tone: "warning", label: "Signer status unknown." };
  }
}
