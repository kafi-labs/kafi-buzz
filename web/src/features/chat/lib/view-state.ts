/**
 * What the timeline and connection banner should show for a given
 * connection/history state — kept pure and out of the render tree so the
 * "unknown must not degrade to a confident assertion" rule is testable
 * without a WebSocket or DOM.
 *
 * The two failure modes this exists to prevent: a channel that has never
 * loaded history rendering identically to one confirmed empty, and a
 * transient reconnect wiping messages that are still known-good.
 */

import type { ConnectionState } from "@/shared/lib/nostr-types";

export type TimelineViewState = "loading" | "unreachable" | "empty" | "ready";

export function deriveTimelineViewState(input: {
  connectionState: ConnectionState;
  isLoadingHistory: boolean;
  messageCount: number;
}): TimelineViewState {
  const { connectionState, isLoadingHistory, messageCount } = input;
  // Known-good data already observed — a later connection hiccup is surfaced
  // by the banner, not by hiding data we already confirmed.
  if (messageCount > 0) {
    return "ready";
  }
  if (connectionState === "error" || connectionState === "disconnected") {
    return "unreachable";
  }
  if (isLoadingHistory) {
    return "loading";
  }
  return "empty";
}

export interface ConnectionBanner {
  tone: "info" | "warning" | "error";
  label: string;
}

/** `null` means nothing to show — the connected, steady-state case is silent. */
export function describeConnectionBanner(
  state: ConnectionState,
  detail?: string,
): ConnectionBanner | null {
  switch (state) {
    case "connected":
      return null;
    case "idle":
    case "connecting":
      return { tone: "info", label: "Connecting to the relay…" };
    case "authenticating":
      return { tone: "info", label: "Authenticating…" };
    case "disconnected":
      return { tone: "warning", label: "Reconnecting…" };
    case "error":
      return {
        tone: "error",
        label: detail ? `Connection error: ${detail}` : "Connection error.",
      };
    default:
      // A future ConnectionState value must surface, never silently
      // disappear — the same rule this module exists to enforce.
      return { tone: "warning", label: "Connection status unknown." };
  }
}
