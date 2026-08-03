#!/usr/bin/env bash
# Reproduce the focused resizable-arraybuffer certification gate.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)

baseline=tests/test262-resizable-arraybuffer-global-baseline.txt
focused_profile=tests/test262-resizable-arraybuffer.conf
parent_profile=tests/test262-resizable-arraybuffer-global-parent.conf
candidate_profile=tests/test262-resizable-arraybuffer-global-candidate.conf
universe_manifest=tests/test262-resizable-arraybuffer-universe.txt
activation_manifest=tests/test262-resizable-arraybuffer.txt
reason_only_manifest=tests/test262-resizable-arraybuffer-reason-only.txt
config_skipped_manifest=tests/test262-resizable-arraybuffer-config-skipped.txt
spillover_activation_manifest=tests/test262-resizable-arraybuffer-spillover-activation.txt
spillover_reason_only_manifest=tests/test262-resizable-arraybuffer-spillover-reason-only.txt

array_buffer_manifest=tests/test262-array-buffer.txt
array_buffer_baseline=tests/test262-array-buffer-baseline.txt
data_view_manifest=tests/test262-data-view.txt
data_view_baseline=tests/test262-data-view-baseline.txt
typed_array_manifest=tests/test262-typed-array-core.txt
typed_array_baseline=tests/test262-typed-array-core-baseline.txt
full_baseline=tests/test262-full-baseline.txt

report=target/test262-resizable-arraybuffer.tsv
json_report=target/test262-resizable-arraybuffer.jsonl
oracle_log=target/test262-resizable-arraybuffer-quickjs.log
workers=${TEST262_WORKERS:-8}

mode=run
case ${1-} in
    "") ;;
    --check) mode=check ;;
    --bless) mode=bless ;;
    -h | --help)
        cat <<'EOF'
usage: scripts/test-test262-resizable-arraybuffer.sh [--check|--bless]

  --check  rebuild every frozen partition, run pinned QuickJS over the same
           381-path manifest, and run the ArrayBuffer differential oracle
  --bless  additionally run quickjs-oxide and print execution receipt fields

With no option, the quickjs-oxide 762-variant report must reproduce the
execution receipt stored in the static baseline.
EOF
        exit 0
        ;;
    *)
        echo "error: unknown option: ${1-}" >&2
        exit 2
        ;;
