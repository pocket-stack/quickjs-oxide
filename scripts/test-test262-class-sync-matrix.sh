#!/usr/bin/env bash
# Reproduce the R3y synchronous class generated-matrix Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-sync-matrix-baseline.txt
manifest=tests/test262-class-sync-matrix.txt
admission_profile=tests/test262-class-sync-matrix.conf
global_profile=compat/test262-oxide.conf
metadata_records=target/test262-class-sync-matrix-metadata.records
report=target/test262-class-sync-matrix.tsv
json_report=target/test262-class-sync-matrix.jsonl
quickjs_log=target/test262-class-sync-matrix-quickjs.log
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs and pinned QuickJS; skip the 7,735-row Oxide gate\n'
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

profile_section() {
    local section=$1 profile=${2:-$admission_profile}
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

inventory_count() {
    awk 'NF { count++ } END { print count + 0 }' "$1"
}

verify_inventory() {
    local name=$1 inventory=$2
    local expected_count expected_hash
    expected_count=$(read_value "${name}_paths")
    expected_hash=$(read_value "${name}_sha256")
    if [[ "$(inventory_count "$inventory")" != "$expected_count" \
        || "$(sha256_file "$inventory")" != "$expected_hash" ]]; then
        echo "error: class sync matrix $name inventory drifted" >&2
        exit 1
    fi
}

variant_count() {
    local inventory=$1
    awk -F'\t' '
        NR == FNR { selected[$1]=1; next }
        $1 in selected {
            flags="," $3 ","
            if (index(flags, ",module,") \
                || index(flags, ",noStrict,") \
                || index(flags, ",raw,") \
                || index(flags, ",onlyStrict,")) {
                variants++
            } else {
                variants += 2
            }
        }
        END { print variants + 0 }
    ' "$inventory" "$metadata_tsv"
}

verify_variant_count() {
    local name=$1 inventory=$2
    if [[ "$(variant_count "$inventory")" != "$(read_value "${name}_variants")" ]]; then
        echo "error: class sync matrix $name variant count drifted" >&2
        exit 1
    fi
}

write_counts() {
    local occurrences=$1 destination=$2
    LC_ALL=C sort "$occurrences" | awk '
        NR == 1 { previous=$0; count=1; next }
        $0 == previous { count++; next }
        { print previous "\t" count; previous=$0; count=1 }
        END { if (NR != 0) print previous "\t" count }
    ' > "$destination"
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 output test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < "$metadata_closure"
    # test262.conf resets run-test262.c's mode, so -a must follow -c.
    if ! output=$(cd -- "$source_dir" \
        && ./run-test262 -m -c test262.conf -a -f "${files[@]}" 2>&1); then
        printf '%s\n' "$output" > "$quickjs_log"
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS could not execute the upper class sync matrix closure" >&2
        exit 1
    fi
    printf '%s\n' "$output" > "$quickjs_log"
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$quickjs_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_upper_passes) tests:" \
            "$quickjs_log"; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS no longer passes the upper class sync matrix closure" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3y.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

metadata_tsv=$tmp_dir/metadata.tsv
features_file=$tmp_dir/features.txt
global_features=$tmp_dir/global-features.txt
target_features=$tmp_dir/target-features.txt
metadata_closure=$tmp_dir/metadata-closure.txt
async_frontier=$tmp_dir/async-frontier.txt
proxy_frontier=$tmp_dir/proxy-frontier.txt
optional_frontier=$tmp_dir/optional-frontier.txt
frontier=$tmp_dir/frontier.txt
derived_manifest=$tmp_dir/manifest.txt
positive=$tmp_dir/positive.txt
negative=$tmp_dir/negative.txt
only_strict=$tmp_dir/only-strict.txt
no_strict=$tmp_dir/no-strict.txt
single_variant=$tmp_dir/single-variant.txt
variant_keys_raw=$tmp_dir/variant-keys-raw.txt
variant_keys=$tmp_dir/variant-keys.txt
sloppy_paths_raw=$tmp_dir/sloppy-paths-raw.txt
sloppy_paths=$tmp_dir/sloppy-paths.txt
strict_paths_raw=$tmp_dir/strict-paths-raw.txt
strict_paths=$tmp_dir/strict-paths.txt
feature_occurrences=$tmp_dir/feature-occurrences.txt
include_occurrences=$tmp_dir/include-occurrences.txt
flag_occurrences=$tmp_dir/flag-occurrences.txt
feature_inventory=$tmp_dir/selected-features.txt
include_inventory=$tmp_dir/selected-includes.txt
flag_inventory=$tmp_dir/selected-flags.txt
feature_counts=$tmp_dir/feature-counts.tsv
include_counts=$tmp_dir/include-counts.tsv
flag_counts=$tmp_dir/flag-counts.tsv
root_counts=$tmp_dir/root-counts.tsv

if [[ "$(read_value quickjs)" != "2026-06-04" \
    || "$(read_value test262)" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$(read_value test262_patch_sha256)" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$(read_value test262_config_sha256)" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$(read_value test262_metadata_sha256)" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$(read_value global_oxide_profile_sha256)" != "6a4d3dc37da05f6e63d7b8564483159c383ed66c665a2b5530624e628f73b908" \
    || "$(read_value oxide_profile_sha256)" != "de71fc1d3c675ed25dc54d43222a10c4f3d607c14cb4d43628d7a4587827a7ef" \
    || "$(read_value schema)" != "test262-canonical-classified-v2" \
    || "$(read_value mode)" != "both" \
    || "$(read_value timeout_ms)" != "30000" \
    || "$(read_value metadata_closure_paths)" != "3890" \
    || "$(read_value metadata_closure_variants)" != "7763" \
    || "$(read_value async_frontier_paths)" != "8" \
    || "$(read_value async_frontier_variants)" != "16" \
    || "$(read_value proxy_frontier_paths)" != "6" \
    || "$(read_value proxy_frontier_variants)" != "12" \
    || "$(read_value optional_frontier_paths)" != "0" \
    || "$(read_value optional_frontier_variants)" != "0" \
    || "$(read_value frontier_paths)" != "14" \
    || "$(read_value frontier_variants)" != "28" \
    || "$(read_value paths)" != "3876" \
    || "$(read_value variants)" != "7735" \
    || "$(read_value quickjs_passes)" != "7735" \
    || "$(read_value quickjs_upper_passes)" != "7763" \
    || "$(read_value positive_paths)" != "3196" \
    || "$(read_value positive_variants)" != "6383" \
    || "$(read_value negative_paths)" != "680" \
    || "$(read_value negative_variants)" != "1352" \
    || "$(read_value only_strict_paths)" != "9" \
    || "$(read_value no_strict_paths)" != "8" \
    || "$(read_value single_variant_paths)" != "17" \
    || "$(read_value sloppy_variants)" != "3867" \
    || "$(read_value strict_variants)" != "3868" \
    || "$(read_value features)" != "19" \
    || "$(read_value target_features)" != "10" \
    || "$(read_value includes)" != "2" \
    || "$(read_value flags)" != "3" \
    || "$(read_value runnable)" != "7735" \
    || "$(read_value passes)" != "7735" \
    || "$(read_value failures)" != "0" \
    || "$(read_value unsupported)" != "0" \
    || "$(read_value skipped)" != "0" ]]; then
    echo "error: class sync matrix baseline identity drifted" >&2
    exit 1
fi

[[ "$(sha256_file "$global_profile")" == "$(read_value global_oxide_profile_sha256)" \
    && "$(sha256_file "$admission_profile")" == "$(read_value oxide_profile_sha256)" ]] \
    || { echo "error: class sync matrix capability profile drifted" >&2; exit 1; }

profile_section features | LC_ALL=C sort > "$features_file"
profile_section features "$global_profile" | LC_ALL=C sort > "$global_features"
comm -23 "$features_file" "$global_features" > "$target_features"
[[ "$(inventory_count "$features_file")" == "$(read_value features)" \
    && "$(sha256_file "$features_file")" == "$(read_value features_sha256)" \
    && "$(inventory_count "$target_features")" == "$(read_value target_features)" \
    && "$(sha256_file "$target_features")" == "$(read_value target_features_sha256)" ]] \
    || { echo "error: class sync matrix feature closure drifted" >&2; exit 1; }

rm -f -- "$metadata_records"
cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --validate-metadata "$metadata_records"
[[ "$(sha256_file "$metadata_records")" == "$(read_value test262_metadata_sha256)" ]] \
    || { echo "error: pinned exhaustive Test262 metadata fingerprint drifted" >&2; exit 1; }
tr '\0' '\t' < "$metadata_records" > "$metadata_tsv"

# Derive the metadata-only upper closure. It is intentionally broader than the
# admitted manifest so undeclared source dependencies remain visible.
awk -F'\t' -v allowed_file="$features_file" -v target_file="$target_features" '
    BEGIN {
        while ((getline feature < allowed_file) > 0) allowed[feature]=1
        close(allowed_file)
        while ((getline feature < target_file) > 0) target[feature]=1
        close(target_file)
    }
    function has(list, value, count, values, i) {
        count=split(list, values, ",")
        for (i=1; i<=count; i++) if (values[i] == value) return 1
        return 0
    }
    function admitted(list, count, values, i) {
        count=split(list, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "" && !allowed[values[i]]) return 0
        }
        return 1
    }
    function targets_new_feature(list, count, values, i) {
        count=split(list, values, ",")
        for (i=1; i<=count; i++) if (values[i] in target) return 1
        return 0
    }
    $1 ~ /^test\/language\/(statements|expressions)\/class\/(dstr|elements)\// \
        && !has($3, "async") \
        && !has($3, "module") \
        && admitted($4) \
        && targets_new_feature($4) {
        print $1
    }
