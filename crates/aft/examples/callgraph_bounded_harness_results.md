# Callgraph bounded-build RSS measurements

Measured on 2026-08-21 with the committed `callgraph_bounded_harness` after
applying the 20,000-reference resolve window and SQLite `cache_size=-8192 KiB`
pin. Each invocation creates a fresh synthetic non-git corpus and launches a
fresh measurement child.

| Corpus | Cold peak RSS | 20k cap | Per-phase peak RSS (bytes) | Warm / cold wall-clock |
| --- | ---: | ---: | --- | ---: |
| 20,000 files | 259,293,184 B (247.3 MiB) | pass (`<= 1.0 GiB`) | enumeration 226,738,176; extraction 227,885,056; symbol/export index 228,229,120; resolution 259,293,184; publication 259,293,184 | 0.9721 |
| 40,000 files | 348,667,904 B (332.5 MiB) | pass (`<= 1.0 GiB`) | enumeration 348,028,928; extraction 348,667,904; symbol/export index 294,387,712; resolution 347,422,720; publication 348,012,544 | 0.9871 |

The 40k/20k full-peak ratio is **1.3447**. The absolute peak and warm ratio
pass their individual pre-registered caps, but this ratio exceeds the 1.3x
scaling cap and must remain visible for follow-up rather than being rounded or
reported as an acceptance pass.

Commands:

```sh
cargo run -p agent-file-tools --example callgraph_bounded_harness -- 20000
cargo run -p agent-file-tools --example callgraph_bounded_harness -- 40000
```
