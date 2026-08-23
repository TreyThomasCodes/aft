# Standing indexed roots (`index.roots`) — design v3

Supersedes v1/v2 (`index-roots-v1.md`) IN FULL. The v1 body survives only as
history; two adversarial consults ruled on this design and v3 is the fold of
both: the machinery audit (consult `...7708a4629230`, findings folded as the v2
amendments) and the document review (consult `...90ca6dff8098`, NO-GO as
written, whose findings this revision exists to close). Do not implement from
the v1 file.

Community demand (Discord 2026-08-20): folders indexed that no session opened —
docs folders, sibling repos, reference checkouts. Framing rulings: non-git roots
get NO eager indexing (OOM-loop campaign); product direction includes document
search for non-SWE users; owner constraints (2026-08-23): per-root granular
index selection, and the session-collision + config-conflict semantics designed
before code.

## 1. Shape: per-entry objects, user-tier ONLY (unchanged from v1)

```jsonc
// ~/.config/cortexkit/aft.jsonc — user tier exclusively. A project tier that
// could add index roots would let a cloned repo index arbitrary disk paths.
"index": {
  "roots": [
    { "path": "~/Documents/notes", "indexes": ["search", "semantic"] },
    { "path": "~/Work/OSS/reference-repo", "indexes": ["search", "callgraph"] },
    { "path": "~/Work/sibling", "indexes": ["search"] }
  ]
}
```

- `path`: absolute or `~`-expanded. Symlink-resolved at config load; the
  resolved target is RECORDED, and every later maintenance pass requires
  resolved-path == recorded-entry (drift = refuse with a named reason).
  Nonexistent paths are warnings (drives unmount).
- `indexes`: explicit subset of `["search", "semantic", "callgraph"]`,
  mandatory, no defaults ladder. UNKNOWN NAMES ARE CONFIG ERRORS (Q4 closed:
  fail the entry loudly, list valid names — silently ignoring an unknown index
  kind would read as "indexed" to the user who typed it).
- DEPENDENCY CLOSURE (Q3 closed): `semantic` implies `search` — hybrid queries
  and zero-result lane escalation require the lexical index, and a
  semantic-only entry would silently degrade every query that needs a lexical
  pass. Config accepts `["semantic"]` and normalizes to
  `["semantic", "search"]` with a load-time notice. `callgraph` is
  independent. A query against an index kind the entry does not carry returns
  the existing typed unavailable status naming the entry's selection.
- Per-entry overrides (backend, budgets) deliberately excluded from v1 scope.

## 2. Identity: SCOPED keys for non-toplevel entries (v3 REWRITE — closes the
   audit's gating finding)

The v2 text held two incompatible rules: "same derivation as session roots"
(git root-commit key) and "index exactly the named subtree". A mid-repo entry
would derive the WHOLE repo's commit key while carrying a NARROWER corpus —
same-key/different-corpus overwrite into the shared slot under
last-rename-wins. v3 resolves it:

- Entry == git toplevel (after symlink recording): the standing entry derives
  the session derivation byte-identically (sorted root-commit set) and SHARES
  the session artifact family. This is the pre-warm case, and the only shared
  case.
- Entry INSIDE a git repo (not toplevel): the key is
  `hash(repo_root_commit_set, rel_path_from_toplevel_normalized, "scoped-v1")`
  — deliberately DIVERGENT from the session key. A subtree corpus never
  addresses the whole-repo slot; it gets its own artifact family, stable
  across worktrees of the same repo and across path spellings (rel-path is
  computed from the recorded resolved entry). No pre-warm for the session key
  by design — correctness beats warmth.
- Non-git entry: path-scope key (canonical-path hash), as today for non-git
  session roots.
- Case-B (nested/overlapping entries): collision policy derives from CORPUS
  IDENTITY (the keys above), never from path-nesting assumptions. Two entries
  with the same key are config-load duplicates (second refused with a notice);
  different keys coexist.

## 3. Session collision (Case-A): bind admission is a real linearization point
   (v3 REWRITE — closes the handoff findings)

Standing maintenance for a root SUSPENDS while any session is bound to it and
resumes only after unbind + idle-TTL. The v2 prose ("revoke + cancellation
token + bounded lease-join") is insufficient against shipped machinery; the
binding contract is:

- ONE root-lifecycle critical section at bind admission atomically: marks bind
  pending, revokes standing admission, increments the ADMISSION EPOCH, and
  signals the standing build's cancellation token. (Shipped pending-bind
  deferral only suppresses new enqueues; this section is new machinery.)
