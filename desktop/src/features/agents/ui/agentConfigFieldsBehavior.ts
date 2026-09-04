/** Supported disclosure policies for the shared global-config renderer. */
export type AgentConfigDisclosure =
  | "full"
  | "onboarding-essential"
  | "progressive-defaults";

// Canonical behaviors (PR 2 flag cleanup). These were per-surface props;
// onboarding's values won every call and are now the only behavior:
// - auto-select a valid model when the provider changes
// - keep the model select usable during discovery
// - preserve credential env vars across provider switches (the abandoned
//   provider's key stays in env_vars — visible/deletable under Advanced —
//   so flipping back never loses a typed key; spawned agents may therefore
//   see credentials for providers they don't use)
// - require a provider before model/effort are editable (no saveable
//   invalid state — design principle #4). Note: legacy configs saved with
//   a model but no provider are cleared by the pre-existing orphan-model
//   effect on next edit — deliberate data healing, documented in PR.
const autoSelectModelOnProviderChange = true;
const disableModelSelectDuringDiscovery = false;
const preserveCredentialEnvVarsOnProviderChange = true;
const requireProviderForModelAndEffort = true;

/** The canonical behavior contract, exported for the contract test. */
export const CANONICAL_CONFIG_BEHAVIORS = {
  autoSelectModelOnProviderChange,
  disableModelSelectDuringDiscovery,
  preserveCredentialEnvVarsOnProviderChange,
  requireProviderForModelAndEffort,
} as const;

/**
 * Disclosure preset → the eight visibility decisions it owns. Full and
 * progressive defaults expose the same controls; the progressive preset
 * changes only when those controls are revealed.
 */
export function resolveDisclosure(disclosure: AgentConfigDisclosure) {
  const full = disclosure !== "onboarding-essential";
  return {
    showAdvancedFields: full,
    showCustomModelOption: full,
    showCustomProviderOption: full,
    showDescriptions: full,
    showEffortField: true,
    showProviderPlaceholderOption: full,
    showRequiredIndicators: full,
    showUnavailableEffortOptions: full,
  } as const;
}

/** Whether provider-dependent controls should be visible for this disclosure. */
export function shouldRevealDependentConfigFields({
  disclosure,
  providerFieldVisible,
  providerValue,
}: {
  disclosure: AgentConfigDisclosure;
  providerFieldVisible: boolean;
  providerValue: string;
}): boolean {
  return (
    disclosure !== "progressive-defaults" ||
    !providerFieldVisible ||
    providerValue.trim().length > 0
  );
}

/**
 * Determines whether the status line beneath the Model field should render.
 *
 * Discovery warnings bypass the `onboarding-essential` preset so that a
 * first-run failure is never silently invisible. On the happy path
 * (`status === null`) the status line stays hidden in onboarding.
 */
export function shouldShowModelStatusMessage(
  showDescriptions: boolean,
  status: { message: string; tone: string } | null,
): boolean {
  return showDescriptions || status !== null;
}

/**
 * Whether the Model control should render given discovery state.
 *
 * Optional-model harnesses omit the control while discovery is in flight and
 * after a confirmed successful empty catalog. Failures keep the control so
 * their status can render; required-model harnesses always render it.
 */
export function shouldRenderModelControl({
  discoveredModelOptions,
  modelDiscoveryLoading,
  modelDiscoverySuccessfulEmpty,
  modelIsOptional,
  showCustomModelOption,
}: {
  discoveredModelOptions: readonly { id: string }[] | null;
  modelDiscoveryLoading: boolean;
  /** True only when discovery IPC resolved with a response that yielded no options. */
  modelDiscoverySuccessfulEmpty: boolean;
  modelIsOptional: boolean;
  showCustomModelOption: boolean;
}): boolean {
  if (!modelIsOptional) return true;
  if (modelDiscoveryLoading) return false;
  const hasExplicitModel = (discoveredModelOptions ?? []).some(
    (option) => option.id.trim().length > 0,
  );
  if (hasExplicitModel) return true;
  if (showCustomModelOption) return true;
  return !modelDiscoverySuccessfulEmpty;
}
