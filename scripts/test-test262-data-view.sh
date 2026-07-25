#!/usr/bin/env bash
# Reproduce the checksum-bound pure DataView Test262 view-core gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-data-view-baseline.txt
manifest=tests/test262-data-view.txt
profile=tests/test262-data-view.conf
exclusions=tests/test262-data-view-exclusions.tsv
report=target/test262-data-view.tsv
json_report=target/test262-data-view.jsonl
oracle_log=target/test262-data-view-quickjs.log
workers=${TEST262_WORKERS:-8}
check_only=false

usage() {
    cat <<'EOF'
usage: scripts/test-test262-data-view.sh [--check]

With --check, rebuild and audit the frozen cohort and verify all 984 variants
against pinned QuickJS without running quickjs-oxide. With no option, also run
the checksum-bound quickjs-oxide gate; this requires a measured baseline.
EOF
}

case ${1:-} in
    "")
        ;;
    --check)
        check_only=true
        ;;
    -h|--help)
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
        echo "error: DataView baseline is missing exactly one $key entry: $baseline" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: DataView baseline contains an empty $key entry: $baseline" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    if [[ "$actual" != "$expected" ]]; then
        echo "error: DataView baseline $key drifted" >&2
        echo "expected: $expected" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

exclusion_paths() {
    awk -F'\t' 'NF && $1 !~ /^#/ { print $1 }' "$exclusions"
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
        inside && /^[[:space:]]*-[[:space:]]*/ {
            line=$0
            sub(/^[[:space:]]*-[[:space:]]*/, "", line)
            if (line != "") print line
            next
        }
        inside { exit }
    '
}

source_body() {
    local test_path=$1
    awk '
        /^\/\*---$/ { in_metadata=1; next }
        in_metadata && /^---\*\/$/ { in_metadata=0; next }
        !in_metadata { print }
    ' "$suite/$test_path"
}

concrete_typed_array_tokens() {
    local source_file=$1 constructor
    for constructor in \
        Uint8ClampedArray Int8Array Uint8Array Int16Array Uint16Array \
        Int32Array Uint32Array BigInt64Array BigUint64Array Float16Array \
        Float32Array Float64Array
    do
        if grep -Eq \
            "(^|[^[:alnum:]_$])${constructor}([^[:alnum:]_$]|$)" \
            "$source_file"; then
            printf '%s\n' "$constructor"
        fi
    done
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < <(manifest_paths)

    if ! (
        cd -- "$source_dir"
        # The QuickJS config selects its default mode, so `-a` must follow
        # `-c` to make sloppy/strict coverage explicit.
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}"
    ) >"$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        echo "error: pinned QuickJS could not execute the DataView cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$oracle_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_variants) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        echo "error: pinned QuickJS no longer passes all DataView variants" >&2
        exit 1
    fi
}

cd -- "$root"

if [[ ! -f "$baseline" ]]; then
    echo "error: DataView Test262 baseline is missing: $baseline" >&2
    exit 1
fi
for required in "$manifest" "$profile" "$exclusions"; do
    if [[ ! -f "$required" ]]; then
        echo "error: DataView Test262 gate input is missing: $required" >&2
        exit 1
    fi
done
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer, found: $workers" >&2
    exit 2
fi

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 \
    f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 \
    79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 \
    a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value oxide_profile_sha256 \
    485ea3baf6695767108fb9f7f346c3a82d5a3db000af4510d6d002b313990cc8
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value candidate_paths 578
expect_value candidate_sha256 \
    1df8f075f57cbcc2cf72f88835bbd08449fe2093bf8f5d33badc0148249db3ed
expect_value excluded_paths 86
expect_value exclusions_sha256 \
    feade99c881ad6763b2241d988ab4c95ff3a8b79ae51f6c3ddf0501b62fd9354
expect_value exclusions_file_sha256 \
    9cdc8a031c926dd59dc152b0cfb76bd97758d63d79703df86d162b3a7eec4f44
