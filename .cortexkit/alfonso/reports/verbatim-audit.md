# Verbatim-path audit

Scope: direct `fs::canonicalize` and `std::fs::canonicalize` calls under
`crates/aft/src`, excluding `*_test.rs`, `#[cfg(test)]` modules, and test-only
blocks.  The inventory contains 142 production call expressions.  “Normalized”
below means the verbatim-stripped form produced by
`crate::inspect::job::canonicalize_normalized` / `normalize_path`, not merely
lexical `.`/`..` cleanup.

`n/a` in test coverage means that the row is not a mixed-form comparison.  For
mixed rows, the coverage cell says whether an existing Windows regression test
exercises the two forms (not merely whether ordinary unit tests execute the
function).

## `bash_background/mod.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 116 | SAFE-OPAQUE | `resolve_sandbox_spawn` receives the canonical root as an execution/payload input; this expression does not compare it with a normalized path. | n/a |

## `bash_background/registry.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 339 | SAFE-INTERNAL | `task.artifact_root.join(name)` at `bash_background/registry.rs:356` is built by `canonical_artifact_root` (`:3900`). | n/a |
| 365 | SAFE-INTERNAL | `expected` at `bash_background/registry.rs:372-377` is the same `artifact_root` form as the requested canonical path. | n/a |
| 3900 | SAFE-INTERNAL | Artifact names are joined to this root at `bash_background/registry.rs:356,376`; requests are canonicalized at `:339,365`. | n/a |
| 4485 | SAFE-INTERNAL | Registry freshness/dedup callers use `canonicalized_path` as their shared key helper. | n/a |

## `bash_permissions/mod.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 65 | SAFE-INTERNAL | `is_system_temp_path` compares each canonical temp root with paths resolved by the same `resolve_with_existing_ancestors` family (`bash_permissions/mod.rs:72-94`). | n/a |
| 77 | SAFE-INTERNAL | The result is immediately normalized by `normalize_path` at `bash_permissions/mod.rs:78`; the missing-path branch uses that same helper at `:81`. | n/a |
| 89 | SAFE-INTERNAL | The reconstructed parent result is normalized at `bash_permissions/mod.rs:94` before it reaches containment checks. | n/a |

## `bash_permissions/scan.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 425 | SAFE-INTERNAL | Project root and cwd are both resolved through `resolve_existing` at `bash_permissions/scan.rs:54-55`; path arguments use it at `:374`. | n/a |

## `bash_rewrite/rules.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 243 | SAFE-INTERNAL | `grep_project_root` canonicalizes the configured root, and `should_suppress_grep_footer` canonicalizes it again at `bash_rewrite/rules.rs:255`. | n/a |
| 255 | SAFE-INTERNAL | Canonical target at `bash_rewrite/rules.rs:263` is checked against this same-form canonical root at `:266`. | n/a |
| 263 | SAFE-INTERNAL | Compared only with the root canonicalized at `bash_rewrite/rules.rs:255`. | n/a |

## `backup.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 3170 | SAFE-INTERNAL | The result is immediately sent to `normalize_absolute_key` at `backup.rs:3171`; backup map keys are made by `canonicalize_key`. | n/a |
| 3193 | SAFE-INTERNAL | Existing-ancestor results are immediately normalized at `backup.rs:3197`, matching the `canonicalize_key` map-key helper. | n/a |

## `callgraph.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 842 | SAFE-INTERNAL | `CallGraph::data` removes/looks up both the supplied and this canonical key at `callgraph.rs:823-828`. | n/a |
| 1411 | SAFE-OPAQUE | Returned resolved module path is consumed by the module resolver; it is not compared to a normalized inspect path here. | n/a |
| 1418 | SAFE-OPAQUE | Same `resolve_file_like_path` opaque result path as line 1411. | n/a |
| 2006 | SAFE-INTERNAL | Workspace member roots are produced through `canonicalize_path`, including `callgraph.rs:1999`. | n/a |
| 2094 | SAFE-INTERNAL | Package roots are cached/resolved using canonical workspace roots from `callgraph.rs:2107,2135,2147`. | n/a |
| 2107 | SAFE-INTERNAL | Same workspace-root resolver/cache form as lines 2094 and 2135. | n/a |
| 2135 | SAFE-INTERNAL | `WORKSPACE_PACKAGE_CACHE` key at `callgraph.rs:2136` uses this canonical workspace root. | n/a |
| 2147 | SAFE-INTERNAL | The resolved member is stored in the cache keyed by the canonical root at `callgraph.rs:2149-2150`. | n/a |
| 2472 | SAFE-INTERNAL | Re-export recursion deduplicates `visited` by this canonical key at `callgraph.rs:2473`. | n/a |
| 2665 | SAFE-OPAQUE | Returned index-file path is a resolver result, not an inspect-normalized comparison value. | n/a |

