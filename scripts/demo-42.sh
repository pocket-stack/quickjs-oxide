#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

cargo run --quiet --example eval -- '(function (a) { return a + 1; })(41)'