esac
if [[ $# -gt 1 ]]; then
    echo "error: expected at most one option" >&2
    exit 2
fi
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer, found: $workers" >&2
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

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}

read_value_from() {
    local file=$1
    local key=$2
    local value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$file"); then
        echo "error: $file must contain exactly one $key entry" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: $file contains an empty $key entry" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

read_value() {
    read_value_from "$baseline" "$1"
}

expect_value() {
    local key=$1
    local expected=$2
    local actual
    actual=$(read_value "$key")
    if [[ "$actual" != "$expected" ]]; then
        printf 'error: resizable-arraybuffer baseline %s drifted: %s != %s\n' \
            "$key" "$actual" "$expected" >&2
        exit 1
    fi
}

profile_section() {
    local profile=$1
    local section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$1"
}

variant_keys() {
    local test_path
    while IFS= read -r test_path; do
        [[ -z "$test_path" ]] && continue
        printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path"
    done | sort
}

inventory_count() {
    wc -l <"$1" | tr -d '[:space:]'
}

verify_path_inventory() {
    local prefix=$1
    local inventory=$2
    local expected_count
    local expected_sha
    expected_count=$(read_value "${prefix}_paths")
    expected_sha=$(read_value "${prefix}_sha256")
    sort -c "$inventory"
    if [[ "$(inventory_count "$inventory")" != "$expected_count" \
        || "$(sort -u "$inventory" | wc -l | tr -d '[:space:]')" \
            != "$expected_count" \
        || "$(sha256_file "$inventory")" != "$expected_sha" ]]; then
        echo "error: $prefix path inventory drifted" >&2
        exit 1
    fi
}

verify_variant_inventory() {
    local prefix=$1
    local paths=$2
    local keys=$3
    local expected_variants
    local expected_keys
    expected_variants=$(read_value "${prefix}_variants")
    expected_keys=$(read_value "${prefix}_keys_sha256")
    variant_keys <"$paths" >"$keys"
    sort -c "$keys"
    if [[ "$(inventory_count "$keys")" != "$expected_variants" \
        || "$(sha256_file "$keys")" != "$expected_keys" ]]; then
        echo "error: $prefix variant inventory drifted" >&2
        exit 1
    fi
}

verify_complete_inventory() {
    local prefix=$1
    local paths=$2
    local keys=$3
    verify_path_inventory "$prefix" "$paths"
    verify_variant_inventory "$prefix" "$paths" "$keys"
}

read_header() {
    local file=$1
    local key=$2
    awk -F= -v key="# $key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$file"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' "$1"
}

report_summary() {
    tail -n 1 "$1" | sed 's/^# summary //'
}

execution_runnable() {
    printf '%s\n' "$1" | awk '
        /^execution: runnable=/ {
            sub(/^execution: runnable=/, "")
            sub(/ .*/, "")
            print
            found=1
        }
        END { if (!found) exit 1 }
    '
}

cleanup() {
    if [[ -n "${tmp_dir-}" && -d "$tmp_dir" ]]; then
        rm -rf -- "$tmp_dir"
    fi
}

cd -- "$root"

required_assets=(
    "$baseline"
    "$focused_profile"
    "$parent_profile"
    "$candidate_profile"
    "$universe_manifest"
    "$activation_manifest"
    "$reason_only_manifest"
    "$config_skipped_manifest"
    "$spillover_activation_manifest"
    "$spillover_reason_only_manifest"
    "$array_buffer_manifest"
    "$array_buffer_baseline"
    "$data_view_manifest"
    "$data_view_baseline"
    "$typed_array_manifest"
    "$typed_array_baseline"
    "$full_baseline"
)
for required in "${required_assets[@]}"; do
    if [[ ! -f "$required" ]]; then
        echo "error: resizable-arraybuffer gate input is missing: $required" >&2
        exit 1
    fi
done

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 \
    f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 \
    79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_records 53125
expect_value test262_metadata_sha256 \
    a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value parent_oxide_profile_sha256 \
    ed80ab5aed86c606a1d7b5c1854b78ab1bb3c517cf0c6898a89e9f8d19135000
expect_value candidate_oxide_profile_sha256 \
    e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898
expect_value scoped_oxide_profile_sha256 \
    e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898
expect_value parent_features 89
expect_value candidate_features 90
expect_value added_features 1
expect_value audited_negative_tests 828
expect_value universe_paths 463
expect_value universe_variants 926
expect_value activation_paths 381
expect_value activation_variants 762
expect_value quickjs_variants 762
expect_value focused_runnable 762
expect_value focused_passes 762
expect_value focused_failures 0
expect_value focused_unsupported 0
expect_value focused_skipped 0
expect_value focused_nonpass_sha256 \
    e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
expect_value focused_tsv_sha256 \
    79baa1c1e323cb1256f3e0f7bdfbc403f3732100f40be807f63dfed6d84ab70c
expect_value focused_jsonl_sha256 \
    8a0ed16786ae3ecec118e1fd84392cb6857fb3c9ecb57d6977e7b962ed8bb0da
expect_value focused_summary pass=762
expect_value reason_only_paths 80
expect_value reason_only_variants 160
expect_value config_skipped_paths 2
expect_value config_skipped_variants 4
expect_value certified_paths 312
expect_value certified_variants 624
expect_value certified_activation_paths 286
expect_value certified_activation_variants 572
expect_value certified_reason_only_paths 26
expect_value certified_reason_only_variants 52
expect_value spillover_paths 151
expect_value spillover_variants 302
expect_value spillover_activation_paths 95
expect_value spillover_activation_variants 190
expect_value spillover_reason_only_paths 54
expect_value spillover_reason_only_variants 108

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ "$(basename -- "$source_dir")" != "quickjs-$(read_value quickjs)" \
    || "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" \
        != "$(read_value test262)" \
    || "$(sha256_file "$source_dir/tests/test262.patch")" \
        != "$(read_value test262_patch_sha256)" \
    || "$(sha256_file "$source_dir/test262.conf")" \
        != "$(read_value test262_config_sha256)" ]]; then
    echo "error: prepared QuickJS/Test262 inputs drifted" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-rab.XXXXXX")
trap cleanup EXIT HUP INT TERM

