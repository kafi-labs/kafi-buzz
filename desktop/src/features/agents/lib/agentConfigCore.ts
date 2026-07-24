import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { BUZZ_AGENT_THINKING_EFFORT } from "../ui/buzzAgentConfig";

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
    };

export type AgentConfigOmission = {
  kind: "effort";
  reason: "ownedByModelId" | "unsupportedByHarness";
};

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
    fields.push({
      kind: "effort",
      optionSource:
        runtime.id === "buzz-agent"
          ? "buzzAgentCatalog"
          : "legacyProviderModelCatalog",
      currentPersistence: {
        kind: "envVar",
        key: BUZZ_AGENT_THINKING_EFFORT,
      },
      targetApplication: { kind: "envVar", key: runtime.thinkingEnvVar },
      render: "control",
      value: valueFromEnv(config, BUZZ_AGENT_THINKING_EFFORT),
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
