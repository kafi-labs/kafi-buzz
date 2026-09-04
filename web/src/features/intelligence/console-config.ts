/**
 * Console deploy configuration.
 *
 * Owner and runner pubkeys are **deploy config, never discovered from event
 * authors**. Discovering them would make the pin circular: an attacker who can
 * publish an event could nominate themselves as a trusted author. These values
 * are baked at build time and are the root of the console's read trust.
 */

function parsePubkeys(raw: string | undefined): string[] {
  if (!raw) return [];
  return raw
    .split(/[,\s]+/)
    .map((key) => key.trim().toLowerCase())
    .filter((key) => /^[0-9a-f]{64}$/.test(key));
}

/** Workspace owner pubkeys whose personas and managed-agent records we trust. */
export const OWNER_PUBKEYS: string[] = parsePubkeys(
  import.meta.env.VITE_CONSOLE_OWNER_PUBKEYS,
);

/** Runner pubkeys whose status and gateway-catalog events we trust. */
export const RUNNER_PUBKEYS: string[] = parsePubkeys(
  import.meta.env.VITE_CONSOLE_RUNNER_PUBKEYS,
);

/** Whether the console has enough configuration to read anything at all. */
export function isConsoleConfigured(): boolean {
  return OWNER_PUBKEYS.length > 0;
}

export const ownerPin = {
  authors: OWNER_PUBKEYS,
  label: "workspace owners (deploy config)",
} as const;

export const runnerPin = {
  authors: RUNNER_PUBKEYS,
  label: "agent runners (deploy config)",
} as const;
