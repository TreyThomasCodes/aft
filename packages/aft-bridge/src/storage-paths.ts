import { homedir } from "node:os";
import { join, resolve } from "node:path";

function homeDir(): string {
  if (process.platform === "win32") return process.env.USERPROFILE || process.env.HOME || homedir();
  return process.env.HOME || homedir();
}

function dataHome(): string {
  const xdg = process.env.XDG_DATA_HOME;
  if (xdg) return xdg;
  if (process.platform === "win32") {
    return process.env.LOCALAPPDATA || process.env.APPDATA || join(homeDir(), "AppData", "Local");
  }
  return join(homeDir(), ".local", "share");
}

/**
 * Expand the supported storage-root spellings and anchor them to this process's
 * cwd. Every caller receives one absolute spelling, so a bridge and a plugin
 * cannot select different roots merely because their cwd differs.
 */
export function resolveStoragePath(raw: string): string {
  let expanded = raw;
  if (raw === "~") {
    expanded = homeDir();
  } else if (raw.startsWith("~/") || raw.startsWith("~\\")) {
    expanded = join(homeDir(), raw.slice(2));
  }
  return resolve(expanded);
}

/** Resolve the shared CortexKit storage root used by every plugin host. */
export function resolveCortexKitStorageRoot(): string {
  const override = process.env.AFT_STORAGE_DIR;
  if (override) return resolveStoragePath(override);
  return resolveStoragePath(join(dataHome(), "cortexkit", "aft"));
}

/**
 * Resolve a process-state storage root. AFT_STORAGE_DIR is checked here rather
 * than at injection time so it wins over a stale or plugin-injected wire value.
 */
export function resolveAftStorageRoot(configuredRoot?: string): string {
  if (process.env.AFT_STORAGE_DIR) return resolveCortexKitStorageRoot();
  if (configuredRoot) return resolveStoragePath(configuredRoot);
  return resolveCortexKitStorageRoot();
}

export function resolveAftLogPath(filename: string, configuredRoot?: string): string {
  return join(resolveAftStorageRoot(configuredRoot), "logs", filename);
}
