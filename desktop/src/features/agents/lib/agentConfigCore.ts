import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { BUZZ_AGENT_THINKING_EFFORT } from "../ui/buzzAgentConfig";

/**
 * Lifecycle status of the ACP runtime catalog query on a per-agent surface.
 *
 * - `loading` — query in flight; structured controls are withheld and env-var
 *   keys are not hidden (saved values remain visible as generic rows).
 * - `ready`   — query resolved; descriptors derived from `selectedRuntime`.
 * - `error`   — query failed; same gate as loading: no structured controls,
 *   keys not hidden, saved values stay visible. Distinguishable from
 *   "runtime not capable" (which is `ready` + no selectedRuntime).
 */
export type RuntimeCatalogStatus = "loading" | "ready" | "error";

export type AgentConfigScope =
  | "onboarding"
  | "global"
  | "definition"
  | "instance";

export type DependentValuePolicy = {
  onContextChange: "resetDependentValues";
  onCatalogMismatch: "explainOnly" | "onboardingCleanup";
};

type NormalizedFieldPersistence = {
  kind: "normalizedField";
  field: "provider" | "model";
};

type EnvVarPersistence = {
  kind: "envVar";
  key: string;
};

type AcpConfigOptionPersistence = {
  kind: "acpConfigOption";
  id: string;
  category: string;
};

type UnavailablePersistence = {
  kind: "unavailable";
};

/** How a provider/model control should be rendered for this runtime. */
export type AgentConfigControlMode =
  /** Anthropic/OpenAI/… catalog dropdown (unlocked LLM provider runtimes). */
  | "llmCatalog"
  /** Free-text bound to provider_env_var / model_env_var (provider_locked runtimes). */
  | "freeText"
  /** ACP-native model selection (no model_env_var). */
  | "acpNative";

export type AgentConfigFieldDescriptor =
  | {
      kind: "provider";
      mode: "llmCatalog" | "freeText";
      /** UI label derived from catalog metadata (not runtime id hardcodes). */
      label: string;
      optionSource: "providerCatalog";
      persistence: NormalizedFieldPersistence;
      targetApplication: { kind: "envVar"; key: string };
      render: "control";
      value: string | null;
      required: boolean;
    }
  | {
      kind: "model";
      mode: "llmCatalog" | "freeText" | "acpNative";
      label: string;
      optionSource: "acpModels";
      persistence: NormalizedFieldPersistence;
      targetApplication:
        | { kind: "envVar"; key: string }
        | { kind: "acpNative" };
      render: "control";
      value: string | null;
      required: boolean;
    }
  | {
      kind: "apiKey";
      label: string;
      targetApplication: { kind: "envVar"; key: string };
      render: "control";
      value: string | null;
      required: boolean;
    }
  | {
      kind: "effort";
      optionSource:
        | "buzzAgentCatalog"
        | "legacyProviderModelCatalog"
        | "harnessNative";
      currentPersistence:
        | EnvVarPersistence
        | AcpConfigOptionPersistence
        | UnavailablePersistence;
      targetApplication:
        | { kind: "envVar"; key: string }
        | { kind: "acpConfigOption"; id: string; category: string };
      render: "control" | "deferredUntilNativeOptionsAvailable";
      value: string | null;
    }
  | {
      kind: "maxOutputTokens" | "contextLimit" | "maxRounds";
      currentPersistence: EnvVarPersistence;
      targetApplication: { kind: "envVar"; key: string };
      render: "control";
      value: string | null;
    };

export type AgentConfigOmission = {
  kind: "effort";
  reason: "ownedByModelId" | "unsupportedByHarness";
};

/**
 * A numeric tuning descriptor: one of the three env-var-backed number fields
 * (max output tokens, context limit, max rounds).
 *
 * Defined here so both the field model derivation and the rendering surfaces
 * share a single type — avoids the type being redefined in UI layers.
 */
export type NumericDescriptor = Extract<
  AgentConfigFieldDescriptor,
  { kind: "maxOutputTokens" | "contextLimit" | "maxRounds" }
>;

export type AgentConfigFieldModel = {
  fields: AgentConfigFieldDescriptor[];
  omissions: AgentConfigOmission[];
  dependentValuePolicy: DependentValuePolicy;
};

