#!/usr/bin/env bash
# Reproduce the R3bp global globalThis admission and whole-corpus join.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-global-this-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
parent_profile=tests/test262-global-this-global-parent.conf
candidate_profile=tests/test262-global-this-global-candidate.conf
focused_parent_profile=tests/test262-global-this-parent.conf
focused_candidate_profile=tests/test262-global-this-candidate.conf
live_profile=compat/test262-oxide.conf
focused_baseline=tests/test262-global-this-baseline.txt
universe_manifest=tests/test262-global-this.txt
activation_manifest=tests/test262-global-this-activation.txt
deferred_manifest=tests/test262-global-this-deferred.txt
focused_transition=tests/test262-global-this-transitions.tsv
tag_transition=tests/test262-global-this-global-transitions.tsv
before_tag_report=target/test262-global-this-global-before.tsv
before_tag_json_report=target/test262-global-this-global-before.jsonl
candidate_tag_report=target/test262-global-this-global-candidate.tsv
candidate_tag_json_report=target/test262-global-this-global-candidate.jsonl
before_full_report=target/test262-global-this-global-before-full.tsv
before_full_json_report=target/test262-global-this-global-before-full.jsonl
candidate_full_report=target/test262-global-this-global-candidate-full.tsv
candidate_full_json_report=target/test262-global-this-global-candidate-full.jsonl
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}

usage() {
    printf 'usage: %s [--check|--bless|--full|--bless-full]\n' "${0##*/}"
    printf '  --check       verify frozen R3bo evidence and R3bp profile wiring only\n'
    printf '  --bless       fill the 165-row tag receipt after its exact join\n'
    printf '  --full        reproduce the frozen 102,037-key admission join\n'
    printf '  --bless-full  fill every PENDING receipt after an exact no-regression join\n'
}

mode=tag
case ${1-} in
    "") ;;
    --check) mode=check ;;
    --bless) mode=bless ;;
    --full) mode=full ;;
    --bless-full) mode=bless-full ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }

read_value_from() {
    local file=$1 key=$2
    awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$file"
}

read_value() {
    read_value_from "$baseline" "$1"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    [[ "$actual" == "$expected" ]] || {
        printf 'error: R3bp baseline identity drifted for %s: %s != %s\n' \
            "$key" "$actual" "$expected" >&2
        exit 1
    }
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    else
        shasum -a 256 | awk '{ print $1 }'
    fi
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$1"
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

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' "$1"
}

report_keys() {
    report_rows "$1" | awk -F'\t' '{ print $1 "\t" $2 }' | LC_ALL=C sort
}

