#!/usr/bin/env bash
# Reproduce the dependency-audited R3d argument-spread Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-argument-spread-baseline.txt
manifest=tests/test262-argument-spread.txt
profile=tests/test262-argument-spread.conf
report=target/test262-argument-spread.tsv
json_report=target/test262-argument-spread.jsonl
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs and the pinned QuickJS oracle only\n'
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

read_value() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$baseline"
}

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

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
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

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262
    local frontier=test/staging/sm/Function/invalid-parameter-list.js
    local test_path output frontier_output
    local -a paths=()
    if [[ ! -x "$runner" ]]; then
        "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    fi
    while IFS= read -r test_path; do
        if [[ "$test_path" == "$frontier" ]]; then
            continue
        fi
        paths+=("test262/$test_path")
    done < "$manifest"
    if ! output=$(
        cd -- "$source_dir"
        ./run-test262 -a -m -c test262.conf -f "${paths[@]}" 2>&1
    ); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the passing argument-spread cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the 66-path argument-spread cohort" >&2
        exit 1
    fi
    if ! grep -Fq "Average memory statistics for $((expected_paths - 1)) tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS did not execute the complete passing argument-spread cohort" >&2
        exit 1
    fi

    frontier_output=$(
        cd -- "$source_dir"
        ./run-test262 -a -c test262.conf -f "test262/$frontier" 2>&1
    ) || true
    if ! grep -Fq 'Expected a SyntaxError to be thrown but no exception was thrown at all' \
        <<<"$frontier_output" \
        || ! grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$frontier_output"; then
        printf '%s\n' "$frontier_output" >&2
        echo "error: pinned QuickJS invalid-parameter-list frontier drifted" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

expected_quickjs=$(read_value quickjs)
expected_test262=$(read_value test262)
expected_patch=$(read_value test262_patch_sha256)
expected_config=$(read_value test262_config_sha256)
expected_metadata=$(read_value test262_metadata_sha256)
expected_profile=$(read_value oxide_profile_sha256)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
expected_timeout_ms=$(read_value timeout_ms)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_keys=$(read_value keys_sha256)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "5db27822923dd066c7afb448ae5dcdef25e57573cd2ac651dfe2b13892980112" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_paths" != "67" \
    || "$expected_variants" != "134" \
    || "$expected_runnable" != "134" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" ]]; then
    echo "error: argument-spread baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(wc -l < "$manifest" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest" | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: argument-spread manifest cardinality drifted" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
actual_manifest=$(sha256_file "$manifest")
if [[ "$actual_manifest" != "$expected_manifest" \
    || "$actual_manifest" != "$expected_manifest_file" ]]; then
    echo "error: argument-spread manifest content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: argument-spread capability profile drifted" >&2
    exit 1
fi

ordinary_call_paths=$(grep -c '^test/language/expressions/call/' "$manifest")
eval_call_paths=$(grep -c '^test/language/expressions/call/eval-spread' "$manifest")
new_paths=$(grep -c '^test/language/expressions/new/' "$manifest")
other_paths=$((actual_paths - ordinary_call_paths - new_paths))
if [[ "$ordinary_call_paths" != "25" \
    || "$eval_call_paths" != "4" \
    || "$new_paths" != "20" \
    || "$other_paths" != "22" ]]; then
    echo "error: argument-spread surface cardinality drifted" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(derive_features | LC_ALL=C sort -u)
if [[ -n "$(profile_section audited-negative-tests)" ]]; then
    echo "error: argument-spread profile unexpectedly admitted negative tests" >&2
    exit 1
fi

while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned argument-spread path is missing: $test_path" >&2
        exit 1
    fi
    if metadata_block "$test_path" | grep -Eq '^negative:|flags:.*(noStrict|onlyStrict|module|raw)'; then
        echo "error: argument-spread path no longer expands to sloppy and strict: $test_path" >&2
        exit 1
    fi
    if ! awk '/^---\*\/$/ { body=1; next } body { print }' "$suite/$test_path" \
        | grep -q '\.\.\.'; then
        echo "error: argument-spread path lost its spread expression: $test_path" >&2
        exit 1
    fi
done < "$manifest"

verify_quickjs_oracle
if "$check_only"; then
    printf 'argument-spread inputs verified: QuickJS %s passes 66 paths with one pinned upstream frontier; Oxide gate expands to %s variants\n' \
        "$expected_quickjs" "$expected_variants"
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
    || "$actual_runnable" != "$expected_runnable" ]]; then
    echo "error: argument-spread report metadata drifted" >&2
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

actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
actual_tsv=$(sha256_file "$report")
actual_jsonl=$(sha256_file "$json_report")
actual_summary=${actual_summary:-$(tail -n 1 "$report" | sed 's/^# summary //')}

expected_frontier=$(cat <<'EOF'
test/staging/sm/Array/concat-proxy.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/concat-proxy.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/join-no-has-trap.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/join-no-has-trap.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/pop-no-has-trap.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/pop-no-has-trap.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/shift-no-has-trap.js	sloppy	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Array/shift-no-has-trap.js	strict	fail-runtime	runtime	ReferenceError	'Proxy' is not defined
test/staging/sm/Function/invalid-parameter-list.js	sloppy	fail-runtime	runtime	Test262Error	Expected a SyntaxError to be thrown but no exception was thrown at all
test/staging/sm/Function/invalid-parameter-list.js	strict	fail-runtime	runtime	Test262Error	Expected a SyntaxError to be thrown but no exception was thrown at all
test/staging/sm/generators/iterator-next-non-object.js	sloppy	fail-runtime	runtime	Test262Error	Expected a TypeError but got a ReferenceError
test/staging/sm/generators/iterator-next-non-object.js	strict	fail-runtime	runtime	Test262Error	Expected a TypeError but got a ReferenceError
EOF
)
actual_frontier=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }
' "$report")

if [[ "$expected_passes" == PENDING_* \
    || "$expected_keys" == PENDING_* \
    || "$expected_tsv" == PENDING_* \
    || "$expected_jsonl" == PENDING_* ]]; then
    echo "error: argument-spread report hashes await the verified R3d run" >&2
    echo "passes=$actual_passes" >&2
    echo "failures=$actual_failures" >&2
    echo "keys_sha256=$actual_keys" >&2
    echo "nonpass_sha256=$actual_nonpass" >&2
    echo "tsv_sha256=$actual_tsv" >&2
    echo "jsonl_sha256=$actual_jsonl" >&2
    echo "summary=$actual_summary" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_frontier" != "$expected_frontier" \
    || "$actual_summary" != "$expected_summary" \
    || "$actual_tsv" != "$expected_tsv" \
    || "$actual_jsonl" != "$expected_jsonl" ]]; then
    echo "error: argument-spread Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'argument-spread Test262 gate is exact: %s/%s pass across %s paths; %s classified frontier variants\n' \
    "$actual_passes" "$actual_variants" "$expected_paths" "$actual_failures"
