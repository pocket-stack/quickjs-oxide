#!/usr/bin/env bash
# Reproduce the R3t synchronous generators + destructuring-binding Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-generator-destructuring-baseline.txt
manifest=tests/test262-generator-destructuring.txt
async_exclusions=tests/test262-generator-destructuring-async-exclusions.txt
admission_profile=tests/test262-generator-destructuring.conf
global_profile=compat/test262-oxide.conf
metadata_records=target/test262-generator-destructuring-metadata.records
report=target/test262-generator-destructuring.tsv
json_report=target/test262-generator-destructuring.jsonl
quickjs_log=target/test262-generator-destructuring-quickjs.log
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify the frozen inventory/profile and pinned QuickJS oracle only\n'
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

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$admission_profile"
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
        echo "error: generator/destructuring $name inventory drifted" >&2
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
    local key=$1 inventory=$2
    if [[ "$(variant_count "$inventory")" != "$(read_value "$key")" ]]; then
        echo "error: generator/destructuring $key drifted" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 output test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < "$manifest"
    # QuickJS processes CLI arguments in order: test262.conf resets mode to
    # default, so `-a` must follow `-c` to authenticate both script variants.
    if ! output=$(cd -- "$source_dir" && ./run-test262 -m -c test262.conf -a -f "${files[@]}" 2>&1); then
        printf '%s\n' "$output" > "$quickjs_log"
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS could not execute the generator/destructuring cohort" >&2
        exit 1
    fi
    printf '%s\n' "$output" > "$quickjs_log"
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$quickjs_log" \
        || ! grep -Fq "Average memory statistics for $(read_value quickjs_passes) tests:" "$quickjs_log"; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS no longer passes all generator/destructuring variants" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3t.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
metadata_tsv=$tmp_dir/metadata.tsv
features_file=$tmp_dir/features.txt
metadata_universe=$tmp_dir/metadata-universe.txt
module_excluded=$tmp_dir/module-excluded.txt
metadata_sync=$tmp_dir/metadata-sync.txt
derived_async=$tmp_dir/async-excluded.txt
derived_manifest=$tmp_dir/manifest.txt
positive=$tmp_dir/positive.txt
negative=$tmp_dir/negative.txt
only_strict=$tmp_dir/only-strict.txt
no_strict=$tmp_dir/no-strict.txt
single_variant=$tmp_dir/single-variant.txt
variant_keys=$tmp_dir/keys.txt
sloppy_keys=$tmp_dir/sloppy-keys.txt
strict_keys=$tmp_dir/strict-keys.txt

if [[ "$(read_value quickjs)" != "2026-06-04" \
    || "$(read_value test262)" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$(read_value test262_patch_sha256)" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$(read_value test262_config_sha256)" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$(read_value test262_metadata_sha256)" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$(read_value schema)" != "test262-canonical-classified-v2" \
    || "$(read_value mode)" != "both" \
    || "$(read_value timeout_ms)" != "30000" \
    || "$(read_value metadata_universe_paths)" != "3418" \
    || "$(read_value metadata_universe_variants)" != "6624" \
    || "$(read_value module_excluded_paths)" != "25" \
    || "$(read_value module_excluded_variants)" != "25" \
    || "$(read_value metadata_sync_paths)" != "3393" \
    || "$(read_value metadata_sync_variants)" != "6599" \
    || "$(read_value async_excluded_paths)" != "3" \
    || "$(read_value async_excluded_variants)" != "6" \
    || "$(read_value paths)" != "3390" \
    || "$(read_value variants)" != "6593" \
    || "$(read_value quickjs_passes)" != "6593" \
    || "$(read_value positive_paths)" != "3011" \
    || "$(read_value positive_variants)" != "5906" \
    || "$(read_value negative_paths)" != "379" \
    || "$(read_value negative_variants)" != "687" \
    || "$(read_value only_strict_paths)" != "77" \
    || "$(read_value no_strict_paths)" != "110" \
    || "$(read_value single_variant_paths)" != "187" \
    || "$(read_value sloppy_variants)" != "3313" \
    || "$(read_value strict_variants)" != "3280" \
    || "$(read_value features)" != "11" ]]; then
    echo "error: generator/destructuring baseline identity drifted" >&2
    exit 1