' "$metadata_tsv" | LC_ALL=C sort -u > "$metadata_closure"
LC_ALL=C sort -c "$metadata_closure"
verify_inventory metadata_closure "$metadata_closure"
verify_variant_count metadata_closure "$metadata_closure"

# Test262 metadata does not tag these source dependencies. Strip frontmatter
# before scanning, freeze each independently, and keep them visible as the
# implementation frontier rather than silently narrowing feature metadata.
while IFS= read -r test_path; do
    if sed '/^\/\*---$/,/^---\*\/$/d' "$suite/$test_path" \
        | grep -Eq '^[[:space:]]*(static[[:space:]]+)?async[[:space:]]*(\*[[:space:]]*)?#[[:alnum:]_$]+[[:space:]]*\('; then
        printf '%s\n' "$test_path"
    fi
done < "$metadata_closure" > "$async_frontier"
while IFS= read -r test_path; do
    if sed '/^\/\*---$/,/^---\*\/$/d' "$suite/$test_path" \
        | grep -Eq '(^|[^[:alnum:]_$])Proxy([^[:alnum:]_$]|$)'; then
        printf '%s\n' "$test_path"
    fi
done < "$metadata_closure" > "$proxy_frontier"
while IFS= read -r test_path; do
    if sed '/^\/\*---$/,/^---\*\/$/d' "$suite/$test_path" \
        | grep -Eq '\?\.'; then
        printf '%s\n' "$test_path"
    fi
