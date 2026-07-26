import { invokeTauri, TauriInvokeError } from "@/shared/api/tauri";

export type IntelAgentRosterEntry = {
  id: string | null;
  name: string;
  description: string | null;
};

export type IntelAgentRosterResponse = {
  agents: IntelAgentRosterEntry[];
};

export type IntelAgentRosterErrorCode =
  | "auth"
  | "connection"
  | "malformedOutput"
  | "unavailable";

export type IntelAgentRosterFailure = {
  code: IntelAgentRosterErrorCode;
  message: string;
};

export async function listIntelAgents(input: {
  gatewayUrl: string;
  apiKey: string;
}): Promise<IntelAgentRosterResponse> {
  return invokeTauri<IntelAgentRosterResponse>("list_intel_agents", { input });
}

export function intelAgentRosterFailure(
  error: unknown,
): IntelAgentRosterFailure {
  if (error instanceof TauriInvokeError) {
    const payload = error.payload;
    if (
      typeof payload === "object" &&
      payload !== null &&
      "code" in payload &&
      "message" in payload &&
      typeof payload.code === "string" &&
      typeof payload.message === "string" &&
      isRosterErrorCode(payload.code)
    ) {
      return { code: payload.code, message: payload.message };
    }
  }

  return {
    code: "connection",
    message:
      "Could not reach the Intelligence Platform gateway. Check the URL and connection.",
  };
}

function isRosterErrorCode(value: string): value is IntelAgentRosterErrorCode {
  return (
    value === "auth" ||
    value === "connection" ||
    value === "malformedOutput" ||
    value === "unavailable"
  );
}