json_report_keys() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3bp JSONL report %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        /^\{"kind":"metadata",/ {
            metadata++
            if (NR != 1) fail("metadata record is not first")
            next
        }
        /^\{"kind":"result",/ {
            if (summary) fail("result appears after summary")
            if (!match($0, /"path":"[^"]*"/)) fail("result is missing path")
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!match($0, /"variant":"[^"]*"/)) fail("result is missing variant")
            variant=substr($0, RSTART + 11, RLENGTH - 12)
            key=path "\t" variant
            if (seen[key]++) fail("duplicate result key")
            print key
            results++
            next
        }
        /^\{"kind":"summary",/ {
            summary++
            summary_line=NR
            next
        }
        { fail("unexpected record") }
        END {
            if (!failed && metadata != 1) fail("expected exactly one metadata record")
            if (!failed && summary != 1) fail("expected exactly one summary record")
            if (!failed && summary_line != NR) fail("summary record is not last")
        }
    ' "$report" | LC_ALL=C sort
}

json_report_summary() {
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

report_summary() {
    tail -n 1 "$1" | sed 's/^# summary //'
}

expected_json_metadata() {
    local profile_hash=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"%s","mode":"%s"}\n' \
        "$(read_value quickjs)" \
        "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" \
        "$profile_hash" \
        "$(read_value schema)" \
        "$(read_value mode)"
}

verify_report() {
    local report=$1 json_report=$2 profile_hash=$3 expected_variants=$4
    local tsv_keys json_keys
    tsv_keys=$(report_keys "$report")
    if ! json_keys=$(json_report_keys "$json_report"); then
        exit 1
    fi
    [[ "$(read_header "$report" quickjs)" == "$(read_value quickjs)" \
        && "$(read_header "$report" test262)" == "$(read_value test262)" \
        && "$(read_header "$report" test262_patch_sha256)" \
            == "$(read_value test262_patch_sha256)" \
        && "$(read_header "$report" test262_config_sha256)" \
            == "$(read_value test262_config_sha256)" \
        && "$(read_header "$report" test262_metadata_sha256)" \
            == "$(read_value test262_metadata_sha256)" \
        && "$(read_header "$report" oxide_profile_sha256)" == "$profile_hash" \
        && "$(read_header "$report" profile)" == "$(read_value schema)" \
        && "$(read_header "$report" mode)" == "$(read_value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
            == "$expected_variants" ]] || {
        printf 'error: R3bp report metadata drifted: %s\n' "$report" >&2
        exit 1
    }
    diff -u <(printf '%s\n' "$tsv_keys") <(printf '%s\n' "$json_keys")
    [[ "$(head -n 1 "$json_report")" == "$(expected_json_metadata "$profile_hash")" \
        && "$(json_report_summary "$json_report")" == "$(report_summary "$report")" ]] || {
        printf 'error: R3bp JSONL metadata or summary drifted: %s\n' "$json_report" >&2
        exit 1
    }
}

rows_for_paths() {
    local paths=$1 report=$2
    awk -F'\t' '
        NR == FNR { if ($0 != "") wanted[$0]=1; next }
        !/^#/ && !($1 == "path" && $2 == "variant") && ($1 in wanted) { print }
    ' <(printf '%s\n' "$paths") "$report"
}

rows_without_paths() {
    local paths=$1 report=$2
    awk -F'\t' '
        NR == FNR { if ($0 != "") blocked[$0]=1; next }
        !/^#/ && !($1 == "path" && $2 == "variant") && !($1 in blocked) { print }
    ' <(printf '%s\n' "$paths") "$report"
}

json_rows_for_paths() {
    local paths=$1 report=$2
    awk '
        NR == FNR { if ($0 != "") wanted[$0]=1; next }
        /^\{"kind":"result"/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (path in wanted) print
        }
    ' <(printf '%s\n' "$paths") "$report"
}

json_rows_without_paths() {
    local paths=$1 report=$2
    awk '
        NR == FNR { if ($0 != "") blocked[$0]=1; next }
        /^\{"kind":"result"/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!(path in blocked)) print
        }
    ' <(printf '%s\n' "$paths") "$report"
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

report_outcome_count() {
    local report=$1 outcome=$2
    report_rows "$report" | awk -F'\t' -v outcome="$outcome" '
        $7 == outcome { count++ }
        END { print count + 0 }
    '
}

report_nonpass_sha256() {
    report_rows "$1" | awk -F'\t' '$7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' | sha256_stream
}

verify_deferred_vector() {
    local label=$1 rows=$2 json_rows=$3
    local key_hash module_rows config_rows json_module_rows json_config_rows counts
    key_hash=$(
        printf '%s\n' "$rows" \
            | awk -F'\t' '{ print $1 "\t" $2 }' \
            | LC_ALL=C sort \
            | sha256_stream
    )
    if ! counts=$(printf '%s\n' "$rows" | awk -F'\t' '
            $7 == "unsupported-module" {
                if ($8 != "selection" ||
                    $9 != "ExecutionMode" ||
                    $10 != "missing execution capabilities: module") exit 2
                module++
                next
            }
            $7 == "skipped-feature" {
                if ($8 != "selection" ||
                    $9 != "" ||
                    $10 != "QuickJS config skips feature explicit-resource-management") {
                    exit 3
                }
                config++
                next
            }
            { exit 4 }
            END { print module + 0, config + 0 }
        '); then
        printf 'error: R3bp %s deferred TSV vector drifted\n' "$label" >&2
        exit 1
    fi
    read -r module_rows config_rows <<<"$counts"
    if ! counts=$(printf '%s\n' "$json_rows" | awk '
            /"outcome":"unsupported-module"/ {
                if ($0 !~ /"outcome":"unsupported-module","actual_phase":"selection","actual_type":"ExecutionMode","detail":"missing execution capabilities: module"\}$/) {
                    exit 2
                }
                module++
                next
            }
            /"outcome":"skipped-feature"/ {
                if ($0 !~ /"outcome":"skipped-feature","actual_phase":"selection","actual_type":"","detail":"QuickJS config skips feature explicit-resource-management"\}$/) {
                    exit 3
                }
                config++
                next
            }
            { exit 4 }
            END { print module + 0, config + 0 }
        '); then
        printf 'error: R3bp %s deferred JSONL vector drifted\n' "$label" >&2
        exit 1
    fi
    read -r json_module_rows json_config_rows <<<"$counts"
    [[ "$(printf '%s\n' "$rows" | wc -l | tr -d '[:space:]')" \
            == "$(read_value tag_deferred_rows)" \
        && "$(printf '%s\n' "$json_rows" | wc -l | tr -d '[:space:]')" \
            == "$(read_value tag_deferred_rows)" \
        && "$key_hash" == "$(read_value deferred_keys_sha256)" \
        && "$module_rows" == "$(read_value tag_module_rows)" \
        && "$config_rows" == "$(read_value tag_config_rows)" \
        && "$json_module_rows" == "$(read_value tag_module_rows)" \
        && "$json_config_rows" == "$(read_value tag_config_rows)" ]] || {
        printf 'error: R3bp %s deferred vector drifted\n' "$label" >&2
        exit 1
    }
}

focused_before_rows() {
    awk -F'\t' -v OFS='\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            print $1,$2,$3,$4,$5,$6,$7,$8,$9,$10
        }
    ' "$focused_transition"
}

focused_candidate_rows() {
    awk -F'\t' -v OFS='\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            print $1,$2,$3,$4,$5,$6,$11,$12,$13,$14
        }
    ' "$focused_transition"
}

pending_keys() {
    local key
    for key in "$@"; do
        [[ "$(read_value "$key")" == PENDING ]] && printf '%s\n' "$key"
    done
    return 0
}

update_values() {
    local file=$1
    shift
    local updates_tmp output_tmp entry
    updates_tmp=$(mktemp "$file.updates.XXXXXX")
    output_tmp=$(mktemp "$file.XXXXXX")
    for entry in "$@"; do
        printf '%s\n' "$entry"
    done >"$updates_tmp"
    awk -F= '
        NR == FNR {
            key=$1
            sub(/^[^=]*=/, "")
            replacement[key]=$0
            next
        }
        $1 in replacement {
            print $1 "=" replacement[$1]
            seen[$1]=1
            next
        }
        { print }
        END {
            for (key in replacement) if (!(key in seen)) {
                print "missing baseline key: " key >"/dev/stderr"
                bad=1
            }
            if (bad) exit 1
        }
    ' "$updates_tmp" "$file" >"$output_tmp"
    chmod 644 "$output_tmp"
    mv -- "$output_tmp" "$file"
    unlink "$updates_tmp"
}

verify_canonical_baseline() {
    local state=$1
    local expected_runnable expected_passes expected_tsv expected_jsonl expected_summary
    if [[ "$state" == parent ]]; then
        expected_runnable=$(read_value before_full_runnable)
        expected_passes=$(read_value before_full_passes)
        expected_tsv=$(read_value before_full_tsv_sha256)
        expected_jsonl=$(read_value before_full_jsonl_sha256)
        expected_summary=$(read_value before_full_summary)
    else
        expected_runnable=$(read_value candidate_full_runnable)
        expected_passes=$(read_value candidate_full_passes)
        expected_tsv=$(read_value candidate_full_tsv_sha256)
        expected_jsonl=$(read_value candidate_full_jsonl_sha256)
        expected_summary=$(read_value candidate_full_summary)
    fi
    [[ "$(read_value_from "$canonical_baseline" schema)" == "$(read_value schema)" \
        && "$(read_value_from "$canonical_baseline" timeout_ms)" \
            == "$(read_value timeout_ms)" \
        && "$(read_value_from "$canonical_baseline" variants)" \
            == "$(read_value full_variants)" \
        && "$(read_value_from "$canonical_baseline" runnable)" == "$expected_runnable" \
        && "$(read_value_from "$canonical_baseline" passes)" == "$expected_passes" \
        && "$(read_value_from "$canonical_baseline" tsv_sha256)" == "$expected_tsv" \
        && "$(read_value_from "$canonical_baseline" jsonl_sha256)" == "$expected_jsonl" \
        && "$(read_value_from "$canonical_baseline" summary)" == "$expected_summary" ]] || {
        printf 'error: R3bp canonical full baseline is not the expected %s vector\n' \
            "$state" >&2
        exit 1
    }
}

cd -- "$root"

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value parent_profile tests/test262-global-this-global-parent.conf
expect_value parent_profile_sha256 8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910
expect_value parent_features 83
expect_value candidate_profile tests/test262-global-this-global-candidate.conf
expect_value candidate_profile_sha256 caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15
expect_value candidate_features 84
expect_value profile_negative_paths 828
expect_value profile_execution_entries 1
expect_value universe_paths 148
expect_value universe_variants 165
expect_value activation_paths 135
expect_value activation_variants 150
expect_value deferred_paths 13
expect_value deferred_variants 15
expect_value focused_transition_rows 150
expect_value tag_rows 165
expect_value tag_activation_rows 150
expect_value tag_deferred_rows 15
expect_value tag_module_rows 11
expect_value tag_config_rows 4
expect_value full_variants 102037
expect_value full_activation_rows 150
expect_value full_deferred_rows 15
expect_value full_non_tag_rows 101872
expect_value full_unchanged_rows 101887
expect_value full_changed_rows 150
expect_value full_detail_only_rows 0
expect_value previous_pass_regressions 0
expect_value before_full_runnable 58121
expect_value before_full_passes 57602
expect_value before_full_unsupported 20523
expect_value candidate_full_runnable 58271
expect_value candidate_full_passes 57752
expect_value candidate_full_unsupported 20373

for required in \
    "$parent_profile" "$candidate_profile" \
    "$focused_parent_profile" "$focused_candidate_profile" "$live_profile" \
    "$focused_baseline" "$universe_manifest" "$activation_manifest" \
    "$deferred_manifest" "$focused_transition" "$canonical_baseline"
do
    [[ -f "$required" ]] || {
        printf 'error: missing R3bp asset: %s\n' "$required" >&2
        exit 1
    }
done

[[ "$(sha256_file "$parent_profile")" == "$(read_value parent_profile_sha256)" \
    && "$(sha256_file "$candidate_profile")" == "$(read_value candidate_profile_sha256)" \
    && "$(sha256_file "$focused_baseline")" == "$(read_value focused_baseline_sha256)" \
    && "$(sha256_file "$universe_manifest")" \
        == "$(read_value universe_manifest_sha256)" \
    && "$(sha256_file "$activation_manifest")" \
        == "$(read_value activation_manifest_sha256)" \
    && "$(sha256_file "$deferred_manifest")" \
        == "$(read_value deferred_manifest_sha256)" ]] || {
    echo "error: R3bp frozen profile or R3bo asset bytes drifted" >&2
    exit 1
}
cmp -s "$parent_profile" "$focused_parent_profile" || {
    echo "error: R3bp parent is not the byte-exact R3bo parent" >&2
    exit 1
}
cmp -s "$candidate_profile" "$focused_candidate_profile" || {
    echo "error: R3bp candidate is not the byte-exact R3bo candidate" >&2
    exit 1
}

parent_features=$(profile_section "$parent_profile" features)
candidate_features=$(profile_section "$candidate_profile" features)
live_features=$(profile_section "$live_profile" features)
parent_negatives=$(profile_section "$parent_profile" audited-negative-tests)
candidate_negatives=$(profile_section "$candidate_profile" audited-negative-tests)
live_negatives=$(profile_section "$live_profile" audited-negative-tests)
parent_execution=$(profile_section "$parent_profile" execution)
candidate_execution=$(profile_section "$candidate_profile" execution)
live_execution=$(profile_section "$live_profile" execution)
diff -u <(printf '%s\n' globalThis) \
    <(comm -13 <(printf '%s\n' "$parent_features") <(printf '%s\n' "$candidate_features"))
[[ -z "$(comm -23 \
    <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))" ]] || {
    echo "error: R3bp candidate lost a parent feature" >&2
    exit 1
}
diff -u <(printf '%s\n' "$parent_negatives") <(printf '%s\n' "$candidate_negatives")
diff -u <(printf '%s\n' "$parent_execution") <(printf '%s\n' "$candidate_execution")
[[ -z "$(comm -23 \
        <(printf '%s\n' "$candidate_features") \
        <(printf '%s\n' "$live_features"))" \
    && -z "$(comm -23 \
        <(printf '%s\n' "$candidate_negatives") \
        <(printf '%s\n' "$live_negatives"))" ]] || {
    echo "error: the live profile lost an R3bp candidate capability" >&2
    exit 1
}
diff -u <(printf '%s\n' "$candidate_execution") <(printf '%s\n' "$live_execution")
for tuple in \
    "parent_features:$parent_features" \
    "candidate_features:$candidate_features" \
    "profile_negative:$candidate_negatives" \
    "profile_execution:$candidate_execution"
do
    name=${tuple%%:*}
    inventory=${tuple#*:}
    case "$name" in
        parent_features | candidate_features) count_key=$name ;;
        profile_negative) count_key=profile_negative_paths ;;
        profile_execution) count_key=profile_execution_entries ;;
    esac
    [[ "$(printf '%s\n' "$inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')" \
            == "$(read_value "$count_key")" \
        && "$(printf '%s\n' "$inventory" | sed '/^$/d' | sha256_stream)" \
            == "$(read_value "${name}_sha256")" ]] || {
        printf 'error: R3bp %s profile section drifted\n' "$name" >&2
        exit 1
    }
done

live_profile_sha256=$(sha256_file "$live_profile")
upstream_profile=$(
    awk -F'"' '$1 ~ /^oxide_profile_sha256 = / { print $2; found++ }
        END { if (found != 1) exit 1 }' compat/upstream.toml
)
[[ "$upstream_profile" == "$live_profile_sha256" ]] || {
    echo "error: compat/upstream.toml does not authenticate the current live profile" >&2
    exit 1
}

"$script_dir/test-test262-global-this.sh" --check

for tuple in \
    "universe:$universe_manifest" \
    "activation:$activation_manifest" \
    "deferred:$deferred_manifest"
do
    name=${tuple%%:*}
    manifest=${tuple#*:}
    manifest_paths "$manifest" | LC_ALL=C sort -c
    [[ "$(manifest_paths "$manifest" | wc -l | tr -d '[:space:]')" \
            == "$(read_value "${name}_paths")" \
        && "$(sha256_file "$manifest")" == "$(read_value "${name}_manifest_sha256")" ]] || {
        printf 'error: R3bp %s manifest drifted\n' "$name" >&2
        exit 1
    }
done
diff -u <(manifest_paths "$universe_manifest") \
    <(printf '%s\n%s\n' \
        "$(manifest_paths "$activation_manifest")" \
        "$(manifest_paths "$deferred_manifest")" \
        | sed '/^$/d' | LC_ALL=C sort -u)

[[ "$(sha256_file "$focused_transition")" \
        == "$(read_value focused_transition_sha256)" \
    && "$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' \
        "$focused_transition" | sha256_stream)" \
        == "$(read_value focused_transition_data_sha256)" \
    && "$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { n++ }
        END { print n + 0 }' "$focused_transition")" \
        == "$(read_value focused_transition_rows)" \
    && "$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2
    }' "$focused_transition" | LC_ALL=C sort | sha256_stream)" \
        == "$(read_value focused_transition_keys_sha256)" ]] || {
    echo "error: R3bp focused R3bo transition receipt drifted" >&2
    exit 1
}
if ! awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") {
        rows++
        paths[$1]=1
        if (NF != 14 ||
            $5 != "normal" || $6 != "" ||
            $7 != "unsupported-feature" ||
            $8 != "selection" ||
            $9 != "EngineCapability" ||
            $10 != "quickjs-oxide does not declare Test262 feature support: globalThis" ||
            $11 != "pass" || $12 != "normal" || $13 != "" || $14 != "") {
            exit 2
        }
    }
    END {
        for (path in paths) path_count++
        if (rows != 150 || path_count != 135) exit 3
    }
' "$focused_transition"; then
    echo "error: R3bp focused transition is not the exact 150-row admission" >&2
    exit 1
fi
diff -u \
    <(manifest_paths "$activation_manifest") \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$focused_transition" | LC_ALL=C sort -u)

tag_receipt_fields=(
    tag_transition_data_sha256 tag_transition_sha256
    tag_before_nonpass_sha256 tag_before_tsv_sha256 tag_before_jsonl_sha256
    tag_candidate_nonpass_sha256 tag_candidate_tsv_sha256
    tag_candidate_jsonl_sha256
)
tag_pending=$(pending_keys "${tag_receipt_fields[@]}")
tag_pending_count=$(
    printf '%s\n' "$tag_pending" | sed '/^$/d' | wc -l | tr -d '[:space:]'
)
if [[ "$tag_pending_count" != 0 \
    && "$tag_pending_count" != "${#tag_receipt_fields[@]}" ]]; then
    echo "error: R3bp tag receipt is only partially PENDING" >&2
    exit 1
fi

full_receipt_fields=(
    candidate_full_tsv_sha256 candidate_full_jsonl_sha256
    full_activation_before_tsv_data_sha256
    full_activation_before_jsonl_data_sha256
    full_activation_candidate_tsv_data_sha256
    full_activation_candidate_jsonl_data_sha256
    full_deferred_tsv_data_sha256 full_deferred_jsonl_data_sha256
    full_non_tag_tsv_data_sha256 full_non_tag_jsonl_data_sha256
    full_unchanged_tsv_data_sha256 full_unchanged_jsonl_data_sha256
)
full_pending=$(pending_keys "${full_receipt_fields[@]}")
full_pending_count=$(
    printf '%s\n' "$full_pending" | sed '/^$/d' | wc -l | tr -d '[:space:]'
)
if [[ "$full_pending_count" != 0 \
    && "$full_pending_count" != "${#full_receipt_fields[@]}" ]]; then
    echo "error: R3bp whole-corpus receipt is only partially PENDING" >&2
    exit 1
fi

if [[ "$mode" == check ]]; then
    printf 'R3bp globalThis inputs verified: %s/%s exhaustive, %s/%s activation, %s/%s deferred; tag/full receipts %s/%s\n' \
        "$(read_value universe_paths)" "$(read_value universe_variants)" \
        "$(read_value activation_paths)" "$(read_value activation_variants)" \
        "$(read_value deferred_paths)" "$(read_value deferred_variants)" \
        "$([[ "$tag_pending_count" == 0 ]] && printf frozen || printf PENDING)" \
        "$([[ "$full_pending_count" == 0 ]] && printf frozen || printf PENDING)"
    exit 0
fi
if [[ "$tag_pending_count" != 0 && "$mode" != bless ]]; then
    printf 'error: R3bp tag baseline requires --bless after an exact join: %s\n' \
        "$(tr '\n' ' ' <<<"$tag_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ "$tag_pending_count" == 0 && "$mode" == bless ]]; then
    mode=tag
