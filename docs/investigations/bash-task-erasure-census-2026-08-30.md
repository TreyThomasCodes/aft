# Background bash task delivery and visibility investigation (2026-08-30)

## Corrected specimen

The original claim that `bash-8f0c2baa8a36ef34` had been erased was false. The SQLite probe selected a nonexistent `created_at` column (the real column is `started_at`) while stderr was suppressed, so an SQL error was mistaken for an empty result.

Correct queries show:

- `bash_tasks` is intact: harness `opencode`, session `ses_331acff95fferWZOYF1pG0cjOn`, status `failed`, exit code 1, started 19:23:52Z, completed 19:25:26Z (94 seconds).
- `bash_pattern_watches` is intact with `pending_match=1`, `scanning=0`, `match_text='(fail)'`, and `match_offset=756237`.
- The watchdog found and durably recorded the match, but no notification reached the session.
- A later query found the task bundle absent from `opencode/bash-tasks/a5ddf3872780000a/`, matching eventual one-hour `cleanup_finished` bundle-only retirement. There is no quarantine entry. This does not prove the bundle was already absent during the earlier status failure.
- The daemon did not restart in the incident window, and its log has no line naming this task.
- About 15 minutes after terminal completion, the owning session's `bash_status` returned `background task not found` even though the database row remained intact.

The incident is therefore an intact-row **delivery and visibility failure**, not erasure. The false-premise query is a methodology warning: never treat suppressed SQLite output as an empty set without checking the command's exit status and schema.

## Real mechanism

### Delivery arm

The terminal path performs a final output scan before completion routing. A match is written to `bash_pattern_watches` by `persist_watch_match`, then emitted as a reliable `BashPatternMatch` push. Terminalization sees watch control, marks the task completion delivered, preserves the pending durable watch until ack, and clears the in-memory watch state.

The subc wake lane had an asymmetry: `BashCompleted` reliable pushes armed the repeating `bg_events` nudge, but `BashPatternMatch` pushes did not. If the one direct pattern push missed the owning route during a bind/re-key window, the durable pending row had no repeating wake to force a drain. A drain also returned only the in-memory completion queue; it did not re-emit pending durable watch rows. Normal operation had no retry seam for that row; eventual `cleanup_finished` retirement removed even the in-memory task and artifact bundle. The pending match could consequently sit forever despite an alive session and intact database.

The fix gives pattern matches the same repeating subc wake as completions and makes every session completion drain re-emit pending durable watch matches directly from SQLite. The nudge remains armed until normal completion ack removes the durable watch row, so direct-push loss, route rebinding, and cleanup cannot strand delivery.

### Visibility arm

`bash_status` first checked the in-memory registry. The surviving evidence does not determine why this task was absent from the active registry only about 15 minutes after completion; a root/registry rebind remains plausible, while one-hour cleanup was not yet eligible. Once the in-memory lookup missed, project replay deliberately selected only nonterminal or undelivered rows. This task was terminal and already marked delivered by watch-controlled completion, so replay skipped its intact row. Relaxed DB lookup could find the row, but still required a bundle under the currently selected storage/session path to reconstruct an in-memory task. That artifact dependency made an intact authoritative row insufficient, and eventual bundle cleanup guaranteed the failure would persist.

The fix reads an owning session's intact terminal DB row into a metadata-only status snapshot when no bundle exists under the selected store. Cross-session same-project lookup has the same terminal-row fallback. Existing but corrupt bundles remain refused rather than being hidden by DB metadata. Missing artifacts now mean empty/unavailable output, not an invisible terminal task. The prior in-memory miss cannot be convicted from the retained specimen, but the terminal-row visibility failure after that miss is deterministic in the old lookup chain.

## Deletion and retirement census

| Site | What it removes | Gate / ownership check | Relevance to corrected specimen |
| --- | --- | --- | --- |
| `db::bash_tasks::delete_delivered_terminal_bash_task` (only caller: persisted GC) | One `bash_tasks` row | Exact harness + session + task identity; delivered; terminal status | Did not fire: the row is intact. Attribution logging remains useful hardening. |
| `BgTaskRegistry::maybe_gc_persisted` | Task bundle, then eligible task row | JSON mtime older than 24 hours; terminal and delivered; persisted and DB PID/PGID liveness both false | Did not fire on this 94-second task. Its former watch-row deletion behavior remains hardened with tombstones. |
| `BgTaskRegistry::cleanup_finished` | In-memory registry entry and task bundle only | In-memory terminal and delivered; `terminal_at` older than one hour | Matches the later absent bundle plus intact DB rows, but was not yet eligible at the 15-minute status observation. It made the already-stranded durable match permanent. |
| Restart replay and watchdog `fate_unknown` transition | No bundle or row directly; may mark metadata terminal | No exit marker and recorded process appears dead | Not involved: there was no daemon restart and the persisted task completed normally as failed. |
| Root reclaim (`kill_running_tasks_for_root`) | Process group; persists terminal state | Confirmed root absence plus quiesced/unbound lifecycle gates; canonical task root match | No evidence it fired; it does not delete rows or bundles directly. |
| Replay/lookup/GC quarantine | Moves invalid, mismatched, or corrupt bundles | Refuses when persisted or DB process identity is live | No quarantine entry exists; not involved. |
| Spawn setup cleanup | Newly allocated or partial bundle | Spawn refusal/failure before successful registration | Cannot select a task that ran for 94 seconds and completed normally. |
| `persistence::delete_resolved_task` | Flat bundle files or one directory tree | Valid task ID, layout match, pinned directory identity | Primitive used by cleanup; it has path-identity checks but lifecycle policy belongs to callers. |
| Quarantine GC | Old quarantine entries | Entry mtime older than 30 days | Not involved. |
| Tests | Fixture bundles and rows | `tempfile`/per-process `AFT_CACHE_DIR`; gate unsets ambient `AFT_STORAGE_DIR` | Sandboxed from the shared store under repository gate conventions. |

## Retained hardening

The earlier tombstone work remains valid defense in depth: if a subject row really vanishes, the watchdog terminalizes its durable watch as `watch target erased`, emits through the normal notification channel, and `bash_status` distinguishes `task_erased` from a never-issued ID. Successful production row deletion also emits a once-per-row warning with task ID and reason.

## Controlled specimen

This task runs in an isolated source worktree and does not inspect or mutate the operator's shared `aft.db`; the live controlled specimen remains undisturbed.