## `callgraph_store/mod.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 10350 | SAFE-INTERNAL | `relative_path` retries with `canonicalize_path(project_root)` and `canonicalize_path(path)` at `callgraph_store/mod.rs:10357-10360`. | n/a |

## `commands/apply_patch.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 108 | MIXED-FORM RISK | Raw canonical `abs` is first used in `abs.strip_prefix(root)` at `commands/apply_patch.rs:113`, where `root` comes from the configured (verbatim-stripped/raw request) root; line 116 is only a later canonical-pair fallback. | None; no Windows test supplies a normalized root and a canonicalized patch path. |
| 116 | SAFE-INTERNAL | Fallback root is canonicalized specifically to pair with the raw canonical `abs` from line 108 at `commands/apply_patch.rs:117-118`. | n/a |

## `commands/bash.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 173 | SAFE-INTERNAL | Sandbox root and candidate cwd are both canonicalized at `commands/bash.rs:173,188`. | n/a |
| 188 | SAFE-INTERNAL | Compared/authorized in the same raw canonical root space established at line 173. | n/a |
| 199 | SAFE-OPAQUE | Shell path is an executable path passed to the sandbox payload, not a normalized path comparison. | n/a |
| 400 | SAFE-INTERNAL | `workdir_matches_project_root` compares `canon(workdir)` and `canon(root)` through this one local helper at `commands/bash.rs:401`. | n/a |

## `commands/call_tree.rs`, `commands/callers.rs`, `commands/impact.rs`, `commands/trace_data.rs`, and `commands/trace_to.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| `call_tree.rs:47,54` | SAFE-INTERNAL | `canonical_input.starts_with(&canonical_root)` at `call_tree.rs:55` uses the two direct canonicalizations. | n/a |
| `callers.rs:49,56` | SAFE-INTERNAL | `canonical_input.starts_with(&canonical_root)` at `callers.rs:57` uses the two direct canonicalizations. | n/a |
| `impact.rs:47,54` | SAFE-INTERNAL | `canonical_input.starts_with(&canonical_root)` at `impact.rs:55` uses the two direct canonicalizations. | n/a |
| `trace_data.rs:77,84` | SAFE-INTERNAL | `canonical_input.starts_with(&canonical_root)` at `trace_data.rs:85` uses the two direct canonicalizations. | n/a |
| `trace_to.rs:47,54` | SAFE-INTERNAL | `canonical_input.starts_with(&canonical_root)` at `trace_to.rs:55` uses the two direct canonicalizations. | n/a |

## `commands/callgraph_store_adapter.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 2157 | SAFE-INTERNAL | `absolute_file` is relativized only against `store.project_root()` at `commands/callgraph_store_adapter.rs:2143-2146`, which is the callgraph-store canonical form. | n/a |

## `commands/configure.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 271 | SAFE-OPAQUE | Resolved home directory is used for configuration discovery, not compared with an inspect-normalized path. | n/a |
| 787, 788 | SAFE-INTERNAL | Git directory and common directory are both canonicalized before comparison at `commands/configure.rs:789`. | n/a |
| 885 | SAFE-OPAQUE | The resolved LSP extra directory is validated (`is_dir`) and stored as an executable search directory, with no normalized-path comparison. | n/a |
| 1794 | IDENTITY-BOUNDARY | Feeds `canonical_cache_root` (`commands/configure.rs:1793-1806`), which drives project scope/artifact identity; do not align this independently of ProjectRootId consumers. | n/a |

