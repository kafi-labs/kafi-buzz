import assert from "node:assert/strict";
import { test } from "node:test";

// Import the compiled-by-tsc-free path: the join is pure TS with no DOM or
// bundler features, so it runs directly under node's type stripping.
import {
  isHeartbeatStale,
  normalizePersonaSlug,
  resolveWorkspaceAgents,
} from "./join.ts";

const OWNER_A = "a".repeat(64);
const OWNER_B = "b".repeat(64);
const AGENT_1 = "1".repeat(64);
const AGENT_2 = "2".repeat(64);
const RUNNER = "f".repeat(64);

function event({ pubkey, kind, d, content, created_at = 1000 }) {
  return {
    id: `${kind}-${d}-${pubkey}`.slice(0, 64),
    pubkey,
    kind,
    created_at,
    tags: [["d", d]],
    content: JSON.stringify(content),
    sig: "",
  };
}

test("normalizePersonaSlug trims and lowercases", () => {
  assert.equal(normalizePersonaSlug("  Intel-CFO  "), "intel-cfo");
  assert.equal(normalizePersonaSlug("already-fine"), "already-fine");
});

test("joins a managed agent to its persona and prefers persona spawn fields", () => {
  const agents = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: {
          name: "record-name",
          persona_id: "intel-cfo",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [
      event({
        pubkey: OWNER_A,
        kind: 30175,
        d: "intel-cfo",
        content: {
          display_name: "Intel CFO",
          runtime: "intel",
          model: "buzz-cfo-agent",
          provider: "kafi-dev",
        },
      }),
    ],
    [],
  );

  assert.equal(agents.length, 1);
  assert.equal(agents[0].displayName, "Intel CFO");
  assert.equal(agents[0].runtime, "intel");
  assert.equal(agents[0].gatewayAgent, "buzz-cfo-agent");
  assert.equal(agents[0].gatewayId, "kafi-dev");
  assert.equal(agents[0].danglingPersona, false);
});

test("the join is scoped by author — same slug from another owner must not match", () => {
  const agents = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: {
          name: "a-agent",
          persona_id: "shared-slug",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [
      // Same slug, DIFFERENT owner. NIP-AP allows this; a slug-only join would
      // cross-wire the two owners' agents.
      event({
        pubkey: OWNER_B,
        kind: 30175,
        d: "shared-slug",
        content: {
          display_name: "Owner B persona",
          runtime: "intel",
          model: "b-model",
        },
      }),
    ],
    [],
  );

  assert.equal(agents.length, 1);
  assert.equal(
    agents[0].danglingPersona,
    true,
    "must not resolve across owners",
  );
  assert.equal(agents[0].gatewayAgent, null);
  assert.notEqual(agents[0].displayName, "Owner B persona");
});

test("slug normalization difference still joins", () => {
  const agents = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: {
          name: "n",
          persona_id: "  Intel-CFO ",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [
      event({
        pubkey: OWNER_A,
        kind: 30175,
        d: "intel-cfo",
        content: { display_name: "Intel CFO", runtime: "intel", model: "m" },
      }),
    ],
    [],
  );
  assert.equal(agents[0].danglingPersona, false);
});

test("a dangling persona_id is surfaced and yields no spawn config", () => {
  const agents = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_2,
        content: {
          name: "orphan",
          persona_id: "deleted-persona",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [],
    [],
  );

  assert.equal(agents.length, 1);
  assert.equal(agents[0].danglingPersona, true);
  assert.equal(
    agents[0].gatewayAgent,
    null,
    "the slimmed projection carries no model, so there is nothing to spawn",
  );
});

test("no persona_id means the projection itself is authoritative", () => {
  const agents = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: {
          name: "inline",
          model: "inline-model",
          provider: "inline-gateway",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [],
    [],
  );
  assert.equal(agents[0].danglingPersona, false);
  assert.equal(agents[0].gatewayAgent, "inline-model");
});

test("runtime status is attached by agent pubkey", () => {
  const agents = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: { name: "n", parallelism: 1, respond_to: "owner-only" },
      }),
    ],
    [],
    [
      event({
        pubkey: RUNNER,
        kind: 30900,
        d: AGENT_1,
        content: { state: "running", heartbeat_at: 5000, restarts: 0 },
      }),
    ],
  );
  assert.equal(agents[0].status.state, "running");
});

test("persona share state is surfaced, because a shared persona is world-readable", () => {
  const sharedPersona = event({
    pubkey: OWNER_A,
    kind: 30175,
    d: "shared-one",
    content: { display_name: "Shared", runtime: "intel", model: "m" },
  });
  sharedPersona.tags = [
    ["d", "shared-one"],
    ["shared", "true"],
  ];

  const [shared] = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: {
          name: "n",
          persona_id: "shared-one",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [sharedPersona],
    [],
  );
  assert.equal(shared.personaShared, true);

  // No `shared` tag → author-only at the relay.
  const [unshared] = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_1,
        content: {
          name: "n",
          persona_id: "private-one",
          parallelism: 1,
          respond_to: "owner-only",
        },
      }),
    ],
    [
      event({
        pubkey: OWNER_A,
        kind: 30175,
        d: "private-one",
        content: { display_name: "Private", runtime: "intel", model: "m" },
      }),
    ],
    [],
  );
  assert.equal(unshared.personaShared, false);

  // No persona linked at all → nothing to report.
  const [inline] = resolveWorkspaceAgents(
    [
      event({
        pubkey: OWNER_A,
        kind: 30177,
        d: AGENT_2,
        content: { name: "n", parallelism: 1, respond_to: "owner-only" },
      }),
    ],
    [],
    [],
  );
  assert.equal(inline.personaShared, null);
});

test("heartbeat staleness is measured against the reader's clock", () => {
  assert.equal(
    isHeartbeatStale({ state: "running", heartbeat_at: 1000 }, 1030),
    false,
  );
  assert.equal(
    isHeartbeatStale({ state: "running", heartbeat_at: 1000 }, 1200),
    true,
  );
  assert.equal(isHeartbeatStale(null, 1000), true, "no status is stale");
  assert.equal(
    isHeartbeatStale({ state: "running" }, 1000),
    true,
    "a status with no heartbeat is stale",
  );
});
