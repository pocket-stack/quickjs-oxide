#!/usr/bin/env bash
# Reproduce the complete dependency-audited R3g public class-init cohort.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-public-init-baseline.txt
manifest=tests/test262-class-public-init.txt
admission_profile=tests/test262-class-public-init.conf
global_profile=compat/test262-oxide.conf
report=target/test262-class-public-init.tsv
json_report=target/test262-class-public-init.jsonl
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

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

excluded_paths() {
    printf '%s\n' \
        'test/language/expressions/generators/static-init-await-binding.js' \
        'test/language/expressions/generators/static-init-await-reference.js' \
        'test/language/expressions/object/method-definition/static-init-await-binding-generator.js' \
        'test/language/expressions/object/method-definition/static-init-await-reference-generator.js' \
        'test/language/statements/class/elements/public-class-field-initialization-is-visible-to-proxy.js' \
        'test/language/statements/class/elements/syntax/valid/grammar-field-named-set-followed-by-generator-asi.js' \
        'test/language/statements/class/static-init-arguments-functions.js' \
        'test/language/statements/class/static-init-arguments-methods.js' \
        'test/language/statements/class/static-init-invalid-await.js' \
        'test/language/statements/class/static-init-invalid-yield.js'
}

tagged_paths() {
    git -C "$suite" grep -l -E \
        'class-fields-public|class-static-fields-public|class-static-block' \
        -- test \
        | LC_ALL=C sort
}

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$admission_profile"
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
            done=1
            exit
        }
        /^features:[[:space:]]*$/ { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]*/ {
            sub(/^[[:space:]]*-[[:space:]]*/, "")
            print
            next
        }
        inside { done=1; exit }
    '
}

derive_candidate_paths() {
    local test_path metadata features
    while IFS= read -r test_path; do
        metadata=$(metadata_block "$test_path")
        features=$(metadata_features "$test_path")
        if ! grep -Eq '^(class-fields-public|class-static-fields-public|class-static-block)$' \
            <<<"$features"; then
            continue
        fi
        if grep -Eq '^(computed-property-names|class-fields-private|class-methods-private|class-static-fields-private|class-static-methods-private|async-functions|async-iteration|generators|top-level-await|explicit-resource-management|Proxy)$' \
            <<<"$features"; then
            continue
        fi
        if grep -Eq '^flags:.*(module|raw|async)|^[[:space:]]*-[[:space:]]*(module|raw|async)[[:space:]]*$' \
            <<<"$metadata"; then
            continue
        fi
        printf '%s\n' "$test_path"
    done < <(tagged_paths)
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
    done < <(manifest_paths)
}

derive_negative_tests() {
    local test_path
    while IFS= read -r test_path; do
        if metadata_block "$test_path" | grep -Fq 'negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < <(manifest_paths)
}

metadata_has_flag() {
    local metadata=$1
    local flag=$2
    grep -Eq "^flags:.*(^|[,[[:space:]])${flag}([],[[:space:]]|$)|^[[:space:]]*-[[:space:]]*${flag}[[:space:]]*$" \
        <<<"$metadata"
}

assert_body_matches() {
    local test_path=$1
    local pattern=$2
    if ! program_body "$test_path" | grep -Eq "$pattern"; then
        echo "error: excluded class-init source-form drifted: $test_path" >&2
        exit 1
    fi
}

