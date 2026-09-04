/** Content bodies of the events the Intelligence console reads. */

/** kind:30175 — NIP-AP persona (public, plaintext, owner-authored). */
export interface PersonaContent {
  display_name?: string | null;
  system_prompt?: string | null;
  /** ACP runtime identifier, e.g. "intel". */
  runtime?: string | null;
  /** For runtime "intel": the deployed gateway agent name. */
  model?: string | null;
  /** For runtime "intel": a curated gateway id. Advisory — never a URL. */
  provider?: string | null;
  avatar_url?: string | null;
}

/** kind:30177 — NIP-AP managed agent projection (secret-free by construction). */
export interface ManagedAgentContent {
  name: string;
  persona_id?: string | null;
  system_prompt?: string | null;
  model?: string | null;
  provider?: string | null;
  parallelism?: number;
  respond_to?: string;
  respond_to_allowlist?: string[];
}

/** kind:30900 — runner-authored runtime status. */
export type RuntimeState =
  | "requested"
  | "awaiting_attestation"
  | "attested"
  | "running"
  | "stopped"
  | "crashed"
  | "failed"
  | "expired"
  | "removed";

export interface RuntimeStatusContent {
  state: RuntimeState;
  /** Unix seconds of the last heartbeat the runner published. */
  heartbeat_at?: number;
  uptime_s?: number;
  restarts?: number;
  /** Error *class*, never a message carrying credential material. */
  last_error_class?: string | null;
  /** Candidate agent pubkey, present while awaiting attestation. */
  agent_pubkey?: string | null;
  /** NIP-OA conditions the runner expects the owner to sign over. */
  conditions?: string | null;
  /** Which runner host this came from, for display. */
  runner?: string | null;
}

/** kind:30901 — runner-authored gateway agent catalog. */
export interface GatewayCatalogContent {
  gateway_id: string;
  agents: { name: string; description?: string | null }[];
  probe?: { ok: boolean; scope?: string | null };
  fetched_at?: number;
}

/** A workspace agent as the console presents it. */
export interface WorkspaceAgent {
  /** Agent pubkey — the `d` tag of its kind:30177. */
  pubkey: string;
  /** Owner pubkey that authored the record. */
  owner: string;
  displayName: string;
  personaId: string | null;
  /** Resolved from the linked persona when present, else the projection. */
  runtime: string | null;
  gatewayAgent: string | null;
  gatewayId: string | null;
  systemPrompt: string | null;
  respondTo: string | null;
  updatedAt: number;
  /** True when `persona_id` is set but no matching persona resolved. */
  danglingPersona: boolean;
  /**
   * Whether the linked persona carries `["shared","true"]`.
   *
   * The relay gates kind 30175 author-only unless shared, so a shared persona's
   * plaintext `system_prompt` is readable by every community member. `null` when
   * no persona is linked or none resolved.
   */
  personaShared: boolean | null;
  status: RuntimeStatusContent | null;
}
