import assert from "node:assert/strict";
import { test } from "node:test";

import {
  describeConnectionBanner,
  deriveTimelineViewState,
} from "./view-state.ts";

test("deriveTimelineViewState: never-loaded + connection down is 'unreachable', not 'empty'", () => {
  assert.equal(
    deriveTimelineViewState({
      connectionState: "error",
      isLoadingHistory: true,
      messageCount: 0,
    }),
    "unreachable",
  );
  assert.equal(
    deriveTimelineViewState({
      connectionState: "disconnected",
      isLoadingHistory: false,
      messageCount: 0,
    }),
    "unreachable",
  );
});

test("deriveTimelineViewState: connected but still awaiting EOSE is 'loading'", () => {
  assert.equal(
    deriveTimelineViewState({
      connectionState: "connected",
      isLoadingHistory: true,
      messageCount: 0,
    }),
    "loading",
  );
});

test("deriveTimelineViewState: connected, history loaded, nothing there is a genuine 'empty'", () => {
  assert.equal(
    deriveTimelineViewState({
      connectionState: "connected",
      isLoadingHistory: false,
      messageCount: 0,
    }),
    "empty",
  );
});

test("deriveTimelineViewState: known-good messages stay 'ready' through a later disconnect", () => {
  assert.equal(
    deriveTimelineViewState({
      connectionState: "disconnected",
      isLoadingHistory: false,
      messageCount: 3,
    }),
    "ready",
  );
  assert.equal(
    deriveTimelineViewState({
      connectionState: "error",
      isLoadingHistory: true,
      messageCount: 1,
    }),
    "ready",
  );
});

test("describeConnectionBanner is silent (null) only when connected", () => {
  assert.equal(describeConnectionBanner("connected"), null);
});

test("describeConnectionBanner surfaces every other state, never silently", () => {
  for (const state of [
    "idle",
    "connecting",
    "authenticating",
    "disconnected",
    "error",
  ]) {
    const banner = describeConnectionBanner(state);
    assert.notEqual(banner, null, `state '${state}' must not be silent`);
    assert.equal(typeof banner.label, "string");
    assert.ok(banner.label.length > 0);
  }
});

test("describeConnectionBanner includes the error detail when given", () => {
  const banner = describeConnectionBanner("error", "socket closed");
  assert.match(banner.label, /socket closed/);
});

test("describeConnectionBanner falls back to a visible 'unknown' banner for an unrecognized state", () => {
  const banner = describeConnectionBanner("some-future-state");
  assert.notEqual(banner, null);
  assert.equal(banner.tone, "warning");
});
