import { lstat } from "node:fs/promises";

/** A canonical project root. The registry never derives identity by parsing it. */
declare const canonicalRootPathBrand: unique symbol;
export type CanonicalRootPath = string & {
  readonly [canonicalRootPathBrand]: "CanonicalRootPath";
};

/** A pool-lifetime root generation. Generations are strictly increasing per root. */
declare const rootGenerationBrand: unique symbol;
export type RootGeneration = number & {
  readonly [rootGenerationBrand]: "RootGeneration";
};

/** A realm-local concrete pool identity. IDs are never reused by a registry. */
declare const concretePoolIdBrand: unique symbol;
export type ConcretePoolId = string & {
  readonly [concretePoolIdBrand]: "ConcretePoolId";
};

/**
 * Immutable provenance for one registration handle. The object identity is the
 * deregistration authority; the serial fields make the provenance inspectable
 * without treating a pool object as a registration token.
 */
export interface RegistrationIdentity {
  readonly concretePoolId: ConcretePoolId;
  readonly registrationSequence: number;
}

export function asCanonicalRootPath(root: string): CanonicalRootPath {
  return root as CanonicalRootPath;
}

export function asRootGeneration(generation: number): RootGeneration {
  if (!Number.isSafeInteger(generation) || generation < 1) {
    throw new RangeError("root generations must be positive safe integers");
  }
  return generation as RootGeneration;
}

export function asConcretePoolId(id: string): ConcretePoolId {
  return id as ConcretePoolId;
}

export type LifecycleCloseCause = "sweep" | "explicit";
export type LifecycleRootState = "live" | "reaping" | "tombstoned";

/** Counts returned by a concrete pool after it has detached its indexed records. */
export interface LifecycleTeardownResult {
  readonly tornDownSessionCount?: number;
  readonly tornDownFacadeCount?: number;
}

/** The concrete-pool portion of the lifecycle contract. */
export interface LifecyclePool {
  closeProjectRoot(root: CanonicalRootPath, generation: RootGeneration): unknown;
}

export interface LifecyclePoolRegistrationOptions {
  readonly reapingEnabled: boolean;
  readonly evictOuterFacade: (root: CanonicalRootPath, generation: RootGeneration) => void;
}

export interface LifecyclePoolRegistration {
  readonly concretePoolId: ConcretePoolId;
  readonly reapingEnabled: boolean;
  readonly registrationIdentity: RegistrationIdentity;
  /** Short alias for callers that refer to the provenance token as identity. */
  readonly identity: RegistrationIdentity;
  deregister(): void;
}

export type LifecycleStatResult = boolean | { readonly exists?: boolean } | unknown;
export type LifecycleStat = (
  root: CanonicalRootPath,
  concretePoolId: ConcretePoolId,
  generation: RootGeneration,
) => LifecycleStatResult | Promise<LifecycleStatResult>;
export type LifecycleDemandCheck = (
  root: CanonicalRootPath,
  concretePoolId: ConcretePoolId,
) => boolean | { readonly exists?: boolean } | Promise<boolean | { readonly exists?: boolean }>;

export interface LifecycleTimerSeam {
  setInterval(callback: () => void, delayMs: number): unknown;
  clearInterval(handle: unknown): void;
}

export interface RootReapedLifecycleEvent {
  readonly type: "subc_root_reaped";
  readonly realm: string;
  readonly concretePoolId: ConcretePoolId;
  readonly canonicalRoot: CanonicalRootPath;
  readonly generation: RootGeneration;
  readonly consecutiveAbsenceCount: number;
  readonly tornDownSessionCount: number;
  readonly tornDownFacadeCount: number;
  readonly cause: LifecycleCloseCause;
}

export interface RootGenerationRejectedLifecycleEvent {
  readonly type: "subc_root_generation_rejected";
  readonly realm: string;
  readonly concretePoolId: ConcretePoolId;
  readonly canonicalRoot: CanonicalRootPath;
  readonly expectedGeneration: RootGeneration;
  readonly currentGeneration?: RootGeneration;
  readonly boundary: string;
}

export type LifecycleEvent = RootReapedLifecycleEvent | RootGenerationRejectedLifecycleEvent;