fi
if [[ "$full_pending_count" != 0 \
    && "$mode" != tag \
    && "$mode" != bless \
    && "$mode" != bless-full ]]; then
    printf 'error: R3bp full baseline requires --bless-full after an exact join: %s\n' \
        "$(tr '\n' ' ' <<<"$full_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ "$full_pending_count" == 0 && "$mode" == bless-full ]]; then
    mode=full
fi

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

run_tag() {
    local profile=$1 report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --manifest "$universe_manifest" \
        --report "$report" \
        --mode "$(read_value mode)" \
        --workers "$workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

for stale in \
    "$before_tag_report" "$before_tag_json_report" \
    "$candidate_tag_report" "$candidate_tag_json_report"
do
    [[ ! -e "$stale" ]] || unlink "$stale"
done
before_tag_output=$(run_tag "$parent_profile" "$before_tag_report")
printf '%s\n' "$before_tag_output"
candidate_tag_output=$(run_tag "$candidate_profile" "$candidate_tag_report")
printf '%s\n' "$candidate_tag_output"
verify_report \
    "$before_tag_report" "$before_tag_json_report" \
    "$(read_value parent_profile_sha256)" "$(read_value tag_rows)"
verify_report \
    "$candidate_tag_report" "$candidate_tag_json_report" \
    "$(read_value candidate_profile_sha256)" "$(read_value tag_rows)"
before_tag_keys=$(report_keys "$before_tag_report")
candidate_tag_keys=$(report_keys "$candidate_tag_report")
diff -u <(printf '%s\n' "$before_tag_keys") <(printf '%s\n' "$candidate_tag_keys")
[[ "$(printf '%s\n' "$before_tag_keys" | sha256_stream)" \
        == "$(read_value universe_keys_sha256)" \
    && "$(execution_runnable "$before_tag_output")" == 0 \
    && "$(execution_runnable "$candidate_tag_output")" \
        == "$(read_value tag_activation_rows)" \
    && "$(report_summary "$before_tag_report")" == "$(read_value tag_before_summary)" \
    && "$(report_summary "$candidate_tag_report")" \
        == "$(read_value tag_candidate_summary)" ]] || {
    echo "error: R3bp tag keys, runnable counts, or summaries drifted" >&2
    exit 1
}

activation_paths=$(manifest_paths "$activation_manifest")
deferred_paths=$(manifest_paths "$deferred_manifest")
before_tag_activation_rows=$(rows_for_paths "$activation_paths" "$before_tag_report")
candidate_tag_activation_rows=$(
    rows_for_paths "$activation_paths" "$candidate_tag_report"
)
before_tag_deferred_rows=$(rows_for_paths "$deferred_paths" "$before_tag_report")
candidate_tag_deferred_rows=$(
    rows_for_paths "$deferred_paths" "$candidate_tag_report"
)
diff -u <(focused_before_rows) <(printf '%s\n' "$before_tag_activation_rows")
diff -u <(focused_candidate_rows) <(printf '%s\n' "$candidate_tag_activation_rows")
diff -u <(printf '%s\n' "$before_tag_deferred_rows") \
    <(printf '%s\n' "$candidate_tag_deferred_rows")
before_tag_deferred_json=$(
    json_rows_for_paths "$deferred_paths" "$before_tag_json_report"
)
candidate_tag_deferred_json=$(
    json_rows_for_paths "$deferred_paths" "$candidate_tag_json_report"
)
before_tag_activation_json=$(
    json_rows_for_paths "$activation_paths" "$before_tag_json_report"
)
candidate_tag_activation_json=$(
    json_rows_for_paths "$activation_paths" "$candidate_tag_json_report"
)
diff -u <(printf '%s\n' "$before_tag_deferred_json") \
    <(printf '%s\n' "$candidate_tag_deferred_json")
verify_deferred_vector tag "$before_tag_deferred_rows" "$before_tag_deferred_json"
if ! printf '%s\n' "$before_tag_activation_json" | awk '
    !/"outcome":"unsupported-feature","actual_phase":"selection","actual_type":"EngineCapability","detail":"quickjs-oxide does not declare Test262 feature support: globalThis"\}$/ {
        exit 1
    }
