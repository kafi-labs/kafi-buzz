import assert from "node:assert/strict";
import { test } from "node:test";

import {
  Nip46Signer,
  SignerRequestError,
  connectBunker,
  describeBunkerBanner,
} from "./bunker-signer.ts";

test("describeBunkerBanner is silent only when disconnected or connected", () => {
  assert.equal(describeBunkerBanner("disconnected"), null);
  assert.equal(describeBunkerBanner("connected"), null);
});

test("describeBunkerBanner surfaces every other state, never silently", () => {
  for (const state of [
    "connecting",
    "awaiting-approval",
    "denied",
    "unreachable",
    "timed-out",
  ]) {
    assert.notEqual(describeBunkerBanner(state), null, state);
  }
});

test("describeBunkerBanner falls back to a visible 'unknown' banner for an unrecognized state", () => {
  const banner = describeBunkerBanner("something-new");
  assert.notEqual(banner, null);
  assert.equal(banner.tone, "warning");
});

function neverSettles() {
  return new Promise(() => {});
}

test("Nip46Signer.signEvent passes through a successful bunker response", async () => {
  const template = { kind: 9, created_at: 1, tags: [], content: "hi" };
  const signed = { ...template, id: "a", pubkey: "b", sig: "c" };
  const signer = new Nip46Signer({
    getPublicKey: async () => "b",
    signEvent: async () => signed,
    close: async () => {},
  });
  assert.deepEqual(await signer.signEvent(template), signed);
});

test("Nip46Signer classifies an AggregateError-shaped rejection as unreachable, not denied", async () => {
  const signer = new Nip46Signer(
    {
      getPublicKey: async () => {
        throw new AggregateError([new Error("refused")], "all relays failed");
      },
      signEvent: async () => {
        throw new Error("unused");
      },
      close: async () => {},
    },
    50,
  );
  await assert.rejects(
    () => signer.getPublicKey(),
    (error) => {
      assert.ok(error instanceof SignerRequestError);
      assert.equal(error.kind, "unreachable");
      return true;
    },
  );
});

test("Nip46Signer classifies a bunker's explicit error response as denied", async () => {
  const signer = new Nip46Signer(
    {
      getPublicKey: async () => {
        throw new Error("user rejected the request");
      },
      signEvent: async () => {
        throw new Error("unused");
      },
      close: async () => {},
    },
    50,
  );
  await assert.rejects(
    () => signer.getPublicKey(),
    (error) => {
      assert.ok(error instanceof SignerRequestError);
      assert.equal(error.kind, "denied");
      return true;
    },
  );
});

test("Nip46Signer classifies a bunker that never answers as timed-out, not denied", async () => {
  const signer = new Nip46Signer(
    {
      getPublicKey: neverSettles,
      signEvent: neverSettles,
      close: async () => {},
    },
    50,
  );
  await assert.rejects(
    () => signer.getPublicKey(),
    (error) => {
      assert.ok(error instanceof SignerRequestError);
      assert.equal(error.kind, "timed-out");
      return true;
    },
  );
});

test("connectBunker rejects malformed input before any connection attempt, with a plain (non-signer) error", async () => {
  const states = [];
  await assert.rejects(
    () => connectBunker("not-a-bunker-uri-at-all", (s) => states.push(s)),
    /valid bunker/i,
  );
  // No state transition happened -- parsing failed before "connecting".
  assert.deepEqual(states, []);
});
