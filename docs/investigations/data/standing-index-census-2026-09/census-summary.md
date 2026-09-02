# Standing-index log census

## Inputs and method

- Log files: 8 (`aft-*.log`), streamed line by line.
- Cache-key roots: 661. Selected roots: 661.
- Log timestamps: 2026-08-30T05:55:18+00:00 to 2026-09-02T18:45:52+00:00.
- `cold_wall_*` uses paired literal start/publish events when present; if none pair, it uses the daemon's direct `perf ... cold_build ... ms=N` duration and the CSV exposes both counts.
- Session attribution accepts only sessions observed with exactly one root. Events without an in-line root, an unambiguous session binding, or a uniquely resolvable cache key remain unassigned.
- Percentiles use nearest-rank (`ceil(p*n)`) over the recorded log samples.

## Pattern coverage

| Pattern family | Matches |
| --- | ---: |
| `cold_build_decision` | 8 |
| `cold_build_resume` | 2 |
| `cold_build_superseded` | 1 |
| `cold_build_start` | 0 |
| `cold_build_publish_or_ready` | 0 |
| `cold_build_reported_duration` | 2 |
| `tier2_callgraph_snapshot` | 194 |
| `tier2_category` | 749 |
| `tier2_dead_code_phases` | 150 |
| `semantic_collect_duration` | 243 |
| `semantic_collect_phases` | 48 |
| `semantic_embed_retry` | 551 |
| `search_index_cold_build` | 12 |
| `slow_tool_call` | 2355 |
| `limiter_queued` | 211 |
| `limiter_slot_acquired` | 208 |
| `tier2_refresh_deferred` | 12 |
| `breaker_or_suspension` | 0 |

## Per-root table

The complete one-row-per-root table is `census-roots.csv`; this compact table retains every root with a standing-index or recorded slow-call event. `cold` is `reported/pairs; p50/max ms`; snapshot is `n/p50/max ms`; limiter is `n/p95/max ms`.

