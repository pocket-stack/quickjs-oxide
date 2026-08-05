#!/usr/bin/env bash
# Stable entry point for the latest checksum-bound global Test262 admission.

set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec "$script_dir/test-test262-array-flatten-global.sh" "$@"
