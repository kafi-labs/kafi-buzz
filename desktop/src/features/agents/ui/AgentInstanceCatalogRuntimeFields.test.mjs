import assert from "node:assert/strict";
import test from "node:test";

import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  AgentInstanceCatalogRuntimeFields,
  deriveAgentInstanceCatalogFields,
  hasAgentInstanceCatalogRuntimeFields,
} from "./AgentInstanceCatalogRuntimeFields.tsx";
import {
  IntelAgentRosterFieldView,
  RuntimeAgentNameField,
} from "./RuntimeAgentNameField.tsx";
import { AgentInstanceGenericProviderModelFields } from "./AgentInstanceGenericProviderModelFields.tsx";
import { resolveRuntimeProviderCapability } from "./personaRuntimeModel.ts";

const providerField = {
  kind: "provider",
  mode: "freeText",
  label: "Gateway URL",
  optionSource: "providerCatalog",
  persistence: { kind: "normalizedField", field: "provider" },
  targetApplication: { kind: "envVar", key: "INTEL_GATEWAY_URL" },
  render: "control",
  value: "https://intel.example",
  required: true,
};

const modelField = {
  kind: "model",
  mode: "freeText",
  label: "Agent name",
  optionSource: "acpModels",
  persistence: { kind: "normalizedField", field: "model" },
  targetApplication: { kind: "envVar", key: "INTEL_AGENT" },
  render: "control",
  value: "existing-agent",
  required: true,
};

const apiKeyField = {
  kind: "apiKey",
  label: "API key",
  targetApplication: { kind: "envVar", key: "INTEL_API_KEY" },
  render: "control",
  value: "intel_test",
  required: true,
};

function findByType(node, type) {
  if (!React.isValidElement(node)) return null;
  if (node.type === type) return node;
  for (const child of React.Children.toArray(node.props.children)) {
    const found = findByType(child, type);
    if (found) return found;
  }
  return null;
}

function intelFields(overrides = {}) {
  return AgentInstanceCatalogRuntimeFields({
    apiKeyField,
    disabled: false,
    effectiveApiKey: "intel_effective",
    effectiveGatewayUrl: "https://intel.example",
    envVars: { INTEL_API_KEY: "intel_local" },
    model: "existing-agent",
    modelField,
    onEnvVarValueChange: () => {},
    onModelChange: () => {},
    onProviderChange: () => {},
    open: true,
    provider: "https://intel.example",
    providerField,
    runtimeId: "intel",
    ...overrides,
  });
}

test("intel catalog metadata derives gateway, agent, and API-key instance fields", () => {
  const fields = deriveAgentInstanceCatalogFields({
    envVars: { INTEL_API_KEY: "intel_test" },
    model: "existing-agent",
    provider: "https://intel.example",
    runtime: {
      id: "intel",
      apiKeyEnvVar: "INTEL_API_KEY",
      modelEnvVar: "INTEL_AGENT",
      providerEnvVar: "INTEL_GATEWAY_URL",
      providerLocked: true,
      requiredNormalizedFields: ["model", "provider"],
      thinkingEnvVar: null,
    },
    runtimeId: "intel",
  });

  assert.equal(fields.providerField?.label, "Gateway URL");
  assert.equal(fields.modelField?.label, "Agent name");
  assert.equal(fields.apiKeyField?.targetApplication.key, "INTEL_API_KEY");
  assert.equal(
    resolveRuntimeProviderCapability(
      "intel",
      hasAgentInstanceCatalogRuntimeFields(fields),
    ),
    "capable",
    "the edited gateway must be persisted instead of treated as provider-locked",
  );
});

test("editing an intel instance renders the shared roster field and writes model", () => {
  let model = "existing-agent";
  const element = intelFields({
    onModelChange: (next) => {
      model = next;
    },
  });
  const rosterField = findByType(element, RuntimeAgentNameField);

  assert.ok(
    rosterField,
    "intel instance edit must reuse RuntimeAgentNameField",
  );
  assert.equal(rosterField.props.gatewayUrl, "https://intel.example");
  assert.equal(rosterField.props.apiKey, "intel_effective");
  rosterField.props.onValueChange("chosen-agent");
  assert.equal(model, "chosen-agent");
});

test("editing an intel instance keeps free text when the roster fails", () => {
  const element = intelFields();
  const rosterField = findByType(element, RuntimeAgentNameField);
  assert.ok(rosterField);

  const html = renderToStaticMarkup(
    React.createElement(IntelAgentRosterFieldView, {
      disabled: rosterField.props.disabled,
      onRetry: () => {},
      onValueChange: rosterField.props.onValueChange,
      placeholder: rosterField.props.placeholder,
      state: {
        status: "connectionError",
        message: "Could not reach the Intelligence Platform gateway.",
      },
      value: rosterField.props.value,
    }),
  );

  assert.match(html, /persona-runtime-agent-name/);
  assert.match(html, /Could not reach the Intelligence Platform gateway/);
  assert.match(html, /Retry/);
});

test("non-intel runtime adds no catalog-only fields", () => {
  assert.equal(
    hasAgentInstanceCatalogRuntimeFields({
      apiKeyField: null,
      modelField: null,
      providerField: null,
    }),
    false,
  );
  assert.equal(
    AgentInstanceCatalogRuntimeFields({
      apiKeyField: null,
      disabled: false,
      effectiveApiKey: "",
      effectiveGatewayUrl: "",
      envVars: {},
      model: "claude-sonnet",
      modelField: null,
      onEnvVarValueChange: () => {},
      onModelChange: () => {},
      onProviderChange: () => {},
      open: true,
      provider: "",
      providerField: null,
      runtimeId: "claude",
    }),
    null,
  );
});

test("non-intel instance edit keeps the existing generic provider and model fields", () => {
  const html = renderToStaticMarkup(
    React.createElement(AgentInstanceGenericProviderModelFields, {
      apiKeyInheritedLabel: "",
      apiKeyIsInherited: false,
      apiKeyIsRequired: false,
      apiKeyValue: "",
      disabled: false,
      effectiveProvider: "anthropic",
      isCustomProviderEditing: false,
      llmProviderFieldVisible: true,
      model: "claude-sonnet",
      modelDiscoveryLoading: false,
      modelDropdownOptions: [
        { label: "Claude Sonnet", value: "claude-sonnet" },
      ],
      modelRequired: true,
      modelSelectValue: "claude-sonnet",
      modelStatusMessage: null,
      onModelDropdownChange: () => {},
      onModelValueChange: () => {},
      onProviderDropdownChange: () => {},
      onProviderValueChange: () => {},
      onSecretValueChange: () => {},
      provider: "anthropic",
      providerDropdownOptions: [{ label: "Anthropic", value: "anthropic" }],
      providerRequired: true,
      providerSelectValue: "anthropic",
      showCustomModelInput: false,
      topLevelSecretEnvVar: "ANTHROPIC_API_KEY",
    }),
  );

  assert.match(html, /edit-agent-llm-provider/);
  assert.match(html, /edit-agent-model/);
  assert.match(html, /Anthropic API Key/);
  assert.doesNotMatch(html, /persona-runtime-agent-roster/);
});
