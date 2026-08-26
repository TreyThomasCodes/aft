/**
 * Renderer coverage for hoisted write/edit call + result summaries.
 */

/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import { buildMutationResult, renderMutationCall, renderMutationResult } from "../tools/hoisted.js";
import { makeContext, makeResult, mockTheme, renderToString } from "./render-test-helpers.js";

describe("hoisted renderers", () => {
  test("renderMutationCall shows the tool name and path", () => {
    const out = renderToString(
      renderMutationCall(
        "edit",
        "src/batch.ts",
        mockTheme,
        makeContext({ filePath: "src/batch.ts" }),
      ),
    );

    expect(out).toContain("edit");
    expect(out).toContain("src/batch.ts");
  });

  test("renderMutationResult keeps CRLF diffs visible and terminal-safe", () => {
    const unchanged = Array.from({ length: 10 }, (_, index) => `context ${index}`).join("\r\n");
    const result = buildMutationResult({
      text: "Edited",
      diff: {
        before: `old line\r\n${unchanged}\r\n`,
        after: `new line\r\n${unchanged}\r\n`,
        additions: 1,
        deletions: 1,
      },
    });
    const context = makeContext({ path: "src/crlf.ts" });

    const collapsed = renderToString(
      renderMutationResult(result, mockTheme, context, { expanded: false }),
    );
    expect(collapsed).toContain("edited src/crlf.ts (+1/-1)");
    expect(collapsed).not.toContain("old line");
    expect(collapsed).not.toContain("\r");

    const expanded = renderToString(
      renderMutationResult(result, mockTheme, context, { expanded: true }),
    );
    expect(expanded).toContain("old line");
    expect(expanded).toContain("new line");
    expect(expanded).not.toContain("\r");
  });

  test("renderMutationResult keeps batch edit counts when only summary counts are available", () => {
    const out = renderToString(
      renderMutationResult(
        makeResult("Edited (+4/-4, 2 edits).", {
          additions: 4,
          deletions: 4,
          editsApplied: 2,
          truncated: true,
        }),
        mockTheme,
        makeContext({ filePath: "src/batch.ts" }),
      ),
    );

    expect(out).toContain("+4/-4, 2 edits");
    expect(out).toContain("diff truncated");
  });

  test("renderMutationResult collapses large diffs and preserves tiny diffs", () => {
    const diff = [
      "@@ -1,8 +1,8 @@",
      "-old line 1",
      "+new line 1",
      " context 1",
      "-old line 2",
      "+new line 2",
      " context 2",
      " context 3",
      " context 4",
    ].join("\n");
    const result = makeResult("Edited", { diff, additions: 1, deletions: 1 });

    const collapsed = renderToString(
      renderMutationResult(result, mockTheme, makeContext({ path: "src/batch.ts" }), {
        expanded: false,
      }),
    );
    expect(collapsed).toContain("edited src/batch.ts (+1/-1)");
    expect(collapsed).not.toContain("old line");

    const expanded = renderToString(
      renderMutationResult(result, mockTheme, makeContext({ path: "src/batch.ts" }), {
        expanded: true,
      }),
    );
    expect(expanded).toContain("old line");
    expect(expanded).toContain("new line");

    const tiny = renderToString(
      renderMutationResult(
        makeResult("Edited", {
          diff: "@@ -1 +1 @@\n-old\n+new",
          additions: 1,
          deletions: 1,
        }),
        mockTheme,
        makeContext({ path: "src/tiny.ts" }),
        { expanded: false },
      ),
    );
    expect(tiny).toContain("old");
    expect(tiny).toContain("new");
  });
});
