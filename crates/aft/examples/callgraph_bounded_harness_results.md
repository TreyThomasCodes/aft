# Callgraph bounded-build RSS measurements

Measured on 2026-08-22 with the committed `callgraph_bounded_harness`. File
discovery is streamed into `staging_file_inventory` in 256-row transactions,
pass 1 scans that table in bounded file/byte batches, and pass 2 resolves each
active caller through SQLite rather than loading the corpus-wide symbol/export
map. Each invocation creates a fresh synthetic non-git corpus and launches a
fresh measurement child.

| Corpus | Cold peak RSS | Absolute cap | Per-phase cold peak RSS (bytes) | Warm / cold wall-clock |
| --- | ---: | ---: | --- | ---: |
| 20,000 files | 94,912,512 B (90.5 MiB) | pass (`<= 1.0 GiB`) | enumeration 14,843,904; extraction 31,539,200; symbol/export index 41,009,152; resolution 94,912,512; publication 87,998,464 | 0.9701 (pass) |
| 40,000 files | 99,926,016 B (95.3 MiB) | pass (`<= 1.0 GiB`) | enumeration 15,466,496; extraction 34,160,640; symbol/export index 48,152,576; resolution 99,926,016; publication 89,276,416 | 0.9516 (pass) |

The 40k/20k full-peak ratio is **1.0528**, passing the pre-registered `<= 1.3x`
scaling cap. Both warm runs are below the `<= 1.10` wall-clock ratio cap.

## Enumeration attribution

The earlier 226.7/348.0 MB values did not represent a live file inventory. The
sampler remained active for the warm rebuild and reused the same phase labels,
so allocator pages retained after the cold resolver were charged to the warm
rebuild's next `enumeration` phase. A macOS `vmmap`/`heap` capture at that
boundary found only 10.0 MiB allocated while 152.0 MiB of `MALLOC_SMALL` regions
were empty. The corpus-wide `ProjectIndex` built during resolution had created
those regions; the caller inventory, fingerprint clone, and pass-1 normalized
clone were much smaller contributors.

The harness now stops and joins its RSS sampler immediately after the cold
build. Discovery no longer creates any corpus-sized path vector in the measured
child: rows are written to SQLite per 256-file batch, the corpus fingerprint is
computed by an order-independent streaming digest, and later batches are read
back by ordered keyset scans. The remaining in-memory planner is capped at 256
paths and 32 MiB of declared source bytes per extraction batch. Resolution retains at
most the 20,000-reference window plus one caller's disk-backed file index.

Commands:

```sh
cargo run -p agent-file-tools --example callgraph_bounded_harness -- 20000
cargo run -p agent-file-tools --example callgraph_bounded_harness -- 40000
```
