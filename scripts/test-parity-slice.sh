#!/usr/bin/env bash
# Reproduce every gate for the currently implemented feature-parity slice.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
cd -- "$root"

oracle=${QJS_ORACLE:-}
if [[ -z "$oracle" ]]; then
    oracle=$($script_dir/build-quickjs-oracle.sh)
fi
if [[ ! -x "$oracle" ]]; then
    echo "error: QJS_ORACLE is not executable: $oracle" >&2
    exit 2
fi

cargo fmt --all -- --check
QJS_ORACLE="$oracle" cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
./scripts/check-rust-only.sh
