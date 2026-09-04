/**
 * Resolve workspace agents by joining managed-agent records to their personas.
 *
 * ## The join
 *
 * `(30177.author, content.persona_id)` → the `30175` coordinate. Three rules
 * that are easy to get wrong and each cause a silent, not loud, failure:
 *
 * 1. **Scope by author, not slug alone.** NIP-AP: "Two different owners can
 *    publish personas with the same slug. Clients MUST always scope queries by
 *    author pubkey." A slug-only join cross-wires two owners' agents.
 * 2. **Normalize the slug identically on both sides**, or the join misses and
 *    the agent silently looks unconfigured.
 * 3. **A dangling `persona_id` is a real state, not an impossibility.** The
 *    `30177` projection *slims the definition quad when `persona_id` is set*, so
 *    a record whose persona was deleted carries no `model` at all — there is
 *    nothing to spawn from. Removal deletes the `30177` coordinate and leaves
 *    the persona standing, but the reverse (persona deleted, record kept) is
 *    equally reachable. Surface it; never render it as a healthy agent.
 */

import type { NostrEvent } from "@/shared/lib/nostr-client";
import { dTag, parseJsonContent, tagValue } from "@/shared/lib/nostr-tags";
import type {
  ManagedAgentContent,
  PersonaContent,
  RuntimeStatusContent,
  WorkspaceAgent,
} from "./types";

/**
 * Normalize a persona slug for joining.
 *
 * Mirrors the desktop's `normalize_d_tag`: trim, lowercase. Kept as a named
 * export so the shared test vectors can pin all implementations to one
 * behaviour rather than three independent guesses.
 */
export function normalizePersonaSlug(slug: string): string {
  return slug.trim().toLowerCase();
}

function personaKey(author: string, slug: string): string {
  return `${author.toLowerCase()}:${normalizePersonaSlug(slug)}`;
}

/**
 * Build the workspace agent list.
 *
 * @param agentEvents  kind:30177, already author-pinned and deduped
 * @param personaEvents kind:30175, already author-pinned and deduped
 * @param statusEvents kind:30900 from pinned runners, already deduped
 */
export function resolveWorkspaceAgents(
  agentEvents: NostrEvent[],
  personaEvents: NostrEvent[],
  statusEvents: NostrEvent[],
): WorkspaceAgent[] {
  // `shared` is a TAG, not a content field, so that toggling share state does not
  // change the content bytes that serve as the persona's drift basis. It decides
  // whether anyone but the author can read the persona at all.
  const personas = new Map<
    string,
    { content: PersonaContent; shared: boolean }
  >();
  for (const event of personaEvents) {
    const content = parseJsonContent<PersonaContent>(event);
    if (content) {
      personas.set(personaKey(event.pubkey, dTag(event)), {
        content,
        shared: tagValue(event, "shared") === "true",
      });
    }
  }

  // Status is addressed by agent pubkey once running, and by persona slug while
  // a candidate is still awaiting attestation.
  const statusByDTag = new Map<string, RuntimeStatusContent>();
  for (const event of statusEvents) {
    const content = parseJsonContent<RuntimeStatusContent>(event);
    if (content?.state) {
      statusByDTag.set(normalizePersonaSlug(dTag(event)), content);
    }
  }

  const agents: WorkspaceAgent[] = [];
  for (const event of agentEvents) {
    const record = parseJsonContent<ManagedAgentContent>(event);
    if (!record) continue;

    const agentPubkey = dTag(event);
    if (!agentPubkey) continue;

    const personaId = record.persona_id?.trim() || null;
    const linked = personaId
      ? personas.get(personaKey(event.pubkey, personaId))
      : undefined;
    const persona = linked?.content;
    const danglingPersona = personaId !== null && linked === undefined;

    // With a linked persona the projection is slimmed, so the persona is the
    // authoritative source for the spawn fields; without one the projection
    // carries them itself.
    agents.push({
      pubkey: agentPubkey,
      owner: event.pubkey,
      displayName:
        persona?.display_name?.trim() || record.name?.trim() || agentPubkey,
      personaId,
      runtime: persona?.runtime ?? null,
      gatewayAgent: persona?.model ?? record.model ?? null,
      gatewayId: persona?.provider ?? record.provider ?? null,
      systemPrompt: persona?.system_prompt ?? record.system_prompt ?? null,
      respondTo: record.respond_to ?? null,
      updatedAt: event.created_at,
      danglingPersona,
      personaShared: linked ? linked.shared : null,
      status:
        statusByDTag.get(normalizePersonaSlug(agentPubkey)) ??
        (personaId
          ? statusByDTag.get(normalizePersonaSlug(personaId))
          : undefined) ??
        null,
    });
  }

  return agents.sort((a, b) => a.displayName.localeCompare(b.displayName));
}

/**
 * Whether a heartbeat is stale.
 *
 * The runner publishes every 30 s; 90 s without one is stale. Compared against
 * the reader's own clock because `created_at` is author-supplied and bounded
 * only by a ±900 s ingest fence — a peer timestamp cannot measure liveness.
 */
export const HEARTBEAT_INTERVAL_S = 30;
export const HEARTBEAT_STALE_S = 90;

export function isHeartbeatStale(
  status: RuntimeStatusContent | null,
  nowSeconds: number,
): boolean {
  if (!status?.heartbeat_at) return true;
  return nowSeconds - status.heartbeat_at > HEARTBEAT_STALE_S;
}
