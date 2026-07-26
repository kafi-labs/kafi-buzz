import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";

export function AgentInstanceRuntimeSelectorFields({
  agentCommand,
  disabled,
  inheritHarness,
  onAgentCommandChange,
  onRuntimeChange,
  runtimeDropdownOptions,
  runtimeDropdownValue,
  selectedRuntime,
  selectedRuntimeId,
}: {
  agentCommand: string;
  disabled: boolean;
  inheritHarness: boolean;
  onAgentCommandChange: (value: string) => void;
  onRuntimeChange: (value: string) => void;
  runtimeDropdownOptions: PersonaDropdownOption[];
  runtimeDropdownValue: string;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  selectedRuntimeId: string;
}) {
  return (
    <>
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-runtime"
        >
          Provider
        </label>
        <PersonaDropdownField
          disabled={disabled}
          id="edit-agent-runtime"
          onValueChange={onRuntimeChange}
          options={runtimeDropdownOptions}
          placeholder="Choose a provider"
          value={runtimeDropdownValue}
        />
        {selectedRuntime ? (
          <p className="text-xs text-muted-foreground">
            Detected at{" "}
            <span className="font-medium">
              {selectedRuntime.binaryPath ??
                selectedRuntime.command ??
                selectedRuntime.id}
            </span>
          </p>
        ) : null}
      </div>
      {selectedRuntimeId === "custom" && !inheritHarness ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-command"
          >
            Agent command
          </label>
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
              disabled={disabled}
              id="edit-agent-command"
              onChange={(event) => onAgentCommandChange(event.target.value)}
              placeholder="Full path or shell command"
              value={agentCommand}
            />
          </div>
        </div>
      ) : null}
    </>
  );
}
