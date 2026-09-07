import assert from "node:assert/strict";
import { test } from "node:test";

import { KINDS } from "@/shared/constants/kinds.ts";
import {
  isChatMessageEvent,
  sortChatMessages,
  toChatMessage,
} from "./timeline.ts";

function event(overrides = {}) {
  return {
    id: "id-1",
    pubkey: "pk-1",
    kind: KINDS.STREAM_MESSAGE,
    content: "hello",
    created_at: 1000,
    tags: [["h", "lobby"]],
    sig: "sig",
    ...overrides,
  };
}

test("isChatMessageEvent accepts kind 9 and 40002, rejects reactions", () => {
  assert.equal(isChatMessageEvent(event({ kind: KINDS.STREAM_MESSAGE })), true);
  assert.equal(
    isChatMessageEvent(event({ kind: KINDS.STREAM_MESSAGE_V2 })),
    true,
  );
  assert.equal(isChatMessageEvent(event({ kind: KINDS.REACTION })), false);
  assert.equal(isChatMessageEvent(event({ kind: 1 })), false);
});

test("toChatMessage extracts fields and has no reply by default", () => {
  const msg = toChatMessage(event());
  assert.equal(msg.id, "id-1");
  assert.equal(msg.pubkey, "pk-1");
  assert.equal(msg.content, "hello");
  assert.equal(msg.createdAt, 1000);
  assert.equal(msg.replyToId, null);
});

test("toChatMessage prefers the marked 'reply' e-tag over an unmarked one", () => {
  const msg = toChatMessage(
    event({
      tags: [
        ["h", "lobby"],
        ["e", "root-id"],
        ["e", "parent-id", "", "reply"],
      ],
    }),
  );
  assert.equal(msg.replyToId, "parent-id");
});

test("toChatMessage falls back to the first e-tag when none is marked 'reply'", () => {
  const msg = toChatMessage(
    event({
      tags: [
        ["h", "lobby"],
        ["e", "root-id"],
      ],
    }),
  );
  assert.equal(msg.replyToId, "root-id");
});

test("sortChatMessages orders chronologically oldest-first", () => {
  const a = toChatMessage(event({ id: "a", created_at: 200 }));
  const b = toChatMessage(event({ id: "b", created_at: 100 }));
  const c = toChatMessage(event({ id: "c", created_at: 300 }));
  assert.deepEqual(
    sortChatMessages([a, b, c]).map((m) => m.id),
    ["b", "a", "c"],
  );
});

test("sortChatMessages breaks same-timestamp ties by id for a stable order", () => {
  const a = toChatMessage(event({ id: "zeta", created_at: 100 }));
  const b = toChatMessage(event({ id: "alpha", created_at: 100 }));
  assert.deepEqual(
    sortChatMessages([a, b]).map((m) => m.id),
    ["alpha", "zeta"],
  );
});

test("sortChatMessages does not mutate its input", () => {
  const a = toChatMessage(event({ id: "a", created_at: 200 }));
  const b = toChatMessage(event({ id: "b", created_at: 100 }));
  const input = [a, b];
  sortChatMessages(input);
  assert.deepEqual(input, [a, b]);
});