verify_excluded_sources() {
    local test_path metadata
    while IFS= read -r test_path; do
        if [[ ! -f "$suite/$test_path" ]]; then
            echo "error: excluded class-init path is missing: $test_path" >&2
            exit 1
        fi
    done < <(excluded_paths)

    test_path=test/language/statements/class/elements/public-class-field-initialization-is-visible-to-proxy.js
    metadata=$(metadata_block "$test_path")
    if ! grep -Fxq 'features: [class, class-fields-public]' <<<"$metadata"; then
        echo "error: untagged Proxy exclusion metadata drifted" >&2
        exit 1
    fi
    assert_body_matches "$test_path" 'new[[:space:]]+Proxy\('

    test_path=test/language/statements/class/elements/syntax/valid/grammar-field-named-set-followed-by-generator-asi.js
    metadata=$(metadata_block "$test_path")
    if ! grep -Fxq 'features: [class-fields-public, class]' <<<"$metadata"; then
        echo "error: generator-method ASI exclusion metadata drifted" >&2
        exit 1
    fi
    assert_body_matches "$test_path" '^[[:space:]]*\*[[:alnum:]_$]+\('

    for test_path in \
        test/language/expressions/generators/static-init-await-binding.js \
        test/language/expressions/generators/static-init-await-reference.js \
        test/language/expressions/object/method-definition/static-init-await-binding-generator.js \
        test/language/expressions/object/method-definition/static-init-await-reference-generator.js \
        test/language/statements/class/static-init-arguments-functions.js \
        test/language/statements/class/static-init-arguments-methods.js \
        test/language/statements/class/static-init-invalid-await.js \
        test/language/statements/class/static-init-invalid-yield.js
    do
        metadata=$(metadata_block "$test_path")
        if ! grep -Fxq 'features: [class-static-block]' <<<"$metadata"; then
            echo "error: untagged async/generator static-block metadata drifted: $test_path" >&2
            exit 1
        fi
    done

    assert_body_matches \
        test/language/expressions/generators/static-init-await-binding.js \
        'function[[:space:]]*\*'
    assert_body_matches \
        test/language/expressions/generators/static-init-await-reference.js \
        'function[[:space:]]*\*'
    assert_body_matches \
        test/language/expressions/object/method-definition/static-init-await-binding-generator.js \
        '\*method\('
    assert_body_matches \
        test/language/expressions/object/method-definition/static-init-await-reference-generator.js \
        '\*method\('
    assert_body_matches \
        test/language/statements/class/static-init-arguments-functions.js \
        'function[[:space:]]*\*'
    assert_body_matches \
        test/language/statements/class/static-init-arguments-functions.js \
        'async[[:space:]]+function'
    assert_body_matches \
        test/language/statements/class/static-init-arguments-methods.js \
        '\*gen\('
    assert_body_matches \
        test/language/statements/class/static-init-arguments-methods.js \
        'async[[:space:]]+async\('
    assert_body_matches \
        test/language/statements/class/static-init-invalid-await.js \
        'async[[:space:]]+function[[:space:]]+f\('
    assert_body_matches \
        test/language/statements/class/static-init-invalid-yield.js \
        'function[[:space:]]*\*[[:space:]]*g\('
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
    done < <(manifest_paths)
    if ! output=$(
        cd -- "$source_dir"
        ./run-test262 -a -m -c test262.conf -f "${paths[@]}" 2>&1
    ); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the public-class-init cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output" \
        || ! grep -Fq "Average memory statistics for $expected_quickjs_passes tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the complete public-class-init cohort" >&2
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
expected_global_profile=$(read_value global_oxide_profile_sha256)
expected_profile=$(read_value oxide_profile_sha256)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
expected_timeout_ms=$(read_value timeout_ms)
expected_tagged_paths=$(read_value tagged_paths)
expected_candidate_paths=$(read_value candidate_paths)
expected_excluded_paths=$(read_value excluded_paths)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_quickjs_passes=$(read_value quickjs_passes)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_public_instance_paths=$(read_value public_instance_paths)
expected_public_static_paths=$(read_value public_static_paths)
expected_static_block_paths=$(read_value static_block_paths)
expected_negative_paths=$(read_value negative_paths)
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_candidate=$(read_value candidate_sha256)
expected_excluded=$(read_value excluded_sha256)
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
    || "$expected_global_profile" != "d01f4f49fbd14b2cad610983624142b468587b2e0bd10ae6264641c39cffa05f" \
    || "$expected_profile" != "f02524f9abedc00c6c84dc33367680bf31a30ae94604a5317a6690f603cbd7b1" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_tagged_paths" != "2179" \
    || "$expected_candidate_paths" != "396" \
    || "$expected_excluded_paths" != "10" \
    || "$expected_paths" != "386" \
    || "$expected_variants" != "767" \
    || "$expected_quickjs_passes" != "386" \
    || "$expected_runnable" != "767" \
    || "$expected_passes" != "767" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_public_instance_paths" != "305" \
    || "$expected_public_static_paths" != "51" \
    || "$expected_static_block_paths" != "54" \
    || "$expected_negative_paths" != "119" ]]; then
    echo "error: public-class-init baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(manifest_paths | wc -l | tr -d '[:space:]')
