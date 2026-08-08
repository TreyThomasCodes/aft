/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import { BridgeTransportTimeoutError, isBridgeTransportTimeout } from "../bridge.js";
import { adaptToolError, BASH_TRANSPORT_DISPOSITION } from "../error-contract.js";

describe("adaptToolError", () => {
  test("adds bash transport disposition guidance while preserving the error", () => {
    const original = new BridgeTransportTimeoutError("bash", 11_000, "bash transport timed out");

    let thrown: unknown;
    try {
      throw original;
    } catch (error) {
      thrown = adaptToolError("bash", error);
    }

    expect(thrown).toBe(original);
    expect(original.message).toBe(`bash transport timed out ${BASH_TRANSPORT_DISPOSITION}`);
    expect(isBridgeTransportTimeout(original)).toBe(true);
  });

  test("does not add bash transport guidance to non-bash commands", () => {
    const original = new BridgeTransportTimeoutError("read", 11_000, "read transport timed out");

    const adapted = adaptToolError("read", original);

    expect(adapted).toBe(original);
    expect(original.message).toBe("read transport timed out");
    expect(original.message).not.toContain(BASH_TRANSPORT_DISPOSITION);
  });
});
