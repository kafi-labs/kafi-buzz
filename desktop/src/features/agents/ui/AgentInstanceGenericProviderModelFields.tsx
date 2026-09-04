import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";

export function AgentInstanceGenericProviderModelFields({
  apiKeyInheritedLabel,
  apiKeyIsInherited,
  apiKeyIsRequired,
  apiKeyValue,
  disabled,
  effectiveProvider,
  isCustomProviderEditing,
  llmProviderFieldVisible,
  model,
  modelDiscoveryLoading,
  modelDropdownOptions,
  modelRequired,
  modelSelectValue,
  modelStatusMessage,
  onModelDropdownChange,
  onModelValueChange,
  onProviderDropdownChange,
  onProviderValueChange,
  onSecretValueChange,
  provider,
  providerDropdownOptions,
  providerRequired,
  providerSelectValue,
  showCustomModelInput,
  topLevelSecretEnvVar,
}: {
  apiKeyInheritedLabel: string;
  apiKeyIsInherited: boolean;
  apiKeyIsRequired: boolean;
  apiKeyValue: string;
  disabled: boolean;
  effectiveProvider: string;
  isCustomProviderEditing: boolean;
  llmProviderFieldVisible: boolean;
  model: string;
  modelDiscoveryLoading: boolean;
  modelDropdownOptions: PersonaDropdownOption[];
  modelRequired: boolean;
  modelSelectValue: string;
  modelStatusMessage: string | null;
  onModelDropdownChange: (value: string) => void;
  onModelValueChange: (value: string) => void;
  onProviderDropdownChange: (value: string) => void;
  onProviderValueChange: (value: string) => void;
  onSecretValueChange: (key: string, value: string) => void;
  provider: string;
  providerDropdownOptions: PersonaDropdownOption[];
  providerRequired: boolean;
  providerSelectValue: string;
  showCustomModelInput: boolean;
  topLevelSecretEnvVar: string | null;
}) {
  return (
    <>
      {llmProviderFieldVisible ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-llm-provider"
          >
            LLM provider
            {providerRequired ? (
              <span className="ml-1 text-destructive" aria-hidden="true">
                *
              </span>
            ) : (
              <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
            )}
          </label>
          <PersonaDropdownField
            disabled={disabled}
            id="edit-agent-llm-provider"
            onValueChange={onProviderDropdownChange}
            options={providerDropdownOptions}
            placeholder="Default (auto)"
            value={providerSelectValue}
          />
          {isCustomProviderEditing ? (
            <div
              className={cn(
                "mt-2 flex min-h-11 items-center px-3",
                PERSONA_FIELD_SHELL_CLASS,
              )}
            >
              <Input
                aria-label="Custom provider ID"
                autoCorrect="off"
                className={cn(
                  "h-8 px-0 py-0 leading-6",
                  PERSONA_FIELD_CONTROL_CLASS,
                )}
                disabled={disabled}
                id="edit-agent-custom-provider"
                onChange={(event) => onProviderValueChange(event.target.value)}
                placeholder="Custom provider ID"
                value={provider}
              />
            </div>
          ) : null}
        </div>
      ) : null}

      {llmProviderFieldVisible && topLevelSecretEnvVar ? (
        <PersonaProviderApiKeyField
          disabled={disabled}
          inheritedLabel={apiKeyInheritedLabel}
          isInherited={apiKeyIsInherited}
          isRequired={apiKeyIsRequired}
          label={
            effectiveProvider === "anthropic"
              ? "Anthropic API Key"
              : "OpenAI API Key"
          }
          onValueChange={(value) =>
            onSecretValueChange(topLevelSecretEnvVar, value)
          }
          value={apiKeyValue}
        />
      ) : null}

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-model"
        >
          Model
          {modelRequired ? (
            <span className="ml-1 text-destructive" aria-hidden="true">
              *
            </span>
          ) : (
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          )}
        </label>
        <PersonaDropdownField
          disabled={disabled || modelDiscoveryLoading}
          id="edit-agent-model"
          onValueChange={onModelDropdownChange}
          options={modelDropdownOptions}
          placeholder="Default model"
          value={modelSelectValue}
        />
        {showCustomModelInput ? (
          <div
            className={cn(
              "mt-2 flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              aria-label="Custom model ID"
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled}
              id="edit-agent-custom-model"
              onChange={(event) => onModelValueChange(event.target.value)}
              placeholder="Custom model ID"
              value={model}
            />
          </div>
        ) : null}
        {modelStatusMessage ? (
          <p className="text-xs text-muted-foreground">{modelStatusMessage}</p>
        ) : null}
      </div>
    </>
  );
}