unique_paths=$(manifest_paths | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: public-class-init manifest cardinality drifted" >&2
    exit 1
fi
manifest_paths | LC_ALL=C sort -c
actual_manifest=$(manifest_paths | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" \
    || "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: public-class-init manifest content drifted" >&2
    exit 1
fi

actual_excluded_paths=$(excluded_paths | wc -l | tr -d '[:space:]')
unique_excluded_paths=$(excluded_paths | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
actual_excluded=$(excluded_paths | LC_ALL=C sort | sha256_stream)
if [[ "$actual_excluded_paths" != "$expected_excluded_paths" \
    || "$unique_excluded_paths" != "$expected_excluded_paths" \
    || "$actual_excluded" != "$expected_excluded" ]]; then
    echo "error: public-class-init exclusion set drifted" >&2
    exit 1
fi
if comm -12 \
    <(manifest_paths | LC_ALL=C sort) \
    <(excluded_paths | LC_ALL=C sort) \
    | grep -q .; then
    echo "error: public-class-init manifest admitted an audited exclusion" >&2
    exit 1
fi
actual_tagged_paths=$(tagged_paths | wc -l | tr -d '[:space:]')
candidate_inventory=$(derive_candidate_paths)
actual_candidate_paths=$(printf '%s\n' "$candidate_inventory" | wc -l | tr -d '[:space:]')
actual_candidate=$(printf '%s\n' "$candidate_inventory" | sha256_stream)
if [[ "$actual_tagged_paths" != "$expected_tagged_paths" \
    || "$actual_candidate_paths" != "$expected_candidate_paths" \
    || "$actual_candidate" != "$expected_candidate" ]]; then
    echo "error: public-class-init 396-path candidate inventory drifted" >&2
    exit 1
fi
diff -u \
    <(printf '%s\n' "$candidate_inventory") \
    <({ manifest_paths; excluded_paths; } | LC_ALL=C sort)

if [[ "$(sha256_file "$global_profile")" != "$expected_global_profile" \
    || "$(sha256_file "$admission_profile")" != "$expected_profile" ]]; then
    echo "error: public-class-init capability profile drifted" >&2
    exit 1
fi
if awk '
    /^\[features\]$/ { inside=1; next }
    /^\[/ { inside=0 }
    inside && ($0 == "class" || $0 == "class-fields-public" ||
        $0 == "class-static-fields-public" || $0 == "class-static-block") {
        found=1
    }
    END { exit !found }
' "$global_profile"; then
    echo "error: global Test262 profile must remain fail-closed for the R3g class surface" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(derive_features | LC_ALL=C sort -u)
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    <(derive_negative_tests | LC_ALL=C sort)

public_instance_paths=0
public_static_paths=0
static_block_paths=0
generated_paths=0
no_strict_paths=0
only_strict_paths=0
plain_paths=0
includes_paths=0
negative_paths=0
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned public-class-init path is missing: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    if ! grep -q '^features: \[' <<<"$metadata"; then
        echo "error: public-class-init path gained non-inline features metadata: $test_path" >&2
        exit 1
    fi
    if ! grep -Eq 'class-fields-public|class-static-fields-public|class-static-block' <<<"$metadata"; then
        echo "error: public-class-init path lost every target feature tag: $test_path" >&2
        exit 1
    fi
    if grep -Eq 'computed-property-names|class-(fields|methods|static-fields|static-methods)-private|async-functions|async-iteration|generators|top-level-await|explicit-resource-management|Proxy' <<<"$metadata"; then
        echo "error: public-class-init path crossed an audited adjacent-feature frontier: $test_path" >&2
        exit 1
    fi
    if grep -Eq '^flags:.*(module|raw|async)|^[[:space:]]*-[[:space:]]*(module|raw|async)[[:space:]]*$' <<<"$metadata"; then
        echo "error: public-class-init path flags left the audited scope: $test_path" >&2
        exit 1
    fi

    if grep -Eq '^features:.*(^|[,[[:space:]])class-fields-public([],[[:space:]]|$)' <<<"$metadata"; then
        public_instance_paths=$((public_instance_paths + 1))
    fi
    if grep -Eq '^features:.*(^|[,[[:space:]])class-static-fields-public([],[[:space:]]|$)' <<<"$metadata"; then
        public_static_paths=$((public_static_paths + 1))
    fi
    if grep -Eq '^features:.*(^|[,[[:space:]])class-static-block([],[[:space:]]|$)' <<<"$metadata"; then
        static_block_paths=$((static_block_paths + 1))
    fi

    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "")
            plain_paths=$((plain_paths + 1))
            ;;
        "flags: [generated]")
            generated_paths=$((generated_paths + 1))
            plain_paths=$((plain_paths + 1))
            ;;
        "flags: [generated, noStrict]")
            generated_paths=$((generated_paths + 1))
            no_strict_paths=$((no_strict_paths + 1))
            ;;
        "flags: [onlyStrict]")
            only_strict_paths=$((only_strict_paths + 1))
            ;;
        *)
            echo "error: public-class-init flags drifted: $test_path: $flag_line" >&2
            exit 1
            ;;
    esac

    if grep -Fq 'negative:' <<<"$metadata"; then
        if ! grep -Fq '  phase: parse' <<<"$metadata" \
            || ! grep -Fq '  type: SyntaxError' <<<"$metadata"; then
            echo "error: public-class-init negative provenance drifted: $test_path" >&2
            exit 1
        fi
        negative_paths=$((negative_paths + 1))
    fi
    if grep -q '^includes:' <<<"$metadata"; then
        if ! grep -Eq '^includes: \[propertyHelper\.js(, compareArray\.js)?\]$' <<<"$metadata"; then
            echo "error: public-class-init harness dependency drifted: $test_path" >&2
            exit 1
        fi
        includes_paths=$((includes_paths + 1))
    fi

    if program_body "$test_path" \
        | sed '/^[[:space:]]*\/\*/,/^[[:space:]]*\*\//d' \
        | grep -Eq '^[[:space:]]*(static[[:space:]]+)?async[[:space:]]|(^|[^[:alnum:]_$])function[[:space:]]*\*|^[[:space:]]*(static[[:space:]]+)?\*'; then
        echo "error: public-class-init path gained an untagged async/generator form: $test_path" >&2
        exit 1
    fi
