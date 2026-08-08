import { mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/**
 * Older transport fixtures use stable synthetic roots instead of creating a
 * project for every test. Keep those fixtures in the OS temp directory so the
 * tests work in read-only CI environments while still exercising root checks.
 */
const fixtureRoot = mkdtempSync(join(tmpdir(), "aft-subc-transport-"));
const rawRoots = {
  project: join(fixtureRoot, "project"),
  other: join(fixtureRoot, "other"),
  reaped: join(fixtureRoot, "reaped"),
  inflight: join(fixtureRoot, "inflight"),
  synthetic: join(fixtureRoot, "synthetic"),
};

for (const root of Object.values(rawRoots)) mkdirSync(root, { recursive: true });

export const TEST_PROJECT_ROOT = realpathSync(rawRoots.project);
export const TEST_OTHER_ROOT = realpathSync(rawRoots.other);
export const TEST_REAPED_ROOT = realpathSync(rawRoots.reaped);
export const TEST_INFLIGHT_ROOT = realpathSync(rawRoots.inflight);
export const TEST_SYNTHETIC_ROOT = realpathSync(rawRoots.synthetic);

process.once("exit", () => {
  rmSync(fixtureRoot, { recursive: true, force: true });
});