'; then
    echo "error: R3bp parent tag activation JSONL drifted" >&2
    exit 1
fi
if ! printf '%s\n' "$candidate_tag_activation_json" | awk '
    !/"outcome":"pass","actual_phase":"normal","actual_type":"","detail":""\}$/ {
        exit 1
    }
'; then
    echo "error: R3bp candidate tag activation JSONL drifted" >&2
    exit 1
fi
diff -u \
    <(printf '%s\n' "$before_tag_activation_json" | sed \
        's/"outcome":"unsupported-feature","actual_phase":"selection","actual_type":"EngineCapability","detail":"quickjs-oxide does not declare Test262 feature support: globalThis"}/"outcome":"pass","actual_phase":"normal","actual_type":"","detail":""}/') \
    <(printf '%s\n' "$candidate_tag_activation_json")

tag_transition_tmp=$(mktemp "$tag_transition.XXXXXX")
cleanup_tag_transition() {
    if [[ -n ${tag_transition_tmp:-} && -e "$tag_transition_tmp" ]]; then
        unlink "$tag_transition_tmp"
    fi
}
trap cleanup_tag_transition EXIT
{
    printf '# R3bp exhaustive globalThis global admission transition.\n'
    printf '# before_oxide_profile_sha256=%s\n' "$(read_value parent_profile_sha256)"
    printf '# after_oxide_profile_sha256=%s\n' "$(read_value candidate_profile_sha256)"
    printf '# manifest_sha256=%s\n' "$(read_value universe_manifest_sha256)"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' -v OFS='\t' '
        NR == FNR {
            if (!/^#/ && !($1 == "path" && $2 == "variant")) {
                key=$1 SUBSEP $2
                if (key in before) exit 2
                for (i=1; i<=10; i++) field[key, i]=$i
                before[key]=1
                before_count++
            }
            next
        }
        !/^#/ && !($1 == "path" && $2 == "variant") {
            key=$1 SUBSEP $2
            if (!(key in before) || key in after) exit 3
            for (i=1; i<=6; i++) if ($i != field[key, i]) exit 4
            print $1,$2,$3,$4,$5,$6,
                field[key,7],field[key,8],field[key,9],field[key,10],
                $7,$8,$9,$10
            after[key]=1
            after_count++
        }
        END {
            if (before_count != after_count) exit 5
            for (key in before) if (!(key in after)) exit 6
        }
    ' "$before_tag_report" "$candidate_tag_report"
} >"$tag_transition_tmp"
tag_transition_rows=$(
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { n++ }
        END { print n + 0 }' "$tag_transition_tmp"
)
tag_transition_keys_sha=$(
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2
    }' "$tag_transition_tmp" | LC_ALL=C sort | sha256_stream
)
tag_transition_changes=$(
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") {
        different=0
        for (i=7; i<=10; i++) if ($i != $(i+4)) different=1
        if (different) changed++; else unchanged++
    }
    END { print changed + 0, unchanged + 0 }' "$tag_transition_tmp"
)
read -r tag_changed tag_unchanged <<<"$tag_transition_changes"
[[ "$tag_transition_rows" == "$(read_value tag_rows)" \
    && "$tag_transition_keys_sha" == "$(read_value universe_keys_sha256)" \
    && "$tag_changed" == "$(read_value tag_activation_rows)" \
    && "$tag_unchanged" == "$(read_value tag_deferred_rows)" ]] || {
    echo "error: R3bp tag transition partition drifted" >&2
    exit 1
}
tag_transition_data_sha=$(
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' \
        "$tag_transition_tmp" | sha256_stream
)
tag_transition_sha=$(sha256_file "$tag_transition_tmp")
tag_before_nonpass=$(report_nonpass_sha256 "$before_tag_report")
tag_before_tsv=$(sha256_file "$before_tag_report")
tag_before_jsonl=$(sha256_file "$before_tag_json_report")
tag_candidate_nonpass=$(report_nonpass_sha256 "$candidate_tag_report")
tag_candidate_tsv=$(sha256_file "$candidate_tag_report")
tag_candidate_jsonl=$(sha256_file "$candidate_tag_json_report")