fi

profile_section features | LC_ALL=C sort > "$features_file"
[[ "$(inventory_count "$features_file")" == "$(read_value features)" \
    && "$(sha256_file "$features_file")" == "$(read_value features_sha256)" ]] \
    || { echo "error: generator/destructuring feature inventory drifted" >&2; exit 1; }
[[ "$(sha256_file "$global_profile")" == "$(read_value global_oxide_profile_sha256)" \
    && "$(sha256_file "$admission_profile")" == "$(read_value oxide_profile_sha256)" ]] \
    || { echo "error: generator/destructuring capability profile drifted" >&2; exit 1; }
for feature in generators destructuring-binding; do
    if awk -v feature="$feature" '
        /^\[features\]$/ { inside=1; next }
        /^\[/ { inside=0 }
        inside && $0 == feature { found=1 }
        END { exit !found }
    ' "$global_profile"; then
        echo "error: global Test262 profile must remain fail-closed for $feature" >&2
        exit 1
    fi
done

rm -f -- "$metadata_records"
cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --validate-metadata "$metadata_records"
[[ "$(sha256_file "$metadata_records")" == "$(read_value test262_metadata_sha256)" ]] \
    || { echo "error: pinned exhaustive Test262 metadata fingerprint drifted" >&2; exit 1; }
tr '\0' '\t' < "$metadata_records" > "$metadata_tsv"

# The static universe is metadata-only: at least one target feature, and every
# other feature must belong to the exact scoped profile. This avoids selecting
# tests by current Oxide outcomes.
awk -F'\t' -v allowed_file="$features_file" '
    BEGIN {
        while ((getline feature < allowed_file) > 0) allowed[feature]=1
        close(allowed_file)
    }
    function has(list, value, count, values, i) {
        count=split(list, values, ",")
        for (i=1; i <= count; i++) {
            if (values[i] == value) return 1
        }
        return 0
    }
    function admitted(list, count, values, i) {
        count=split(list, values, ",")
        for (i=1; i <= count; i++) {
            if (values[i] != "" && !allowed[values[i]]) return 0
        }
        return 1
    }
    (has($4, "generators") || has($4, "destructuring-binding")) && admitted($4) {
        print $1
    }
' "$metadata_tsv" > "$metadata_universe"
LC_ALL=C sort -c "$metadata_universe"
verify_inventory metadata_universe "$metadata_universe"
verify_variant_count metadata_universe_variants "$metadata_universe"

awk -F'\t' '
    NR == FNR { selected[$1]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    ($1 in selected) && has($3, "module") { print $1 }
' "$metadata_universe" "$metadata_tsv" > "$module_excluded"
verify_inventory module_excluded "$module_excluded"
verify_variant_count module_excluded_variants "$module_excluded"

comm -23 "$metadata_universe" "$module_excluded" > "$metadata_sync"
verify_inventory metadata_sync "$metadata_sync"
verify_variant_count metadata_sync_variants "$metadata_sync"

# Three pinned tests contain real async syntax but omit the async metadata flag.
# Strip only frontmatter before scanning so prose about AsyncFunction does not
# widen the exclusion. The exact result is frozen and source-audited.
while IFS= read -r test_path; do
    if sed '/^\/\*---$/,/^---\*\/$/d' "$suite/$test_path" \
        | grep -Eq '(^|[^[:alnum:]_$])async[[:space:]]*(\(|function)'; then
        printf '%s\n' "$test_path"
    fi
done < "$metadata_sync" > "$derived_async"
verify_inventory async_excluded "$derived_async"
verify_variant_count async_excluded_variants "$derived_async"
diff -u "$async_exclusions" "$derived_async"
[[ "$(sha256_file "$async_exclusions")" == "$(read_value async_excluded_sha256)" ]] \
    || { echo "error: checked async exclusion file drifted" >&2; exit 1; }

comm -23 "$metadata_sync" "$derived_async" > "$derived_manifest"
diff -u "$manifest" "$derived_manifest"
if [[ -n "$(comm -23 "$derived_async" "$metadata_sync")" \
    || -n "$(comm -12 "$derived_async" "$derived_manifest")" ]]; then
    echo "error: async exclusions escaped or overlap the synchronous manifest" >&2
    exit 1
fi
[[ "$(inventory_count "$manifest")" == "$(read_value paths)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_sha256)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_file_sha256)" ]] \
    || { echo "error: generator/destructuring manifest drifted" >&2; exit 1; }
verify_variant_count variants "$manifest"
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
    echo "error: synchronous manifest contains module, async, or raw metadata" >&2
    exit 1
fi

awk -F'\t' \
    -v positive="$positive" \
    -v negative="$negative" \
    -v only_strict="$only_strict" \
    -v no_strict="$no_strict" \
    -v single_variant="$single_variant" \
    -v variant_keys="$variant_keys" \
    -v sloppy_keys="$sloppy_keys" \
    -v strict_keys="$strict_keys" '
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
        if (has($3, "noStrict") || has($3, "raw")) {
            if (has($3, "noStrict")) print $1 > no_strict
            print $1 > single_variant
            print $1 "\tsloppy" > variant_keys
            print $1 "\tsloppy" > sloppy_keys
        } else if (has($3, "onlyStrict")) {
            print $1 > only_strict
            print $1 > single_variant
            print $1 "\tstrict" > variant_keys
            print $1 "\tstrict" > strict_keys
        } else {
            print $1 "\tsloppy" > variant_keys
            print $1 "\tstrict" > variant_keys
            print $1 "\tsloppy" > sloppy_keys
            print $1 "\tstrict" > strict_keys
        }
    }
' "$manifest" "$metadata_tsv"

feature_inventory=$tmp_dir/selected-features.txt
awk -F'\t' '
    NR == FNR { selected[$1]=1; next }
    $1 in selected {
        count=split($4, features, ",")
        for (i=1; i <= count; i++) {
            if (features[i] != "") print features[i]
        }
    }
' "$manifest" "$metadata_tsv" | LC_ALL=C sort -u > "$feature_inventory"

verify_inventory positive "$positive"
verify_variant_count positive_variants "$positive"
verify_inventory negative "$negative"
verify_variant_count negative_variants "$negative"
verify_inventory only_strict "$only_strict"
verify_inventory no_strict "$no_strict"
verify_inventory single_variant "$single_variant"
[[ "$(sha256_file "$feature_inventory")" == "$(read_value features_sha256)" ]] \
    || { echo "error: selected feature inventory drifted" >&2; exit 1; }
diff -u "$features_file" "$feature_inventory"
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    "$negative"
[[ "$(sha256_file "$variant_keys")" == "$(read_value keys_sha256)" \
    && "$(inventory_count "$variant_keys")" == "$(read_value variants)" \
    && "$(inventory_count "$sloppy_keys")" == "$(read_value sloppy_variants)" \
    && "$(inventory_count "$strict_keys")" == "$(read_value strict_variants)" ]] \
    || { echo "error: generator/destructuring variant key inventory drifted" >&2; exit 1; }

verify_quickjs_oracle
if "$check_only"; then
    printf 'generator/destructuring inputs verified: %s raw - %s modules - %s async = %s paths; QuickJS %s passes all %s variants\n' \
        "$(read_value metadata_universe_paths)" \
        "$(read_value module_excluded_paths)" \
        "$(read_value async_excluded_paths)" \
        "$(read_value paths)" \
        "$(read_value quickjs)" \
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
    --timeout-ms "$expected_timeout_ms" \
    --allow-failures)
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
    echo "error: generator/destructuring report metadata drifted" >&2
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
' "$report" | {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
})

if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$(tail -n 1 "$report")" != "# summary $expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: generator/destructuring all-pass vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'generator/destructuring Test262 gate passes: %s/%s variants across %s paths\n' \
    "$expected_passes" "$expected_variants" "$(read_value paths)"
