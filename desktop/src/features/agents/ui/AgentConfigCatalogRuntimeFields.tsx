import type { RuntimeFileConfigSubset } from "@/shared/api/tauri";
import type {
  AgentConfigFieldDescriptor,
  AgentConfigFieldModel,
} from "../lib/agentConfigCore";
import {
  AgentConfigTextInput,
  RequiredFieldLabel,
} from "./agentConfigControls";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";

type FreeTextProviderField = Extract<
  AgentConfigFieldDescriptor,
  { kind: "provider" }
> & { mode: "freeText" };

type FreeTextModelField = Extract<
  AgentConfigFieldDescriptor,
  { kind: "model" }
> & { mode: "freeText" };

type ApiKeyField = Extract<AgentConfigFieldDescriptor, { kind: "apiKey" }>;

/** Free-text provider/model descriptors and optional runtime-owned credential. */
export type GlobalCatalogRuntimeFields = {
  apiKeyField: ApiKeyField | null;
  modelField: FreeTextModelField;
  providerField: FreeTextProviderField;
};

/** Effective runtime-owned credential state across local, build, and file layers. */
export type GlobalCatalogCredentialState = {
  apiKeyEnvVar: string | null;
  apiKeyFileSatisfied: boolean;
  apiKeyInherited: boolean;
  apiKeyValue: string;
};

/**
 * Selects the catalog-owned free-text field set used by runtimes such as the
 * Intelligence Platform. Mixed-mode runtimes remain on the established
 * renderer until that combination has an explicit product design.
 */
export function getGlobalCatalogRuntimeFields(
  fieldModel: AgentConfigFieldModel,
): GlobalCatalogRuntimeFields | null {
  const providerField = fieldModel.fields.find(
    (field): field is FreeTextProviderField =>
      field.kind === "provider" && field.mode === "freeText",
  );
  const modelField = fieldModel.fields.find(
    (field): field is FreeTextModelField =>
      field.kind === "model" && field.mode === "freeText",
  );
  if (!providerField || !modelField) return null;

  return {
    apiKeyField:
      fieldModel.fields.find(
        (field): field is ApiKeyField => field.kind === "apiKey",
      ) ?? null,
    modelField,
    providerField,
  };
}

/** Resolves the runtime-owned API-key field without consulting LLM providers. */
export function getGlobalCatalogCredentialState({
  bakedEnvKeys,
  envVars,
  fields,
  runtimeFileConfig,
}: {
  bakedEnvKeys: readonly string[];
  envVars: Record<string, string>;
  fields: GlobalCatalogRuntimeFields;
  runtimeFileConfig: RuntimeFileConfigSubset | null | undefined;
}): GlobalCatalogCredentialState {
  const apiKeyEnvVar =
    fields.apiKeyField?.targetApplication.kind === "envVar"
      ? fields.apiKeyField.targetApplication.key
      : null;
  const apiKeyValue = apiKeyEnvVar ? (envVars[apiKeyEnvVar] ?? "") : "";
  const locallyDefined = apiKeyEnvVar !== null && apiKeyEnvVar in envVars;
  const apiKeyBakedSatisfied =
    apiKeyEnvVar !== null &&
    !locallyDefined &&
    bakedEnvKeys.includes(apiKeyEnvVar);
  const apiKeyFileSatisfied =
    apiKeyEnvVar !== null &&
    !locallyDefined &&
    !apiKeyBakedSatisfied &&
    (runtimeFileConfig?.satisfiedEnvKeys.includes(apiKeyEnvVar) ?? false);
  const apiKeyInherited =
    apiKeyEnvVar !== null &&
    apiKeyValue.trim().length === 0 &&
    !locallyDefined &&
    (apiKeyBakedSatisfied || apiKeyFileSatisfied);

  return {
    apiKeyEnvVar,
    apiKeyFileSatisfied,
    apiKeyInherited,
    apiKeyValue,
  };
}

