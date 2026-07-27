import { Bot, Eye, Users } from "lucide-react";

import {
  type IntelligenceEntry,
  useIntelligenceInventory,
} from "../use-intelligence-inventory";

function VisibilityNotice() {
  return (
    <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm leading-relaxed text-black/75 dark:text-white/75">
      This list may be incomplete: personas are visible only to their author
      unless they carry a shared tag.
    </p>
  );
}

function IntelligenceEntryCard({ entry }: { entry: IntelligenceEntry }) {
  return (
    <article className="rounded-lg border border-black/10 bg-white p-5 shadow-xs dark:border-white/10 dark:bg-white/5">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          {entry.name && (
            <h2 className="text-lg font-semibold text-black dark:text-white">
              {entry.name}
            </h2>
          )}
          {entry.slug && (
            <p className="mt-1 break-all font-mono text-sm text-black/60 dark:text-white/60">
              {entry.slug}
            </p>
          )}
        </div>
        <span className="shrink-0 rounded-full bg-black/5 px-2.5 py-1 text-xs font-medium text-black/70 dark:bg-white/10 dark:text-white/70">
          {entry.kind}
        </span>
      </div>
      {entry.description && (
        <p className="mt-4 text-sm leading-relaxed text-black/75 dark:text-white/75">
          {entry.description}
        </p>
      )}
      <p className="mt-4 text-sm text-black/60 dark:text-white/60">
        {entry.shared ? "Shared" : "Not shared"}
      </p>
    </article>
  );
}

export function IntelligencePage() {
  const { data: entries, error, isLoading } = useIntelligenceInventory();

  return (
    <div className="flex flex-1 justify-center bg-[#F3F3F3] px-4 py-8 dark:bg-[#171717]">
      <main className="w-full max-w-3xl" aria-labelledby="intelligence-heading">
        <div className="mb-6 flex items-start gap-3">
          <div className="mt-1 rounded-lg bg-black/5 p-2 dark:bg-white/10">
            <Bot className="h-5 w-5 text-black dark:text-white" />
          </div>
          <div>
            <h1
              id="intelligence-heading"
              className="text-2xl font-semibold tracking-tight text-black dark:text-white"
            >
              Intelligence inventory
            </h1>
            <p className="mt-1 text-sm text-black/60 dark:text-white/60">
              Agents and personas visible to you from this relay.
            </p>
          </div>
        </div>

        <VisibilityNotice />

        {isLoading && (
          <p className="py-10 text-sm text-black/60 dark:text-white/60">
            Reading visible intelligence…
          </p>
        )}

        {error && (
          <section
            className="mt-6 rounded-lg border border-red-500/30 bg-red-500/10 p-5"
            aria-labelledby="intelligence-error-heading"
          >
            <h2
              id="intelligence-error-heading"
              className="text-lg font-semibold text-black dark:text-white"
            >
              Could not read the intelligence inventory
            </h2>
            <p className="mt-1 text-sm text-black/75 dark:text-white/75">
              {error.message}
            </p>
          </section>
        )}

        {!isLoading && !error && entries?.length === 0 && (
          <section className="mt-6 flex flex-col items-center rounded-lg border border-black/10 bg-white px-6 py-12 text-center dark:border-white/10 dark:bg-white/5">
            <Eye className="h-7 w-7 text-black/50 dark:text-white/50" />
            <h2 className="mt-4 text-lg font-semibold text-black dark:text-white">
              Nothing visible to you
            </h2>
            <p className="mt-1 max-w-md text-sm leading-relaxed text-black/60 dark:text-white/60">
              There are no visible agent or persona events for your identity.
            </p>
          </section>
        )}

        {!isLoading && !error && entries && entries.length > 0 && (
          <section className="mt-6" aria-label="Visible intelligence">
            <div className="mb-3 flex items-center gap-2 text-sm font-medium text-black/70 dark:text-white/70">
              <Users className="h-4 w-4" />
              {entries.length} visible{" "}
              {entries.length === 1 ? "entry" : "entries"}
            </div>
            <div className="space-y-3">
              {entries.map((entry) => (
                <IntelligenceEntryCard entry={entry} key={entry.id} />
              ))}
            </div>
          </section>
        )}
      </main>
    </div>
  );
}
