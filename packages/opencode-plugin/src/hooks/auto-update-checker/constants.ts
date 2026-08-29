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
 */

/** Root directory OpenCode uses for cached npm plugin wrapper installs. */
export const CACHE_DIR = join(getOpenCodeCacheRoot(), "packages");

/** Primary OpenCode configuration file path (standard JSON). */
export const USER_OPENCODE_CONFIG = join(getOpenCodeConfigRoot(), "opencode.json");

/** Alternative OpenCode configuration file path (JSON with Comments). */
export const USER_OPENCODE_CONFIG_JSONC = join(getOpenCodeConfigRoot(), "opencode.jsonc");
