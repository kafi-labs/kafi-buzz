import assert from "node:assert/strict";
import { test } from "node:test";

import { isPinnedAuthor, UnpinnedReadError } from "./author-pinned-query.ts";
import { dedupeReplaceable } from "./nostr-tags.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);

test("isPinnedAuthor accepts only pinned authors, case-insensitively", () => {
  const pin = { authors: [A.toUpperCase()], label: "test" };
  assert.equal(isPinnedAuthor({ pubkey: A }, pin), true);
  assert.equal(isPinnedAuthor({ pubkey: B }, pin), false);
});

test("a malformed pubkey is not a pin — it must not silently widen the set", () => {
  assert.equal(
    isPinnedAuthor({ pubkey: A }, { authors: ["nope"], label: "t" }),
    false,
  );
  assert.equal(
    isPinnedAuthor({ pubkey: A }, { authors: [], label: "t" }),
    false,
  );
});

test("queryPinned refuses an empty pin instead of reading from anyone", async () => {
  const { queryPinned } = await import("./author-pinned-query.ts");
  await assert.rejects(
    () =>
      queryPinned(
        "ws://unused",
        { kinds: [30177] },
        { authors: [], label: "empty" },
      ),
    (error) => {
      assert.ok(error instanceof UnpinnedReadError);
      assert.match(error.message, /unpinned relay read/i);
      return true;
    },
  );
});

test("queryPinned refuses a kindless filter — it would trip the relay p-gate", async () => {
  const { queryPinned } = await import("./author-pinned-query.ts");
  await assert.rejects(
    () => queryPinned("ws://unused", {}, { authors: [A], label: "kindless" }),
    (error) => {
      assert.ok(error instanceof UnpinnedReadError);
      assert.match(error.message, /kindless/i);
      return true;
    },
  );
});

test("dedupeReplaceable keeps one revision per (author, kind, d) and never merges authors", () => {
  const make = (pubkey, created_at, id) => ({
    id,
    pubkey,
    kind: 30177,
    created_at,
    tags: [["d", "agent-1"]],
    content: "{}",
    sig: "",
  });

  // Same author, two revisions → newest wins.
  const sameAuthor = dedupeReplaceable([make(A, 100, "x"), make(A, 200, "y")]);
  assert.equal(sameAuthor.length, 1);
  assert.equal(sameAuthor[0].created_at, 200);

  // Two authors, same d → BOTH survive. Collapsing them by recency is the
  // forgery path: created_at is author-supplied, so a forged row could
  // out-sort a genuine one.
  const twoAuthors = dedupeReplaceable([make(A, 100, "x"), make(B, 999, "z")]);
  assert.equal(twoAuthors.length, 2);
});

test("same-second ties break by lowest event id, matching the relay", () => {
  const make = (id) => ({
    id,
    pubkey: A,
    kind: 30177,
    created_at: 500,
    tags: [["d", "agent-1"]],
    content: "{}",
    sig: "",
  });
  const result = dedupeReplaceable([make("bbbb"), make("aaaa")]);
  assert.equal(result.length, 1);
  assert.equal(result[0].id, "aaaa");
});
