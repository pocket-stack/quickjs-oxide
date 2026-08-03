#!/usr/bin/env bash
# Reproduce the focused rest-parameters admission certificate.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-rest-parameters-baseline.txt
live_profile=compat/test262-oxide.conf
workers=${TEST262_WORKERS:-8}
lock_dir=$root/target/test262-rest-parameters-focused.lock
lock_held=0

if [[ $# -gt 0 ]]; then
    if [[ $# == 1 && ( "$1" == -h || "$1" == --help ) ]]; then
        cat <<'EOF'
usage: scripts/test-test262-rest-parameters.sh

Rebuild the complete rest-parameters metadata inventory, certify its 192
variants in pinned QuickJS, and reproduce the exact parent/candidate Oxide
transition. TEST262_WORKERS controls parallelism (default: 8).
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
    [[ -n "$value" ]] || {
        echo "error: baseline contains an empty $key entry" >&2
        exit 1
    }
    printf '%s\n' "$key" >>"$consumed_keys"
    printf '%s\n' "$value"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    if [[ "$actual" != "$expected" ]]; then
        printf 'error: rest-parameters baseline %s drifted: %s != %s\n' \
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

manifest_data() {
    awk 'NF && $1 !~ /^#/ {print}' "$1"
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

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") {print}' "$1"
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

expected_json_metadata() {
    local profile_hash=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"%s","mode":"%s"}\n' \
        "$(read_value quickjs)" "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" "$profile_hash" \
        "$(read_value schema)" "$(read_value mode)"
}

cleanup() {
    if [[ -n "${tmp_dir-}" && -d "$tmp_dir" ]]; then
        rm -rf -- "$tmp_dir"
    fi
    if [[ "$lock_held" == 1 ]]; then
        rmdir -- "$lock_dir" 2>/dev/null || true
        lock_held=0
    fi
}

cd -- "$root"
[[ -f "$baseline" ]] || { echo "error: missing $baseline" >&2; exit 1; }
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-rest-parameters.XXXXXX")
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p -- "$(dirname -- "$lock_dir")"
if ! mkdir -- "$lock_dir" 2>/dev/null; then
    echo "error: another focused rest-parameters gate holds $lock_dir" >&2
    exit 1
fi
lock_held=1
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
expect_value parent_features 91
expect_value parent_negative_paths 828
expect_value candidate_features 92
expect_value candidate_negative_paths 924
expect_value added_features 1
expect_value added_negative_paths 96
expect_value profile_execution_entries 1
expect_value universe_paths 96
expect_value universe_variants 192
expect_value universe_sloppy_variants 96
expect_value universe_strict_variants 96
expect_value activation_paths 96
expect_value activation_variants 192
expect_value metadata_flags generated
expect_value metadata_expected_phase parse
expect_value metadata_expected_type SyntaxError
expect_value tag_only_paths 39
expect_value async_functions_dependency_paths 9
expect_value async_iteration_dependency_paths 27
expect_value generators_dependency_paths 21
expect_value parent_runnable 0
expect_value parent_passes 0
expect_value parent_failures 0
expect_value parent_unsupported 192
expect_value parent_skipped 0
expect_value parent_summary unsupported-feature=192
expect_value candidate_runnable 192
expect_value candidate_passes 192
expect_value candidate_failures 0
expect_value candidate_unsupported 0
expect_value candidate_skipped 0
expect_value candidate_summary pass=192
expect_value quickjs_variants 192

parent_profile=$(read_value parent_profile)
candidate_profile=$(read_value candidate_profile)
universe_manifest=$(read_value universe_manifest)
activation_manifest=$(read_value activation_manifest)
for asset in "$live_profile" "$parent_profile" "$candidate_profile" \
    "$universe_manifest" "$activation_manifest"; do
    [[ -f "$asset" ]] || { echo "error: missing gate input: $asset" >&2; exit 1; }
done

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

parent_features_file=$tmp_dir/parent-features.txt
candidate_features_file=$tmp_dir/candidate-features.txt
parent_negatives=$tmp_dir/parent-negatives.txt
candidate_negatives=$tmp_dir/candidate-negatives.txt
parent_execution=$tmp_dir/parent-execution.txt
candidate_execution=$tmp_dir/candidate-execution.txt
added_features_file=$tmp_dir/added-features.txt
added_negatives=$tmp_dir/added-negatives.txt
profile_section "$parent_profile" features >"$parent_features_file"
profile_section "$candidate_profile" features >"$candidate_features_file"
profile_section "$parent_profile" audited-negative-tests >"$parent_negatives"
profile_section "$candidate_profile" audited-negative-tests >"$candidate_negatives"
profile_section "$parent_profile" execution >"$parent_execution"
profile_section "$candidate_profile" execution >"$candidate_execution"
for sorted in "$parent_features_file" "$candidate_features_file" \
    "$parent_negatives" "$candidate_negatives"; do
    sort -c "$sorted"
done
comm -13 "$parent_features_file" "$candidate_features_file" >"$added_features_file"
comm -23 "$parent_features_file" "$candidate_features_file" >"$tmp_dir/removed-features.txt"
comm -13 "$parent_negatives" "$candidate_negatives" >"$added_negatives"
comm -23 "$parent_negatives" "$candidate_negatives" >"$tmp_dir/removed-negatives.txt"

if [[ "$(sha256_file "$parent_profile")" \
        != "$(read_value parent_profile_sha256)" \
    || "$(sha256_file "$candidate_profile")" \
        != "$(read_value candidate_profile_sha256)" \
    || "$(wc -l <"$parent_features_file" | tr -d '[:space:]')" \
        != "$(read_value parent_features)" \
    || "$(sha256_file "$parent_features_file")" \
        != "$(read_value parent_features_sha256)" \
    || "$(wc -l <"$candidate_features_file" | tr -d '[:space:]')" \
        != "$(read_value candidate_features)" \
    || "$(sha256_file "$candidate_features_file")" \
        != "$(read_value candidate_features_sha256)" \
    || "$(wc -l <"$added_features_file" | tr -d '[:space:]')" \
        != "$(read_value added_features)" \
    || "$(sha256_file "$added_features_file")" \
        != "$(read_value added_features_sha256)" \
    || "$(cat "$added_features_file")" != rest-parameters \
    || -s "$tmp_dir/removed-features.txt" \
    || "$(wc -l <"$parent_negatives" | tr -d '[:space:]')" \
        != "$(read_value parent_negative_paths)" \
    || "$(sha256_file "$parent_negatives")" \
        != "$(read_value parent_negative_sha256)" \
    || "$(wc -l <"$candidate_negatives" | tr -d '[:space:]')" \
        != "$(read_value candidate_negative_paths)" \
    || "$(sha256_file "$candidate_negatives")" \
        != "$(read_value candidate_negative_sha256)" \
    || "$(wc -l <"$added_negatives" | tr -d '[:space:]')" \
        != "$(read_value added_negative_paths)" \
    || "$(sha256_file "$added_negatives")" \
        != "$(read_value added_negative_sha256)" \
    || -s "$tmp_dir/removed-negatives.txt" ]]; then
    echo "error: parent/candidate capability transition drifted" >&2
    exit 1
fi
diff -u "$parent_execution" "$candidate_execution"
if [[ "$(wc -l <"$candidate_execution" | tr -d '[:space:]')" \
        != "$(read_value profile_execution_entries)" \
    || "$(sha256_file "$candidate_execution")" \
        != "$(read_value profile_execution_sha256)" \
    || "$(cat "$candidate_execution")" != async=true ]]; then
    echo "error: profile execution policy drifted" >&2
    exit 1
fi
if ! cmp -s "$live_profile" "$parent_profile" \
    && ! cmp -s "$live_profile" "$candidate_profile"; then
    echo "error: live profile is neither the frozen parent nor candidate" >&2
    exit 1
fi

metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
runner=$tmp_dir/run-test262
cargo build --locked --release --quiet --bin run-test262
cp -- target/release/run-test262 "$runner"
chmod 755 "$runner"
[[ -x "$runner" ]] || {
    echo "error: failed to stage a stable focused run-test262 binary" >&2
    exit 1
}
"$runner" --suite "$suite" --validate-metadata "$metadata_records"
if [[ "$(sha256_file "$metadata_records")" \
        != "$(read_value test262_metadata_sha256)" ]]; then
    echo "error: exhaustive Test262 metadata fingerprint drifted" >&2
    exit 1
fi
tr '\0' '\t' <"$metadata_records" >"$metadata_tsv"
if [[ "$(wc -l <"$metadata_tsv" | tr -d '[:space:]')" \
        != "$(read_value test262_metadata_records)" ]] \
    || ! awk -F'\t' 'NF != 6 || $1 == "" {exit 1}' "$metadata_tsv" \
    || ! cut -f1 "$metadata_tsv" | sort -c \
    || [[ -n "$(cut -f1 "$metadata_tsv" | uniq -d)" ]]; then
    echo "error: exhaustive Test262 metadata record structure drifted" >&2
    exit 1
fi

skipped_features=$tmp_dir/skipped-features.txt
awk '
    $0 == "[features]" {inside=1; next}
    /^\[/ {inside=0}
    inside && NF && $1 !~ /^#/ && /=skip$/ {sub(/=skip$/, ""); print}
' "$source_dir/test262.conf" | sort -u >"$skipped_features"

metadata_paths=$tmp_dir/metadata-paths.txt
metadata_keys=$tmp_dir/metadata-keys.txt
metadata_stats=$tmp_dir/metadata-stats.txt
awk -F'\t' \
    -v paths="$metadata_paths" -v keys="$metadata_keys" \
    -v stats="$metadata_stats" '
    function has(list, value) {return index("," list ",", "," value ",") != 0}
    FILENAME == ARGV[1] {supported[$1]=1; next}
    FILENAME == ARGV[2] {skipped[$1]=1; next}
    !has($4, "rest-parameters") {next}
    {
        print $1 >paths
        print $1 "\tsloppy" >keys
        print $1 "\tstrict" >keys
        if ($2 != "" || $3 != "generated" || $5 != "parse" ||
            $6 != "SyntaxError") bad=1
        missing=0
        n=split($4, feature, ",")
        for (i=1; i<=n; i++) {
            if (feature[i] in skipped) bad=1
            if (!(feature[i] in supported)) missing++
        }
        if (missing != 1 || has($3, "module") || has($3, "raw") ||
            has($3, "noStrict") || has($3, "onlyStrict")) bad=1
        if ($4 == "rest-parameters") tag_only++
        else if ($4 == "rest-parameters,async-functions") async_functions++
        else if ($4 == "rest-parameters,async-iteration") async_iteration++
        else if ($4 == "rest-parameters,generators") generators++
        else bad=1
    }
    END {
        print "tag_only_paths=" tag_only >stats
        print "async_functions_dependency_paths=" async_functions >stats
        print "async_iteration_dependency_paths=" async_iteration >stats
        print "generators_dependency_paths=" generators >stats
        if (bad) exit 1
    }
' "$parent_features_file" "$skipped_features" "$metadata_tsv" || {
    echo "error: rest-parameters metadata contract drifted" >&2
    exit 1
}

universe_paths_file=$tmp_dir/universe-paths.txt
activation_paths_file=$tmp_dir/activation-paths.txt
manifest_data "$universe_manifest" >"$universe_paths_file"
manifest_data "$activation_manifest" >"$activation_paths_file"
for sorted in "$universe_paths_file" "$activation_paths_file" \
    "$metadata_paths" "$metadata_keys"; do
    sort -c "$sorted"
done
diff -u "$universe_paths_file" "$metadata_paths"
diff -u "$activation_paths_file" "$metadata_paths"
diff -u "$activation_paths_file" "$added_negatives"
while IFS='=' read -r key actual; do
    [[ "$actual" == "$(read_value "$key")" ]] || {
        echo "error: metadata cohort count drifted: $key=$actual" >&2
        exit 1
    }
done <"$metadata_stats"
if [[ "$(sha256_file "$universe_manifest")" \
        != "$(read_value universe_manifest_sha256)" \
    || "$(sha256_file "$universe_paths_file")" \
        != "$(read_value universe_paths_sha256)" \
    || "$(wc -l <"$universe_paths_file" | tr -d '[:space:]')" \
        != "$(read_value universe_paths)" \
    || "$(sha256_file "$activation_manifest")" \
        != "$(read_value activation_manifest_sha256)" \
    || "$(sha256_file "$activation_paths_file")" \
        != "$(read_value activation_paths_sha256)" \
    || "$(wc -l <"$activation_paths_file" | tr -d '[:space:]')" \
        != "$(read_value activation_paths)" \
    || "$(wc -l <"$metadata_keys" | tr -d '[:space:]')" \
        != "$(read_value universe_variants)" \
    || "$(wc -l <"$metadata_keys" | tr -d '[:space:]')" \
        != "$(read_value activation_variants)" \
    || "$(awk -F'\t' '$2 == "sloppy" {n++} END {print n+0}' "$metadata_keys")" \
        != "$(read_value universe_sloppy_variants)" \
    || "$(awk -F'\t' '$2 == "strict" {n++} END {print n+0}' "$metadata_keys")" \
        != "$(read_value universe_strict_variants)" \
    || "$(sha256_file "$metadata_keys")" \
        != "$(read_value universe_keys_sha256)" \
    || "$(sha256_file "$metadata_keys")" \
        != "$(read_value activation_keys_sha256)" ]]; then
    echo "error: rest-parameters inventory drifted" >&2
    exit 1
fi
printf 'Rest parameters assets pass: 96 generated parse-negative paths / 192 variants\n'

quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$activation_paths_file"
oracle_log=target/test262-rest-parameters-quickjs.log
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
    || [[ "$(sha256_file "$oracle_log")" \
        != "$(read_value quickjs_log_sha256)" ]]; then
    tail -n 100 "$oracle_log" >&2
    echo "error: pinned QuickJS activation receipt drifted" >&2
    exit 1
fi
printf 'Pinned QuickJS passes all %s activation variants\n' \
    "$(read_value quickjs_variants)"

run_report() {
    local label=$1 profile=$2
    local report=target/test262-rest-parameters-$label.tsv
    local json=target/test262-rest-parameters-$label.jsonl
    local output rows=$tmp_dir/$label.rows json_data=$tmp_dir/$label-json.rows
    output=$("$runner" \
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
        echo "error: $label TSV row schema drifted" >&2
        exit 1
    }
    awk -v expected="$(read_value universe_variants)" '
        NR == 1 && /^\{"kind":"metadata",/ {metadata++; next}
        /^\{"kind":"result",/ {results++; next}
        /^\{"kind":"summary",/ {summary++; summary_line=NR; next}
        {exit 1}
        END {
            if (metadata != 1 || results != expected || summary != 1 ||
                summary_line != NR) exit 1
        }
    ' "$json" || { echo "error: $label JSONL structure drifted" >&2; exit 1; }

    local profile_hash expected_summary
    profile_hash=$(read_value "${label}_profile_sha256")
    expected_summary=$(read_value "${label}_summary")
    if [[ "$(read_header "$report" quickjs)" != "$(read_value quickjs)" \
        || "$(read_header "$report" test262)" != "$(read_value test262)" \
        || "$(read_header "$report" test262_patch_sha256)" \
            != "$(read_value test262_patch_sha256)" \
        || "$(read_header "$report" test262_config_sha256)" \
            != "$(read_value test262_config_sha256)" \
        || "$(read_header "$report" test262_metadata_sha256)" \
            != "$(read_value test262_metadata_sha256)" \
        || "$(read_header "$report" oxide_profile_sha256)" != "$profile_hash" \
        || "$(read_header "$report" profile)" != "$(read_value schema)" \
        || "$(read_header "$report" mode)" != "$(read_value mode)" \
        || "$(head -n 1 "$json")" != "$(expected_json_metadata "$profile_hash")" \
        || "$(execution_runnable "$output")" != "$(read_value "${label}_runnable")" \
        || "$(tail -n 1 "$report")" != "# summary $expected_summary" \
        || "$(json_summary "$json")" != "$expected_summary" \
        || "$(wc -l <"$rows" | tr -d '[:space:]')" \
            != "$(read_value universe_variants)" \
        || "$(wc -l <"$json_data" | tr -d '[:space:]')" \
            != "$(read_value universe_variants)" \
        || "$(sha256_file "$rows")" \
            != "$(read_value "${label}_tsv_data_sha256")" \
        || "$(sha256_file "$json_data")" \
            != "$(read_value "${label}_jsonl_data_sha256")" \
        || "$(awk -F'\t' '$7 != "pass" {print}' "$rows" | sha256_stream)" \
            != "$(read_value "${label}_nonpass_sha256")" \
        || "$(sha256_file "$report")" != "$(read_value "${label}_tsv_sha256")" \
        || "$(sha256_file "$json")" != "$(read_value "${label}_jsonl_sha256")" ]]; then
        echo "error: $label report receipt drifted" >&2
        exit 1
    fi

    local passes unsupported skipped failures total
    total=$(wc -l <"$rows" | tr -d '[:space:]')
    passes=$(awk -F'\t' '$7 == "pass" {n++} END {print n+0}' "$rows")
    unsupported=$(awk -F'\t' '$7 ~ /^unsupported-/ {n++} END {print n+0}' "$rows")
    skipped=$(awk -F'\t' '$7 ~ /^skipped-/ {n++} END {print n+0}' "$rows")
    failures=$((total - passes - unsupported - skipped))
    if [[ "$passes" != "$(read_value "${label}_passes")" \
        || "$failures" != "$(read_value "${label}_failures")" \
        || "$unsupported" != "$(read_value "${label}_unsupported")" \
        || "$skipped" != "$(read_value "${label}_skipped")" ]]; then
        echo "error: $label outcome totals drifted" >&2
        exit 1
    fi
    awk -F'\t' '{print $1 "\t" $2}' "$rows" | sort >"$tmp_dir/$label.keys"
    awk -F'\t' '{print $1 "\t" $2}' "$tmp_dir/$label-json.triplets" \
        | sort >"$tmp_dir/$label-json.keys"
    awk -F'\t' '{print $1 "\t" $2 "\t" $7}' "$rows" | sort \
        >"$tmp_dir/$label-tsv.triplets"
    sort "$tmp_dir/$label-json.triplets" >"$tmp_dir/$label-json-triplets.sorted"
    diff -u "$metadata_keys" "$tmp_dir/$label.keys"
    diff -u "$metadata_keys" "$tmp_dir/$label-json.keys"
    diff -u "$tmp_dir/$label-tsv.triplets" \
        "$tmp_dir/$label-json-triplets.sorted"
}

run_report parent "$parent_profile"
run_report candidate "$candidate_profile"

if ! awk -F'\t' -v expected="$(read_value universe_variants)" '
    NR == FNR {
        key=$1 SUBSEP $2
        if (key in old || $7 != "unsupported-feature" || $8 != "selection" ||
            $9 != "EngineCapability" ||
            $10 != "quickjs-oxide does not declare Test262 feature support: rest-parameters") exit 2
        for (i=1; i<=6; i++) before[key,i]=$i
        old[key]=1
        old_count++
        next
    }
    {
        key=$1 SUBSEP $2
        if (!(key in old) || key in seen) exit 3
        for (i=1; i<=6; i++) if ($i != before[key,i]) exit 4
        if ($7 != "pass" || $8 != "parse" || $9 != "SyntaxError" ||
            $10 != "\"use strict\" not allowed in function with default or destructuring parameter") exit 5
        seen[key]=1
        seen_count++
    }
    END {
        if (old_count != expected || seen_count != expected) exit 6
        for (key in old) if (!(key in seen)) exit 7
    }
' "$tmp_dir/parent.rows" "$tmp_dir/candidate.rows"; then
    echo "error: keyed parent/candidate transition drifted" >&2
    exit 1
fi

awk -F= 'NF && $1 !~ /^#/ {print $1}' "$baseline" | sort \
    >"$tmp_dir/all-baseline-keys.txt"
sort -u "$consumed_keys" >"$tmp_dir/consumed-baseline-keys-sorted.txt"
if [[ -n "$(awk -F= 'NF && $1 !~ /^#/ {seen[$1]++} END {for (k in seen) if (seen[k] != 1) print k}' "$baseline")" ]]; then
    echo "error: duplicate baseline keys" >&2
    exit 1
fi
diff -u "$tmp_dir/all-baseline-keys.txt" \
    "$tmp_dir/consumed-baseline-keys-sorted.txt"

printf 'Rest parameters focused gate passes: QuickJS and Oxide pass all 192 activation variants\n'
