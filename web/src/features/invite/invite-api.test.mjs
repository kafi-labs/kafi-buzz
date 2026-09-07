import assert from "node:assert/strict";
import { test } from "node:test";

import { channelIdFromMetadata, isOpenChannelMetadata } from "./invite-api.ts";

test("isOpenChannelMetadata accepts the NIP-29 single-element 'public' tag", () => {
  assert.equal(isOpenChannelMetadata({ tags: [["public"]] }), true);
});

test("isOpenChannelMetadata rejects 'private' and rejects a missing tag", () => {
  assert.equal(isOpenChannelMetadata({ tags: [["private"]] }), false);
  assert.equal(isOpenChannelMetadata({ tags: [] }), false);
  assert.equal(isOpenChannelMetadata({}), false);
});

test("isOpenChannelMetadata does not match 'public' as part of a longer tag", () => {
  // A multi-value tag happening to start with "public" (e.g. a future
  // ["public", "reason"]) is not the visibility marker — only the bare
  // single-element form is, matching the relay's own emission.
  assert.equal(isOpenChannelMetadata({ tags: [["public", "extra"]] }), false);
});

test("channelIdFromMetadata extracts the channel id from the d-tag", () => {
  assert.equal(
    channelIdFromMetadata({
      tags: [["d", "channel-123"], ["public"]],
    }),
    "channel-123",
  );
});

test("channelIdFromMetadata returns null when there is no d-tag", () => {
  assert.equal(channelIdFromMetadata({ tags: [["public"]] }), null);
  assert.equal(channelIdFromMetadata({}), null);
});
