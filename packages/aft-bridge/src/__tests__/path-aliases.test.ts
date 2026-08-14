import { describe, expect, test } from "bun:test";
import {
  InvalidRequestError,
  isWellFormedUnicodeString,
  prepareCanonicalEditArguments,
  prepareCanonicalPathArguments,
} from "../path-aliases.js";

function expectInvalid(tool: string, args: unknown, fields: string[] = ["path", "filePath"]): void {
  try {
    prepareCanonicalPathArguments(tool, args);
    throw new Error("expected invalid_request");
  } catch (error) {
    expect(error).toBeInstanceOf(InvalidRequestError);
    expect((error as InvalidRequestError).code).toBe("invalid_request");
    for (const field of fields) expect((error as Error).message).toContain(field);
  }
}

describe("canonical path alias preparation", () => {
  test.each([
    ["canonical only", { path: "src/main.ts" }, { path: "src/main.ts" }],
    ["legacy only", { filePath: "src/main.ts" }, { path: "src/main.ts" }],
    [
      "equal dual spelling",
      { path: "src/main.ts", filePath: "src/main.ts" },
      { path: "src/main.ts" },
    ],
    [
      "equivalent JSON escapes",
      { path: "src/a.ts", filePath: "src/\u0061.ts" },
      { path: "src/a.ts" },
    ],
    [
      "supplementary character",
      { path: "src/😀.ts", filePath: "src/😀.ts" },
      { path: "src/😀.ts" },
    ],
  ])("accepts %s", (_label, input, expected) => {
    expect(prepareCanonicalPathArguments("read", input)).toEqual(expected);
  });

  test("compares decoded strings without path transformations", () => {
    const input = { path: " src\\main.ts ", filePath: "src/main.ts" };
    expectInvalid("read", input);
    expect(input).toEqual({ path: " src\\main.ts ", filePath: "src/main.ts" });
  });

  test("rejects unequal and incompatible dual spellings atomically", () => {
    expectInvalid("read", { path: "src/a.ts", filePath: "src/b.ts" });
    expectInvalid("read", { path: "src/a.ts", filePath: 42 });
    expectInvalid("read", { path: 42, filePath: "src/a.ts" });
  });

  test("does not normalize canonically distinct Unicode spellings", () => {
    expectInvalid("read", { path: "src/é.ts", filePath: "src/e\u0301.ts" });
  });

  test("requires a non-empty canonical path without trimming", () => {
    expectInvalid("read", { path: "" }, ["path"]);
    expectInvalid("read", { path: 42 }, ["path"]);
    expect(prepareCanonicalPathArguments("read", { path: " " }).path).toBe(" ");
  });

  test("rejects malformed UTF-16 at the preparation boundary", () => {
    expect(isWellFormedUnicodeString("😀")).toBe(true);
    expect(isWellFormedUnicodeString("\ud800")).toBe(false);
    expect(isWellFormedUnicodeString("\udc00")).toBe(false);
    expectInvalid("read", { path: "\ud800" }, ["path"]);
    expectInvalid("read", { path: "src/a.ts", filePath: "\ud800" });
  });

  test("normalizes nested zoom targets and callgraph target aliases", () => {
    expect(
      prepareCanonicalPathArguments("zoom", {
        targets: [{ filePath: "src/main.ts", symbol: "main" }],
      }),
    ).toEqual({ targets: [{ path: "src/main.ts", symbol: "main" }] });
    expect(
      prepareCanonicalPathArguments("callgraph", {
        path: "src/main.ts",
        toFile: "src/target.ts",
        op: "trace_to_symbol",
        symbol: "main",
        toSymbol: "target",
      }),
    ).toMatchObject({ path: "src/main.ts", toPath: "src/target.ts" });
    expectInvalid("callgraph", {
      path: "src/main.ts",
      filePath: "src/other.ts",
      symbol: "main",
      op: "callers",
    });
  });

  test("leaves role-specific collection and destination properties unchanged", () => {
    const input = {
      files: ["src/a.ts"],
      destination: "src/b.ts",
      target: "src/c.ts",
      path: "src/main.ts",
    };
    expect(prepareCanonicalPathArguments("move", input)).toEqual({
      ...input,
    });
  });
});

