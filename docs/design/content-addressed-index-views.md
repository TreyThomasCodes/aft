# Content-addressed index artifacts and per-view assembly (v1 design, 2026-09-04)

Status: design for an Athena pre-draft consult, then a spec campaign. No implementation before the
consult rules the document.

## Problem, with evidence

Every index plane is keyed per checkout: the artifact key (`sha256(sorted root commits)`) selects the
store, and the store holds the files *as they are on disk now*. Three consequences we measured:

1. **Branch switches re-do work and can trigger full rebuilds.** A checkout touching more than
   `WATCHER_BATCH_INLINE_CAP = 256` paths (`crates/aft/src/runtime_drain.rs:1496`) routes to
   `refresh_project_corpus` (`:2581-2591` -> `:1703`), which drops the resident callgraph store and
   forces a cold rebuild (`:1749`; log `callgraph cold-build decision: reason=corpus drift`). This
   repo: 305,297 refs, ~36 refs/s in the resolution stage on 2026-09-04 = hours at 100% CPU. Switching
   back re-embeds and re-extracts the same content again; nothing is content-addressed below the
   file level. (The 256-path reaction is being fixed independently; the re-do-on-return remains.)
2. **Worktrees are frozen readers.** A linked worktree is borrow-only on the main checkout's
   artifacts (`root_cache.rs`, `readonly_artifacts.rs`); its own edits are invisible to semantic
   search and its callgraph is the parent's. The RAM overlay (`worktree.ram_overlay`) covers trigram
   and symbols only. Masons (97.7% of `aft_inspect` calls) live in worktrees.
3. **First load is neither resumable nor shareable for the expensive planes.** The semantic index
   persists only on a completed build (`crates/aft/src/commands/configure.rs:4107-4171`,
   `persist_completed_index` after `build_result` is `Ready`); a daemon kill mid-embed loses
   everything (GitHub #280). The callgraph cold build has staged resume, but extraction is redone
   per checkout. A second clone of an already-indexed repo on the same machine reuses artifacts only
   through the artifact key; a fresh machine reuses nothing.

## Principle

Split each plane into a **per-file part** keyed by content and a **per-view part** keyed by the
checkout:

- `blob = (content_hash, plane_version)` -> per-file artifact, immutable, append-only, shared by every
  checkout of every repo on the machine. Semantic: chunk boundaries + vectors, keyed additionally by
  `(chunker_version, model_fingerprint)`. Callgraph: the per-file extraction (nodes, refs, imports,
  exports, declared modules — everything `extract` produces before cross-file resolution). Symbols:
  the outline.
- `view = (root_id, path -> content_hash)` -> the assembly: which blobs are present, and the per-view
  work that depends on the whole set: callgraph **resolution** (cross-file edges), search top-k over
  the view's blobs, dead-code projection.

Content hash: the git blob hash (`git ls-tree -r HEAD`) for tracked, unmodified files — zero content
reads on a checkout; BLAKE3 of the working-tree bytes for dirty/untracked files, computed by the
watcher path that already reads them. A file's blob key is whichever applies; both are content
hashes, so a dirty file that matches a committed blob elsewhere still shares.

## What falls out

| situation | today | with views |
|---|---|---|
| checkout branch B (300 files) | watcher burst -> force cold rebuild (callgraph); re-embed 300 files | view switch: 300 hash lookups; embed/extract only blobs never seen; resolution re-run for the view (bounded by the resolver fix) |
| switch back to A | everything again | zero embeds, zero extractions; resolution re-run or restored from the view cache |
| worktree edit | invisible to semantic; callgraph is the parent's | new blob embedded once, shared back; worktree view = shared blobs + its deltas; no writer lease (idempotent appends) |
| daemon killed mid-embed | lose all | lose nothing; every finished blob is durable |
| second clone / same repo | artifact-key reuse only | free (all blobs present) |
| new machine | nothing | export/import of the blob store (later) |

## Storage

- Blob store: one SQLite per plane under `<storage>/blobs/<plane>/`, WAL, `PRIMARY KEY(content_hash,
  plane_version[, model_fingerprint])`, `INSERT OR IGNORE` (idempotent puts; concurrent writers from
  several checkouts are correct without a lease), `busy_timeout`. Vectors stored as fixed-width blobs;
  extraction rows as the same schema the callgraph store uses for nodes/refs but keyed by hash
  instead of path. GC: refcount by live views (a view lists its hashes); unreferenced blobs older than
  N days are swept by the existing maintenance sweeps.
- View: per root, `<storage>/views/<root_id>/` holding the path->hash manifest (generation-swapped like
  today's callgraph pointer) and the per-view derived tables (resolved edges, dead-code projection).
  The trigram index stays per-view as today (cheap, incremental, not where the cost is).

## Refresh model

- Checkout / large batch: read HEAD's tree, diff manifests, enqueue missing blobs to the plane
  workers (bounded by `ColdBuildLimiter` and the breaker as today), publish the new manifest when the
  blobs exist, then run per-view resolution. No `refresh_project_corpus` on size; only on true
  overflow (lost events) and ignore-rule changes.
- Editing: the watcher hashes the file, puts the blob if new, updates the manifest entry.
- Query while assembling: the view reports which paths are pending (honest partial, as today), never
  a wholesale `Building`.

## Invariants (the consult should attack these)

1. A blob's content is a pure function of `(bytes, plane_version[, model_fingerprint])`; nothing
   path-dependent may live in a blob. (Callgraph extraction today records `caller_file` paths inside
   refs — those move to the view join, or the blob stores path-relative data only.)
2. A view is exactly `manifest + blobs`; two views with equal manifests produce byte-identical
   derived tables (resolution is deterministic given the set).
3. Concurrent writers never conflict: puts are idempotent; manifests are generation-swapped per root;
   no cross-root lease exists.
4. Model/chunker/plane-version changes never mix: the key carries them; old blobs are GC'd, never
   reinterpreted.
5. Read-only roots (borrow-only, untrusted) may READ the blob store and write their own manifests;
   whether they may PUT blobs is a trust question — default yes for first-party (content is
   deterministic from bytes they can already read), no for `mcp:*`.

## Non-goals (v1)

Trigram index restructuring; cross-machine export; changing the artifact key; any change to the
tool surface.

## Relationship to RFC #286

randomvariable's RFC proposes incremental segment indexing, coverage manifests, and CoW worktree
generations. A "coverage manifest" is this design's view manifest; "segments" are blob groups. The
designs converge on the split; they differ on the unit (segment vs blob) and on whether the trigram
index participates. The #286 reply is written in this frame after the OSS matrix numbers.

## Open questions for the consult

1. Blob granularity for semantic: per file or per chunk? Per chunk shares across files with common
   regions but multiplies rows ~30x; per file is simpler and shares on the case that matters
   (unchanged files across views).
2. Where does per-view callgraph resolution get its memoized module-resolution state — per view, or
   shared by repo identity?
3. Migration: does the first view of an existing checkout adopt today's artifacts (import
   `semantic.bin` vectors as blobs keyed by the file hashes recorded there) or rebuild once?
4. GC policy and the on-disk budget interaction with #210 (aggregate artifact budget).
5. Untrusted binds and blob PUT (invariant 5).