parent_features_file=$tmp_dir/parent-features.txt
candidate_features_file=$tmp_dir/candidate-features.txt
parent_negatives_file=$tmp_dir/parent-negatives.txt
candidate_negatives_file=$tmp_dir/candidate-negatives.txt
parent_execution_file=$tmp_dir/parent-execution.txt
candidate_execution_file=$tmp_dir/candidate-execution.txt
added_features_file=$tmp_dir/added-features.txt

profile_section "$parent_profile" features >"$parent_features_file"
profile_section "$candidate_profile" features >"$candidate_features_file"
profile_section "$parent_profile" audited-negative-tests >"$parent_negatives_file"
profile_section "$candidate_profile" audited-negative-tests >"$candidate_negatives_file"
profile_section "$parent_profile" execution >"$parent_execution_file"
profile_section "$candidate_profile" execution >"$candidate_execution_file"
comm -13 "$parent_features_file" "$candidate_features_file" >"$added_features_file"

sort -c "$parent_features_file"
sort -c "$candidate_features_file"
sort -c "$parent_negatives_file"
sort -c "$candidate_negatives_file"
if [[ "$(sha256_file "$parent_profile")" \
        != "$(read_value parent_oxide_profile_sha256)" \
    || "$(sha256_file "$candidate_profile")" \
        != "$(read_value candidate_oxide_profile_sha256)" \
    || "$(sha256_file "$focused_profile")" \
        != "$(read_value scoped_oxide_profile_sha256)" \
    || "$(inventory_count "$parent_features_file")" \
        != "$(read_value parent_features)" \
    || "$(sha256_file "$parent_features_file")" \
        != "$(read_value parent_features_sha256)" \
    || "$(inventory_count "$candidate_features_file")" \
        != "$(read_value candidate_features)" \
    || "$(sha256_file "$candidate_features_file")" \
        != "$(read_value candidate_features_sha256)" \
    || "$(inventory_count "$added_features_file")" \
        != "$(read_value added_features)" \
    || "$(sha256_file "$added_features_file")" \
        != "$(read_value added_features_sha256)" \
    || "$(inventory_count "$candidate_negatives_file")" \
        != "$(read_value audited_negative_tests)" \
    || "$(sha256_file "$candidate_negatives_file")" \
        != "$(read_value audited_negative_tests_sha256)" ]]; then
    echo "error: resizable-arraybuffer profile identity drifted" >&2
    exit 1
fi
if [[ "$(cat "$added_features_file")" != "resizable-arraybuffer" ]]; then
    echo "error: focused profile must add only resizable-arraybuffer" >&2
    exit 1
fi
if [[ -n "$(comm -23 "$parent_features_file" "$candidate_features_file")" ]]; then
    echo "error: candidate profile removed a parent feature" >&2
    exit 1
fi
diff -u "$parent_negatives_file" "$candidate_negatives_file"
diff -u "$parent_execution_file" "$candidate_execution_file"
if [[ "$(cat "$candidate_execution_file")" != "async=true" ]]; then
    echo "error: resizable-arraybuffer profiles changed global execution policy" >&2
    exit 1
fi
cmp -s "$focused_profile" "$candidate_profile" || {
    echo "error: focused and candidate profiles must be byte-identical" >&2
    exit 1
}

if [[ "$(sha256_file "$array_buffer_manifest")" \
        != "$(read_value array_buffer_manifest_sha256)" \
    || "$(sha256_file "$array_buffer_baseline")" \
        != "$(read_value array_buffer_baseline_sha256)" \
    || "$(sha256_file "$data_view_manifest")" \
        != "$(read_value data_view_manifest_sha256)" \
    || "$(sha256_file "$data_view_baseline")" \
        != "$(read_value data_view_baseline_sha256)" \
    || "$(sha256_file "$typed_array_manifest")" \
        != "$(read_value typed_array_core_manifest_sha256)" \
    || "$(sha256_file "$typed_array_baseline")" \
        != "$(read_value typed_array_core_baseline_sha256)" ]]; then
    echo "error: an antecedent certification asset drifted" >&2
    exit 1
fi
if [[ "$(read_value_from "$array_buffer_baseline" summary)" != "pass=288" \
    || "$(read_value_from "$data_view_baseline" summary)" != "pass=984" \
    || "$(read_value_from "$typed_array_baseline" summary)" != "pass=4463" \
    || "$(read_value_from "$full_baseline" tsv_sha256)" \
        != "$(read_value parent_full_tsv_sha256)" \
    || "$(read_value_from "$full_baseline" jsonl_sha256)" \
        != "$(read_value parent_full_jsonl_sha256)" ]]; then
    echo "error: antecedent all-green or parent full receipt drifted" >&2
    exit 1
