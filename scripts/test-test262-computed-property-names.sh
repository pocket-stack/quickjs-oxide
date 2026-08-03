#!/usr/bin/env bash
# Reproduce the focused computed-property-names admission certificate.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-computed-property-names-baseline.txt
full_baseline=tests/test262-full-baseline.txt
live_profile=compat/test262-oxide.conf
workers=${TEST262_WORKERS:-8}

if [[ $# -gt 0 ]]; then
    if [[ $# == 1 && ( "$1" == -h || "$1" == --help ) ]]; then
        cat <<'EOF'
usage: scripts/test-test262-computed-property-names.sh

Rebuild the complete computed-property-names metadata partition, certify its
439 activation variants in pinned QuickJS, and reproduce the exact 946-row
parent/candidate quickjs-oxide transition.
EOF
        exit 0
    fi
    echo "error: this gate accepts no arguments" >&2
    exit 2
fi
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer: $workers" >&2
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

read_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$baseline"); then
        echo "error: baseline must contain exactly one $key entry" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: baseline contains an empty $key entry" >&2
        exit 1
    fi
    if [[ -n "${consumed_keys-}" ]]; then
        printf '%s\n' "$key" >>"$consumed_keys"
    fi
    printf '%s\n' "$value"
}

read_full_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$full_baseline"); then
        echo "error: full baseline must contain exactly one $key entry" >&2
        exit 1
    fi
    [[ -n "$value" ]] || { echo "error: empty full baseline $key" >&2; exit 1; }
    printf '%s\n' "$value"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    if [[ "$actual" != "$expected" ]]; then
        printf 'error: computed-property-names baseline %s drifted: %s != %s\n' \
            "$key" "$actual" "$expected" >&2
        exit 1
    fi
}

profile_section() {
    local profile=$1 section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

read_header() {
    local report=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$report"
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

summary_count() {
    local summary=$1 key=$2
    printf '%s\n' "$summary" | awk -v key="$key" '
        {
            for (i=1; i<=NF; i++) {
                split($i, pair, "=")
                if (pair[1] == key) {
                    if (found++) exit 2
                    print pair[2]
                }
            }
        }
        END { if (found != 1) exit 1 }
    '
}

unsupported_total() {
    printf '%s\n' "$1" | awk '
        {
            for (i=1; i<=NF; i++) {
                split($i, pair, "=")
                if (pair[1] ~ /^unsupported-/) total+=pair[2]
            }
        }
        END { print total + 0 }
    '
}

cleanup() {
    if [[ -n "${tmp_dir-}" && -d "$tmp_dir" ]]; then
        rm -rf -- "$tmp_dir"
    fi
}

cd -- "$root"
[[ -f "$baseline" ]] || { echo "error: missing $baseline" >&2; exit 1; }
[[ -f "$full_baseline" ]] || { echo "error: missing $full_baseline" >&2; exit 1; }
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-cpn.XXXXXX")
trap cleanup EXIT HUP INT TERM
consumed_keys=$tmp_dir/consumed-baseline-keys.txt
: >"$consumed_keys"

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
expect_value parent_profile tests/test262-computed-property-names-parent.conf
expect_value parent_features 90
expect_value candidate_profile tests/test262-computed-property-names.conf
expect_value candidate_features 91
expect_value added_features 1
expect_value profile_negative_paths 828
expect_value profile_execution_entries 1
expect_value universe_manifest tests/test262-computed-property-names-universe.txt
expect_value universe_paths 478
expect_value universe_variants 946
expect_value activation_manifest tests/test262-computed-property-names-activation.txt
expect_value activation_paths 220
expect_value activation_variants 439
expect_value reason_only_manifest tests/test262-computed-property-names-reason-only.txt
expect_value reason_only_paths 228
expect_value reason_only_variants 456
expect_value config_skipped_manifest \
    tests/test262-computed-property-names-config-skipped.txt
expect_value config_skipped_paths 21
expect_value config_skipped_variants 42
expect_value module_manifest tests/test262-computed-property-names-module.txt
expect_value module_paths 9
expect_value module_variants 9
expect_value parent_tag_runnable 0
expect_value parent_tag_summary \
    'skipped-feature=42 unsupported-feature=895 unsupported-module=9'
expect_value candidate_tag_variants 946
expect_value candidate_tag_runnable 439
expect_value candidate_tag_passes 439
expect_value candidate_tag_failures 0
expect_value candidate_tag_unsupported 465
expect_value candidate_tag_skipped 42
expect_value candidate_tag_summary \
    'pass=439 skipped-feature=42 unsupported-feature=456 unsupported-module=9'
expect_value quickjs_variants 439

parent_profile=$(read_value parent_profile)
candidate_profile=$(read_value candidate_profile)
universe_manifest=$(read_value universe_manifest)
activation_manifest=$(read_value activation_manifest)
reason_only_manifest=$(read_value reason_only_manifest)
config_skipped_manifest=$(read_value config_skipped_manifest)
module_manifest=$(read_value module_manifest)
for asset in "$live_profile" "$parent_profile" "$candidate_profile" \
    "$universe_manifest" "$activation_manifest" "$reason_only_manifest" \
    "$config_skipped_manifest" "$module_manifest"
do
    [[ -f "$asset" ]] || { echo "error: missing gate input: $asset" >&2; exit 1; }
done

printf '%s\n' schema timeout_ms variants runnable passes tsv_sha256 \
    jsonl_sha256 summary | sort >"$tmp_dir/expected-full-baseline-keys.txt"
awk -F= 'NF && $1 !~ /^#/ {print $1}' "$full_baseline" | sort \
    >"$tmp_dir/actual-full-baseline-keys.txt"
diff -u "$tmp_dir/expected-full-baseline-keys.txt" \
    "$tmp_dir/actual-full-baseline-keys.txt"
parent_full_summary=$(read_value parent_full_summary)
if [[ "$(read_full_value schema)" != "$(read_value schema)" \
    || "$(read_full_value timeout_ms)" != "$(read_value timeout_ms)" \
    || "$(read_full_value variants)" != "$(read_value full_variants)" \
    || "$(read_full_value runnable)" != "$(read_value parent_full_runnable)" \
    || "$(read_full_value passes)" != "$(read_value parent_full_passes)" \
    || "$(read_full_value tsv_sha256)" != "$(read_value parent_full_tsv_sha256)" \
    || "$(read_full_value jsonl_sha256)" \
        != "$(read_value parent_full_jsonl_sha256)" \
    || "$(read_full_value summary)" != "$parent_full_summary" \
    || "$(summary_count "$parent_full_summary" pass)" \
        != "$(read_value parent_full_passes)" \
    || "$(summary_count "$parent_full_summary" unsupported-feature)" \
        != "$(read_value parent_full_unsupported_feature)" \
    || "$(unsupported_total "$parent_full_summary")" \
        != "$(read_value parent_full_total_unsupported)" ]]; then
    echo "error: canonical R3bu full baseline is not the frozen parent receipt" >&2
    exit 1
fi

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

parent_features=$tmp_dir/parent-features.txt
candidate_features=$tmp_dir/candidate-features.txt
parent_negatives=$tmp_dir/parent-negatives.txt
candidate_negatives=$tmp_dir/candidate-negatives.txt
parent_execution=$tmp_dir/parent-execution.txt
candidate_execution=$tmp_dir/candidate-execution.txt
added_features=$tmp_dir/added-features.txt
profile_section "$parent_profile" features >"$parent_features"
profile_section "$candidate_profile" features >"$candidate_features"
profile_section "$parent_profile" audited-negative-tests >"$parent_negatives"
profile_section "$candidate_profile" audited-negative-tests >"$candidate_negatives"
profile_section "$parent_profile" execution >"$parent_execution"
profile_section "$candidate_profile" execution >"$candidate_execution"
comm -13 "$parent_features" "$candidate_features" >"$added_features"

for sorted in "$parent_features" "$candidate_features" \
    "$parent_negatives" "$candidate_negatives"; do
    sort -c "$sorted"
done
if [[ "$(sha256_file "$parent_profile")" != "$(read_value parent_profile_sha256)" \
    || "$(sha256_file "$candidate_profile")" \
        != "$(read_value candidate_profile_sha256)" \
    || "$(wc -l <"$parent_features" | tr -d '[:space:]')" \
        != "$(read_value parent_features)" \
    || "$(sha256_file "$parent_features")" \
        != "$(read_value parent_features_sha256)" \
    || "$(wc -l <"$candidate_features" | tr -d '[:space:]')" \
        != "$(read_value candidate_features)" \
    || "$(sha256_file "$candidate_features")" \
        != "$(read_value candidate_features_sha256)" \
    || "$(wc -l <"$added_features" | tr -d '[:space:]')" \
        != "$(read_value added_features)" \
    || "$(sha256_file "$added_features")" \
        != "$(read_value added_features_sha256)" \
    || "$(cat "$added_features")" != computed-property-names \
    || -n "$(comm -23 "$parent_features" "$candidate_features")" ]]; then
    echo "error: parent/candidate feature transition drifted" >&2
    exit 1
fi
diff -u "$parent_negatives" "$candidate_negatives"
diff -u "$parent_execution" "$candidate_execution"
if [[ "$(wc -l <"$candidate_negatives" | tr -d '[:space:]')" \
        != "$(read_value profile_negative_paths)" \
    || "$(sha256_file "$candidate_negatives")" \
        != "$(read_value profile_negative_sha256)" \
    || "$(wc -l <"$candidate_execution" | tr -d '[:space:]')" \
        != "$(read_value profile_execution_entries)" \
    || "$(sha256_file "$candidate_execution")" \
        != "$(read_value profile_execution_sha256)" \
    || "$(cat "$candidate_execution")" != async=true ]]; then
    echo "error: profile negative or execution policy drifted" >&2
    exit 1
fi
if ! cmp -s "$live_profile" "$parent_profile" \
    && ! cmp -s "$live_profile" "$candidate_profile"; then
    echo "error: live profile is neither the frozen parent nor candidate" >&2
    exit 1
fi

metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" --validate-metadata "$metadata_records"
if [[ "$(sha256_file "$metadata_records")" \
        != "$(read_value test262_metadata_sha256)" ]]; then
    echo "error: exhaustive Test262 metadata fingerprint drifted" >&2
    exit 1
fi
tr '\0' '\t' <"$metadata_records" >"$metadata_tsv"
if [[ "$(wc -l <"$metadata_tsv" | tr -d '[:space:]')" \
        != "$(read_value test262_metadata_records)" ]] \
    || ! awk -F'\t' 'NF != 6 || $1 == "" { exit 1 }' "$metadata_tsv" \
    || ! cut -f1 "$metadata_tsv" | sort -c \
    || [[ -n "$(cut -f1 "$metadata_tsv" | uniq -d)" ]]; then
    echo "error: exhaustive Test262 metadata record structure drifted" >&2
    exit 1
fi

skipped_features=$tmp_dir/skipped-features.txt
awk '
    $0 == "[features]" { inside=1; next }
    /^\[/ { inside=0 }
    inside && NF && $1 !~ /^#/ && /=skip$/ {
        sub(/=skip$/, "")
        print
    }
' "$source_dir/test262.conf" | sort -u >"$skipped_features"

for prefix in universe activation reason_only config_skipped module; do
    : >"$tmp_dir/$prefix.paths"
    : >"$tmp_dir/$prefix.keys"
done
: >"$tmp_dir/full.keys"
awk -F'\t' \
    -v full_keys="$tmp_dir/full.keys" \
    -v universe="$tmp_dir/universe.paths" -v universe_keys="$tmp_dir/universe.keys" \
    -v activation="$tmp_dir/activation.paths" -v activation_keys="$tmp_dir/activation.keys" \
    -v reason="$tmp_dir/reason_only.paths" -v reason_keys="$tmp_dir/reason_only.keys" \
    -v config="$tmp_dir/config_skipped.paths" -v config_keys="$tmp_dir/config_skipped.keys" \
    -v module="$tmp_dir/module.paths" -v module_keys="$tmp_dir/module.keys" '
    function has(list, value) { return index("," list ",", "," value ",") != 0 }
    function emit_keys(keys, path, flags) {
        if (has(flags, "module") || has(flags, "raw") || has(flags, "noStrict")) {
            print path "\tsloppy" > keys
        } else if (has(flags, "onlyStrict")) {
            print path "\tstrict" > keys
        } else {
            print path "\tsloppy" > keys
            print path "\tstrict" > keys
        }
    }
    function emit(paths, keys, path, flags) {
        print path > paths
        emit_keys(keys, path, flags)
    }
    FILENAME == ARGV[1] { parent[$1]=1; next }
    FILENAME == ARGV[2] { candidate[$1]=1; next }
    FILENAME == ARGV[3] { skipped[$1]=1; next }
    {
        emit_keys(full_keys, $1, $3)
        if (!has($4, "computed-property-names")) next
        emit(universe, universe_keys, $1, $3)
        if (has($3, "module")) {
            emit(module, module_keys, $1, $3)
            next
        }
        config_skip=parent_missing=candidate_missing=0
        n=split($4, feature, ",")
        for (i=1; i<=n; i++) {
            if (feature[i] in skipped) config_skip=1
            if (!(feature[i] in parent)) parent_missing++
            if (!(feature[i] in candidate)) candidate_missing++
        }
        if (config_skip) {
            emit(config, config_keys, $1, $3)
        } else if (parent_missing != candidate_missing + 1) {
            printf "bad computed-property-names dependency partition: %s\n", $1 > "/dev/stderr"
            bad=1
        } else if (candidate_missing == 0) {
            emit(activation, activation_keys, $1, $3)
        } else {
            emit(reason, reason_keys, $1, $3)
        }
    }
    END { if (bad) exit 1 }
' "$parent_features" "$candidate_features" "$skipped_features" "$metadata_tsv"

verify_inventory() {
    local prefix=$1 manifest generated=$tmp_dir/$1.paths keys=$tmp_dir/$1.keys
    manifest=$(read_value "${prefix}_manifest")
    for sorted in "$manifest" "$generated" "$keys"; do sort -c "$sorted"; done
    diff -u "$manifest" "$generated"
    if [[ "$(wc -l <"$generated" | tr -d '[:space:]')" \
            != "$(read_value "${prefix}_paths")" \
        || "$(sort -u "$generated" | wc -l | tr -d '[:space:]')" \
            != "$(read_value "${prefix}_paths")" \
        || "$(sha256_file "$manifest")" \
            != "$(read_value "${prefix}_manifest_sha256")" \
        || "$(sha256_file "$generated")" \
            != "$(read_value "${prefix}_paths_sha256")" \
        || "$(wc -l <"$keys" | tr -d '[:space:]')" \
            != "$(read_value "${prefix}_variants")" \
        || "$(sha256_file "$keys")" \
            != "$(read_value "${prefix}_keys_sha256")" ]]; then
        echo "error: $prefix inventory drifted" >&2
        exit 1
    fi
}
for prefix in universe activation reason_only config_skipped module; do
    verify_inventory "$prefix"
done

activation_stats=$tmp_dir/activation-stats.txt
awk -F'\t' '
    function variants(flags) {
        return flags ~ /(^|,)(module|raw|noStrict|onlyStrict)(,|$)/ ? 1 : 2
    }
    NR == FNR { wanted[$0]=1; next }
    !($1 in wanted) { next }
    {
        count=variants($3)
        if ($5 == "" && $6 == "") normal+=count
        else bad=1
        if ($3 == "") plain+=count
        else if ($3 == "async") async+=count
        else if ($3 == "generated") generated+=count
        else if ($3 == "noStrict") no_strict+=count
        else bad=1

        if ($1 ~ /^test\/built-ins\/BigInt\/(asIntN|asUintN)\//) group="bigint"
        else if ($1 ~ /^test\/built-ins\/Promise\/any\//) group="promise_any"
        else if ($1 ~ /^test\/built-ins\/Reflect\/ownKeys\//) group="reflect_own_keys"
        else if ($1 ~ /^test\/built-ins\/String\/prototype\/indexOf\//) group="string_index_of"
        else if ($1 ~ /^test\/language\/expressions\/class\//) group="class_expression"
        else if ($1 ~ /^test\/language\/statements\/class\//) group="class_declaration"
        else if ($1 ~ /^test\/language\/expressions\/object\//) group="object_literal"
        else if ($1 ~ /^test\/language\/expressions\//) group="operator"
        else { bad=1; next }
        paths[group]++
        totals[group]+=count
    }
    END {
        if (bad) exit 1
        print "activation_normal_variants=" normal
        print "activation_plain_variants=" plain
        print "activation_async_variants=" async
        print "activation_generated_variants=" generated
        print "activation_no_strict_variants=" no_strict
        split("bigint promise_any reflect_own_keys string_index_of class_expression class_declaration object_literal operator", groups, " ")
        for (i=1; i<=8; i++) {
            group=groups[i]
            print "activation_" group "_paths=" paths[group]
            print "activation_" group "_variants=" totals[group]
        }
    }
' "$activation_manifest" "$metadata_tsv" >"$activation_stats"

reason_stats=$tmp_dir/reason-stats.txt
awk -F'\t' '
    function variants(flags) {
        return flags ~ /(^|,)(module|raw|noStrict|onlyStrict)(,|$)/ ? 1 : 2
    }
    FILENAME == ARGV[1] { supported[$1]=1; next }
    FILENAME == ARGV[2] { wanted[$1]=1; next }
    !($1 in wanted) { next }
    {
        row++
        count=0
        n=split($4, feature, ",")
        for (i=1; i<=n; i++) if (!(feature[i] in supported)) {
            missing[feature[i]]=row
            count++
        }
        if (count == 2 && missing["class-fields-public"] == row &&
            missing["class-static-fields-public"] == row) group="class_fields_public_static"
        else if (count == 2 && missing["class-fields-public"] == row &&
            missing["class"] == row) group="class_fields_public"
        else if (count == 3 && missing["class-fields-public"] == row &&
            missing["class-static-fields-public"] == row && missing["class"] == row) {
            group="class_fields_public_static_with_class"
        } else if (count == 2 && missing["DataView"] == row &&
            missing["DataView.prototype.setUint8"] == row) group="data_view_set_uint8"
        else { bad=1; next }
        paths[group]++
        totals[group]+=variants($3)
    }
    END {
        if (bad) exit 1
        split("class_fields_public_static class_fields_public class_fields_public_static_with_class data_view_set_uint8", groups, " ")
        for (i=1; i<=4; i++) {
            group=groups[i]
            print "reason_only_" group "_paths=" paths[group]
            print "reason_only_" group "_variants=" totals[group]
        }
    }
' "$candidate_features" "$reason_only_manifest" "$metadata_tsv" >"$reason_stats"

config_stats=$tmp_dir/config-stats.txt
awk -F'\t' '
    function variants(flags) {
        return flags ~ /(^|,)(module|raw|noStrict|onlyStrict)(,|$)/ ? 1 : 2
    }
    NR == FNR { wanted[$0]=1; next }
    $1 in wanted {
        if ($1 !~ /^test\/built-ins\/Atomics\/waitAsync\//) bad=1
        total+=variants($3)
    }
    END {
        if (bad) exit 1
        print "config_skipped_atomics_wait_async_variants=" total
    }
' "$config_skipped_manifest" "$metadata_tsv" >"$config_stats"

while IFS='=' read -r key actual; do
    if [[ "$actual" != "$(read_value "$key")" ]]; then
        echo "error: detailed cohort count drifted: $key=$actual" >&2
        exit 1
    fi
done < <(cat "$activation_stats" "$reason_stats" "$config_stats")

if [[ "$(awk -F'\t' '$2 == "sloppy" {n++} END {print n+0}' \
        "$tmp_dir/universe.keys")" != "$(read_value universe_sloppy_variants)" \
    || "$(awk -F'\t' '$2 == "strict" {n++} END {print n+0}' \
        "$tmp_dir/universe.keys")" != "$(read_value universe_strict_variants)" \
    || "$(awk -F'\t' '$2 == "sloppy" {n++} END {print n+0}' \
        "$tmp_dir/activation.keys")" != "$(read_value activation_sloppy_variants)" \
    || "$(awk -F'\t' '$2 == "strict" {n++} END {print n+0}' \
        "$tmp_dir/activation.keys")" != "$(read_value activation_strict_variants)" ]]; then
    echo "error: computed-property-names variant modes drifted" >&2
    exit 1
fi
cat "$tmp_dir/activation.paths" "$tmp_dir/reason_only.paths" \
    "$tmp_dir/config_skipped.paths" "$tmp_dir/module.paths" | sort \
    >"$tmp_dir/partition-union.paths"
if [[ -n "$(uniq -d "$tmp_dir/partition-union.paths")" ]]; then
    echo "error: computed-property-names partitions overlap" >&2
    exit 1
fi
diff -u "$tmp_dir/universe.paths" "$tmp_dir/partition-union.paths"

sort -c "$tmp_dir/full.keys"
comm -12 "$tmp_dir/full.keys" "$tmp_dir/universe.keys" \
    >"$tmp_dir/full-universe.keys"
comm -23 "$tmp_dir/full.keys" "$tmp_dir/universe.keys" \
    >"$tmp_dir/full-non-universe.keys"
diff -u "$tmp_dir/universe.keys" "$tmp_dir/full-universe.keys"
full_variants=$(read_value full_variants)
universe_rows=$(read_value full_universe_rows)
activation_rows=$(read_value full_activation_rows)
reason_rows=$(read_value full_reason_only_rows)
config_rows=$(read_value full_config_skipped_rows)
module_rows=$(read_value full_module_rows)
changed_rows=$(read_value full_changed_rows)
outcome_changed_rows=$(read_value full_outcome_changed_rows)
detail_only_rows=$(read_value full_detail_only_rows)
non_universe_rows=$(read_value full_non_universe_rows)
unchanged_rows=$(read_value full_unchanged_rows)
if [[ "$(wc -l <"$tmp_dir/full.keys" | tr -d '[:space:]')" != "$full_variants" \
    || "$(sha256_file "$tmp_dir/full.keys")" != "$(read_value full_keys_sha256)" \
    || "$universe_rows" != "$(read_value universe_variants)" \
    || "$activation_rows" != "$(read_value activation_variants)" \
    || "$reason_rows" != "$(read_value reason_only_variants)" \
    || "$config_rows" != "$(read_value config_skipped_variants)" \
    || "$module_rows" != "$(read_value module_variants)" \
    || $((activation_rows + reason_rows + config_rows + module_rows)) \
        -ne "$universe_rows" \
    || $((full_variants - universe_rows)) -ne "$non_universe_rows" \
    || "$(wc -l <"$tmp_dir/full-non-universe.keys" | tr -d '[:space:]')" \
        != "$non_universe_rows" \
    || $((activation_rows + reason_rows)) -ne "$changed_rows" \
    || $((outcome_changed_rows + detail_only_rows)) -ne "$changed_rows" \
    || "$activation_rows" != "$outcome_changed_rows" \
    || "$reason_rows" != "$detail_only_rows" \
    || $((full_variants - changed_rows)) -ne "$unchanged_rows" \
    || $((non_universe_rows + config_rows + module_rows)) -ne "$unchanged_rows" \
    || "$(read_value previous_pass_regressions)" != 0 ]]; then
    echo "error: projected full key partition arithmetic drifted" >&2
    exit 1
fi

projected_summary=$(printf '%s\n' "$parent_full_summary" | awk \
    -v activated="$activation_rows" '
    {
        output=""
        for (i=1; i<=NF; i++) {
            split($i, pair, "=")
            if (pair[1] == "pass") pair[2]+=activated
            if (pair[1] == "unsupported-feature") pair[2]-=activated
            output=output (output == "" ? "" : " ") pair[1] "=" pair[2]
        }
        print output
    }
')
expected_candidate_summary=$(read_value expected_candidate_full_summary)
if [[ $(( $(read_value parent_full_runnable) + activation_rows )) \
        -ne "$(read_value expected_candidate_full_runnable)" \
    || $(( $(read_value parent_full_passes) + activation_rows )) \
        -ne "$(read_value expected_candidate_full_passes)" \
    || $(( $(read_value parent_full_unsupported_feature) - activation_rows )) \
        -ne "$(read_value expected_candidate_full_unsupported_feature)" \
    || $(( $(read_value parent_full_total_unsupported) - activation_rows )) \
        -ne "$(read_value expected_candidate_full_total_unsupported)" \
    || "$projected_summary" != "$expected_candidate_summary" \
    || "$(summary_count "$expected_candidate_summary" pass)" \
        != "$(read_value expected_candidate_full_passes)" \
    || "$(summary_count "$expected_candidate_summary" unsupported-feature)" \
        != "$(read_value expected_candidate_full_unsupported_feature)" \
    || "$(unsupported_total "$expected_candidate_summary")" \
        != "$(read_value expected_candidate_full_total_unsupported)" ]]; then
    echo "error: projected full candidate totals or summary drifted" >&2
    exit 1
fi
printf 'Computed property assets pass: 478 paths/946 variants = 439 activation + 456 reason-only + 42 config-skip + 9 module\n'

quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$activation_manifest"
oracle_log=target/test262-computed-property-names-quickjs.log
if ! (
    cd -- "$source_dir"
    ./run-test262 -m -c test262.conf -a -T "$workers" \
        -f "${quickjs_files[@]}"
) >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    echo "error: pinned QuickJS failed the activation manifest" >&2
    exit 1
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || ! grep -Fq \
        "Average memory statistics for $(read_value quickjs_variants) tests:" \
        "$oracle_log" \
    || [[ "$(sha256_file "$oracle_log")" != "$(read_value quickjs_log_sha256)" ]]; then
    tail -n 100 "$oracle_log" >&2
    echo "error: pinned QuickJS activation receipt drifted" >&2
    exit 1
fi
printf 'Pinned QuickJS passes all %s activation variants\n' \
    "$(read_value quickjs_variants)"

report_rows() {
    awk -F'\t' '!/^#/ && !( $1 == "path" && $2 == "variant" ) {print}' "$1"
}

json_rows() {
    awk '/^\{"kind":"result",/' "$1"
}

json_triplets() {
    awk '
        /^\{"kind":"result",/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!match($0, /"variant":"[^"]*"/)) exit 3
            variant=substr($0, RSTART + 11, RLENGTH - 12)
            if (!match($0, /"outcome":"[^"]*"/)) exit 4
            outcome=substr($0, RSTART + 11, RLENGTH - 12)
            print path "\t" variant "\t" outcome
        }
    ' "$1"
}

json_summary() {
    tail -n 1 "$1" | awk '
        /^\{"kind":"summary","outcomes":\{.*\}\}$/ {
            sub(/^\{"kind":"summary","outcomes":\{/, "")
            sub(/\}\}$/, "")
            gsub(/":/, "=")
            gsub(/"/, "")
            gsub(/,/, " ")
            print
            found=1
        }
        END { if (!found) exit 1 }
    '
}

filter_tsv() {
    awk -F'\t' 'NR == FNR {wanted[$0]=1; next} $1 in wanted {print}' "$1" "$2"
}

filter_json() {
    awk 'NR == FNR {wanted[$0]=1; next}
        /^\{"kind":"result",/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (path in wanted) print
        }
    ' "$1" "$2"
}

expected_json_metadata() {
    local profile_hash=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"%s","mode":"%s"}\n' \
        "$(read_value quickjs)" "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" "$profile_hash" \
        "$(read_value schema)" "$(read_value mode)"
}

run_tag_report() {
    local label=$1 profile=$2 report=$tmp_dir/$1.tsv json=$tmp_dir/$1.jsonl
    local output rows=$tmp_dir/$1.rows json_data=$tmp_dir/$1-json.rows
    rm -f -- "$report" "$json"
    output=$(cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$universe_manifest" \
        --report "$report" --mode "$(read_value mode)" \
        --workers "$workers" --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures)
    printf '%s\n' "$output"
    report_rows "$report" >"$rows"
    json_rows "$json" >"$json_data"
    json_triplets "$json" >"$tmp_dir/$label-json.triplets"
    awk -F'\t' 'NF != 10 {exit 1}' "$rows" || {
        echo "error: $label TSV row schema drifted" >&2; exit 1;
    }
    awk -v expected="$(read_value universe_variants)" '
        NR == 1 && /^\{"kind":"metadata",/ { metadata++; next }
        /^\{"kind":"result",/ { results++; next }
        /^\{"kind":"summary",/ { summary++; summary_line=NR; next }
        { exit 1 }
        END {
            if (metadata != 1 || results != expected || summary != 1 ||
                summary_line != NR) exit 1
        }
    ' "$json" || { echo "error: $label JSONL structure drifted" >&2; exit 1; }
    if [[ "$(read_header "$report" quickjs)" != "$(read_value quickjs)" \
        || "$(read_header "$report" test262)" != "$(read_value test262)" \
        || "$(read_header "$report" test262_patch_sha256)" \
            != "$(read_value test262_patch_sha256)" \
        || "$(read_header "$report" test262_config_sha256)" \
            != "$(read_value test262_config_sha256)" \
        || "$(read_header "$report" test262_metadata_sha256)" \
            != "$(read_value test262_metadata_sha256)" \
        || "$(read_header "$report" oxide_profile_sha256)" \
            != "$(read_value "${label}_profile_sha256")" \
        || "$(read_header "$report" profile)" != "$(read_value schema)" \
        || "$(read_header "$report" mode)" != "$(read_value mode)" \
        || "$(head -n 1 "$json")" \
            != "$(expected_json_metadata "$(read_value "${label}_profile_sha256")")" \
        || "$(json_summary "$json")" != "$(read_value "${label}_tag_summary")" \
        || "$(tail -n 1 "$report")" \
            != "# summary $(read_value "${label}_tag_summary")" \
        || "$(execution_runnable "$output")" \
            != "$(read_value "${label}_tag_runnable")" \
        || "$(wc -l <"$rows" | tr -d '[:space:]')" \
            != "$(read_value universe_variants)" \
        || "$(wc -l <"$json_data" | tr -d '[:space:]')" \
            != "$(read_value universe_variants)" \
        || "$(sha256_file "$rows")" \
            != "$(read_value "${label}_tag_tsv_data_sha256")" \
        || "$(sha256_file "$json_data")" \
            != "$(read_value "${label}_tag_jsonl_data_sha256")" \
        || "$(awk -F'\t' '$7 != "pass" {print}' "$rows" | sha256_stream)" \
            != "$(read_value "${label}_tag_nonpass_sha256")" ]]; then
        echo "error: $label complete tag receipt drifted" >&2
        exit 1
    fi
    awk -F'\t' '{print $1 "\t" $2}' "$rows" | sort >"$tmp_dir/$label.keys"
    awk -F'\t' '{print $1 "\t" $2}' "$tmp_dir/$label-json.triplets" \
        | sort >"$tmp_dir/$label-json.keys"
    diff -u "$tmp_dir/universe.keys" "$tmp_dir/$label.keys"
    diff -u "$tmp_dir/universe.keys" "$tmp_dir/$label-json.keys"
    if [[ "$(sha256_file "$report")" \
            != "$(read_value "${label}_tag_tsv_sha256")" \
        || "$(sha256_file "$json")" \
            != "$(read_value "${label}_tag_jsonl_sha256")" ]]; then
        echo "error: $label complete encoded receipt drifted" >&2
        exit 1
    fi
}

run_tag_report parent "$parent_profile"
run_tag_report candidate "$candidate_profile"

verify_partition_receipt() {
    local label=$1 prefix=$2 manifest rows json_rows_file
    manifest=$(read_value "${prefix}_manifest")
    rows=$tmp_dir/$label-$prefix.rows
    json_rows_file=$tmp_dir/$label-$prefix-json.rows
    filter_tsv "$manifest" "$tmp_dir/$label.rows" >"$rows"
    filter_json "$manifest" "$tmp_dir/$label-json.rows" >"$json_rows_file"
    if [[ "$(wc -l <"$rows" | tr -d '[:space:]')" \
            != "$(read_value "${prefix}_variants")" \
        || "$(wc -l <"$json_rows_file" | tr -d '[:space:]')" \
            != "$(read_value "${prefix}_variants")" \
        || "$(sha256_file "$rows")" \
            != "$(read_value "${prefix}_${label}_tsv_data_sha256")" \
        || "$(sha256_file "$json_rows_file")" \
            != "$(read_value "${prefix}_${label}_jsonl_data_sha256")" ]]; then
        echo "error: $label $prefix partition receipt drifted" >&2
        exit 1
    fi
    awk -F'\t' '{print $1 "\t" $2}' "$rows" | sort \
        >"$tmp_dir/$label-$prefix.keys"
    diff -u "$tmp_dir/$prefix.keys" "$tmp_dir/$label-$prefix.keys"
}

for label in parent candidate; do
    for prefix in activation reason_only config_skipped module; do
        verify_partition_receipt "$label" "$prefix"
    done
done

activation_candidate=$tmp_dir/candidate-activation.rows
activation_passes=$(awk -F'\t' '$7 == "pass" {n++} END {print n+0}' \
    "$activation_candidate")
activation_unsupported=$(awk -F'\t' '$7 ~ /^unsupported-/ {n++} END {print n+0}' \
    "$activation_candidate")
activation_skipped=$(awk -F'\t' '$7 ~ /^skipped-/ {n++} END {print n+0}' \
    "$activation_candidate")
activation_failures=$(awk -F'\t' '
    $7 != "pass" && $7 !~ /^unsupported-/ && $7 !~ /^skipped-/ {n++}
    END {print n+0}
' "$activation_candidate")
if [[ "$(read_value activation_candidate_runnable)" \
        != "$(read_value candidate_tag_runnable)" \
    || "$activation_passes" != "$(read_value activation_candidate_passes)" \
    || "$activation_failures" != "$(read_value activation_candidate_failures)" \
    || "$activation_unsupported" \
        != "$(read_value activation_candidate_unsupported)" \
    || "$activation_skipped" != "$(read_value activation_candidate_skipped)" ]]; then
    echo "error: activation candidate execution totals drifted" >&2
    exit 1
fi

for specification in \
    'activation pass' \
    'reason_only unsupported-feature' \
    'config_skipped skipped-feature' \
    'module unsupported-module'
do
    prefix=${specification%% *}
    outcome=${specification#* }
    awk -F'\t' -v outcome="$outcome" '$3 == outcome {print $1 "\t" $2}' \
        "$tmp_dir/candidate-json.triplets" | sort >"$tmp_dir/outcome-json.keys"
    awk -F'\t' -v outcome="$outcome" '$7 == outcome {print $1 "\t" $2}' \
        "$tmp_dir/candidate.rows" | sort >"$tmp_dir/outcome-tsv.keys"
    diff -u "$tmp_dir/$prefix.keys" "$tmp_dir/outcome-tsv.keys"
    diff -u "$tmp_dir/$prefix.keys" "$tmp_dir/outcome-json.keys"
done

if [[ "$(awk -F'\t' '$7 == "pass" {n++} END {print n+0}' \
        "$tmp_dir/candidate.rows")" != "$(read_value candidate_tag_passes)" \
    || "$(awk -F'\t' '$7 ~ /^unsupported-/ {n++} END {print n+0}' \
        "$tmp_dir/candidate.rows")" != "$(read_value candidate_tag_unsupported)" \
    || "$(awk -F'\t' '$7 ~ /^skipped-/ {n++} END {print n+0}' \
        "$tmp_dir/candidate.rows")" != "$(read_value candidate_tag_skipped)" \
    || "$(awk -F'\t' '$7 != "pass" && $7 !~ /^unsupported-/ && $7 !~ /^skipped-/ {n++} END {print n+0}' \
        "$tmp_dir/candidate.rows")" != "$(read_value candidate_tag_failures)" ]]; then
    echo "error: candidate outcome totals drifted" >&2
    exit 1
fi

verify_transition() {
    local kind=$1 expected=$2
    if ! awk -F'\t' -v kind="$kind" -v expected="$expected" '
        function without_tag(detail,    prefix,n,item,i,found,result) {
            prefix="quickjs-oxide does not declare Test262 feature support: "
            if (index(detail, prefix) != 1) return "!bad-prefix!"
            detail=substr(detail, length(prefix) + 1)
            n=split(detail, item, /, /)
            result=""
            for (i=1; i<=n; i++) {
                if (item[i] == "computed-property-names") found++
                else result=result (result == "" ? "" : ", ") item[i]
            }
            if (found != 1) return "!bad-tag-count!"
            return result == "" ? "" : prefix result
        }
        NR == FNR {
            key=$1 SUBSEP $2
            if (key in old) exit 2
            for (i=1; i<=10; i++) before[key,i]=$i
            if (kind == "activation" || kind == "reason_only") {
                if ($7 != "unsupported-feature" || $8 != "selection" ||
                    $9 != "EngineCapability") exit 3
                expected_detail[key]=without_tag($10)
                if (expected_detail[key] ~ /^!bad-/) exit 4
                if (kind == "activation" && expected_detail[key] != "") exit 5
                if (kind == "reason_only" && expected_detail[key] == "") exit 6
            } else if (kind == "config_skipped") {
                if ($7 != "skipped-feature" || $8 != "selection") exit 7
            } else if ($7 != "unsupported-module" || $8 != "selection" ||
                $9 != "ExecutionMode" || $10 != "missing execution capabilities: module") {
                exit 8
            }
            old[key]=1
            old_count++
            next
        }
        {
            key=$1 SUBSEP $2
            if (!(key in old) || key in seen) exit 9
            for (i=1; i<=6; i++) if ($i != before[key,i]) exit 10
            if (kind == "activation") {
                if ($7 != "pass" || $8 != "normal" || $9 != "" || $10 != "") exit 11
            } else if (kind == "reason_only") {
                if ($7 != "unsupported-feature" || $8 != "selection" ||
                    $9 != "EngineCapability" || $10 != expected_detail[key]) exit 12
            } else {
                for (i=7; i<=10; i++) if ($i != before[key,i]) exit 13
            }
            seen[key]=1
            seen_count++
        }
        END {
            if (old_count != expected || seen_count != expected) exit 14
            for (key in old) if (!(key in seen)) exit 15
        }
    ' "$tmp_dir/parent-$kind.rows" "$tmp_dir/candidate-$kind.rows"; then
        echo "error: keyed $kind parent/candidate transition drifted" >&2
        exit 1
    fi
}
verify_transition activation "$(read_value activation_variants)"
verify_transition reason_only "$(read_value reason_only_variants)"
verify_transition config_skipped "$(read_value config_skipped_variants)"
verify_transition module "$(read_value module_variants)"

awk -F= 'NF && $1 !~ /^#/ {print $1}' "$baseline" | sort \
    >"$tmp_dir/all-baseline-keys.txt"
sort -u "$consumed_keys" >"$tmp_dir/consumed-baseline-keys-sorted.txt"
if [[ "$(wc -l <"$tmp_dir/all-baseline-keys.txt" | tr -d '[:space:]')" != 154 ]]; then
    echo "error: computed-property-names baseline key schema drifted" >&2
    exit 1
fi
diff -u "$tmp_dir/all-baseline-keys.txt" \
    "$tmp_dir/consumed-baseline-keys-sorted.txt"

printf 'Computed property names focused gate passes: 946/946 rows classified; QuickJS and Oxide pass all 439 activation variants\n'
