import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import type { AftConfig } from "./config.js";
import { resolveBashConfig } from "./config.js";
import { registerAstTools } from "./tools/ast.js";
import { registerBashCompanionTools, registerBashTool } from "./tools/bash.js";
import { registerConflictsTool } from "./tools/conflicts.js";
import { registerFsTools } from "./tools/fs.js";
import { registerHoistedTools } from "./tools/hoisted.js";
import { registerImportTools } from "./tools/imports.js";
import { registerInspectTool } from "./tools/inspect.js";
import { registerNavigateTool } from "./tools/navigate.js";
import { registerReadingTools } from "./tools/reading.js";
import { registerRefactorTool } from "./tools/refactor.js";
import { registerSafetyTool } from "./tools/safety.js";
import { registerSemanticTool } from "./tools/semantic.js";
import type { PluginContext } from "./types.js";

export interface PiToolSurface {
  /** Whether AFT replaces host-native tool names instead of registering aft_ alternatives. */
  hoistBuiltinTools: boolean;
  hoistBash: boolean;
  hoistRead: boolean;
  hoistWrite: boolean;
  hoistEdit: boolean;
  hoistGrep: boolean;
  restrictToProjectRoot: boolean;
  outline: boolean;
  zoom: boolean;
  semantic: boolean;
  inspect: boolean;
  navigate: boolean;
  conflicts: boolean;
  importTool: boolean;
  safety: boolean;
  delete: boolean;
  move: boolean;
  astSearch: boolean;
  astReplace: boolean;
  refactor: boolean;
}

const ALL_ONLY_TOOLS = new Set(["aft_callgraph", "aft_delete", "aft_move", "aft_refactor"]);

/** Resolve the feature predicates used by Pi's production registration path. */
export function resolvePiToolSurface(config: AftConfig): PiToolSurface {
  const surface = config.tool_surface ?? "recommended";
  const disabled = new Set(config.disabled_tools ?? []);
  const hoistBuiltinTools = config.hoist_builtin_tools !== false;
  const ok = (name: string): boolean => !disabled.has(name);
  const builtinToolEnabled = (bareName: string): boolean =>
    ok(hoistBuiltinTools ? bareName : `aft_${bareName}`);
  const allOnly = (name: string): boolean => ALL_ONLY_TOOLS.has(name) && ok(name);
  const restrictToProjectRoot = config.restrict_to_project_root ?? false;

  if (surface === "minimal") {
    return {
      hoistBuiltinTools,
      hoistBash: builtinToolEnabled("bash"),
      hoistRead: false,
      hoistWrite: false,
      hoistEdit: false,
      hoistGrep: false,
      restrictToProjectRoot,
      outline: ok("aft_outline"),
      zoom: ok("aft_zoom"),
      semantic: false,
      inspect: false,
      navigate: false,
      conflicts: false,
      importTool: false,
      safety: ok("aft_safety"),
      delete: false,
      move: false,
      astSearch: false,
      astReplace: false,
      refactor: false,
    };
  }

  const base: PiToolSurface = {
    hoistBuiltinTools,
    hoistBash: builtinToolEnabled("bash"),
    hoistRead: builtinToolEnabled("read"),
    hoistWrite: builtinToolEnabled("write"),
    hoistEdit: builtinToolEnabled("edit"),
    hoistGrep: builtinToolEnabled("grep") && config.search_index === true,
    restrictToProjectRoot,
    outline: ok("aft_outline"),
    zoom: ok("aft_zoom"),
    semantic: ok("aft_search") && config.semantic_search === true,
    inspect: ok("aft_inspect") && config.inspect?.enabled !== false,
    navigate: false,
    conflicts: ok("aft_conflicts"),
    importTool: ok("aft_import"),
    safety: ok("aft_safety"),
    delete: false,
    move: false,
    astSearch: ok("ast_grep_search"),
    astReplace: ok("ast_grep_replace"),
    refactor: false,
  };

  if (surface === "all") {
    return {
      ...base,
      navigate: allOnly("aft_callgraph"),
      delete: allOnly("aft_delete"),
      move: allOnly("aft_move"),
      refactor: allOnly("aft_refactor"),
    };
  }

  return base;
}

/**
 * Invoke every Pi tool registration branch for the resolved production surface.
 * Commands, prompt hints, and lifecycle hooks intentionally remain outside this
 * function because they are not entries in the agent-facing tool registry.
 */
export function registerPiToolSurface(
  pi: ExtensionAPI,
  ctx: PluginContext,
  surface: PiToolSurface,
): void {
  if (surface.hoistBash && resolveBashConfig(ctx.config).enabled) {
    registerBashTool(
      pi,
      ctx,
      surface.semantic,
      surface.hoistBuiltinTools ? "bash" : "aft_bash",
      false,
    );
    // Companion tools always address AFT task IDs, so their names remain free
    // of the primary bash tool's prefix and never collide with Pi's native bash.
    registerBashCompanionTools(pi, ctx);
  }
  registerHoistedTools(pi, ctx, surface);

  if (surface.outline || surface.zoom) {
    registerReadingTools(pi, ctx, surface);
  }
  if (surface.semantic) registerSemanticTool(pi, ctx);
  if (surface.inspect) registerInspectTool(pi, ctx);
  if (surface.navigate) registerNavigateTool(pi, ctx);
  if (surface.conflicts) registerConflictsTool(pi, ctx);
  if (surface.importTool) registerImportTools(pi, ctx);
  if (surface.safety && ctx.config.backup?.enabled !== false) registerSafetyTool(pi, ctx);
  if (surface.astSearch || surface.astReplace) registerAstTools(pi, ctx, surface);
  if (surface.delete || surface.move) registerFsTools(pi, ctx, surface);
  if (surface.refactor) registerRefactorTool(pi, ctx);
}