/** Catalog slice needed for capability helpers. */
export type RuntimeCapabilityCatalog = Pick<
  AcpRuntimeCatalogEntry,
  | "id"
  | "providerEnvVar"
  | "modelEnvVar"
  | "providerLocked"
  | "requiredNormalizedFields"
  | "apiKeyEnvVar"
>;

/**
 * True when the runtime uses the Anthropic/OpenAI-style LLM provider catalog.
 * Driven by catalog metadata: provider env present and not provider-locked.
 *
 * String overload preserves pre-catalog bootstrap for goose/buzz-agent only.
 */
export function runtimeSupportsLlmProviderSelection(
  runtime: string | RuntimeCapabilityCatalog | null | undefined,
): boolean {
  if (runtime == null) return false;
  if (typeof runtime === "string") {
    const id = runtime.trim();
    return id === "buzz-agent" || id === "goose";
  }
  return Boolean(runtime.providerEnvVar) && !runtime.providerLocked;
}

/**
 * provider_locked + provider_env_var → free-text (gateway URL, etc.).
 */
export function runtimeUsesFreeTextProvider(
  runtime: RuntimeCapabilityCatalog | null | undefined,
): boolean {
  return Boolean(runtime?.providerEnvVar) && Boolean(runtime?.providerLocked);
}

/**
 * provider_locked + model_env_var → free-text agent/model name (not LLM catalog).
 */
export function runtimeUsesFreeTextModel(
  runtime: RuntimeCapabilityCatalog | null | undefined,
): boolean {
  return Boolean(runtime?.modelEnvVar) && Boolean(runtime?.providerLocked);
}

export function runtimeRequiredNormalizedFields(
  runtime: RuntimeCapabilityCatalog | null | undefined,
): readonly string[] {
  return runtime?.requiredNormalizedFields ?? [];
}

export function runtimeApiKeyEnvVar(
  runtime: RuntimeCapabilityCatalog | null | undefined,
): string | null {
  return runtime?.apiKeyEnvVar ?? null;
}

function isRequiredNormalizedField(
  runtime: RuntimeCapabilityCatalog | undefined,
  field: "provider" | "model",
): boolean {
  return runtimeRequiredNormalizedFields(runtime).includes(field);
}

function freeTextProviderLabel(envKey: string): string {
  if (/GATEWAY/i.test(envKey) || /URL/i.test(envKey)) return "Gateway URL";
  return "Provider";
}

function freeTextModelLabel(envKey: string): string {
  if (/AGENT/i.test(envKey)) return "Agent name";
  return "Model";
}

function apiKeyLabel(envKey: string): string {
  if (/API_KEY/i.test(envKey)) return "API key";
  return envKey;
}

function valueFromEnv(config: GlobalAgentConfig, key: string) {
  return config.env_vars[key]?.trim() || null;
}

/**
 * Derives the numeric descriptor set for a runtime from catalog fields.
 *
 * The returned descriptors drive `NumericTuningFields` on any surface that
 * renders numeric knobs. Surfaces pass the same descriptor set to both the
 * renderer and `structuredEnvKeys()` — one policy, no local rebuilding.
 *
 * Returns `[]` when `runtime` is undefined (catalog not yet settled, or the
 * runtime has no numeric env-var fields).
 */
export function deriveNumericDescriptors(
  runtime: AcpRuntimeCatalogEntry | undefined,
): NumericDescriptor[] {
  if (!runtime) return [];
  const ds: NumericDescriptor[] = [];
  if (runtime.maxTokensEnvVar) {
    ds.push({
      kind: "maxOutputTokens",
      currentPersistence: { kind: "envVar", key: runtime.maxTokensEnvVar },
      targetApplication: { kind: "envVar", key: runtime.maxTokensEnvVar },
      render: "control",
      value: null,
    });
  }
  if (runtime.contextLimitEnvVar) {
    ds.push({
      kind: "contextLimit",
      currentPersistence: { kind: "envVar", key: runtime.contextLimitEnvVar },
      targetApplication: { kind: "envVar", key: runtime.contextLimitEnvVar },
      render: "control",
      value: null,
    });
  }
  if (runtime.maxRoundsEnvVar) {
    ds.push({
      kind: "maxRounds",
      currentPersistence: { kind: "envVar", key: runtime.maxRoundsEnvVar },
      targetApplication: { kind: "envVar", key: runtime.maxRoundsEnvVar },
      render: "control",
      value: null,
    });
  }
  return ds;
}

