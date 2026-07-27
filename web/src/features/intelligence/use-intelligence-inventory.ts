import { useQuery } from "@tanstack/react-query";

import { KIND_MANAGED_AGENT, KIND_PERSONA } from "@/shared/constants/kinds";
import { queryEvents, type NostrEvent } from "@/shared/lib/nostr-client";
import { relayWsUrl } from "@/shared/lib/relay-url";

const INTELLIGENCE_EVENT_KINDS = [KIND_PERSONA, KIND_MANAGED_AGENT] as const;

type IntelligenceEventContent = {
  description?: unknown;
  display_name?: unknown;
  name?: unknown;
};

export type IntelligenceEntry = {
  description?: string;
  id: string;
  kind: "managed agent" | "persona";
  name?: string;
  shared: boolean;
  slug?: string;
};

function tagValue(event: NostrEvent, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

function stringField(
  content: IntelligenceEventContent,
  field: keyof IntelligenceEventContent,
): string | undefined {
  const value = content[field];
  return typeof value === "string" && value.trim() ? value : undefined;
}

function parseContent(event: NostrEvent): IntelligenceEventContent {
  try {
    const value: unknown = JSON.parse(event.content);
    return value && typeof value === "object"
      ? (value as IntelligenceEventContent)
      : {};
  } catch {
    return {};
  }
}

/** Build the bounded relay query required by the relay's p-gate. */
export function intelligenceFilter() {
  return { kinds: [...INTELLIGENCE_EVENT_KINDS] };
}

/** Convert only the public, event-carried inventory fields into a UI row. */
export function eventToIntelligenceEntry(
  event: NostrEvent,
): IntelligenceEntry | undefined {
  const content = parseContent(event);
  const isPersona = event.kind === KIND_PERSONA;
  const isManagedAgent = event.kind === KIND_MANAGED_AGENT;

  if (!isPersona && !isManagedAgent) return undefined;

  return {
    id: event.id,
    kind: isPersona ? "persona" : "managed agent",
    name: stringField(content, isPersona ? "display_name" : "name"),
    slug: tagValue(event, "d"),
    description: stringField(content, "description"),
    shared: event.tags.some((tag) => tag[0] === "shared"),
  };
}

async function fetchIntelligenceInventory(): Promise<IntelligenceEntry[]> {
  const events = await queryEvents(relayWsUrl(), intelligenceFilter());
  return events
    .map(eventToIntelligenceEntry)
    .filter((entry): entry is IntelligenceEntry => entry !== undefined);
}

/** Read the authenticated viewer's visible agent and persona inventory. */
export function useIntelligenceInventory() {
  return useQuery({
    queryKey: ["intelligence", "inventory"],
    queryFn: fetchIntelligenceInventory,
    staleTime: 60_000,
  });
}