## `commands/glob.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 60 | SAFE-INTERNAL | `resolve_path_or_multi` receives this root at `commands/glob.rs:61-68` and delegates scope resolution to the search-index canonical-key family. | n/a |

## `commands/inspect.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 313, 314 | SAFE-INTERNAL | Debug deadline predicate compares the two direct canonical values at `commands/inspect.rs:315`. | n/a |
| 485 | SAFE-INTERNAL | `JobScope::from_roots` de-verbatims the result with `inspect/job.rs:173-180` before scope hashing/comparison. | n/a |

## LSP command handlers

| line | bucket | partner | test-coverage |
|---|---|---|---|
| `commands/lsp_find_references.rs:90` | SAFE-INTERNAL | `build_text_document_position` receives the path at `:101-103`; URI conversion normalizes it in `lsp/position.rs:153-155`. | n/a |
| `commands/lsp_goto_definition.rs:75` | SAFE-INTERNAL | `build_text_document_position` at `:86-88` routes through `lsp/position.rs:153-155`. | n/a |
| `commands/lsp_hover.rs:71` | SAFE-INTERNAL | `build_text_document_position` at `:82-84` routes through `lsp/position.rs:153-155`. | n/a |
| `commands/lsp_inspect.rs:233` | SAFE-INTERNAL | Manager lookup re-normalizes at `lsp/manager.rs:1601-1603`; server selection delegates to normalized LSP roots. | n/a |
| `commands/lsp_prepare_rename.rs:76` | SAFE-INTERNAL | `build_text_document_position` at `:87-89` routes through `lsp/position.rs:153-155`. | n/a |
| `commands/lsp_rename.rs:115` | SAFE-INTERNAL | `build_text_document_position` at `:126-128` routes through `lsp/position.rs:153-155`. | n/a |

## `commands/move_symbol.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 90, 92, 95 | SAFE-INTERNAL | Source/destination equality at `commands/move_symbol.rs:100-103` uses paths canonicalized by this same local sequence. | n/a |
| 361 | SAFE-INTERNAL | Fallback project root is canonicalized to the callgraph store’s root form before it is returned at `commands/move_symbol.rs:357-370`. | n/a |
| 1101 | SAFE-INTERNAL | Walker candidates are compared only with `source_path` and `dest_path`, both canonicalized at `commands/move_symbol.rs:90-101`. | n/a |

## `commands/multi_path.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 92 | SAFE-INTERNAL | `canonical_key` is the common key producer for multi-path dedup/nesting checks at `commands/multi_path.rs:75-86`. | n/a |

## `commands/outline.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 543 | SAFE-INTERNAL | The gitignore root is paired with a canonical candidate from line 880 at `commands/outline.rs:881-888`. | n/a |
| 622, 623 | SAFE-INTERNAL | Fallback relativization compares the two direct canonical values at `commands/outline.rs:624-627`. | n/a |
| 880 | SAFE-INTERNAL | Candidate is compared with the canonical gitignore root from line 543 at `commands/outline.rs:881-888`. | n/a |

## `commands/trace_to_symbol.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 196 | SAFE-INTERNAL | Both candidate and target are converted with `canonicalize_for_compare` before equality at `commands/trace_to_symbol.rs:191-193`. | n/a |
| 208, 215 | SAFE-INTERNAL | `canonical_input.starts_with(&canonical_root)` at `commands/trace_to_symbol.rs:216` uses the two direct canonicalizations. | n/a |

