/**
 * Canonical path aliases accepted at host and subc preparation boundaries.
 *
 * Compatibility is intentionally about decoded string values only. The
 * preparation layer must not trim, normalize, fold case, rewrite separators,
 * or resolve paths before comparing the two spellings.
 */

import { AftToolError } from "./error-contract.js";

export type CanonicalPathTool =
  | "read"
  | "write"
  | "edit"
  | "zoom"
  | "callgraph"
  | "safety"
  | "move"
  | "import"
  | "refactor"
  | "grep"
  | "search"
  | "conflicts";

export class InvalidRequestError extends AftToolError {
  constructor(message: string) {
    super(message, "invalid_request", {
      success: false,
      code: "invalid_request",
      message,
    });
    this.name = "InvalidRequestError";
  }
}

/** Return false for lone UTF-16 surrogate code units. */
export function isWellFormedUnicodeString(value: string): boolean {
  for (let index = 0; index < value.length; index++) {
    const codeUnit = value.charCodeAt(index);
    if (codeUnit >= 0xd800 && codeUnit <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff || Number.isNaN(next)) return false;
      index++;
    } else if (codeUnit >= 0xdc00 && codeUnit <= 0xdfff) {
      return false;
    }
  }
  return true;
}

function hasOwn(record: Record<string, unknown>, key: string): boolean {
  return Object.hasOwn(record, key);
}

function invalidPathValue(property: string): never {
  throw new InvalidRequestError(`'${property}' must be a non-empty well-formed Unicode string`);
}

function pathValue(record: Record<string, unknown>, property: string): string {
  const value = record[property];
  if (typeof value !== "string" || value.length === 0 || !isWellFormedUnicodeString(value)) {
    invalidPathValue(property);
  }
  return value;
}

function normalizeAliasPair(
  record: Record<string, unknown>,
  canonical: string,
  legacy: string,
  required: boolean,
): void {
  const hasCanonical = hasOwn(record, canonical);
  const hasLegacy = hasOwn(record, legacy);

  if (!hasCanonical && !hasLegacy) {
    if (required) {
      throw new InvalidRequestError(`'${canonical}' is required`);
    }
    return;
  }

  if (hasCanonical && hasLegacy) {
    let canonicalValue: string;
    let legacyValue: string;
    try {
      canonicalValue = pathValue(record, canonical);
      legacyValue = pathValue(record, legacy);
    } catch {
      throw new InvalidRequestError(
        `Invalid request: '${canonical}' and '${legacy}' must both be non-empty well-formed Unicode strings`,
      );
    }
    if (canonicalValue !== legacyValue) {
      throw new InvalidRequestError(
        `Invalid request: '${canonical}' and '${legacy}' must contain equal decoded strings`,
      );
    }
    delete record[legacy];
    return;
  }

  if (hasCanonical) {
    pathValue(record, canonical);
    return;
  }

  record[canonical] = pathValue(record, legacy);
  delete record[legacy];
}

function validateOptionalCanonicalPath(record: Record<string, unknown>, property: string): void {
  if (hasOwn(record, property)) pathValue(record, property);
}

function normalizeZoomTargets(record: Record<string, unknown>): void {
  if (!hasOwn(record, "targets")) return;
  const targets = record.targets;
  const normalizeTarget = (target: unknown, index: number): Record<string, unknown> => {
    if (!target || typeof target !== "object" || Array.isArray(target)) {
      throw new InvalidRequestError(`'targets[${index}].path' must be a non-empty string`);
    }
    const source = target as Record<string, unknown>;
    // Model calls sometimes serialize an omitted target as an entirely empty
    // target object. Preserve that sentinel so the tool can ignore it while
    // still rejecting any target that supplies a real symbol with an empty path.
    const emptyTarget =
      source.symbol === "" &&
      ((hasOwn(source, "path") && source.path === "") ||
        (hasOwn(source, "filePath") && source.filePath === ""));
    if (emptyTarget) return { ...source };
    const normalized = { ...source };
    try {
      normalizeAliasPair(normalized, "path", "filePath", true);
    } catch (error) {
      if (error instanceof InvalidRequestError) {
        throw new InvalidRequestError(
          error.message
            .replace("'filePath'", `'targets[${index}].filePath'`)
            .replace("'path'", `'targets[${index}].path'`),
        );
      }
      throw error;
    }
    return normalized;
  };

  if (Array.isArray(targets)) {
    if (targets.length === 0) return;
    record.targets = targets.map(normalizeTarget);
    return;
  }

  if (targets && typeof targets === "object") {
    record.targets = normalizeTarget(targets, 0);
  }
}