done < "$metadata_closure" > "$optional_frontier"

verify_inventory async_frontier "$async_frontier"
verify_variant_count async_frontier "$async_frontier"
verify_inventory proxy_frontier "$proxy_frontier"
verify_variant_count proxy_frontier "$proxy_frontier"
verify_inventory optional_frontier "$optional_frontier"
verify_variant_count optional_frontier "$optional_frontier"
if LC_ALL=C sort "$async_frontier" "$proxy_frontier" "$optional_frontier" \
    | uniq -d | grep -q .; then
    echo "error: class sync matrix source frontiers overlap" >&2
    exit 1
fi
LC_ALL=C sort -u \
    "$async_frontier" "$proxy_frontier" "$optional_frontier" > "$frontier"
verify_inventory frontier "$frontier"
verify_variant_count frontier "$frontier"

comm -23 "$metadata_closure" "$frontier" > "$derived_manifest"
diff -u "$manifest" "$derived_manifest"
if [[ -n "$(comm -23 "$frontier" "$metadata_closure")" \
    || -n "$(comm -12 "$frontier" "$derived_manifest")" ]]; then
    echo "error: class sync matrix source frontier escaped its metadata closure" >&2
    exit 1
fi
[[ "$(inventory_count "$manifest")" == "$(read_value paths)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_sha256)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_file_sha256)" ]] \
    || { echo "error: class sync matrix manifest drifted" >&2; exit 1; }
