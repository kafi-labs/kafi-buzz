/**
 * Guard: console surfaces must read the relay only through the author-pinned
 * helper.
 *
 * The relay accepts persona (30175), managed-agent (30177), and runner-status
 * (30900) events from any community member, and `created_at` is author-supplied.
 * So an unpinned read can be fed a forged row — which matters most on the
 * enrollment screen, where the owner signs a permanent, unbounded attestation
 * over a pubkey taken from an event.
 *
 * `queryEvents` is the raw, unpinned primitive. Anything under
 * `src/features/intelligence/` that imports it directly has bypassed the pin,
 * so this fails the build. Legacy surfaces (`features/repos`) are unaffected.
 */

import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

/** Directories whose reads must be author-pinned. */
const GUARDED_ROOTS = ["src/features/intelligence"];

/** The raw read primitives that bypass the pin. */
const FORBIDDEN = ["queryEvents"];

/** Modules allowed to reference the raw primitive (the pin itself wraps it). */
const ALLOWLIST = new Set(["src/shared/lib/author-pinned-query.ts"]);

async function walk(dir) {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  const files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await walk(full)));
    } else if (/\.(ts|tsx)$/.test(entry.name)) {
      files.push(full);
    }
  }
  return files;
}

const violations = [];

for (const root of GUARDED_ROOTS) {
  const files = await walk(path.resolve(projectRoot, root));
  for (const file of files) {
    const rel = path.relative(projectRoot, file);
    if (ALLOWLIST.has(rel)) continue;
    const source = await readFile(file, "utf8");
    const lines = source.split("\n");
    lines.forEach((line, index) => {
      for (const needle of FORBIDDEN) {
        if (line.includes(needle)) {
          violations.push(`${rel}:${index + 1}: uses \`${needle}\``);
        }
      }
    });
  }
}

if (violations.length > 0) {
  console.error(
    "Unpinned relay read in a console surface:\n" +
      violations.map((v) => `  ${v}`).join("\n") +
      "\n\nUse `queryPinned(url, filter, pin)` from " +
      "@/shared/lib/author-pinned-query instead. Any member can publish these " +
      "event kinds, so an unpinned read is forgeable.",
  );
  process.exit(1);
}

console.log(
  `Author-pinned read guard: OK (${GUARDED_ROOTS.join(", ")} clean of ${FORBIDDEN.join(", ")})`,
);
