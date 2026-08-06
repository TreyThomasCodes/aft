#!/usr/bin/env bash
# Run a verification command BARE (no pipes — a pipe's exit code replaces the
# gated command's and has pushed red builds), then push only on success.
#
# Usage: scripts/gated-push.sh [--remote origin] [--branch main] -- <gate command...>
# Example: scripts/gated-push.sh -- cargo test -p agent-file-tools --lib
set -euo pipefail

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