## `context.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 69, 81, 98 | SAFE-INTERNAL | `canonicalize_lenient` retains one filesystem-canonical form for its own existing-component reconstruction at `context.rs:67-120`. | n/a |
| 880 | SAFE-INTERNAL | `resolve_with_existing_ancestors` returns this canonical base plus missing tail; containment later compares against the similarly canonical `resolved_root` at `context.rs:941`. | n/a |
| 987 | SAFE-INTERNAL | `canonical_target.starts_with(resolved_root)` at `context.rs:989` compares canonical target/root forms. | n/a |
| 1015, 1021 | SAFE-INTERNAL | `database_path_key` uses its canonical file/parent result as the process-global database key, with both existing and missing paths following this helper. | n/a |
| 2227 | SAFE-INTERNAL | `GitignoreBuilder` root is paired with watcher paths canonicalized by `watcher_filter::canonicalize_watcher_path`; matcher checks occur at `watcher_filter.rs:232-235`. | n/a |
| 2780 | SAFE-INTERNAL | The requested root is compared with the resolved Git root at `context.rs:2795`; both are filesystem-canonical roots from the readonly-artifact resolver. | n/a |
| 2809, 2844 | IDENTITY-BOUNDARY | Each canonical root is passed to `memoized_artifact_cache_key` at `context.rs:2810,2845`; artifact key identity is a separate alignment domain. | n/a |
| 3426 | IDENTITY-BOUNDARY | Fallback callgraph project root feeds callgraph-store/project scope identity via `callgraph_project_root` at `context.rs:3419-3427`; do not change independently. | n/a |
| 5827 | SAFE-INTERNAL | Path restriction uses this canonical root with canonical/resolved targets at `context.rs:5946-5949`. | n/a |
| 5880 | SAFE-INTERNAL | Canonical parent is rebuilt and lexically normalized at `context.rs:5887` before its comparison with the same root form at `:5889`. | n/a |
| 5931 | SAFE-INTERNAL | Canonical target is checked against `resolved_root`, both produced by the restriction resolver, at `context.rs:5946`. | n/a |

## `grep_executor.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 87 | SAFE-INTERNAL | `ResolvedRoot` construction consumes this root together with search-index scope resolution, whose root key is canonicalized in `search_index.rs:3761`. | n/a |

## `inspect/cache.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 1773 | SAFE-INTERNAL | Resolver config paths are canonical-file paths from `walk_resolver_config_files` (`inspect/cache.rs:1867-1874`) and are relativized against this canonical root at `:1776-1779`. | n/a |
| 1941, 1942, 1947 | SAFE-INTERNAL | Project boundary, config directory, and ancestor are all canonicalized in `node_modules_resolver_config_dependencies` before `starts_with` at `inspect/cache.rs:1943-1949`. | n/a |
| 2037 | SAFE-OPAQUE | Canonical config file is read/hashed; resolver dependency collection stores this raw canonical file form. | n/a |
| 2045 | MIXED-FORM RISK | Raw `manifest_root` is used by `manifest.strip_prefix(&manifest_root)` at `inspect/cache.rs:2048-2050`, but `manifest` comes from `collect_entry_point_manifests`, whose `snapshot_path` normalizes at `inspect/entry_points.rs:126-129`. | None; no Windows regression covers a normalized snapshot manifest against this raw root. |
| 2068 (project root), 2068 (path) | SAFE-INTERNAL | Fallback `relative_string` canonicalizes both operands together and strips at `inspect/cache.rs:2067-2071`. | n/a |

## `inspect/job.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 746 | SAFE-INTERNAL | The raw result never escapes: it is immediately de-verbatimed by `normalize_path` at `inspect/job.rs:747`, matching the fallback at `:748`. | n/a |

## `inspect/manager.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 2053, 2054 | SAFE-INTERNAL | Debug reuse-delay predicate compares the two direct canonical paths at `inspect/manager.rs:2055`. | n/a |

## `inspect/oxc_engine/graph.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 1262, 1263 | SAFE-INTERNAL | The fallback relative-path branch canonicalizes root and path together before `strip_prefix` at `inspect/oxc_engine/graph.rs:1261-1266`. | n/a |

## `inspect/oxc_engine/mod.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 215, 259 | SAFE-INTERNAL | Both roots are immediately passed through the OXC `normalize_path` at `inspect/oxc_engine/mod.rs:216,260`. | n/a |
| 366 | SAFE-INTERNAL | `normalize_input_path` immediately de-verbatims its canonical branch at `inspect/oxc_engine/mod.rs:366-374`. | n/a |
| 386, 403 | SAFE-INTERNAL | Canonical variants are inserted only after `normalize_path` at `inspect/oxc_engine/mod.rs:386-390,403-405`. | n/a |

