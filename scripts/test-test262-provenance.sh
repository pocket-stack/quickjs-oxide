#!/usr/bin/env bash
# Prove known unsupported grammar cannot pass a negative test for the wrong reason.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
suite=$($script_dir/prepare-test262.sh)
source_dir=$(dirname -- "$suite")
report=target/test262-provenance.tsv
json_report=target/test262-provenance.jsonl
baseline=tests/test262-provenance-baseline.tsv
json_baseline=tests/test262-provenance-baseline.jsonl
workers=${TEST262_WORKERS:-4}

cd -- "$root"
rm -f -- "$report" "$json_report"
cargo run --locked --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile compat/test262-oxide.conf \
    --manifest tests/test262-provenance-canaries.txt \
    --report "$report" \
    --mode both \
    --workers "$workers" \
    --allow-failures

if ! cmp -s "$baseline" "$report"; then
    echo "error: Test262 parser-provenance canaries drifted" >&2
    diff -u "$baseline" "$report" >&2 || true
    exit 1
fi
if ! cmp -s "$json_baseline" "$json_report"; then
    echo "error: Test262 parser-provenance JSONL canaries drifted" >&2
    diff -u "$json_baseline" "$json_report" >&2 || true
    exit 1
fi

echo "Test262 provenance canaries match: 8 audited pass, 11 fail-closed"
