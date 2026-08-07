/// <reference path="./bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import {
  asCanonicalRootPath,
  type CanonicalRootPath,
  type LifecyclePool,
  LifecycleRegistry,
  type LifecycleTimerSeam,
  type RootGeneration,
} from "./lifecycle-registry.js";

function root(value: string): CanonicalRootPath {
  return asCanonicalRootPath(`/tmp/lifecycle-registry/${value}`);
}

function missing(): Error & { code: string } {
  return Object.assign(new Error("root is absent"), { code: "ENOENT" });
}

function errorWithCode(code: string): Error & { code: string } {
  return Object.assign(new Error(code), { code });
}

class FakeTimer implements LifecycleTimerSeam {
  readonly created: Array<{ callback: () => void; delayMs: number }> = [];
  cleared = 0;

  setInterval(callback: () => void, delayMs: number): unknown {
    const handle = { callback, delayMs };
    this.created.push(handle);
    return handle;
  }

  clearInterval(_handle: unknown): void {
    this.cleared += 1;
  }

  tick(): void {
    for (const timer of [...this.created]) timer.callback();
  }
}

class FakePool implements LifecyclePool {
  readonly closes: Array<{ root: CanonicalRootPath; generation: RootGeneration }> = [];
  readonly evictions: Array<{ root: CanonicalRootPath; generation: RootGeneration }> = [];
  closeGate: Promise<void> | null = null;
  closeResult = { tornDownSessionCount: 1, tornDownFacadeCount: 2 };

  async closeProjectRoot(rootPath: CanonicalRootPath, generation: RootGeneration) {
    this.closes.push({ root: rootPath, generation });
    if (this.closeGate) await this.closeGate;
    return this.closeResult;
  }
}

