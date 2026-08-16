import { spawn } from "node:child_process";

export const BASH_HOST_FALLBACK_BANNER =
  "[AFT host fallback - module transport down; no rewrites/compression/background]";
export const BASH_HOST_FALLBACK_MAX_OUTPUT_BYTES = 100 * 1024;
export const BASH_HOST_FALLBACK_MAX_TIMEOUT_MS = 10 * 60 * 1000;
export const BASH_HOST_FALLBACK_REFUSAL =
  "AFT transport is down; only foreground execution is available in host fallback";

export interface BashHostFallbackOptions {
  command: string;
  projectRoot: string;
  timeoutMs?: number;
  signal?: AbortSignal;
  env?: NodeJS.ProcessEnv;
}

export interface BashHostFallbackResult extends Record<string, unknown> {
  success: true;
  output: string;
  exit_code: number;
  truncated: boolean;
}

export function bashHostFallbackAskPattern(command: string, cwd: string): string {
  return `AFT UNAVAILABLE - host fallback execution:\n\nExact command:\n${command}\n\nWorking directory:\n${cwd}`;
}

function appendTail(chunks: Buffer[], chunk: Buffer): { chunks: Buffer[]; truncated: boolean } {
  const combined = Buffer.concat([...chunks, chunk]);
  if (combined.byteLength <= BASH_HOST_FALLBACK_MAX_OUTPUT_BYTES) {
    return { chunks: [combined], truncated: false };
  }
  return {
    chunks: [combined.subarray(combined.byteLength - BASH_HOST_FALLBACK_MAX_OUTPUT_BYTES)],
    truncated: true,
  };
}

function renderOutput(output: Buffer, exitCode: number): string {
  const body = output.toString("utf8");
  const separator = body.length === 0 || body.endsWith("\n") ? "" : "\n";
  return `${BASH_HOST_FALLBACK_BANNER}\n${body}${separator}[exit code: ${exitCode}]`;
}

/** Execute one explicitly-approved shell command without any AFT processing. */
export async function runBashHostFallback(
  options: BashHostFallbackOptions,
): Promise<BashHostFallbackResult> {
  const timeoutMs = Math.min(
    Math.max(1, options.timeoutMs ?? BASH_HOST_FALLBACK_MAX_TIMEOUT_MS),
    BASH_HOST_FALLBACK_MAX_TIMEOUT_MS,
  );

  if (options.signal?.aborted) {
    throw new DOMException("The host fallback command was aborted", "AbortError");
  }

  return await new Promise<BashHostFallbackResult>((resolve, reject) => {
    const child = spawn(options.command, {
      cwd: options.projectRoot,
      shell: true,
      env: { ...process.env, ...options.env },
      stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
      windowsHide: true,
    });

    let chunks: Buffer[] = [];
    let truncated = false;
    let timedOut = false;
    let aborted = false;
    let settled = false;
    let abortForceTimer: ReturnType<typeof setTimeout> | undefined;

    const capture = (chunk: Buffer | string) => {
      const next = appendTail(chunks, Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
      chunks = next.chunks;
      truncated ||= next.truncated;
    };
    child.stdout?.on("data", capture);
    child.stderr?.on("data", capture);

    const kill = (signal: NodeJS.Signals) => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      if (process.platform !== "win32" && child.pid !== undefined) {
        try {
          process.kill(-child.pid, signal);
          return;
        } catch {
          // The process may have exited between the liveness check and group kill.
        }
      }
      child.kill(signal);
    };
    const onAbort = () => {
      aborted = true;
      kill("SIGTERM");
      abortForceTimer = setTimeout(() => kill("SIGKILL"), 250);
      abortForceTimer.unref?.();
    };
    options.signal?.addEventListener("abort", onAbort, { once: true });

    const timer = setTimeout(() => {
      timedOut = true;
      kill("SIGKILL");
    }, timeoutMs);

    const cleanup = () => {
      clearTimeout(timer);
      if (abortForceTimer !== undefined) clearTimeout(abortForceTimer);
      options.signal?.removeEventListener("abort", onAbort);
    };

    child.once("error", (error) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    });
    child.once("close", (code) => {
      if (settled) return;
      settled = true;
      cleanup();
      if (aborted) {
        reject(new DOMException("The host fallback command was aborted", "AbortError"));
        return;
      }
      const exitCode = timedOut ? 124 : (code ?? 1);
      resolve({
        success: true,
        output: renderOutput(Buffer.concat(chunks), exitCode),
        exit_code: exitCode,
        truncated,
      });
    });
  });
}