/**
 * Derives the harness-scoped field model consumed by agent config renderers.
 *
 * The runtime catalog is authoritative for environment-variable application.
 * Harness-native ACP options are named here until discovery exposes them to the
 * desktop; descriptors marked deferred must not be rendered as generic fields.
 */
export function deriveAgentConfigFieldModel({
  config,
  runtime,
  scope,
}: {
  config: GlobalAgentConfig;
  runtime: AcpRuntimeCatalogEntry | undefined;
  scope: AgentConfigScope;
}): AgentConfigFieldModel {
  const fields: AgentConfigFieldDescriptor[] = [];
  const omissions: AgentConfigOmission[] = [];

  if (runtime?.providerEnvVar) {
    const freeText = runtimeUsesFreeTextProvider(runtime);
    fields.push({
      kind: "provider",
      mode: freeText ? "freeText" : "llmCatalog",
      label: freeText
        ? freeTextProviderLabel(runtime.providerEnvVar)
        : "LLM provider",
      optionSource: "providerCatalog",
      persistence: { kind: "normalizedField", field: "provider" },
      targetApplication: { kind: "envVar", key: runtime.providerEnvVar },
      render: "control",
      value: config.provider,
      required: isRequiredNormalizedField(runtime, "provider"),
    });
  }

  const freeTextModelEnv = runtimeUsesFreeTextModel(runtime)
    ? runtime?.modelEnvVar
    : null;
  if (freeTextModelEnv) {
    fields.push({
      kind: "model",
      mode: "freeText",
      label: freeTextModelLabel(freeTextModelEnv),
      optionSource: "acpModels",
      persistence: { kind: "normalizedField", field: "model" },
      targetApplication: { kind: "envVar", key: freeTextModelEnv },
      render: "control",
      value: config.model,
      required: isRequiredNormalizedField(runtime, "model"),
    });
  } else {
    fields.push({
      kind: "model",
      mode: runtime?.modelEnvVar ? "llmCatalog" : "acpNative",
      label: "Model",
      optionSource: "acpModels",
      persistence: { kind: "normalizedField", field: "model" },
      targetApplication: runtime?.modelEnvVar
        ? { kind: "envVar", key: runtime.modelEnvVar }
        : { kind: "acpNative" },
      render: "control",
      value: config.model,
      required: isRequiredNormalizedField(runtime, "model"),
    });
  }

  if (runtime?.apiKeyEnvVar) {
    fields.push({
      kind: "apiKey",
      label: apiKeyLabel(runtime.apiKeyEnvVar),
      targetApplication: { kind: "envVar", key: runtime.apiKeyEnvVar },
      render: "control",
      value: valueFromEnv(config, runtime.apiKeyEnvVar),
      required: true,
    });
  }

  if (runtime?.thinkingEnvVar) {
    // targetApplication is always the runtime's native key — how the harness
    // should receive effort. currentPersistence (where the value lives today)
    // is scope-split until PR 2.7 migrates per-agent Goose/Claude:
    //   - global/onboarding: native key, matching the launch projection's global
    //     tier (native-only; the legacy alias is record/persona scope), so a
    //     selection actually reaches the spawn rather than persisting a key the
    //     projection ignores. For buzz-agent this IS BUZZ_AGENT_THINKING_EFFORT.
    //   - definition/instance: still the generic legacy BUZZ_AGENT_THINKING_EFFORT
    //     row, unchanged pending the per-agent migration.
    const nativeKey = runtime.thinkingEnvVar;
    const persistenceKey =
      scope === "global" || scope === "onboarding"
        ? nativeKey
        : BUZZ_AGENT_THINKING_EFFORT;
    fields.push({
      kind: "effort",
      optionSource:
        runtime.id === "buzz-agent" ? "buzzAgentCatalog" : "harnessNative",
      currentPersistence: {
        kind: "envVar",
        key: persistenceKey,
      },
      targetApplication: { kind: "envVar", key: nativeKey },
      render: "control",
      value: valueFromEnv(config, persistenceKey),
    });
  } else if (runtime?.id === "claude") {
    fields.push({
      kind: "effort",
      optionSource: "harnessNative",
      currentPersistence: { kind: "unavailable" },
      targetApplication: {
        kind: "acpConfigOption",
        id: "effort",
        category: "thought_level",
      },
      render: "deferredUntilNativeOptionsAvailable",
      value: null,
    });
  } else {
    omissions.push({
      kind: "effort",
      reason:
        runtime?.id === "codex" ? "ownedByModelId" : "unsupportedByHarness",
    });
  }

  // Numeric fields — derived from the shared helper, then value-populated
  // from config. Any surface needing only the descriptor structure (without
  // saved values) calls deriveNumericDescriptors(runtime) directly.
  for (const d of deriveNumericDescriptors(runtime)) {
    fields.push({
      ...d,
      value: valueFromEnv(config, d.currentPersistence.key),
    });
  }

  return {
    fields,
    omissions,
    dependentValuePolicy: {
      onContextChange: "resetDependentValues",
      onCatalogMismatch:
        scope === "onboarding" ? "onboardingCleanup" : "explainOnly",
    },
  };
}

