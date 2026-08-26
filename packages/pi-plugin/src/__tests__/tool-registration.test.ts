/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import type { AftConfig } from "../config.js";
import { registerPiToolSurface, resolvePiToolSurface } from "../tool-registration.js";
import { executeTool, makeMockApi, makeMockBridge, makePluginContext } from "./tool-test-utils.js";

function register(config: AftConfig) {
  const { api, tools } = makeMockApi();
  const { bridge, calls } = makeMockBridge(() => ({ success: true, text: "ok" }));
  const ctx = makePluginContext(bridge, { config });
  registerPiToolSurface(api, ctx, resolvePiToolSurface(config));
  return { tools, calls };
}

function publicSurface(
  tools: Map<string, { name: string; label?: string; description?: string; parameters?: unknown }>,
) {
  return [...tools.values()]
    .map(({ name, label, description, parameters }) => ({ name, label, description, parameters }))
    .sort((left, right) => left.name.localeCompare(right.name));
}

describe("Pi dual-mode tool registration", () => {
  test("defaults and explicit hoisting preserve the existing registered surface", () => {
    const defaults = register({ tool_surface: "recommended", search_index: true, bash: true });
    const explicit = register({
      tool_surface: "recommended",
      search_index: true,
      bash: true,
      hoist_builtin_tools: true,
    });

    expect(publicSurface(defaults.tools)).toEqual(publicSurface(explicit.tools));
  });

  test("registers aft_ alternatives without replacing native tool names", async () => {
    const { tools, calls } = register({
      tool_surface: "recommended",
      search_index: true,
      bash: true,
      hoist_builtin_tools: false,
    });

    for (const name of ["aft_read", "aft_write", "aft_edit", "aft_grep", "aft_bash"]) {
      expect(tools.has(name)).toBe(true);
    }
    for (const name of ["read", "write", "edit", "grep", "bash"]) {
      expect(tools.has(name)).toBe(false);
    }
    // The AFT task family never collides with a host bash tool, so it remains
    // available under its established names in both registration modes.
    for (const name of ["bash_status", "bash_watch", "bash_write", "bash_kill"]) {
      expect(tools.has(name)).toBe(true);
    }

    await executeTool(tools.get("aft_edit")!, {
      path: "prefixed.ts",
      edits: [{ oldString: "before", newString: "after" }],
    });
    expect(calls.at(-1)?.params.name).toBe("edit");
  });

  test("uses the active registration spelling for disabled native replacements", () => {
    const prefixed = resolvePiToolSurface({
      tool_surface: "recommended",
      hoist_builtin_tools: false,
      disabled_tools: ["aft_edit", "aft_bash"],
    });
    expect(prefixed.hoistEdit).toBe(false);
    expect(prefixed.hoistBash).toBe(false);

    const hoisted = resolvePiToolSurface({
      tool_surface: "recommended",
      disabled_tools: ["edit", "bash"],
    });
    expect(hoisted.hoistEdit).toBe(false);
    expect(hoisted.hoistBash).toBe(false);
  });
});
