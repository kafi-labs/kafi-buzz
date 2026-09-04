/**
 * Node ESM resolve hooks for `node --test` on the web sources.
 *
 * Two gaps between Vite's resolver and node's: the `@/` alias, and
 * extensionless relative imports between `.ts` files. Node's type stripping
 * handles the TypeScript itself, so no transpiler is needed here.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const srcRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "src",
);

const EXTENSIONS = [".ts", ".tsx", ".mjs", ".js"];

function resolveSourcePath(basePath) {
  // Existence decides, not path.extname — a dotted basename can look like it
  // already carries an extension while still needing one resolved.
  if (fs.existsSync(basePath) && fs.statSync(basePath).isFile()) {
    return basePath;
  }
  for (const extension of EXTENSIONS) {
    const candidate = `${basePath}${extension}`;
    if (fs.existsSync(candidate)) return candidate;
  }
  for (const extension of EXTENSIONS) {
    const candidate = path.join(basePath, `index${extension}`);
    if (fs.existsSync(candidate)) return candidate;
  }
  return null;
}

export function resolve(specifier, context, nextResolve) {
  if (specifier.startsWith("@/")) {
    const resolved = resolveSourcePath(path.join(srcRoot, specifier.slice(2)));
    return nextResolve(
      resolved ?? path.join(srcRoot, specifier.slice(2)),
      context,
    );
  }

  if (
    (specifier.startsWith("./") || specifier.startsWith("../")) &&
    context.parentURL?.startsWith("file:")
  ) {
    const parentPath = fileURLToPath(context.parentURL);
    const resolved = resolveSourcePath(
      path.resolve(path.dirname(parentPath), specifier),
    );
    if (resolved) return nextResolve(resolved, context);
  }

  return nextResolve(specifier, context);
}
