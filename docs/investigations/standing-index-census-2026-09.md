# Standing-index census (2026-09)

## Method and corpus

This is a read-only snapshot of eight `aft-*.log` generations, streamed from `~/.local/share/cortexkit/aft/logs/`, plus `cache-keys.json`. The captured log range is **2026-08-30 05:55:18Z through 2026-09-02 18:45:52Z**.

The reproducible census is [`scripts/telemetry/index-census.py`](../../scripts/telemetry/index-census.py). It wrote these complete artifacts:

- [`census-roots.csv`](data/standing-index-census-2026-09/census-roots.csv): one row for each of the 661 cached roots (93 primary checkouts, 568 worktrees; 250 paths still existed when measured).
- [`census-summary.md`](data/standing-index-census-2026-09/census-summary.md): literal-pattern coverage, the full per-root metric table, shape rollup, and attribution gaps.

A worktree's repository label comes from the primary checkout with the same cache key. Git classification is based on the presence of `git_root_commit`, not on whether the path was still present. File shape uses `git ls-files` for git roots and an excluded-directory `find` walk otherwise; all measurements are `n/a` for paths that no longer exist.

Every pattern is counted, including absent families: the generated summary records **0** literal callgraph start records, **0** publish/ready records, and **0** breaker/suspension records. Percentiles are nearest-rank values.

## Per-root table

The generated CSV is the complete table and includes, for every root: repo, primary/worktree kind, git flag, file count, top three extension languages, workspace shape, cold-build count and wall p50/max, resumes, supersessions, decision reasons, tier-2 snapshot time, per-tool slow-call p50/p95, and limiter-wait p95.

The table below is the cold-build/standing-index subset. `cold` is `reported completed builds; p50/max ms`; no literal start/publish pairs were present. `tier2` is `count/p50/max ms`. The complete per-tool p50/p95 cells are deliberately kept in the CSV because a full cell can contain dozens of tools.

| Repo | Kind | Git? | Files | Top languages | Workspace | Cold | Resumes | Supersessions | Decision reasons | Tier2 snapshot | Slow calls >10s | Limiter p95/max ms |
| --- | --- | --- | ---: | --- | --- | --- | ---: | --- | --- | --- | --- | --- |
| `aft` | primary | yes | 2,352 | TS 564; Rust 484; JSON 412 | cargo:2; node:6 | 0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | none | n/a/n/a |
| `anthropic-auth` | primary | yes | 203 | TS 140; Markdown 22; JSON 17 | node:4 | 1; 230,300/230,300 | 0 | 0 | corpus drift=1 | 0/n/a/n/a | none | 2,896/3,516 |
| `broca` | primary | yes | 426 | Rust 242; JSON 54; Markdown 41 | cargo:17 | 0; n/a/n/a | 2 | 0 | corpus drift=2 | 0/n/a/n/a | inspect=1 | n/a/n/a |
| `magic-context` | primary | yes | 1,831 | TS 1,267; Markdown 267; JSON 101 | cargo:4; node:7 | 0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | inspect_tier2_run=1 | n/a/n/a |
| `openai-auth` | primary | yes | 134 | TS 98; JSON 8; Markdown 8 | node:2 | 0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | none | n/a/n/a |
| `subconscious` | primary | yes | 553 | Rust 125; Swift 107; JSON 102 | cargo:10 | 0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | none | n/a/n/a |
| unattributed log records | — | — | — | — | — | 1; 49,801/49,801 | 0 | 1 at 40,000/78,811 (50.8%, resolution) | corpus drift=1 | 188 records | — | — |

`pi-mono` and `subconscious` are the only roots with attributable search-index cold-build timing: 274 ms and 415 ms respectively. Ten of the 12 literal search cold-build lines lacked a root/session attribution. Semantic collection is likewise not a cold-build wall clock: the two roots with attributed backend retries were `magic-context` (259 retries, 18 collection records) and `pi-mono` (18 retries, one collection record).

## Per-shape rollup

| Size bucket | Kind | Roots | Roots with cold marker | Reported cold builds | Cold wall p50/max ms | >10s recorded slow calls | Limiter p95/max ms |
| --- | --- | ---: | ---: | ---: | --- | ---: | --- |
| <2k | primary | 46 | 5 | 1 | 230,300/230,300 | 2 | 2,896/3,516 |
| <2k | worktree | 133 | 0 | 0 | n/a/n/a | 92 | 32,769/39,823 |
| 2k-10k | primary | 18 | 1 | 0 | n/a/n/a | 0 | n/a/n/a |
| 2k-10k | worktree | 49 | 0 | 0 | n/a/n/a | 36 | 46,984/49,313 |
| 10k-50k | primary | 2 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| 10k-50k | worktree | 0 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| >50k | primary | 0 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| >50k | worktree | 0 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| unknown (path gone) | primary | 27 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| unknown (path gone) | worktree | 386 | 0 | 0 | n/a/n/a | 45 | 35,272/39,175 |

