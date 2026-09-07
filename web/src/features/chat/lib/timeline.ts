/**
 * Pure timeline helpers for the single-channel chat surface.
 *
 * `BuzzClient.subscribeTimeline` deliberately requests reactions (kind 7)
 * alongside messages so a future reaction UI can share the subscription —
 * this module is what narrows that raw stream down to renderable chat
 * messages, kept pure so it's testable without a WebSocket or React tree.
 */

import { KINDS } from "@/shared/constants/kinds";
import type { NostrEvent } from "@/shared/lib/nostr-types";

export interface ChatMessage {
  id: string;
  pubkey: string;
  content: string;
  createdAt: number;
  replyToId: string | null;
}

const MESSAGE_KINDS: ReadonlySet<number> = new Set([
  KINDS.STREAM_MESSAGE,
  KINDS.STREAM_MESSAGE_V2,
]);

export function isChatMessageEvent(event: NostrEvent): boolean {
  return MESSAGE_KINDS.has(event.kind);
}

function findReplyToId(event: NostrEvent): string | null {
  const replyTag = event.tags.find((t) => t[0] === "e" && t[3] === "reply");
  return replyTag?.[1] ?? event.tags.find((t) => t[0] === "e")?.[1] ?? null;
}

export function toChatMessage(event: NostrEvent): ChatMessage {
  return {
    id: event.id,
    pubkey: event.pubkey,
    content: event.content,
    createdAt: event.created_at,
    replyToId: findReplyToId(event),
  };
}

/** Chronological order (oldest first); ties broken by event id for a stable sort. */
export function sortChatMessages(messages: ChatMessage[]): ChatMessage[] {
  return [...messages].sort((a, b) => {
    if (a.createdAt !== b.createdAt) return a.createdAt - b.createdAt;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
}
