#!/usr/bin/env bash
set -euo pipefail

# Run the entire gate at reduced scheduling priority so saturated test
# windows cannot starve the supervised ck-* modules into missing health
# probes (three health-kills on 2026-08-08 traced to gate-window load).
# taskpolicy demotes to the utility QoS class on macOS (E-cores under
# contention); nice covers Linux and the priority dimension everywhere.
# Self-demotion only covers load WE generate; the supervisor-side
# threshold fix covers foreign load.
if [ -z "${AFT_GATE_DEMOTED:-}" ]; then
  export AFT_GATE_DEMOTED=1
  # nice-only, deliberately NOT taskpolicy utility: the QoS class pins the
  # whole gate to E-cores, and the integration suite's per-test 60s response
  # deadlines then fail wholesale (three gate runs red while the same suite
  # was green undemoted). nice yields to the normal-priority ck-* modules
  # under contention but keeps P-cores when the machine has headroom, which
  # is the property the demotion exists for.
  exec nice -n 10 "$0" "$@"
fi


runner="${AFT_RUST_TEST_RUNNER:-nextest}"

if [[ "$runner" == "cargo" ]]; then
  exec cargo test --workspace --quiet
fi

if [[ "$runner" != "nextest" ]]; then
  echo "Unsupported AFT_RUST_TEST_RUNNER='$runner' (expected 'nextest' or 'cargo')" >&2
  exit 2
fi

if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "cargo-nextest is required; install it with: cargo install cargo-nextest --locked" >&2
  exit 127
fi

run_phase() {
  local label="$1"
  shift
  local started=$SECONDS

  echo "==> $label"
  "$@"
  echo "    ok ($((SECONDS - started))s)"
}

# `cargo test --workspace -- --list` currently reports zero doctests for both
# workspace crates (`aft` and `aft_tokenizer`), so the split gate omits
# `cargo test --workspace --doc` until doctests actually exist.
#
# CI can split the independent execution phases after one job creates a nextest
# archive. The default remains `all`, preserving the single-machine gate used
# locally and by workflows that do not opt into Windows sharding.
requested_phases="${AFT_GATE_PHASES:-all}"
if [[ "$requested_phases" == "all" ]]; then
  phase_enabled() { return 0; }
else
  IFS=',' read -r -a selected_phases <<< "$requested_phases"
  for selected_phase in "${selected_phases[@]}"; do
    case "$selected_phase" in
      lib|nextest|watcher|storm) ;;
      *)
        echo "Unsupported AFT_GATE_PHASES entry '$selected_phase' (expected lib, nextest, watcher, storm, or all)" >&2
        exit 2
        ;;
    esac
  done

  phase_enabled() {
    local wanted="$1"
    local selected_phase
    for selected_phase in "${selected_phases[@]}"; do
      [[ "$selected_phase" == "$wanted" ]] && return 0
    done
    return 1
  }
fi

if phase_enabled lib; then
  # The platform-verifier TLS test spawns a subprocess whose keychain trust
  # evaluation is unbounded on Macs with third-party root CAs (NordVPN: ~10s
  # quiet, minutes under full-suite load — blew a 600s budget twice on
  # 2026-08-09). Isolation is the fix, not a bigger budget: run it alone
  # first (seconds when serial), then exclude it from the parallel phase.
  run_phase "cargo test -p agent-file-tools --lib platform_verifier_tls_client_subprocess --quiet (serial: keychain-latency-sensitive)" \
    cargo test -p agent-file-tools --lib platform_verifier_tls_client_subprocess --quiet

  run_phase "cargo test --workspace --lib --bins --quiet" \
    cargo test --workspace --lib --bins --quiet -- --skip platform_verifier_tls_client_subprocess
fi

