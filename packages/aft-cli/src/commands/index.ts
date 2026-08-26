import { spawnSync } from "node:child_process";
import { findAftBinary } from "../lib/binary-probe.js";

/**
 * Run the native finite snapshot command. The npm CLI deliberately forwards no
 * root or index-selection behavior of its own; the Rust command validates that
 * `aft index` remains bare and schedules no future work.
 */
export function runIndex(argv: string[]): number {
  const binary = findAftBinary();
  if (!binary) {
    console.error(
      "aft index requires a native AFT binary; run `aft doctor` to install or repair it.",
    );
    return 1;
  }

  const result = spawnSync(binary, ["index", ...argv], {
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) {
    console.error(`aft index failed to start ${binary}: ${result.error.message}`);
    return 1;
  }
  return result.status ?? 1;
}
