import { AlertTriangle, MessageSquare } from "lucide-react";
import { useEffect, useRef } from "react";

import { useChatClient } from "../context/ChatClientContext";
import { useChannelTimeline } from "../hooks/useChannelTimeline";
import { deriveTimelineViewState } from "../lib/view-state";
import { MessageBubble } from "./MessageBubble";

/** Distance (px) from the bottom within which a new message auto-scrolls the view. */
const AUTOSCROLL_THRESHOLD_PX = 120;

function LoadingState() {
  return (
    <div className="flex flex-1 flex-col justify-end gap-3 px-4 py-4">
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex items-start gap-3">
          <div className="h-6 w-6 shrink-0 animate-pulse rounded-lg bg-black/10 dark:bg-white/10" />
          <div className="flex-1 space-y-1.5">
            <div className="h-3 w-32 animate-pulse rounded bg-black/10 dark:bg-white/10" />
            <div className="h-4 w-2/3 animate-pulse rounded bg-black/10 dark:bg-white/10" />
          </div>
        </div>
      ))}
    </div>
  );
}

function UnreachableState({ channelId }: { channelId: string }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 px-4 text-center">
      <AlertTriangle className="h-8 w-8 text-destructive" />
      <p className="text-sm font-medium text-foreground">
        Can't reach the relay
      </p>
      <p className="max-w-sm text-sm text-muted-foreground">
        #{channelId}'s history couldn't be loaded. This is not the same as an
        empty channel — reconnecting automatically.
      </p>
    </div>
  );
}

function EmptyState({ channelId }: { channelId: string }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 px-4 text-center">
      <MessageSquare className="h-8 w-8 text-muted-foreground" />
      <p className="text-sm font-medium text-foreground">No messages yet</p>
      <p className="max-w-sm text-sm text-muted-foreground">
        Be the first to say something in #{channelId}.
      </p>
    </div>
  );
}

export function ChannelTimeline({ channelId }: { channelId: string }) {
  const { client, connectionState } = useChatClient();
  const { messages, isLoadingHistory } = useChannelTimeline(client, channelId);
  const scrollRef = useRef<HTMLDivElement>(null);
  const wasAtBottomRef = useRef(true);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !wasAtBottomRef.current || messages.length === 0) return;
    el.scrollTop = el.scrollHeight;
  }, [messages]);

  function handleScroll() {
    const el = scrollRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    wasAtBottomRef.current = distanceFromBottom <= AUTOSCROLL_THRESHOLD_PX;
  }

  const viewState = deriveTimelineViewState({
    connectionState,
    isLoadingHistory,
    messageCount: messages.length,
  });

  if (viewState === "loading") return <LoadingState />;
  if (viewState === "unreachable") {
    return <UnreachableState channelId={channelId} />;
  }
  if (viewState === "empty") return <EmptyState channelId={channelId} />;

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      className="flex-1 overflow-y-auto py-2"
    >
      {messages.map((message) => (
        <MessageBubble key={message.id} message={message} />
      ))}
    </div>
  );
}
