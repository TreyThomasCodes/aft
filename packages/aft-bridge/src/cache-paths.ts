import { homedir } from "node:os";
import { join } from "node:path";

interface CacheEnvironment {
  AFT_CACHE_DIR?: string;
  XDG_CACHE_HOME?: string;
  LOCALAPPDATA?: string;
  APPDATA?: string;
  USERPROFILE?: string;
  HOME?: string;
}

function homeDir(env: CacheEnvironment): string {
  return (process.platform === "win32" ? env.USERPROFILE || env.HOME : env.HOME) || homedir();
}

/**
 * Resolve the root shared by AFT's downloadable binaries and LSP artifacts.
 *
 * `AFT_CACHE_DIR` is an explicit root override. Otherwise Windows follows
 * LOCALAPPDATA, then APPDATA, while POSIX platforms use XDG_CACHE_HOME or
 * `~/.cache`. macOS intentionally follows the POSIX `~/.cache` layout: that is
 * where existing AFT artifacts live, so switching to `~/Library/Caches` would
 * strand the cache rather than consolidate it.
 */
export function getAftCacheRoot(env: CacheEnvironment = process.env): string {
  if (env.AFT_CACHE_DIR) return env.AFT_CACHE_DIR;

  if (process.platform === "win32") {
    const base = env.LOCALAPPDATA || env.APPDATA || join(homeDir(env), "AppData", "Local");
    return join(base, "aft");
  }

  const base = env.XDG_CACHE_HOME || join(homeDir(env), ".cache");
  return join(base, "aft");
}

/** Directory holding versioned AFT binaries. */
export function getAftBinaryCacheDir(env: CacheEnvironment = process.env): string {
  return join(getAftCacheRoot(env), "bin");
}

/** Directory holding npm-installed LSP packages. */
export function getAftLspPackagesDir(env: CacheEnvironment = process.env): string {
  return join(getAftCacheRoot(env), "lsp-packages");
}

/** Directory holding GitHub-installed LSP binaries. */
export function getAftLspBinariesDir(env: CacheEnvironment = process.env): string {
  return join(getAftCacheRoot(env), "lsp-binaries");
}