function bareToolName(toolName: string): CanonicalPathTool | undefined {
  const bare = toolName.startsWith("aft_") ? toolName.slice(4) : toolName;
  if (
    bare === "read" ||
    bare === "write" ||
    bare === "edit" ||
    bare === "zoom" ||
    bare === "callgraph" ||
    bare === "safety" ||
    bare === "move" ||
    bare === "import" ||
    bare === "refactor" ||
    bare === "grep" ||
    bare === "search" ||
    bare === "conflicts"
  ) {
    return bare;
  }
  return undefined;
}

/**
 * Prepare raw arguments for one registered tool before schema validation.
 *
 * The returned object is a fresh object, and nested zoom targets are copied,
 * so an alias conflict or invalid value cannot partially mutate caller state.
 */
export function prepareCanonicalPathArguments(
  toolName: string,
  rawArguments: unknown,
): Record<string, unknown> {
  if (!rawArguments || typeof rawArguments !== "object" || Array.isArray(rawArguments)) {
    throw new InvalidRequestError("tool arguments must be an object");
  }

  const tool = bareToolName(toolName);
  const record = { ...(rawArguments as Record<string, unknown>) };
  if (!tool) return record;

  switch (tool) {
    case "read":
    case "write":
    case "edit":
    case "move":
    case "import":
    case "refactor":
      normalizeAliasPair(record, "path", "filePath", true);
      break;
    case "zoom":
      normalizeAliasPair(record, "path", "filePath", false);
      normalizeZoomTargets(record);
      break;
    case "callgraph":
      normalizeAliasPair(record, "path", "filePath", true);
      normalizeAliasPair(record, "toPath", "toFile", false);
      break;
    case "safety":
      normalizeAliasPair(record, "path", "filePath", false);
      break;
    case "grep":
    case "search":
    case "conflicts":
      validateOptionalCanonicalPath(record, "path");
      break;
  }

  return record;
}

const EDIT_ROOT_COMPATIBILITY_KEYS = new Set([
  "oldString",
  "newString",
  "replaceAll",
  "occurrence",
]);

const EDIT_ROOT_CANONICAL_KEYS = new Set(["path", "appendContent", "edits", "symbol", "content"]);

const EDIT_ITEM_KEYS = new Set([
  "oldString",
  "newString",
  "replaceAll",
  "occurrence",
  "startLine",
  "endLine",
  "content",
]);

const ASCII_WHITESPACE = /^[\t\n\v\f\r ]+$/;
const ASCII_TRIM = /^[\t\n\v\f\r ]+|[\t\n\v\f\r ]+$/g;
const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;

/**
 * Normalize a raw edit request before a host has parsed or stripped it.
 *
 * Edit has compatibility-only top-level fields that are intentionally absent
 * from its published schema. This function therefore owns mode selection,
 * item-family validation, and scalar coercion instead of relying on a typed
 * execute handler that may never see those fields.
 */
