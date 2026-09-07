# OpenCode V2 Delta Audit Evidence: 0.0.0-beta-19234

- Harness Owner: OpenCode (OC)
- Record ID: pm_7725ea5f
- Date: 2026-09-07
- Evaluated Range: 0.0.0-beta-19059 -> 0.0.0-beta-19234
- Coherent Package Set: `@opencode-ai/core`, `@opencode-ai/cli`, `@opencode-ai/plugin`, `@opencode-ai/schema` at `0.0.0-beta-19234` (published 2026-09-07T06:26Z)

## Verdict

ZERO delta on every relied-on contract line between 19059 and 19234.

## Relied-on Contract Lines and Citations

1. Permission endpoint: `packages/server/src/handlers/permission.ts` (0 commits)
2. Effect entry precedence: `packages/core/src/plugin/module.ts:14-27,115` unchanged
3. Typed RPC: `packages/core/src/rpc.ts:99-103` unchanged
4. Interruption ordering: `packages/core/src/session/runner/step.ts:140-142` (Fiber.interruptAll before awaitAll and failUnsettledTools) unchanged
5. Built-in replacement: `packages/core/src/tool.ts:181-202` unchanged
6. `path` header keys: `session-ui/src/tools/tool-renderer.tsx:362`, `tui/src/mini/tool.ts:412` unchanged
7. Inert effect dep: declared effect dependency inert on V1 server loader, zero delta
8. SessionID-scoped permission.create: route path sessionID selects owning Permission.Service, zero delta
9. Dev/beta cadence: coherent rolling release cadence, zero delta
