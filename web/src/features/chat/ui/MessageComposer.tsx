import { SendHorizontal } from "lucide-react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import {
  SignerRequestError,
  describeBunkerBanner,
} from "@/shared/lib/bunker-signer";
import { Button } from "@/shared/ui/button";
import { useChatClient } from "../context/ChatClientContext";
import { isSendableContent, shouldSubmitOnKeyDown } from "../lib/composer";

const MAX_TEXTAREA_HEIGHT_PX = 200;

export function MessageComposer({ channelId }: { channelId: string }) {
  const { client } = useChatClient();
  const [content, setContent] = useState("");
  const [isSending, setIsSending] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    if (content.length === 0) return;
    el.style.height = `${Math.min(el.scrollHeight, MAX_TEXTAREA_HEIGHT_PX)}px`;
  }, [content]);

  const canSend = isSendableContent(content) && !isSending;

  async function handleSubmit() {
    if (!isSendableContent(content) || isSending) return;
    const trimmed = content.trim();
    setIsSending(true);
    try {
      const ack = await client.sendMessage(channelId, trimmed);
      if (!ack.ok) {
        throw new Error(ack.message || "The relay rejected the message.");
      }
      // Only clear the draft once the relay has actually confirmed it —
      // sent is not received, and the draft is the user's only recovery
      // affordance if the publish fails.
      setContent("");
    } catch (error) {
      if (error instanceof SignerRequestError) {
        // A signing failure and a relay-publish failure are different
        // problems with different remedies (retry the signer vs. check the
        // connection) — the closed set the bunker banner uses becomes the
        // toast title so they read as distinctly here as anywhere else,
        // never collapsed into one generic "not sent".
        toast.error(
          describeBunkerBanner(error.kind)?.label ?? "Your signer failed.",
          { description: "Message not sent." },
        );
      } else {
        toast.error("Message not sent", {
          description:
            error instanceof Error ? error.message : "Unknown error.",
        });
      }
    } finally {
      setIsSending(false);
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (shouldSubmitOnKeyDown(event)) {
      event.preventDefault();
      void handleSubmit();
    }
  }

  return (
    <div className="border-t border-border p-3">
      <div className="flex items-end gap-2">
        <textarea
          ref={textareaRef}
          value={content}
          onChange={(e) => setContent(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={isSending}
          rows={1}
          placeholder={`Message #${channelId}`}
          aria-label={`Message #${channelId}`}
          className="flex-1 resize-none rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
        />
        <Button
          type="button"
          size="icon"
          disabled={!canSend}
          aria-label="Send message"
          onClick={() => void handleSubmit()}
        >
          <SendHorizontal className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
