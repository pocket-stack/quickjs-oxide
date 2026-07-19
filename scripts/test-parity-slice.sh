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
trap 'rm -f -- "$generated_ident" "$generated_case" "$generated_property"' EXIT HUP INT TERM
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
rm -f -- "$generated_ident" "$generated_case" "$generated_property"
trap - EXIT HUP INT TERM

cargo fmt --all -- --check
QJS_ORACLE="$oracle" cargo test --locked --workspace --all-targets
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
./scripts/run-test262-arrow.sh
./scripts/test-test262-reflect.sh
./scripts/test-test262-date.sh
./scripts/test-test262-string-split.sh
./scripts/test-test262-regexp-core.sh
./scripts/run-test262-regexp-literals.sh
./scripts/run-test262-regexp-search.sh
./scripts/run-test262-regexp-match.sh
./scripts/run-test262-regexp-split.sh
./scripts/run-test262-regexp-compile.sh
./scripts/run-test262-regexp-modifiers.sh
./scripts/run-test262-replace.sh
./scripts/run-test262-regexp-match-all.sh
./scripts/run-test262-regexp-backreferences.sh
./scripts/run-test262-regexp-lookahead.sh
./scripts/run-test262-regexp-lookbehind.sh
./scripts/run-test262-regexp-unicode-properties.sh
./scripts/run-test262-regexp-named-groups.sh
./scripts/run-test262-regexp-duplicate-named-groups.sh
./scripts/run-test262-regexp-match-indices.sh
./scripts/run-test262-regexp-dotall.sh
./scripts/run-test262-unicode-u180e.sh
./scripts/run-test262-eval-intrinsic.sh
./scripts/run-test262-eval-declarations.sh
./scripts/run-test262-nested-direct-eval.sh
./scripts/run-test262-with.sh
./scripts/run-test262-object-methods.sh
./scripts/run-test262-object-accessors.sh
./scripts/run-test262-object-super.sh
./scripts/run-test262-object-super-arrow.sh
./scripts/run-test262-object-super-eval.sh
./scripts/test-test262-tagged-template.sh
./scripts/test-test262-json-parse.sh
./scripts/test-test262-full.sh
cargo clippy --locked --workspace --all-targets -- -D warnings
./scripts/check-rust-only.sh