export interface LifecycleRegistryOptions {
  /** Label attached to realm-local metrics and lifecycle events. */
  readonly realm?: string;
  /** Production cadence. Tests normally replace the timer and use a short value. */
  readonly intervalMs?: number;
  readonly timer?: LifecycleTimerSeam;
  readonly setInterval?: LifecycleTimerSeam["setInterval"];
  readonly clearInterval?: LifecycleTimerSeam["clearInterval"];
  /** A stat that resolves for an existing root and rejects with ENOENT when absent. */
  readonly stat?: LifecycleStat;
  /** Positive demand is the only signal that permits an initial or successor root. */
  readonly demandCheck?: LifecycleDemandCheck;
  readonly onEvent?: (event: LifecycleEvent) => void;
}

export interface LifecycleRootSnapshot {
  readonly concretePoolId: ConcretePoolId;
  readonly registrationIdentity: RegistrationIdentity;
  readonly canonicalRoot: CanonicalRootPath;
  readonly generation: RootGeneration;
  readonly state: LifecycleRootState;
  readonly consecutiveAbsences: number;
}

export interface LifecycleRegistrationSnapshot {
  readonly concretePoolId: ConcretePoolId;
  readonly reapingEnabled: boolean;
  readonly roots: readonly LifecycleRootSnapshot[];
}

export interface LifecycleRegistrySnapshot {
  readonly realm: string;
  readonly intervalMs: number;
  readonly timerActive: boolean;
  readonly registrations: readonly LifecycleRegistrationSnapshot[];
}

export const DEFAULT_LIFECYCLE_INTERVAL_MS = 5 * 60 * 1000;
const DEFAULT_REALM = "default";

interface InternalRoot {
  readonly registration: InternalRegistration;
  readonly canonicalRoot: CanonicalRootPath;
  readonly generation: RootGeneration;
  state: LifecycleRootState;
  consecutiveAbsences: number;
  tombstoned: boolean;
  synchronousPhaseComplete: boolean;
  closePromise: Promise<void> | null;
  eventEmitted: boolean;
  cause: LifecycleCloseCause | null;
}

interface InternalRegistration {
  readonly pool: LifecyclePool;
  readonly concretePoolId: ConcretePoolId;
  readonly registrationIdentity: RegistrationIdentity;
  readonly reapingEnabled: boolean;
  readonly evictOuterFacade: LifecyclePoolRegistrationOptions["evictOuterFacade"];
  readonly roots: Map<string, InternalRoot>;
  readonly handle: LifecyclePoolRegistration;
  active: boolean;
}

function rootKey(root: CanonicalRootPath): string {
  return root;
}

function resultMeansPresent(result: LifecycleStatResult): boolean {
  if (typeof result === "boolean") return result;
  if (typeof result === "object" && result !== null && "exists" in result) {
    return result.exists !== false;
  }
  return true;
}

function isEnoent(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    (error as { code?: unknown }).code === "ENOENT"
  );
}

function closeCounts(result: unknown): {
  sessions: number;
  facades: number;
} {
  const teardown =
    typeof result === "object" && result !== null ? (result as LifecycleTeardownResult) : undefined;
  const sessions = teardown?.tornDownSessionCount;
  const facades = teardown?.tornDownFacadeCount;
  return {
    sessions: typeof sessions === "number" && Number.isFinite(sessions) ? Math.max(0, sessions) : 0,
    facades: typeof facades === "number" && Number.isFinite(facades) ? Math.max(0, facades) : 0,
  };
}

function defaultTimer(): LifecycleTimerSeam {
  return {
    setInterval(callback, delayMs): unknown {
      const handle = setInterval(callback, delayMs);
      const maybeUnref = handle as unknown as { unref?: () => void };
      maybeUnref.unref?.();
      return handle;
    },
    clearInterval(handle): void {
      clearInterval(handle as ReturnType<typeof setInterval>);
    },
  };
}

/**
 * One lifecycle registry per JavaScript module realm.
 *
 * The registry deliberately knows only the concrete pool's root-close seam. It
 * owns root generations, absence observations, and transition ownership, while
 * the concrete pool remains responsible for indexed session and route cleanup.
 */
export class LifecycleRegistry {
  readonly realm: string;
  readonly intervalMs: number;

