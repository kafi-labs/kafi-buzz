import { useEffect, useMemo, useState } from "react";

import type { BuzzClient } from "@/shared/lib/buzz-client";
import type { NostrEvent } from "@/shared/lib/nostr-types";
import {
  type ChatMessage,
  isChatMessageEvent,
  sortChatMessages,
  toChatMessage,
} from "../lib/timeline";

export interface ChannelTimelineState {
  messages: ChatMessage[];
  /** False once EOSE has been observed for this channel's subscription. */
  isLoadingHistory: boolean;
}

export function useChannelTimeline(
  client: BuzzClient,
  channelId: string,
): ChannelTimelineState {
  const [eventsById, setEventsById] = useState<Map<string, NostrEvent>>(
    () => new Map(),
  );
  const [hasReachedEose, setHasReachedEose] = useState(false);

  useEffect(() => {
    setEventsById(new Map());
    setHasReachedEose(false);

    const unsubscribe = client.subscribeTimeline(
      channelId,
      (event) => {
        setEventsById((prev) => {
          if (prev.has(event.id)) return prev;
          const next = new Map(prev);
          next.set(event.id, event);
          return next;
        });
      },
      { onEose: () => setHasReachedEose(true) },
    );

    return unsubscribe;
  }, [client, channelId]);

  const messages = useMemo(
    () =>
      sortChatMessages(
        [...eventsById.values()].filter(isChatMessageEvent).map(toChatMessage),
      ),
    [eventsById],
  );

  return { messages, isLoadingHistory: !hasReachedEose };
}