if [[ "$mode" == bless ]]; then
    chmod 644 "$tag_transition_tmp"
    mv -- "$tag_transition_tmp" "$tag_transition"
    tag_transition_tmp=
    update_values "$baseline" \
        "tag_transition_data_sha256=$tag_transition_data_sha" \
        "tag_transition_sha256=$tag_transition_sha" \
        "tag_before_nonpass_sha256=$tag_before_nonpass" \
        "tag_before_tsv_sha256=$tag_before_tsv" \
        "tag_before_jsonl_sha256=$tag_before_jsonl" \
        "tag_candidate_nonpass_sha256=$tag_candidate_nonpass" \
        "tag_candidate_tsv_sha256=$tag_candidate_tsv" \
        "tag_candidate_jsonl_sha256=$tag_candidate_jsonl"
    printf 'R3bp tag baseline blessed: %s exact activations; %s module/config rows unchanged\n' \
        "$tag_changed" "$tag_unchanged"
    exit 0
fi

[[ -f "$tag_transition" ]] || {
    echo "error: missing R3bp tag transition receipt" >&2
    exit 1
}
cmp -s "$tag_transition_tmp" "$tag_transition" || {
    echo "error: R3bp checked-in tag transition receipt drifted" >&2
    exit 1
}
unlink "$tag_transition_tmp"
tag_transition_tmp=
for entry in \
    "tag_transition_data_sha256:$tag_transition_data_sha" \
    "tag_transition_sha256:$tag_transition_sha" \
    "tag_before_nonpass_sha256:$tag_before_nonpass" \
    "tag_before_tsv_sha256:$tag_before_tsv" \
    "tag_before_jsonl_sha256:$tag_before_jsonl" \
    "tag_candidate_nonpass_sha256:$tag_candidate_nonpass" \
    "tag_candidate_tsv_sha256:$tag_candidate_tsv" \
    "tag_candidate_jsonl_sha256:$tag_candidate_jsonl"
