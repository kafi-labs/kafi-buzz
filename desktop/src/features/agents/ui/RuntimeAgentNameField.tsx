import * as React from "react";
import { LoaderCircle, RefreshCw } from "lucide-react";

import {
  intelAgentRosterFailure,
  listIntelAgents,
  type IntelAgentRosterEntry,
} from "@/shared/api/intelAgentRoster";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

export type IntelAgentRosterState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "populated"; agents: IntelAgentRosterEntry[] }
  | { status: "empty" }
  | { status: "authError"; message: string }
  | { status: "connectionError"; message: string }
  | { status: "malformedError"; message: string };

export function intelAgentRosterErrorState(
  error: unknown,
): IntelAgentRosterState {
  const failure = intelAgentRosterFailure(error);
  if (failure.code === "auth") {
    return { status: "authError", message: failure.message };
  }
  if (failure.code === "malformedOutput") {
    return { status: "malformedError", message: failure.message };
  }
  return { status: "connectionError", message: failure.message };
}

type RuntimeAgentNameFieldProps = {
  apiKey: string;
  disabled: boolean;
  enabled: boolean;
  gatewayUrl: string;
  label: string;
  onValueChange: (value: string) => void;
  placeholder: string;
  required: boolean;
  runtimeId: string;
  value: string;
};

type IntelAgentRosterFieldViewProps = {
  disabled: boolean;
  onRetry: () => void;
  onValueChange: (value: string) => void;
  placeholder: string;
  state: IntelAgentRosterState;
  value: string;
};

export function RuntimeAgentNameField({
  apiKey,
  disabled,
  enabled,
  gatewayUrl,
  label,
  onValueChange,
  placeholder,
  required,
  runtimeId,
  value,
}: RuntimeAgentNameFieldProps) {
  const { retry, state } = useIntelAgentRoster({
    apiKey,
    enabled: enabled && runtimeId === "intel",
    gatewayUrl,
  });

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel
        htmlFor="persona-runtime-model-env"
        isRequired={required}
      >
        {label}
      </RequiredFieldLabel>
      {runtimeId === "intel" ? (
        <IntelAgentRosterFieldView
          disabled={disabled}
          onRetry={retry}
          onValueChange={onValueChange}
          placeholder={placeholder}
          state={state}
          value={value}
        />
      ) : (
        <AgentNameInput
          disabled={disabled}
          onValueChange={onValueChange}
          placeholder={placeholder}
          value={value}
        />
      )}
    </div>
  );
}