done < <(manifest_paths)

if [[ "$public_instance_paths" != "$expected_public_instance_paths" \
    || "$public_static_paths" != "$expected_public_static_paths" \
    || "$static_block_paths" != "$expected_static_block_paths" \
    || "$generated_paths" != "286" \
    || "$no_strict_paths" != "4" \
    || "$only_strict_paths" != "1" \
    || "$plain_paths" != "381" \
    || "$includes_paths" != "102" \
    || "$negative_paths" != "$expected_negative_paths" ]]; then
    echo "error: public-class-init metadata composition drifted" >&2
    exit 1
fi

verify_excluded_sources
verify_quickjs_oracle
if "$check_only"; then
    printf 'public-class-init inputs verified: QuickJS %s passes %s paths; Oxide admission gate expands to %s variants\n' \
        "$expected_quickjs" "$expected_quickjs_passes" "$expected_variants"
    exit 0
fi

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
    echo "error: public-class-init report metadata drifted" >&2
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
    done < <(manifest_paths) | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort)

actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
expected_runner_summary="Test262: total=$expected_variants pass=$expected_passes fail=$expected_failures unsupported=$expected_unsupported skipped=$expected_skipped"
if [[ "$runner_summary" != "$expected_runner_summary" ]]; then
    echo "error: public-class-init runner summary drifted: $runner_summary" >&2
    exit 1
fi

if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: public-class-init Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'public-class-init Test262 gate is exact: %s/%s pass across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$expected_paths"
