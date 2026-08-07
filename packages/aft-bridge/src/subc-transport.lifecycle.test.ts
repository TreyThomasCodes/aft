/// <reference path="./bun-test.d.ts" />

import { describe, expect, test } from "bun:test";
import type {
  BindIdentity,
  RequestOptions,
  RouteHandle,
  RouteTarget,
} from "@cortexkit/subc-client";
import {
  asCanonicalRootPath,
  LifecycleRegistry,
  type LifecycleTimerSeam,
} from "./lifecycle-registry.js";
import {
  type SubcClientLike,
  SubcRootGenerationExpiredError,
  SubcRootReapedError,
  SubcTransportPool,
} from "./subc-transport.js";

class Timer implements LifecycleTimerSeam {
  readonly callbacks: Array<() => void> = [];
  setInterval(callback: () => void): unknown {
    this.callbacks.push(callback);
    return callback;
  }
  clearInterval(_handle: unknown): void {}
}

class Client implements SubcClientLike {
  readonly routeOpens: BindIdentity[] = [];
  readonly closedRoutes: number[] = [];
  closed = 0;
  private channel = 1;
  requestGate: Promise<void> | null = null;
  releaseRequest!: () => void;

  async routeOpen(_target: RouteTarget, identity: BindIdentity): Promise<RouteHandle> {
    this.routeOpens.push(identity);
    const channel = this.channel++;
    return { channel, epoch: channel } as RouteHandle;
  }
  async request(_route: RouteHandle, _body: unknown, _options?: RequestOptions): Promise<unknown> {
    if (this.requestGate) await this.requestGate;
    return { structuredContent: { success: true, text: "ok" } };
  }
  subscribe(): never {
    throw new Error("background subscriptions are not used by this fixture");
  }
  async closeRouteChannel(route: RouteHandle): Promise<void> {
    this.closedRoutes.push(route.channel);
  }
  close(): void {
    this.closed += 1;
  }
}

function missing(): Error & { code: string } {
  return Object.assign(new Error("missing"), { code: "ENOENT" });
}

function setup(present: () => boolean): {
  pool: SubcTransportPool;
  registry: LifecycleRegistry;
  client: Client;
  root: ReturnType<typeof asCanonicalRootPath>;
} {
  const timer = new Timer();
  const registry = new LifecycleRegistry({
    timer,
    demandCheck: () => present(),
    stat: async () => {
      if (present()) return true;
      throw missing();
    },
  });
  const client = new Client();
  const pool = new SubcTransportPool({
    connectionFile: "/tmp/fake",
    harness: "opencode",
    connect: async () => client,
    lifecycleRegistry: registry,
    lifecycleDemandCheck: () => present(),
    reapingEnabled: true,
  });
  return { pool, registry, client, root: asCanonicalRootPath("/work/reaped") };
}

describe("SubcTransportPool lifecycle integration", () => {
  test("indexes records, performs one coordinated two-sweep teardown, and creates a successor", async () => {
    let present = true;
    const rig = setup(() => present);
    const bridge = rig.pool.getBridge(rig.root);
    await bridge.toolCall("session", "read", {});

    const internals = rig.pool as unknown as {
      rootIndex: Map<string, Set<string>>;
      sessions: Map<string, unknown>;
    };
    expect(internals.rootIndex.get(rig.root)?.size).toBe(1);
    expect(internals.sessions.size).toBe(1);

    present = false;
    await rig.registry.sweep();
    expect(rig.client.closedRoutes).toEqual([]);
    await rig.registry.sweep();

    expect(rig.client.closedRoutes).toEqual([1]);
    expect(internals.rootIndex.size).toBe(0);
    expect(internals.sessions.size).toBe(0);
    expect(rig.pool.getActiveBridgeForRoot(rig.root)).toBeNull();
    expect(rig.registry.snapshot().registrations[0]?.roots).toHaveLength(0);

    present = true;
    const successor = rig.pool.getBridge(rig.root);
    expect(successor.getGeneration()).toBeGreaterThan(bridge.getGeneration()!);
    await successor.toolCall("session", "read", {});
    await expect(bridge.toolCall("session", "read", {})).rejects.toBeInstanceOf(
      SubcRootGenerationExpiredError,
    );
    await rig.pool.shutdown();
  });

  test("disabled registration preserves legacy synthetic-root behavior without stat calls", async () => {
    let stats = 0;
    const registry = new LifecycleRegistry({
      stat: async () => {
        stats += 1;
        return true;
      },
    });
    const client = new Client();
    const pool = new SubcTransportPool({
      connectionFile: "/tmp/fake",
      harness: "opencode",
      connect: async () => client,
      lifecycleRegistry: registry,
      reapingEnabled: false,
    });

    await pool.getBridge("/synthetic/root").toolCall("session", "read", {});
    await registry.sweep();
    expect(stats).toBe(0);
    expect(pool.getLifecycleRegistration()?.reapingEnabled).toBe(false);
    await pool.shutdown();
  });

  test("annotates an in-flight reap failure without charging or dropping the client", async () => {
    const present = true;
    const rig = setup(() => present);
    const root = asCanonicalRootPath("/work/inflight");
    const bridge = rig.pool.getBridge(root);
    const gate = new Promise<void>((resolve) => {
      rig.client.releaseRequest = resolve;
    });
    rig.client.requestGate = gate;
    const call = bridge.toolCall("session", "read", {});
    await Promise.resolve();
    await Promise.resolve();
    const generation = bridge.getGeneration()!;
    await rig.registry.requestProjectRootClose(
      rig.pool.getConcretePoolId()!,
      root,
      generation,
      "explicit",
    );
    rig.client.requestGate = null;
    rig.client.releaseRequest();

    const error = await call.catch((value) => value);
    expect(error).toSatisfy(
      (value: unknown) =>
        value instanceof SubcRootReapedError ||
        (value instanceof Error &&
          (value as Error & { subcTeardownReason?: string }).subcTeardownReason === "root_reaped"),
    );
    expect(rig.client.closed).toBe(0);
    await rig.pool.shutdown();
  });
});