export function hasRenderableAgentConfigField(
  model: AgentConfigFieldModel,
  kind: AgentConfigFieldDescriptor["kind"],
) {
  return model.fields.some(
    (field) => field.kind === kind && field.render === "control",
  );
}

export function getRenderableEffortField(
  model: AgentConfigFieldModel,
): Extract<AgentConfigFieldDescriptor, { kind: "effort" }> | undefined {
  return model.fields.find(
    (field): field is Extract<AgentConfigFieldDescriptor, { kind: "effort" }> =>
      field.kind === "effort" && field.render === "control",
  );
}

/**
 * Returns the env-var keys owned by the rendered descriptors on a surface.
 *
 * Pass only the descriptors that **actually render controls** on the surface —
 * the resulting key set should be used as `EnvVarsEditor.hiddenKeys` and to
 * exclude keys from baked-row generic display.
 *
 * Invariant: a key appears in the output only when a first-class control for
 * it renders on the surface — a persisted value must never have zero editors.
 *
 * Per-surface consequences (assuming standard descriptor sets):
 * - Global: effort key + numeric keys rendered by the descriptors
 * - Per-agent buzz-agent: effort key + 3 numeric keys
 * - Per-agent Goose: 2 numeric keys only — Goose effort (BUZZ_AGENT_THINKING_EFFORT)
 *   stays a visible generic env row because no effort control renders per-agent
 *   for Goose (effort migration is out of scope)
 */
export function structuredEnvKeys(
  renderedDescriptors: AgentConfigFieldDescriptor[],
): string[] {
  const keys: string[] = [];
  for (const d of renderedDescriptors) {
    if (d.render !== "control") continue;
    if (d.kind === "effort" && d.currentPersistence.kind === "envVar") {
      keys.push(d.currentPersistence.key);
    } else if (
      d.kind === "maxOutputTokens" ||
      d.kind === "contextLimit" ||
      d.kind === "maxRounds"
    ) {
      keys.push(d.currentPersistence.key);
    }
  }
  return keys;
}

/**
 * Filters a baked-env row array to exclude keys already covered by structured
 * controls, preventing double-editing. The result is the set of baked rows
 * that the generic env-vars editor should display.
 *
 * Call with the union of always-structured keys (provider/model/effort set)
 * and numeric structured keys derived from `structuredEnvKeys()`.
 *
 * Pure — suitable for Node-layer unit tests without a component renderer.
 */
export function filterBakedGenericRows<T extends { key: string }>(
  bakedEnv: readonly T[],
  excludeKeys: ReadonlySet<string> | readonly string[],
): T[] {
  const exclude =
    excludeKeys instanceof Set ? excludeKeys : new Set(excludeKeys);
  return bakedEnv.filter((e) => !exclude.has(e.key));
}

/**
 * Returns the placeholder string for a numeric tuning input.
 *
 * When an inherited value is present, the field shows `"Inherit (<value>)"`.
 * When absent (no global setting), the field shows `"Inherit (agent default)"`.
 *
 * Pure — used by NumericTuningFields and testable without a component renderer.
 */
export function numericTuningPlaceholder(
  inheritedValue: string | null | undefined,
): string {
  return inheritedValue
    ? `Inherit (${inheritedValue})`
    : "Inherit (agent default)";
}
