import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { truncatePubkey } from "@/shared/lib/pubkey";
import { relativeTime } from "@/shared/lib/relative-time";
import { PubkeyAvatar } from "@/shared/ui/PubkeyAvatar";
import type { ChatMessage } from "../lib/timeline";

export function MessageBubble({ message }: { message: ChatMessage }) {
  return (
    <div className="flex items-start gap-3 px-4 py-1.5 hover:bg-accent/40">
      <PubkeyAvatar pubkey={message.pubkey} size="sm" />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-sm font-medium text-foreground">
            {truncatePubkey(message.pubkey)}
          </span>
          <span className="text-xs text-muted-foreground">
            {relativeTime(message.createdAt)}
          </span>
        </div>
        <div className="prose prose-sm max-w-none break-words text-sm text-foreground [&_p]:my-0 [&_pre]:my-1 dark:prose-invert">
          <Markdown remarkPlugins={[remarkGfm]}>{message.content}</Markdown>
        </div>
      </div>
    </div>
  );
}