export function IntelAgentRosterFieldView({
  disabled,
  onRetry,
  onValueChange,
  placeholder,
  state,
  value,
}: IntelAgentRosterFieldViewProps) {
  const selectedRosterName =
    state.status === "populated" &&
    state.agents.some((agent) => agent.name === value)
      ? value
      : "";

  return (
    <div className="space-y-2">
      {state.status === "populated" ? (
        <div className={PERSONA_FIELD_SHELL_CLASS}>
          <select
            className={cn(
              "h-11 w-full bg-transparent px-3 py-2 text-sm leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            data-testid="persona-runtime-agent-roster"
            disabled={disabled}
            id="persona-runtime-agent-roster"
            onChange={(event) => {
              if (event.target.value) {
                onValueChange(event.target.value);
              }
            }}
            value={selectedRosterName}
          >
            <option value="">Choose an available agent</option>
            {state.agents.map((agent) => (
              <option key={agent.id ?? agent.name} value={agent.name}>
                {agent.description
                  ? `${agent.name} — ${agent.description}`
                  : agent.name}
              </option>
            ))}
          </select>
        </div>
      ) : null}

      <AgentNameInput
        disabled={disabled}
        onValueChange={onValueChange}
        placeholder={placeholder}
        value={value}
      />

      <RosterStatus state={state} onRetry={onRetry} disabled={disabled} />
    </div>
  );
}

function AgentNameInput({
  disabled,
  onValueChange,
  placeholder,
  value,
}: {
  disabled: boolean;
  onValueChange: (value: string) => void;
  placeholder: string;
  value: string;
}) {
  return (
    <div
      className={cn(
        "flex min-h-11 items-center px-3",
        PERSONA_FIELD_SHELL_CLASS,
      )}
    >
      <Input
        autoCorrect="off"
        className={cn("h-8 px-0 py-0 leading-6", PERSONA_FIELD_CONTROL_CLASS)}
        data-testid="persona-runtime-agent-name"
        disabled={disabled}
        id="persona-runtime-model-env"
        onChange={(event) => onValueChange(event.target.value)}
        placeholder={placeholder}
        value={value}
      />
    </div>
  );
}

function RosterStatus({
  disabled,
  onRetry,
  state,
}: {
  disabled: boolean;
  onRetry: () => void;
  state: IntelAgentRosterState;
}) {
  if (state.status === "populated") {
    return (
      <p className="text-xs text-muted-foreground">
        Choose an available agent or enter a name manually.
      </p>
    );
  }

  const retryable =
    state.status === "empty" ||
    state.status === "authError" ||
    state.status === "connectionError" ||
    state.status === "malformedError";
  const tone =
    state.status === "authError"
      ? "text-destructive"
      : state.status === "connectionError" || state.status === "malformedError"
        ? "text-warning"
        : "text-muted-foreground";
  const message = rosterStatusMessage(state);

  return (
    <div
      aria-live="polite"
      className={cn("flex min-h-6 items-center justify-between gap-2", tone)}
      data-testid={`persona-runtime-agent-roster-${state.status}`}
    >
      <p className="flex items-center gap-1.5 text-xs">
        {state.status === "loading" ? (
          <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
        ) : null}
        {message}
      </p>
      {retryable ? (
        <Button
          disabled={disabled}
          onClick={onRetry}
          size="xs"
          type="button"
          variant="ghost"
        >
          <RefreshCw />
          Retry
        </Button>
      ) : null}
    </div>
  );
}

function rosterStatusMessage(state: IntelAgentRosterState): string {
  switch (state.status) {
    case "idle":
      return "Enter the gateway URL and API key to load available agents.";
    case "loading":
      return "Loading agents from the gateway…";
    case "empty":
      return "The gateway returned no agents. Enter a name manually.";
    case "authError":
    case "connectionError":
    case "malformedError":
      return state.message;
    case "populated":
      return "";
  }
}

function useIntelAgentRoster({
  apiKey,
  enabled,
  gatewayUrl,
}: {
  apiKey: string;
  enabled: boolean;
  gatewayUrl: string;
}) {
  const [state, setState] = React.useState<IntelAgentRosterState>({
    status: "idle",
  });
  const requestIdRef = React.useRef(0);
  const trimmedApiKey = apiKey.trim();
  const trimmedGatewayUrl = gatewayUrl.trim();

  const load = React.useCallback(async () => {
    if (!enabled || !trimmedApiKey || !trimmedGatewayUrl) {
      requestIdRef.current += 1;
      setState({ status: "idle" });
      return;
    }

    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setState({ status: "loading" });
    try {
      const response = await listIntelAgents({
        apiKey: trimmedApiKey,
        gatewayUrl: trimmedGatewayUrl,
      });
      if (requestId !== requestIdRef.current) return;
      setState(
        response.agents.length > 0
          ? { status: "populated", agents: response.agents }
          : { status: "empty" },
      );
    } catch (error) {
      if (requestId !== requestIdRef.current) return;
      setState(intelAgentRosterErrorState(error));
    }
  }, [enabled, trimmedApiKey, trimmedGatewayUrl]);

  React.useEffect(() => {
    requestIdRef.current += 1;
    if (!enabled || !trimmedApiKey || !trimmedGatewayUrl) {
      setState({ status: "idle" });
      return;
    }

    setState({ status: "loading" });
    const timer = window.setTimeout(() => {
      void load();
    }, 400);
    return () => {
      window.clearTimeout(timer);
      requestIdRef.current += 1;
    };
  }, [enabled, load, trimmedApiKey, trimmedGatewayUrl]);

  return { retry: () => void load(), state };
}
