#!/usr/bin/env bash
# Reproduce the checksum-bound WeakMap/WeakSet Test262 milestone gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-weak-collections-baseline.txt
manifest=tests/test262-weak-collections.txt
profile=tests/test262-weak-collections.conf
global_profile=compat/test262-oxide.conf
typed_array_exclusions=tests/test262-typed-array-core-exclusions.tsv
report=target/test262-weak-collections.tsv
json_report=target/test262-weak-collections.jsonl
oracle_log=target/test262-weak-collections-quickjs.log
workers=${TEST262_WORKERS:-8}
run_oxide=1

usage() {
    printf 'usage: %s [--check]\n' "$0" >&2
    printf '  --check  verify the frozen universe, TypedArray audit, and pinned QuickJS only\n' >&2
}

case ${1:-} in
    '') ;;
    --check) run_oxide=0 ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
esac
if [[ $# -gt 1 ]]; then
    usage
    exit 2
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    else
        shasum -a 256 | awk '{ print $1 }'
    fi
}

read_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found++ }
        END { if (found != 1) exit 1 }
    ' "$baseline"); then
        echo "error: Weak collections baseline is missing exactly one $key entry" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: Weak collections baseline contains an empty $key entry" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

profile_section() {
    local file=$1 section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$file"
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

core_candidate_paths() {
    {
        find "$suite/test/built-ins/WeakMap" \
            "$suite/test/built-ins/WeakSet" \
            -type f -name '*.js' ! -name '*_FIXTURE.js'
        printf '%s\n' \
            "$suite/test/built-ins/Object/seal/seal-weakmap.js" \
            "$suite/test/built-ins/Object/seal/seal-weakset.js" \
            "$suite/test/staging/sm/WeakMap/symbols.js" \
            "$suite/test/staging/sm/extensions/weakmap.js" \
            "$suite/test/staging/sm/regress/regress-1507322-deep-weakmap.js"
    } | sed "s#^$suite/##" | LC_ALL=C sort -u
}

tagged_weak_collection_paths() {
    find "$suite/test" -type f -name '*.js' ! -name '*_FIXTURE.js' \
        -exec awk '
            FNR == 1 { in_metadata=0; in_features=0; weak=0 }
            $0 == "/*---" { in_metadata=1; next }
            in_metadata && $0 == "---*/" {
                if (weak) print FILENAME
                in_metadata=0
                in_features=0
                next
            }
            !in_metadata { next }
            /^features:[[:space:]]*\[/ {
                line=$0
                sub(/^features:[[:space:]]*\[/, "", line)
                sub(/\][[:space:]]*$/, "", line)
                count=split(line, values, /,[[:space:]]*/)
                for (i=1; i <= count; i++) {
                    if (values[i] == "WeakMap" || values[i] == "WeakSet") {
                        weak=1
                    }
                }
                in_features=0
                next
            }
            /^features:[[:space:]]*$/ { in_features=1; next }
            /^[[:alnum:]_-]+:/ { in_features=0 }
            in_features \
                && /^[[:space:]]*-[[:space:]]*(WeakMap|WeakSet)[[:space:]]*$/ {
                weak=1
            }
        ' {} + | sed "s#^$suite/##" | LC_ALL=C sort -u
}

candidate_paths() {
    {
        core_candidate_paths
        tagged_weak_collection_paths
    } | LC_ALL=C sort -u
}

metadata_block() {
    local test_path=$1
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path"
}

metadata_list() {
    local test_path=$1 key=$2
    metadata_block "$test_path" | awk -v key="$key" '
        $0 ~ ("^" key ":[[:space:]]*\\[") {
            line=$0
            sub("^[^:]+:[[:space:]]*\\[", "", line)
            sub("\\][[:space:]]*$", "", line)
            count=split(line, values, /,[[:space:]]*/)
            for (i=1; i <= count; i++) {
                if (values[i] != "") print values[i]
            }
            exit
        }
        $0 == key ":" { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]+/ {
            line=$0
            sub(/^[[:space:]]*-[[:space:]]+/, "", line)
            if (line != "") print line
            next
        }
        inside { exit }
    '
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done <"$manifest_inventory"

    if ! (
        cd -- "$source_dir"
        # QuickJS config chooses a default mode, so -a must follow -c to make
        # sloppy/strict coverage explicit while preserving onlyStrict paths.
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}"
    ) >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$root/$oracle_log" >&2
        echo "error: pinned QuickJS could not execute the Weak collections cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
            "$root/$oracle_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_variants) tests:" \
            "$root/$oracle_log"; then
        tail -n 100 "$root/$oracle_log" >&2
        echo "error: pinned QuickJS no longer has a zero-failure Weak collections receipt" >&2
        exit 1
    fi
}

cd -- "$root"

for required in \
    "$baseline" "$manifest" "$profile" "$global_profile" \
    "$typed_array_exclusions"; do
    if [[ ! -f "$required" ]]; then
        echo "error: Weak collections gate input is missing: $required" >&2
        exit 1
    fi
done
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer, found: $workers" >&2
    exit 2
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
expected_core_candidate_paths=$(read_value core_candidate_paths)
expected_core_candidate=$(read_value core_candidate_sha256)
expected_tagged_paths=$(read_value tagged_paths)
expected_tagged=$(read_value tagged_sha256)
expected_tagged_external_paths=$(read_value tagged_external_paths)
expected_tagged_external=$(read_value tagged_external_sha256)
expected_candidate_paths=$(read_value candidate_paths)
expected_candidate=$(read_value candidate_sha256)
expected_excluded_paths=$(read_value excluded_paths)
expected_exclusions=$(read_value exclusions_sha256)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_quickjs_variants=$(read_value quickjs_variants)
expected_features=$(read_value features)
expected_features_hash=$(read_value features_sha256)
expected_includes=$(read_value includes)
expected_includes_hash=$(read_value includes_sha256)
expected_only_strict_paths=$(read_value only_strict_paths)
expected_only_strict_hash=$(read_value only_strict_sha256)
expected_generated_paths=$(read_value generated_paths)
expected_generated_hash=$(read_value generated_sha256)
expected_typed_array_audit_paths=$(read_value typed_array_audit_paths)
expected_typed_array_audit=$(read_value typed_array_audit_sha256)
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_keys=$(read_value keys_sha256)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "a23cfb3270eb40eb3839413f3dacaf75fee2cecaca9d1b0ecc40d2c6c3c804c1" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_core_candidate_paths" != "231" \
    || "$expected_core_candidate" != "c5786b80dae58beb0dbf5159b18f8f9962c9f5c5bdaee0dda699d8f27e84536c" \
    || "$expected_tagged_paths" != "110" \
    || "$expected_tagged" != "319888cb3634034c22038c5bf68fe042fc75a1ae463373e48c82ce46d51a51a5" \
    || "$expected_tagged_external_paths" != "33" \
    || "$expected_tagged_external" != "51b108cd6a4334d0e8e4b4ee48daacf3034de748ba0216c8bd079549f175f46a" \
    || "$expected_candidate_paths" != "264" \
    || "$expected_candidate" != "9ef69530aa5c034bcbf5055da7217ba896a8660468e7a7593ce24b219f4403dc" \
    || "$expected_excluded_paths" != "7" \
    || "$expected_exclusions" != "1659358cd874efa645d6c0ef5b6e1666496790c1e30a45b9d51366db90ee8570" \
    || "$expected_paths" != "257" \
    || "$expected_variants" != "513" \
    || "$expected_quickjs_variants" != "513" \
    || "$expected_features" != "11" \
    || "$expected_features_hash" != "e158f7b746c66870c9109ee9829ebf81d024574e3346ff035e974db8316122e3" \
    || "$expected_includes" != "3" \
    || "$expected_includes_hash" != "4668b897190c3996c2141090fb75cc70a398fec78262fa08f96c8826acfe6f40" \
    || "$expected_only_strict_paths" != "1" \
    || "$expected_only_strict_hash" != "c282262c35918d616ea1aa7aff376148255bff048e323fbe344100cbbd343cfb" \
    || "$expected_generated_paths" != "4" \
    || "$expected_generated_hash" != "dfbba072c6c953b1e24e909499b11151dbe2368a23c80e1223f7a81c9dc70ed2" \
    || "$expected_typed_array_audit_paths" != "29" \
    || "$expected_typed_array_audit" != "3d96b98d0b986dd035a06f855e5747b93c4996430f12cddf771efcd820c7452d" \
    || "$expected_manifest" != "d31a2560c687a2214ad7c69e121248321cce0431f5d3c8317f02fca07116fc82" \
    || "$expected_manifest_file" != "6189cde88a7fcb15222d536d19f3e8172be66e35de24f47107e0c67910b92b7a" \
    || "$expected_keys" != "8690431650495da67496b470f5b2cdd7b10a343a3e8dffe56635657f49365a10" \
    || "$expected_runnable" != "513" \
    || "$expected_passes" != "513" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_nonpass" != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" \
    || "$expected_summary" != "pass=513" ]]; then
    echo "error: Weak collections baseline identity drifted" >&2
    exit 1
fi

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ "$(basename -- "$source_dir")" != "quickjs-$expected_quickjs" \
    || "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" != "$expected_test262" \
    || "$(sha256_file "$source_dir/tests/test262.patch")" != "$expected_patch" \
    || "$(sha256_file "$source_dir/test262.conf")" != "$expected_config" ]]; then
    echo "error: prepared QuickJS/Test262 inputs drifted from the Weak collections baseline" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-weak-collections.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
