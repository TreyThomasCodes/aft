#!/usr/bin/env bash
set -euo pipefail

# The archive gate calls this after extracting reusable build metadata and before
# timed tests start. macOS charges a large first-exec cost per fresh test-binary
# inode, so archive shards must pay that cost here rather than inside test runs.
if [[ "$(uname)" != "Darwin" || -z "${AFT_NEXTEST_EXTRACT_TO:-}" ]]; then
  exit 0
fi

metadata="$AFT_NEXTEST_EXTRACT_TO/target/nextest/binaries-metadata.json"
if [[ ! -f "$metadata" ]]; then
  echo "nextest archive metadata not found after extraction: $metadata" >&2
  exit 1
fi

while IFS= read -r binary; do
  [[ -n "$binary" && -x "$binary" ]] || continue
  codesign -f -s - "$binary" 2>/dev/null || true
  "$binary" --list >/dev/null 2>&1 || true
done < <(
  python3 - "$metadata" "$AFT_NEXTEST_EXTRACT_TO/target" <<'PY'
import json
import os
import sys

metadata_path, extracted_target = sys.argv[1:]
with open(metadata_path, encoding="utf-8") as handle:
    metadata = json.load(handle)

source_target = metadata["rust-build-meta"]["target-directory"]
for binary in metadata["rust-binaries"].values():
    source_path = binary["binary-path"]
    relative_path = os.path.relpath(source_path, source_target)
    print(os.path.join(extracted_target, relative_path))
PY
)

# Integration tests spawn the CLI, which is not a test harness and therefore is
# absent from rust-binaries metadata.
aft_binary="$AFT_NEXTEST_EXTRACT_TO/target/debug/aft"
if [[ -x "$aft_binary" ]]; then
  codesign -f -s - "$aft_binary" 2>/dev/null || true
  "$aft_binary" --version >/dev/null 2>&1 || true
fi
