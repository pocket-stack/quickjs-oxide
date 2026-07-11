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

unicode_source=$(dirname -- "$oracle")/libunicode-table.h
if [[ ! -f "$unicode_source" ]]; then
    pinned_oracle=$($script_dir/build-quickjs-oracle.sh)
    unicode_source=$(dirname -- "$pinned_oracle")/libunicode-table.h
fi
generated_unicode=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-ident.XXXXXX")
trap 'rm -f -- "$generated_unicode"' EXIT HUP INT TERM
./scripts/generate-unicode-ident-tables.sh "$unicode_source" "$generated_unicode"
if ! cmp -s "$generated_unicode" src/unicode_ident_tables.rs; then
    echo "error: checked-in Unicode identifier tables do not match the pinned source" >&2
    exit 1
fi
rm -f -- "$generated_unicode"
trap - EXIT HUP INT TERM

cargo fmt --all -- --check
QJS_ORACLE="$oracle" cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
./scripts/check-rust-only.sh
