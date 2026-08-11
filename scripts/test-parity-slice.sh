#!/usr/bin/env bash
# Reproduce the broad aggregate gate for the implemented feature-parity slice.
# Additional focused milestone gates remain independently reproducible.

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
unicode_root=$(dirname -- "$unicode_source")
for unicode_file in libunicode.c libunicode.h cutils.c cutils.h; do
    if [[ ! -f "$unicode_root/$unicode_file" ]]; then
        pinned_oracle=$($script_dir/build-quickjs-oracle.sh)
        unicode_root=$(dirname -- "$pinned_oracle")
        unicode_source=$unicode_root/libunicode-table.h
        break
    fi
done
generated_ident=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-ident.XXXXXX")
generated_case=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-case.XXXXXX")
generated_property=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-property.XXXXXX")
generated_normalize=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-normalize.XXXXXX")
trap 'rm -f -- "$generated_ident" "$generated_case" "$generated_property" "$generated_normalize"' EXIT HUP INT TERM
./scripts/generate-unicode-ident-tables.sh "$unicode_source" "$generated_ident"
if ! cmp -s "$generated_ident" src/unicode_ident_tables.rs; then
    echo "error: checked-in Unicode identifier tables do not match the pinned source" >&2
    exit 1
fi
./scripts/generate-unicode-case-tables.sh "$unicode_source" "$generated_case"
if ! cmp -s "$generated_case" src/unicode_case_tables.rs; then
    echo "error: checked-in Unicode case tables do not match the pinned source" >&2
    exit 1
fi
./scripts/generate-unicode-property-tables.sh "$unicode_root" "$generated_property"
if ! cmp -s "$generated_property" src/unicode_property_tables.rs; then
    echo "error: checked-in Unicode property tables do not match the pinned source" >&2
    exit 1
fi
./scripts/generate-unicode-normalize-tables.py "$unicode_source" "$generated_normalize"
if ! cmp -s "$generated_normalize" src/unicode_normalize_tables.rs; then
    echo "error: checked-in Unicode normalization tables do not match the pinned source" >&2
    exit 1
fi
./scripts/check-unicode-normalize-fingerprint.sh "$unicode_root"
rm -f -- "$generated_ident" "$generated_case" "$generated_property" "$generated_normalize"
trap - EXIT HUP INT TERM

cargo fmt --all -- --check
QJS_ORACLE="$oracle" cargo test --locked --workspace --all-targets
QJS_ORACLE="$oracle" cargo test --locked -p quickjs-oxide \
    --features test262-host --lib --bins --test unsupported_diagnostics
QJS_ORACLE="$oracle" cargo test --locked -p quickjs-oxide \
    --features test262-host --test oracle test262_
./scripts/check-oracle-registry.sh --compiled
./scripts/test-quickjs-fixtures.sh --all --oxide ./target/debug/qjs
./scripts/test-test262.sh --check
./scripts/test-test262.sh --focused
./scripts/test-test262.sh --full
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p quickjs-oxide --features test262-host \
    --lib --bins --test unsupported_diagnostics --test oracle \
    -- -D warnings
./scripts/check-rust-only.sh
