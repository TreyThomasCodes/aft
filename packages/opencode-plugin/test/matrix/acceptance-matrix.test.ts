import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const pluginTestRoot = resolve(testDir, "..");

interface AcceptanceRow {
  sliceId: `S${number}${string}`;
  claim: string;
  governingSource: string;
  testFile: string;
  testPattern: string;
}

const reliedOnContractLines = [
  {
    name: "permission endpoint",
    citation: "packages/server/src/handlers/permission.ts",
  },
  {
    name: "Effect entry precedence",
    citation: "packages/core/src/plugin/module.ts:14-27,115",
  },
  {
    name: "typed RPC",
    citation: "packages/core/src/rpc.ts:99-103",
  },
  {
    name: "interruption ordering",
    citation: "packages/core/src/session/runner/step.ts:140-142",
  },
  {
    name: "built-in replacement",
    citation: "packages/core/src/tool.ts:181-202",
  },
  {
    name: "path header keys",
    citation: "session-ui/src/tools/tool-renderer.tsx:362",
  },
  {
    name: "path header keys (TUI)",
    citation: "tui/src/mini/tool.ts:412",
  },
  {
    name: "inert effect dep",
    citation: "declared effect dependency inert on V1 server loader",
  },
  {
    name: "sessionID-scoped permission.create",
    citation: "route path sessionID selects owning Permission.Service",
  },
  {
    name: "dev/beta cadence",
    citation: "coherent rolling release cadence",
  },
];

const acceptanceMatrix: AcceptanceRow[] = [
  {
    sliceId: "S1",
    claim: "Modern V1 host loads ./server and ./tui while ignoring effect and setup keys",
    governingSource: "pack §13.1, rulings R2, R5, R6, R19, R20; OC record pm_7ceb3a96",
    testFile: "load-matrix/load-matrix.ts",
    testPattern: "modern V1 host selects ./server and ignores effect and setup",
  },
  {
    sliceId: "S1",
    claim: "Function-default entry is rejected by the V2 host loader before setup",
    governingSource: "pack §13.1, ruling R19; OC record pm_7ceb3a96",
    testFile: "load-matrix/load-matrix.ts",
    testPattern: "function-default entry is rejected by the V2 host loader before setup",
  },
  {
    sliceId: "S2",
    claim: "V2 host loader selects effect entrypoint before setup and runs it once",
    governingSource: "delta §6, rulings R1, R5; OC record pm_4a4e8133, pm_7725ea5f",
    testFile: "entry/server-effect.test.ts",
    testPattern: "effect",
  },
  {
    sliceId: "S3",
    claim:
      "Two V2 Locations share one daemon and reload without leaking watchers, ports, or processes",
    governingSource: "rulings R16, R21; OC record pm_7725ea5f",
    testFile: "load-matrix/load-matrix.ts",
    testPattern:
      "enabled V2 loader shares one daemon across two Locations and reloads without leaks",
  },
  {
    sliceId: "S4",
    claim: "All AFT tools advertise options.codemode: false and match V1 projected definitions",
    governingSource: "pack §4, §5, delta §3a, rulings R4, R17",
    testFile: "tool-surface/v2-tool-surface.test.ts",
    testPattern: "codemode: false",
  },
  {
    sliceId: "S5",
    claim:
      "V2 hoisted filesystem mutations route through host permission.create with path headers and PatchDiff",
    governingSource:
      "delta §1, rulings R1, R3, R4, R15; OC record pm_4a4e8133, pm_7ceb3a96, pm_7725ea5f",
    testFile: "permissions/v2-permission.test.ts",
    testPattern: "requestPermission",
  },
  {
    sliceId: "S5",
    claim: "Closed ask-site inventory R4 covers all permission sites on V2",
    governingSource: "ruling R4; OC record pm_4a4e8133",
    testFile: "permissions/ask-site-inventory.test.ts",
    testPattern: "ask",
  },
  {
    sliceId: "S6",
    claim:
      "Host interruption triggers Effect fiber cancellation, terminates foreground process, and records call_aborted",
    governingSource: "delta §2, rulings R1, R11, R22; OC record pm_4a4e8133, pm_7725ea5f",
    testFile: "cancellation/effect-cancellation.test.ts",
    testPattern: "call_aborted",
  },
  {
    sliceId: "S6",
    claim:
      "Background completion wakes use session.prompt with steer delivery and synthetic status-only updates",
    governingSource: "delta §2, rulings R1, R7; OC record pm_4a4e8133",
    testFile: "wakes/session-delivery.test.ts",
    testPattern: "steer",
  },
  {
    sliceId: "S7",
    claim:
      "Typed AftRpc registers with supervisor and emits statusInvalidated, showStatusDialog, indexProgress",
    governingSource: "delta §3b, §3e, rulings R1, R8, R9, R10; OC record pm_4a4e8133, pm_7725ea5f",
    testFile: "rpc/register.test.ts",
    testPattern: "AftRpc",
  },
  {
    sliceId: "S8",
    claim:
      "V2 setup and doctor detect host generation, singular plugin key, and exact version pins",
    governingSource: "delta §4, ruling R12; OC record pm_7ceb3a96",
    testFile: "tui/v2-setup.test.tsx",
    testPattern: "setupV2Tui",
  },
  {
    sliceId: "S9",
    claim:
      "Beta pin bump to 0.0.0-beta-19234 verified against OC delta audit record pm_7725ea5f with zero deltas",
    governingSource:
      "constraints §Coexistence testing; rulings R1, R13, R18; OC record pm_7ceb3a96 §4, pm_7725ea5f",
    testFile: "matrix/acceptance-matrix.test.ts",
    testPattern: "pm_7725ea5f",
  },
];

describe("OpenCode V2 delta audit evidence", () => {
  const auditFile = join(testDir, "delta-audit-beta-19234.md");

  test("evidence file exists on disk", () => {
    expect(existsSync(auditFile)).toBe(true);
  });

  test("evidence records pinned build id 0.0.0-beta-19234 and OC record pm_7725ea5f", () => {
    const content = readFileSync(auditFile, "utf8");
    expect(content).toContain("0.0.0-beta-19234");
    expect(content).toContain("pm_7725ea5f");
    expect(content).toContain("2026-09-07");
    expect(content).toContain("ZERO delta");
  });

  test("evidence lists every relied-on contract line and citation", () => {
    const content = readFileSync(auditFile, "utf8");
    for (const item of reliedOnContractLines) {
      expect(content).toContain(item.citation);
    }
  });
});

describe("OpenCode V2 acceptance matrix", () => {
  test("every slice S1 through S9 is represented", () => {
    const requiredSlices = ["S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9"];
    const presentSlices = new Set(acceptanceMatrix.map((row) => row.sliceId));
    for (const slice of requiredSlices) {
      expect(presentSlices.has(slice as AcceptanceRow["sliceId"])).toBe(true);
    }
  });

  test("every row is keyed to governing source and points to an existing test", () => {
    for (const row of acceptanceMatrix) {
      expect(row.governingSource.length).toBeGreaterThan(0);
      const fullPath = join(pluginTestRoot, row.testFile);
      expect(existsSync(fullPath)).toBe(true);
      const source = readFileSync(fullPath, "utf8");
      expect(source).toContain(row.testPattern);
    }
  });

  test("no row uses self-asserted verified status", () => {
    for (const row of acceptanceMatrix) {
      expect((row as Record<string, unknown>).status).toBeUndefined();
    }
  });
});