export function prepareCanonicalEditArguments(
  toolName: string,
  rawArguments: unknown,
): Record<string, unknown> {
  if (!rawArguments || typeof rawArguments !== "object" || Array.isArray(rawArguments)) {
    throw new InvalidRequestError("tool arguments must be an object");
  }

  const raw = rawArguments as Record<string, unknown>;
  const record = copyOwnProperties(raw);
  normalizeEditPathAlias(record);

  const suppliedLineFields = ["startLine", "endLine"].filter((key) => hasOwn(record, key));
  if (suppliedLineFields.length > 0) {
    throw new InvalidRequestError(
      `edit: top-level ${suppliedLineFields.map((key) => `'${key}'`).join(" and ")} are invalid; ` +
        "line-range fields are valid only inside 'edits[]'. " +
        "Use edits: [{ startLine, endLine, content }].",
    );
  }

  const isOpenCodeRetiredBoundary = toolName === "aft_edit";
  const retiredFields = ["file", "mode"].filter((key) => hasOwn(record, key));
  if (retiredFields.length > 0 && isOpenCodeRetiredBoundary) {
    throw new InvalidRequestError(
      "aft_edit: the retired `mode`/`file` edit form is no longer supported; use `path` with " +
        "exactly one of `appendContent`, `edits`, or `symbol` plus `content`.",
    );
  }

  const unknownRootKeys = Object.getOwnPropertyNames(record)
    .filter(
      (key) =>
        !EDIT_ROOT_CANONICAL_KEYS.has(key) &&
        !EDIT_ROOT_COMPATIBILITY_KEYS.has(key) &&
        key !== "filePath",
    )
    .sort();
  if (unknownRootKeys.length > 0) {
    throw new InvalidRequestError(formatUnknownKeys(unknownRootKeys));
  }

  const modes = editModesPresent(record);
  if (hasOrphanedSymbolContent(record)) {
    throw new InvalidRequestError(
      "edit: 'content' requires a non-empty string 'symbol' when symbol mode is selected",
    );
  }
  if (modes.length > 1) {
    throw new InvalidRequestError(
      `edit: conflicting modes: ${modes.join(", ")}. ${OMIT_OPTIONAL_FIELDS_STEERING}`,
    );
  }
  if (modes.length === 0) {
    throw new InvalidRequestError(
      "edit: exactly one of `appendContent`, `edits`, or `symbol` plus `content` is required. " +
        OMIT_OPTIONAL_FIELDS_STEERING,
    );
  }

  const mode = modes[0];
  if (mode === "appendContent") {
    if (typeof record.appendContent !== "string") {
      throw new InvalidRequestError("edit: 'appendContent' must be a string");
    }
  } else if (mode === "edits") {
    const parsedEdits = parseEditArray(record.edits);
    record.edits = parsedEdits.map((item, index) => normalizeEditItem(item, index));
  } else if (mode === "symbol/content") {
    if (!hasOwn(record, "symbol") || typeof record.symbol !== "string") {
      throw new InvalidRequestError("edit: 'symbol' must be a string when symbol mode is selected");
    }
    if (!hasOwn(record, "content") || typeof record.content !== "string") {
      throw new InvalidRequestError(
        "edit: symbol mode requires both 'symbol' and 'content' string properties",
      );
    }
  } else {
    const item: Record<string, unknown> = {};
    for (const key of EDIT_ROOT_COMPATIBILITY_KEYS) {
      if (hasOwn(record, key)) item[key] = record[key];
    }
    record.edits = [normalizeEditItem(item, 0)];
    for (const key of EDIT_ROOT_COMPATIBILITY_KEYS) delete record[key];
  }

  // Canonical-only path validation is deliberately last. Alias conflicts and
  // legacy-only aliases must be decided during preparation, but a malformed
  // canonical-only path must not hide a higher-precedence edit contract error.
  validateEditPath(record);
  return record;
}

function normalizeEditPathAlias(record: Record<string, unknown>): void {
  const hasCanonical = hasOwn(record, "path");
  const hasLegacy = hasOwn(record, "filePath");
  if (!hasCanonical && !hasLegacy) return;

  if (hasCanonical && hasLegacy) {
    let canonical: string;
    let legacy: string;
    try {
      canonical = pathValue(record, "path");
      legacy = pathValue(record, "filePath");
    } catch {
      throw new InvalidRequestError(
        "Invalid request: 'path' and 'filePath' must both be non-empty well-formed Unicode strings",
      );
    }
    if (canonical !== legacy) {
      throw new InvalidRequestError(
        "Invalid request: 'path' and 'filePath' must contain equal decoded strings",
      );
    }
    delete record.filePath;
    return;
  }

  if (!hasCanonical) {
    record.path = pathValue(record, "filePath");
    delete record.filePath;
  }
}

function validateEditPath(record: Record<string, unknown>): void {
  if (!hasOwn(record, "path")) {
    throw new InvalidRequestError("'path' is required");
  }
  pathValue(record, "path");
}

function formatUnknownKeys(keys: string[]): string {
  return `Unrecognized keys: ${keys.map((key) => `"${key}"`).join(", ")}`;
}

