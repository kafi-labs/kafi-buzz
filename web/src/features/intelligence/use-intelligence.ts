import { useQuery } from "@tanstack/react-query";

import { queryPinned } from "@/shared/lib/author-pinned-query";
import {
  dTag,
  dedupeReplaceable,
  parseJsonContent,
} from "@/shared/lib/nostr-tags";
import { relayWsUrl } from "@/shared/lib/relay-url";
import { KINDS } from "@/shared/constants/kinds";
import {
  OWNER_PUBKEYS,
  RUNNER_PUBKEYS,
  ownerPin,
  runnerPin,
} from "./console-config";
import { resolveWorkspaceAgents } from "./join";
import type { GatewayCatalogContent, WorkspaceAgent } from "./types";

const STALE_TIME_MS = 15_000;

async function fetchWorkspaceAgents(): Promise<WorkspaceAgent[]> {
  const url = relayWsUrl();

  // Owner-authored carriers and runner-authored status have different pins, so
  // they are separate reads — a single merged read would need the union of both
  // author sets and would let a runner forge a persona.
  const [agentEvents, personaEvents] = await Promise.all([
    queryPinned(url, { kinds: [KINDS.MANAGED_AGENT] }, ownerPin),
    queryPinned(url, { kinds: [KINDS.PERSONA] }, ownerPin),
  ]);

  const statusEvents =
    RUNNER_PUBKEYS.length > 0
      ? await queryPinned(
          url,
          { kinds: [KINDS.AGENT_RUNTIME_STATUS] },
          runnerPin,
        )
      : [];

  return resolveWorkspaceAgents(
    dedupeReplaceable(agentEvents),
    dedupeReplaceable(personaEvents),
    dedupeReplaceable(statusEvents),
  );
}

export function useWorkspaceAgents() {
  return useQuery({
    queryKey: ["intelligence", "agents", OWNER_PUBKEYS, RUNNER_PUBKEYS],
    queryFn: fetchWorkspaceAgents,
    enabled: OWNER_PUBKEYS.length > 0,
    staleTime: STALE_TIME_MS,
    refetchInterval: STALE_TIME_MS,
  });
}

export interface GatewayCatalog extends GatewayCatalogContent {
  /** Runner pubkey that published this catalog. */
  publishedBy: string;
  publishedAt: number;
}

async function fetchGatewayCatalogs(): Promise<GatewayCatalog[]> {
  const events = await queryPinned(
    relayWsUrl(),
    { kinds: [KINDS.INTEL_GATEWAY_CATALOG] },
    runnerPin,
  );

  return dedupeReplaceable(events)
    .map((event) => {
      const content = parseJsonContent<GatewayCatalogContent>(event);
      if (!content) return null;
      return {
        ...content,
        gateway_id: content.gateway_id || dTag(event),
        agents: Array.isArray(content.agents) ? content.agents : [],
        publishedBy: event.pubkey,
        publishedAt: event.created_at,
      } satisfies GatewayCatalog;
    })
    .filter((catalog): catalog is GatewayCatalog => catalog !== null);
}

export function useGatewayCatalogs() {
  return useQuery({
    queryKey: ["intelligence", "gateways", RUNNER_PUBKEYS],
    queryFn: fetchGatewayCatalogs,
    enabled: RUNNER_PUBKEYS.length > 0,
    staleTime: STALE_TIME_MS,
    refetchInterval: STALE_TIME_MS,
  });
}

export function useWorkspaceAgent(agentPubkey: string) {
  return useQuery({
    queryKey: ["intelligence", "agents", OWNER_PUBKEYS, RUNNER_PUBKEYS],
    queryFn: fetchWorkspaceAgents,
    enabled: OWNER_PUBKEYS.length > 0,
    staleTime: STALE_TIME_MS,
    select: (agents) => agents.find((agent) => agent.pubkey === agentPubkey),
  });
}