/** Validates required catalog-owned fields using their effective values. */
export function globalCatalogRuntimeFieldsAreValid({
  credentialState,
  fields,
}: {
  credentialState: GlobalCatalogCredentialState;
  fields: GlobalCatalogRuntimeFields;
}): boolean {
  const providerValid =
    !fields.providerField.required ||
    (fields.providerField.value?.trim().length ?? 0) > 0;
  const modelValid =
    !fields.modelField.required ||
    (fields.modelField.value?.trim().length ?? 0) > 0;
  const apiKeyValid =
    !fields.apiKeyField?.required ||
    credentialState.apiKeyInherited ||
    credentialState.apiKeyValue.trim().length > 0;
  return providerValid && modelValid && apiKeyValid;
}

/** Renders a catalog-owned free-text provider field such as a gateway URL. */
export function GlobalCatalogRuntimeProviderField({
  field,
  fieldClassName,
  fieldLabelClassName,
  onValueChange,
  showRequiredIndicator,
  usePersonaInputStyle,
}: {
  field: FreeTextProviderField;
  fieldClassName: string;
  fieldLabelClassName: string | undefined;
  onValueChange: (value: string) => void;
  showRequiredIndicator: boolean;
  usePersonaInputStyle: boolean;
}) {
  return (
    <div className={fieldClassName}>
      <RequiredFieldLabel
        className={fieldLabelClassName}
        htmlFor="global-agent-runtime-gateway-url"
        isRequired={showRequiredIndicator && field.required}
      >
        {field.label}
      </RequiredFieldLabel>
      <AgentConfigTextInput
        autoCorrect="off"
        data-testid="global-agent-runtime-gateway-url"
        id="global-agent-runtime-gateway-url"
        onChange={(event) => onValueChange(event.target.value)}
        placeholder={field.targetApplication.key}
        usePersonaInputStyle={usePersonaInputStyle}
        value={field.value ?? ""}
      />
    </div>
  );
}

/** Renders the runtime-owned credential and free-text agent/model field. */
export function GlobalCatalogRuntimeDependentFields({
  blockClassName,
  credentialState,
  disabled,
  fieldClassName,
  fieldLabelClassName,
  fields,
  onApiKeyValueChange,
  onModelValueChange,
  showRequiredIndicators,
  usePersonaInputStyle,
}: {
  blockClassName: string;
  credentialState: GlobalCatalogCredentialState;
  disabled: boolean;
  fieldClassName: string;
  fieldLabelClassName: string | undefined;
  fields: GlobalCatalogRuntimeFields;
  onApiKeyValueChange: (key: string, value: string) => void;
  onModelValueChange: (value: string) => void;
  showRequiredIndicators: boolean;
  usePersonaInputStyle: boolean;
}) {
  const apiKeyEnvVar = credentialState.apiKeyEnvVar;
  return (
    <>
      {fields.apiKeyField && apiKeyEnvVar ? (
        <div className={blockClassName}>
          <PersonaProviderApiKeyField
            disabled={false}
            inheritedLabel={
              credentialState.apiKeyFileSatisfied
                ? "Set in runtime config"
                : "Provided by this build"
            }
            isInherited={credentialState.apiKeyInherited}
            isRequired={
              showRequiredIndicators &&
              fields.apiKeyField.required &&
              !credentialState.apiKeyInherited &&
              credentialState.apiKeyValue.trim().length === 0
            }
            label={fields.apiKeyField.label}
            onValueChange={(value) => onApiKeyValueChange(apiKeyEnvVar, value)}
            value={credentialState.apiKeyValue}
          />
        </div>
      ) : null}

      <div className={fieldClassName}>
        <RequiredFieldLabel
          className={fieldLabelClassName}
          htmlFor="global-agent-runtime-agent-name"
          isRequired={
            showRequiredIndicators && fields.modelField.required && !disabled
          }
        >
          {fields.modelField.label}
        </RequiredFieldLabel>
        <AgentConfigTextInput
          autoCorrect="off"
          data-testid="global-agent-runtime-agent-name"
          disabled={disabled}
          id="global-agent-runtime-agent-name"
          onChange={(event) => onModelValueChange(event.target.value)}
          placeholder={
            fields.modelField.targetApplication.kind === "envVar"
              ? fields.modelField.targetApplication.key
              : "Agent name"
          }
          usePersonaInputStyle={usePersonaInputStyle}
          value={disabled ? "" : (fields.modelField.value ?? "")}
        />
      </div>
    </>
  );
}
