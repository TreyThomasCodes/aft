/// <reference path="../bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import {
  getAftBinaryCacheDir,
  getAftCacheRoot,
  getAftLspBinariesDir,
  getAftLspPackagesDir,
} from "../cache-paths.js";
import { getCacheDir } from "../downloader.js";

function withPlatform<T>(platform: NodeJS.Platform, fn: () => T): T {
  const descriptor = Object.getOwnPropertyDescriptor(process, "platform");
  Object.defineProperty(process, "platform", { configurable: true, value: platform });
  try {
    return fn();
  } finally {
    if (descriptor) Object.defineProperty(process, "platform", descriptor);
  }
}

describe("shared AFT cache paths", () => {
  test("uses one controlled root for the binary and both LSP subdirectories", () => {
    const env = {
      AFT_CACHE_DIR: "/tmp/aft-cache-override",
      XDG_CACHE_HOME: "/tmp/xdg-cache",
      HOME: "/tmp/home",
    };
    const root = getAftCacheRoot(env);

    expect(root).toBe(env.AFT_CACHE_DIR);
    expect(getAftBinaryCacheDir(env)).toBe(join(root, "bin"));
    expect(getAftLspPackagesDir(env)).toBe(join(root, "lsp-packages"));
    expect(getAftLspBinariesDir(env)).toBe(join(root, "lsp-binaries"));
  });

  test("uses XDG_CACHE_HOME on POSIX and LOCALAPPDATA on Windows", () => {
    const posixEnv = { XDG_CACHE_HOME: "/tmp/xdg-cache", HOME: "/tmp/home" };
    expect(getAftCacheRoot(posixEnv)).toBe(join(posixEnv.XDG_CACHE_HOME, "aft"));

    const windowsEnv = {
      LOCALAPPDATA: "/tmp/local-app-data",
      APPDATA: "/tmp/app-data",
      USERPROFILE: "/tmp/profile",
      HOME: "/tmp/home",
    };
    withPlatform("win32", () => {
      expect(getAftCacheRoot(windowsEnv)).toBe(join(windowsEnv.LOCALAPPDATA, "aft"));
      expect(getAftBinaryCacheDir(windowsEnv)).toBe(join(windowsEnv.LOCALAPPDATA, "aft", "bin"));

      const appDataEnv = { APPDATA: windowsEnv.APPDATA, USERPROFILE: windowsEnv.USERPROFILE };
      expect(getAftCacheRoot(appDataEnv)).toBe(join(windowsEnv.APPDATA, "aft"));

      const profileEnv = { USERPROFILE: windowsEnv.USERPROFILE };
      expect(getAftCacheRoot(profileEnv)).toBe(
        join(windowsEnv.USERPROFILE, "AppData", "Local", "aft"),
      );
    });
  });

  test("downloader compatibility export delegates to the shared binary path", () => {
    const env = { AFT_CACHE_DIR: "/tmp/aft-cache-override" };
    expect(getCacheDir(env)).toBe(getAftBinaryCacheDir(env));
  });
});