do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: R3bp tag receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done
if [[ "$mode" == tag ]]; then
    printf 'R3bp globalThis tag transition is exact: %s activations; %s module/config rows unchanged\n' \
        "$tag_changed" "$tag_unchanged"
    exit 0
fi

if [[ "$full_pending_count" != 0 && "$mode" != bless-full ]]; then
    printf 'error: R3bp full baseline requires --bless-full after an exact join: %s\n' \
        "$(tr '\n' ' ' <<<"$full_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ "$mode" == bless-full ]]; then
    [[ "$live_profile_sha256" == "$(read_value candidate_profile_sha256)" ]] || {
        echo "error: R3bp can bless the canonical vector only while its candidate is live" >&2
        exit 1
    }
    verify_canonical_baseline parent
fi

run_full() {
    local profile=$1 report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --all \
        --report "$report" \
        --mode "$(read_value mode)" \
        --workers "$full_workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

for stale in \
    "$before_full_report" "$before_full_json_report" \
    "$candidate_full_report" "$candidate_full_json_report"
do
    [[ ! -e "$stale" ]] || unlink "$stale"
done
before_output=$(run_full "$parent_profile" "$before_full_report")
printf '%s\n' "$before_output"
verify_report \
    "$before_full_report" "$before_full_json_report" \
    "$(read_value parent_profile_sha256)" "$(read_value full_variants)"
before_keys=$(report_keys "$before_full_report")
full_keys_sha=$(printf '%s\n' "$before_keys" | sha256_stream)
[[ "$full_keys_sha" == "$(read_value full_keys_sha256)" ]] || {
    echo "error: R3bp complete Test262 key inventory drifted" >&2
    exit 1
}
before_runnable=$(execution_runnable "$before_output")
before_passes=$(report_outcome_count "$before_full_report" pass)
before_unsupported=$(report_outcome_count "$before_full_report" unsupported-feature)
before_summary=$(report_summary "$before_full_report")
before_tsv=$(sha256_file "$before_full_report")
before_jsonl=$(sha256_file "$before_full_json_report")
[[ "$before_runnable" == "$(read_value before_full_runnable)" \
    && "$before_passes" == "$(read_value before_full_passes)" \
    && "$before_unsupported" == "$(read_value before_full_unsupported)" \
    && "$before_summary" == "$(read_value before_full_summary)" \
    && "$before_tsv" == "$(read_value before_full_tsv_sha256)" \
    && "$before_jsonl" == "$(read_value before_full_jsonl_sha256)" ]] || {
    echo "error: R3bp authoritative historical parent full vector drifted" >&2
    exit 1
}

candidate_output=$(run_full "$candidate_profile" "$candidate_full_report")
printf '%s\n' "$candidate_output"
verify_report \
    "$candidate_full_report" "$candidate_full_json_report" \
    "$(read_value candidate_profile_sha256)" "$(read_value full_variants)"
candidate_keys=$(report_keys "$candidate_full_report")
diff -u <(printf '%s\n' "$before_keys") <(printf '%s\n' "$candidate_keys")
candidate_runnable=$(execution_runnable "$candidate_output")
candidate_passes=$(report_outcome_count "$candidate_full_report" pass)
candidate_unsupported=$(report_outcome_count "$candidate_full_report" unsupported-feature)
candidate_summary=$(report_summary "$candidate_full_report")
candidate_tsv=$(sha256_file "$candidate_full_report")
candidate_jsonl=$(sha256_file "$candidate_full_json_report")
[[ "$candidate_runnable" == "$(read_value candidate_full_runnable)" \
    && "$candidate_passes" == "$(read_value candidate_full_passes)" \
    && "$candidate_unsupported" == "$(read_value candidate_full_unsupported)" \
    && "$candidate_summary" == "$(read_value candidate_full_summary)" ]] || {
    echo "error: R3bp candidate full summary or admission counts drifted" >&2
    exit 1
}

activation_paths=$(manifest_paths "$activation_manifest")
deferred_paths=$(manifest_paths "$deferred_manifest")
universe_paths=$(manifest_paths "$universe_manifest")
before_activation_rows=$(rows_for_paths "$activation_paths" "$before_full_report")
candidate_activation_rows=$(rows_for_paths "$activation_paths" "$candidate_full_report")
before_deferred_rows=$(rows_for_paths "$deferred_paths" "$before_full_report")
candidate_deferred_rows=$(rows_for_paths "$deferred_paths" "$candidate_full_report")
before_universe_rows=$(rows_for_paths "$universe_paths" "$before_full_report")
candidate_universe_rows=$(rows_for_paths "$universe_paths" "$candidate_full_report")
before_non_tag_rows=$(rows_without_paths "$universe_paths" "$before_full_report")
candidate_non_tag_rows=$(rows_without_paths "$universe_paths" "$candidate_full_report")
before_unchanged_rows=$(rows_without_paths "$activation_paths" "$before_full_report")
candidate_unchanged_rows=$(rows_without_paths "$activation_paths" "$candidate_full_report")

[[ "$(printf '%s\n' "$before_activation_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_activation_rows)" \
    && "$(printf '%s\n' "$before_deferred_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_deferred_rows)" \
    && "$(printf '%s\n' "$before_non_tag_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_non_tag_rows)" \
    && "$(printf '%s\n' "$before_unchanged_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_unchanged_rows)" ]] || {
    echo "error: R3bp full activation/deferred/non-tag partition drifted" >&2
    exit 1
}
diff -u <(focused_before_rows) <(printf '%s\n' "$before_activation_rows")
diff -u <(focused_candidate_rows) <(printf '%s\n' "$candidate_activation_rows")
diff -u <(report_rows "$before_tag_report") <(printf '%s\n' "$before_universe_rows")
diff -u <(report_rows "$candidate_tag_report") \
    <(printf '%s\n' "$candidate_universe_rows")
diff -u <(printf '%s\n' "$before_deferred_rows") \
    <(printf '%s\n' "$candidate_deferred_rows")
diff -u <(printf '%s\n' "$before_non_tag_rows") \
    <(printf '%s\n' "$candidate_non_tag_rows")
diff -u <(printf '%s\n' "$before_unchanged_rows") \
    <(printf '%s\n' "$candidate_unchanged_rows")

join_counts=$(
    awk -F'\t' '
        NR == FNR {
            if (/^#/ || ($1 == "path" && $2 == "variant")) next
            key=$1 SUBSEP $2
            if (key in before) exit 2
            before[key]=$7 SUBSEP $8 SUBSEP $9 SUBSEP $10
            metadata[key]=$3 SUBSEP $4 SUBSEP $5 SUBSEP $6
            before_count++
            next
        }
        /^#/ || ($1 == "path" && $2 == "variant") { next }
        {
            key=$1 SUBSEP $2
            if (!(key in before) || key in after) exit 3
            if (metadata[key] != $3 SUBSEP $4 SUBSEP $5 SUBSEP $6) exit 4
            split(before[key], old, SUBSEP)
            if (old[1] == "pass" && $7 != "pass") regressions++
            current=$7 SUBSEP $8 SUBSEP $9 SUBSEP $10
            if (before[key] != current) {
                if (old[1] == $7) {
                    detail_only++
                } else if (old[1] == "unsupported-feature" &&
                    old[2] == "selection" &&
                    old[3] == "EngineCapability" &&
                    old[4] == "quickjs-oxide does not declare Test262 feature support: globalThis" &&
                    $7 == "pass" && $8 == "normal" && $9 == "" && $10 == "") {
                    changes++
                } else {
                    exit 5
                }
            }
            after[key]=1
            after_count++
        }
        END {
            if (before_count != after_count) exit 6
            for (key in before) if (!(key in after)) exit 7
            print changes + 0, detail_only + 0, regressions + 0
        }
    ' "$before_full_report" "$candidate_full_report"
) || {
    echo "error: R3bp complete before/after keyed join failed" >&2
    exit 1
}
read -r changed_rows detail_only_rows previous_pass_regressions <<<"$join_counts"
[[ "$changed_rows" == "$(read_value full_changed_rows)" \
    && "$detail_only_rows" == "$(read_value full_detail_only_rows)" \
    && "$previous_pass_regressions" == "$(read_value previous_pass_regressions)" ]] || {
    echo "error: R3bp join is not exactly 150 transitions, zero detail-only changes, and zero pass regressions" >&2
    exit 1
}

before_activation_json=$(json_rows_for_paths "$activation_paths" "$before_full_json_report")
candidate_activation_json=$(
    json_rows_for_paths "$activation_paths" "$candidate_full_json_report"
)
before_deferred_json=$(json_rows_for_paths "$deferred_paths" "$before_full_json_report")
candidate_deferred_json=$(
    json_rows_for_paths "$deferred_paths" "$candidate_full_json_report"
)
before_universe_json=$(json_rows_for_paths "$universe_paths" "$before_full_json_report")
candidate_universe_json=$(
    json_rows_for_paths "$universe_paths" "$candidate_full_json_report"
)
before_non_tag_json=$(
    json_rows_without_paths "$universe_paths" "$before_full_json_report"
)
candidate_non_tag_json=$(
    json_rows_without_paths "$universe_paths" "$candidate_full_json_report"
)
before_unchanged_json=$(
    json_rows_without_paths "$activation_paths" "$before_full_json_report"
)
candidate_unchanged_json=$(
    json_rows_without_paths "$activation_paths" "$candidate_full_json_report"
)
diff -u <(printf '%s\n' "$before_deferred_json") \
    <(printf '%s\n' "$candidate_deferred_json")
verify_deferred_vector full "$before_deferred_rows" "$before_deferred_json"
diff -u \
    <(json_rows_for_paths "$universe_paths" "$before_tag_json_report") \
    <(printf '%s\n' "$before_universe_json")
diff -u \
    <(json_rows_for_paths "$universe_paths" "$candidate_tag_json_report") \
    <(printf '%s\n' "$candidate_universe_json")
diff -u <(printf '%s\n' "$before_non_tag_json") \
    <(printf '%s\n' "$candidate_non_tag_json")
diff -u <(printf '%s\n' "$before_unchanged_json") \
    <(printf '%s\n' "$candidate_unchanged_json")
if ! printf '%s\n' "$before_activation_json" | awk '
    !/"outcome":"unsupported-feature","actual_phase":"selection","actual_type":"EngineCapability","detail":"quickjs-oxide does not declare Test262 feature support: globalThis"\}$/ {
        exit 1
    }
'; then
    echo "error: R3bp parent activation JSONL is not the exact rejection vector" >&2
    exit 1
fi
if ! printf '%s\n' "$candidate_activation_json" | awk '
    !/"outcome":"pass","actual_phase":"normal","actual_type":"","detail":""\}$/ {
        exit 1
    }
