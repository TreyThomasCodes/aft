# Standing indexed roots (`index.roots`) — design v1

Community demand (Discord 2026-08-20): agents/users want folders indexed that no
session has opened — docs folders, sibling repos, reference checkouts. Two prior
rulings frame the design: non-git roots get NO eager indexing (the OOM-loop
campaign), and the product direction includes document search for non-SWE users.
An explicit standing-roots list is the reconciliation: user intent replaces
eagerness.

Owner constraints (Ufuk, 2026-08-23): NOT a flat path list — per-root granular
control over which indexes; the already-a-project-root collision and the
global-vs-project config conflict must be designed before code.

## 1. Shape: per-entry objects, user-tier ONLY

```jsonc
// ~/.config/cortexkit/aft.jsonc — user tier exclusively. A project tier that
// could add index roots would let a cloned repo index arbitrary disk paths.
"index": {
  "roots": [
    { "path": "~/Documents/notes", "indexes": ["search", "semantic"] },
    { "path": "~/Work/OSS/reference-repo", "indexes": ["search", "callgraph"] },
    { "path": "~/Work/sibling", "indexes": ["search"] }  // minimal entry
  ]
}
```

- `path`: absolute or `~`-expanded. Symlink-resolved at load; nonexistent paths
  are warnings, not errors (drives unmount).
- `indexes`: explicit subset of `["search", "semantic", "callgraph"]`. No
  defaults ladder — the owner's granularity requirement is satisfied by making
  the selection explicit and mandatory (empty/absent `indexes` = config error).
  Semantic entries inherit the user-tier semantic backend config verbatim.
- Per-entry overrides deliberately EXCLUDED from v1 (no per-entry backend, no
  per-entry budgets). One knob per axis; entry-level tuning only if real usage
  demands it.

## 2. Identity: same derivation, no new cache universe

A standing root derives its `artifact_cache_key` exactly as session roots do
(git root-commit set where git exists, path-scope key otherwise). Consequences:

- A standing root that IS a git repo shares artifacts byte-for-byte with any
  future session on that checkout. Standing indexing is pre-warming, not a
  parallel store.
- Non-git standing roots use the path-scope key (the inspect-cache precedent).
- The #250 breaker covers standing builds with the same domains and the same
  suspension surfaces (health/doctor name the standing root like any root).

## 3. Collision semantics (the hard part)

Case A — standing root later opened as a session project root:
- The session takes precedence categorically. Ownership uses the EXISTING
  writer-lease machinery: the standing maintainer holds leases only while
  actively building, in short lease windows, and NEVER contends against a bind
  (bind-time lease acquisition already queue-jumps cold builds post-#250).
- The July ownership-asymmetry storm is the anti-pattern to design against: a
  background maintainer must not lock a live session into ReadOnly. Rule:
  standing maintenance for root R suspends entirely while any live session
  (route/bridge) is bound to R — the session's own configure/watcher machinery
  is now the maintainer of the same artifacts. Standing maintenance resumes
  only after the root is unbound AND idle-TTL-expired.

Case B — standing root nests with a project root (parent or child):
- No merging. A standing entry that is an ANCESTOR of a session root indexes
  its own scope (including the session subtree — same as any monorepo parent);
  artifact keys differ, no contention (different scope keys). A standing entry
  that is a DESCENDANT of a session root is refused at config load with a
  warning naming the covering root (indexing a subtree twice buys nothing and
  double-pays watchers/builds).

Case C — config conflict (global entry vs project's own `.cortexkit/aft.jsonc`):
- When a session is bound, the project+user resolved config governs — standing
  entry settings are IGNORED for a bound root (they only ever govern unbound
  maintenance). If the standing entry's semantic fingerprint differs from what
  the session builds (backend/model mismatch), the standing maintainer adopts
  the on-disk artifact's fingerprint if servable, else skips semantic for that
  root with a health-visible reason — it never rebuilds against a live
  session's artifact (no flip/flop re-embeds; the foreign-artifact guard
  already enforces preserve-on-mismatch).

## 4. Execution model: who does the work

- subc mode: the daemon maintains standing roots as ordinary (bindless) roots —
  watcherless, verify-on-demand. Maintenance runs under the cold-build limiter
  and the #250 breaker, in the MaintenanceCommit class (never contends with
  interactive lanes).
- Standalone mode: any live bridge process opportunistically adopts standing
  maintenance during idle sweeps (shared storage means artifacts persist across
  sessions). No live process = no maintenance = stale-with-disclosure on next
  query. This keeps the feature meaningful for the public (non-subc) majority.
