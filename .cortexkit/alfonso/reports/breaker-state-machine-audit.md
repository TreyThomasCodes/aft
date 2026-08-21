# Build-death breaker state-machine audit

**Status:** confirmed prerequisite for the bounded/resumable callgraph implementation.

**Authority:** the campaign draft
`drafts/2026-08-21-bounded-resumable-callgraph-builds-with-a-general-build-death-breaker-refire-3.md`, especially the binding breaker, attribution, reset, and temporary-sweep rulings. This audit makes no threshold amendment. A breaker implementation may not ship until its automated tests satisfy the matrix below.

## Locked constants

| Name | Value | Decision and rationale |
|---|---:|---|
| `ZERO_CREDIT_DEATH_LIMIT` | 3 attributed deaths | Confirmed owner starting threshold. Credited work does not erase this tally. |
| `CREDITED_DEATH_LIMIT` | 6 attributed deaths | Confirmed owner starting threshold. This is independent of the zero-credit tally so one committed batch per death cannot make a death loop immortal. |
| `IN_BUILD_BURN_LIMIT_MS` | 30 minutes | Confirmed owner starting threshold. It is cumulative time spent in an expensive build phase without ready publication. |
| `TRIP_TTL_MS` | 24 hours | Confirmed owner starting threshold. Expiry lifts suspension only; it is not a full reset. |
| `ATTEMPT_MARKER_HEARTBEAT_INTERVAL_MS` | 5 seconds | Matches the existing durable lock heartbeat cadence in `fs_lock`. |
| `ATTEMPT_MARKER_RECENT_HEARTBEAT_MS` | 15 seconds | Three heartbeat intervals. A valid marker is recent when `now - heartbeat_at_ms <= 15_000`; at 15,001 ms it is stale. This is a named, fake-clock-testable sweep classification boundary. |
| `TEMP_DELETE_AGE_FLOOR_MS` | 24 hours from temp mtime | Confirmed owner ruling for ordinary same-host orphan deletion. |
| `SWEEP_AMBIGUITY_TTL_MS` | 7 days | Confirmed fixed owner ruling. Ambiguous evidence is retained for this entire interval. |
| `SWEEP_STAT_CHECK_CAP` | 64 per startup or maintenance pass | Confirmed starting work cap; continuation is required rather than an unbounded scan. |

The first four values are locked by the campaign chair ruling; no additional approval is required because none is amended here. Changing any locked value requires an explicit owner amendment in the campaign draft, including rationale, owner identity, and replacement tests.

## Scope and durable namespace

Breaker state is keyed by the canonical **`(root_id, domain, corpus_fingerprint)`** tuple. `root_id` is always part of the key even where two roots happen to have identical content. The corpus fingerprint changes only for content or ignore-rules changes; configure generation, reconnect, rebind, cache-key movement, and force-rebuild token are not fingerprint inputs.

The covered-domain enum and required scheduler mapping are:

| Domain | Covered work |
|---|---|
| `callgraph_cold` | Callgraph-store cold builds, including configure warm and forced rebuild paths. |
| `search_cold` | Trigram search cold builds, including configure warm and rebuild-from-scratch paths. |
| `semantic_seed` | Semantic cold seed collect-and-embed runs. |
| `tier2_scan` | Scheduler-admitted inspect Tier-2 full scans, not quick reuse verification. |

Watcher delta refreshes, legacy migrations, LSP lifecycle, foreground formatter/checker work, and ordinary in-process build failures remain excluded for the reasons fixed in the campaign ruling. Adding a background-build domain requires an explicit enum coverage or exclusion decision.

A breaker record contains separate cumulative `zero_credit_deaths`, `credited_deaths`, and `in_build_burn_ms` fields; suspension reason and time; an applicable breaker-configuration version; and an idempotency ledger keyed by `attempt_id`. It never derives credit from database length, SQLite page count, cursor position, or row count. Historical records for an old fingerprint remain auditable but cannot suspend a new fingerprint.

## Attempt admission, marker, and heartbeat protocol

An attempt is chargeable only after all of the following ordered actions succeed:

1. Resolve the root, domain, and corpus fingerprint; acquire the domain writer lease; and atomically admit the attempt against the breaker record. A suspended record rejects admission before any staging write. Lease waiters are never admitted attempts and never receive a marker.
2. Allocate an `attempt_id`, reserve a staging-temp identity without writing it, and snapshot that staging generation's `committed_extracted_bytes` counter as `start_committed_extracted_bytes`.
3. Durably create the attempt marker, using write-temp, file sync, atomic rename, and parent sync. The marker records root, domain, cache key, corpus fingerprint, attempt id, pid, process start time, hostname, phase, lease epoch, heartbeat, staging-temp identity, and the starting counter.
4. Only after the marker is durable may the process create or write the staging SQLite temp. Immediately before extraction, index construction, resolution, or reconciliation begins, the marker must durably enter that expensive phase.

