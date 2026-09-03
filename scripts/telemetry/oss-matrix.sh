#!/usr/bin/env bash
# Run the standalone OSS cold-build matrix. The Python helper owns NDJSON I/O so
# stdout push frames cannot be confused with command responses in shell.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/oss-matrix.py" "$@"
