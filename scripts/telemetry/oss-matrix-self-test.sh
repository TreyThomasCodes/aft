#!/usr/bin/env bash
# Exercise the tmignore-rs five-minute measurement and short-budget control.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$script_dir/oss-matrix.sh" --self-test "$@"