A crash after admission but before durable marker creation is an uncharged preflight abort: recovery idempotently releases the provisional admission, and no staging temp can exist because the ordering forbids its creation. A marker initially in `admitted` or `preparing` is not chargeable. An expensive phase is `extracting`, `indexing`, `resolving`, or `reconciling`; lease waiting, idle state, and completed publication are not expensive phases. Every five seconds the active attempt atomically replaces and syncs its marker heartbeat. The marker writer must stop and join before terminal removal so a late heartbeat cannot recreate a cleared marker.

A marker reference is classified as follows during recovery or sweeping:

- **live:** valid same-host metadata identifies a currently live process with the recorded process-start time;
- **recent:** valid metadata has a non-future durable heartbeat no older than `ATTEMPT_MARKER_RECENT_HEARTBEAT_MS`;
- **dead:** valid same-host metadata proves the recorded process instance is gone, including a PID-reuse mismatch; and
- **ambiguous:** cross-host metadata, malformed or unreadable metadata, unavailable process-start evidence for a live PID, future timestamps, clock regression, or any other unclassifiable evidence.

Live and recent references protect the temp. Ambiguous references protect it through the seven-day ambiguity TTL. A dead marker can support attribution only when it names an expensive phase and the exact process instance is proven dead.

## State transitions and attribution

| Event | Required durable transition | Breaker effect |
|---|---|---|
| Clean terminal exit | Stop heartbeat, durably mark terminal or remove the marker, then release lease. | No death is charged. |
| Intentional lease-loss exit | Durably record `lease_lost` before stopping work and clearing the marker. | No death is charged. Work may not publish after lease loss. |
| Intentional supersession exit | Durably record `superseded` before stopping work and clearing the marker. | No death is charged. A replacement attempts normal admission. |
| Process death | Before adoption or new staging writes, reconcile the orphan marker. Insert an idempotent death-ledger row for its `attempt_id`, then remove the marker. | Charge only an exact-dead, expensive-phase marker. Repeated reconciliation of the same marker cannot add another death. |
| Successful ready publication | Flip the ready generation under the revalidated lease, then durably record publication/reset evidence before marker cleanup. | Full reset for that exact namespace only. Staged commits are never a reset. |
| Threshold crossing | Persist reason, count, `suspended_since`, and `suspended_until = now + 24h`; stop the live attempt with a terminal `tripped` disposition. | Later admissions for this tuple return durable suspended status without creating staging work. |
| TTL expiration | Clear only active suspension after `suspended_until`. | This is a rate-limited probe, not a reset: all tallies and burn remain. The next attributed death or durable burn evaluation re-evaluates retained evidence and can trip again. |
| Doctor reset | Record an explicit audited reset for one root, one domain, and one fingerprint. | Full reset only for the requested tuple. |
| Force-rebuild token movement | Record the new token and audited reset for the target domain's exact tuple. | Full reset for that tuple; the token does not change the fingerprint. |
| Applicable breaker-configuration change | Archive the old configuration-version record and create the new version's empty record atomically. | Full reset only where the changed breaker configuration applies. |
| Configure-generation movement, reconnect, or rebind | May invalidate a staging cursor where specified, but does not mutate the breaker namespace or counters. | Never resets or launders breaker history. |
| Content or ignore-rules fingerprint movement | Create/use the new fingerprint record; retain the old record for audit. | New fingerprint starts unsuspended without deleting sibling history. |

For an attributed death, recovery reads the staging metadata counter before another attempt may write that staging generation. It computes:

`credit_delta = committed_extracted_bytes_at_death - start_committed_extracted_bytes`.

The delta is credited only when both counter values are valid and monotonic. Invalid or backward evidence is an integrity ambiguity: it grants no credit, cannot silently reset history, preserves the temp under the ambiguity rule, and is surfaced for repair. A positive delta increments `credited_deaths`; a zero delta increments `zero_credit_deaths`. Neither category resets the other. The accumulated durable expensive-phase time is also evaluated against the 30-minute ceiling.

The idempotent death ledger must commit the charged classification and its measured delta once. If the process dies after that ledger transaction but before marker removal, the next recovery sees the same `attempt_id`, does not recharge it, and only finishes cleanup.

## Credit, burn, and transaction invariants

`committed_extracted_bytes` is a monotonic staging-metadata value. Every successful extraction or reconciliation re-extraction transaction inserts its batch-local state and increments the counter by that batch's extracted byte count in the **same SQLite transaction**. A rollback commits neither. Resolution-only rows, file-size changes, database growth, WAL growth, SQLite page reuse, compaction, cursor movement, and row-count changes do not modify the counter and therefore grant no credit.

The burn clock starts only when the durable marker enters an expensive phase. Heartbeats checkpoint bounded wall-clock intervals into the breaker record with an attempt-specific, idempotent watermark. A dead attempt contributes only through its last valid durable heartbeat, never through recovery delay. Future or regressing wall-clock data contributes no burn and is ambiguous for sweeping. This prevents a reboot, clock jump, or delayed startup from fabricating the 30-minute ceiling.

