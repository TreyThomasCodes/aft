# Zoom refusal steering census (2026-08-30)

## Telemetry method

This census queried `~/.local/share/opencode/opencode.db` at **2026-08-30 09:04:48 UTC**, covering the preceding 14 days.

Before aggregating, the query inspected known-good live rows. A successful `aft_zoom` row has `data.type = "tool"`, `data.tool = "aft_zoom"`, request arguments in `data.state.input`, and outcome in `data.state.status`; a failed row has `data.state.status = "error"` and its text in `data.state.error`. The census filters those verified fields rather than an absent `is_error` field, so an empty result cannot be mistaken for a clean population.

| Outcome | Calls |
| --- | ---: |
| `error` | 601 |
| `completed` | 9,947 |
| `running` at snapshot | 2 |

### Error/refusal classes

| Refusal class | Error calls |
| --- | ---: |
| Symbol not found | 492 |
| File not found | 46 |
| Ambiguous symbol/path | 0 |
| Container member menu | 0 |
| Other | 63 |
| **Total** | **601** |

`other` is primarily malformed invocation (`zoom: missing required param 'symbol'`, 9 calls) and transient bridge/route failures, not a stable zoom refusal.

### Unchanged-argument retries

A retry chain groups rows by session and the exact JSON value of `data.state.input`. A row is a retry only when it follows that pair's first error by `(time_created, id)`, avoiding a similarly shaped request from another session being counted as a retry.

| Measure | Count |
| --- | ---: |
| Exact `(aft_zoom, input)` pairs retried after an error | 3 |
| Pairs retried two or more times after an error | 0 |
| Total unchanged calls after an error | 3 |
| Longest retry tail | 1 |

The local corpus has the same unchanged-argument retry shape as the BROCA finding, but at lower severity in this window: three one-retry chains and no chain with two or more retries. The leading first-error classes are symbol-not-found, file-not-found, and a document-heading miss.

## Steering change

The success response remains lean. Only deterministic non-body outcomes receive the retry warning and a next action.

| Class | Before | After |
| --- | --- | --- |
| Symbol miss | `symbol 'comput' not found, did you mean: [compute]` | `symbol 'comput' not found. Retrying this exact zoom call will fail again. Choose one of these names from the file outline: \`compute\` (lines 7-10).` |
| Likely wrong file | `symbol 'entirely_unrelated_lookup' not found` | States the file's symbol count, its closest ranged symbol, and to change `file` or `symbol`. |
| Markdown/HTML heading miss | Normalized heading suggestion without a range | Lists nearest normalized headings using their raw labels and line ranges. |
| Missing file | `file not found: …` | Explains unchanged retry fails and directs the caller to an existing `file` or reachable `url`. |
| Ambiguous symbol/menu | `zoom a qualified name for its body` | Explains unchanged retry cannot choose a body and says to pick one of the listed qualified names. |
| Large container menu | `zoom a member for its body` | Explains unchanged retry cannot return a body and says to pick one of the listed members. |

The nearest-name matcher itself is unchanged. The new presentation layer maps its capped results back to the file or document outline so names retain their ranges; its unit test explicitly asserts the matcher does not mutate its candidate list.
