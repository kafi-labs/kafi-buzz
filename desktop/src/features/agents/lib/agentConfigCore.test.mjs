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

test("Intel runtime projects gateway + agent name via provider/model env vars", () => {
  // Mirrors the desktop catalog projection for KNOWN_ACP_RUNTIMES id=intel
  // (discovery.rs): model_env_var=INTEL_AGENT, provider_env_var=INTEL_GATEWAY_URL.
  // agentConfigCore is the field-descriptor builder used by AgentConfigFields /
  // defaults panels. Note: AgentDefinitionDialog still gates the LLM-provider
  // picker with runtimeSupportsLlmProviderSelection (goose|buzz-agent only), so
  // this projection alone does not make create-dialog gateway URL editable —
  // see Iteration-4 wiring report.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      model: "buzz-e2e-assistant",
      provider: "https://intel-platform.exe.xyz",
    },
    runtime: runtime("intel", {
      label: "Intelligence Platform",
      modelEnvVar: "INTEL_AGENT",
      providerEnvVar: "INTEL_GATEWAY_URL",
      thinkingEnvVar: null,
    }),
    scope: "definition",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["provider", "model"],
  );
  assert.deepEqual(field(model, "provider").targetApplication, {
    kind: "envVar",
    key: "INTEL_GATEWAY_URL",
  });
  assert.equal(
    field(model, "provider").value,
    "https://intel-platform.exe.xyz",
  );
  assert.deepEqual(field(model, "model").targetApplication, {
    kind: "envVar",
    key: "INTEL_AGENT",
  });
  assert.equal(field(model, "model").value, "buzz-e2e-assistant");
  assert.deepEqual(model.omissions, [
    { kind: "effort", reason: "unsupportedByHarness" },
  ]);
});
