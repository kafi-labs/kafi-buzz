import assert from "node:assert/strict";
import { test } from "node:test";

import { isSendableContent, shouldSubmitOnKeyDown } from "./composer.ts";

test("isSendableContent rejects empty and whitespace-only content", () => {
  assert.equal(isSendableContent(""), false);
  assert.equal(isSendableContent("   "), false);
  assert.equal(isSendableContent("\n\t "), false);
});

test("isSendableContent accepts real content, trimming outer whitespace", () => {
  assert.equal(isSendableContent("hello"), true);
  assert.equal(isSendableContent("  hello  "), true);
});

test("shouldSubmitOnKeyDown: plain Enter submits", () => {
  assert.equal(shouldSubmitOnKeyDown({ key: "Enter", shiftKey: false }), true);
});

test("shouldSubmitOnKeyDown: Shift+Enter does not submit (inserts a newline)", () => {
  assert.equal(shouldSubmitOnKeyDown({ key: "Enter", shiftKey: true }), false);
});

test("shouldSubmitOnKeyDown: non-Enter keys never submit", () => {
  assert.equal(shouldSubmitOnKeyDown({ key: "a", shiftKey: false }), false);
  assert.equal(shouldSubmitOnKeyDown({ key: "Tab", shiftKey: false }), false);
});
