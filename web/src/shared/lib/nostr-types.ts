/**
 * Shared protocol types for the persistent BuzzClient.
 *
 * Event/signing shapes are re-exported from `nostr-signer.ts` (the single
 * source of truth already used by `nostr-client.ts` and `nip98.ts`) so the
 * persistent client never drifts from the one-shot client on event shape.
 */

export type { SignedNostrEvent, UnsignedNostrEvent } from "./nostr-signer";

import type { SignedNostrEvent } from "./nostr-signer";

export type NostrEvent = SignedNostrEvent;

export interface NostrFilter {
  ids?: string[];
  authors?: string[];
  kinds?: number[];
  since?: number;
  until?: number;
  limit?: number;
  [tag: `#${string}`]: string[] | undefined;
}

/** Connection lifecycle for the persistent BuzzClient. */
export type ConnectionState =
  | "idle"
  | "connecting"
  | "authenticating"
  | "connected"
  | "disconnected"
  | "error";

export interface PublishAck {
  id: string;
  ok: boolean;
  message?: string;
}

export type TimelineListener = (event: NostrEvent) => void;
export type ConnectionListener = (
  state: ConnectionState,
  detail?: string,
) => void;
export type Unsub = () => void;