function editModesPresent(record: Record<string, unknown>): string[] {
  // Some hosts serialize every optional field with an empty sentinel. Remove
  // fields that cannot select a mode so later translation cannot revive them.
  const hasAppendContent = isNonEmptyString(record.appendContent);
  if (!hasAppendContent) delete record.appendContent;

  const hasEdits = normalizeEditArraySentinels(record);
  if (!hasEdits) delete record.edits;

  const hasSymbol = isNonEmptyString(record.symbol);
  if (!hasSymbol) {
    delete record.symbol;
    if (record.content === null || record.content === "") delete record.content;
  } else if (record.content === null) {
    delete record.content;
  }

  const hasSingleEdit = isNonEmptyString(record.oldString);
  if (!hasSingleEdit) {
    for (const key of EDIT_ROOT_COMPATIBILITY_KEYS) delete record[key];
  } else {
    for (const key of ["newString", "replaceAll", "occurrence"]) {
      if (record[key] === null) delete record[key];
    }
  }

  const modes: string[] = [];
  if (hasAppendContent) modes.push("appendContent");
  if (hasEdits) modes.push("edits");
  if (hasSymbol) modes.push("symbol/content");
  if (hasSingleEdit) modes.push("oldString/newString");
  return modes;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

const OMIT_OPTIONAL_FIELDS_STEERING =
  "Omit unused optional fields entirely; do not send empty strings or empty arrays for them.";

/**
 * An edits item is a serialization sentinel when the host emitted every
 * optional field with a type-default value and put the real payload in a
 * sibling field. Such an item carries no real edit intent and must not claim
 * the `edits` mode.
 *
 * A pure line-range item ({startLine,endLine,content}) has no `oldString` key,
 * so it is never a sentinel even when `content` is "" (deleting lines is real
 * intent). A real replacement has a non-empty `oldString`, so it is never a
 * sentinel. `{oldString:"", newString:"non-empty"}` is deliberately NOT a
 * sentinel: it is kept so the batch parser reports its specific empty-match
 * error instead of silently discarding a broken but intentional edit.
 */
function isEditSentinelItem(item: unknown): boolean {
  if (!item || typeof item !== "object" || Array.isArray(item)) return false;
  const record = item as Record<string, unknown>;
  if (record.oldString !== "") return false;
  const newStringEmpty =
    !hasOwn(record, "newString") || record.newString === "" || record.newString === null;
  if (!newStringEmpty) return false;
  return !hasOwn(record, "content") || record.content === "" || record.content === null;
}

/**
 * Filter serialization-sentinel items out of the edits array (or its
 * stringified form) and rewrite `record.edits` to the survivors. Returns
 * whether any real edit items remain, i.e. whether the edits mode is still
 * claimed. A non-empty malformed string (or a non-array root) stays an edits
 * claim so the existing parser can report its specific validation error.
 */
function normalizeEditArraySentinels(record: Record<string, unknown>): boolean {
  const value = record.edits;
  if (Array.isArray(value)) {
    const survivors = value.filter((item) => !isEditSentinelItem(item));
    if (survivors.length === 0) return false;
    record.edits = survivors;
    return true;
  }
  if (typeof value !== "string" || value.length === 0) return false;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed)) return true;
    const survivors = parsed.filter((item) => !isEditSentinelItem(item));
    if (survivors.length === 0) return false;
    record.edits = survivors;
    return true;
  } catch {
    // A non-empty malformed string is still an edits claim so the existing
    // parser can report its specific validation error instead of no-mode.
    return true;
  }
}

function hasOrphanedSymbolContent(record: Record<string, unknown>): boolean {
  return isNonEmptyString(record.content) && !isNonEmptyString(record.symbol);
}

function parseEditArray(value: unknown): unknown[] {
  if (typeof value === "string") {
    let parsed: unknown;
    try {
      parsed = JSON.parse(value);
    } catch {
      throw new InvalidRequestError("edit: 'edits' must contain valid JSON representing an array");
    }
    if (!Array.isArray(parsed)) {
      throw new InvalidRequestError("edit: 'edits' JSON must have an array root");
    }
    if (parsed.length === 0) {
      throw new InvalidRequestError("edit: 'edits' array must not be empty");
    }
    return parsed;
  }
  if (!Array.isArray(value)) {
    throw new InvalidRequestError("edit: 'edits' must be a non-empty array");
  }
  if (value.length === 0) {
    throw new InvalidRequestError("edit: 'edits' array must not be empty");
  }
  return value;
}

/**
 * Strip default values from the find/replace arm of a line-range edit.
 *
 * Some hosts serialize all optional fields. These values cannot affect a
 * line-range edit, while non-default values remain to surface a mixed-mode
 * request instead of being discarded.
 */
function stripLineRangeSentinels(item: Record<string, unknown>): void {
  const hasRangeField = ["startLine", "endLine", "content"].some((key) => hasOwn(item, key));
  if (!hasRangeField) return;

  if (item.oldString === "") delete item.oldString;
  if (item.newString === "") delete item.newString;
  if (item.replaceAll === false) delete item.replaceAll;
  if (item.occurrence === 1) delete item.occurrence;
}

