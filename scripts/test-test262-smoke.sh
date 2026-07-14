#!/usr/bin/env bash
# Run the fixed synchronous-script Test262 smoke slice.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
suite=$($script_dir/prepare-test262.sh)
source_dir=$(dirname -- "$suite")
metadata_records=target/test262-metadata.records
report=target/test262-smoke.tsv
json_report=target/test262-smoke.jsonl
baseline=tests/test262-smoke-baseline.tsv
json_baseline=tests/test262-smoke-baseline.jsonl
expected_metadata_sha256=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a

cd -- "$root"
rm -f -- "$metadata_records" "$report" "$json_report"
cargo run --locked --quiet --bin run-test262 -- \
    --suite "$suite" \
    --validate-metadata "$metadata_records"

if command -v sha256sum >/dev/null 2>&1; then
    actual_metadata_sha256=$(sha256sum "$metadata_records" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    actual_metadata_sha256=$(shasum -a 256 "$metadata_records" | awk '{print $1}')
else
    echo "error: sha256sum or shasum is required to verify Test262 metadata" >&2
    exit 2
fi
if [[ "$actual_metadata_sha256" != "$expected_metadata_sha256" ]]; then
    echo "error: Rust metadata parser disagrees with the pinned exhaustive fingerprint" >&2
    echo "expected: $expected_metadata_sha256" >&2
    echo "actual:   $actual_metadata_sha256" >&2
    exit 1
fi

cargo run --locked --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --manifest tests/test262-smoke.txt \
    --report "$report" \
    --mode both \
    --allow-failures

if ! cmp -s "$baseline" "$report"; then
    echo "error: Test262 smoke outcome vector drifted from its checked-in baseline" >&2
    diff -u "$baseline" "$report" >&2 || true
    exit 1
fi
if ! cmp -s "$json_baseline" "$json_report"; then
    echo "error: Test262 JSONL outcome vector drifted from its checked-in baseline" >&2
    diff -u "$json_baseline" "$json_report" >&2 || true
    exit 1
fi

echo "Test262 smoke baseline matches: 189 pass, 4 unsupported-parser"