fi

metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --validate-metadata "$metadata_records"
if [[ "$(sha256_file "$metadata_records")" \
    != "$(read_value test262_metadata_sha256)" ]]; then
    echo "error: exhaustive Test262 metadata fingerprint drifted" >&2
    exit 1
fi
tr '\0' '\t' <"$metadata_records" >"$metadata_tsv"
if [[ "$(inventory_count "$metadata_tsv")" \
    != "$(read_value test262_metadata_records)" ]]; then
    echo "error: exhaustive Test262 metadata record count drifted" >&2
    exit 1
fi

generated_universe=$tmp_dir/universe.txt
generated_activation=$tmp_dir/activation.txt
generated_reason_only=$tmp_dir/reason-only.txt
generated_config_skipped=$tmp_dir/config-skipped.txt
skipped_features=$tmp_dir/skipped-features.txt

awk '
    $0 == "[features]" { inside=1; next }
    /^\[/ { inside=0 }
    inside && NF && $1 !~ /^#/ && /=skip$/ {
        sub(/=skip$/, "")
        print
    }
' "$source_dir/test262.conf" | sort -u >"$skipped_features"

awk -F'\t' \
    -v universe="$generated_universe" \
    -v activation="$generated_activation" \
    -v reason_only="$generated_reason_only" \
    -v config_skipped="$generated_config_skipped" '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    FILENAME == ARGV[1] { parent[$1]=1; next }
    FILENAME == ARGV[2] { candidate[$1]=1; next }
    FILENAME == ARGV[3] { skipped[$1]=1; next }
    !has($4, "resizable-arraybuffer") { next }
    {
        print $1 > universe
        config_skip=0
        parent_missing=0
        candidate_missing=0
        n=split($4, feature, ",")
        for (i=1; i<=n; i++) {
            if (feature[i] in skipped) config_skip=1
            if (!(feature[i] in parent)) parent_missing++
            if (!(feature[i] in candidate)) candidate_missing++
        }
        if (config_skip) {
            print $1 > config_skipped
            next
        }
        if (!has($4, "resizable-arraybuffer") || parent_missing < 1 ||
            candidate_missing != parent_missing - 1) {
            printf "bad profile dependency partition: %s\n", $1 > "/dev/stderr"
            bad=1
            next
        }
        if (parent_missing == 1 && candidate_missing == 0) {
            print $1 > activation
        } else {
            print $1 > reason_only
        }
    }
    END { if (bad) exit 1 }
' "$parent_features_file" "$candidate_features_file" "$skipped_features" \
    "$metadata_tsv"

for generated in \
    "$generated_universe" "$generated_activation" \
    "$generated_reason_only" "$generated_config_skipped"
do
    sort -o "$generated" "$generated"
done

diff -u "$universe_manifest" "$generated_universe"
diff -u "$activation_manifest" "$generated_activation"
diff -u "$reason_only_manifest" "$generated_reason_only"
diff -u "$config_skipped_manifest" "$generated_config_skipped"

universe_keys=$tmp_dir/universe.keys
activation_keys=$tmp_dir/activation.keys
reason_only_keys=$tmp_dir/reason-only.keys
config_skipped_keys=$tmp_dir/config-skipped.keys
verify_complete_inventory universe "$generated_universe" "$universe_keys"
verify_complete_inventory activation "$generated_activation" "$activation_keys"
verify_complete_inventory reason_only "$generated_reason_only" "$reason_only_keys"
verify_complete_inventory config_skipped "$generated_config_skipped" \
    "$config_skipped_keys"

top_level_union=$tmp_dir/top-level-union.txt
cat "$generated_activation" "$generated_reason_only" \
    "$generated_config_skipped" | sort >"$top_level_union"
if [[ -n "$(uniq -d "$top_level_union")" ]]; then
    echo "error: activation, reason-only, and config-skip partitions overlap" >&2
    exit 1
fi
diff -u "$generated_universe" "$top_level_union"

universe_metadata=$tmp_dir/universe-metadata.tsv
awk -F'\t' '
    NR == FNR { wanted[$1]=1; next }
    $1 in wanted { print }
