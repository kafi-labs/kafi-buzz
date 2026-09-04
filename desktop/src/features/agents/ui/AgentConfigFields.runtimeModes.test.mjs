import assert from "node:assert/strict";
import test from "node:test";

import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { deriveAgentConfigFieldModel } from "../lib/agentConfigCore.ts";
import {
  getGlobalCatalogCredentialState,
  getGlobalCatalogRuntimeFields,
  GlobalCatalogRuntimeDependentFields,
} from "./AgentConfigCatalogRuntimeFields.tsx";
import { AgentConfigFields } from "./AgentConfigFields.tsx";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField.tsx";

function runtime(id, overrides) {
  return {
    id,
    label: id,
    availability: "available",
    command: id,
    defaultArgs: [],
    modelEnvVar: null,
    providerEnvVar: null,
    providerLocked: false,
    requiredNormalizedFields: [],
    apiKeyEnvVar: null,
    thinkingEnvVar: null,
    ...overrides,
  };
}

function renderFields(selectedRuntime, config) {
  return renderToStaticMarkup(
    React.createElement(AgentConfigFields, {
      bakedEnv: [],
      selectedRuntime,
      config,
      isCustomModelEditing: false,
      isCustomProvider: false,
      onConfigChange: () => {},
      onCustomModelEditingChange: () => {},
      onIsCustomProviderChange: () => {},
      useCustomSelect: true,
    }),
  );
}

function findByType(node, type) {
  if (!React.isValidElement(node)) return null;
  if (node.type === type) return node;
  for (const child of React.Children.toArray(node.props.children)) {
    const found = findByType(child, type);
    if (found) return found;
  }
  return null;
}

test("Intel global defaults honor gateway, runtime API key, and agent-name descriptors", () => {
  const html = renderFields(
    runtime("intel", {
      label: "Intelligence Platform",
      modelEnvVar: "INTEL_AGENT",
      providerEnvVar: "INTEL_GATEWAY_URL",
      providerLocked: true,
      requiredNormalizedFields: ["model", "provider"],
      apiKeyEnvVar: "INTEL_API_KEY",
    }),
    {
      env_vars: { INTEL_API_KEY: "intel_test" },
      model: "buzz-e2e-assistant",
      preferred_runtime: "intel",
      provider: "https://intel.example",
    },
  );

  assert.match(html, /Gateway URL/);
  assert.match(html, /global-agent-runtime-gateway-url/);
  assert.match(html, /API key/);
  assert.match(html, /persona-provider-api-key/);
  assert.match(html, /Agent name/);
  assert.match(html, /global-agent-runtime-agent-name/);
  assert.doesNotMatch(html, /data-testid="global-agent-provider"/);
  assert.doesNotMatch(html, /data-testid="global-agent-model"/);
  assert.doesNotMatch(html, /runtime-agent-roster/);
});

test("Intel global API-key edits target the runtime-owned INTEL_API_KEY field", () => {
  const config = {
    env_vars: { INTEL_API_KEY: "intel_test" },
    model: "buzz-e2e-assistant",
    preferred_runtime: "intel",
    provider: "https://intel.example",
  };
  const fieldModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("intel", {
      modelEnvVar: "INTEL_AGENT",
      providerEnvVar: "INTEL_GATEWAY_URL",
      providerLocked: true,
      requiredNormalizedFields: ["model", "provider"],
      apiKeyEnvVar: "INTEL_API_KEY",
    }),
    scope: "global",
  });
  const fields = getGlobalCatalogRuntimeFields(fieldModel);
  assert.ok(fields);
  const credentialState = getGlobalCatalogCredentialState({
    bakedEnvKeys: [],
    envVars: config.env_vars,
    fields,
    runtimeFileConfig: null,
  });
  let changed = null;
  const element = GlobalCatalogRuntimeDependentFields({
    blockClassName: "",
    credentialState,
    disabled: false,
    fieldClassName: "",
    fieldLabelClassName: undefined,
    fields,
    onApiKeyValueChange: (key, value) => {
      changed = { key, value };
    },
    onModelValueChange: () => {},
    showRequiredIndicators: true,
    usePersonaInputStyle: false,
  });
  const apiKeyField = findByType(element, PersonaProviderApiKeyField);
  assert.ok(apiKeyField);

  apiKeyField.props.onValueChange("updated");
  assert.deepEqual(changed, { key: "INTEL_API_KEY", value: "updated" });
});

test("LLM-catalog global defaults keep the existing provider and model controls", () => {
  const html = renderFields(
    runtime("buzz-agent", {
      label: "Buzz Agent",
      modelEnvVar: "BUZZ_AGENT_MODEL",
      providerEnvVar: "BUZZ_AGENT_PROVIDER",
      requiredNormalizedFields: ["model", "provider"],
      thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
    }),
    {
      env_vars: {},
      model: "gpt-4o",
      preferred_runtime: "buzz-agent",
      provider: "openai",
    },
  );

  assert.match(html, /data-testid="global-agent-provider"/);
  assert.match(html, /data-testid="global-agent-model"/);
  assert.doesNotMatch(html, /global-agent-runtime-gateway-url/);
  assert.doesNotMatch(html, /global-agent-runtime-agent-name/);
});
