# macOS Gatekeeper assessment of local Rust binaries

## Result

On this Mac, a fresh provenance-bearing Mach-O executable is Gatekeeper/XProtect
assessed on its first execution. The assessment log contains `allowUI is YES`
and `GK eval - was allowed: 0, show prompt: 1`, which is the log-level indication
that the verification UI may be shown; this investigation used those signals,
rather than visual observation of a dialog.

Ad-hoc signing with `--identifier aft-dev-gate` **does not suppress** that first
assessment or its UI request. It gives a stable identifier in the logs. A
controlled warm execution suppresses the assessment for subsequent executions
of that exact worktree binary, so the practical no-sudo fix is to sign and warm
once immediately after Cargo has linked all test executables and `target/debug/aft`.

The assessment cache is not keyed solely by cdhash. Two files with identical
bytes, the same stable identifier, and the same cdhash were both assessed after
being placed at different fresh inodes. Do not expect identical rebuilds in
parallel worktrees to share the assessment result.

## Environment and method

- macOS 26.6.2 (Darwin 25.6.0), arm64
- Fresh executables had `com.apple.provenance`; none had
  `com.apple.quarantine`.
- The probe was a small `clang -O0` Mach-O. Cargo built the real 201,799,128-byte
  debug `target/debug/aft` in this worktree with `cargo build -p agent-file-tools`.
- For each execution, `log stream --style compact --level info --predicate
  'process == "syspolicyd" && subsystem == "com.apple.syspolicy.exec"'` ran before
  the command. `GK performScan`, `GK Xprotect results`, and `GK scan complete`
  delimit an assessment. The binary's path appears in the XProtect result.

A Mach-O emitted by the linker reports an ad-hoc `linker-signed` signature even
before an explicit `codesign` call. Removing that signature with
`codesign --remove-signature` made the arm64 test process fail immediately with
`Invalid argument`, so the "unsigned" row below means the normal fresh,
linker-ad-hoc binary with no explicit developer signing.

## Measured matrix

| Variant | Binary and preparation | Assessment / UI request in `syspolicyd` | Assessment duration | Execution result |
| --- | --- | --- | --- | --- |
| (a) fresh linker-ad-hoc | 33,440-byte probe; no explicit `codesign` | Yes: `allowUI is YES`, XProtect result, `show prompt: 1` | 1.015 s (13:45:31.727–13:45:32.742) | 1.040 s |
| (b) stable ad-hoc before first execution | 51,344-byte probe, `codesign -f -s - --identifier aft-dev-gate` | Yes: XProtect result and `show prompt: 1` | 0.377 s (13:46:15.287–13:46:15.664) | 0.400 s |
| (c) current recipe: sign then immediate throwaway execution | 51,376-byte probe, `codesign -f -s -`, then execute twice | The warm execution was assessed and requested UI; the second execution produced no path-matching scan | 0.636 s for the warm execution (13:46:50.862–13:46:51.498) | warm: 0.668 s; second: 0.004 s |
| (d) same signed contents at a new inode | Copy of (b), re-signed with the same stable identifier; byte-for-byte equal and both CDHash values were `1f3dba83192b6328be22bbd84e83819578b22929` | Yes, again: distinct XProtect result and `show prompt: 1` | 0.256 s (13:46:18.736–13:46:18.992) | 0.290 s |
| Real debug `aft`, stable ad-hoc then warm | Fresh Cargo output; 201,799,128 bytes before signing and 200,643,968 after signing | Yes: XProtect result for this worktree's `target/debug/aft`, identifier `aft-dev-gate`, and `show prompt: 1` | 1.396 s (13:44:52.597–13:44:53.993) | warm: 1.459 s |
| Real debug `aft`, second execution | Same warmed inode | No path-matching `GK performScan` or XProtect result | n/a | 12.394 s while other parallel gates were active; do not use this loaded-host sample as a latency comparison |

The short probe assessments may be too brief to leave a useful human-visible
progress window for every row, but `show prompt: 1` establishes that Gatekeeper
requested one.
The debug binary's scan was four times longer than the small stable probe on
this loaded host, and its duration can grow substantially when several large
binaries are assessed at once.

## No-sudo mitigation

`scripts/rust-test-gate.sh` now uses the following operation for every built test
harness and for the separately spawned product binary before nextest starts:

```bash
codesign -f -s - --identifier aft-dev-gate "$binary"
"$binary" --list   # --version for target/debug/aft
```

The same stable-sign-and-warm operation is used for archive-extracted nextest
binaries and the TypeScript test helper. It does **not** pretend to eliminate
Gatekeeper: it serializes one expected assessment per new binary before the
timed, parallel run. `AFT_RUST_TEST_RUNNER=cargo` now receives the same
pre-warming pass instead of bypassing it.

No Rust integration test invokes `Command::new("cargo")`, so there is no
test-internal `cargo build` path that needs a second hook. A test that begins
building `aft` internally in the future must sign and warm its new output before
spawning it repeatedly.

## Policy options not applied

No `sudo`, System Settings change, `spctl` mutation, or allowlisting command was
run for this investigation.

On this macOS release, `spctl`'s displayed basic usage includes assessment and
`--global-disable`, but does not include the older `spctl --add`/`--label`
per-binary allowlist interface. Current `spctl` documentation marks those rule
database operations as deprecated and unsupported, so do not rely on an
`spctl --add` rule for a standalone CLI binary.

There is a terminal-scoped one-time option that the machine owner may choose:

```bash
sudo /usr/sbin/spctl developer-mode enable-terminal
```

It requires local administrator authentication. Then enable the terminal that
launches the gate in **System Settings → Privacy & Security → Developer Tools**
and restart that terminal. This approval covers that terminal and its child
processes, not a particular binary, Finder, `launchd`, or another launcher. It
is the closest scoped option for local CLI development, but it was **not** run
for this investigation, so the matrix does not claim a measured result for it.
There is no documented `disable-terminal` subcommand; disable the terminal again
in the same Developer Tools settings pane to revert it.

The broad fallback is the system-wide Gatekeeper switch:

```bash
sudo /usr/sbin/spctl --global-disable
# revert:
sudo /usr/sbin/spctl --global-enable
```

That requires administrator authorization and weakens Gatekeeper for the whole
machine. It is deliberately left to the machine owner; the gate does not run it.
Re-enable it immediately after any owner-authorized diagnostic use.