'; then
    echo "error: R3bp candidate activation JSONL is not the exact pass vector" >&2
    exit 1
fi
diff -u \
    <(printf '%s\n' "$before_activation_json" | sed \
        's/"outcome":"unsupported-feature","actual_phase":"selection","actual_type":"EngineCapability","detail":"quickjs-oxide does not declare Test262 feature support: globalThis"}/"outcome":"pass","actual_phase":"normal","actual_type":"","detail":""}/') \
    <(printf '%s\n' "$candidate_activation_json")

activation_before_tsv_sha=$(printf '%s\n' "$before_activation_rows" | sha256_stream)
activation_before_json_sha=$(printf '%s\n' "$before_activation_json" | sha256_stream)
activation_candidate_tsv_sha=$(
    printf '%s\n' "$candidate_activation_rows" | sha256_stream
)
activation_candidate_json_sha=$(
    printf '%s\n' "$candidate_activation_json" | sha256_stream
)
deferred_tsv_sha=$(printf '%s\n' "$before_deferred_rows" | sha256_stream)
deferred_json_sha=$(printf '%s\n' "$before_deferred_json" | sha256_stream)
non_tag_tsv_sha=$(printf '%s\n' "$before_non_tag_rows" | sha256_stream)
non_tag_json_sha=$(printf '%s\n' "$before_non_tag_json" | sha256_stream)
unchanged_tsv_sha=$(printf '%s\n' "$before_unchanged_rows" | sha256_stream)
unchanged_json_sha=$(printf '%s\n' "$before_unchanged_json" | sha256_stream)

