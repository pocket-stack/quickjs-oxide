#!/usr/bin/env bash
# Reproduce the dependency-audited R3f derived-class and super() Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-derived-baseline.txt
manifest=tests/test262-class-derived.txt
profile=tests/test262-class-derived.conf
report=target/test262-class-derived.tsv
json_report=target/test262-class-derived.jsonl
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

program_body() {
    local test_path=$1
    awk '/^---\*\/$/ { body=1; next } body { print }' "$suite/$test_path"
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

derive_negative_tests() {
    local test_path
    while IFS= read -r test_path; do
        if metadata_block "$test_path" | grep -Fq 'negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < "$manifest"
}

metadata_has_flag() {
    local metadata=$1
    local flag=$2
    grep -Eq "^flags:.*(^|[,[[:space:]])${flag}([],[[:space:]]|$)|^[[:space:]]*-[[:space:]]*${flag}[[:space:]]*$" \
        <<<"$metadata"
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262
    local output test_path
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
        echo "error: pinned QuickJS could not execute the derived-class cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output" \
        || ! grep -Fq "Average memory statistics for $expected_paths tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the complete derived-class cohort" >&2
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
    || "$expected_profile" != "1aa167fef279273185060224bd8a65765283d95fe1e08986c5c4ea197657e160" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_paths" != "386" \
    || "$expected_variants" != "767" \
    || "$expected_runnable" != "767" \
    || "$expected_passes" != "767" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" ]]; then
    echo "error: derived-class baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(wc -l < "$manifest" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest" | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: derived-class manifest cardinality drifted" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
actual_manifest=$(sha256_file "$manifest")
if [[ "$actual_manifest" != "$expected_manifest" \
    || "$actual_manifest" != "$expected_manifest_file" ]]; then
    echo "error: derived-class manifest content drifted" >&2
    exit 1
fi
if grep -Eq '/(ArrayBuffer|DataView|PrivateName|Promise|Proxy|TypedArray|WeakMap|WeakSet)/|optional-chain-class-heritage|^test/staging/sm/(Array/from-iterator-close|Map/constructor-iterator-close)\.js$' "$manifest"; then
    echo "error: derived-class manifest crossed an audited adjacent-feature frontier" >&2
    exit 1
fi
if grep -Eq '^test/language/statements/class/subclass/builtins\.js$|^test/staging/sm/class/(superCallBadNewTargetPrototype|superCallBaseInvoked|superPropDelete)\.js$|^test/staging/sm/destructuring/order-super\.js$' "$manifest"; then
    echo "error: derived-class manifest admitted an untagged Proxy or TypedArray dependency" >&2
    exit 1
fi
if grep -Eq '^test/language/expressions/object/method-definition/early-errors-object-method-formals-contains-super-call\.js$|^test/language/statements/class/definition/early-errors-class-method-(body|formals)-contains-super-call\.js$' "$manifest"; then
    echo "error: derived-class manifest admitted an untagged async-method negative" >&2
    exit 1
fi
if grep -Eq '^test/staging/sm/class/(boundFunctionSubclassing|strictExecution|superPropOrdering)\.js$' "$manifest"; then
    echo "error: derived-class manifest admitted a pinned QuickJS staging known error" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: derived-class capability profile drifted" >&2
    exit 1
