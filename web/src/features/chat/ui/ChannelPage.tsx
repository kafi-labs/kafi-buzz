import {
  ChatClientProvider,
  useChatClient,
} from "../context/ChatClientContext";
import { describeConnectionBanner } from "../lib/view-state";
import { ChannelTimeline } from "./ChannelTimeline";
import { MessageComposer } from "./MessageComposer";

function ConnectionBanner() {
  const { connectionState, connectionDetail } = useChatClient();
  const banner = describeConnectionBanner(connectionState, connectionDetail);
  if (!banner) return null;

  const toneClasses: Record<typeof banner.tone, string> = {
    info: "bg-muted text-muted-foreground",
    warning: "bg-muted text-muted-foreground",
    error: "bg-destructive/10 text-destructive",
  };

  return (
    <div
      className={`px-4 py-1.5 text-center text-xs ${toneClasses[banner.tone]}`}
    >
      {banner.label}
    </div>
  );
}

function ChannelPageContent({ channelId }: { channelId: string }) {
  return (
    <div className="flex h-dvh flex-col">
      <ConnectionBanner />
      <div className="flex items-center border-b border-border px-4 py-3">
        <h1 className="truncate text-sm font-semibold text-foreground">
          #{channelId}
        </h1>
      </div>
      <ChannelTimeline channelId={channelId} />
      <MessageComposer channelId={channelId} />
    </div>
  );
}

export function ChannelPage({ channelId }: { channelId: string }) {
  return (
    <ChatClientProvider>
      <ChannelPageContent channelId={channelId} />
    </ChatClientProvider>
  );
}
