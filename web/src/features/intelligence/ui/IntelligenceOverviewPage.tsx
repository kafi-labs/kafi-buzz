import { Link } from "@tanstack/react-router";
import { Bot, Cpu, ServerCog, ShieldAlert } from "lucide-react";

import { truncatePubkey } from "@/shared/lib/pubkey";
import { ThemeToggle } from "@/shared/theme/ThemeToggle";
import { OWNER_PUBKEYS, RUNNER_PUBKEYS } from "../console-config";
import { isHeartbeatStale } from "../join";
import { useGatewayCatalogs, useWorkspaceAgents } from "../use-intelligence";
import type { WorkspaceAgent } from "../types";
import { StatusPill } from "./StatusPill";

function Card({
  title,
  icon,
  children,
}: {
  title: string;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-xl border border-black/10 bg-white p-5 dark:border-white/10 dark:bg-white/[0.03]">
      <header className="flex items-center gap-2 text-black/60 dark:text-white/60">
        {icon}
        <h2 className="text-xs font-semibold uppercase tracking-wide">
          {title}
        </h2>
      </header>
      <div className="mt-3">{children}</div>
    </section>
  );
}

function NotConfigured() {
  return (
    <div className="rounded-xl border border-amber-500/30 bg-amber-500/5 p-5">
      <div className="flex items-center gap-2 text-amber-700 dark:text-amber-400">
        <ShieldAlert className="h-4 w-4" />
        <h2 className="text-sm font-semibold">Console not configured</h2>
      </div>
      <p className="mt-2 max-w-2xl text-sm text-black/70 dark:text-white/70">
        No workspace owner pubkeys are pinned, so there is nothing this console
        is willing to read. Owner and runner pubkeys are deploy configuration (
        <code className="font-mono text-xs">VITE_CONSOLE_OWNER_PUBKEYS</code>,{" "}
        <code className="font-mono text-xs">VITE_CONSOLE_RUNNER_PUBKEYS</code>)
        and are deliberately not discovered from event authors — any community
        member can publish these event kinds, so discovering the trust root from
        the data would make it forgeable.
      </p>
    </div>
  );
}

function AgentRow({ agent, now }: { agent: WorkspaceAgent; now: number }) {
  const stale = isHeartbeatStale(agent.status, now);
  return (
    <Link
      className="flex items-center justify-between gap-4 rounded-lg px-3 py-3 transition-colors hover:bg-black/[0.03] dark:hover:bg-white/[0.04]"
      params={{ agentPubkey: agent.pubkey }}
      to="/intelligence/agents/$agentPubkey"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium text-black dark:text-white">
            {agent.displayName}
          </span>
          {agent.danglingPersona ? (
            <span
              className="rounded bg-red-500/10 px-1.5 py-0.5 text-xs font-medium text-red-700 ring-1 ring-inset ring-red-500/30 dark:text-red-400"
              title="This record's persona_id resolves to no persona. The projection is slimmed when a persona is linked, so there is no model to spawn from."
            >
              persona missing
            </span>
          ) : null}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-black/50 dark:text-white/50">
          <span className="font-mono">{truncatePubkey(agent.pubkey)}</span>
          {agent.gatewayAgent ? <span>agent: {agent.gatewayAgent}</span> : null}
          {agent.gatewayId ? <span>gateway: {agent.gatewayId}</span> : null}
          {agent.runtime ? <span>runtime: {agent.runtime}</span> : null}
        </div>
      </div>
      <StatusPill stale={stale} state={agent.status?.state ?? null} />
    </Link>
  );
}

