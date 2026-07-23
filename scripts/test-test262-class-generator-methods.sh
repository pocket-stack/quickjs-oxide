#!/usr/bin/env bash
# Reproduce the R3k bootstrap cohort for public synchronous class generators.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-generator-methods-baseline.txt
manifest=tests/test262-class-generator-methods.txt
admission_profile=tests/test262-class-generator-methods.conf
global_profile=compat/test262-oxide.conf
report=target/test262-class-generator-methods.tsv
json_report=target/test262-class-generator-methods.jsonl
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs and the pinned QuickJS oracle only\n'
}

check_only=false
case ${1-} in
    "") ;;
    --check) check_only=true ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }

read_value() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$baseline"
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

profile_section() {
    local section=$1 profile=${2:-$admission_profile}
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

metadata_features() {
    local test_path=$1
    metadata_block "$test_path" | awk '
        /^features:[[:space:]]*\[/ {
            sub(/^features:[[:space:]]*\[/, "")
            sub(/\][[:space:]]*$/, "")
            count=split($0, values, /,[[:space:]]*/)
            for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            exit
        }
        /^features:[[:space:]]*$/ { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]*/ {
            sub(/^[[:space:]]*-[[:space:]]*/, "")
            print
            next
        }
        inside { exit }
    '
}

derive_direct_paths() {
    local test_path features
    git -C "$suite" ls-files \
        'test/language/expressions/class/gen-method/*.js' \
        'test/language/expressions/class/gen-method-static/*.js' \
        'test/language/statements/class/gen-method/*.js' \
        'test/language/statements/class/gen-method-static/*.js' \
        | LC_ALL=C sort | while IFS= read -r test_path; do
            features=$(metadata_features "$test_path" | LC_ALL=C sort)
            if [[ "$features" == "generators" ]]; then
                printf '%s\n' "$test_path"
            fi
        done
    true
}

derive_definition_paths() {
    git -C "$suite" ls-files 'test/language/statements/class/definition/*.js' \
        | awk -F/ '$NF == "fn-name-gen-method.js" || $NF ~ /^methods-gen-.*\.js$/ { print }' \
        | LC_ALL=C sort
}

derive_syntax_valid_paths() {
    git -C "$suite" ls-files \
        'test/language/expressions/class/elements/syntax/valid/*.js' \
        'test/language/statements/class/elements/syntax/valid/*.js' \
        | awk -F/ '$NF == "grammar-special-prototype-gen-meth-valid.js" || $NF == "grammar-static-ctor-gen-meth-valid.js" { print }' \
        | LC_ALL=C sort
}

derive_scope_paths() {
    git -C "$suite" ls-files \
        'test/language/expressions/class/*.js' \
        'test/language/statements/class/*.js' \
        | awk -F/ '$NF ~ /^scope-(static-)?gen-meth-paramsbody-var-(close|open)\.js$/ { print }' \
        | LC_ALL=C sort
}

derive_negative_paths() {
    local test_path
    while IFS= read -r test_path; do
        if metadata_block "$test_path" | grep -Fq 'negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < <(manifest_paths)
    true
}

derive_positive_paths() {
    local test_path
    while IFS= read -r test_path; do
        if ! metadata_block "$test_path" | grep -Fq 'negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < <(manifest_paths)
    true
}

verify_inventory() {
    local name=$1 inventory=$2 expected_count expected_hash
    expected_count=$(read_value "${name}_paths")
    expected_hash=$(read_value "${name}_sha256")
    if [[ "$(printf '%s\n' "$inventory" | wc -l | tr -d '[:space:]')" != "$expected_count" \
        || "$(printf '%s\n' "$inventory" | sha256_stream)" != "$expected_hash" ]]; then
        echo "error: class-generator $name inventory drifted" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 output test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < <(manifest_paths)
    if ! output=$(cd -- "$source_dir" && ./run-test262 -a -m -c test262.conf -f "${files[@]}" 2>&1); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the class-generator cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output" \
        || ! grep -Fq "Average memory statistics for $(read_value quickjs_passes) tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the complete class-generator cohort" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

if [[ "$(read_value quickjs)" != "2026-06-04" \
    || "$(read_value test262)" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$(read_value test262_patch_sha256)" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$(read_value test262_config_sha256)" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$(read_value test262_metadata_sha256)" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$(read_value schema)" != "test262-canonical-classified-v2" \
    || "$(read_value mode)" != "both" \
    || "$(read_value timeout_ms)" != "30000" \
    || "$(read_value paths)" != "82" \
    || "$(read_value variants)" != "160" \
    || "$(read_value quickjs_passes)" != "82" ]]; then
    echo "error: class-generator baseline identity drifted" >&2
    exit 1
fi

actual_paths=$(manifest_paths | wc -l | tr -d '[:space:]')
unique_paths=$(manifest_paths | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
[[ "$actual_paths" == "82" && "$unique_paths" == "82" ]] \
    || { echo "error: class-generator manifest cardinality drifted" >&2; exit 1; }
manifest_paths | LC_ALL=C sort -c
[[ "$(manifest_paths | sha256_stream)" == "$(read_value manifest_sha256)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_file_sha256)" ]] \
    || { echo "error: class-generator manifest content drifted" >&2; exit 1; }

direct_inventory=$(derive_direct_paths)
definition_inventory=$(derive_definition_paths)
syntax_valid_inventory=$(derive_syntax_valid_paths)
scope_inventory=$(derive_scope_paths)
derived_inventory=$(printf '%s\n' "$direct_inventory" "$definition_inventory" "$syntax_valid_inventory" "$scope_inventory" | LC_ALL=C sort)
verify_inventory direct "$direct_inventory"
verify_inventory definition "$definition_inventory"
verify_inventory syntax_valid "$syntax_valid_inventory"
verify_inventory scope "$scope_inventory"
diff -u <(printf '%s\n' "$derived_inventory") <(manifest_paths)

positive_inventory=$(derive_positive_paths)
negative_inventory=$(derive_negative_paths)
yield_star_inventory=$(manifest_paths | grep '/methods-gen-yield-star-')
single_variant_inventory=
variant_keys=
feature_inventory=
while IFS= read -r test_path; do
    [[ -f "$suite/$test_path" ]] || { echo "error: missing class-generator path: $test_path" >&2; exit 1; }
    metadata=$(metadata_block "$test_path")
    features=$(metadata_features "$test_path")
    feature_inventory+=$'\n'"$features"
    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "" | "flags: [generated]")
            variant_keys+=$'\n'"$test_path"$'\t'sloppy$'\n'"$test_path"$'\t'strict
            ;;
        "flags: [generated, onlyStrict]")
            single_variant_inventory+=$'\n'"$test_path"
            variant_keys+=$'\n'"$test_path"$'\t'strict
            ;;
        *) echo "error: class-generator flags drifted: $test_path: $flag_line" >&2; exit 1 ;;
    esac
    if grep -Fq 'negative:' <<<"$metadata" \
        && { ! grep -Fq '  phase: parse' <<<"$metadata" || ! grep -Fq '  type: SyntaxError' <<<"$metadata"; }; then
        echo "error: class-generator negative provenance drifted: $test_path" >&2
        exit 1
    fi
done < <(manifest_paths)
single_variant_inventory=$(printf '%s\n' "$single_variant_inventory" | sed '/^$/d' | LC_ALL=C sort)
variant_keys=$(printf '%s\n' "$variant_keys" | sed '/^$/d' | LC_ALL=C sort)
feature_inventory=$(printf '%s\n' "$feature_inventory" | sed '/^$/d' | LC_ALL=C sort -u)

verify_inventory positive "$positive_inventory"
verify_inventory negative "$negative_inventory"
verify_inventory yield_star "$yield_star_inventory"
verify_inventory single_variant "$single_variant_inventory"
[[ "$(printf '%s\n' "$variant_keys" | wc -l | tr -d '[:space:]')" == "$(read_value variants)" \
    && "$(printf '%s\n' "$variant_keys" | sha256_stream)" == "$(read_value keys_sha256)" ]] \
    || { echo "error: class-generator variant inventory drifted" >&2; exit 1; }

diff -u <(profile_section features | LC_ALL=C sort) <(printf '%s\n' "$feature_inventory")
diff -u <(profile_section audited-negative-tests | LC_ALL=C sort) <(printf '%s\n' "$negative_inventory")
[[ "$(sha256_file "$global_profile")" == "$(read_value global_oxide_profile_sha256)" \
    && "$(sha256_file "$admission_profile")" == "$(read_value oxide_profile_sha256)" ]] \
    || { echo "error: class-generator capability profile drifted" >&2; exit 1; }
if ! profile_section features "$global_profile" | grep -Fxq generators; then
    echo "error: global Test262 profile must admit generators" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'class-generator inputs verified: QuickJS %s passes %s paths; Oxide gate covers %s variants\n' \
        "$(read_value quickjs)" "$(read_value quickjs_passes)" "$(read_value variants)"
    exit 0
fi

pending_keys=()
for key in runnable passes failures unsupported skipped nonpass_sha256 tsv_sha256 jsonl_sha256 summary; do
    if [[ "$(read_value "$key")" == "PENDING" ]]; then
        pending_keys+=("$key")
    fi
done
if [[ ${#pending_keys[@]} -ne 0 ]]; then
    printf 'error: class-generator Oxide baseline needs refresh before execution: %s\n' \
        "${pending_keys[*]}" >&2
    exit 1
fi

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
expected_keys=$(read_value keys_sha256)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$admission_profile" \
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
    echo "error: class-generator report metadata drifted" >&2
    exit 1
fi

actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
expected_runner_summary="Test262: total=$expected_variants pass=$expected_passes fail=$expected_failures unsupported=$expected_unsupported skipped=$expected_skipped"

if [[ "$runner_summary" != "$expected_runner_summary" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: class-generator Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'class-generator Test262 gate is exact: %s/%s pass across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$expected_paths"
