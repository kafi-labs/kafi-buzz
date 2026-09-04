/**
 * Tag helpers and NIP-33 replacement semantics for parameterized-replaceable
 * events, shared by console surfaces.
 */

import type { NostrEvent } from "@/shared/lib/nostr-client";

/** First value for a tag name, or undefined. */
export function tagValue(event: NostrEvent, name: string): string | undefined {
  return event.tags.find((t) => t[0] === name)?.[1];
}

/** All values for a tag name. */
export function tagValues(event: NostrEvent, name: string): string[] {
  return event.tags.filter((t) => t[0] === name).map((t) => t[1]);
}

/** The `d` tag of a parameterized-replaceable event. */
export function dTag(event: NostrEvent): string {
  return tagValue(event, "d") ?? "";
}

/**
 * Collapse parameterized-replaceable events to one per `(pubkey, kind, d)`.
 *
 * Latest `created_at` wins, with lowest event id breaking a same-second tie —
 * matching the relay's own replacement rule (`buzz-db`: "Same-second ties are
 * broken by lowest event `id`"), so the client agrees with the server about
 * which revision is current instead of inventing its own answer.
 *
 * Note the key includes `pubkey`: this is replacement *within* one author, which
 * is legitimate NIP-33. It is NOT a cross-author merge — collapsing two authors
 * onto one row by recency is exactly the forgery path `author-pinned-query`
 * exists to prevent, because `created_at` is author-supplied.
 */
export function dedupeReplaceable(events: NostrEvent[]): NostrEvent[] {
  const best = new Map<string, NostrEvent>();
  for (const event of events) {
    const key = `${event.pubkey}:${event.kind}:${dTag(event)}`;
    const prev = best.get(key);
    if (!prev || isNewerRevision(event, prev)) {
      best.set(key, event);
    }
  }
  return [...best.values()];
}

function isNewerRevision(candidate: NostrEvent, current: NostrEvent): boolean {
  if (candidate.created_at !== current.created_at) {
    return candidate.created_at > current.created_at;
  }
  return candidate.id < current.id;
}

/** Parse an event's JSON content body, or null when it isn't valid JSON. */
export function parseJsonContent<T>(event: NostrEvent): T | null {
  try {
    const parsed: unknown = JSON.parse(event.content);
    return parsed && typeof parsed === "object" ? (parsed as T) : null;
  } catch {
    return null;
  }
}
