/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import { SubcError } from "@cortexkit/subc-client";
import { BridgeTransportTimeoutError, isBridgeTransportTimeout } from "../bridge.js";
import {
  adaptToolError,
  BASH_TRANSPORT_DISPOSITION,
  SUBC_MODULE_RESTART_DISPOSITION,
} from "../error-contract.js";

/** Shaped exactly as subc-client raises it: bare SubcError, no code. */
function routeGoodbyeError(): SubcError {
  return new SubcError("route closed by subc (GOODBYE)");
}

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

  test("a route GOODBYE reports an UNKNOWN outcome, never a failure", () => {
    const original = routeGoodbyeError();

    const adapted = adaptToolError("write", original);

    expect(adapted).toBe(original);
    expect(original.message).toContain(SUBC_MODULE_RESTART_DISPOSITION);
    // The wording is the safety property: an operator or agent that reads
    // "failed" re-runs the call, which double-applies a mutation that may
    // already have landed before the daemon dropped the reply.
    expect(original.message).toContain("UNKNOWN");
    expect(original.message).toContain("never blind-retry a mutation");
    expect(original.message).not.toContain("Re-run the command.");
  });

  test("a GOODBYE'd bash call gets the unknown-outcome text, not the re-run text", () => {
    // BASH_TRANSPORT_DISPOSITION asserts no task was created and says to re-run.
    // That is true for a not-sent transport failure and FALSE for a GOODBYE,
    // where the command may already have executed.
    const original = routeGoodbyeError();

    adaptToolError("bash", original);

    expect(original.message).toContain(SUBC_MODULE_RESTART_DISPOSITION);
    expect(original.message).not.toContain(BASH_TRANSPORT_DISPOSITION);
  });

  test("disposition is appended once when the error passes through twice", () => {
    const original = routeGoodbyeError();

    adaptToolError("read", original);
    adaptToolError("read", original);

    const occurrences = original.message.split(SUBC_MODULE_RESTART_DISPOSITION).length - 1;
    expect(occurrences).toBe(1);
  });

  test("a coded SubcError is left alone — only the bare GOODBYE shape matches", () => {
    // module_reloading is proven-not-forwarded and retryable; it must not be
    // dressed up as an unknown outcome.
    const coded = new SubcError("route closed by subc (GOODBYE)", "module_reloading");

    const adapted = adaptToolError("write", coded);

    expect(adapted).toBe(coded);
    expect(coded.message).not.toContain(SUBC_MODULE_RESTART_DISPOSITION);
  });
});
