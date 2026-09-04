/**
 * Author-pinned relay reads — the console's only read path.
 *
 * ## Why this module exists
 *
 * The relay accepts persona (30175), managed-agent (30177), and runner-status
 * (30900) events from **any community member**: `POST /events` grants
 * `Scope::all_known()` and those kinds map to an ordinary `UsersWrite` scope.
 * NIP-AA widens the publisher set again to any agent a member has attested.
 *
 * So a console that renders "whatever the relay returned" can be fed a forged
 * row. That matters most on the enrollment screen, where the owner signs a
 * NIP-OA attestation over an `agent_pubkey` taken from an event: the resulting
 * agent→owner binding is first-write-wins and permanent, and NIP-OA conditions
 * are not enforced at relay login, so a forged pubkey yields an unbounded
 * credential that Remove and Retire do not revoke.
 *
 * `created_at` is author-supplied and bounded only by a ±900 s ingest fence, so
 * a forged event can also deterministically sort above a genuine one. Recency
 * is therefore never a trust signal here.
 *
 * ## The rule
 *
 * Every read states its expected authors. The pin is applied twice: in the
 * filter (so the relay does the work) and again over the response (so a
 * misbehaving or compromised relay cannot widen the set). Adding `authors`
 * costs nothing at the relay's read gates — `p_gated_filters_authorized`
 * inspects only `kinds`, `ids`, and `#p`.
 */

import {
  type NostrEvent,
  type NostrFilter,
  queryEvents,
} from "@/shared/lib/nostr-client";

/** A read whose authors are known ahead of time. */
export interface AuthorPin {
  /** Lowercase hex pubkeys this read will accept. Must be non-empty. */
  authors: string[];
  /** Human-readable origin of the pin, surfaced in errors and in the UI. */
  label: string;
}

export class UnpinnedReadError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "UnpinnedReadError";
  }
}

function normalize(pubkeys: string[]): string[] {
  return pubkeys
    .map((key) => key.trim().toLowerCase())
    .filter((key) => /^[0-9a-f]{64}$/.test(key));
}

/**
 * Query the relay for events from a pinned author set.
 *
 * Throws `UnpinnedReadError` rather than falling back to an unpinned read when
 * the pin is empty or malformed — a console that silently reads from anyone is
 * the failure this module exists to prevent, so it must fail loudly.
 */
export async function queryPinned(
  wsUrl: string,
  filter: NostrFilter,
  pin: AuthorPin,
): Promise<NostrEvent[]> {
  const authors = normalize(pin.authors);
  if (authors.length === 0) {
    throw new UnpinnedReadError(
      `Refusing an unpinned relay read for "${pin.label}": no valid author pubkeys. ` +
        "Configure the expected authors before reading.",
    );
  }
  if (!filter.kinds || filter.kinds.length === 0) {
    // An open-ended filter also trips the relay's p-gate and returns 403.
    throw new UnpinnedReadError(
      `Refusing a kindless relay read for "${pin.label}": every query must name its kinds.`,
    );
  }

  const events = await queryEvents(wsUrl, { ...filter, authors });

  const allowed = new Set(authors);
  return events.filter((event) => allowed.has(event.pubkey.toLowerCase()));
}

/**
 * Whether an event's author is inside a pin. Use before treating any relay
 * value as actionable — in particular before offering to sign over it.
 */
export function isPinnedAuthor(event: NostrEvent, pin: AuthorPin): boolean {
  return new Set(normalize(pin.authors)).has(event.pubkey.toLowerCase());
}