  private readonly registrations = new Map<ConcretePoolId, InternalRegistration>();
  private readonly registrationByPool = new WeakMap<object, InternalRegistration>();
  private readonly nextGenerationByRoot = new Map<string, number>();
  private nextConcretePoolNumber = 1;
  private nextRegistrationSequence = 1;
  private readonly timer: LifecycleTimerSeam;
  private readonly statRoot: LifecycleStat;
  private readonly demand: LifecycleDemandCheck;
  private readonly onEvent?: (event: LifecycleEvent) => void;
  private intervalHandle: unknown = null;
  private sweepInFlight: Promise<void> | null = null;

  constructor(options: LifecycleRegistryOptions = {}) {
    this.realm = options.realm ?? DEFAULT_REALM;
    this.intervalMs = options.intervalMs ?? DEFAULT_LIFECYCLE_INTERVAL_MS;
    if (!Number.isFinite(this.intervalMs) || this.intervalMs <= 0) {
      throw new RangeError("lifecycle interval must be a positive finite number");
    }

    const nativeTimer = defaultTimer();
    this.timer =
      options.timer ??
      (options.setInterval || options.clearInterval
        ? {
            setInterval: options.setInterval ?? nativeTimer.setInterval,
            clearInterval: options.clearInterval ?? nativeTimer.clearInterval,
          }
        : nativeTimer);
    this.statRoot = options.stat ?? ((root) => lstat(root));
    this.demand = options.demandCheck ?? (() => false);
    this.onEvent = options.onEvent;
  }

  /** Register an unregistered concrete pool and permanently bind its reapingEnabled setting. */
  registerLifecyclePool(
    pool: LifecyclePool,
    options: LifecyclePoolRegistrationOptions,
  ): LifecyclePoolRegistration {
    if (!pool || typeof pool.closeProjectRoot !== "function") {
      throw new TypeError("a lifecycle pool must provide closeProjectRoot");
    }
    if (typeof options?.evictOuterFacade !== "function") {
      throw new TypeError("lifecycle registration requires evictOuterFacade");
    }
    if (typeof options.reapingEnabled !== "boolean") {
      throw new TypeError("reapingEnabled must be a boolean");
    }

    const previous = this.registrationByPool.get(pool);
    if (previous?.active) this.deregisterInternal(previous);

    const concretePoolId = asConcretePoolId(`pool-${this.nextConcretePoolNumber++}`);
    const registrationIdentity = Object.freeze({
      concretePoolId,
      registrationSequence: this.nextRegistrationSequence++,
    });
    const internal = {
      pool,
      concretePoolId,
      registrationIdentity,
      reapingEnabled: options.reapingEnabled,
      evictOuterFacade: options.evictOuterFacade,
      roots: new Map<string, InternalRoot>(),
      handle: undefined as unknown as LifecyclePoolRegistration,
      active: true,
    } satisfies Omit<InternalRegistration, "handle"> & { handle: LifecyclePoolRegistration };
    const handle: LifecyclePoolRegistration = {
      registrationIdentity,
      identity: registrationIdentity,
      get concretePoolId(): ConcretePoolId {
        return concretePoolId;
      },
      get reapingEnabled(): boolean {
        return internal.reapingEnabled;
      },
      deregister: (): void => {
        this.deregisterInternal(internal);
      },
    };
    internal.handle = handle;
    this.registrations.set(concretePoolId, internal);
    this.registrationByPool.set(pool, internal);
    this.refreshTimer();
    return handle;
  }

  /** Functional form used by construction sites that keep a registry instance. */
  registerPool(
    pool: LifecyclePool,
    options: LifecyclePoolRegistrationOptions,
  ): LifecyclePoolRegistration {
    return this.registerLifecyclePool(pool, options);
  }