describe("LifecycleRegistry", () => {
  test("uses one realm timer, excludes disabled pools, and deregisters exact handles", () => {
    const timer = new FakeTimer();
    const stats: CanonicalRootPath[] = [];
    const registry = new LifecycleRegistry({
      timer,
      stat: async (rootPath) => {
        stats.push(rootPath);
        return true;
      },
    });
    const disabledPool = new FakePool();
    const enabledPool = new FakePool();
    const disabled = registry.registerLifecyclePool(disabledPool, {
      reapingEnabled: false,
      evictOuterFacade: () => undefined,
    });
    const enabled = registry.registerLifecyclePool(enabledPool, {
      reapingEnabled: true,
      evictOuterFacade: () => undefined,
    });
    const disabledRoot = root("disabled");
    const enabledRoot = root("enabled");
    registry.registerRoot(disabled.concretePoolId, disabledRoot);
    registry.registerRoot(enabled.concretePoolId, enabledRoot);

    expect(timer.created).toHaveLength(1);
    timer.tick();
    expect(stats).toEqual([enabledRoot]);

    enabled.deregister();
    expect(timer.cleared).toBe(1);
    expect(registry.snapshot().registrations).toHaveLength(1);
    expect(registry.snapshot().registrations[0]?.concretePoolId).toBe(disabled.concretePoolId);
    disabled.deregister();
    expect(registry.snapshot().registrations).toHaveLength(0);
  });

  test("keeps registration switches immutable and prevents a stale handle from removing a successor", () => {
    const timer = new FakeTimer();
    const registry = new LifecycleRegistry({ timer });
    const pool = new FakePool();
    const first = registry.registerLifecyclePool(pool, {
      reapingEnabled: true,
      evictOuterFacade: () => undefined,
    });
    const firstId = first.concretePoolId;
    const second = registry.registerLifecyclePool(pool, {
      reapingEnabled: false,
      evictOuterFacade: () => undefined,
    });

    expect(second.concretePoolId).not.toBe(firstId);
    expect(first.reapingEnabled).toBe(true);
    expect(second.reapingEnabled).toBe(false);
    expect(timer.cleared).toBe(1);
    first.deregister();
    expect(registry.snapshot().registrations.map((entry) => entry.concretePoolId)).toEqual([
      second.concretePoolId,
    ]);
  });

  test("counts only consecutive ENOENT observations and emits one reap event", async () => {
    const timer = new FakeTimer();
    const observations: Array<true | Error> = [
      missing(),
      errorWithCode("EACCES"),
      errorWithCode("EIO"),
      true,
      missing(),
      missing(),
    ];
    const events: unknown[] = [];
    const pool = new FakePool();
    const registry = new LifecycleRegistry({
      timer,
      stat: async () => {
        const observation = observations.shift();
        if (observation instanceof Error) throw observation;
        return observation ?? true;
      },
      onEvent: (event) => events.push(event),
    });
    const registration = registry.registerLifecyclePool(pool, {
      reapingEnabled: true,
      evictOuterFacade: (rootPath, generation) =>
        pool.evictions.push({ root: rootPath, generation }),
    });
    const canonicalRoot = root("enoent");
    const generation = registry.registerRoot(registration.concretePoolId, canonicalRoot);

    await registry.sweep();
    expect(
      registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)?.consecutiveAbsences,
    ).toBe(1);
    await registry.sweep();
    expect(
      registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)?.consecutiveAbsences,
    ).toBe(1);
    await registry.sweep();
    expect(
      registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)?.consecutiveAbsences,
    ).toBe(1);
    await registry.sweep();
    expect(
      registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)?.consecutiveAbsences,
    ).toBe(0);
    await registry.sweep();
    expect(
      registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)?.consecutiveAbsences,
    ).toBe(1);
    await registry.sweep();

    expect(pool.evictions).toEqual([{ root: canonicalRoot, generation }]);
    expect(pool.closes).toEqual([{ root: canonicalRoot, generation }]);
    expect(registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)).toBeUndefined();
    expect(events).toEqual([
      {
        type: "subc_root_reaped",
        realm: "default",
        concretePoolId: registration.concretePoolId,
        canonicalRoot,
        generation,
        consecutiveAbsenceCount: 2,
        tornDownSessionCount: 1,
        tornDownFacadeCount: 2,
        cause: "sweep",
      },
    ]);
  });

  test("coalesces concurrent sweeps and does not let a stale stat touch a successor", async () => {
    let releaseStat!: () => void;
    const statGate = new Promise<void>((resolve) => {
      releaseStat = resolve;
    });
    const pool = new FakePool();
    const registry = new LifecycleRegistry({
      demandCheck: async () => true,
      stat: async () => {
        await statGate;
        throw missing();
      },
    });
    const registration = registry.registerLifecyclePool(pool, {
      reapingEnabled: true,
      evictOuterFacade: () => undefined,
    });
    const canonicalRoot = root("stale");
    const firstGeneration = registry.registerRoot(registration.concretePoolId, canonicalRoot);
    const firstSweep = registry.sweep();
    expect(registry.sweep()).toBe(firstSweep);

    const close = registry.requestProjectRootClose(
      registration.concretePoolId,
      canonicalRoot,
      firstGeneration,
      "explicit",
    );
    expect(registry.isTombstoned(registration.concretePoolId, canonicalRoot, firstGeneration)).toBe(
      true,
    );
    const successor = await registry.ensureRootForDemand(
      registration.concretePoolId,
      canonicalRoot,
    );
    expect(successor).toBe(firstGeneration + 1);
    releaseStat();
    await close;
    await firstSweep;

    const secondGeneration = registry.currentGeneration(registration.concretePoolId, canonicalRoot);
    expect(secondGeneration).toBe(firstGeneration + 1);
    expect(registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)).toMatchObject({
      generation: secondGeneration,
      state: "live",
      consecutiveAbsences: 0,
    });
  });

  test("requires positive demand for initial and successor generations without repeating live checks", async () => {
    let present = false;
    let checks = 0;
    const pool = new FakePool();
    const registry = new LifecycleRegistry({
      demandCheck: async () => {
        checks += 1;
        return present;
      },
    });
    const registration = registry.registerLifecyclePool(pool, {
      reapingEnabled: false,
      evictOuterFacade: () => undefined,
    });
    const canonicalRoot = root("demand");

    expect(
      await registry.ensureRootForDemand(registration.concretePoolId, canonicalRoot),
    ).toBeUndefined();
    expect(registry.snapshot().registrations[0]?.roots).toHaveLength(0);
    present = true;
    const first = await registry.ensureRootForDemand(registration.concretePoolId, canonicalRoot);
    expect(first).toBeDefined();
    expect(await registry.ensureRootForDemand(registration.concretePoolId, canonicalRoot)).toBe(
      first,
    );
    expect(checks).toBe(2);

    await registry.requestProjectRootClose(
      registration.concretePoolId,
      canonicalRoot,
      first!,
      "explicit",
    );
    const successor = await registry.ensureRootForDemand(
      registration.concretePoolId,
      canonicalRoot,
    );
    expect(successor).toBe(first! + 1);
    expect(checks).toBe(3);
  });

  test("performs the transition before returning and coalesces repeated close requests", async () => {
    let releaseClose!: () => void;
    const closeGate = new Promise<void>((resolve) => {
      releaseClose = resolve;
    });
    const pool = new FakePool();
    pool.closeGate = closeGate;
    const order: string[] = [];
    const registry = new LifecycleRegistry();
    const registration = registry.registerLifecyclePool(pool, {
      reapingEnabled: true,
      evictOuterFacade: () => order.push("outer"),
    });
    const canonicalRoot = root("close");
    const generation = registry.registerRoot(registration.concretePoolId, canonicalRoot);
    const first = registry.requestProjectRootClose(
      registration.concretePoolId,
      canonicalRoot,
      generation,
      "sweep",
    );
    order.push("request-returned");
    const second = registry.requestProjectRootClose(
      registration.concretePoolId,
      canonicalRoot,
      generation,
      "explicit",
    );

    expect(first).toBe(second);
    expect(order).toEqual(["outer", "request-returned"]);
    expect(pool.closes).toHaveLength(1);
    expect(registry.isTombstoned(registration.concretePoolId, canonicalRoot, generation)).toBe(
      true,
    );
    releaseClose();
    await first;
    expect(registry.getRootSnapshot(registration.concretePoolId, canonicalRoot)).toBeUndefined();
  });
});
