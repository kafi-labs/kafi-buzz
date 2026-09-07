import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
} from "nostr-tools/pure";
import { decode as decodeNip19, nsecEncode } from "nostr-tools/nip19";

export type UnsignedNostrEvent = {
  kind: number;
  created_at: number;
  tags: string[][];
  content: string;
};

export type SignedNostrEvent = UnsignedNostrEvent & {
  id: string;
  pubkey: string;
  sig: string;
};

type Nip07Provider = {
  getPublicKey(): Promise<string>;
  signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent>;
};

declare global {
  interface Window {
    nostr?: Nip07Provider;
  }
}

export class Nip07UnavailableError extends Error {
  constructor() {
    super("A NIP-07 browser extension is required to join in the browser.");
    this.name = "Nip07UnavailableError";
  }
}

let ephemeralSecretKey: Uint8Array | null = null;

function getEphemeralSecretKey(): Uint8Array {
  if (!ephemeralSecretKey) {
    ephemeralSecretKey = generateSecretKey();
  }
  return ephemeralSecretKey;
}

export function hasNip07Provider(): boolean {
  return typeof window !== "undefined" && window.nostr != null;
}

function sameUnsignedEvent(
  expected: UnsignedNostrEvent,
  actual: SignedNostrEvent,
): boolean {
  return (
    actual.kind === expected.kind &&
    actual.created_at === expected.created_at &&
    actual.content === expected.content &&
    JSON.stringify(actual.tags) === JSON.stringify(expected.tags)
  );
}

/**
 * Sign with NIP-07 when available, otherwise use a page-lifetime key.
 *
 * The ephemeral fallback preserves anonymous browsing on open relays. Flows
 * that create durable membership must set `requireNip07` so a reload cannot
 * orphan a relay-membership row.
 */
export async function signNostrEvent(
  template: Omit<UnsignedNostrEvent, "created_at"> & {
    created_at?: number;
  },
  options?: { requireNip07?: boolean },
): Promise<SignedNostrEvent> {
  const unsigned: UnsignedNostrEvent = {
    ...template,
    created_at: template.created_at ?? Math.floor(Date.now() / 1000),
  };
  const provider = typeof window === "undefined" ? undefined : window.nostr;

  if (provider) {
    const expectedPubkey = await provider.getPublicKey();
    const signed = await provider.signEvent(unsigned);
    if (
      signed.pubkey !== expectedPubkey ||
      !sameUnsignedEvent(unsigned, signed) ||
      typeof signed.id !== "string" ||
      typeof signed.sig !== "string"
    ) {
      throw new Error("The NIP-07 extension returned an invalid signed event.");
    }
    return signed;
  }

  if (options?.requireNip07) {
    throw new Nip07UnavailableError();
  }

  const secretKey = getEphemeralSecretKey();
  const signed = finalizeEvent(unsigned, secretKey);
  if (signed.pubkey !== getPublicKey(secretKey)) {
    throw new Error("Failed to create the ephemeral browser identity.");
  }
  return signed;
}

/**
 * Signing seam for the persistent BuzzClient: NIP-07 extension, a NIP-46
 * remote signer ("bunker" — see `bunker-signer.ts`), a persisted local key,
 * or a page-lifetime ephemeral key.
 */
export interface Signer {
  /** Hex-encoded public key of the signing identity. */
  getPublicKey(): Promise<string>;
  /** Sign an unsigned event template; returns a fully formed Nostr event. */
  signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent>;
  readonly type: "nip07" | "bunker" | "local" | "ephemeral";
}

/** Delegates to `window.nostr` (Alby, nos2x, etc.). Throws when unavailable. */
export class Nip07Signer implements Signer {
  readonly type = "nip07" as const;

  async getPublicKey(): Promise<string> {
    const provider = typeof window === "undefined" ? undefined : window.nostr;
    if (!provider) {
      throw new Nip07UnavailableError();
    }
    return provider.getPublicKey();
  }

  async signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent> {
    return signNostrEvent(event, { requireNip07: true });
  }
}

const LOCAL_NSEC_STORAGE_KEY = "buzz:nsec";

function loadOrCreateLocalSecretKey(): Uint8Array {
  if (typeof window === "undefined") {
    return generateSecretKey();
  }
  const stored = window.localStorage.getItem(LOCAL_NSEC_STORAGE_KEY);
  if (stored) {
    try {
      const decoded = decodeNip19(stored);
      if (decoded.type === "nsec") {
        return decoded.data;
      }
    } catch {
      // Corrupt or foreign value — fall through and mint a fresh identity.
    }
  }
  const fresh = generateSecretKey();
  window.localStorage.setItem(LOCAL_NSEC_STORAGE_KEY, nsecEncode(fresh));
  return fresh;
}

/**
 * Persistent browser identity: a secret key generated once and stored under
 * `localStorage["buzz:nsec"]`. Survives reloads without a NIP-07 extension —
 * the zero-friction path for web onboarding.
 */
export class LocalKeySigner implements Signer {
  readonly type = "local" as const;
  private readonly secretKey: Uint8Array;
  private readonly pubkey: string;

  constructor(secretKey?: Uint8Array) {
    this.secretKey = secretKey ?? loadOrCreateLocalSecretKey();
    this.pubkey = getPublicKey(this.secretKey);
  }

  async getPublicKey(): Promise<string> {
    return this.pubkey;
  }

  async signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent> {
    return finalizeEvent(event, this.secretKey);
  }
}

/**
 * Page-lifetime key, never persisted — a reload mints a new identity. Shares
 * the same module-level key as the `signNostrEvent` ephemeral fallback so a
 * tab has one consistent anonymous identity across both call paths.
 */
export class EphemeralSigner implements Signer {
  readonly type = "ephemeral" as const;
  private readonly secretKey: Uint8Array;
  private readonly pubkey: string;

  constructor(secretKey?: Uint8Array) {
    this.secretKey = secretKey ?? getEphemeralSecretKey();
    this.pubkey = getPublicKey(this.secretKey);
  }

  async getPublicKey(): Promise<string> {
    return this.pubkey;
  }

  async signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent> {
    return finalizeEvent(event, this.secretKey);
  }
}
