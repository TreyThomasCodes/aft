import {
  prepareCanonicalEditArguments,
  prepareCanonicalPathArguments,
} from "@cortexkit/aft-bridge";
import type { ToolDefinition } from "@opencode-ai/plugin";
import { tool } from "@opencode-ai/plugin";

type ToolArgSchema = ToolDefinition["args"][string];

type SchemaWithJsonSchemaOverride = ToolArgSchema & {
  _zod: ToolArgSchema["_zod"] & {
    toJSONSchema?: () => unknown;
  };
};

function stripRootJsonSchemaFields(jsonSchema: Record<string, unknown>): Record<string, unknown> {
  const { $schema: _schema, ...rest } = jsonSchema;
  return rest;
}

function attachJsonSchemaOverride(schema: SchemaWithJsonSchemaOverride): void {
  if (schema._zod.toJSONSchema) {
    return;
  }

  schema._zod.toJSONSchema = (): Record<string, unknown> => {
    const originalOverride = schema._zod.toJSONSchema;
    delete schema._zod.toJSONSchema;

    try {
      return stripRootJsonSchemaFields(tool.schema.toJSONSchema(schema));
    } finally {
      schema._zod.toJSONSchema = originalOverride;
    }
  };
}

/**
 * Patch tool arg schemas so that `.describe()` and `.meta()` survive
 * cross-Zod-instance JSON Schema serialization.
 *
 * OpenCode's host Zod can't see descriptions set by the plugin's Zod.
 * This patches `_zod.toJSONSchema` on each arg to use the plugin's own
 * `tool.schema.toJSONSchema`, which preserves all metadata.
 */
export function normalizeToolArgSchemas<T extends Pick<ToolDefinition, "args">>(
  toolDefinition: T,
): T {
  for (const schema of Object.values(toolDefinition.args)) {
    attachJsonSchemaOverride(schema);
  }
  return toolDefinition;
}

function bareToolName(toolName: string): string {
  return toolName.startsWith("aft_") ? toolName.slice(4) : toolName;
}

/** Prepare raw OpenCode arguments and retain legacy fields for old schemas. */
export function prepareOpenCodeArguments(
  toolName: string,
  rawArguments: unknown,
): Record<string, unknown> {
  const bare = bareToolName(toolName);
  if (bare === "edit") {
    return prepareCanonicalEditArguments(toolName, rawArguments);
  }

  // Keep the canonical object intact after compatibility normalization. The host
  // schema and execute handler must observe the same published vocabulary; the
  // bridge payload conversion belongs to each command adapter below that boundary.
  return prepareCanonicalPathArguments(toolName, rawArguments);
}

const DISPLAY_FILE_PATH_TOOLS = new Set(["read", "write", "edit"]);

/**
 * OpenCode's metadata callback closes over the host argument object, while the
 * plugin's compatibility normalizer intentionally returns a fresh object.
 * Mirror the canonical path onto that host object after normalization so a
 * later metadata update persists the alias that OpenCode's file UIs expect.
 */
function preserveDisplayFilePathAlias(
  toolName: string,
  rawArguments: unknown,
  prepared: Record<string, unknown>,
): void {
  if (!DISPLAY_FILE_PATH_TOOLS.has(toolName)) return;
  if (!rawArguments || typeof rawArguments !== "object" || Array.isArray(rawArguments)) return;

  const raw = rawArguments as Record<string, unknown>;
  if (typeof prepared.path === "string" && !Object.hasOwn(raw, "filePath")) {
    raw.filePath = prepared.path;
  }
}

function prepareToolMap(
  tools: Record<string, ToolDefinition>,
  passthroughToolNames: ReadonlySet<string> = new Set(),
): Record<string, ToolDefinition> {
  for (const [toolName, def] of Object.entries(tools)) {
    const execute = def.execute;
    def.execute = (async (args, context) => {
      const prepared = passthroughToolNames.has(toolName)
        ? (args as Record<string, unknown>)
        : prepareOpenCodeArguments(toolName, args);
      preserveDisplayFilePathAlias(toolName, args, prepared);
      return execute(prepared, context);
    }) as ToolDefinition["execute"];
  }
  return tools;
}

/** Normalize tool definitions and attach raw-argument preparation. */
export function normalizeToolMap(
  tools: Record<string, ToolDefinition>,
): Record<string, ToolDefinition> {
  for (const def of Object.values(tools)) normalizeToolArgSchemas(def);
  return prepareToolMap(tools);
}

/** Attach argument preparation without changing emitted schema metadata. */
export { prepareToolMap };