expect_value paths 492
expect_value variants 984
expect_value quickjs_variants 984
expect_value features 20
expect_value features_sha256 \
    ac47df305dba0ae0643e399d8008b0c044a9efc6dcdb32386f7524a0673794f9
expect_value includes 4
expect_value includes_sha256 \
    009fed6d039dfbc5df8954c7b33903cf3ec6228fd38e4bf7db490e3414305aff
expect_value manifest_sha256 \
    3475b4a32f0a5f0ab50d5cd4e4843a7c7a59365298ecabcc5986b3fdd3f697e2
expect_value manifest_file_sha256 \
    3475b4a32f0a5f0ab50d5cd4e4843a7c7a59365298ecabcc5986b3fdd3f697e2
expect_value keys_sha256 \
    07d60a25d9dcb8316d4602456931cedff7668df634a92ab11c6efe4798c3f90c
expect_value runnable 984

expected_quickjs=$(read_value quickjs)
expected_test262=$(read_value test262)
expected_patch=$(read_value test262_patch_sha256)
expected_config=$(read_value test262_config_sha256)
expected_metadata=$(read_value test262_metadata_sha256)
expected_profile=$(read_value oxide_profile_sha256)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
expected_timeout_ms=$(read_value timeout_ms)
expected_candidate_paths=$(read_value candidate_paths)
expected_candidate=$(read_value candidate_sha256)
expected_excluded_paths=$(read_value excluded_paths)
expected_exclusions=$(read_value exclusions_sha256)
expected_exclusions_file=$(read_value exclusions_file_sha256)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_quickjs_variants=$(read_value quickjs_variants)
expected_features=$(read_value features)
expected_features_hash=$(read_value features_sha256)
expected_includes=$(read_value includes)
expected_includes_hash=$(read_value includes_sha256)
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_keys=$(read_value keys_sha256)
expected_runnable=$(read_value runnable)

if [[ "$check_only" == false ]]; then
    pending_keys=$(awk -F= '$2 == "PENDING" { print $1 }' "$baseline")
    if [[ -n "$pending_keys" ]]; then
        echo "error: DataView Test262 baseline still contains PENDING measured values" >&2
        printf '%s\n' "$pending_keys" | sed 's/^/  /' >&2
        echo "error: run --check first, then record an all-green quickjs-oxide report" >&2
        exit 1
    fi
fi

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ "$(basename -- "$source_dir")" != "quickjs-$expected_quickjs" \
    || "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" != "$expected_test262" \
    || "$(sha256_file "$source_dir/tests/test262.patch")" != "$expected_patch" \
    || "$(sha256_file "$source_dir/test262.conf")" != "$expected_config" ]]; then
    echo "error: prepared QuickJS/Test262 inputs drifted from the DataView baseline" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-data-view.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
manifest_inventory=$tmp_dir/manifest.txt
excluded_inventory=$tmp_dir/excluded.txt
candidate_inventory=$tmp_dir/candidate.txt
combined_inventory=$tmp_dir/combined.txt
derived_exclusions=$tmp_dir/derived-exclusions.tsv
derived_manifest=$tmp_dir/derived-manifest.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
variant_keys=$tmp_dir/variant-keys.txt
source_file=$tmp_dir/source-body.js
candidate_features=$tmp_dir/candidate-features.txt
candidate_includes=$tmp_dir/candidate-includes.txt
typed_array_tokens=$tmp_dir/typed-array-tokens.txt

manifest_paths >"$manifest_inventory"
exclusion_paths >"$excluded_inventory"
LC_ALL=C sort -c "$manifest_inventory"
LC_ALL=C sort -c "$excluded_inventory"