fi
if awk '
    /^\[features\]$/ { inside=1; next }
    /^\[/ { inside=0 }
    inside && $0 == "class" { found=1 }
    END { exit !found }
' compat/test262-oxide.conf; then
    echo "error: global Test262 profile must not declare whole-feature class support" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(derive_features | LC_ALL=C sort -u)
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    <(derive_negative_tests | LC_ALL=C sort)

construct_paths=$(grep -Ec '^test/built-ins/Function/internals/Construct/derived-' "$manifest")
to_string_paths=$(grep -Ec '^test/built-ins/Function/prototype/toString/class-' "$manifest")
consumer_paths=$(grep -Ec '^test/built-ins/(Object/subclass-object-arg|RegExp/named-groups/groups-object-subclass|Set/prototype/.*/subclass|String/prototype/replaceAll/searchValue-replacer-RegExp-call|Symbol/species/subclassing)' "$manifest")
eval_paths=$(grep -Ec '^test/language/eval-code/(direct|indirect)/super-' "$manifest")
global_paths=$(grep -Ec '^test/language/global-code/super-' "$manifest")
arrow_paths=$(grep -Ec '^test/language/expressions/arrow-function/lexical-super' "$manifest")
assignment_paths=$(grep -Ec '^test/language/expressions/assignment/target-super-' "$manifest")
class_expression_paths=$(grep -Ec '^test/language/expressions/class/' "$manifest")
delete_paths=$(grep -Ec '^test/language/expressions/delete/super-property' "$manifest")
expression_function_paths=$(grep -Ec '^test/language/expressions/function/early-(body|params)-super-' "$manifest")
new_target_paths=$(grep -Ec '^test/language/expressions/new.target/value-via-super-' "$manifest")
object_negative_paths=$(grep -Ec '^test/language/expressions/object/method-definition/.+super-call' "$manifest")
super_expression_paths=$(grep -Ec '^test/language/expressions/super/' "$manifest")
rest_paths=$(grep -Ec '^test/language/rest-parameters/with-new-target.js$' "$manifest")
class_statement_paths=$(grep -Ec '^test/language/statements/class/' "$manifest")
statement_function_paths=$(grep -Ec '^test/language/statements/function/early-(body|params)-super-' "$manifest")
staging_class_paths=$(grep -Ec '^test/staging/sm/class/' "$manifest")
staging_consumer_paths=$(grep -Ec '^test/staging/sm/(RegExp/|destructuring/order-super|expressions/(ToPropertyKey-symbols|short-circuit-compound-assignment-tdz))' "$manifest")
if [[ "$construct_paths" != "2" \
    || "$to_string_paths" != "5" \
    || "$consumer_paths" != "21" \
    || "$eval_paths" != "12" \
    || "$global_paths" != "4" \
    || "$arrow_paths" != "4" \
    || "$assignment_paths" != "3" \
    || "$class_expression_paths" != "29" \
    || "$delete_paths" != "5" \
    || "$expression_function_paths" != "4" \
    || "$new_target_paths" != "2" \
    || "$object_negative_paths" != "1" \
    || "$super_expression_paths" != "91" \
    || "$rest_paths" != "1" \
    || "$class_statement_paths" != "141" \
    || "$statement_function_paths" != "4" \
    || "$staging_class_paths" != "53" \
    || "$staging_consumer_paths" != "4" ]]; then
    echo "error: derived-class surface cardinality drifted" >&2
    exit 1
fi

generated_paths=0
no_strict_paths=0
only_strict_paths=0
plain_paths=0
includes_paths=0
negative_paths=0
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned derived-class path is missing: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    if grep -Eq '^flags:.*(module|raw)|^[[:space:]]*-[[:space:]]*(module|raw)[[:space:]]*$' <<<"$metadata"; then
        echo "error: derived-class path metadata left the audited scope: $test_path" >&2
        exit 1
    fi
    if metadata_has_flag "$metadata" generated; then
        generated_paths=$((generated_paths + 1))
    elif metadata_has_flag "$metadata" noStrict; then
        no_strict_paths=$((no_strict_paths + 1))
    elif metadata_has_flag "$metadata" onlyStrict; then
        only_strict_paths=$((only_strict_paths + 1))
    elif grep -q '^flags:' <<<"$metadata"; then
        echo "error: derived-class path flags left the audited scope: $test_path" >&2
        exit 1
    else
        plain_paths=$((plain_paths + 1))
    fi
    if grep -Fq 'negative:' <<<"$metadata"; then
        if ! grep -Fq '  phase: parse' <<<"$metadata" \
            || ! grep -Fq '  type: SyntaxError' <<<"$metadata"; then
            echo "error: derived-class negative provenance drifted: $test_path" >&2
            exit 1
        fi
        negative_paths=$((negative_paths + 1))
    fi
    if grep -q '^includes:' <<<"$metadata"; then
        if ! grep -Eq '^includes: \[(compareArray|nativeFunctionMatcher|propertyHelper)\.js\]$' <<<"$metadata"; then
            echo "error: derived-class harness dependency drifted: $test_path" >&2
            exit 1
        fi
        includes_paths=$((includes_paths + 1))
    fi
    if program_body "$test_path" | sed '/^[[:space:]]*\/\*/,/^[[:space:]]*\*\//d' | grep -Eq \
        '^[[:space:]]*(static[[:space:]]+)?async[[:space:]]|(^|[^[:alnum:]_$])function[[:space:]]*\*|^[[:space:]]*(static[[:space:]]+)?\*|^[[:space:]]*static[[:space:]]*\{'; then
        echo "error: derived-class path gained an async/generator/static-block form: $test_path" >&2
        exit 1
    fi
done < "$manifest"
if [[ "$generated_paths" != "91" \
    || "$no_strict_paths" != "3" \
    || "$only_strict_paths" != "2" \
    || "$plain_paths" != "290" \
    || "$includes_paths" != "39" \
    || "$negative_paths" != "29" ]]; then
    echo "error: derived-class metadata composition drifted" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'derived-class inputs verified: QuickJS %s passes %s paths; Oxide gate expands to %s variants\n' \
        "$expected_quickjs" "$expected_paths" "$expected_variants"
    exit 0
fi

run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" \
    --manifest "$manifest" \
    --report "$report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms")
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
    echo "error: derived-class report metadata drifted" >&2
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
    <(while IFS= read -r test_path; do
        metadata=$(metadata_block "$test_path")
        if metadata_has_flag "$metadata" noStrict; then
            printf '%s\tsloppy\n' "$test_path"
        elif metadata_has_flag "$metadata" onlyStrict; then
            printf '%s\tstrict\n' "$test_path"
        else
            printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path"
        fi
    done < "$manifest" | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort)

actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')

if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: derived-class Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'derived-class Test262 gate is exact: %s/%s pass across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$expected_paths"