if [[ "$(variant_count "$manifest")" != "$(read_value variants)" ]]; then
    echo "error: class sync matrix admitted variant count drifted" >&2
    exit 1
fi

awk -F'\t' \
    -v positive="$positive" \
    -v negative="$negative" \
    -v only_strict="$only_strict" \
    -v no_strict="$no_strict" \
    -v single_variant="$single_variant" \
    -v variant_keys="$variant_keys_raw" \
    -v sloppy_paths="$sloppy_paths_raw" \
    -v strict_paths="$strict_paths_raw" \
    -v feature_occurrences="$feature_occurrences" \
    -v include_occurrences="$include_occurrences" \
    -v flag_occurrences="$flag_occurrences" '
    NR == FNR { selected[$1]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    $1 in selected {
        if ($5 == "") {
            print $1 > positive
        } else {
            if ($5 != "parse" || $6 != "SyntaxError") {
                print "bad negative provenance: " $1 > "/dev/stderr"
                exit 2
            }
            print $1 > negative
        }

        count=split($4, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "") print values[i] > feature_occurrences
        }
        count=split($2, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "") print values[i] > include_occurrences
        }
        count=split($3, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "") print values[i] > flag_occurrences
        }

        if (has($3, "noStrict") || has($3, "raw")) {
            if (has($3, "noStrict")) print $1 > no_strict
            print $1 > single_variant
            print $1 "\tsloppy" > variant_keys
            print $1 > sloppy_paths
        } else if (has($3, "onlyStrict")) {
            print $1 > only_strict
            print $1 > single_variant
            print $1 "\tstrict" > variant_keys
            print $1 > strict_paths
        } else {
            print $1 "\tsloppy" > variant_keys
            print $1 "\tstrict" > variant_keys
            print $1 > sloppy_paths
            print $1 > strict_paths
        }
    }
' "$manifest" "$metadata_tsv"

LC_ALL=C sort "$variant_keys_raw" > "$variant_keys"
LC_ALL=C sort "$sloppy_paths_raw" > "$sloppy_paths"
LC_ALL=C sort "$strict_paths_raw" > "$strict_paths"
LC_ALL=C sort -u "$feature_occurrences" > "$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" > "$include_inventory"
LC_ALL=C sort -u "$flag_occurrences" > "$flag_inventory"
write_counts "$feature_occurrences" "$feature_counts"
write_counts "$include_occurrences" "$include_counts"
write_counts "$flag_occurrences" "$flag_counts"

