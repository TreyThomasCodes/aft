# AFT memory census contract

`memory.census` is a passive management operation. It is not an agent tool. In
subc mode it is carried on channel 0 and reads the health worker's published
snapshot; the reply does not enumerate processes or walk allocator statistics.

The response is:

```json
{
  "roots": {
    "/worktree": {
      "root": "/worktree",
      "root_id": "/worktree",
      "bound_routes": 0,
      "last_request_age_ms": 0,
      "idle_ttl_ms": 1800000,
      "lsp_idle_ttl_ms": 600000,
      "evictable_in_ms": 1800000,
      "planes": {"search": 0, "semantic": 0, "symbols": 0, "callgraph": 0, "inspect": 0},
      "attributed_bytes": 0,
      "evictable_bytes": 0,
      "lsp_children": {"count": 0, "rss_bytes": 0}
    }
  },
  "process": {
    "phys_footprint_bytes": null,
    "rss_bytes": 0,
    "allocator_slack_bytes": 0,
    "allocator_slack_label": "reclaimable by relief",
    "sqlite_bytes": 0,
    "total_attributed_bytes": 0,
    "unattributed_bytes": 0,
    "last_relief_at_ms": null,
    "last_relief_freed_bytes": 0
  }
}
```

`roots` is never capped. `evictable_in_ms` is null while a root has a bound
route, and otherwise is the configured root idle TTL minus its request age
(clamped at zero). `evictable_bytes` is the artifact, symbol, and inspect data
released by the idle reaper, not a second estimate. `allocator_slack_bytes` is
an overlapping allocator envelope and is labelled as reclaimable by relief;
`unattributed_bytes` is footprint (or RSS when footprint is unavailable) minus
attributed bytes minus allocator slack.

`aft profile --memory` renders this contract when supplied a daemon census. In
standalone mode it reports that no shared process exists to attribute rather
than fabricating a census. The `ck health aft --memory` client can delegate to
this operation and render the same payload.