## `inspect/oxc_engine/resolver.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 139 | SAFE-INTERNAL | Canonical alias is inserted into `path_to_id` only after OXC `normalize_path` at `inspect/oxc_engine/resolver.rs:139-141`. | n/a |
| 395 | SAFE-INTERNAL | Canonical fallback is normalized before the map lookup at `inspect/oxc_engine/resolver.rs:395-398`. | n/a |

## `inspect/scanners/unused_exports.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 1509 (project root), 1509 (path) | SAFE-INTERNAL | Fallback `relative_string` canonicalizes both operands and normalizes both before `strip_prefix` at `inspect/scanners/unused_exports.rs:1508-1514`. | n/a |

## `lsp/manager.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 1700 | MIXED-FORM RISK | `canonical_parent.join(name)` is compared with `normalized` from `normalize_lookup_path` at `lsp/manager.rs:1692,1701-1703`; the latter strips Windows verbatim prefixes. | Existing deleted-file test at `lsp/manager.rs:2289` uses raw canonical test keys, so none covers this Windows mixed-form case. |
| 1722 | MIXED-FORM RISK | Reconstructed raw canonical path is compared against `candidates`, which already contains `normalized` from `normalize_lookup_path`, at `lsp/manager.rs:1715-1728`. | None; no Windows stale-diagnostics regression inserts a normalized key then delivers a deleted-file event. |
| 2082 | SAFE-INTERNAL | `canonicalize_for_lsp` immediately calls `inspect::job::normalize_path` at `lsp/manager.rs:2082-2084`. | n/a |
| 2090, 2107 | SAFE-INTERNAL | `resolve_for_lsp_uri` normalizes each canonical result at `lsp/manager.rs:2090-2113`. | n/a |
| 2134 | SAFE-INTERNAL | `normalize_lookup_path` immediately normalizes the canonical branch at `lsp/manager.rs:2134-2136`. | n/a |

## `lsp/position.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 153 | SAFE-INTERNAL | URI lookup path immediately routes its canonical result through `inspect::job::normalize_path` at `lsp/position.rs:153-155`. | n/a |

## `lsp_hints.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 100, 101 | SAFE-INTERNAL | `paths_match` compares the two direct canonical paths with each other at `lsp_hints.rs:99-104`. | n/a |

## `parser.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 7481 | SAFE-INTERNAL | Re-export recursion uses the canonical path solely as the `visited` key at `parser.rs:7481-7483`. | n/a |

## `readonly_artifacts.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 288, 316 | IDENTITY-BOUNDARY | Existing-parent and Git-top-level canonical roots feed the readonly artifact/root-resolution path later used by `memoized_artifact_cache_key` (`context.rs:2809-2845`); retain path-identity policy. | n/a |

## `root_cache.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 229 | IDENTITY-BOUNDARY | Canonical root feeds `project_scope_key` and `configured_artifact_access` map identity at `root_cache.rs:135-152`; do not independently normalize identity values. | n/a |
| 249, 275 | SAFE-INTERNAL | Writer-lease acquisition counts insert/read the same canonical project-root key at `root_cache.rs:249-256,275-284`. | n/a |
| 734, 752 | IDENTITY-BOUNDARY | Process lease directory becomes the `ProcessLeaseKey` used to coordinate `fs_lock` acquisition at `root_cache.rs:306-322`; do not alter independently. | n/a |

## `sandbox_spawn.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 2369 | SAFE-INTERNAL | Test-seam observations insert and remove with the same `observation_key` helper at `sandbox_spawn.rs:2345-2369`. | n/a |

