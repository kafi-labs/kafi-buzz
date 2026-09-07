import { useBunkerSigner } from "@/shared/context/BunkerSignerContext";
import { describeBunkerBanner } from "@/shared/lib/bunker-signer";
import {
  ChatClientProvider,
  useChatClient,
} from "../context/ChatClientContext";
import { describeConnectionBanner } from "../lib/view-state";
import { ChannelTimeline } from "./ChannelTimeline";
import { MessageComposer } from "./MessageComposer";

const TONE_CLASSES: Record<"info" | "warning" | "error", string> = {
  info: "bg-muted text-muted-foreground",
  warning: "bg-muted text-muted-foreground",
  error: "bg-destructive/10 text-destructive",
};

/**
 * Two independent axes, each surfaced on its own row: "can I sign?" (bunker
 * connection — this component) and "may I post?" (relay membership — the
 * relay ConnectionBanner below it). A user can be bunker-connected but not
 * enrolled, or enrolled but bunker-disconnected; collapsing those into one
 * banner would hide which one actually needs fixing.
 */
function BunkerBanner() {
  const { state } = useBunkerSigner();
  const banner = describeBunkerBanner(state);
  if (!banner) return null;
  return (
    <div
      className={`px-4 py-1.5 text-center text-xs ${TONE_CLASSES[banner.tone]}`}
    >
      {banner.label}
    </div>
  );
}

function ConnectionBanner() {
  const { connectionState, connectionDetail } = useChatClient();
  const banner = describeConnectionBanner(connectionState, connectionDetail);
  if (!banner) return null;

  return (
    <div
      className={`px-4 py-1.5 text-center text-xs ${TONE_CLASSES[banner.tone]}`}
    >
      {banner.label}
    </div>
  );
}

function ChannelPageContent({ channelId }: { channelId: string }) {
  return (
    <div className="flex h-dvh flex-col">
      <BunkerBanner />
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