awk -F'\t' '
    NR == FNR { selected[$1]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    $1 in selected {
        count=split($1, path, "/")
        root=path[3] "/" path[5]
        paths[root]++
        variants[root] += (has($3, "noStrict") || has($3, "raw") \
            || has($3, "onlyStrict") || has($3, "module")) ? 1 : 2
    }
    END {
        for (root in paths) print root "\t" paths[root] "\t" variants[root]
    }
' "$manifest" "$metadata_tsv" | LC_ALL=C sort > "$root_counts"

verify_inventory positive "$positive"
verify_variant_count positive "$positive"
verify_inventory negative "$negative"
verify_variant_count negative "$negative"
verify_inventory only_strict "$only_strict"
verify_inventory no_strict "$no_strict"
verify_inventory single_variant "$single_variant"
diff -u "$features_file" "$feature_inventory"
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    "$negative"

if [[ "$(inventory_count "$feature_inventory")" != "$(read_value features)" \
    || "$(sha256_file "$feature_inventory")" != "$(read_value features_sha256)" \
    || "$(inventory_count "$include_inventory")" != "$(read_value includes)" \
    || "$(sha256_file "$include_inventory")" != "$(read_value includes_sha256)" \
    || "$(inventory_count "$flag_inventory")" != "$(read_value flags)" \
    || "$(sha256_file "$flag_inventory")" != "$(read_value flags_sha256)" \
    || "$(sha256_file "$feature_counts")" != "$(read_value feature_counts_sha256)" \
    || "$(sha256_file "$include_counts")" != "$(read_value include_counts_sha256)" \
    || "$(sha256_file "$flag_counts")" != "$(read_value flag_counts_sha256)" \
    || "$(sha256_file "$root_counts")" != "$(read_value root_counts_sha256)" \
    || "$(inventory_count "$variant_keys")" != "$(read_value variants)" \
    || "$(sha256_file "$variant_keys")" != "$(read_value keys_sha256)" \
    || "$(inventory_count "$sloppy_paths")" != "$(read_value sloppy_variants)" \
    || "$(sha256_file "$sloppy_paths")" != "$(read_value sloppy_paths_sha256)" \
    || "$(inventory_count "$strict_paths")" != "$(read_value strict_variants)" \
    || "$(sha256_file "$strict_paths")" != "$(read_value strict_paths_sha256)" ]]; then
    echo "error: class sync matrix metadata composition drifted" >&2
    exit 1
fi

if awk -F'\t' '
    NR == FNR { selected[$1]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    ($1 in selected) && (has($3, "module") || has($3, "async") || has($3, "raw")) {
        print $1 > "/dev/stderr"
        found=1
    }
    END { exit found ? 0 : 1 }
' "$manifest" "$metadata_tsv"; then
    echo "error: class sync matrix admitted module, async, or raw metadata" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'class sync matrix inputs verified: %s metadata paths - %s source-frontier paths = %s paths; QuickJS %s passes all %s upper-closure variants; clean admission is %s variants\n' \
        "$(read_value metadata_closure_paths)" \
        "$(read_value frontier_paths)" \
        "$(read_value paths)" \
        "$(read_value quickjs)" \
        "$(read_value quickjs_upper_passes)" \
        "$(read_value quickjs_passes)"
    exit 0
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
expected_variants=$(read_value variants)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
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

actual_variants=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { count++ }
    END { print count + 0 }
' "$report")
execution_line=$(printf '%s\n' "$run_output" | awk '
    /^execution: runnable=/ { print; found=1 }
    END { if (!found) exit 1 }
')
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
    echo "error: class sync matrix report metadata drifted" >&2
    exit 1
fi

diff -u "$variant_keys" <(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2
    }
' "$report" | LC_ALL=C sort)

actual_passes=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ }
    END { print count + 0 }
' "$report")
actual_unsupported=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }
' "$report")
actual_skipped=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }
' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }
' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" | awk '
    /^Test262: total=/ { print; found=1 }
    END { if (!found) exit 1 }
')
expected_runner_summary="Test262: total=$expected_variants pass=$expected_passes fail=$expected_failures unsupported=$expected_unsupported skipped=$expected_skipped"

if [[ "$runner_summary" != "$expected_runner_summary" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: class sync matrix all-pass vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'class sync matrix Test262 gate passes: %s/%s variants across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$(read_value paths)"