function normalizeEditItem(value: unknown, index: number): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new InvalidRequestError(`edit: edits[${index}] must be an object`);
  }

  const source = value as Record<string, unknown>;
  const item = copyOwnProperties(source);
  normalizeItemAlias(item, "oldString", "oldText");
  normalizeItemAlias(item, "newString", "newText");
  stripLineRangeSentinels(item);

  const hasFindField = ["oldString", "newString", "replaceAll", "occurrence"].some((key) =>
    hasOwn(item, key),
  );
  const hasRangeField = ["startLine", "endLine", "content"].some((key) => hasOwn(item, key));
  if (hasFindField && hasRangeField) {
    throw new InvalidRequestError(`edit: edits[${index}] mixes find/replace and line-range fields`);
  }

  if (hasFindField) {
    if (!hasOwn(item, "oldString") || typeof item.oldString !== "string") {
      throw new InvalidRequestError(`edit: edits[${index}] requires string 'oldString'`);
    }
    if (hasOwn(item, "newString") && typeof item.newString !== "string") {
      throw new InvalidRequestError(`edit: edits[${index}].newString must be a string`);
    }
    coerceEditScalars(item, index);
    validateEditItemKeys(item, index);
    return item;
  }

  if (hasRangeField) {
    for (const key of ["startLine", "endLine"]) {
      // Models routinely send stringified line numbers ("3"); coerce exact
      // integer strings before validating, matching the other edit scalars.
      const value = item[key];
      if (typeof value === "string" && /^[0-9]+$/.test(value.trim())) {
        item[key] = Number(value.trim());
      }
      if (!hasOwn(item, key) || !isPositiveSafeInteger(item[key])) {
        throw new InvalidRequestError(`edit: edits[${index}].${key} must be a positive integer`);
      }
    }
    if ((item.startLine as number) > (item.endLine as number)) {
      throw new InvalidRequestError(`edit: edits[${index}] requires startLine <= endLine`);
    }
    if (!hasOwn(item, "content") || typeof item.content !== "string") {
      throw new InvalidRequestError(`edit: edits[${index}] requires string 'content'`);
    }
    validateEditItemKeys(item, index);
    return item;
  }

  throw new InvalidRequestError(`edit: edits[${index}] must be a find/replace or line-range item`);
}

function normalizeItemAlias(
  item: Record<string, unknown>,
  canonical: string,
  legacy: string,
): void {
  if (hasOwn(item, legacy)) {
    if (!hasOwn(item, canonical)) item[canonical] = item[legacy];
    delete item[legacy];
  }
}

function validateEditItemKeys(item: Record<string, unknown>, index: number): void {
  const unknown = Object.getOwnPropertyNames(item)
    .filter((key) => !EDIT_ITEM_KEYS.has(key))
    .sort();
  if (unknown.length > 0) {
    throw new InvalidRequestError(`edit: edits[${index}] contains ${formatUnknownKeys(unknown)}`);
  }
}

function coerceEditScalars(item: Record<string, unknown>, index: number): void {
  if (hasOwn(item, "replaceAll") && hasOwn(item, "occurrence")) {
    throw new InvalidRequestError(
      `edit: edits[${index}] cannot contain both 'replaceAll' and 'occurrence'`,
    );
  }
  if (hasOwn(item, "replaceAll")) item.replaceAll = coerceEditBoolean(item.replaceAll, index);

  if (hasOwn(item, "occurrence")) {
    const occurrence = coerceEditOccurrence(item.occurrence, index);
    if (occurrence === undefined) delete item.occurrence;
    else item.occurrence = occurrence;
  }
}

function coerceEditBoolean(value: unknown, index: number): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number" && Number.isFinite(value) && (value === 0 || value === 1)) {
    return value === 1;
  }
  if (typeof value === "string") {
    if (value === "1") return true;
    if (value === "0") return false;
    if (/^(?:true|false)$/i.test(value)) return value.toLowerCase() === "true";
  }
  throw new InvalidRequestError(
    `edit: edits[${index}].replaceAll must be a boolean, true/false string, or 0/1`,
  );
}

function coerceEditOccurrence(value: unknown, index: number): number | undefined {
  if (value === null) return undefined;
  if (typeof value === "string") {
    const trimmed = value.replace(ASCII_TRIM, "");
    if (trimmed.length === 0 || ASCII_WHITESPACE.test(trimmed)) return undefined;
    if (!/^[+]?[0-9]+$/.test(trimmed)) {
      throw new InvalidRequestError(`edit: edits[${index}].occurrence must be a positive integer`);
    }
    try {
      const parsed = BigInt(trimmed);
      if (parsed < 1n || parsed > BigInt(MAX_SAFE_INTEGER)) throw new Error("out of range");
      return Number(parsed);
    } catch {
      throw new InvalidRequestError(`edit: edits[${index}].occurrence must be a positive integer`);
    }
  }
  if (typeof value === "number" && isPositiveSafeInteger(value)) return value;
  throw new InvalidRequestError(`edit: edits[${index}].occurrence must be a positive integer`);
}

function isPositiveSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 1;
}

function copyOwnProperties(source: Record<string, unknown>): Record<string, unknown> {
  const copy = Object.create(null) as Record<string, unknown>;
  for (const key of Object.getOwnPropertyNames(source)) copy[key] = source[key];
  return copy;
}
