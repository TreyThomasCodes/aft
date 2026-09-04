# Content-addressed index artifacts and per-view assembly (v2 design, 2026-09-04)

Status: v2 after the Athena pre-draft consult `ct_…4d2449c5c708` (4 seats, all "directionally right,
invariants not yet true of the code"). Every panel finding is either folded into a ruling below
(R1–R14) or listed under open questions. v2 goes back to the panel once; the spec campaign fires on
that verdict. No implementation before then.

## Problem, with evidence

Every index plane is keyed per checkout: the artifact key (`sha256(sorted root commits)`) selects the
store, and the store holds the files *as they are on disk now*. Three consequences we measured:

1. **Branch switches re-do work and could trigger full rebuilds.** A checkout touching more than
   `WATCHER_BATCH_INLINE_CAP = 256` paths (`crates/aft/src/runtime_drain.rs:1496`) routed to
   `refresh_project_corpus` (`:2581-2591` -> `:1703`), which dropped the resident callgraph store and
   forced a cold rebuild (`:1749`). This repo: 305,297 refs, ~36 refs/s in the resolution stage on
   2026-09-04 = hours at 100% CPU. (The 256-path reaction is fixed independently; the re-do-on-return
   remains: switching back re-embeds and re-extracts identical content.)
2. **Worktrees are frozen readers.** A linked worktree is borrow-only on the main checkout's artifacts
   (`artifact_owner.rs:93-110`, `readonly_artifacts.rs:1-7`); its own edits are invisible to semantic
   search and its callgraph is the parent's. Masons (97.7% of `aft_inspect` calls) live in worktrees.
3. **First load is neither resumable nor shareable for the expensive planes.** The semantic index
   persists only on a completed build (`commands/configure.rs:4107-4171`); a daemon kill mid-embed
   loses everything (#280). Callgraph extraction is redone per checkout.

## Principle

Split each expensive plane into a **per-file part** keyed by content and a **per-view part** keyed by
the checkout:

- `blob` = immutable per-file artifact in a shared append-only store, keyed as ruled in R1/R2.
- `view` = `(project_scope_key, manifest: path -> blob key)` plus the derived per-view tables (callgraph
  resolution = cross-file edges; dead-code projection). Trigram search stays per-view (R11).

## Rulings from the consult

**R1 — Semantic blobs are keyed by `(blake3(bytes), rel_path, chunker_version, model_fingerprint)`.**
The panel showed the embed text bakes `file:<relative>` into the vector input
(`semantic_index.rs:1895-1915, 4579-4609`), so identical bytes at different paths yield different
vectors. Stripping the path changes retrieval quality and forbids importing today's `semantic.bin`
vectors; keeping it forecloses cross-path sharing. Ruling: keep the path in the key. Sharing then holds
for the same path across branches, worktrees, clones and switch-backs — the measured cases — and a
rename costs one re-embed. Invariant 1 is restated as "a blob is a pure function of its key tuple",
with `rel_path` an explicit key component for the semantic plane only. `model_fingerprint` must cover
model id, dimensions, normalization/output format (panel c4). Existing `semantic.bin` vectors import
losslessly on the same root because `IndexedFileMetadata` records the content hash
(`semantic_index.rs:4884-4901`).

**R2 — Callgraph blobs hold unresolved extraction only.** Today `build_file_extract`
(`callgraph_store/mod.rs:8612-8693`) resolves imports, reexports and module refs against
`project_root` and reads other files (tsconfig/package.json via the memo, `callgraph.rs:168-194,
1744`) at extraction time; nodes, refs and hints carry file paths. Ruling: the blob stores the parse
result only — symbols, raw refs with unresolved `module_path` strings, declared modules, exports —
with **blob-local ordinals** for node/ref identity. Path binding, ID materialization, import/module
resolution and every `project_root`-taking helper move into the view join, which already dispatches
through `ResolverIndex` (`mod.rs:2266-2288, 9581-9674`). This is a refactor of the extract helpers,
larger than v1 implied; the spec enumerates every helper (panel ev-8) and its new home. Callgraph key:
`(blake3(bytes), language, extractor_version)`; no path component.

**R3 — Views are keyed by `project_scope_key`, never by the artifact key.** Two worktrees of one repo
share an artifact key but must not share a view (`path_identity.rs:32-50`); keying views by artifact
key would rebuild the frozen-reader problem inside the design. The artifact key is unchanged and now
serves one purpose: the **blob namespace** (R10).

**R4 — Hash domains.** Git blob SHA-1 (over header+bytes) and BLAKE3 over bytes are different
functions; "both are content hashes" was false. Canonical blob content key = `blake3(bytes)`. An alias
table `(git_oid -> blake3)` is filled whenever a clean tracked file is hashed, so subsequent checkouts
resolve most paths from `git ls-tree -r HEAD` with zero reads; a miss falls back to reading and
hashing (still no re-embed if the blake3 is present). Dirty and untracked files hash from the working
tree on the watcher path that already reads them.

**R5 — Determinism (invariant 2).** Equal manifests produce byte-identical derived tables only if (a)
resolution consumes refs in a canonical order — `(caller blob key, ref ordinal)` — not staging rowid
order (`mod.rs:2160-2167`), and (b) every resolution input (tsconfig, package.json, Cargo.toml,
workspace layouts, ignore files) is read from the **manifest's blobs**, never the live filesystem.
The module-resolution memo is per view (it is already snapshot-scoped, `callgraph.rs:96-99`); sharing
it by repo identity would import another checkout's config state. Open question 2 is closed.

**R6 — Same-root manifest publication is serialized.** Generation-swapped pointers give atomic
visibility, not lost-update prevention (`mod.rs:7836-7858`). One writer per view at a time:
compare-and-swap on the manifest generation (publish fails if the base generation moved; the loser
re-derives from the new base). Cross-root blob puts stay lease-free.

**R7 — Puts are hash-verified and atomic; reads verify.** `INSERT OR IGNORE` alone lets the first
wrong writer poison the machine-global store permanently. Each put is one SQLite transaction carrying
the payload and a completeness marker, and the key is recomputed from the payload before insert;
readers verify `key == hash(payload)` on read and treat a mismatch as absent (and log it). Idempotent
and atomic are different properties; the spec claims both and tests both (crash mid-put leaves no
partial row).

**R8 — Durability before visibility.** Order: commit every referenced blob (WAL, `synchronous=NORMAL`
minimum, stated in the spec), fsync, write and fsync the immutable manifest generation, rename the
pointer, then expose. A manifest may only be published when every blob it names is durable; while
blobs are pending the *previous complete* generation stays current and the desired generation reports
per-path pending/failed (`Ready{refreshing}` shape, `context.rs:887-896`). Admission (limiter,
breaker) may delay or refuse work; it never publishes an incomplete manifest.

**R9 — GC is budgeted mark-and-sweep, not refcount.** Roots of the mark: retained manifest generations
(current + previous per view), publication pins written *before* a view starts assembling (the
inspect-sweep and `gc_old_generations` read-marker precedents, `cache.rs:1784-1791`,
`mod.rs:7930-7951`), and a minimum blob age above worst-case assembly time. A byte budget (#210) may
evict only unreferenced blobs; evicting a referenced blob turns invariant 2 into a re-derivation
obligation and is forbidden. View directories of deleted checkouts follow the existing two-sweep
directory-absence reclaim.

**R10 — Blob namespace = artifact key (repo family).** The panel flagged a machine-global store as a
cross-repo existence oracle readable by untrusted binds (vectors of other repos' files). Ruling: the
blob store is partitioned per artifact key. This keeps every measured win (branches, worktrees, clones,
forks — same root commits) and drops cross-repo sharing of vendored files, which was never a goal. GC
enumeration is then bounded to one family's views.

**R11 — Trigram stays per-view**, published under the same view generation so a query never combines
trigram state from one manifest with semantic/callgraph state from another (panel).

**R12 — Trust.** Only daemon-side first-party code performs shared puts (borrow-only linked worktrees
included — this is a deliberate weakening of `readonly_artifacts.rs:4-7`, stated as such); `mcp:*`,
`Unverified` and `fed:*` binds never put and read only their own family's namespace, with an explicit
bind-to-capability rule in `subc/mod.rs` and tests. Quotas per family bound disk exhaustion.

**R13 — Refresh runs on plane workers, never in watcher slices.** The watcher slices
(`runtime_drain.rs:2032-2362`) invalidate and collect paths; hashing, embedding and extraction go to
the plane workers exactly as `SemanticRefreshRequest::Files` does today, preserving park/replay on
unbind and fixing the existing drop path (`:2325-2337`, which loses staleness records). Breaker keys
move from `(root, domain, corpus_fingerprint)` to `(family, plane)` for blob work so one root's
suspension cannot starve blobs another view needs, and identical work is admitted once.

**R14 — Migration.** Semantic: import-on-same-root (R1 makes it lossless). Callgraph: rebuild once via
the existing staged cold build. Migration is one-way with the old artifacts retained until the first
view publishes; rollback = delete the view directory.

## Storage (revised)

- Blob stores: `<storage>/blobs/<artifact_key>/<plane>.sqlite`, WAL, `synchronous=NORMAL`,
  `busy_timeout`, rows `(key tuple as PRIMARY KEY, payload BLOB, complete INTEGER, created_at)`.
- Alias table: `<storage>/blobs/<artifact_key>/oid-alias.sqlite` `(git_oid PRIMARY KEY, blake3)`.
- Views: `<storage>/views/<project_scope_key>/manifest-<gen>.json` (+ pointer), derived tables in
  `<storage>/views/<project_scope_key>/derived-<gen>.sqlite`, pins under `pins/`.

## Invariants (restated)

1. A blob is a pure function of its key tuple (R1/R2); nothing outside the tuple influences the payload.
2. A view is exactly `manifest + blobs`; equal manifests yield byte-identical derived tables (R5).
3. Puts are idempotent, atomic and hash-verified (R7); same-root publication is CAS-serialized (R6);
   GC never removes a pinned or referenced blob (R9).
4. Version and model changes never mix: they are key components; old blobs age out, never reinterpret.
5. Only first-party daemon code writes the shared store; untrusted binds read their own family and
   write only their own manifest (R12).

## Non-goals (v1)

Trigram restructuring; cross-repo blob sharing; cross-machine export (the per-family store makes it a
later feature, not a design change); artifact-key changes; tool-surface changes.

## What the spec must specify (from the panel's "would have to guess" lists)

Canonical path and case-folding rules; symlink and submodule handling; deletion and rename semantics;
dirty-file race detection (hash-then-stat); which config/ignore files enter the manifest; blob-local
ordinal construction; SQLite schema, synchronous mode, corruption recovery, `CREATE TABLE` races under
concurrent openers; manifest CAS protocol; query generation pinning; pending/failed-path semantics and
their rendering in each tool; derived-table invalidation; breaker retry behaviour under the new key;
GC roots, retention, quotas, read markers; authorization per bind class; migration rollback;
observability (`index_event` kinds for put/pin/publish/sweep); conformance tests for all five
invariants.

## Relationship to RFC #286

randomvariable's RFC proposes incremental segment indexing, coverage manifests, and CoW worktree
generations. A coverage manifest is this design's view manifest; segments are blob groups. The designs
converge on the split and differ on the unit and on trigram participation. The #286 reply is written
in this frame after the OSS matrix numbers.

## Open questions for round 2

1. R1 keeps `rel_path` in the semantic key. Is there a retrieval-quality argument for stripping the
   path from the embed text (and taking the one-time re-embed) that outweighs lossless migration?
   Decide with an A/B on the LeBench retrieval set, not by argument.
2. Blob-local ordinals (R2): stable across extractor versions or not? If not, every extractor bump
   invalidates derived tables for all views of the family — acceptable?
3. R6 CAS vs a per-view writer lock: CAS needs re-derivation on conflict (cost = one resolution pass);
   a lock serializes but reintroduces a lease. Which failure mode is preferable under daemon restarts?