# macOS: the first exec of a freshly-linked binary is expensive, and it is NOT
# Gatekeeper assessment — setting com.apple.quarantine changes nothing, and a
# plain `cat > /dev/null` buys the same speedup as re-signing. Measured on a
# 178 MB debug binary, relinking before each sample: cold 4.2s, after a full
# read 1.14s, after ad-hoc signing 1.1s (indistinguishable from the read),
# second exec of the same inode 0.01s. So the cost is two layers — page-in,
# clearable by any full read, plus a per-inode first-exec cost that ONLY an
# actual exec clears.
#
# nextest exec's EVERY test-harness binary in target/*/deps, and the
# integration tests additionally spawn target/debug/aft. Without warming, that
# first wave of cold execs lands inside the TIMED run and dies together at the
# per-test timeout (the 16-test SIGTERM-at-400s storm). Pay it HERE, untimed.
#
# The EXEC is the load-bearing step; the sign only helps because it reads the
# file. Signing is kept anyway for an unrelated reason: overwriting a signed
# binary invalidates its signature and macOS then SIGKILLs it.
#
# NOTE: an earlier version tried `pkill XprotectService/syspolicyd` on a slow
# probe — a no-op without sudo (both run as root; pkill returns "Operation not
# permitted"), and pointless anyway now the actor is known not to be them.
# Opt out with AFT_GATE_NO_XPROTECT_REMEDIATION=1.
warm_macos_test_binaries() {
  # Ask cargo for the EXACT set of test-harness executables it built (the
  # `executable` field in the build JSON — ~24 binaries, not the thousands of
  # incremental fragments under deps/). Sign each, then exec `--list` once,
  # which pays both layers without running any test. $@ = the cargo build args
  # that define the profile/scope.
  local bins
  bins="$(cargo test "$@" --no-run --message-format=json 2>/dev/null | python3 -c "
import sys, json
seen = set()
for line in sys.stdin:
    try: o = json.loads(line)
    except Exception: continue
    e = o.get('executable')
    if e: seen.add(e)
for p in sorted(seen): print(p)
")"
  local bin
  while IFS= read -r bin; do
    [[ -n "$bin" && -x "$bin" ]] || continue
    codesign -f -s - "$bin" 2>/dev/null || true
    "$bin" --list >/dev/null 2>&1 || true
  done <<< "$bins"
  # The CLI binary is spawned as a subprocess by integration tests but is not
  # a test harness, so it never appears as an `executable`; warm it explicitly.
  if [[ -x target/debug/aft ]]; then
    codesign -f -s - target/debug/aft 2>/dev/null || true
    target/debug/aft --version >/dev/null 2>&1 || true
  fi
}
if phase_enabled nextest && [[ "$(uname)" == "Darwin" && "${AFT_GATE_NO_XPROTECT_REMEDIATION:-}" != "1" ]]; then
  run_phase "warm macOS first-exec cost: sign + exec every debug test binary" \
    bash -c "$(declare -f warm_macos_test_binaries)
      warm_macos_test_binaries --workspace"
fi

if phase_enabled nextest; then
  nextest_args=(cargo nextest run)
  if [[ -n "${AFT_NEXTEST_ARCHIVE_FILE:-}" ]]; then
    nextest_label="cargo nextest run --archive-file $AFT_NEXTEST_ARCHIVE_FILE -E kind(test) - binary(=watcher_integration)"
    nextest_args+=(--archive-file "$AFT_NEXTEST_ARCHIVE_FILE")
  else
    nextest_label="cargo nextest run --workspace -E kind(test) - binary(=watcher_integration)"
    nextest_args+=(--workspace)
  fi
  nextest_args+=(-E 'kind(test) - binary(=watcher_integration)')
  if [[ -n "${AFT_NEXTEST_PARTITION:-}" ]]; then
    nextest_label+=" --partition $AFT_NEXTEST_PARTITION"
    nextest_args+=(--partition "$AFT_NEXTEST_PARTITION")
  fi
  run_phase "$nextest_label" "${nextest_args[@]}"
fi

if phase_enabled watcher; then
  run_phase "cargo test -p agent-file-tools --test watcher_integration --quiet -- --test-threads=1" \
    cargo test -p agent-file-tools --test watcher_integration --quiet -- --test-threads=1
fi

# The main subc storm test asserts production-calibrated absolute latencies
# (2s bind headroom, the module's real 12s bind deadline). It is
# debug-ignored because an unoptimized build under load cannot honor those
# bounds even when the code is correct; the release profile is the
# authoritative calibration (measured ~14s for the whole storm suite).
# Skippable because the 2-core Windows CI runner can neither afford the
# cold release-profile build inside the job timeout nor honor absolute
# latency bounds — Linux and macOS CI remain the release-storm arbiters.
if phase_enabled storm; then
  if [[ "${AFT_GATE_SKIP_RELEASE_STORM:-}" == "1" ]]; then
    echo "==> release-storm phase skipped (AFT_GATE_SKIP_RELEASE_STORM=1)"
  else
    run_phase "cargo nextest run --cargo-profile release -E 'test(subc_storm)' (release-calibrated latency bounds)" \
      cargo nextest run --cargo-profile release -p agent-file-tools --test integration -E 'test(subc_storm)'
  fi
fi
