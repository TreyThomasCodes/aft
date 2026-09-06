export interface EffectAbortForwarder {
  /** Whether this invocation has already forwarded an interruption. */
  readonly forwarded: boolean;
  /** Stop accepting interruption without triggering cancellation. */
  dispose(): void;
}

/**
 * Forward one Effect-owned AbortSignal without retaining process-global state.
 * Disposal wins any later interruption, while an already-started best-effort
 * send is observed so transport teardown cannot create an unhandled rejection.
 */
export function forwardEffectAbort(
  signal: AbortSignal,
  send: () => unknown | PromiseLike<unknown>,
): EffectAbortForwarder {
  let disposed = false;
  let forwarded = false;

  const onAbort = (): void => {
    if (disposed || forwarded) return;
    forwarded = true;
    try {
      void Promise.resolve(send()).catch(() => undefined);
    } catch {
      // A synchronous transport teardown is equivalent to a rejected abort send.
    }
  };

  signal.addEventListener("abort", onAbort, { once: true });
  if (signal.aborted) onAbort();

  return {
    get forwarded() {
      return forwarded;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      signal.removeEventListener("abort", onAbort);
    },
  };
}
