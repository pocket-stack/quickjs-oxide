#!/usr/bin/env bash
# Reproduce the complete R3c AggregateError and Error cause feature cohort.
# Three upstream paths have an undeclared Proxy dependency; their six variants
# stay pinned as the independent Proxy frontier instead of becoming false
# AggregateError failures.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest=tests/test262-aggregate-error.txt
profile=tests/test262-aggregate-error.conf
report=target/test262-aggregate-error.tsv
json_report=target/test262-aggregate-error.jsonl
workers=${TEST262_WORKERS:-8}

expected_quickjs=2026-06-04
expected_test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
expected_patch=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expected_config=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expected_metadata=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expected_profile=ad9e38f7b1b42445a848ee01437e925fc23f5525276bc45dd15c5ae7a1454d7a
expected_manifest=f54979cc3881fd7d361dda7ffbbe75a5bf846e233512c7428711c1091b8474c5
expected_schema=test262-canonical-classified-v2
expected_mode=both
expected_timeout_ms=30000
expected_paths=28
expected_variants=56
expected_passes=50
expected_failures=6
expected_quickjs_tests=28

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify the frozen manifest, profile, metadata, and QuickJS runner only\n'
}

check_only=false
case ${1-} in
    "") ;;
    --check) check_only=true ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
if [[ $# -gt 1 ]]; then
    usage >&2
    exit 2
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

read_header() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == "# " key { print $2; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

metadata_block() {
    local test_path=$1
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path"
}

derive_manifest() {
    find "$suite/test/built-ins/AggregateError" -type f -name '*.js' \
        ! -name 'proto-from-ctor-realm.js' \
        | sed "s#^$suite/##"

    local test_file
    while IFS= read -r test_file; do
        if sed -n '/^\/\*---$/,/^---\*\/$/p' "$test_file" \
            | grep -Eq 'features:.*(^|[[:space:],[])error-cause([],[[:space:]]|$)'; then
            printf '%s\n' "${test_file#"$suite/"}"
        fi
    done < <(find \
        "$suite/test/built-ins/Error" \
        "$suite/test/built-ins/NativeErrors" \
        -type f -name '*.js' | LC_ALL=C sort)
}

derive_features() {
    local test_path
    while IFS= read -r test_path; do
        metadata_block "$test_path" | awk '
            /^features:/ {
                sub(/^features:[[:space:]]*\[/, "")
                sub(/\][[:space:]]*$/, "")
                count=split($0, values, /,[[:space:]]*/)
                for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            }
        '
    done < "$manifest"
}

verify_quickjs_runner() {
    local runner=$source_dir/run-test262
    local test_path output
    local -a paths=()
    if [[ ! -x "$runner" ]]; then
        "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    fi
    while IFS= read -r test_path; do
        paths+=("test262/$test_path")
    done < "$manifest"
    if ! output=$(
        cd -- "$source_dir"
        ./run-test262 -a -m -c test262.conf -f "${paths[@]}" 2>&1
    ); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the AggregateError cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the AggregateError cohort" >&2
        exit 1
    fi
    if ! grep -Fq "Average memory statistics for ${expected_quickjs_tests} tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS did not execute the complete AggregateError cohort" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

actual_paths=$(wc -l < "$manifest" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest" | wc -l | tr -d '[:space:]')
aggregate_paths=$(grep -c '^test/built-ins/AggregateError/' "$manifest")
cause_paths=$((actual_paths - aggregate_paths))
if [[ "$actual_paths" != "$expected_paths" \
    || "$unique_paths" != "$expected_paths" \
    || "$aggregate_paths" != "24" \
    || "$cause_paths" != "4" ]]; then
    echo "error: AggregateError manifest cardinality drifted" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
if [[ "$(sha256_file "$manifest")" != "$expected_manifest" ]]; then
    echo "error: AggregateError manifest content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: AggregateError capability profile drifted" >&2
    exit 1
fi

diff -u "$manifest" <(derive_manifest | LC_ALL=C sort -u)
diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(derive_features | LC_ALL=C sort -u)
if [[ -n "$(profile_section audited-negative-tests)" ]]; then
    echo "error: AggregateError profile unexpectedly admitted negative tests" >&2
    exit 1
fi

while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned AggregateError path is missing: $test_path" >&2
        exit 1
    fi
    if metadata_block "$test_path" | grep -Eq '^negative:|^flags:'; then
        echo "error: AggregateError path no longer expands to sloppy and strict variants: $test_path" >&2
        exit 1
    fi
    case "$test_path" in
        *proto-from-ctor-realm.js | */Promise/* | */class/*)
            echo "error: AggregateError manifest admitted an independent frontier: $test_path" >&2
            exit 1
            ;;
    esac
done < "$manifest"

verify_quickjs_runner
if "$check_only"; then
    printf 'AggregateError inputs verified: QuickJS %s passes %s paths; Oxide gate expands to %s variants\n' \
        "$expected_quickjs" "$expected_paths" "$expected_variants"
    exit 0
fi

rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" \
    --manifest "$manifest" \
    --report "$report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms" \
    --allow-failures)
printf '%s\n' "$run_output"

actual_variants=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { count++ } END { print count + 0 }' "$report")
execution_line=$(printf '%s\n' "$run_output" | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
actual_runnable=${execution_line#*runnable=}
actual_runnable=${actual_runnable%% *}
if [[ "$(read_header quickjs)" != "$expected_quickjs" \
    || "$(read_header test262)" != "$expected_test262" \
    || "$(read_header test262_patch_sha256)" != "$expected_patch" \
    || "$(read_header test262_config_sha256)" != "$expected_config" \
    || "$(read_header test262_metadata_sha256)" != "$expected_metadata" \
    || "$(read_header oxide_profile_sha256)" != "$expected_profile" \
    || "$(read_header profile)" != "$expected_schema" \
    || "$(read_header mode)" != "$expected_mode" \
    || "$actual_variants" != "$expected_variants" \
    || "$actual_runnable" != "$expected_variants" ]]; then
    echo "error: AggregateError report metadata drifted" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i<=count; i++) if (features[i] != "") print features[i]
        }
    ' "$report" | LC_ALL=C sort -u)
diff -u \
    <(while IFS= read -r path; do printf '%s\tsloppy\n%s\tstrict\n' "$path" "$path"; done < "$manifest" | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort)

actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
json_lines=$(wc -l < "$json_report" | tr -d '[:space:]')
if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "0" \
    || "$actual_skipped" != "0" \
    || "$json_lines" != "$((expected_variants + 2))" \
    || "$(tail -n 1 "$report")" != "# summary fail-runtime=$expected_failures pass=$expected_passes" ]]; then
    echo "error: AggregateError Test262 gate outcome drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

expected_proxy_frontier=$(cat <<'EOF'
test/built-ins/AggregateError/newtarget-proto-custom.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/built-ins/AggregateError/newtarget-proto-custom.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/built-ins/AggregateError/newtarget-proto-fallback.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/built-ins/AggregateError/newtarget-proto-fallback.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/built-ins/Error/cause_abrupt.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/built-ins/Error/cause_abrupt.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
EOF
)
actual_proxy_frontier=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }
' "$report")
if [[ "$actual_proxy_frontier" != "$expected_proxy_frontier" ]]; then
    echo "error: AggregateError undeclared-Proxy frontier drifted" >&2
    diff -u <(printf '%s\n' "$expected_proxy_frontier") <(printf '%s\n' "$actual_proxy_frontier") >&2 || true
    exit 1
fi

printf 'AggregateError Test262 cohort is exact: %s/%s pass; %s variants remain at the Proxy frontier\n' \
    "$actual_passes" "$expected_variants" "$expected_failures"
