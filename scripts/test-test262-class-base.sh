#!/usr/bin/env bash
# Reproduce the dependency-audited R3e base-class Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-base-baseline.txt
manifest=tests/test262-class-base.txt
profile=tests/test262-class-base.conf
report=target/test262-class-base.tsv
json_report=target/test262-class-base.jsonl
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
        echo "error: pinned QuickJS could not execute the base-class cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output" \
        || ! grep -Fq "Average memory statistics for $expected_paths tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the complete base-class cohort" >&2
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
    || "$expected_profile" != "df73a1ac299cce6ade0b0638f0a4c3322310aa2db8e15a28039f483328e69f00" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_paths" != "157" \
    || "$expected_variants" != "294" \
    || "$expected_runnable" != "294" \
    || "$expected_passes" != "294" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" ]]; then
    echo "error: base-class baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(wc -l < "$manifest" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest" | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: base-class manifest cardinality drifted" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
actual_manifest=$(sha256_file "$manifest")
if [[ "$actual_manifest" != "$expected_manifest" \
    || "$actual_manifest" != "$expected_manifest_file" ]]; then
    echo "error: base-class manifest content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: base-class capability profile drifted" >&2
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
if [[ -n "$(profile_section audited-negative-tests)" ]]; then
    echo "error: base-class profile unexpectedly admitted negative tests" >&2
    exit 1
fi

call_paths=$(grep -c '^test/built-ins/Function/internals/Call/class-ctor.js$' "$manifest")
eval_paths=$(grep -Ec '^test/language/eval-code/(direct|indirect)/lex-env-(distinct|no-init)-cls.js$' "$manifest")
accessor_paths=$(grep -Ec '^test/language/(expressions|statements)/class/accessor-name-(inst|static)/' "$manifest")
syntax_paths=$(grep -Ec '^test/language/(expressions|statements)/class/elements/syntax/valid/' "$manifest")
method_paths=$(grep -Ec '^test/language/(expressions|statements)/class/method(-static)?/forbidden-ext/' "$manifest")
statement_list_paths=$(grep -Ec '^test/language/statementList/(eval-)?class-' "$manifest")
name_paths=$(grep -Ec '^test/language/(expressions/assignment|statements/(const|let|variable))/fn-name-class.js$' "$manifest")
definition_paths=$(grep -Ec '^test/language/statements/class/definition/' "$manifest")
name_binding_paths=$(grep -Ec '^test/language/statements/class/name-binding/' "$manifest")
if [[ "$call_paths" != "1" \
    || "$eval_paths" != "4" \
    || "$accessor_paths" != "84" \
    || "$syntax_paths" != "10" \
    || "$method_paths" != "20" \
    || "$statement_list_paths" != "14" \
    || "$name_paths" != "4" \
    || "$definition_paths" != "17" \
    || "$name_binding_paths" != "3" ]]; then
    echo "error: base-class surface cardinality drifted" >&2
    exit 1
fi

generated_paths=0
no_strict_paths=0
plain_paths=0
includes_paths=0
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned base-class path is missing: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    declared_features=$(awk '/^features:/ { print }' <<<"$metadata")
    if [[ "$declared_features" != "" \
        && "$declared_features" != "features: [class]" \
        && "$declared_features" != "features: [Symbol]" ]] \
        || grep -Eq '^negative:|flags:.*(onlyStrict|module|raw)' <<<"$metadata"; then
        echo "error: base-class path metadata left the audited scope: $test_path" >&2
        exit 1
    fi
    case $(awk '/^flags:/ { print; found=1 } END { if (!found) print "plain" }' <<<"$metadata") in
        'flags: [generated]') generated_paths=$((generated_paths + 1)) ;;
        'flags: [generated, noStrict]') no_strict_paths=$((no_strict_paths + 1)) ;;
        plain) plain_paths=$((plain_paths + 1)) ;;
        *)
            echo "error: base-class path flags left the audited scope: $test_path" >&2
            exit 1
            ;;
    esac
    if grep -q '^includes:' <<<"$metadata"; then
        if ! grep -Fxq 'includes: [propertyHelper.js]' <<<"$metadata"; then
            echo "error: base-class harness dependency drifted: $test_path" >&2
            exit 1
        fi
        includes_paths=$((includes_paths + 1))
    fi
    if program_body "$test_path" | grep -Eq \
        '(^|[^[:alnum:]_$])extends([^[:alnum:]_$]|$)|#[[:alpha:]_$]|^[[:space:]]*(static[[:space:]]+)?async[[:space:]]|^[[:space:]]*(static[[:space:]]+)?\*|^[[:space:]]*static[[:space:]]*\{'; then
        echo "error: base-class path gained an out-of-scope class form: $test_path" >&2
        exit 1
    fi
done < "$manifest"
if [[ "$generated_paths" != "108" \
    || "$no_strict_paths" != "20" \
    || "$plain_paths" != "29" \
    || "$includes_paths" != "13" ]]; then
    echo "error: base-class metadata composition drifted" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'base-class inputs verified: QuickJS %s passes %s paths; Oxide gate expands to %s variants\n' \
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
    echo "error: base-class report metadata drifted" >&2
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
        if metadata_block "$test_path" | grep -Fq 'noStrict'; then
            printf '%s\tsloppy\n' "$test_path"
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
    echo "error: base-class Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'base-class Test262 gate is exact: %s/%s pass across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$expected_paths"
