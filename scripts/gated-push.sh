#!/usr/bin/env bash
# Run a verification command BARE (no pipes — a pipe's exit code replaces the
# gated command's and has pushed red builds), then push only on success.
#
# Usage: scripts/gated-push.sh [--remote origin] [--branch main] -- <gate command...>
# Example: scripts/gated-push.sh -- cargo test -p agent-file-tools --lib
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
  if command -v taskpolicy >/dev/null 2>&1; then
    exec taskpolicy -c utility nice -n 10 "$0" "$@"
  else
    exec nice -n 10 "$0" "$@"
  fi
fi


# Refuse to gate a tree that is mid-merge/cherry-pick/rebase: a conflicted
# tree can compile stale HEAD state while the gate's pipes mask failures,
# and the final push becomes a no-op "up-to-date" that reads as success.
for marker in CHERRY_PICK_HEAD MERGE_HEAD REBASE_HEAD; do
  if [ -e "$(git rev-parse --git-dir)/$marker" ]; then
    echo "gated-push: refusing — $marker present (unresolved git operation)" >&2
    exit 2
  fi
done
set -o pipefail


remote="origin"
branch="main"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --remote) remote="$2"; shift 2 ;;
    --branch) branch="$2"; shift 2 ;;
    --) shift; break ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "no gate command given" >&2
  exit 2
fi

# Warm the shared fixture binary before any gate that spawns it: a gate chain
# that just rebuilt target/debug/aft mints a fresh inode, and macOS charges a
# first-exec assessment (seconds warm, minutes under a wedged XprotectService)
# to whichever test spawns it first — observed as 49 parallel tests all dying
# at their 60s response deadline while the binary was still being assessed.
# The throwaway exec is the load-bearing step (memory: signing alone does not
# clear it); rust-test-gate.sh does this for test binaries, this covers the
# fixture binary for ad-hoc gate commands.
if [[ -x target/debug/aft ]]; then
  codesign -f -s - target/debug/aft 2>/dev/null || true
  ./target/debug/aft --version >/dev/null 2>&1 || true
fi

echo "gated-push: running gate: $*"
"$@"
rc=$?
if [[ $rc -ne 0 ]]; then
  echo "gated-push: gate FAILED (rc=$rc) — not pushing" >&2
  exit "$rc"
fi

echo "gated-push: gate green — running governed-surface preflight"
# The v0.49 governed manifests byte-pin surface files (path-aliases.ts, the
# .md inventories, tool schemas). Any edit to one of them silently stales the
# manifests, and the unit suite fails at merge time on exactly that. Running
# the audit and release gate here turns that from a CI round-trip into a
# local refusal. See docs/v0.49-agent-surface-manifest.json for the pinned set.
bun scripts/audit-v049-agent-surface.ts
node scripts/release-gate-v049.mjs

# Biome runs in CI's unit suites; a style-only miss costs a full CI roundtrip
# (a template-vs-concat nit killed a push after every Rust gate was green).
for pkg in packages/aft-bridge packages/opencode-plugin packages/pi-plugin packages/aft-cli; do
  if [[ -d "$pkg/src" ]]; then
    (cd "$pkg" && bunx biome check src)
  fi
done

echo "gated-push: preflight green — pushing to $remote $branch"
git push "$remote" "$branch"

# Outcome check, not just command check: a push can report success through a
# wrapper (or fail on auth) while origin never moved. Non-empty @{u}..HEAD
# after a "successful" push is the tell that survives any false green.
git fetch -q "$remote" "$branch"
unpushed=$(git rev-list --count "$remote/$branch".."$branch")
if [[ "$unpushed" -ne 0 ]]; then
  echo "gated-push: push reported success but $unpushed commit(s) not on $remote/$branch — origin did not move" >&2
  exit 1
fi
