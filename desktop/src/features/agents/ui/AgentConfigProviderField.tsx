import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import {
  AgentConfigTextInput,
  AgentDropdownSelect,
} from "./agentConfigControls";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  type PersonaModelOption,
} from "./agentConfigOptions";

/** Existing LLM-catalog provider control, extracted without behavior changes. */
export function AgentConfigProviderField({
  compactProviderZeroLabel,
  fieldClassName,
  fieldLabelClassName,
  hasBakedProvider,
  isCustomProvider,
  onCustomProviderInput,
  onProviderChange,
  placeholderClassName,
  progressiveDefaults,
  providerOptions,
  providerZeroLabel,
  providerSelectValue,
  providerValue,
  selectClassName,
  showCustomProviderOption,
  showProviderPlaceholderOption,
  useChevronSelectIcon,
  useCustomSelect,
}: {
  compactProviderZeroLabel: string;
  fieldClassName: string;
  fieldLabelClassName: string | undefined;
  hasBakedProvider: boolean;
  isCustomProvider: boolean;
  onCustomProviderInput: (value: string) => void;
  onProviderChange: (value: string) => void;
  placeholderClassName: string | undefined;
  progressiveDefaults: boolean;
  providerOptions: readonly PersonaModelOption[];
  providerZeroLabel: string | null;
  providerSelectValue: string;
  providerValue: string;
  selectClassName: string | undefined;
  showCustomProviderOption: boolean;
  showProviderPlaceholderOption: boolean;
  useChevronSelectIcon: boolean;
  useCustomSelect: boolean;
}) {
  const providerDropdownOptions = [
    ...providerOptions
      .filter(
        (option) =>
          showProviderPlaceholderOption ||
          option.id !== "" ||
          providerSelectValue === AUTO_PROVIDER_DROPDOWN_VALUE,
      )
      .map((option) => ({
        label:
          option.id === ""
            ? showProviderPlaceholderOption
              ? (providerZeroLabel ?? option.label)
              : compactProviderZeroLabel
            : option.label,
        value: option.id || AUTO_PROVIDER_DROPDOWN_VALUE,
      })),
    ...(showCustomProviderOption
      ? [{ label: "Custom provider…", value: CUSTOM_PROVIDER_DROPDOWN_VALUE }]
      : []),
  ];
  const providerSelect = useCustomSelect ? (
    <AgentDropdownSelect
      className={selectClassName}
      id="global-agent-provider"
      onValueChange={onProviderChange}
      options={providerDropdownOptions}
      placeholder={
        showProviderPlaceholderOption
          ? "Select provider"
          : compactProviderZeroLabel
      }
      placeholderClassName={placeholderClassName}
      placeholderValue={
        !showProviderPlaceholderOption && !hasBakedProvider
          ? AUTO_PROVIDER_DROPDOWN_VALUE
          : undefined
      }
      testId="global-agent-provider"
      value={providerSelectValue}
    />
  ) : (
    <select
      className={cn(
        "flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs",
        useChevronSelectIcon && "appearance-none pr-10",
        selectClassName,
      )}
      id="global-agent-provider"
      onChange={(event) => onProviderChange(event.target.value)}
      value={providerSelectValue}
    >
      {providerDropdownOptions.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );

  return (
    <div className={fieldClassName}>
      <label
        className={cn("text-sm font-medium", fieldLabelClassName)}
        htmlFor="global-agent-provider"
      >
        Provider
      </label>
      {!useCustomSelect && useChevronSelectIcon ? (
        <div className="relative">
          {providerSelect}
          <ChevronDown
            aria-hidden="true"
            className="pointer-events-none absolute right-4 top-1/2 h-4 w-4 -translate-y-1/2 text-foreground"
          />
        </div>
      ) : (
        providerSelect
      )}
      {isCustomProvider ? (
        <AgentConfigTextInput
          aria-label="Custom global provider ID"
          autoCorrect="off"
          onChange={(event) => onCustomProviderInput(event.target.value)}
          placeholder="Custom provider ID"
          usePersonaInputStyle={progressiveDefaults}
          value={providerValue}
        />
      ) : null}
    </div>
  );
}
