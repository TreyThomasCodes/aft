/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import {
  BASH_HOST_FALLBACK_BANNER,
  bashHostFallbackAskPattern,
  runBashHostFallback,
} from "../bash-host-fallback.js";

describe("bash host fallback", () => {
  test("permission pattern carries the exact command and project root", () => {
    const command = "printf 'exact  value'\nprintf done";
    const cwd = "/tmp/project with spaces";

    expect(bashHostFallbackAskPattern(command, cwd)).toBe(
      `AFT UNAVAILABLE - host fallback execution:\n\nExact command:\n${command}\n\nWorking directory:\n${cwd}`,
    );
  });

  test("captures real stdout and stderr with banner and exit code", async () => {
    const result = await runBashHostFallback({
      command:
        process.platform === "win32"
          ? `${JSON.stringify(process.execPath)} -e "process.stdout.write('stdout'); process.stderr.write('stderr'); process.exit(3)"`
          : "printf stdout; printf stderr >&2; exit 3",
      projectRoot: process.cwd(),
      timeoutMs: 5_000,
    });

    expect(result.output).toStartWith(`${BASH_HOST_FALLBACK_BANNER}\n`);
    expect(result.output).toContain("stdout");
    expect(result.output).toContain("stderr");
    expect(result.output).toEndWith("[exit code: 3]");
    expect(result.exit_code).toBe(3);
  });

  test("keeps only the 100 KB output tail", async () => {
    const result = await runBashHostFallback({
      command: `${JSON.stringify(process.execPath)} -e "process.stdout.write('x'.repeat(120 * 1024)); process.stdout.write('TAIL')"`,
      projectRoot: process.cwd(),
      timeoutMs: 5_000,
    });

    expect(result.truncated).toBe(true);
    expect(result.output).toContain("TAIL");
    expect(Buffer.byteLength(result.output)).toBeLessThanOrEqual(100 * 1024 + 200);
  });

  test("hard timeout kills the command and reports exit code 124", async () => {
    const result = await runBashHostFallback({
      command: `${JSON.stringify(process.execPath)} -e "setInterval(() => {}, 1000)"`,
      projectRoot: process.cwd(),
      timeoutMs: 20,
    });

    expect(result.exit_code).toBe(124);
    expect(result.output).toEndWith("[exit code: 124]");
  });

  test("an abort kills the inline child and rejects promptly", async () => {
    const controller = new AbortController();
    const started = Date.now();
    const running = runBashHostFallback({
      command: `${JSON.stringify(process.execPath)} -e "setInterval(() => {}, 1000)"`,
      projectRoot: process.cwd(),
      signal: controller.signal,
    });

    setTimeout(() => controller.abort(), 25);

    await expect(running).rejects.toMatchObject({ name: "AbortError" });
    expect(Date.now() - started).toBeLessThan(2_000);
  });
});
