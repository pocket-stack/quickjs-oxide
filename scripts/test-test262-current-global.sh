#!/usr/bin/env bash
# Stable entry point for the latest checksum-bound Test262 milestone.

set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec "$script_dir/test-test262-error-regexp-typedarray-global.sh" "$@"