' "$generated_universe" "$metadata_tsv" >"$universe_metadata"
if awk -F'\t' '$3 != "" || $5 != "" || $6 != "" { print; bad=1 }
    END { exit bad ? 0 : 1 }' "$universe_metadata" >&2; then
    echo "error: resizable-arraybuffer universe gained flags or negative metadata" >&2
    exit 1
fi

include_counts=$tmp_dir/include-counts.txt
awk -F'\t' '
    {
        n=split($2, item, ",")
        for (i=1; i<=n; i++) if (item[i] != "") count[item[i]]++
    }
    END { for (name in count) print name "=" count[name] }
' "$universe_metadata" | sort >"$include_counts"
cat >"$tmp_dir/expected-include-counts.txt" <<'EOF'
compareArray.js=153
detachArrayBuffer.js=5
isConstructor.js=2
propertyHelper.js=18
resizableArrayBufferUtils.js=188
testTypedArray.js=112
EOF
diff -u "$tmp_dir/expected-include-counts.txt" "$include_counts"

quickjs_exclusions=$tmp_dir/quickjs-exclusions.txt
config_excluded=$tmp_dir/config-excluded.txt
awk '
    $0 == "[exclude]" { inside=1; next }
    /^\[/ { inside=0 }
    inside && NF && $1 !~ /^#/ {
        sub(/^test262\//, "")
        sub(/\/$/, "")
        print
    }
' "$source_dir/test262.conf" | sort -u >"$quickjs_exclusions"
awk '
    NR == FNR { excluded[++count]=$0; next }
    {
        for (i=1; i<=count; i++) {
            if ($0 == excluded[i] || index($0, excluded[i] "/") == 1) {
                print
                break
            }
        }
    }
' "$quickjs_exclusions" "$generated_activation" >"$config_excluded"
if [[ -s "$config_excluded" ]]; then
    echo "error: focused activation intersects QuickJS config exclusions" >&2
    cat "$config_excluded" >&2
    exit 1
fi

array_buffer_paths=$tmp_dir/array-buffer.txt
data_view_paths=$tmp_dir/data-view.txt
typed_array_paths=$tmp_dir/typed-array.txt
certification_union=$tmp_dir/certification-union.txt
certified=$tmp_dir/certified.txt
certified_activation=$tmp_dir/certified-activation.txt
certified_reason_only=$tmp_dir/certified-reason-only.txt
spillover=$tmp_dir/spillover.txt
spillover_activation=$tmp_dir/spillover-activation.txt
spillover_reason_only=$tmp_dir/spillover-reason-only.txt
spillover_config_skipped=$tmp_dir/spillover-config-skipped.txt

manifest_paths "$array_buffer_manifest" >"$array_buffer_paths"
manifest_paths "$data_view_manifest" >"$data_view_paths"
manifest_paths "$typed_array_manifest" >"$typed_array_paths"
cat "$array_buffer_paths" "$data_view_paths" "$typed_array_paths" \
    | sort -u >"$certification_union"
comm -12 "$generated_universe" "$certification_union" >"$certified"
comm -12 "$generated_activation" "$certified" >"$certified_activation"
comm -12 "$generated_reason_only" "$certified" >"$certified_reason_only"
comm -23 "$generated_universe" "$certified" >"$spillover"
comm -12 "$generated_activation" "$spillover" >"$spillover_activation"
comm -12 "$generated_reason_only" "$spillover" >"$spillover_reason_only"
comm -12 "$generated_config_skipped" "$spillover" \
    >"$spillover_config_skipped"

diff -u "$spillover_activation_manifest" "$spillover_activation"
diff -u "$spillover_reason_only_manifest" "$spillover_reason_only"
diff -u "$config_skipped_manifest" "$spillover_config_skipped"

certified_keys=$tmp_dir/certified.keys
certified_activation_keys=$tmp_dir/certified-activation.keys
certified_reason_only_keys=$tmp_dir/certified-reason-only.keys
spillover_keys=$tmp_dir/spillover.keys
spillover_activation_keys=$tmp_dir/spillover-activation.keys
spillover_reason_only_keys=$tmp_dir/spillover-reason-only.keys
verify_complete_inventory certified "$certified" "$certified_keys"
verify_complete_inventory certified_activation "$certified_activation" \
    "$certified_activation_keys"
verify_complete_inventory certified_reason_only "$certified_reason_only" \
    "$certified_reason_only_keys"
verify_complete_inventory spillover "$spillover" "$spillover_keys"
verify_complete_inventory spillover_activation "$spillover_activation" \
    "$spillover_activation_keys"
verify_complete_inventory spillover_reason_only "$spillover_reason_only" \
    "$spillover_reason_only_keys"

certified_activation_array_buffer=$tmp_dir/certified-activation-array-buffer.txt
certified_activation_data_view=$tmp_dir/certified-activation-data-view.txt
certified_activation_typed_array=$tmp_dir/certified-activation-typed-array.txt
certified_reason_array_buffer=$tmp_dir/certified-reason-array-buffer.txt
certified_reason_data_view=$tmp_dir/certified-reason-data-view.txt
certified_reason_typed_array=$tmp_dir/certified-reason-typed-array.txt

comm -12 "$generated_activation" "$array_buffer_paths" \
    >"$certified_activation_array_buffer"
comm -12 "$generated_activation" "$data_view_paths" \
    >"$certified_activation_data_view"
comm -12 "$generated_activation" "$typed_array_paths" \
    >"$certified_activation_typed_array"
comm -12 "$generated_reason_only" "$array_buffer_paths" \
    >"$certified_reason_array_buffer"
comm -12 "$generated_reason_only" "$data_view_paths" \
    >"$certified_reason_data_view"
comm -12 "$generated_reason_only" "$typed_array_paths" \
    >"$certified_reason_typed_array"

verify_path_inventory certified_activation_array_buffer \
    "$certified_activation_array_buffer"
verify_path_inventory certified_activation_data_view \
    "$certified_activation_data_view"
verify_path_inventory certified_activation_typed_array_core \
    "$certified_activation_typed_array"
verify_path_inventory certified_reason_only_array_buffer \
    "$certified_reason_array_buffer"
verify_path_inventory certified_reason_only_data_view \
    "$certified_reason_data_view"
verify_path_inventory certified_reason_only_typed_array_core \
    "$certified_reason_typed_array"

subgroup_union=$tmp_dir/subgroup-union.txt
cat "$certified_activation_array_buffer" \
    "$certified_activation_data_view" \
    "$certified_activation_typed_array" | sort >"$subgroup_union"
if [[ -n "$(uniq -d "$subgroup_union")" ]]; then
    echo "error: certified activation antecedents overlap" >&2
    exit 1
fi
diff -u "$certified_activation" "$subgroup_union"
cat "$certified_reason_array_buffer" \
    "$certified_reason_data_view" \
    "$certified_reason_typed_array" | sort >"$subgroup_union"
if [[ -n "$(uniq -d "$subgroup_union")" ]]; then
    echo "error: certified reason-only antecedents overlap" >&2
    exit 1
fi
diff -u "$certified_reason_only" "$subgroup_union"

printf 'Resizable ArrayBuffer assets pass: universe=%s paths/%s variants; activation=%s/%s, reason-only=%s/%s, config-skip=%s/%s; certified=%s and spillover=%s paths\n' \
    "$(read_value universe_paths)" "$(read_value universe_variants)" \
    "$(read_value activation_paths)" "$(read_value activation_variants)" \
    "$(read_value reason_only_paths)" "$(read_value reason_only_variants)" \
    "$(read_value config_skipped_paths)" \
    "$(read_value config_skipped_variants)" \
    "$(read_value certified_paths)" "$(read_value spillover_paths)"

quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$generated_activation"
if ! (
    cd -- "$source_dir"
    # `-a` follows `-c` because the config otherwise restores its default mode.
    ./run-test262 -m -c test262.conf -a -T "$workers" \
        -f "${quickjs_files[@]}"
) >"$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    echo "error: pinned QuickJS could not execute the focused RAB manifest" >&2
    exit 1
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$oracle_log" \
    || ! grep -Fq \
        "Average memory statistics for $(read_value quickjs_variants) tests:" \
        "$oracle_log" \
    || grep -Fq 'SKIPPED FEATURE' "$oracle_log"; then
    tail -n 100 "$oracle_log" >&2
    echo "error: pinned QuickJS no longer passes every focused RAB variant" >&2
    exit 1
fi
printf 'Pinned QuickJS passes all %s focused RAB variants\n' \
    "$(read_value quickjs_variants)"

QJS_ORACLE="$source_dir/qjs" \
    cargo test --locked --quiet --test oracle_array_buffer

if [[ "$mode" == check ]]; then
    printf 'Resizable ArrayBuffer check passes without running quickjs-oxide\n'
    exit 0
fi

rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$focused_profile" \
    --manifest "$activation_manifest" \
    --report "$report" \
    --mode "$(read_value mode)" \
    --timeout-ms "$(read_value timeout_ms)" \
    --workers "$workers")
printf '%s\n' "$run_output"

if [[ ! -f "$json_report" \
    || "$(read_header "$report" quickjs)" != "$(read_value quickjs)" \
    || "$(read_header "$report" test262)" != "$(read_value test262)" \
    || "$(read_header "$report" test262_patch_sha256)" \
        != "$(read_value test262_patch_sha256)" \
    || "$(read_header "$report" test262_config_sha256)" \
        != "$(read_value test262_config_sha256)" \
    || "$(read_header "$report" test262_metadata_sha256)" \
        != "$(read_value test262_metadata_sha256)" \
    || "$(read_header "$report" oxide_profile_sha256)" \
        != "$(read_value scoped_oxide_profile_sha256)" \
    || "$(read_header "$report" profile)" != "$(read_value schema)" \
    || "$(read_header "$report" mode)" != "$(read_value mode)" ]]; then
    echo "error: focused RAB report metadata drifted" >&2
    exit 1
fi

report_keys=$tmp_dir/report.keys
report_rows "$report" | awk -F'\t' '{ print $1 "\t" $2 }' | sort \
    >"$report_keys"
diff -u "$activation_keys" "$report_keys"
if [[ "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
        != "$(read_value activation_variants)" \
    || "$(execution_runnable "$run_output")" \
        != "$(read_value focused_runnable)" \
    || "$(report_summary "$report")" != "$(read_value focused_summary)" ]]; then
    echo "error: focused RAB report count or summary drifted" >&2
    exit 1
fi
if report_rows "$report" | awk -F'\t' '
    !($7 == "pass" && $8 == "normal" && $9 == "" && $10 == "") {
        print
        bad=1
    }
    END { exit bad ? 0 : 1 }
' >&2; then
    echo "error: focused RAB report contains a non-pass row" >&2
    exit 1
fi

json_results=$(grep -c '^{"kind":"result",' "$json_report" || true)
if [[ "$json_results" != "$(read_value activation_variants)" \
    || "$(grep -c '"outcome":"pass"' "$json_report" || true)" \
        != "$(read_value activation_variants)" \
    || "$(tail -n 1 "$json_report")" \
        != "{\"kind\":\"summary\",\"outcomes\":{\"pass\":$(read_value activation_variants)}}" ]]; then
    echo "error: focused RAB JSONL report drifted from the all-pass vector" >&2
    exit 1
fi

nonpass_sha=$(report_rows "$report" | awk -F'\t' '$7 != "pass" {
    print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
}' | sha256_stream)
tsv_sha=$(sha256_file "$report")
jsonl_sha=$(sha256_file "$json_report")
summary=$(report_summary "$report")

if [[ "$mode" == bless ]]; then
    cat <<EOF
focused_nonpass_sha256=$nonpass_sha
focused_tsv_sha256=$tsv_sha
focused_jsonl_sha256=$jsonl_sha
focused_summary=$summary
EOF
    printf 'Resizable ArrayBuffer focused receipt is ready to freeze\n'
    exit 0
fi

expected_nonpass=$(read_value focused_nonpass_sha256)
expected_tsv=$(read_value focused_tsv_sha256)
expected_jsonl=$(read_value focused_jsonl_sha256)
expected_summary=$(read_value focused_summary)
if [[ "$nonpass_sha" != "$expected_nonpass" \
    || "$tsv_sha" != "$expected_tsv" \
    || "$jsonl_sha" != "$expected_jsonl" \
    || "$summary" != "$expected_summary" ]]; then
    echo "error: focused RAB execution receipt drifted" >&2
    exit 1
fi

printf 'Resizable ArrayBuffer focused gate passes: %s/%s variants across %s paths\n' \
    "$(read_value activation_variants)" \
    "$(read_value activation_variants)" \
    "$(read_value activation_paths)"