A kill at the SQLite commit boundary must leave both batch rows and the counter increment visible, or neither visible. It must never leave a batch without its counter increment or grant credit for rows rolled back by SQLite. A one-batch-per-death sequence is still credited work, but its credited-death tally reaches six and trips.

## Temporary-file sweep decision

A staging temp and its `-journal`, `-wal`, and `-shm` sidecars form one artifact set. Ordinary deletion requires all of the following for that set:

1. its mtime is at least `TEMP_DELETE_AGE_FLOOR_MS` old;
2. the creator is same-host and its exact process instance is provably dead;
3. no live writer lease exists for the root and domain; and
4. no live or recent durable attempt marker references the staging-temp identity.

Before a sweep can treat a referenced marker as dead and destructible, recovery must persist that marker's idempotent attribution decision. The sweep must not remove a referenced staging set first and thereby erase the counter evidence used for credit. `NotFound` during a deletion race is success. The adopted resume temp remains protected by its marker, including every SQLite sidecar. Startup checks at most 64 candidates and persists/resumes a cursor through periodic maintenance passes; a root that never publishes still participates in that maintenance path.

On ambiguity, the sweeper creates or retains a durable ambiguity observation keyed by the temp identity and records `ambiguous_since` once. It must not refresh that timestamp on every scan. The artifact is retained until `ambiguous_since + SWEEP_AMBIGUITY_TTL_MS`; loss of the observation record restarts, rather than shortens, retention. Cross-host evidence, malformed names or metadata, I/O errors, clock anomalies, and future mtimes/heartbeats are all ambiguity, never ordinary deletion evidence.

## Required automated verification matrix

The implementation slice must add deterministic fixtures with fake clock, process-instance, lease, marker-I/O, and SQLite commit barriers. Tests must observe the durable database/marker state rather than calculate expected credit through the production helper under test.

| Coverage | Required proof |
|---|---|
| Admission and ordering | A forced marker-sync failure, or a kill after admission but before marker persistence, leaves no staging temp and no charged death. A successful admission writes a durable marker before the first temp write. Lease waiters and refused suspended requests write neither marker nor temp. |
| Heartbeat and sweep reference | At 15,000 ms a valid marker remains recent; at 15,001 ms it is stale only when the exact process is dead. A live same-host process remains protected even with an old heartbeat. Cross-host, malformed, future, and clock-regressing evidence survive through seven days. |
| Clean and intentional terminal paths | Clean exit, lease-loss exit, and supersession exit leave no attributed death. Their durable terminal disposition prevents a crash during cleanup from being mischarged. |
| Attributed death | Exact dead pid/start-time/host plus an expensive phase charges once. Idle/preparing markers, lease waiters, clean markers, superseded markers, lease-loss markers, a wrong host, and a live exact instance do not charge. PID reuse proves the recorded prior instance dead without charging the replacement process. |
| Thresholds and TTL | Three zero-credit deaths trip; six credited deaths trip; 30 minutes of durable in-build time trips. Five one-batch credited deaths do not trip and the sixth does. TTL expiration allows one new admission without erasing tallies; the next qualifying observation re-trips from retained history. |
| Reset matrix | Ready publication, doctor reset, force-rebuild-token movement, content movement, ignore-rules movement, and an applicable breaker-configuration change reset only their intended tuple. Configure-generation movement, reconnect, and rebind leave a trip and its counters intact. A kill between pointer flip and breaker-reset evidence recovers to the same publication reset, never an early reset. |
| Root and domain isolation | Parameterize the admission, trip, reporting, and reset suite across every covered domain. A trip or reset for one root/domain/fingerprint cannot charge, suspend, report as, or reset a sibling root or domain. In particular, a search trip never gates callgraph and a callgraph trip never gates search. |
| Credit fabrication resistance | Grow the database, reuse/free SQLite pages, compact or vacuum, move only a cursor, alter row counts outside the extraction transaction, and roll back a batch; each produces zero credit. Reconciliation re-extraction in a successful batch does produce counter growth and credited work. |
| Kill during commit | Kill the writer at an actual SQLite commit barrier for extract and reconciliation batches. After restart, assert both batch rows and counter increment are present or both absent; then prove attribution observes only that counter. |
| Sweep mechanics | Fresh temps survive; aged eligible bases and all sidecars are removed; `NotFound` is success; an adopted temp survives; 64 checks cap one pass and later maintenance drains the remainder. Ambiguity retains at 6 days 23:59:59 and only becomes eligible after the fixed seven-day boundary. |
| Reporting | Health, status, inspect builder-state, and doctor render the same durable domain-specific trip reason, count, and age. A refused callgraph query returns the terminal suspended status rather than inline work or perpetual `Building`. |

Any executable audit fixture or shared-infrastructure change must run `cargo fmt --all` and the full `bash scripts/rust-test-gate.sh` before it ships. This artifact itself introduces no executable fixture or Rust source change.
