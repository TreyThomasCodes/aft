/**
 * Host-neutral error adaptation for agent tool failures.
 *
 * The Rust response owns the logical code, message, and structured fields. Host
 * adapters may attach those values to their native error surface, but must not
 * rewrite the contract-owned message while doing so.
 */

import { isConsumerReconnectTransient, StaleRouteHandleError } from "@cortexkit/subc-client";

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
  if (command !== "bash" || !isTransportClassError(error)) return error;
  if (!(error instanceof Error)) return error;
  if (error.message.includes(BASH_TRANSPORT_DISPOSITION)) return error;
  error.message = error.message
    ? `${error.message} ${BASH_TRANSPORT_DISPOSITION}`
    : BASH_TRANSPORT_DISPOSITION;
  return error;
}
