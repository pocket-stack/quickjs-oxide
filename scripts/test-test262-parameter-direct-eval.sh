#!/usr/bin/env bash
# Reproduce the dependency-audited R3b parameter-environment direct-eval gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest=tests/test262-parameter-direct-eval.txt
profile=tests/test262-parameter-direct-eval.conf
report=target/test262-parameter-direct-eval.tsv
json_report=target/test262-parameter-direct-eval.jsonl
workers=${TEST262_WORKERS:-8}

expected_quickjs=2026-06-04
expected_test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
expected_patch=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expected_config=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expected_metadata=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expected_profile=98b5e323db1b4be493c1e05b8937a1060b71f7a1cc126087d05e88e7c2a2b335
expected_manifest=3df66805796888dd41acbc007b2a958aba5751e9694c0deffa5f0efba19c61a1
expected_schema=test262-canonical-classified-v2
expected_mode=both
expected_timeout_ms=30000
expected_paths=71
expected_variants=71

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify the frozen manifest, profile, metadata, and QuickJS oracle only\n'
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

derive_negative_tests() {
    local test_path
    while IFS= read -r test_path; do
        if sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path" | grep -q '^negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < "$manifest"
}

derive_manifest() {
    local prefix test_file
    for prefix in arrow-fn- func-decl- func-expr- meth-; do
        for test_file in "$suite/test/language/eval-code/direct/${prefix}"*.js; do
            if grep -Eq '=[[:space:]]*eval[[:space:]]*\(' "$test_file"; then
                printf '%s\n' "${test_file#"$suite/"}"
            fi
        done
    done

    for test_file in \
        "$suite"/test/language/expressions/{arrow-function,function}/scope-param-{elem,rest-elem}-var-{close,open}.js \
        "$suite"/test/language/expressions/object/scope-meth-param-{elem,rest-elem}-var-{close,open}.js \
        "$suite"/test/language/statements/function/scope-param-{elem,rest-elem}-var-{close,open}.js; do
        printf '%s\n' "${test_file#"$suite/"}"
    done

    printf '%s\n' \
        test/language/expressions/arrow-function/eval-var-scope-syntax-err.js \
        test/language/expressions/function/eval-var-scope-syntax-err.js \
        test/language/expressions/object/method-definition/meth-eval-var-scope-syntax-err.js \
        test/language/function-code/eval-param-env-with-computed-key.js \
        test/language/function-code/eval-param-env-with-prop-initializer.js \
        test/language/statements/function/eval-var-scope-syntax-err.js \
        test/staging/sm/eval/redeclared-arguments-in-param-expression-eval.js
}

body_has_direct_eval() {
    local test_path=$1
    awk '
        /^---\*\/$/ { body=1; next }
        body { print }
    ' "$suite/$test_path" \
        | grep -Eq '(^|[^[:alnum:]_$])eval[[:space:]]*\('
}

metadata_has_no_strict() {
    local test_path=$1
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path" \
        | grep -Eq 'flags:.*noStrict|^[[:space:]]*-[[:space:]]*noStrict[[:space:]]*$'
}

verify_quickjs_oracle() {
    local oracle_runner=$source_dir/run-test262
    local test_path output
    local -a oracle_paths=()
    if [[ ! -x "$oracle_runner" ]]; then
        "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    fi
    while IFS= read -r test_path; do
        oracle_paths+=("test262/$test_path")
    done < "$manifest"
    if ! output=$(
        cd -- "$source_dir"
        ./run-test262 -a -m -c test262.conf -f "${oracle_paths[@]}" 2>&1
    ); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the parameter direct-eval cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the parameter direct-eval cohort" >&2
        exit 1
    fi
    if ! grep -Fq "Average memory statistics for ${expected_variants} tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS did not execute the complete parameter direct-eval cohort" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

actual_paths=$(wc -l < "$manifest" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest" | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: parameter direct-eval manifest cardinality drifted" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
if [[ "$(sha256_file "$manifest")" != "$expected_manifest" ]]; then
    echo "error: parameter direct-eval manifest content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: parameter direct-eval capability profile drifted" >&2
    exit 1
fi
if [[ "$(profile_section features)" != "default-parameters" \
    || -n "$(profile_section audited-negative-tests)" ]]; then
    echo "error: parameter direct-eval capability profile widened" >&2
    exit 1
fi

diff -u "$manifest" <(derive_manifest | LC_ALL=C sort -u)
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    <(derive_negative_tests | LC_ALL=C sort)

while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned parameter direct-eval path is missing: $test_path" >&2
        exit 1
    fi
    case "$test_path" in
        */async-*/* | */generators/* | */class/* | *accessor* | *getter* | *setter*)
            echo "error: parameter direct-eval manifest admitted an independent frontier: $test_path" >&2
            exit 1
            ;;
        test/staging/sm/Function/function-name-method.js | \
        test/staging/sm/Function/implicit-this-in-parameter-expression.js)
            echo "error: parameter direct-eval manifest admitted an excluded oracle case: $test_path" >&2
            exit 1
            ;;
    esac
    if ! metadata_has_no_strict "$test_path"; then
        echo "error: parameter direct-eval path is not a noStrict fixture: $test_path" >&2
        exit 1
    fi
    if ! body_has_direct_eval "$test_path"; then
        echo "error: parameter direct-eval path lost its direct eval call: $test_path" >&2
        exit 1
    fi
done < "$manifest"

verify_quickjs_oracle
if "$check_only"; then
    printf 'parameter direct-eval inputs verified: QuickJS %s passes %s variants across %s paths\n' \
        "$expected_quickjs" "$expected_variants" "$expected_paths"
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
    echo "error: parameter direct-eval report metadata drifted" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i <= count; i++) if (features[i] != "") print features[i]
        }
    ' "$report" | LC_ALL=C sort -u)
diff -u \
    <(awk '{ print $0 "\tsloppy" }' "$manifest") \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort)

actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
json_lines=$(wc -l < "$json_report" | tr -d '[:space:]')
if [[ "$actual_passes" != "$expected_variants" \
    || "$actual_failures" != "0" \
    || "$actual_unsupported" != "0" \
    || "$actual_skipped" != "0" \
    || "$json_lines" != "$((expected_variants + 2))" \
    || "$(tail -n 1 "$report")" != "# summary pass=$expected_variants" ]]; then
    echo "error: parameter direct-eval Test262 gate did not fully pass" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'parameter direct-eval Test262 gate passes: %s/%s variants across %s paths\n' \
    "$actual_passes" "$expected_variants" "$expected_paths"