export function IntelligenceOverviewPage() {
  const agentsQuery = useWorkspaceAgents();
  const catalogsQuery = useGatewayCatalogs();
  const now = Math.floor(Date.now() / 1000);

  const agents = agentsQuery.data ?? [];
  const running = agents.filter((a) => a.status?.state === "running").length;
  const awaiting = agents.filter(
    (a) => a.status?.state === "awaiting_attestation",
  ).length;
  const unhealthy = agents.filter(
    (a) => a.status?.state === "crashed" || a.status?.state === "failed",
  ).length;

  return (
    <div className="mx-auto w-full max-w-5xl px-4 py-10 sm:px-6">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-black dark:text-white">
            Intelligence
          </h1>
          <p className="mt-1 text-sm text-black/60 dark:text-white/60">
            Agents backed by the Intelligence Platform in this workspace.
          </p>
        </div>
        <ThemeToggle />
      </header>

      {OWNER_PUBKEYS.length === 0 ? (
        <div className="mt-8">
          <NotConfigured />
        </div>
      ) : (
        <>
          <div className="mt-8 grid gap-4 sm:grid-cols-3">
            <Card icon={<Bot className="h-4 w-4" />} title="Agents">
              <p className="text-2xl font-semibold text-black dark:text-white">
                {agentsQuery.isPending ? "—" : agents.length}
              </p>
              <p className="mt-1 text-xs text-black/50 dark:text-white/50">
                {running} running · {awaiting} awaiting authorization
                {unhealthy > 0 ? ` · ${unhealthy} unhealthy` : ""}
              </p>
            </Card>

            <Card icon={<Cpu className="h-4 w-4" />} title="Platform">
              {catalogsQuery.data && catalogsQuery.data.length > 0 ? (
                catalogsQuery.data.map((catalog) => (
                  <div key={catalog.gateway_id}>
                    <p className="font-medium text-black dark:text-white">
                      {catalog.gateway_id}
                    </p>
                    <p className="mt-1 text-xs text-black/50 dark:text-white/50">
                      {catalog.probe?.ok
                        ? `authenticated${catalog.probe.scope ? ` · ${catalog.probe.scope}` : ""}`
                        : "not authenticated"}{" "}
                      · {catalog.agents.length} agents available
                    </p>
                  </div>
                ))
              ) : (
                <p className="text-sm text-black/50 dark:text-white/50">
                  {RUNNER_PUBKEYS.length === 0
                    ? "No runner pinned."
                    : "No runner attached."}
                </p>
              )}
            </Card>

            <Card icon={<ServerCog className="h-4 w-4" />} title="Runners">
              {RUNNER_PUBKEYS.length === 0 ? (
                <p className="text-sm text-black/50 dark:text-white/50">
                  None pinned.
                </p>
              ) : (
                <ul className="space-y-1">
                  {RUNNER_PUBKEYS.map((pubkey) => (
                    <li
                      className="font-mono text-xs text-black/60 dark:text-white/60"
                      key={pubkey}
                    >
                      {truncatePubkey(pubkey)}
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          </div>

          <section className="mt-8">
            <h2 className="text-sm font-semibold text-black dark:text-white">
              In this workspace
            </h2>

            {agentsQuery.isPending ? (
              <p className="mt-3 text-sm text-black/50 dark:text-white/50">
                Loading from the relay…
              </p>
            ) : agentsQuery.isError ? (
              <div
                className="mt-3 rounded-lg border border-red-500/30 bg-red-500/5 p-4 text-sm text-red-700 dark:text-red-400"
                data-testid="agents-error"
              >
                Could not read from the relay:{" "}
                {agentsQuery.error instanceof Error
                  ? agentsQuery.error.message
                  : "unknown error"}
              </div>
            ) : agents.length === 0 ? (
              <p
                className="mt-3 text-sm text-black/50 dark:text-white/50"
                data-testid="agents-empty"
              >
                No intelligence agents yet.
              </p>
            ) : (
              <div
                className="mt-2 divide-y divide-black/5 dark:divide-white/5"
                data-testid="agent-list"
              >
                {agents.map((agent) => (
                  <AgentRow agent={agent} key={agent.pubkey} now={now} />
                ))}
              </div>
            )}
          </section>

          <footer className="mt-10 border-t border-black/5 pt-4 text-xs text-black/40 dark:border-white/5 dark:text-white/40">
            Reads are pinned to {OWNER_PUBKEYS.length} owner
            {OWNER_PUBKEYS.length === 1 ? "" : "s"} and {RUNNER_PUBKEYS.length}{" "}
            runner{RUNNER_PUBKEYS.length === 1 ? "" : "s"} from deploy config.
            Turn-level detail is encrypted to the agent's owner and is not
            readable here.
          </footer>
        </>
      )}
    </div>
  );
}
