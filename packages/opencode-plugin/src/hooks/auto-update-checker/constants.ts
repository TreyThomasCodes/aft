import { join } from "node:path";
import { getOpenCodeCacheRoot, getOpenCodeConfigRoot } from "@cortexkit/aft-bridge";

export const PACKAGE_NAME = "@cortexkit/aft-opencode";
export const NPM_REGISTRY_URL = "https://registry.npmjs.org";
export const NPM_FETCH_TIMEOUT = 10_000;

export { getOpenCodeCacheRoot, getOpenCodeConfigRoot };

/**
 * OpenCode creates this directory when it installs an npm plugin. If it is
 * absent, the checker cannot find a cached version or a fallback install root
 * when the runtime package path is unavailable, so it skips the update.
 *
 * These are functions, not module-level constants: they read XDG_* environment
 * variables, and several test files mutate those process-wide. A path captured
 * at import time would depend on module-import order relative to those
 * mutations, so every caller resolves at call time instead.
 */

/** Root directory OpenCode uses for cached npm plugin wrapper installs. */
export function cacheDir(): string {
  return join(getOpenCodeCacheRoot(), "packages");
}

/** Primary OpenCode configuration file path (standard JSON). */
export function userOpenCodeConfig(): string {
  return join(getOpenCodeConfigRoot(), "opencode.json");
}

/** Alternative OpenCode configuration file path (JSON with Comments). */
export function userOpenCodeConfigJsonc(): string {
  return join(getOpenCodeConfigRoot(), "opencode.jsonc");
}