  /** Register a root before its facade is exposed. Repeated live registration is idempotent. */
  registerRoot(poolId: ConcretePoolId, root: CanonicalRootPath): RootGeneration {
    const registration = this.currentRegistration(poolId);
    if (!registration) throw new Error(`unknown lifecycle pool ${poolId}`);

    const key = rootKey(root);
    const existing = registration.roots.get(key);
    if (existing?.state === "live") return existing.generation;
    if (existing && !existing.synchronousPhaseComplete) {
      // This is only reachable through re-entrant code in the synchronous close
      // phase. Exposing the old generation would permit resurrection, so callers
      // must wait until that phase has finished and ask again.
      throw new Error("root registration raced its synchronous close phase");
    }

    const next = (this.nextGenerationByRoot.get(`${poolId}\u0000${key}`) ?? 0) + 1;
    this.nextGenerationByRoot.set(`${poolId}\u0000${key}`, next);
    const entry: InternalRoot = {
      registration,
      canonicalRoot: root,
      generation: asRootGeneration(next),
      state: "live",
      consecutiveAbsences: 0,
      tombstoned: false,
      synchronousPhaseComplete: true,
      closePromise: null,
      eventEmitted: false,
      cause: null,
    };
    registration.roots.set(key, entry);
    return entry.generation;
  }

  /**
   * Get an existing live generation without creating one. This is the only
   * non-creating lookup used by status and background paths.
   */
  currentGeneration(poolId: ConcretePoolId, root: CanonicalRootPath): RootGeneration | undefined {
    return this.currentRoot(poolId, root)?.generation;
  }

  isTombstoned(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
    generation: RootGeneration,
  ): boolean {
    const entry = this.currentRoot(poolId, root);
    return entry?.generation === generation && entry.tombstoned;
  }

  isCurrentRegistration(identity: RegistrationIdentity): boolean {
    const registration = this.registrations.get(identity.concretePoolId);
    return registration?.active === true && registration.registrationIdentity === identity;
  }

  isCurrentLiveGeneration(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
    generation: RootGeneration,
  ): boolean {
    const entry = this.currentRoot(poolId, root);
    return entry?.generation === generation && entry.state === "live";
  }

  getRootSnapshot(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
  ): LifecycleRootSnapshot | undefined {
    const entry = this.currentRoot(poolId, root);
    return entry ? this.snapshotRoot(entry) : undefined;
  }

  /** Return only roots visible to lifecycle operations; tombstones are retained until close settles. */
  rootSnapshots(poolId?: ConcretePoolId): LifecycleRootSnapshot[] {
    const entries = poolId
      ? [this.currentRegistration(poolId)].filter(
          (registration): registration is InternalRegistration => registration != null,
        )
      : [...this.registrations.values()];
    return entries.flatMap((registration) =>
      [...registration.roots.values()].map((entry) => this.snapshotRoot(entry)),
    );
  }

  snapshot(): LifecycleRegistrySnapshot {
    return {
      realm: this.realm,
      intervalMs: this.intervalMs,
      timerActive: this.intervalHandle !== null,
      registrations: [...this.registrations.values()].map((registration) => ({
        concretePoolId: registration.concretePoolId,
        reapingEnabled: registration.reapingEnabled,
        roots: [...registration.roots.values()].map((entry) => this.snapshotRoot(entry)),
      })),
    };
  }

  /** Alias for callers that use status terminology. */
  status(): LifecycleRegistrySnapshot {
    return this.snapshot();
  }

  /**
   * Demand gate for initial and successor creation. A live root is returned
   * without repeating the filesystem check; absent roots require a fresh positive
   * result and then receive exactly one greater generation.
   */
  async ensureRootForDemand(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
  ): Promise<RootGeneration | undefined> {
    const registration = this.currentRegistration(poolId);
    if (!registration) return undefined;
    const existing = registration.roots.get(rootKey(root));
    if (existing?.state === "live") return existing.generation;
    if (existing && !existing.synchronousPhaseComplete) return undefined;

    let result: boolean | { readonly exists?: boolean };
    try {
      result = await this.demand(root, poolId);
    } catch {
      return undefined;
    }
    if (!resultMeansPresent(result)) return undefined;

    const stillCurrent = this.currentRegistration(poolId);
    if (stillCurrent !== registration) return undefined;
    const afterCheck = registration.roots.get(rootKey(root));
    if (afterCheck?.state === "live") return afterCheck.generation;
    if (afterCheck && !afterCheck.synchronousPhaseComplete) return undefined;
    return this.registerRoot(poolId, root);
  }

  demandRoot(poolId: ConcretePoolId, root: CanonicalRootPath): Promise<RootGeneration | undefined> {
    return this.ensureRootForDemand(poolId, root);
  }

  getBridgeDemand(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
  ): Promise<RootGeneration | undefined> {
    return this.ensureRootForDemand(poolId, root);
  }