| Repo | Kind | Git? | Files | Languages | Workspace | Cold | Resumes | Supersessions | Decisions | Tier2 snapshot | Slow calls p50/p95 by tool | Limiter |
| --- | --- | --- | ---: | --- | --- | --- | ---: | --- | --- | --- | --- | --- |
| synapse (`bg_cf99840838a84af2`) | worktree | yes | 793 | .rs Rust=177; .json JSON=135; .md Markdown=113 | cargo:22 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=20,p50=132ms,p95=172ms; edit:n=4,p50=60ms,p95=78ms; grep:n=3,p50=110ms,p95=148ms; inspect:n=3,p50=1772ms,p95=86363ms; read:n=1,p50=96ms,p95=96ms; safety:n=2,p50=58ms,p95=65ms; search:n=29,p50=1095ms,p95=2362ms | 9/654/654 |
| broca (`bg_09c2692a4c850d8c`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=3,p50=130ms,p95=185ms; glob:n=2,p50=10002ms,p95=10045ms; grep:n=4,p50=3310ms,p95=3791ms; read:n=1,p50=283ms,p95=283ms; search:n=10,p50=4676ms,p95=26954ms; zoom:n=2,p50=52ms,p95=128ms | 0/n/a/n/a |
| broca (`bg_f91e3af6445fe366`) | worktree | yes | 426 | .rs Rust=242; .json JSON=54; .md Markdown=41 | cargo:17 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=9,p50=58ms,p95=2389ms; edit:n=2,p50=159ms,p95=162ms; inspect:n=2,p50=23456ms,p95=44904ms; read:n=1,p50=588ms,p95=588ms; safety:n=1,p50=84ms,p95=84ms; search:n=10,p50=4377ms,p95=5577ms; write:n=1,p50=114ms,p95=114ms | 9/39823/39823 |
| broca (`bg_fd6f9f94c155c94f`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=7,p50=121ms,p95=266ms; glob:n=1,p50=10433ms,p95=10433ms; inspect:n=1,p50=72206ms,p95=72206ms; outline:n=2,p50=52ms,p95=1909ms; search:n=15,p50=3390ms,p95=10869ms; zoom:n=7,p50=78ms,p95=189ms | 5/39175/39175 |
| aft (`bg_02363a7f10492583`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | inspect_tier2_run:n=1,p50=4814ms,p95=4814ms | 0/n/a/n/a |
| aft (`bg_599f1bcb776a3429`) | worktree | yes | 2352 | .ts TypeScript=564; .rs Rust=484; .json JSON=412 | cargo:2; node:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=10,p50=1366ms,p95=2955ms; edit:n=6,p50=170ms,p95=236ms; glob:n=1,p50=64ms,p95=64ms; inspect:n=3,p50=11878ms,p95=150030ms; outline:n=1,p50=425ms,p95=425ms; search:n=21,p50=11242ms,p95=18462ms; zoom:n=7,p50=1686ms,p95=4857ms | 9/2909/2909 |
| aft (`bg_798bea5fbadfa800`) | worktree | yes | 2352 | .ts TypeScript=564; .rs Rust=484; .json JSON=412 | cargo:2; node:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=2,p50=89ms,p95=99ms; outline:n=2,p50=69ms,p95=71ms; search:n=12,p50=6277ms,p95=11837ms | 0/n/a/n/a |
| aft (`bg_ac864965e2ba2525`) | worktree | yes | 2352 | .ts TypeScript=564; .rs Rust=484; .json JSON=412 | cargo:2; node:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=12,p50=102ms,p95=7173ms; edit:n=13,p50=83ms,p95=128ms; grep:n=4,p50=166ms,p95=195ms; inspect:n=3,p50=3547ms,p95=53682ms; read:n=2,p50=6943ms,p95=7169ms; search:n=4,p50=1882ms,p95=5362ms | 9/3862/3862 |
| aft (`bg_b97c541a04acdbff`) | worktree | yes | 2356 | .ts TypeScript=564; .rs Rust=484; .json JSON=412 | cargo:2; node:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=7,p50=128ms,p95=428ms; edit:n=2,p50=128ms,p95=319ms; inspect:n=2,p50=53330ms,p95=139149ms; search:n=16,p50=18250ms,p95=69353ms | 4/49313/49313 |
| magic-context (`bg_0e4a2a3c44f353b4`) | worktree | yes | 1830 | .ts TypeScript=1267; .md Markdown=266; .json JSON=101 | cargo:4; node:7 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=10,p50=62ms,p95=11686ms; edit:n=9,p50=79ms,p95=126ms; glob:n=3,p50=124ms,p95=131ms; grep:n=26,p50=103ms,p95=419ms; inspect:n=1,p50=77064ms,p95=77064ms; read:n=1,p50=328ms,p95=328ms; search:n=47,p50=11141ms,p95=15929ms | 4/27171/27171 |
| magic-context (`bg_1d190f2f615098f5`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | ast_replace:n=1,p50=269ms,p95=269ms; bash_drain_completions:n=7,p50=69ms,p95=866ms; edit:n=6,p50=63ms,p95=96ms; glob:n=1,p50=52ms,p95=52ms; inspect:n=2,p50=2898ms,p95=31262ms; outline:n=1,p50=137ms,p95=137ms; search:n=26,p50=92ms,p95=2501ms; zoom:n=1,p50=90ms,p95=90ms | 6/2288/2288 |
| magic-context (`bg_266b0f48de6bb601`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=5,p50=118ms,p95=267ms; edit:n=4,p50=60ms,p95=93ms; glob:n=7,p50=449ms,p95=890ms; inspect:n=2,p50=6135ms,p95=60807ms; outline:n=3,p50=279ms,p95=1378ms; read:n=5,p50=3820ms,p95=3830ms; search:n=27,p50=22906ms,p95=49087ms; write:n=2,p50=97ms,p95=115ms | 9/17509/17509 |
| magic-context (`bg_32630da8d1959daa`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=6,p50=114ms,p95=118ms; edit:n=6,p50=56ms,p95=81ms; glob:n=3,p50=95ms,p95=97ms; grep:n=10,p50=95ms,p95=515ms; inspect:n=1,p50=35925ms,p95=35925ms; read:n=1,p50=966ms,p95=966ms; search:n=2,p50=104ms,p95=117ms | 3/4156/4156 |
| magic-context (`bg_35e8ab2bff4bc10b`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | ast_replace:n=5,p50=166ms,p95=367ms; ast_search:n=3,p50=160ms,p95=177ms; bash_drain_completions:n=11,p50=2819ms,p95=6435ms; edit:n=17,p50=182ms,p95=247ms; glob:n=3,p50=96ms,p95=152ms; grep:n=21,p50=87ms,p95=277ms; inspect:n=4,p50=4006ms,p95=56785ms; outline:n=3,p50=167ms,p95=202ms; read:n=3,p50=89ms,p95=145ms; search:n=11,p50=6678ms,p95=14652ms; zoom:n=3,p50=196ms,p95=240ms | 12/3877/3877 |
| magic-context (`bg_4b51c3bc90a95b6c`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=3,p50=117ms,p95=118ms; edit:n=7,p50=60ms,p95=166ms; grep:n=7,p50=91ms,p95=185ms; inspect:n=4,p50=5264ms,p95=31221ms; outline:n=1,p50=167ms,p95=167ms; search:n=28,p50=1405ms,p95=3442ms; zoom:n=1,p50=95ms,p95=95ms | 15/4063/4063 |
| magic-context (`bg_65bc9abb5f80a128`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=9,p50=166ms,p95=3699ms; edit:n=6,p50=110ms,p95=457ms; glob:n=2,p50=71ms,p95=77ms; inspect:n=4,p50=4526ms,p95=36777ms; outline:n=1,p50=157ms,p95=157ms; search:n=3,p50=121ms,p95=4098ms; write:n=1,p50=50ms,p95=50ms | 13/9749/9749 |
| magic-context (`bg_94029bf368b23028`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=8,p50=115ms,p95=5845ms; grep:n=2,p50=76ms,p95=133ms; inspect:n=2,p50=3217ms,p95=62419ms; read:n=1,p50=740ms,p95=740ms; search:n=6,p50=1630ms,p95=7704ms | 6/11400/11400 |
| magic-context (`bg_9b3291a9b86e8a75`) | worktree | yes | 1830 | .ts TypeScript=1267; .md Markdown=266; .json JSON=101 | cargo:4; node:7 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | ast_search:n=3,p50=291ms,p95=1907ms; bash_drain_completions:n=8,p50=61ms,p95=7164ms; edit:n=1,p50=70ms,p95=70ms; glob:n=7,p50=271ms,p95=2076ms; grep:n=9,p50=186ms,p95=375ms; inspect:n=1,p50=87248ms,p95=87248ms; outline:n=1,p50=1906ms,p95=1906ms; read:n=2,p50=1661ms,p95=1661ms; search:n=10,p50=4785ms,p95=29244ms; write:n=1,p50=83ms,p95=83ms; zoom:n=2,p50=334ms,p95=4570ms | 4/30032/30032 |
| magic-context (`bg_d8c65bf7d6e607c9`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=6,p50=178ms,p95=4577ms; edit:n=9,p50=197ms,p95=521ms; glob:n=2,p50=95ms,p95=117ms; grep:n=9,p50=71ms,p95=98ms; inspect:n=3,p50=7838ms,p95=48846ms; outline:n=2,p50=71ms,p95=1042ms; read:n=1,p50=80ms,p95=80ms; search:n=8,p50=2608ms,p95=5357ms; zoom:n=5,p50=93ms,p95=138ms | 9/5622/5622 |
| magic-context (`bg_d9a1d0ec867febdf`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=29,p50=114ms,p95=176ms; edit:n=3,p50=62ms,p95=82ms; glob:n=2,p50=69ms,p95=71ms; grep:n=6,p50=89ms,p95=109ms; import:n=1,p50=52ms,p95=52ms; inspect:n=3,p50=3387ms,p95=31797ms; outline:n=1,p50=328ms,p95=328ms; search:n=8,p50=106ms,p95=535ms | 9/2801/2801 |
| magic-context (`bg_e4bd6cd12c18e497`) | worktree | yes | 1831 | .ts TypeScript=1267; .md Markdown=267; .json JSON=101 | cargo:4; node:7 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=11,p50=153ms,p95=3386ms; edit:n=25,p50=93ms,p95=322ms; grep:n=8,p50=109ms,p95=675ms; inspect:n=1,p50=97501ms,p95=97501ms; outline:n=3,p50=81ms,p95=663ms; read:n=5,p50=109ms,p95=519ms; search:n=17,p50=9840ms,p95=23340ms; zoom:n=4,p50=77ms,p95=263ms | 4/22434/22434 |
| magic-context (`bg_f2c15f72e2af9c64`) | worktree | yes | n/a | n/a | n/a | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=12,p50=82ms,p95=3644ms; edit:n=3,p50=79ms,p95=113ms; glob:n=2,p50=85ms,p95=102ms; grep:n=16,p50=84ms,p95=147ms; inspect:n=2,p50=5931ms,p95=27249ms; outline:n=1,p50=140ms,p95=140ms; search:n=4,p50=96ms,p95=240ms; zoom:n=1,p50=69ms,p95=69ms | 6/3747/3747 |
| prefrontal (`bg_20f7aafb46c715f6`) | worktree | yes | 1891 | .ts TypeScript=708; .rs Rust=457; .md Markdown=260 | cargo:8 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=4,p50=63ms,p95=90ms; edit:n=16,p50=118ms,p95=906ms; grep:n=3,p50=279ms,p95=279ms; inspect:n=2,p50=17982ms,p95=36221ms; outline:n=4,p50=201ms,p95=1803ms; read:n=1,p50=149ms,p95=149ms; search:n=27,p50=7166ms,p95=56374ms; write:n=1,p50=77ms,p95=77ms; zoom:n=1,p50=459ms,p95=459ms | 8/15470/15470 |
| prefrontal (`bg_3a64ccf16c2021a4`) | worktree | yes | 1891 | .ts TypeScript=708; .rs Rust=457; .md Markdown=260 | cargo:8 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=3,p50=96ms,p95=146ms; edit:n=21,p50=95ms,p95=325ms; inspect:n=2,p50=23395ms,p95=125102ms; outline:n=1,p50=219ms,p95=219ms; read:n=2,p50=52ms,p95=54ms; search:n=32,p50=10986ms,p95=38797ms | 4/18590/18590 |
| prefrontal (`bg_ba3a41614246a9c5`) | worktree | yes | 1892 | .ts TypeScript=708; .rs Rust=457; .md Markdown=261 | cargo:8 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=9,p50=248ms,p95=283ms; bash_wait_detach:n=1,p50=57ms,p95=57ms; edit:n=2,p50=56ms,p95=86ms; outline:n=2,p50=121ms,p95=205ms; read:n=3,p50=687ms,p95=816ms; search:n=11,p50=9422ms,p95=24863ms; write:n=1,p50=67ms,p95=67ms | 0/n/a/n/a |
| prefrontal (`bg_e09d8a62d788de20`) | worktree | yes | 1891 | .ts TypeScript=708; .rs Rust=457; .md Markdown=260 | cargo:8 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=4,p50=183ms,p95=220ms; grep:n=2,p50=123ms,p95=519ms; outline:n=1,p50=80ms,p95=80ms; read:n=1,p50=89ms,p95=89ms; search:n=21,p50=11011ms,p95=23678ms; zoom:n=5,p50=181ms,p95=3551ms | 0/n/a/n/a |
| pi-mono (`pi-mono`) | primary | yes | 1415 | .ts TypeScript=1174; .md Markdown=97; .json JSON=47 | node:15 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=9,p50=67ms,p95=5670ms; glob:n=2,p50=3834ms,p95=8726ms; grep:n=4,p50=1609ms,p95=1631ms; inspect_tier2_run:n=1,p50=463ms,p95=463ms; outline:n=1,p50=581ms,p95=581ms; read:n=2,p50=1232ms,p95=1382ms; search:n=2,p50=52ms,p95=91ms | 0/n/a/n/a |
| aft (`aft`) | primary | yes | 2352 | .ts TypeScript=564; .rs Rust=484; .json JSON=412 | cargo:2; node:6 | 0/0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | bash_ack_completions:n=1,p50=236ms,p95=236ms; bash_drain_completions:n=140,p50=84ms,p95=447ms; edit:n=3,p50=106ms,p95=200ms; grep:n=4,p50=91ms,p95=200ms; inspect_tier2_run:n=1,p50=2082ms,p95=2082ms; search:n=1,p50=206ms,p95=206ms | 0/n/a/n/a |
| alfonso-ios (`alfonso-ios`) | primary | yes | 331 | .swift Swift=290; .md Markdown=10; .sh Shell=10 | none | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=3,p50=61ms,p95=188ms | 0/n/a/n/a |
| alfonso-tui (`alfonso-tui`) | primary | yes | 44 | .rs Rust=18; .md Markdown=13; .sh Shell=6 | none | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=2,p50=54ms,p95=58ms; inspect_tier2_run:n=1,p50=8960ms,p95=8960ms | 0/n/a/n/a |
| anthropic-auth (`anthropic-auth`) | primary | yes | 203 | .ts TypeScript=140; .md Markdown=22; .json JSON=17 | node:4 | 1/0; 230300/230300 | 0 | 0 | corpus drift=1 | 0/n/a/n/a | ast_search:n=1,p50=104ms,p95=104ms; bash_drain_completions:n=85,p50=95ms,p95=230ms; callgraph:n=1,p50=76ms,p95=76ms; checkpoint_paths:n=1,p50=92ms,p95=92ms; conflicts:n=11,p50=459ms,p95=663ms; edit:n=116,p50=81ms,p95=354ms; grep:n=31,p50=114ms,p95=379ms; import:n=2,p50=95ms,p95=99ms; inspect:n=18,p50=1736ms,p95=6956ms; outline:n=42,p50=168ms,p95=942ms; read:n=30,p50=94ms,p95=3480ms; safety:n=2,p50=53ms,p95=65ms; search:n=85,p50=246ms,p95=846ms; status:n=2,p50=58ms,p95=81ms; write:n=2,p50=60ms,p95=67ms; zoom:n=20,p50=95ms,p95=368ms | 47/2896/3516 |
| antigravity-auth (`antigravity-auth`) | primary | yes | 338 | .ts TypeScript=262; .json JSON=26; .md Markdown=16 | node:4 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=6,p50=141ms,p95=211ms; bash_kill:n=6,p50=79ms,p95=97ms; callgraph:n=1,p50=722ms,p95=722ms; edit:n=15,p50=92ms,p95=216ms; grep:n=7,p50=112ms,p95=121ms; inspect:n=2,p50=947ms,p95=3943ms; outline:n=1,p50=159ms,p95=159ms; read:n=4,p50=187ms,p95=188ms; search:n=11,p50=890ms,p95=2357ms; status:n=2,p50=56ms,p95=84ms; zoom:n=5,p50=186ms,p95=299ms | 4/1254/1254 |
| astrocyte (`astrocyte`) | primary | yes | 81 | .rs Rust=32; .json JSON=21; .md Markdown=18 | cargo:2 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=10,p50=64ms,p95=299ms; search:n=13,p50=207ms,p95=607ms | 0/n/a/n/a |
| avatar (`avatar`) | primary | yes | 342 | .svg svg=97; .json JSON=69; .rs Rust=56 | cargo:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=7,p50=58ms,p95=82ms; edit:n=1,p50=81ms,p95=81ms; inspect_tier2_run:n=1,p50=8686ms,p95=8686ms | 0/n/a/n/a |
| broca (`broca`) | primary | yes | 426 | .rs Rust=242; .json JSON=54; .md Markdown=41 | cargo:17 | 0/0; n/a/n/a | 2 | 0 | corpus drift=2 | 0/n/a/n/a | bash_drain_completions:n=9,p50=76ms,p95=413ms; bash_status:n=4,p50=55ms,p95=71ms; callgraph:n=1,p50=96ms,p95=96ms; inspect:n=2,p50=1248ms,p95=14967ms; search:n=12,p50=272ms,p95=890ms | 0/n/a/n/a |
| cerebellum (`cerebellum`) | primary | yes | 195 | .rs Rust=154; .json JSON=15; .sh Shell=8 | cargo:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=15,p50=56ms,p95=224ms; edit:n=5,p50=146ms,p95=160ms; search:n=64,p50=199ms,p95=339ms | 0/n/a/n/a |
| claustrum (`claustrum`) | primary | yes | 113 | .rs Rust=66; .md Markdown=14; .json JSON=6 | cargo:2 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=25,p50=211ms,p95=389ms; bash_status:n=24,p50=132ms,p95=261ms; bash_wait_detach:n=1,p50=66ms,p95=66ms; edit:n=7,p50=60ms,p95=286ms; read:n=3,p50=621ms,p95=2262ms; search:n=29,p50=185ms,p95=289ms | 0/n/a/n/a |
| cortexkit-e2e (`cortexkit-e2e`) | primary | yes | 610 | .json JSON=376; .ts TypeScript=156; .md Markdown=25 | cargo:1 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=10,p50=66ms,p95=755ms; bash_wait_detach:n=1,p50=157ms,p95=157ms; glob:n=2,p50=78ms,p95=167ms; grep:n=3,p50=86ms,p95=121ms; search:n=2,p50=322ms,p95=605ms | 0/n/a/n/a |
| engram (`engram`) | primary | yes | 236 | .md Markdown=122; .rs Rust=85; .json JSON=8 | cargo:4 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=2,p50=54ms,p95=58ms; inspect_tier2_run:n=1,p50=9007ms,p95=9007ms | 0/n/a/n/a |
| fusiform (`fusiform`) | primary | yes | 91 | .rs Rust=59; .md Markdown=10; .toml TOML=7 | cargo:6 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=2,p50=54ms,p95=58ms | 0/n/a/n/a |
| insula (`insula`) | primary | yes | 129 | .rs Rust=82; .py Python=18; .md Markdown=15 | cargo:2 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=8,p50=170ms,p95=299ms; edit:n=9,p50=58ms,p95=160ms; read:n=4,p50=118ms,p95=2811ms; search:n=23,p50=267ms,p95=581ms; zoom:n=1,p50=97ms,p95=97ms | 0/n/a/n/a |
| magic-context (`magic-context`) | primary | yes | 1831 | .ts TypeScript=1267; .md Markdown=267; .json JSON=101 | cargo:4; node:7 | 0/0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | bash_drain_completions:n=15,p50=69ms,p95=868ms; bash_wait_detach:n=1,p50=909ms,p95=909ms; callgraph:n=3,p50=156ms,p95=413ms; edit:n=4,p50=59ms,p95=117ms; grep:n=2,p50=59ms,p95=66ms; inspect_tier2_run:n=1,p50=10579ms,p95=10579ms; read:n=6,p50=689ms,p95=1806ms; search:n=20,p50=1005ms,p95=1606ms; write:n=1,p50=66ms,p95=66ms; zoom:n=1,p50=82ms,p95=82ms | 0/n/a/n/a |
| openai-auth (`openai-auth`) | primary | yes | 134 | .ts TypeScript=98; .json JSON=8; .md Markdown=8 | node:2 | 0/0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | bash_drain_completions:n=3,p50=66ms,p95=66ms; bash_wait_detach:n=1,p50=157ms,p95=157ms; edit:n=3,p50=60ms,p95=63ms; import:n=1,p50=66ms,p95=66ms; write:n=1,p50=162ms,p95=162ms | 0/n/a/n/a |
| plexus (`plexus`) | primary | yes | 137 | .rs Rust=66; .jsonc jsonc=22; .md Markdown=16 | cargo:4 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=13,p50=58ms,p95=474ms; search:n=1,p50=1508ms,p95=1508ms | 0/n/a/n/a |
| prefrontal (`prefrontal`) | primary | yes | 1892 | .ts TypeScript=708; .rs Rust=457; .md Markdown=261 | cargo:8 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=34,p50=65ms,p95=371ms; glob:n=2,p50=50ms,p95=315ms; grep:n=5,p50=107ms,p95=142ms; inspect_tier2_run:n=2,p50=195ms,p95=6182ms; outline:n=1,p50=315ms,p95=315ms; read:n=2,p50=63ms,p95=224ms; search:n=11,p50=308ms,p95=1566ms | 0/n/a/n/a |
| subconscious (`subconscious`) | primary | yes | 553 | .rs Rust=125; .swift Swift=107; .json JSON=102 | cargo:10 | 0/0; n/a/n/a | 0 | 0 | corpus drift=1 | 0/n/a/n/a | bash_ack_completions:n=2,p50=132ms,p95=132ms; bash_drain_completions:n=24,p50=62ms,p95=208ms; edit:n=5,p50=53ms,p95=70ms; search:n=35,p50=208ms,p95=402ms | 0/n/a/n/a |
| synapse (`synapse`) | primary | yes | 793 | .rs Rust=177; .json JSON=135; .md Markdown=113 | cargo:22 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=26,p50=76ms,p95=154ms; bash_status:n=1,p50=270ms,p95=270ms; bash_wait_detach:n=1,p50=72ms,p95=72ms; edit:n=5,p50=53ms,p95=476ms; search:n=2,p50=131ms,p95=181ms | 0/n/a/n/a |
| wernicke (`wernicke`) | primary | yes | 56 | .rs Rust=26; .md Markdown=14; .sh Shell=6 | cargo:1 | 0/0; n/a/n/a | 0 | 0 | none | 0/n/a/n/a | bash_drain_completions:n=9,p50=66ms,p95=182ms; search:n=1,p50=838ms,p95=838ms | 0/n/a/n/a |

## Per-shape rollup

| Size bucket | Kind | Roots | Roots with cold marker | Reported cold builds | Cold wall p50/max ms | >10s recorded slow calls | Limiter wait p95/max ms |
| --- | --- | ---: | ---: | ---: | --- | ---: | --- |
| <2k | primary | 46 | 5 | 1 | 230300/230300 | 2 | 2896/3516 |
| <2k | worktree | 133 | 0 | 0 | n/a/n/a | 92 | 32769/39823 |
| 2k-10k | primary | 18 | 1 | 0 | n/a/n/a | 0 | n/a/n/a |
| 2k-10k | worktree | 49 | 0 | 0 | n/a/n/a | 36 | 46984/49313 |
| 10k-50k | primary | 2 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| 10k-50k | worktree | 0 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| >50k | primary | 0 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| >50k | worktree | 0 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| unknown | primary | 27 | 0 | 0 | n/a/n/a | 0 | n/a/n/a |
| unknown | worktree | 386 | 0 | 0 | n/a/n/a | 45 | 35272/39175 |

## Attribution gaps

| Family | Unassigned matches | Why |
| --- | ---: | --- |
| `cold_build_decision` | 1 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `cold_build_resume` | 0 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `cold_build_superseded` | 1 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `cold_build_reported_duration` | 1 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `tier2_callgraph_snapshot` | 194 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `tier2_category` | 749 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `tier2_dead_code_phases` | 150 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `semantic_collect_duration` | 105 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `semantic_collect_phases` | 31 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `semantic_embed_retry` | 274 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `search_index_cold_build` | 10 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `slow_tool_call` | 2 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `limiter_queued` | 0 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `limiter_slot_acquired` | 0 | no in-line root, uniquely bound session, or uniquely resolvable key |
| `tier2_refresh_deferred` | 3 | no in-line root, uniquely bound session, or uniquely resolvable key |

## Recorded >10s tool calls

175 `slow tool_call` records exceeded 10 seconds: `bash_drain_completions`=1, `glob`=3, `inspect`=28, `inspect_tier2_run`=1, `search`=142. The log shape records timing but no causal `build_state` field, so this is an observed wait count, not a claim that every wait was caused by cold building.
Inspect limiter: 211 queued, 208 acquired; p95=32769ms, max=49313ms.
