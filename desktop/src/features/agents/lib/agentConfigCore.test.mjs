import assert from "node:assert/strict";
import test from "node:test";

import { deriveAgentConfigFieldModel } from "./agentConfigCore.ts";

const config = {
  env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
  model: "test-model",
  preferred_runtime: null,
  provider: "anthropic",
};

function runtime(id, metadata = {}) {
  return {
    id,
    label: id,
    avatarUrl: "",
    availability: "available",
    command: id,
    binaryPath: id,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    providerLocked: false,
    requiredNormalizedFields: [],
    apiKeyEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
    ...metadata,
  };
}

function field(model, kind) {
  return model.fields.find((candidate) => candidate.kind === kind);
}

test("Buzz Agent exposes provider, model, and Buzz-owned effort", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("buzz-agent", {
      modelEnvVar: "BUZZ_AGENT_MODEL",
      providerEnvVar: "BUZZ_AGENT_PROVIDER",
      thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
    }),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["provider", "model", "effort"],
  );
  assert.equal(field(model, "effort").optionSource, "buzzAgentCatalog");
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "BUZZ_AGENT_THINKING_EFFORT",
  });
});

test("Goose exposes provider, model, and its real effort application key", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      modelEnvVar: "GOOSE_MODEL",
      providerEnvVar: "GOOSE_PROVIDER",
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
    }),
    scope: "global",
  });

  assert.equal(
    field(model, "effort").optionSource,
    "legacyProviderModelCatalog",
  );
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "BUZZ_AGENT_THINKING_EFFORT",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
});

test("Claude models effort as a deferred native ACP option", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("claude"),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model", "effort"],
  );
  assert.equal(
    field(model, "effort").render,
    "deferredUntilNativeOptionsAvailable",
  );
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "unavailable",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "acpConfigOption",
    id: "effort",
    category: "thought_level",
  });
});

test("Codex omits separate effort because model IDs own it", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("codex"),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model"],
  );
  assert.deepEqual(model.omissions, [
    { kind: "effort", reason: "ownedByModelId" },
  ]);
});

test("catalog mismatch cleanup is named and restricted to onboarding", () => {
  const selectedRuntime = runtime("buzz-agent", {
    modelEnvVar: "BUZZ_AGENT_MODEL",
    providerEnvVar: "BUZZ_AGENT_PROVIDER",
    thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
  });
  const onboarding = deriveAgentConfigFieldModel({
    config,
    runtime: selectedRuntime,
    scope: "onboarding",
  });
  const evergreen = deriveAgentConfigFieldModel({
    config,
    runtime: selectedRuntime,
    scope: "instance",
  });

  assert.deepEqual(onboarding.dependentValuePolicy, {
    onContextChange: "resetDependentValues",
    onCatalogMismatch: "onboardingCleanup",
  });
  assert.deepEqual(evergreen.dependentValuePolicy, {
    onContextChange: "resetDependentValues",
    onCatalogMismatch: "explainOnly",
  });
});

test("Intel runtime projects free-text gateway + agent name + API key (no LLM catalog)", () => {
  // Catalog: model_env_var=INTEL_AGENT, provider_env_var=INTEL_GATEWAY_URL,
  // provider_locked=true, required_normalized_fields=[model,provider],
  // api_key_env_var=INTEL_API_KEY. LLM Anthropic/OpenAI dropdowns suppressed.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      model: "buzz-e2e-assistant",
      provider: "https://intel-platform.exe.xyz",
      env_vars: { INTEL_API_KEY: "intel_test" },
    },
    runtime: runtime("intel", {
      label: "Intelligence Platform",
      modelEnvVar: "INTEL_AGENT",
      providerEnvVar: "INTEL_GATEWAY_URL",
      providerLocked: true,
      requiredNormalizedFields: ["model", "provider"],
      apiKeyEnvVar: "INTEL_API_KEY",
      thinkingEnvVar: null,
    }),
    scope: "definition",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["provider", "model", "apiKey"],
  );
  assert.equal(field(model, "provider").mode, "freeText");
  assert.equal(field(model, "provider").label, "Gateway URL");
  assert.equal(field(model, "provider").required, true);
  assert.deepEqual(field(model, "provider").targetApplication, {
    kind: "envVar",
    key: "INTEL_GATEWAY_URL",
  });
  assert.equal(
    field(model, "provider").value,
    "https://intel-platform.exe.xyz",
  );
  assert.equal(field(model, "model").mode, "freeText");
  assert.equal(field(model, "model").label, "Agent name");
  assert.equal(field(model, "model").required, true);
  assert.deepEqual(field(model, "model").targetApplication, {
    kind: "envVar",
    key: "INTEL_AGENT",
  });
  assert.equal(field(model, "model").value, "buzz-e2e-assistant");
  assert.equal(field(model, "apiKey").label, "API key");
  assert.deepEqual(field(model, "apiKey").targetApplication, {
    kind: "envVar",
    key: "INTEL_API_KEY",
  });
  assert.equal(field(model, "apiKey").value, "intel_test");
  // No effort control for intel.
  assert.deepEqual(model.omissions, [
    { kind: "effort", reason: "unsupportedByHarness" },
  ]);
});

test("runtimeSupportsLlmProviderSelection uses catalog providerLocked (intel false)", async () => {
  const { runtimeSupportsLlmProviderSelection } = await import(
    "./agentConfigCore.ts"
  );
  assert.equal(
    runtimeSupportsLlmProviderSelection(
      runtime("intel", {
        providerEnvVar: "INTEL_GATEWAY_URL",
        providerLocked: true,
      }),
    ),
    false,
  );
  assert.equal(
    runtimeSupportsLlmProviderSelection(
      runtime("goose", {
        providerEnvVar: "GOOSE_PROVIDER",
        providerLocked: false,
      }),
    ),
    true,
  );
});
