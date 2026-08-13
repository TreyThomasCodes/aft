/// <reference path="../bun-test.d.ts" />

import { expect, test } from "bun:test";
import type { ToolDefinition } from "@opencode-ai/plugin";
import { prepareToolMap } from "../normalize-schemas.js";

test("prepareToolMap normalizes a counting probe exactly once after repeated preparation", async () => {
  let aliasWrites = 0;
  let executions = 0;
  const probe = {
    description: "count argument preparation",
    args: {},
    execute: async (args: unknown) => {
      executions += 1;
      return JSON.stringify(args);
    },
  } as unknown as ToolDefinition;
  const tools = { edit: probe };

  prepareToolMap(tools, { hashlineEffective: true });
  const preparedExecute = probe.execute;
  prepareToolMap(tools, { hashlineEffective: true });

  const rawArguments = new Proxy<Record<string, unknown>>(
    { path: "sample.ts" },
    {
      getOwnPropertyDescriptor(target, property) {
        if (property === "filePath") return undefined;
        return Reflect.getOwnPropertyDescriptor(target, property);
      },
      set(target, property, value, receiver) {
        if (property === "filePath") {
          aliasWrites += 1;
          return true;
        }
        return Reflect.set(target, property, value, receiver);
      },
    },
  );

  await probe.execute(rawArguments, {} as never);

  expect(probe.execute).toBe(preparedExecute);
  expect(aliasWrites).toBe(1);
  expect(executions).toBe(1);
});