describe("edit boundary preparation", () => {
  const meaningfulModeCases: Array<{
    label: string;
    input: Record<string, unknown>;
    expected?: Record<string, unknown>;
    error?: string;
  }> = [
    {
      label: "edits ignores empty mode sentinels",
      input: {
        filePath: "src/example.ts",
        edits: [{ oldString: "old", newString: "new" }],
        appendContent: "",
        symbol: "",
        content: "",
      },
      expected: {
        path: "src/example.ts",
        edits: [{ oldString: "old", newString: "new" }],
      },
    },
    {
      label: "append ignores empty edits",
      input: {
        filePath: "src/example.ts",
        appendContent: "append",
        edits: [],
      },
      expected: { path: "src/example.ts", appendContent: "append" },
    },
    {
      label: "symbol deletion keeps empty content",
      input: { filePath: "src/example.ts", symbol: "target", content: "" },
      expected: { path: "src/example.ts", symbol: "target", content: "" },
    },
    {
      label: "content without a symbol is rejected",
      input: { filePath: "src/example.ts", symbol: "", content: "replacement" },
      error: "requires a non-empty string 'symbol'",
    },
    {
      label: "two real modes conflict",
      input: {
        filePath: "src/example.ts",
        appendContent: "append",
        edits: [{ oldString: "old", newString: "new" }],
      },
      error: "conflicting modes",
    },
    {
      label: "all empty fields have no mode",
      input: {
        filePath: "src/example.ts",
        appendContent: "",
        edits: [],
        symbol: "",
        content: "",
        oldString: "",
        newString: "",
        replaceAll: null,
        occurrence: null,
      },
      error: "exactly one of",
    },
    {
      label: "whole-schema sentinel report resolves to appendContent",
      input: {
        path: "src/example.ts",
        symbol: "",
        content: "",
        appendContent: "CONTENT IT APPENDS",
        edits: [
          {
            oldString: "",
            newString: "",
            replaceAll: false,
            occurrence: 1,
            startLine: 1,
            endLine: 1,
            content: "",
          },
        ],
      },
      expected: { path: "src/example.ts", appendContent: "CONTENT IT APPENDS" },
    },
    {
      label: "sentinel item alongside a real item keeps the real item",
      input: {
        path: "src/example.ts",
        edits: [
          { oldString: "", newString: "", content: "" },
          { oldString: "before", newString: "after" },
        ],
      },
      expected: { path: "src/example.ts", edits: [{ oldString: "before", newString: "after" }] },
    },
    {
      label: "line-range delete item is never treated as a sentinel",
      input: {
        path: "src/example.ts",
        edits: [{ startLine: 1, endLine: 1, content: "" }],
      },
      expected: { path: "src/example.ts", edits: [{ startLine: 1, endLine: 1, content: "" }] },
    },
    {
      label: "empty-old with real replacement is kept for the batch error",
      input: {
        path: "src/example.ts",
        edits: [{ oldString: "", newString: "real" }],
      },
      expected: { path: "src/example.ts", edits: [{ oldString: "", newString: "real" }] },
    },
    {
      label: "line-range edit removes embedded find/replace sentinels",
      input: {
        path: "src/example.ts",
        edits: [
          {
            content: "const value = new;",
            startLine: 14,
            endLine: 14,
            oldString: "",
            newString: "",
            replaceAll: false,
            occurrence: 1,
          },
        ],
      },
      expected: {
        path: "src/example.ts",
        edits: [{ content: "const value = new;", startLine: 14, endLine: 14 }],
      },
    },
  ];

  for (const { label, input, expected, error } of meaningfulModeCases) {
    test(label, () => {
      if (error) {
        expect(() => prepareCanonicalEditArguments("edit", input)).toThrow(error);
      } else {
        expect(prepareCanonicalEditArguments("edit", input)).toEqual(expected);
      }
    });
  }

  test("retains meaningful fields alongside line-range edits as mixed-mode errors", () => {
    const lineRange = { content: "replacement", startLine: 14, endLine: 14 };
    for (const findFields of [
      { oldString: "meaningful", newString: "" },
      { oldString: "", newString: "", replaceAll: true },
      { oldString: "", newString: "", occurrence: 2 },
    ]) {
      expect(() =>
        prepareCanonicalEditArguments("edit", {
          path: "src/example.ts",
          edits: [{ ...lineRange, ...findFields }],
        }),
      ).toThrow("mixes find/replace and line-range fields");
    }
  });

  test("applies mode conflict precedence before parsing stringified edits", () => {
    expect(() =>
      prepareCanonicalEditArguments("edit", {
        path: "src/main.ts",
        appendContent: "append",
        edits: "not-json",
      }),
    ).toThrow("conflicting modes");

    expect(() =>
      prepareCanonicalEditArguments("edit", {
        path: "src/main.ts",
        edits: "not-json",
      }),
    ).toThrow("valid JSON");
    expect(
      prepareCanonicalEditArguments("edit", {
        path: "src/main.ts",
        edits: '[{"oldString":"before","newString":"after"}]',
      }),
    ).toEqual({
      path: "src/main.ts",
      edits: [{ oldString: "before", newString: "after" }],
    });
    expect(() =>
      prepareCanonicalEditArguments("edit", {
        path: "src/main.ts",
        edits: "[]",
      }),
    ).toThrow("exactly one of");
  });

  test("keeps canonical-only path validation after edit contract validation", () => {
    expect(() =>
      prepareCanonicalEditArguments("edit", {
        path: 42,
        startLine: 1,
      }),
    ).toThrow("startLine");
    expect(() => prepareCanonicalEditArguments("edit", { path: 42 })).toThrow("exactly one of");
    expect(() =>
      prepareCanonicalEditArguments("edit", { path: 42, appendContent: "append" }),
    ).toThrow("'path'");
  });

  test("uses the retired-form error only at the OpenCode-prefixed raw boundary", () => {
    expect(() =>
      prepareCanonicalEditArguments("aft_edit", {
        mode: "write",
        file: "src/main.ts",
        content: "x",
      }),
    ).toThrow("retired");

    expect(() =>
      prepareCanonicalEditArguments("edit", { mode: "write", file: "src/main.ts" }),
    ).toThrow('Unrecognized keys: "file", "mode"');
  });

  test("normalizes item aliases and the complete scalar compatibility domains", () => {
    const replaceAllValues: unknown[] = [true, false, "true", "TRUE", "fAlSe", 1, 0, "1", "0"];
    for (const value of replaceAllValues) {
      const result = prepareCanonicalEditArguments("edit", {
        path: "src/main.ts",
        edits: [{ oldText: "before", newText: "after", replaceAll: value }],
      });
      const expected =
        value === true ||
        value === 1 ||
        value === "1" ||
        (typeof value === "string" && value.toLowerCase() === "true");
      expect(result.edits).toEqual([
        { oldString: "before", newString: "after", replaceAll: expected },
      ]);
    }

    const result = prepareCanonicalEditArguments("edit", {
      path: "src/main.ts",
      edits: [{ oldString: "before", oldText: "legacy", occurrence: " +01 " }],
    });
    expect(result.edits).toEqual([{ oldString: "before", occurrence: 1 }]);

    for (const value of [null, "", " \t"]) {
      const omitted = prepareCanonicalEditArguments("edit", {
        path: "src/main.ts",
        edits: [{ oldString: "before", occurrence: value }],
      });
      expect(omitted.edits).toEqual([{ oldString: "before" }]);
    }

    for (const value of ["0", "00", "+0", "1.0", "1e0", "0x1", "-1", "9007199254740992"]) {
      expect(() =>
        prepareCanonicalEditArguments("edit", {
          path: "src/main.ts",
          edits: [{ oldString: "before", occurrence: value }],
        }),
      ).toThrow("occurrence");
    }
  });
});