core_candidate_inventory=$tmp_dir/core-candidates.txt
tagged_inventory=$tmp_dir/tagged.txt
tagged_external_inventory=$tmp_dir/tagged-external.txt
candidate_inventory=$tmp_dir/candidates.txt
manifest_inventory=$tmp_dir/manifest.txt
derived_manifest=$tmp_dir/derived-manifest.txt
excluded_inventory=$tmp_dir/excluded.txt
dependency_features=$tmp_dir/dependency-features.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
only_strict_inventory=$tmp_dir/only-strict.txt
generated_inventory=$tmp_dir/generated.txt
variant_keys=$tmp_dir/variant-keys.txt
typed_array_audit=$tmp_dir/typed-array-audit.txt

core_candidate_paths >"$core_candidate_inventory"
tagged_weak_collection_paths >"$tagged_inventory"
LC_ALL=C comm -23 "$tagged_inventory" "$core_candidate_inventory" \
    >"$tagged_external_inventory"
{
    cat "$core_candidate_inventory"
    cat "$tagged_inventory"
} | LC_ALL=C sort -u >"$candidate_inventory"
manifest_paths >"$manifest_inventory"
LC_ALL=C sort -c "$core_candidate_inventory"
LC_ALL=C sort -c "$tagged_inventory"
LC_ALL=C sort -c "$tagged_external_inventory"
LC_ALL=C sort -c "$candidate_inventory"
LC_ALL=C sort -c "$manifest_inventory"

