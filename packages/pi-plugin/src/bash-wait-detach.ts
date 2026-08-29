import type { AftProjectTransport, AftTransportPool } from "@cortexkit/aft-bridge";
import type { AftConfig } from "./config.js";
import { resolveBashConfig } from "./config.js";
import { warn } from "./logger.js";

const BASH_TRANSPORT_TIMEOUT_MS = 30_000;

type ActiveBridgePool = Pick<AftTransportPool, "getActiveBridgeForRoot">;

export const BASH_WAIT_DETACH_MAGIC_KEYWORD = "&detach";
const EMPTY_DETACH_MESSAGE = "(requested background detach)";
const STANDALONE_DETACH_KEYWORD_SOURCE = `(^|[^\\p{L}\\p{N}_])${BASH_WAIT_DETACH_MAGIC_KEYWORD}(?![\\p{L}\\p{N}_])`;
const STANDALONE_DETACH_KEYWORD_PATTERN = new RegExp(STANDALONE_DETACH_KEYWORD_SOURCE, "u");
const STANDALONE_DETACH_KEYWORDS_PATTERN = new RegExp(STANDALONE_DETACH_KEYWORD_SOURCE, "gu");

function containsStandaloneDetachKeyword(messageText: string): boolean {
  return STANDALONE_DETACH_KEYWORD_PATTERN.test(messageText);
}

function stripStandaloneDetachKeywords(messageText: string): string {
  return messageText.replace(STANDALONE_DETACH_KEYWORDS_PATTERN, "$1");
}

/** Strip the `&detach` control token before Pi's input transform delivers the user message to the model. */
export function stripUserMessageDetachKeyword(messageText: string): string {
  if (!containsStandaloneDetachKeyword(messageText)) return messageText;
  const stripped = stripStandaloneDetachKeywords(messageText).replace(/[ \t]{2,}/g, " ");
  return stripped.trim() === "" ? EMPTY_DETACH_MESSAGE : stripped;
}

/** Decide whether a user message should signal an active wait:true bash call. */
export function shouldDetachBashWaitOnUserMessage(config: AftConfig, messageText: string): boolean {
  return (
    resolveBashConfig(config).detach_on_user_message || containsStandaloneDetachKeyword(messageText)
  );
}

async function sendBashWaitDetach(bridge: AftProjectTransport, sessionID: string): Promise<void> {
  const response = await bridge.send(
    "bash_wait_detach",
    { session_id: sessionID },
    { keepBridgeOnTimeout: true, transportTimeoutMs: BASH_TRANSPORT_TIMEOUT_MS },
  );
  if (response.success === false) {
    throw new Error(String(response.message ?? "bash_wait_detach failed"));
  }
}

export async function signalBashWaitDetachForProject(
  pool: ActiveBridgePool,
  projectRoot: string,
  sessionID: string | undefined,
): Promise<void> {
  if (!sessionID) return;
  const bridge = pool.getActiveBridgeForRoot(projectRoot);
  if (!bridge) return;
  try {
    await sendBashWaitDetach(bridge, sessionID);
  } catch (err) {
    warn(
      `[bash_wait_detach] failed for session ${sessionID}: ${err instanceof Error ? err.message : String(err)}`,
    );
  }
}
