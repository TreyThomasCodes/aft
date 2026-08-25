import { spawn } from "node:child_process";

/**
 * Sign and execute a binary before tests spawn it repeatedly on macOS.
 *
 * Gatekeeper assesses freshly written provenance-bearing Mach-O binaries through
 * XProtect and can present verification UI. A warm execution pays that one
 * assessment before a timed test needs the binary. A stable ad-hoc identifier
 * makes logs recognizable, but does not exempt the binary: an identical signed
 * binary at a fresh inode is assessed again.
 *
 * This is best-effort latency mitigation, not a correctness gate. It is a no-op
 * off Darwin and deliberately ignores signing or warm-exec failures.
 */
export async function warmMacosExec(binaryPath: string): Promise<void> {
  if (process.platform !== "darwin") return;

  await runQuietly("codesign", ["-f", "-s", "-", "--identifier", "aft-dev-gate", binaryPath]);
  await runQuietly(binaryPath, ["--version"]);
}

function runQuietly(command: string, args: string[]): Promise<void> {
  return new Promise((resolve) => {
    let settled = false;
    const done = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve();
    };

    // A wedged security daemon can stall these calls indefinitely. Cap the wait:
    // an unwarmed binary is slower than we would like, never broken, so waiting
    // longer than the delay we are trying to avoid defeats the purpose.
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      done();
    }, 30_000);

    const child = spawn(command, args, { stdio: "ignore" });
    child.once("error", done);
    child.once("exit", done);
  });
}