if [[ "$mode" == bless-full ]]; then
    update_values "$baseline" \
        "candidate_full_tsv_sha256=$candidate_tsv" \
        "candidate_full_jsonl_sha256=$candidate_jsonl" \
        "full_activation_before_tsv_data_sha256=$activation_before_tsv_sha" \
        "full_activation_before_jsonl_data_sha256=$activation_before_json_sha" \
        "full_activation_candidate_tsv_data_sha256=$activation_candidate_tsv_sha" \
        "full_activation_candidate_jsonl_data_sha256=$activation_candidate_json_sha" \
        "full_deferred_tsv_data_sha256=$deferred_tsv_sha" \
        "full_deferred_jsonl_data_sha256=$deferred_json_sha" \
        "full_non_tag_tsv_data_sha256=$non_tag_tsv_sha" \
        "full_non_tag_jsonl_data_sha256=$non_tag_json_sha" \
        "full_unchanged_tsv_data_sha256=$unchanged_tsv_sha" \
        "full_unchanged_jsonl_data_sha256=$unchanged_json_sha"
    update_values "$canonical_baseline" \
        "runnable=$candidate_runnable" \
        "passes=$candidate_passes" \
        "tsv_sha256=$candidate_tsv" \
        "jsonl_sha256=$candidate_jsonl" \
        "summary=$candidate_summary"
    printf 'R3bp full transition blessed: %s exact unsupported-feature -> pass rows; %s unchanged; zero previous-pass regressions\n' \
        "$changed_rows" "$(read_value full_unchanged_rows)"
    printf 'Run ./scripts/test-test262-full.sh for the independent canonical repeat.\n'
    exit 0
fi

for entry in \
    "candidate_full_tsv_sha256:$candidate_tsv" \
    "candidate_full_jsonl_sha256:$candidate_jsonl" \
    "full_activation_before_tsv_data_sha256:$activation_before_tsv_sha" \
    "full_activation_before_jsonl_data_sha256:$activation_before_json_sha" \
    "full_activation_candidate_tsv_data_sha256:$activation_candidate_tsv_sha" \
    "full_activation_candidate_jsonl_data_sha256:$activation_candidate_json_sha" \
    "full_deferred_tsv_data_sha256:$deferred_tsv_sha" \
    "full_deferred_jsonl_data_sha256:$deferred_json_sha" \
    "full_non_tag_tsv_data_sha256:$non_tag_tsv_sha" \
    "full_non_tag_jsonl_data_sha256:$non_tag_json_sha" \
    "full_unchanged_tsv_data_sha256:$unchanged_tsv_sha" \
    "full_unchanged_jsonl_data_sha256:$unchanged_json_sha"
do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: R3bp full receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done

printf 'R3bp full transition is exact: %s unsupported-feature -> pass rows; %s unchanged; zero previous-pass regressions\n' \
    "$changed_rows" "$(read_value full_unchanged_rows)"