actual_core_candidate_paths=$(wc -l <"$core_candidate_inventory" | tr -d '[:space:]')
actual_tagged_paths=$(wc -l <"$tagged_inventory" | tr -d '[:space:]')
actual_tagged_external_paths=$(wc -l <"$tagged_external_inventory" \
    | tr -d '[:space:]')
actual_candidate_paths=$(wc -l <"$candidate_inventory" | tr -d '[:space:]')
actual_paths=$(wc -l <"$manifest_inventory" | tr -d '[:space:]')
if [[ "$actual_core_candidate_paths" != "$expected_core_candidate_paths" \
    || "$(sha256_file "$core_candidate_inventory")" != "$expected_core_candidate" \
    || "$actual_tagged_paths" != "$expected_tagged_paths" \
    || "$(sha256_file "$tagged_inventory")" != "$expected_tagged" \
    || "$actual_tagged_external_paths" != "$expected_tagged_external_paths" \
    || "$(sha256_file "$tagged_external_inventory")" \
        != "$expected_tagged_external" \
    || "$actual_candidate_paths" != "$expected_candidate_paths" \
    || "$(LC_ALL=C sort -u "$candidate_inventory" | wc -l | tr -d '[:space:]')" \
        != "$expected_candidate_paths" \
    || "$(sha256_file "$candidate_inventory")" != "$expected_candidate" \
    || "$actual_paths" != "$expected_paths" \
    || "$(LC_ALL=C sort -u "$manifest_inventory" | wc -l | tr -d '[:space:]')" \
        != "$expected_paths" \
    || "$(sha256_stream <"$manifest_inventory")" != "$expected_manifest" \
    || "$(sha256_file "$manifest")" != "$expected_manifest_file" \
    || "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: Weak collections core/tagged universe, manifest, or profile drifted" >&2
    exit 1
fi

{
    profile_section "$global_profile" features
    profile_section "$profile" features
} | LC_ALL=C sort -u >"$dependency_features"

: >"$derived_manifest"
: >"$excluded_inventory"
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: missing Weak collections candidate: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    if [[ -z "$metadata" ]]; then
        echo "error: Weak collections candidate lost metadata: $test_path" >&2
        exit 1
    fi
    missing=0
    while IFS= read -r feature; do
        if [[ -n "$feature" ]] && ! grep -Fxq "$feature" "$dependency_features"; then
            missing=1
        fi
    done < <(metadata_list "$test_path" features)
    if [[ "$missing" == 1 ]]; then
        printf '%s\n' "$test_path" >>"$excluded_inventory"
    else
        printf '%s\n' "$test_path" >>"$derived_manifest"
    fi
done <"$candidate_inventory"

if [[ "$(wc -l <"$excluded_inventory" | tr -d '[:space:]')" \
        != "$expected_excluded_paths" \
    || "$(sha256_file "$excluded_inventory")" != "$expected_exclusions" ]]; then
    echo "error: Weak collections metadata-derived dependency/host exclusion boundary drifted" >&2
    exit 1
fi
diff -u "$derived_manifest" "$manifest_inventory"

: >"$feature_occurrences"
: >"$include_occurrences"
: >"$only_strict_inventory"
: >"$generated_inventory"
: >"$variant_keys"
while IFS= read -r test_path; do
    metadata=$(metadata_block "$test_path")
    if grep -Fq 'negative:' <<<"$metadata"; then
        echo "error: all-green Weak collections cohort gained a negative test: $test_path" >&2
        exit 1
    fi
    metadata_list "$test_path" features >>"$feature_occurrences"
    metadata_list "$test_path" includes >>"$include_occurrences"
    flags=$(metadata_list "$test_path" flags | LC_ALL=C sort -u)
    case "$flags" in
        '')
            printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path" \
                >>"$variant_keys"
            ;;
        onlyStrict)
            printf '%s\n' "$test_path" >>"$only_strict_inventory"
            printf '%s\tstrict\n' "$test_path" >>"$variant_keys"
            ;;
        generated)
            printf '%s\n' "$test_path" >>"$generated_inventory"
            printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path" \
                >>"$variant_keys"
            ;;
        *)
            echo "error: Weak collections path gained unsupported flags: $test_path: $flags" >&2
            exit 1
            ;;
    esac
