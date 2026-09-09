#!/usr/bin/env bash
# Keep the existing entry point for CI and local callers.
set -euo pipefail
tool_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
script_dir=$(CDPATH= cd -- "$tool_dir/.." && pwd)
exec python3 "$script_dir/checks/check-oracle-registry.py" "$@"