actual_paths=$(wc -l <"$manifest_inventory" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest_inventory" | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" \
    || "$(sha256_stream <"$manifest_inventory")" != "$expected_manifest" \
    || "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: DataView manifest cardinality or content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: DataView scoped capability profile drifted" >&2
    exit 1
fi

if ! awk -F'\t' '
    NR == 1 {
        if ($1 != "# path" || $2 != "reason" || NF != 2) exit 1
        next
    }
    {
        if (NF != 2 || $1 == "") exit 1
        if ($2 != "metadata declares TypedArray" &&
            $2 != "metadata declares SharedArrayBuffer" &&
            $2 != "metadata declares immutable-arraybuffer" &&
            $2 != "metadata declares cross-realm" &&
            $2 != "metadata declares Int8Array" &&
            $2 != "metadata declares Uint8Array" &&
            $2 != "source directly uses Int8Array without declaring TypedArray metadata") {
            exit 1
        }
        counts[$2]++
    }
    END {
        if (NR != 87 ||
            counts["metadata declares TypedArray"] != 5 ||
            counts["metadata declares SharedArrayBuffer"] != 39 ||
            counts["metadata declares immutable-arraybuffer"] != 11 ||
            counts["metadata declares cross-realm"] != 1 ||
            counts["metadata declares Int8Array"] != 24 ||
            counts["metadata declares Uint8Array"] != 5 ||
            counts["source directly uses Int8Array without declaring TypedArray metadata"] != 1) {
            exit 1
        }
    }
' "$exclusions"; then
    echo "error: DataView exclusion ledger format or reason inventory drifted" >&2
    exit 1
fi
actual_excluded_paths=$(wc -l <"$excluded_inventory" | tr -d '[:space:]')
unique_excluded_paths=$(LC_ALL=C sort -u "$excluded_inventory" \
    | wc -l | tr -d '[:space:]')
if [[ "$actual_excluded_paths" != "$expected_excluded_paths" \
    || "$unique_excluded_paths" != "$expected_excluded_paths" \
    || "$(sha256_stream <"$excluded_inventory")" != "$expected_exclusions" \
    || "$(sha256_file "$exclusions")" != "$expected_exclusions_file" ]]; then
    echo "error: DataView exclusion ledger cardinality or content drifted" >&2
    exit 1
fi
if [[ -n "$(comm -12 "$manifest_inventory" "$excluded_inventory")" ]]; then
    echo "error: DataView manifest overlaps its exclusion ledger" >&2
    exit 1
fi

(
    cd -- "$suite"
    find test/built-ins/DataView \
        -type f -name '*.js' ! -name '*_FIXTURE.js' -print
    find test/built-ins/ArrayBuffer/isView \
        -type f -name '*.js' ! -name '*_FIXTURE.js' -print
) | LC_ALL=C sort >"$candidate_inventory"
if [[ "$(wc -l <"$candidate_inventory" | tr -d '[:space:]')" \
        != "$expected_candidate_paths" \
    || "$(LC_ALL=C sort -u "$candidate_inventory" | wc -l | tr -d '[:space:]')" \
        != "$expected_candidate_paths" \
    || "$(sha256_file "$candidate_inventory")" != "$expected_candidate" ]]; then
    echo "error: DataView 578-path candidate inventory drifted" >&2
    exit 1
fi
LC_ALL=C sort -u "$manifest_inventory" "$excluded_inventory" >"$combined_inventory"
diff -u "$candidate_inventory" "$combined_inventory"

printf '# path\treason\n' >"$derived_exclusions"
: >"$derived_manifest"
: >"$feature_occurrences"
: >"$include_occurrences"
: >"$variant_keys"
while IFS= read -r test_path; do
    if [[ ( "$test_path" != test/built-ins/DataView/* \
            && "$test_path" != test/built-ins/ArrayBuffer/isView/* ) \
        || ! -f "$suite/$test_path" ]]; then
        echo "error: invalid or missing DataView candidate path: $test_path" >&2
        exit 1
    fi

    metadata=$(metadata_block "$test_path")
    if [[ -z "$metadata" \
        || "$(grep -c '^/\*---$' "$suite/$test_path" || true)" != "1" \
        || "$(grep -c '^---\*/$' "$suite/$test_path" || true)" != "1" ]]; then
        echo "error: DataView candidate lost a unique Test262 metadata block: $test_path" >&2
        exit 1
    fi

    metadata_list "$test_path" features >"$candidate_features"
    metadata_list "$test_path" includes >"$candidate_includes"
    source_body "$test_path" >"$source_file"
    concrete_typed_array_tokens "$source_file" >"$typed_array_tokens"

    reason=
    if [[ "$test_path" == test/built-ins/ArrayBuffer/isView/* ]] \
        && grep -Fxq TypedArray "$candidate_features"; then
        reason="metadata declares TypedArray"
    elif grep -Fxq SharedArrayBuffer "$candidate_features"; then
        reason="metadata declares SharedArrayBuffer"
    elif grep -Fxq immutable-arraybuffer "$candidate_features"; then
        reason="metadata declares immutable-arraybuffer"
    elif grep -Fxq cross-realm "$candidate_features"; then
        reason="metadata declares cross-realm"
    elif grep -Fxq Int8Array "$candidate_features"; then
        reason="metadata declares Int8Array"
    elif grep -Fxq Uint8Array "$candidate_features"; then
        reason="metadata declares Uint8Array"
    elif [[ -s "$typed_array_tokens" ]]; then
        if [[ "$(tr '\n' ' ' <"$typed_array_tokens" | sed 's/ $//')" != "Int8Array" ]]; then
            echo "error: unexpected latent TypedArray source dependency: $test_path" >&2
            sed 's/^/  /' "$typed_array_tokens" >&2
            exit 1
        fi
        reason="source directly uses Int8Array without declaring TypedArray metadata"
    fi

    if [[ -n "$reason" ]]; then
        case "$reason" in
            "metadata declares TypedArray")
                if ! grep -Fxq testTypedArray.js "$candidate_includes" \
                    || ! grep -Fq testWithTypedArrayConstructors "$source_file"; then
                    echo "error: TypedArray exclusion lost its harness/source dependency: $test_path" >&2
                    exit 1
                fi
                ;;
            "metadata declares SharedArrayBuffer")
                if ! grep -Eq \
                    '(^|[^[:alnum:]_$])SharedArrayBuffer([^[:alnum:]_$]|$)' \
                    "$source_file"; then
                    echo "error: SharedArrayBuffer exclusion lost its source dependency: $test_path" >&2
                    exit 1
                fi
                ;;
            "metadata declares immutable-arraybuffer")
                if ! grep -Fq transferToImmutable "$source_file"; then
                    echo "error: immutable-buffer exclusion lost its source dependency: $test_path" >&2
                    exit 1
                fi
                ;;
            "metadata declares cross-realm")
                if ! grep -Fq '$262.createRealm' "$source_file"; then
                    echo "error: cross-realm exclusion lost its source dependency: $test_path" >&2
                    exit 1
                fi
                ;;
            "metadata declares Int8Array")
                if ! grep -Fxq Int8Array "$typed_array_tokens"; then
                    echo "error: Int8Array exclusion lost its source dependency: $test_path" >&2
                    exit 1
                fi
                ;;
            "metadata declares Uint8Array")
                if ! grep -Fxq Uint8Array "$typed_array_tokens"; then
                    echo "error: Uint8Array exclusion lost its source dependency: $test_path" >&2
                    exit 1
                fi
                ;;
            "source directly uses Int8Array without declaring TypedArray metadata")
                if grep -Eq '^(TypedArray|Int8Array)$' "$candidate_features"; then
                    echo "error: latent Int8Array exclusion now declares its dependency: $test_path" >&2
                    exit 1
                fi
                ;;
        esac
        printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusions"
        continue
    fi

    if grep -Eq \
        '^(TypedArray|SharedArrayBuffer|immutable-arraybuffer|cross-realm|Int8Array|Uint8Array)$' \
        "$candidate_features"; then
        echo "error: DataView manifest retained an excluded metadata dependency: $test_path" >&2
        exit 1
    fi
    if [[ -s "$typed_array_tokens" ]] \
        || grep -Eq \
            '(^|[^[:alnum:]_$])SharedArrayBuffer([^[:alnum:]_$]|$)' \
            "$source_file" \
        || grep -Fq transferToImmutable "$source_file" \
        || grep -Fq '$262.createRealm' "$source_file" \
        || grep -Fxq testTypedArray.js "$candidate_includes"; then
        echo "error: DataView manifest retained a source or harness dependency: $test_path" >&2
        exit 1
    fi
    if grep -Fq 'negative:' <<<"$metadata"; then
        echo "error: DataView all-green cohort gained a negative test: $test_path" >&2
        exit 1
    fi
    if [[ -n "$(metadata_list "$test_path" flags)" ]]; then
        echo "error: DataView cohort gained non-default variant flags: $test_path" >&2
        exit 1
    fi

    printf '%s\n' "$test_path" >>"$derived_manifest"
    cat "$candidate_features" >>"$feature_occurrences"
    cat "$candidate_includes" >>"$include_occurrences"
    printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path" >>"$variant_keys"
done <"$candidate_inventory"

diff -u "$manifest_inventory" "$derived_manifest"
diff -u "$exclusions" "$derived_exclusions"

LC_ALL=C sort -u "$feature_occurrences" >"$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" >"$include_inventory"
LC_ALL=C sort -o "$variant_keys" "$variant_keys"
if [[ "$(wc -l <"$feature_inventory" | tr -d '[:space:]')" != "$expected_features" \
    || "$(sha256_file "$feature_inventory")" != "$expected_features_hash" ]]; then
    echo "error: DataView Test262 feature inventory drifted" >&2
    exit 1
fi
if [[ "$(wc -l <"$include_inventory" | tr -d '[:space:]')" != "$expected_includes" \
    || "$(sha256_file "$include_inventory")" != "$expected_includes_hash" ]]; then
    echo "error: DataView Test262 include inventory drifted" >&2
    exit 1
fi
if grep -Fxq testTypedArray.js "$include_inventory"; then
    echo "error: DataView manifest retained testTypedArray.js" >&2
    exit 1
fi
if [[ "$(wc -l <"$variant_keys" | tr -d '[:space:]')" != "$expected_variants" \
    || "$(sha256_file "$variant_keys")" != "$expected_keys" ]]; then
    echo "error: DataView Test262 path/variant key stream drifted" >&2
    exit 1
fi
diff -u <(profile_section features | LC_ALL=C sort) "$feature_inventory"
if [[ -n "$(profile_section audited-negative-tests)" \
    || -n "$(profile_section execution)" ]]; then
    echo "error: DataView scoped profile must contain neither negatives nor execution opt-ins" >&2
    exit 1
fi

verify_quickjs_oracle

if [[ "$check_only" == true ]]; then
    printf 'DataView Test262 assets pass: %s paths, %s exclusions, %s variants; pinned QuickJS passes %s/%s\n' \
        "$expected_paths" \
        "$expected_excluded_paths" \
        "$expected_variants" \
        "$expected_quickjs_variants" \
        "$expected_quickjs_variants"
    exit 0
fi

expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)
if [[ "$expected_passes" != "$expected_variants" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_nonpass" \
        != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" \
    || "$expected_summary" != "pass=$expected_variants" ]]; then
    echo "error: DataView measured baseline is not an all-green 984-variant gate" >&2
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
    || "$actual_runnable" != "$expected_runnable" ]]; then
    echo "error: DataView Test262 report metadata drifted" >&2
    exit 1
fi

diff -u \
    "$manifest_inventory" \
    <(awk -F'\t' \
        '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$report" | LC_ALL=C sort -u)
diff -u \
    "$feature_inventory" \
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
actual_passes=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ }
    END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" \
    | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
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
    echo "error: DataView Test262 classified vector drifted" >&2
    printf 'path\tvariant\toutcome\tactual_phase\tactual_type\tdetail\n' >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            if (++shown == 80) exit
        }
    ' "$report" >&2
    exit 1
fi

printf 'DataView Test262 gate passes: %s/%s variants across %s paths; pinned QuickJS passes %s/%s\n' \
    "$expected_passes" \
    "$expected_variants" \
    "$expected_paths" \
    "$expected_quickjs_variants" \
    "$expected_quickjs_variants"
