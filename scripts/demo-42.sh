#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

cargo run --quiet --bin qjs -- --print-result -e '(function (a) { return a + 1; })(41)'