  /**
   * Coordinate one generation's close. The live-to-reaping transition, tombstone,
   * outer eviction, and concrete close invocation all happen before the returned
   * promise can yield. Concurrent requests coalesce on the same close promise.
   */
  requestProjectRootClose(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
    expectedGeneration: RootGeneration,
    cause: LifecycleCloseCause = "explicit",
  ): Promise<void> {
    const registration = this.currentRegistration(poolId);
    if (!registration) return Promise.resolve();
    const entry = registration.roots.get(rootKey(root));
    if (!entry || entry.generation !== expectedGeneration) return Promise.resolve();
    if (entry.state === "reaping") return entry.closePromise ?? Promise.resolve();
    if (entry.state === "tombstoned") return entry.closePromise ?? Promise.resolve();

    entry.state = "reaping";
    entry.tombstoned = true;
    entry.cause = cause;

    let closeResult: unknown;
    try {
      registration.evictOuterFacade(entry.canonicalRoot, entry.generation);
      closeResult = registration.pool.closeProjectRoot(entry.canonicalRoot, entry.generation);
      entry.synchronousPhaseComplete = true;
      entry.state = "tombstoned";
    } catch (error) {
      entry.synchronousPhaseComplete = true;
      entry.state = "tombstoned";
      const failed = Promise.reject(error);
      entry.closePromise = failed.then(
        () => undefined,
        (failure) => {
          this.finishRootClose(entry, undefined);
          throw failure;
        },
      );
      return entry.closePromise;
    }

    entry.closePromise = Promise.resolve(closeResult).then(
      (result) => {
        const counts = closeCounts(result);
        this.finishRootClose(entry, counts);
      },
      (error) => {
        this.finishRootClose(entry, undefined);
        throw error;
      },
    );
    return entry.closePromise;
  }

  /** Coalesce concurrent ticks and observe only enabled, live registrations. */
  sweep(): Promise<void> {
    if (this.sweepInFlight) return this.sweepInFlight;
    const sweep = this.runSweep();
    this.sweepInFlight = sweep;
    sweep.then(
      () => {
        if (this.sweepInFlight === sweep) this.sweepInFlight = null;
      },
      () => {
        if (this.sweepInFlight === sweep) this.sweepInFlight = null;
      },
    );
    return sweep;
  }

  /** Alias used by timer seams and tests. */
  sweepNow(): Promise<void> {
    return this.sweep();
  }

  /** Emit a generation-rejection event when a lifecycle boundary refuses the expected generation. */
  recordGenerationRejection(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
    expectedGeneration: RootGeneration,
    boundary: string,
  ): void {
    const currentGeneration = this.currentGeneration(poolId, root);
    this.emit({
      type: "subc_root_generation_rejected",
      realm: this.realm,
      concretePoolId: poolId,
      canonicalRoot: root,
      expectedGeneration,
      currentGeneration,
      boundary,
    });
  }

  rejectGeneration(
    poolId: ConcretePoolId,
    root: CanonicalRootPath,
    expectedGeneration: RootGeneration,
    boundary: string,
  ): void {
    this.recordGenerationRejection(poolId, root, expectedGeneration, boundary);
  }

  private async runSweep(): Promise<void> {
    const snapshots = [...this.registrations.values()]
      .filter((registration) => registration.active && registration.reapingEnabled)
      .flatMap((registration) =>
        [...registration.roots.values()]
          .filter((entry) => entry.state === "live")
          .map((entry) => ({
            concretePoolId: registration.concretePoolId,
            canonicalRoot: entry.canonicalRoot,
            generation: entry.generation,
          })),
      );

    const closes: Promise<void>[] = [];
    await Promise.all(
      snapshots.map(async (snapshot) => {
        let present = false;
        try {
          const result = await this.statRoot(
            snapshot.canonicalRoot,
            snapshot.concretePoolId,
            snapshot.generation,
          );
          present = resultMeansPresent(result);
        } catch (error) {
          if (!isEnoent(error)) return;
        }

        const entry = this.currentRoot(snapshot.concretePoolId, snapshot.canonicalRoot);
        if (!entry || entry.generation !== snapshot.generation || entry.state !== "live") return;
        if (present) {
          entry.consecutiveAbsences = 0;
          return;
        }

        entry.consecutiveAbsences += 1;
        if (entry.consecutiveAbsences >= 2) {
          closes.push(
            this.requestProjectRootClose(
              snapshot.concretePoolId,
              snapshot.canonicalRoot,
              snapshot.generation,
              "sweep",
            ),
          );
        }
      }),
    );
    await Promise.all(closes);
  }

