import { Link } from "@tanstack/react-router";
import { ArrowLeft, ShieldAlert } from "lucide-react";

import { ThemeToggle } from "@/shared/theme/ThemeToggle";
import { HEARTBEAT_STALE_S, isHeartbeatStale } from "../join";
import { useWorkspaceAgent } from "../use-intelligence";
import { StatusPill } from "./StatusPill";

function Field({
  label,
  value,
  mono,
  hint,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  hint?: string;
}) {
  return (
    <div className="border-t border-black/5 py-3 dark:border-white/5 sm:grid sm:grid-cols-3 sm:gap-4">
      <dt className="text-sm text-black/50 dark:text-white/50">
        {label}
        {hint ? (
          <span
            className="ml-1 cursor-help text-black/30 dark:text-white/30"
            title={hint}
          >
            ⓘ
          </span>
        ) : null}
      </dt>
      <dd
        className={`mt-1 text-sm text-black dark:text-white sm:col-span-2 sm:mt-0 ${
          mono ? "break-all font-mono text-xs" : ""
        }`}
      >
        {value}
      </dd>
    </div>
  );
}

export function AgentDetailPage({ agentPubkey }: { agentPubkey: string }) {
  const query = useWorkspaceAgent(agentPubkey);
  const agent = query.data;
  const now = Math.floor(Date.now() / 1000);

  return (
    <div className="mx-auto w-full max-w-3xl px-4 py-10 sm:px-6">
      <header className="flex items-start justify-between gap-4">
        <Link
          className="inline-flex items-center gap-1.5 text-sm text-black/60 transition-colors hover:text-black dark:text-white/60 dark:hover:text-white"
          to="/intelligence"
        >
          <ArrowLeft className="h-4 w-4" />
          Intelligence
        </Link>
        <ThemeToggle />
      </header>

      {query.isPending ? (
        <p className="mt-8 text-sm text-black/50 dark:text-white/50">
          Loading from the relay…
        </p>
      ) : !agent ? (
        <div className="mt-8" data-testid="agent-not-found">
          <h1 className="text-xl font-semibold text-black dark:text-white">
            Agent not found
          </h1>
          <p className="mt-2 text-sm text-black/60 dark:text-white/60">
            No managed-agent record from a pinned owner matches{" "}
            <span className="break-all font-mono text-xs">{agentPubkey}</span>.
          </p>
        </div>
      ) : (
        <>
          <div className="mt-6 flex items-center gap-3">
            <h1 className="text-2xl font-semibold tracking-tight text-black dark:text-white">
              {agent.displayName}
            </h1>
            <StatusPill
              stale={isHeartbeatStale(agent.status, now)}
              state={agent.status?.state ?? null}
            />
          </div>

          {agent.danglingPersona ? (
            <div className="mt-4 rounded-lg border border-red-500/30 bg-red-500/5 p-4">
              <div className="flex items-center gap-2 text-red-700 dark:text-red-400">
                <ShieldAlert className="h-4 w-4" />
                <h2 className="text-sm font-semibold">Persona missing</h2>
              </div>
              <p className="mt-2 text-sm text-black/70 dark:text-white/70">
                This record links persona{" "}
                <code className="font-mono text-xs">{agent.personaId}</code>,
                which does not resolve. Because the projection is slimmed when a
                persona is linked, there is no model to spawn from — a runner
                must refuse to start this agent rather than guess.
              </p>
            </div>
          ) : null}

          <dl className="mt-6">
            <Field label="Agent pubkey" mono value={agent.pubkey} />
            <Field label="Owner" mono value={agent.owner} />
            <Field
              hint="Resolved as (record author, persona_id) → persona coordinate, scoped by author because slugs are not unique across owners."
              label="Persona"
              value={
                agent.personaId ? (
                  <span className="font-mono text-xs">{agent.personaId}</span>
                ) : (
                  <span className="text-black/40 dark:text-white/40">
                    none — configuration is inline on the record
                  </span>
                )
              }
            />
            <Field label="Runtime" value={agent.runtime ?? <Absent />} />
            <Field
              hint="For the intel runtime this is the deployed gateway agent name (INTEL_AGENT)."
              label="Gateway agent"
              value={agent.gatewayAgent ?? <Absent />}
            />
            <Field
              hint="A curated gateway id. The gateway URL and credential are runner-side operator config and never appear in an event."
              label="Gateway"
              value={agent.gatewayId ?? <Absent />}
            />
            <Field
              label="Inbound author gate"
              value={agent.respondTo ?? <Absent />}
            />
            {agent.personaShared !== null ? (
              <Field
                hint={
                  "The relay gates kind 30175 author-only unless the event carries a " +
                  "shared=true tag. A runner that is not the persona's author can only " +
                  "read it when shared."
                }
                label="Persona visibility"
                value={
                  agent.personaShared ? (
                    <span className="text-amber-700 dark:text-amber-400">
                      shared — readable by every member of this community
                    </span>
                  ) : (
                    <span>author-only — not readable by other members</span>
                  )
                }
              />
            ) : null}
            <Field
              label="System prompt"
              value={
                agent.systemPrompt ? (
                  <>
                    <pre className="whitespace-pre-wrap rounded bg-black/[0.03] p-3 text-xs dark:bg-white/[0.04]">
                      {agent.systemPrompt}
                    </pre>
                    {agent.personaShared ? (
                      <p className="mt-2 text-xs text-amber-700 dark:text-amber-400">
                        This prompt is stored in a shared persona event, so it
                        is plaintext and world-readable within this community.
                        Do not put anything sensitive in it.
                      </p>
                    ) : null}
                  </>
                ) : (
                  <Absent />
                )
              }
            />
          </dl>

          <h2 className="mt-8 text-sm font-semibold text-black dark:text-white">
            Runtime
          </h2>
          {agent.status ? (
            <dl className="mt-2">
              <Field
                hint={`The runner publishes every 30s; older than ${HEARTBEAT_STALE_S}s is stale. Measured against your clock, because created_at is author-supplied.`}
                label="Last heartbeat"
                value={
                  agent.status.heartbeat_at
                    ? `${now - agent.status.heartbeat_at}s ago`
                    : "never"
                }
              />
              <Field
                label="Restarts"
                value={agent.status.restarts ?? <Absent />}
              />
              <Field
                label="Last error class"
                value={agent.status.last_error_class ?? <Absent />}
              />
              <Field
                label="Reported by"
                mono
                value={agent.status.runner ?? "pinned runner"}
              />
            </dl>
          ) : (
            <p className="mt-2 text-sm text-black/50 dark:text-white/50">
              No pinned runner has published a status event for this agent.
            </p>
          )}

          <h2 className="mt-8 text-sm font-semibold text-black dark:text-white">
            Authorization
          </h2>
          <p className="mt-2 max-w-2xl text-sm text-black/60 dark:text-white/60">
            NIP-OA conditions are{" "}
            <strong>recorded but not enforced at relay login</strong> on this
            deployment, so an attestation is effectively unbounded in time.
            Revoking an agent's access means revoking the owner's community
            membership or banning the agent pubkey — removing the agent from
            this workspace does not do it.
          </p>

          <footer className="mt-10 border-t border-black/5 pt-4 text-xs text-black/40 dark:border-white/5 dark:text-white/40">
            Per-turn token metrics are NIP-44 encrypted to this agent's owner
            and p-gated at the relay, so they are not readable here unless you
            are signed in as that owner.
          </footer>
        </>
      )}
    </div>
  );
}

function Absent() {
  return <span className="text-black/40 dark:text-white/40">—</span>;
}
