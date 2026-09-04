import { cn } from "@/shared/lib/cn";
import type { RuntimeState } from "../types";

const TONE: Record<RuntimeState, string> = {
  running:
    "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400 ring-emerald-500/30",
  awaiting_attestation:
    "bg-amber-500/10 text-amber-700 dark:text-amber-400 ring-amber-500/30",
  requested: "bg-sky-500/10 text-sky-700 dark:text-sky-400 ring-sky-500/30",
  attested: "bg-sky-500/10 text-sky-700 dark:text-sky-400 ring-sky-500/30",
  stopped:
    "bg-black/5 text-black/60 dark:bg-white/10 dark:text-white/60 ring-black/10 dark:ring-white/15",
  removed:
    "bg-black/5 text-black/60 dark:bg-white/10 dark:text-white/60 ring-black/10 dark:ring-white/15",
  expired:
    "bg-black/5 text-black/60 dark:bg-white/10 dark:text-white/60 ring-black/10 dark:ring-white/15",
  crashed: "bg-red-500/10 text-red-700 dark:text-red-400 ring-red-500/30",
  failed: "bg-red-500/10 text-red-700 dark:text-red-400 ring-red-500/30",
};

const LABEL: Record<RuntimeState, string> = {
  running: "Running",
  awaiting_attestation: "Awaiting authorization",
  requested: "Requested",
  attested: "Attested",
  stopped: "Stopped",
  removed: "Removed",
  expired: "Expired",
  crashed: "Crashed",
  failed: "Failed",
};

export function StatusPill({
  state,
  stale,
  className,
}: {
  state: RuntimeState | null;
  stale?: boolean;
  className?: string;
}) {
  if (state === null) {
    return (
      <span
        className={cn(
          "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset",
          "bg-black/5 text-black/50 ring-black/10 dark:bg-white/10 dark:text-white/50 dark:ring-white/15",
          className,
        )}
        title="No runner has published a status event for this agent."
      >
        No runner reporting
      </span>
    );
  }

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset",
        TONE[state],
        className,
      )}
    >
      {LABEL[state]}
      {stale && state === "running" ? (
        <span
          className="text-amber-700 dark:text-amber-400"
          title="Last heartbeat is older than 90s — the runner may be gone even though its last published state was 'running'."
        >
          · stale
        </span>
      ) : null}
    </span>
  );
}