- NO standing watchers. Freshness is verify-on-query (stat-first memo, strict
  on drift) exactly like evicted-root rebinds. Watcher fds/CPU for never-
  queried roots is the #202 thread-explosion class in new clothes.

## 5. Query surface

- `aft_search`/`grep` with `path` into a standing root serve from its artifacts
  (today's borrowed-index path, minus the "nobody ever indexed this" hole).
- Force-restricted binds (MCP) keep today's gate: standing roots are NOT
  reachable through restricted binds. Standing indexing changes what exists on
  disk, never who may read it.
- No agent-facing "index this directory" verb. Adding a root is a human config
  act. (An agent may SUGGEST an entry in prose; it cannot mint one.)

## 6. Failure/lifecycle surfaces

- Standing roots appear in health/status/doctor with their index states and any
  breaker suspensions, labeled as standing (not session) roots.
- Removal from config retires maintenance; artifacts age out via the existing
  generation GC (no immediate deletion — re-adding is cheap).
- Disk budget: standing roots count into the aggregate-artifact-budget design
  (#210) when that lands; v1 ships without a dedicated cap but with per-root
  sizes visible in health.

## v2 amendments (binding, from the machinery audit 2026-08-23)

The adversarial panel never received this document (it was uncommitted - Athena
gathers from the git snapshot; this file is now force-added), but its
machinery findings against fc29e33e are evidence-anchored and binding on v2:

1. STANDING IS A NEW LIFECYCLE, NOT A REUSE. The shipped lifecycle inverts
   standing needs: unbound-quiesced roots refuse drains, and reap_idle_roots
   evicts session-less roots after TTL, then invalidates artifacts after the
   watcher gap. Standing roots get their own lifecycle bit: maintenance runs
   only while unbound, skips on pending-bind, and is exempt from idle reap
   and gap invalidation.
2. CASE-A NEEDS A REAL HANDOFF. Pending-bind deferral only suppresses NEW
   maintenance enqueues; nothing cancels an in-flight standing build, leases
   cannot be yanked, and the cold-build limiter has class alternation - not
   bind priority. Amendment: bind admission is the linearization point - it
   revokes standing admission, sets the standing build's cancellation token,
   and JOINS the writer lease with a bounded yield (builds get cancellation
   checkpoints; an admission-epoch check under the lease immediately before
   publication prevents a stale standing publish after bind).
3. CASE-C NEEDS WRITE-SIDE FENCING. The session fingerprint check is
   read-side reject-and-rebuild; semantic publish is last-rename-wins. Two
   entitled writers with divergent fingerprints can alternate publishes.
   Amendment: fingerprint adoption is durable for an ownership epoch -
   write-side compare-and-swap on (fingerprint, generation) under the lease;
   a standing maintainer never publishes a fingerprint the artifact does not
   already carry.
4. STANDING WORK RUNS IN ITS OWN ACTOR. A foreign-root build inside a
   session-scoped bridge inherits that session's path restriction, memory
   attribution, and undo/backup adjacency. The v1 "standalone opportunistic
   adoption" model is WITHDRAWN; standing maintenance is subc-mode only in
   v2, in a dedicated standing actor with its own limiter class and memory
   accounting. Standalone users get a CLI verb (npx @cortexkit/aft index)
   as the no-daemon path (open question 2 resolved).
5. PATH PINNING. Canonicalization follows symlinks and git-root resolution
   walks to toplevel - both can broaden the corpus beyond what the user
   typed. Amendment: record the canonicalized target at config ingestion and
   require resolved-path == recorded-entry at every maintenance pass (drift
   = refuse with a named reason); NO git-toplevel expansion for standing
   entries - the entry indexes exactly the tree the user named.
6. Confirmed from v1: user-tier only (project-tier and MCP-supplied entries
   rejected), breaker/limiter coverage shared across standing/session
   handoffs, watcher-gap strict verification preserved.

## Open questions for review

1. Case-B descendant refusal vs allow-with-dedup — is refusal too rigid for
   monorepo users who want a hot subtree indexed semantically but not the repo?
2. Standalone opportunistic adoption: acceptable that a user with no sessions
   running gets no standing maintenance, or does that demand a CLI verb
   (`npx @cortexkit/aft index --once`) as the no-daemon fallback?
3. Should `indexes: ["semantic"]` imply search (semantic search currently
   fuses with lexical for hybrid ranking — a semantic-only root would answer
   hybrid queries oddly)?
4. Config validation surface: config error vs warning for unknown index names
   (forward-compat with future index kinds).