## `search_index.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 712, 715 | SAFE-INTERNAL | Fallback index root and its ignore fingerprint share direct canonical root production at `search_index.rs:712-720`. | n/a |
| 1359 | SAFE-INTERNAL | Refresh baseline replaces `project_root` with this canonical path, and all subsequent fingerprint/walk operations use that field at `search_index.rs:1359-1369`. | n/a |
| 2523 | SAFE-INTERNAL | Streaming build walks and fingerprints from this canonical `project_root` at `search_index.rs:2523-2527`. | n/a |
| 2839 | SAFE-INTERNAL | Canonical file is retried against `plan.project_root`, whose entries are constructed by the same index build canonical-root path, at `search_index.rs:2837-2842`. | n/a |
| 3606 | MIXED-FORM RISK | Raw `canonical_path.starts_with(&normalized_root)` at `search_index.rs:3606-3609` compares with the lexical/possibly verbatim-stripped root from `:3603-3604`; line 3612 is a later raw-canonical recovery check. | Existing `cached_path_under_root` tests at `search_index.rs:5037` use a raw canonical root, so none covers the Windows normalized-root case. |
| 3612 | SAFE-INTERNAL | Recovery compares `canonical_path` and `canonical_root`, both direct canonical values, at `search_index.rs:3612-3615`. | n/a |
| 4431 | MIXED-FORM RISK | `canonicalize_or_normalize` returns raw canonical paths on success but lexical paths on failure; search scope then meets stored file paths in `is_within_search_root` at `search_index.rs:420-423,1659-1678,1855-1865`. Its artifact-key caller at `:3901` remains an identity subflow. | None; existing scope tests use canonical paths consistently and do not cover Windows verbatim/non-verbatim transition. |
| 4472, 4483 | SAFE-INTERNAL | Existing/deleted watcher paths are reconstructed from the same raw canonical parent form in `canonicalize_existing_or_deleted_path` at `search_index.rs:4471-4485`. | n/a |

## `semantic_index.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 1422 | SAFE-OPAQUE | Resolved ONNX runtime path is inspected only for its file name/version at `semantic_index.rs:1421-1429`. | n/a |
| 4253, 4264 | SAFE-INTERNAL | Invalidation adds the original and this existing/deleted canonical key together before map removal at `semantic_index.rs:3128-3141`. | n/a |

## `watcher_filter.rs`

| line | bucket | partner | test-coverage |
|---|---|---|---|
| 179 | SAFE-INTERNAL | `watcher_same_path` compares watcher paths with this canonical target; watcher intake uses the same canonical family at `watcher_filter.rs:441-447`. | n/a |
| 209, 216 | SAFE-INTERNAL | `canonicalize_watcher_path` is the sole watcher set-key producer at `watcher_filter.rs:292-296,441-447`; project matcher/root paths are rebuilt from canonical watcher roots. | n/a |

# Mixed-form risk summary

Priority is production impact first.  These are the only `MIXED-FORM RISK`
rows above; identity-boundary rows are intentionally omitted.

1. **`inspect/cache.rs:2045` — inspect manifest-cache fingerprint.** A raw
   root strips a normalized snapshot manifest at `inspect/cache.rs:2048-2050`.
   This can hash absolute paths rather than root-relative paths on Windows;
   no Windows regression test covers it.
2. **`search_index.rs:4431` — search scope/index membership.** The helper
   alternates raw canonical and lexical fallback forms, then scopes indexed
   file paths at `search_index.rs:420-423,1659-1678,1855-1865`.  No Windows
   verbatim-transition regression test exists.  Its artifact-key caller is an
   identity-boundary consumer and must stay aligned with that policy.
3. **`commands/apply_patch.rs:108` — patch response relative paths.** Raw
   canonical patch paths first meet configured root form at
   `commands/apply_patch.rs:113`; the later raw-canonical fallback does not
   eliminate the cross-form first branch.  No Windows regression test exists.
4. **`lsp/manager.rs:1700` — deleted-file diagnostic eviction.** A raw
   reconstructed parent path is compared with the normalized diagnostics key
   at `lsp/manager.rs:1692,1701-1703`.  The existing test uses raw keys only.
5. **`lsp/manager.rs:1722` — watcher stale-diagnostics fan-out.** The same
   raw reconstructed form is deduplicated against normalized candidates at
   `lsp/manager.rs:1715-1728`; no Windows regression test covers this route.
6. **`search_index.rs:3606` — cached-path containment guard.** A raw
   canonical path first meets lexical/possibly verbatim-stripped root at
   `search_index.rs:3603-3609`; a same-form recovery exists at `:3612-3615`,
   but the initial mixed comparison remains untested on Windows.
