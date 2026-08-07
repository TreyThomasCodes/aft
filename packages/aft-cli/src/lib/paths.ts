import { homedir } from "node:os";
import { join } from "node:path";
import {
  getAftBinaryCacheDir,
  getAftLspBinariesDir,
  getAftLspPackagesDir,
} from "@cortexkit/aft-bridge";

export { getAftBinaryCacheDir, getAftLspBinariesDir, getAftLspPackagesDir };

export function getAftBinaryName(): string {
  return process.platform === "win32" ? "aft.exe" : "aft";
}

function homeDir(): string {
  if (process.platform === "win32") return process.env.USERPROFILE || process.env.HOME || homedir();
  return process.env.HOME || homedir();
}

function dataHome(): string {
  if (process.env.XDG_DATA_HOME) return process.env.XDG_DATA_HOME;
  if (process.platform === "win32") {
    return process.env.LOCALAPPDATA || process.env.APPDATA || join(homeDir(), "AppData", "Local");
  }
  return join(homeDir(), ".local", "share");
}

export function getCortexKitStorageRoot(): string {
  if (process.env.AFT_CACHE_DIR) return join(process.env.AFT_CACHE_DIR, "aft");
  return join(dataHome(), "cortexkit", "aft");
}