done <"$manifest_inventory"

LC_ALL=C sort -u "$feature_occurrences" >"$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" >"$include_inventory"
LC_ALL=C sort -o "$only_strict_inventory" "$only_strict_inventory"
LC_ALL=C sort -o "$generated_inventory" "$generated_inventory"
LC_ALL=C sort -o "$variant_keys" "$variant_keys"
if [[ "$(wc -l <"$feature_inventory" | tr -d '[:space:]')" != "$expected_features" \
    || "$(sha256_file "$feature_inventory")" != "$expected_features_hash" \
    || "$(wc -l <"$include_inventory" | tr -d '[:space:]')" != "$expected_includes" \
    || "$(sha256_file "$include_inventory")" != "$expected_includes_hash" \
    || "$(wc -l <"$only_strict_inventory" | tr -d '[:space:]')" \
        != "$expected_only_strict_paths" \
    || "$(sha256_file "$only_strict_inventory")" != "$expected_only_strict_hash" \
    || "$(wc -l <"$generated_inventory" | tr -d '[:space:]')" \
        != "$expected_generated_paths" \
    || "$(sha256_file "$generated_inventory")" != "$expected_generated_hash" \
    || "$(wc -l <"$variant_keys" | tr -d '[:space:]')" != "$expected_variants" \
    || "$(sha256_file "$variant_keys")" != "$expected_keys" ]]; then
    echo "error: Weak collections metadata inventory drifted" >&2
    exit 1
