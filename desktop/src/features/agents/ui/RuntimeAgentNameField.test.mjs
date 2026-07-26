import assert from "node:assert/strict";
import test from "node:test";

import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { TauriInvokeError } from "@/shared/api/tauri";
import { intelAgentRosterFailure } from "@/shared/api/intelAgentRoster";
import {
  IntelAgentRosterFieldView,
  intelAgentRosterErrorState,
} from "./RuntimeAgentNameField.tsx";

const agents = [
  {
    id: "agent-1",
    name: "builder-sandbox-build_19ccd48c1b8345f0-f822bd9c",
    description: "Build agent",
  },
  {
    id: "agent-2",
    name: "buzz-cfo-agent",
    description: "Finance Q&A",
  },
];

function render(state, value = "") {
  return renderToStaticMarkup(
    React.createElement(IntelAgentRosterFieldView, {
      disabled: false,
      onRetry: () => {},
      onValueChange: () => {},
      placeholder: "INTEL_AGENT",
      state,
      value,
    }),
  );
}

function findByTestId(node, testId) {
  if (!React.isValidElement(node)) return null;
  if (node.props["data-testid"] === testId) return node;
  for (const child of React.Children.toArray(node.props.children)) {
    const found = findByTestId(child, testId);
    if (found) return found;
  }
  return null;
}

test("populated roster renders gateway agents and keeps manual entry", () => {
  const html = render({ status: "populated", agents });

  assert.match(html, /persona-runtime-agent-roster/);
  assert.match(html, /builder-sandbox-build_19ccd48c1b8345f0-f822bd9c/);
  assert.match(html, /buzz-cfo-agent/);
  assert.match(html, /persona-runtime-agent-name/);
});

test("choosing a roster option writes the existing model value", () => {
  let model = "";
  const element = IntelAgentRosterFieldView({
    disabled: false,
    onRetry: () => {},
    onValueChange: (next) => {
      model = next;
    },
    placeholder: "INTEL_AGENT",
    state: { status: "populated", agents },
    value: model,
  });
  const select = findByTestId(element, "persona-runtime-agent-roster");

  assert.ok(select, "roster select must render");
  select.props.onChange({ target: { value: "buzz-cfo-agent" } });
  assert.equal(model, "buzz-cfo-agent");
});

test("connection failure visibly falls back to free-text with retry", () => {
  const failedFetchState = intelAgentRosterErrorState(
    new TauriInvokeError("safe connection message", {
      code: "connection",
      message: "Could not reach the Intelligence Platform gateway.",
    }),
  );
  const html = render(failedFetchState);

  assert.equal(failedFetchState.status, "connectionError");
  assert.match(html, /persona-runtime-agent-name/);
  assert.match(html, /Could not reach the Intelligence Platform gateway/);
  assert.match(html, /Retry/);
  assert.doesNotMatch(html, /persona-runtime-agent-roster"/);
});

test("auth and connection failures have distinct user guidance", () => {
  const authHtml = render({
    status: "authError",
    message: "The gateway rejected the API key.",
  });
  const connectionHtml = render({
    status: "connectionError",
    message: "Could not reach the Intelligence Platform gateway.",
  });

  assert.match(authHtml, /rejected the API key/);
  assert.doesNotMatch(authHtml, /Could not reach/);
  assert.match(connectionHtml, /Could not reach/);
  assert.doesNotMatch(connectionHtml, /rejected the API key/);
});

test("structured Tauri errors preserve auth versus connection classification", () => {
  const auth = intelAgentRosterFailure(
    new TauriInvokeError("safe auth message", {
      code: "auth",
      message: "The gateway rejected the API key.",
    }),
  );
  const connection = intelAgentRosterFailure(
    new TauriInvokeError("safe connection message", {
      code: "connection",
      message: "Could not reach the Intelligence Platform gateway.",
    }),
  );

  assert.equal(auth.code, "auth");
  assert.equal(connection.code, "connection");
  assert.notEqual(auth.message, connection.message);
});

test("idle, loading, and empty roster states are explicit", () => {
  assert.match(render({ status: "idle" }), /Enter the gateway URL and API key/);
  assert.match(
    render({ status: "loading" }),
    /Loading agents from the gateway/,
  );
  const emptyHtml = render({ status: "empty" });
  assert.match(emptyHtml, /gateway returned no agents/);
  assert.match(emptyHtml, /persona-runtime-agent-name/);
});
