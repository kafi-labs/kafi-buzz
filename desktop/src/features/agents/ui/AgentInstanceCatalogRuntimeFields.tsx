import {
  deriveAgentConfigFieldModel,
  type AgentConfigFieldDescriptor,
} from "../lib/agentConfigCore";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";
import { RequiredFieldLabel } from "./agentConfigControls";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";
import { RuntimeAgentNameField } from "./RuntimeAgentNameField";

export type AgentInstanceFreeTextProviderField = Extract<
  AgentConfigFieldDescriptor,
  { kind: "provider" }
> & { mode: "freeText" };

export type AgentInstanceFreeTextModelField = Extract<
  AgentConfigFieldDescriptor,
  { kind: "model" }
> & { mode: "freeText" };

export type AgentInstanceApiKeyField = Extract<
  AgentConfigFieldDescriptor,
  { kind: "apiKey" }
>;

export type AgentInstanceCatalogFields = {
  apiKeyField: AgentInstanceApiKeyField | null;
  modelField: AgentInstanceFreeTextModelField | null;
  providerField: AgentInstanceFreeTextProviderField | null;
};

export function deriveAgentInstanceCatalogFields({
  envVars,
  model,
  provider,
  runtime,
  runtimeId,
}: {
  envVars: Record<string, string>;
  model: string;
  provider: string;
  runtime: AcpRuntimeCatalogEntry | undefined;
  runtimeId: string;
}): AgentInstanceCatalogFields {
  const fieldModel = deriveAgentConfigFieldModel({
    config: {
      env_vars: envVars,
      model: model || null,
      preferred_runtime: runtimeId || null,
      provider: provider || null,
    },
    runtime,
    scope: "instance",
  });
  return {
    apiKeyField:
      fieldModel.fields.find(
        (field): field is AgentInstanceApiKeyField => field.kind === "apiKey",
      ) ?? null,
    modelField:
      fieldModel.fields.find(
        (field): field is AgentInstanceFreeTextModelField =>
          field.kind === "model" && field.mode === "freeText",
      ) ?? null,
    providerField:
      fieldModel.fields.find(
        (field): field is AgentInstanceFreeTextProviderField =>
          field.kind === "provider" && field.mode === "freeText",
      ) ?? null,
  };
}

export function hasAgentInstanceCatalogRuntimeFields({
  modelField,
  providerField,
}: AgentInstanceCatalogFields) {
  return providerField !== null || modelField !== null;
}

export function AgentInstanceCatalogRuntimeFields({
  apiKeyField,
  disabled,
  effectiveApiKey,
  effectiveGatewayUrl,
  envVars,
  model,
  modelField,
  onEnvVarValueChange,
  onModelChange,
  onProviderChange,
  open,
  provider,
  providerField,
  runtimeId,
}: AgentInstanceCatalogFields & {
  disabled: boolean;
  effectiveApiKey: string;
  effectiveGatewayUrl: string;
  envVars: Record<string, string>;
  model: string;
  onEnvVarValueChange: (key: string, value: string) => void;
  onModelChange: (value: string) => void;
  onProviderChange: (value: string) => void;
  open: boolean;
  provider: string;
  runtimeId: string;
}) {
  if (
    !hasAgentInstanceCatalogRuntimeFields({
      apiKeyField,
      modelField,
      providerField,
    })
  ) {
    return null;
  }

  const apiKeyEnvVar =
    apiKeyField?.targetApplication.kind === "envVar"
      ? apiKeyField.targetApplication.key
      : null;
  const localApiKey = apiKeyEnvVar ? (envVars[apiKeyEnvVar] ?? "") : "";
  const apiKeyIsInherited =
    localApiKey.trim().length === 0 && effectiveApiKey.trim().length > 0;

  return (
    <>
      {providerField ? (
        <div className="space-y-1.5">
          <RequiredFieldLabel
            htmlFor="edit-agent-runtime-provider-env"
            isRequired={providerField.required}
          >
            {providerField.label}
          </RequiredFieldLabel>
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              data-testid="edit-agent-runtime-gateway-url"
              disabled={disabled}
              id="edit-agent-runtime-provider-env"
              onChange={(event) => onProviderChange(event.target.value)}
              placeholder={
                /URL|GATEWAY/i.test(providerField.targetApplication.key)
                  ? "https://…"
                  : providerField.targetApplication.key
              }
              value={provider}
            />
          </div>
        </div>
      ) : null}

      {apiKeyField && apiKeyEnvVar ? (
        <PersonaProviderApiKeyField
          disabled={disabled}
          inheritedLabel="Inherited from agent defaults or linked definition"
          isInherited={apiKeyIsInherited}
          isRequired={
            apiKeyField.required && effectiveApiKey.trim().length === 0
          }
          label={apiKeyField.label}
          onValueChange={(value) => onEnvVarValueChange(apiKeyEnvVar, value)}
          value={localApiKey}
        />
      ) : null}

      {modelField ? (
        <RuntimeAgentNameField
          apiKey={effectiveApiKey}
          disabled={disabled}
          enabled={open}
          gatewayUrl={effectiveGatewayUrl}
          label={modelField.label}
          onValueChange={onModelChange}
          placeholder={
            modelField.targetApplication.kind === "envVar"
              ? modelField.targetApplication.key
              : "Agent name"
          }
          required={modelField.required}
          runtimeId={runtimeId}
          value={model}
        />
      ) : null}
    </>
  );
}