fi
diff -u <(profile_section "$profile" features | LC_ALL=C sort) "$feature_inventory"
if [[ -n "$(profile_section "$profile" audited-negative-tests)" \
    || -n "$(profile_section "$profile" execution)" ]]; then
    echo "error: Weak collections profile must not admit negatives or execution capabilities" >&2
    exit 1
fi

# These 29 paths are only a downstream audit. WeakMap removes the first hidden
# harness blocker in sm/non262-TypedArray-shell.js, but it does not prove the
# paths' TypedArray, SharedArrayBuffer, or other semantics. Keep them out of the
# Weak collections manifest and preserve their separate checksum receipt.
awk -F'\t' '
    $1 ~ /^test\/staging\/sm\/TypedArray\// \
        && ($2 == "external:WeakMap" || $2 == "external:SharedArrayBuffer") {
        print $1
    }
' "$typed_array_exclusions" | LC_ALL=C sort -u >"$typed_array_audit"
if [[ "$(wc -l <"$typed_array_audit" | tr -d '[:space:]')" \
        != "$expected_typed_array_audit_paths" \
    || "$(sha256_file "$typed_array_audit")" != "$expected_typed_array_audit" \
    || -n "$(comm -12 "$manifest_inventory" "$typed_array_audit")" ]] \
    || ! grep -Fq 'new WeakMap()' \
        "$suite/harness/sm/non262-TypedArray-shell.js"; then
    echo "error: 29-path TypedArray harness-unblocked audit drifted" >&2
    exit 1
fi
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]] \
        || ! metadata_list "$test_path" includes \
            | grep -Fxq 'sm/non262-TypedArray-shell.js'; then
        echo "error: invalid TypedArray harness-unblocked audit path: $test_path" >&2
        exit 1
    fi
done <"$typed_array_audit"

verify_quickjs_oracle

if [[ "$run_oxide" == 0 ]]; then
    printf 'Weak collections inputs verified: %s candidates - %s dependency/host paths = %s paths; QuickJS %s passes %s variants; %s TypedArray paths remain audit-only\n' \
        "$expected_candidate_paths" "$expected_excluded_paths" "$expected_paths" \
        "$expected_quickjs" "$expected_quickjs_variants" \
        "$expected_typed_array_audit_paths"
    exit 0
fi

if [[ "$expected_nonpass" == PENDING \
    || "$expected_tsv" == PENDING \
    || "$expected_jsonl" == PENDING ]]; then
    echo "error: Weak collections baseline still contains PENDING Oxide receipts" >&2
    exit 1
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

actual_variants=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { count++ } END { print count + 0 }' \
    "$report")
actual_passes=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } \
    END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } \
    END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } \
    END { print count + 0 }' "$report")
actual_failures=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") \
        && $7 != "pass" && $7 !~ /^unsupported-/ && $7 !~ /^skipped-/ { count++ } \
    END { print count + 0 }' "$report")
execution_line=$(printf '%s\n' "$run_output" \
    | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
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
    || "$actual_runnable" != "$expected_runnable" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" ]]; then
    echo "error: Weak collections classified report metadata drifted" >&2
    exit 1
fi

diff -u "$manifest_inventory" \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$report" | LC_ALL=C sort -u)
diff -u "$feature_inventory" \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i <= count; i++) {
                if (features[i] != "") print features[i]
            }
        }
    ' "$report" | LC_ALL=C sort -u)
actual_keys=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' \
    "$report" | LC_ALL=C sort | sha256_stream)
nonpass_count=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { count++ } \
    END { print count + 0 }' "$report")
actual_nonpass=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' "$report" | sha256_stream)
if [[ "$actual_keys" != "$expected_keys" \
    || "$nonpass_count" != "0" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$(tail -n 1 "$report")" != "# summary $expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: Weak collections zero-failure classified vector drifted" >&2
    if [[ "$nonpass_count" != 0 ]]; then
        printf 'path\tvariant\toutcome\tactual_phase\tactual_type\tdetail\n' >&2
        awk -F'\t' '
            !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
                print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            }
        ' "$report" >&2
    fi
    exit 1
fi

printf 'Weak collections Test262 gate passes: %s/%s variants across %s paths; %s TypedArray paths remain audit-only\n' \
    "$expected_passes" "$expected_variants" "$expected_paths" \
    "$expected_typed_array_audit_paths"
