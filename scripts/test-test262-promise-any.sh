#!/usr/bin/env bash
# Reproduce the R3q complete Promise.any cohort.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export PROMISE_AGGREGATE_COHORT=any
exec "$script_dir/test-test262-promise-all-settled.sh" "$@"