- Standing builds carry cancellation CHECKPOINTS (batch boundaries — the
  resumable-build machinery from the OOM campaign already has these) and an
  admission-epoch capture at start.
- PUBLICATION FENCING: every standing publication executes inside an EXCLUSIVE
  per-root publication section — the `ArtifactPublishEpoch::run_if_current`
  shape (mutex held ACROSS the closure), which is the shipped in-process
  primitive with the right geometry. Inside that one section: writer-lease
  epoch check, admission-epoch compare, fingerprint/generation compare, and
  the final rename. Adjacent checks are the race; one section is the fix.
  The filesystem WriterLease remains the cross-process fence; the publication
  section is the in-process fence the Arc-shared lease cannot provide (a
  same-process `acquire_shared` returns the SHARED lease and proves nothing
  about the standing worker's quiescence).
- BOUNDED JOIN, DEFINED: bind admission waits up to 2s for the standing
  build's checkpoint-acknowledged cancellation; on timeout the bind proceeds
  (never blocks a user), and the admission-epoch compare inside the
  publication section makes the abandoned build's publish a guaranteed no-op.
  Both-writers-proceed is structurally impossible: the epoch was bumped
  before the bind proceeded.

## 4. Freshness: verification is never exempted (v3 REWRITE — resolves the
   amendment 1/6 contradiction)

Standing roots are exempt from IDLE EVICTION and artifact DESTRUCTION — never
from verification. The narrow invariant:

- A `needs_strict_verify` flag is set on EVERY observation-gap transition:
  bound→standing handoff, maintainer suspension→resume, daemon restart,
  watcher gap/overflow, and one-shot CLI build→any later query.
- No artifact under a standing entry is reported fresh, served as fresh, or
  used as a publication baseline until strict verification (the shipped
  `WarmVerifyPlan::Strict` path) clears the flag. Suspension explicitly
  RECORDS that standing observation stopped; resume runs strict corpus
  reconciliation before serving or publishing — a bound session having
  maintained its own selection proves nothing about the standing entry's
  selection or fingerprint compatibility.

## 5. Execution: dedicated standing actor, subc-only; CLI is a SNAPSHOT
   (v3 clarification — closes the coherence finding)

- Standing maintenance runs in a dedicated standing actor (its own memory
  attribution, undo/backup isolation — none, it never mutates user files —
  and breaker accounting per standing key). v1's session-bridge opportunistic
  adoption stays WITHDRAWN.
- LIMITER CLASS, SPECIFIED: the cold-build limiter gains a `Standing`
  admission class in its existing alternation rotation (no absolute
  priority), with one rule the existing classes lack: `Standing` YIELDS its
  waiting slot whenever an interactive or session-maintenance acquirer is
  waiting (standing work is never latency-critical). Cancellation semantics:
  the class's permits carry the admission-epoch token from §3. Breaker: death
  charging and suspension per standing artifact key, same durable store.
- CLI (`npx @cortexkit/aft index`, one contract, Q2 closed): a SNAPSHOT
  operation, stated as such in its own output. Runs the selected builds once
  under the standing keys, exits 0 on full success / 2 on partial (named
  gaps) / 1 on failure. Every daemonless query against a standing artifact
  strictly verifies first (§4 flag semantics) and, on drift, serves with an
  explicit `stale: rerun 'npx @cortexkit/aft index'` disclosure — index-once
  is honest-once, never silently forever. No background scheduling: users
  schedule externally or run the daemon.

## 6. Q1 (owner decision recorded): corpus for document roots

Standing entries index the same file classes session roots do (code + text +
docs formats the chunkers support today). Rich-document corpus expansion
(PDF etc.) is a SEPARATE product track — this design deliberately does not
couple to it.

## 7. Security posture (unchanged from v2, restated as binding)

User-tier only; project-tier and MCP-supplied entries are rejected at the
resolver trust boundary. No agent-facing mint-an-index verb. Recorded
resolved-path pinning (§1) kills symlink retargeting; no git-toplevel
expansion ever widens a corpus beyond the named entry (§2 makes the scoped
key carry exactly the subtree). Network-filesystem refusal, budgets, and the
build-death breaker apply to standing work identically to session work.
