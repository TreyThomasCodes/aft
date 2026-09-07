/**
 * Harness detection for Pi and Pi-compatible hosts (such as OMP / oh-my-pi).
 */

export type PiHarness = "pi" | "omp" | "unknown";

let recordedApi: unknown = null;
let testHarnessOverride: PiHarness | null = null;

/**
 * Set or clear an explicit harness override for tests.
 */
export function setHarnessOverrideForTesting(override: PiHarness | null): void {
  testHarnessOverride = override;
}

/**
 * Record the active host ExtensionAPI instance passed to the extension factory.
 */
export function recordActiveExtensionApi(api: unknown): void {
  recordedApi = api;
}

/**
 * Clear any recorded ExtensionAPI reference.
 */
export function resetRecordedExtensionApi(): void {
  recordedApi = null;
}

/**
 * Determine the active Pi harness ("pi" | "omp" | "unknown").
 *
 * Runtime signal selection:
 * - Candidate 1: Host package resolution (`@oh-my-pi/pi-coding-agent` vs `@earendil-works/pi-coding-agent`).
 *   Rejected because resolving package names depends on node_modules layout. If `@oh-my-pi/pi-coding-agent`
 *   is present in node_modules (e.g. in a monorepo or test environment), resolving it succeeds even when the
 *   active host process is upstream Pi, producing a false positive on Pi.
 * - Candidate 2: `process.argv[1]`.
 *   Rejected because it is fragile and unreliable. Under test runners (bun test, vitest), CLI wrappers,
 *   symlinks, or programmatic embeds, `argv[1]` reflects the test runner or wrapper rather than the host.
 *   Furthermore, directory paths containing "omp" would produce false positives.
 * - Candidate 3: Fields on the `ExtensionAPI` object passed by the host.
 *   SELECTED. When an extension is loaded, the host creates and injects an `ExtensionAPI` instance into
 *   the extension factory (`ConcreteExtensionAPI` in OMP vs `createExtensionAPI` in upstream Pi).
 *   OMP's `ExtensionAPI` implementation (`extensibility/extensions/loader.ts:154-172` and `types.ts:1218-1231`)
 *   injects OMP-specific members that upstream Pi (`pi-mono/packages/coding-agent/src/core/extensions/loader.ts:280-360`
 *   and `types.ts:1252-1360`) has never exposed:
 *     - `api.arktype`: injected `@oh-my-pi/omptype` schema builder for extension tools (upstream Pi uses `@sinclair/typebox` and has no ArkType dependency).
 *     - `api.registerFileWriteFallback` / `api.registerFileDeleteFallback`: OMP's sandbox write/delete fallback handlers.
 *     - `api.events`: OMP's internal `EventBus` instance.
 *   Upstream Pi's `createExtensionAPI` returns a plain object with only standard Pi extension methods (`registerTool`,
 *   `registerCommand`, `registerShortcut`, `registerFlag`, `registerMessageRenderer`, `registerMarkdownTransformer`,
 *   `registerEntryRenderer`, `on`, `getFlag`). None of `arktype`, `events`, or `registerFileWriteFallback` exist on Pi's `ExtensionAPI`.
 *   Inspecting these fields cannot false-positive on upstream Pi.
 *
 * @param api Optional ExtensionAPI instance. If omitted, the recorded API from extension initialization is checked,
 *            followed by global OMP environment indicators.
 */
export function detectPiHarness(api?: unknown): PiHarness {
  if (testHarnessOverride !== null) {
    return testHarnessOverride;
  }

  const target = api ?? recordedApi;
  if (target && typeof target === "object") {
    const candidate = target as Record<string, unknown>;
    if (
      "arktype" in candidate ||
      "registerFileWriteFallback" in candidate ||
      "events" in candidate
    ) {
      return "omp";
    }
    if (typeof candidate.registerTool === "function") {
      return "pi";
    }
  }

  // Secondary fallback if no ExtensionAPI is available (e.g. called out-of-band)
  const g = globalThis as Record<string, unknown>;
  if (g.__ompLegacyPiBundledModules !== undefined) {
    return "omp";
  }

  return "unknown";
}