  private currentRegistration(poolId: ConcretePoolId): InternalRegistration | undefined {
    const registration = this.registrations.get(poolId);
    return registration?.active ? registration : undefined;
  }

  private currentRoot(poolId: ConcretePoolId, root: CanonicalRootPath): InternalRoot | undefined {
    return this.currentRegistration(poolId)?.roots.get(rootKey(root));
  }

  private snapshotRoot(entry: InternalRoot): LifecycleRootSnapshot {
    return {
      concretePoolId: entry.registration.concretePoolId,
      registrationIdentity: entry.registration.registrationIdentity,
      canonicalRoot: entry.canonicalRoot,
      generation: entry.generation,
      state: entry.state,
      consecutiveAbsences: entry.consecutiveAbsences,
    };
  }

  private finishRootClose(
    entry: InternalRoot,
    counts: { sessions: number; facades: number } | undefined,
  ): void {
    if (counts && !entry.eventEmitted) {
      entry.eventEmitted = true;
      this.emit({
        type: "subc_root_reaped",
        realm: this.realm,
        concretePoolId: entry.registration.concretePoolId,
        canonicalRoot: entry.canonicalRoot,
        generation: entry.generation,
        consecutiveAbsenceCount: entry.consecutiveAbsences,
        tornDownSessionCount: counts.sessions,
        tornDownFacadeCount: counts.facades,
        cause: entry.cause ?? "explicit",
      });
    }
    const registration = entry.registration;
    if (registration.roots.get(rootKey(entry.canonicalRoot)) === entry) {
      registration.roots.delete(rootKey(entry.canonicalRoot));
    }
  }

  private deregisterInternal(registration: InternalRegistration): void {
    if (
      !registration.active ||
      this.registrations.get(registration.concretePoolId) !== registration
    ) {
      return;
    }

    for (const entry of [...registration.roots.values()]) {
      if (entry.state === "live") {
        const close = this.requestProjectRootClose(
          registration.concretePoolId,
          entry.canonicalRoot,
          entry.generation,
          "explicit",
        );
        void close.catch(() => undefined);
      }
      registration.roots.delete(rootKey(entry.canonicalRoot));
    }
    registration.active = false;
    this.registrations.delete(registration.concretePoolId);
    if (this.registrationByPool.get(registration.pool) === registration) {
      this.registrationByPool.delete(registration.pool);
    }
    this.refreshTimer();
  }

  private refreshTimer(): void {
    const shouldRun = [...this.registrations.values()].some(
      (registration) => registration.active && registration.reapingEnabled,
    );
    if (shouldRun && this.intervalHandle === null) {
      this.intervalHandle = this.timer.setInterval(() => {
        void this.sweep().catch(() => undefined);
      }, this.intervalMs);
      const maybeUnref = this.intervalHandle as { unref?: () => void } | null;
      maybeUnref?.unref?.();
      return;
    }
    if (!shouldRun && this.intervalHandle !== null) {
      this.timer.clearInterval(this.intervalHandle);
      this.intervalHandle = null;
    }
  }

  private emit(event: LifecycleEvent): void {
    try {
      this.onEvent?.(event);
    } catch {
      // Observability must not change lifecycle ownership or teardown behavior.
    }
  }
}

const defaultLifecycleRegistry = new LifecycleRegistry();

/** Create a lifecycle registry for callers that do not inject one; use getLifecycleRegistry() for the shared default. */
export function createLifecycleRegistry(options: LifecycleRegistryOptions = {}): LifecycleRegistry {
  return new LifecycleRegistry(options);
}

export function getLifecycleRegistry(): LifecycleRegistry {
  return defaultLifecycleRegistry;
}

export { LifecycleRegistry as SubcLifecycleRegistry };

export function registerLifecyclePool(
  pool: LifecyclePool,
  options: LifecyclePoolRegistrationOptions,
): LifecyclePoolRegistration {
  return defaultLifecycleRegistry.registerLifecyclePool(pool, options);
}
