import type { ToolResult } from "@opencode-ai/plugin";

import { forwardEffectAbort } from "../../cancellation/effect-abort.js";
import type { V2BashExecution } from "../definitions/v2.js";

/**
 * Run the shared bash definition with a per-call signal derived from its Effect
 * fiber. The shared definition maps that signal to `bash_abort_inflight` only
 * while a foreground invocation is active; explicit background and PTY calls
 * therefore remain untouched.
 */
export async function executeV2Bash(execution: V2BashExecution): Promise<ToolResult> {
  const downstream = new AbortController();
  const forwarder = forwardEffectAbort(execution.context.abort, () => {
    downstream.abort(execution.context.abort.reason);
  });

  try {
    return await execution.definition.execute(execution.input, {
      ...execution.context,
      abort: downstream.signal,
    } as never);
  } finally {
    forwarder.dispose();
  }
}
