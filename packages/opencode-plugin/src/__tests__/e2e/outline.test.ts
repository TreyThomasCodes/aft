/// <reference path="../../bun-test.d.ts" />

import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import {
  cleanupHarnesses,
  createHarness,
  discoverOutlineFiles,
  type E2EHarness,
  type PreparedBinary,
  prepareBinary,
  sendOutlineDirectoryLikePlugin,
} from "./helpers.js";

const initialBinary = await prepareBinary();
const maybeDescribe = describe.skipIf(!initialBinary.binaryPath);

maybeDescribe("e2e outline command", () => {
  let preparedBinary: PreparedBinary = initialBinary;
  const harnesses: E2EHarness[] = [];

  beforeAll(async () => {
    preparedBinary = await prepareBinary();
  });

  afterEach(async () => {
    await cleanupHarnesses(harnesses);
  });

  async function harness(): Promise<E2EHarness> {
    const created = await createHarness(preparedBinary);
    harnesses.push(created);
    return created;
  }

  test("outlines a single file with signatures", async () => {
    const h = await harness();

    const response = await h.bridge.send("outline", { file: h.path("sample.ts") });

    expect(response.success).toBe(true);
    expect(String(response.text)).toContain("sample.ts");
    // Signature prefix dedup (v0.49.4): the redundant kind marker was dropped
    // from signature lines; exported-ness shows as a bare `E` marker only where
    // the signature itself doesn't carry `export`.
    expect(String(response.text)).toContain("funcA(input: string): string");
    expect(String(response.text)).toContain("SampleService");
  });

  test("outlines multiple files with relative paths and no signatures", async () => {
    const h = await harness();
    const files = [h.path("sample.ts"), h.path("sample.py")];

    const response = await h.bridge.send("outline", { files });

    expect(response.success).toBe(true);
    const text = String(response.text);
    expect(text).toContain("sample.ts");
    expect(text).toContain("sample.py");
    expect(text).toContain("funcA");
    expect(text).not.toContain("funcA(input: string): string");
  });

  test("outlines a directory using plugin discovery semantics", async () => {
    const h = await harness();

    const response = await sendOutlineDirectoryLikePlugin(h.bridge, h.tempDir);

    expect(response.success).toBe(true);
    const text = String(response.text);
    expect(text).toContain("directory/");
    expect(text).toContain("alpha.ts");
    expect(text).toContain("sample.ts");
  });

  test("outlines markdown heading hierarchy", async () => {
    const h = await harness();

    const response = await h.bridge.send("outline", { file: h.path("sample.md") });

    expect(response.success).toBe(true);
    const text = String(response.text);
    expect(text).toContain("Project Title");
    expect(text).toContain("Features");
    expect(text).toContain("Fast Path");
    // Markdown outlines render heading markers (#, ##) directly since the
    // v0.49.4 prefix dedup; the old ` h ` kind column is gone.
    expect(text).toContain("## Features");
  });

  test("directory discovery respects the 200 file cap", async () => {
    const h = await harness();
    await mkdir(h.path("bulk"), { recursive: true });
    for (let index = 0; index < 205; index += 1) {
      await writeFile(
        h.path("bulk", `file-${index}.ts`),
        `export const value${index} = ${index};\n`,
      );
    }

    const files = await discoverOutlineFiles(h.tempDir);
    expect(files.length).toBe(200);
  });
});
