# subc-client background stream dispatch observability

`@cortexkit/subc-client` 0.5.0 dispatches every non-control frame by the tuple
`(channel, epoch, corr)`. Its source in `src/client.ts` first resolves the live
route at lines 1028-1035 and then looks up the pending request at lines
1038-1040.

Two dispatch failures are silent for `StreamData`:

- A missing route handle or epoch mismatch increments `ingressEpochDropCount`
  and returns. The public `droppedIngressFrames` getter exposes this cumulative
  count. `aft-bridge` samples it while each `bg_events` subscription is open and
  logs any increase with the open subscription's client-side `channel@epoch` as
  observation context; the counter is client-wide and does not attribute the
  dropped frame to that subscription.
- A missing pending key falls through without a counter or callback. Terminal
  frames have a debug-log fallback, but `StreamData` does not.

## Required upstream hook

The client needs a public, cumulative `droppedPendingStreamFrames` counter (or
an equivalent `onDispatchDrop` callback) incremented when all of these are true:

1. route/epoch validation succeeded;
2. `pending.get(pendingKey(handle, frame.header.corr))` returned no entry; and
3. `frame.header.ty === FrameType.StreamData`.

A callback should report `reason: "pending_miss"`, `channel`, `epoch`, and
`corr`; a snapshot getter should expose both `droppedIngressFrames` and
`droppedPendingStreamFrames`. These values let a wrapper distinguish epoch
replacement from correlation loss without inspecting private client maps.

Until that hook is published, AFT avoids a scalar last-subscriber match key on
the module side. It retains every live route for a `(root, session)` and emits a
wake through each route's own `BgSub`, preserving the correlation captured from
that route's subscribe request. Closing the newest client record therefore does
not orphan an older live subscription, and a current record does not depend on
a stale record's correlation.
