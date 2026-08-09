/**
 * Host-neutral error adaptation for agent tool failures.
 *
 * The Rust response owns the logical code, message, and structured fields. Host
 * adapters may attach those values to their native error surface, but must not
 * rewrite the contract-owned message while doing so.
 */

import {
  isConsumerReconnectTransient,
  StaleRouteHandleError,
  SubcError,
} from "@cortexkit/subc-client";

import { isBridgeTransportTimeout } from "./bridge.js";
import { SubcRootGenerationExpiredError, SubcRootReapedError } from "./subc-transport.js";

export interface AftToolErrorCause {
  code: string;
  message: string;
  response: Record<string, unknown>;
}

export class AftToolError extends Error {
  readonly code: string;
  readonly response: Record<string, unknown>;
  declare readonly cause: AftToolErrorCause;

  constructor(message: string, code: string, response: Record<string, unknown>) {
    const cause: AftToolErrorCause = { code, message, response };
    super(message, { cause });
    this.name = "AftToolError";
    this.code = code;
    this.response = response;
  }
}

/**
 * Lift a failed bridge response into a host error without losing its logical
 * code or structured response fields.
 */
export function toolErrorFromResponse(
  command: string,
  response: Record<string, unknown>,
): AftToolError {
  const code =
    typeof response.code === "string" && response.code.length > 0 ? response.code : "unknown_error";
  const message =
    typeof response.message === "string" && response.message.length > 0
      ? response.message
      : `${command} failed`;
  return new AftToolError(message, code, response);
}

/** Agent-facing guidance for a bash request whose transport outcome is unknown. */
export const BASH_TRANSPORT_DISPOSITION =
  "The transport to the AFT daemon was interrupted; no background task was created for this command and no task ID exists. Re-run the command. Do not poll bash_status for it.";

/**
 * Agent-facing guidance for a call the daemon GOODBYE'd mid-flight.
 *
 * Deliberately does NOT say the call failed. The daemon emits route GOODBYEs
 * after its drain wait regardless of whether that drain completed, so a call
 * in flight at GOODBYE was admitted BEFORE the gate closed and may already
 * have run to completion with only its reply lost. "Failed" reads as an
 * invitation to re-run, which double-applies a mutation that already landed.
 */
export const SUBC_MODULE_RESTART_DISPOSITION =
  "The AFT daemon module restarted while this call was in flight, so its outcome is UNKNOWN: it may or may not have executed. Verify actual state before re-running, and never blind-retry a mutation.";

/**
 * A route GOODBYE delivered against an in-flight request.
 *
 * COUPLING: subc-client raises this as a bare `SubcError` carrying no code
 * (client.ts, the `FrameType.Goodbye` branch), so the message literal is the
 * only discriminator available. Asked upstream for a stable `code` on that
 * error; until it exists this match is the seam and will fail open (no
 * disposition appended) rather than misclassify.
 */
function isRouteGoodbyeError(error: unknown): boolean {
  return (
    error instanceof SubcError &&
    error.code === undefined &&
    error.message.includes("route closed by subc")
  );
}

function isTransportClassError(error: unknown): boolean {
  return (
    isBridgeTransportTimeout(error) ||
    isConsumerReconnectTransient(error) ||
    error instanceof StaleRouteHandleError ||
    error instanceof SubcRootGenerationExpiredError ||
    error instanceof SubcRootReapedError
  );
}

/**
 * Add bash execution recovery guidance without changing the original error
 * object, class, code, or retry behavior. Other commands retain their errors.
 */
export function adaptToolError(command: string, error: unknown): unknown {
  if (!(error instanceof Error)) return error;

  // Checked before the bash branch, and applied to every command: a GOODBYE'd
  // call has an unknown outcome, so BASH_TRANSPORT_DISPOSITION's "no task was
  // created, re-run the command" would be actively wrong here.
  if (isRouteGoodbyeError(error)) {
    if (error.message.includes(SUBC_MODULE_RESTART_DISPOSITION)) return error;
    error.message = error.message
      ? `${error.message} ${SUBC_MODULE_RESTART_DISPOSITION}`
      : SUBC_MODULE_RESTART_DISPOSITION;
    return error;
  }

  if (command !== "bash" || !isTransportClassError(error)) return error;
  if (error.message.includes(BASH_TRANSPORT_DISPOSITION)) return error;
  error.message = error.message
    ? `${error.message} ${BASH_TRANSPORT_DISPOSITION}`
    : BASH_TRANSPORT_DISPOSITION;
  return error;
}