## Findings

1. **The logs do not show a fleet-wide callgraph cold-start wait.** There are eight cold-build decisions, all `reason=corpus drift`; seven map to six primary roots and one is unassigned. The only two completed-build duration lines are 49.801 s (unassigned) and 230.300 s (`anthropic-auth`, 203 files). Neither reported completion exceeds five minutes. However, there are zero literal start and ready/publish lines, so the logs cannot establish that no root ever took more than five minutes *to a ready callgraph*; they only show that no logged completed duration crossed that threshold.

2. **The visible pathology is the shared `broca` artifact, not a completed cold-start-to-first-result measurement.** Cache key `bcf9718d69e7b23f` resolves to primary `broca`; it has two staged-generation resumes and two `corpus drift` decisions. The only supersession line is an unattributed resolution stop at 40,000/78,811 (50.8%). From the first mapped `broca` drift decision at 13:41Z to this snapshot at 18:41Z, the key has staged-generation activity but no matching ready publication: a roughly five-hour **censored observation interval**, not a measured five-hour build or user wait. Treat it as a resolver/build-state pathology, not evidence that a user waited five hours for one tool result.

3. **Recorded long waits are dominated by search and general worktree contention, not proven callgraph waits.** Of 175 literal `slow tool_call` records over ten seconds, 142 are `search`, 28 `inspect`, three `glob`, one `bash_drain_completions`, and one `inspect_tier2_run`. The slow-call line has total/queue/exec timing but no build identifier or `build_state` cause. Therefore the answer to “how often did agents wait >10 s *because of build state*?” is **not measurable from these logs**; 175 is the upper-bound population of long observed waits, not a causal count. No root has both a paired cold-start-to-first-result interval and enough causal evidence to call that wait dominant.

4. **Admission can be user-visible.** Inspect-triggered cold builds queued 211 times and acquired 208 slots. The recorded acquire wait p95 is 32.769 s and maximum is 49.313 s; the three unmatched queued records may still have been pending, dropped, or completed outside this log snapshot. There were also 12 tier-2 refresh deferrals. This is a separately observable limiter path and should not be folded into build execution time.

5. **Other index planes are visible but weakly attributable.** The corpus has 188 callgraph snapshot lines, 734 tier-2 category lines, and 147 dead-code phase lines, but none carries enough root identity to attribute it safely. It has 551 semantic embedding-backend retry lines and 12 search cold-build lines; the semantic lines show retry/backoff and collection work, not end-to-end embedding duration. Search build timing itself is short in the two attributable cases (274 and 415 ms), while the large search tool waits above show that build duration and user-visible search latency are different measurements.

6. **No breaker evidence was emitted.** `BuildDeathBreaker`, `Suspended`, and `suspension` match count is zero. This means there is no observed breaker/suspension event in this corpus; it does not prove the breaker could not have affected an unlogged request.

## Gaps for the benchmark harness

Instrument the following on every build attempt, with a stable `build_id`, root path, shared cache key, session, and plane (`callgraph`, `tier2`, `semantic`, or `search`):

- A literal `build_started`, stage-progress, `ready_published`, cancelled/superseded, and terminal-failure record. Include wall time and progress totals so start-to-ready and partial work pair without timestamp inference.
- The first query that can use each newly ready plane, including queue delay, service time, result status, and `build_id`. This is the missing cold-start-to-first-queryable/result metric.
- Root and cache key on callgraph snapshots, tier-2 category/phase lines, search-build completion, semantic progress, and direct cold-build duration lines. Current rootless lines make 188 snapshots and the majority of search/semantic records unusable for repo-shape attribution.
- A causal wait field on every tool call: `waiting_on_build_id`, `waiting_on_limiter`, `artifact_load`, `resolver`, or `none`. Without it, the 169 long calls cannot distinguish cold build from unrelated search/queue work.
- Semantic embedding batch progress, embedding service duration, retry/backoff totals, and ready publication. Current collection timing stops before the external embedding plane.
- Search index build start/end and load/borrowed-artifact outcomes tied to the caller. The existing streaming-build duration has no first-queryable or root coverage for ten of twelve lines.
- Resource samples per build: RSS/high-water mark, CPU time, disk read/write bytes, SQLite lock/transaction time, queue depth, and limiter occupancy. The logs cannot explain memory pressure, host saturation, or the cost split inside the resolver.
- Explicit breaker admission, suspension, reset, and recovery events keyed to the same build attempt. A zero text match is not a health measurement.
