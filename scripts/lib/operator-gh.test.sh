#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOLVER="$SCRIPT_DIR/operator-gh.sh"
BASH_BIN="$(command -v bash)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/operator-gh-test.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

SHIM_DIR="$TMP_DIR/fake/cortexkit/aft/shims"
SHIM_TARGET_DIR="$TMP_DIR/fake/cortexkit/bin"
UPSTREAM_DIR="$TMP_DIR/fake/upstream"
mkdir -p "$SHIM_DIR" "$SHIM_TARGET_DIR" "$UPSTREAM_DIR"
printf '#!/usr/bin/env bash\nexit 0\n' > "$SHIM_TARGET_DIR/ck-aft"
chmod +x "$SHIM_TARGET_DIR/ck-aft"
ln -s "$SHIM_TARGET_DIR/ck-aft" "$SHIM_DIR/gh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$UPSTREAM_DIR/gh"
chmod +x "$UPSTREAM_DIR/gh"

SUCCESS_STDERR="$TMP_DIR/success.stderr"
RESOLVED=$(
  PATH="$SHIM_DIR:$UPSTREAM_DIR:/usr/bin:/bin" \
    OPERATOR_GH_FALLBACK_PATHS="$TMP_DIR/no-fallback" \
    "$BASH_BIN" -c 'source "$1"; printf "%s" "$OPERATOR_GH"' _ "$RESOLVER" \
    2>"$SUCCESS_STDERR"
)
[[ "$RESOLVED" == "$UPSTREAM_DIR/gh" ]] || {
  echo "expected upstream gh, got: $RESOLVED" >&2
  exit 1
}
[[ ! -s "$SUCCESS_STDERR" ]] || {
  echo "resolver wrote to stderr on success" >&2
  exit 1
}

FAILURE_STDERR="$TMP_DIR/failure.stderr"
if PATH="$SHIM_DIR:/usr/bin:/bin" \
  OPERATOR_GH_FALLBACK_PATHS="$TMP_DIR/no-fallback" \
  "$BASH_BIN" -c 'source "$1"' _ "$RESOLVER" 2>"$FAILURE_STDERR"; then
  echo "expected resolver to fail when only the shim is available" >&2
  exit 1
fi
[[ "$(cat "$FAILURE_STDERR")" == *"operator-gh: no upstream gh executable found"* ]] || {
  echo "resolver failure did not explain that upstream gh is missing:" >&2
  cat "$FAILURE_STDERR" >&2
  exit 1
}

echo "operator-gh.test.sh: selected upstream gh after skipping shim"
echo "operator-gh.test.sh: reported missing upstream gh without fallback roots"
echo "operator-gh.test.sh: passed"
