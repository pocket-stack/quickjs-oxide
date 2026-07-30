#!/usr/bin/env bash
# Reproduce the R3bn global iterator-helpers admission and transition receipts.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-iterator-helpers-global-baseline.txt
parent_profile=tests/test262-iterator-helpers-global-parent.conf
candidate_profile=tests/test262-iterator-helpers-global-candidate.conf
live_profile=compat/test262-oxide.conf
manifest=tests/test262-iterator-helpers-global.txt
activation_manifest=tests/test262-iterator-helpers-global-activation.txt
reason_only_manifest=tests/test262-iterator-helpers-global-reason-only.txt
host_config_manifest=tests/test262-iterator-helpers-global-host-config.txt
transition_receipt=tests/test262-iterator-helpers-global-transitions.tsv
before_report=target/test262-iterator-helpers-global-before.tsv
before_json_report=target/test262-iterator-helpers-global-before.jsonl
candidate_report=target/test262-iterator-helpers-global-candidate.tsv
candidate_json_report=target/test262-iterator-helpers-global-candidate.jsonl
before_full_report=target/test262-iterator-helpers-global-before-full.tsv
before_full_json_report=target/test262-iterator-helpers-global-before-full.jsonl
candidate_full_report=target/test262-iterator-helpers-global-candidate-full.tsv
candidate_full_json_report=target/test262-iterator-helpers-global-candidate-full.jsonl
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}

usage() {
    printf 'usage: %s [--check|--bless|--full|--bless-full]\n' "${0##*/}"
    printf '  --check       verify frozen profiles, manifests, and set algebra only\n'
    printf '  --bless       bless tag reports and the 1,134-row transition receipt\n'
    printf '  --full        reproduce and verify the parent/candidate whole-suite join\n'
    printf '  --bless-full  bless whole-suite receipts after an exact no-regression join\n'
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

read_value() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$baseline"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    [[ "$actual" == "$expected" ]] || {
        printf 'error: R3bn baseline identity drifted for %s: %s != %s\n' \
            "$key" "$actual" "$expected" >&2
        exit 1
    }
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
    local input=${1:-$manifest}
    awk 'NF && $1 !~ /^#/ { print }' "$input"
}

profile_section() {
    local profile=$1 section=$2
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
            sub("^[^:]+:[[:space:]]*\\[", "")
            sub(/\][[:space:]]*$/, "")
            count=split($0, values, /,[[:space:]]*/)
            for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            exit
        }
        $0 ~ ("^" key ":[[:space:]]*$") { inside=1; next }
        inside && /^[[:space:]]+-[[:space:]]+/ {
            sub(/^[[:space:]]+-[[:space:]]+/, "")
            print
            next
        }
        inside { exit }
    '
}

program_body() {
    local test_path=$1
    sed '/^\/\*---$/,/^---\*\/$/d' "$suite/$test_path"
}

variant_keys() {
    local test_path
    while IFS= read -r test_path; do
        [[ -z "$test_path" ]] && continue
        printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path"
    done | LC_ALL=C sort
}

verify_inventory() {
    local name=$1 inventory=$2 expected_count expected_hash actual_count actual_hash
    expected_count=$(read_value "${name}_paths")
    expected_hash=$(read_value "${name}_sha256")
    actual_count=$(printf '%s\n' "$inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$inventory" | sed '/^$/d' | sha256_stream)
    [[ "$actual_count" == "$expected_count" && "$actual_hash" == "$expected_hash" ]] || {
        printf 'error: R3bn %s inventory drifted\n' "$name" >&2
        exit 1
    }
}

verify_key_inventory() {
    local name=$1 inventory=$2 keys expected_count expected_hash actual_count actual_hash
    expected_count=$(read_value "${name}_variants")
    expected_hash=$(read_value "${name}_keys_sha256")
    keys=$(printf '%s\n' "$inventory" | variant_keys)
    actual_count=$(printf '%s\n' "$keys" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$keys" | sed '/^$/d' | sha256_stream)
    [[ "$actual_count" == "$expected_count" && "$actual_hash" == "$expected_hash" ]] || {
        printf 'error: R3bn %s variant-key inventory drifted\n' "$name" >&2
        exit 1
    }
}

read_header() {
    local report=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

report_rows() {
    local report=$1
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' "$report"
}

report_keys() {
    local report=$1
    report_rows "$report" | awk -F'\t' '{ print $1 "\t" $2 }' | LC_ALL=C sort
}

json_report_keys() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3bn JSONL report %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        /^\{"kind":"metadata",/ {
            metadata++
            if (NR != 1) fail("metadata record is not first")
            next
        }
        /^\{"kind":"result",/ {
            results++
            if (!match($0, /"path":"[^"]*"/)) fail("result is missing path")
            path=substr($0, RSTART, RLENGTH)
            sub(/^"path":"/, "", path)
            sub(/"$/, "", path)
            if (!match($0, /"variant":"[^"]*"/)) fail("result is missing variant")
            variant=substr($0, RSTART, RLENGTH)
            sub(/^"variant":"/, "", variant)
            sub(/"$/, "", variant)
            key=path "\t" variant
            if (seen[key]++) fail("duplicate result key")
            print key
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
    local output=$1
    printf '%s\n' "$output" | awk '
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
    local report=$1 pattern=$2
    report_rows "$report" | awk -F'\t' -v pattern="$pattern" '
        $7 ~ pattern { count++ }
        END { print count + 0 }
    '
}

report_nonpass_sha256() {
    local report=$1
    report_rows "$report" | awk -F'\t' '$7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' | sha256_stream
}

report_summary() {
    local report=$1
    tail -n 1 "$report" | sed 's/^# summary //'
}

json_report_summary() {
    local report=$1
    tail -n 1 "$report" | awk '
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
    local expected_profile=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"%s","mode":"%s"}\n' \
        "$(read_value quickjs)" \
        "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" \
        "$expected_profile" \
        "$(read_value schema)" \
        "$(read_value mode)"
}

verify_report_metadata() {
    local report=$1 expected_profile=$2 expected_variants=$3
    [[ "$(read_header "$report" quickjs)" == "$(read_value quickjs)" \
        && "$(read_header "$report" test262)" == "$(read_value test262)" \
        && "$(read_header "$report" test262_patch_sha256)" \
            == "$(read_value test262_patch_sha256)" \
        && "$(read_header "$report" test262_config_sha256)" \
            == "$(read_value test262_config_sha256)" \
        && "$(read_header "$report" test262_metadata_sha256)" \
            == "$(read_value test262_metadata_sha256)" \
        && "$(read_header "$report" oxide_profile_sha256)" == "$expected_profile" \
        && "$(read_header "$report" profile)" == "$(read_value schema)" \
        && "$(read_header "$report" mode)" == "$(read_value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
            == "$expected_variants" ]] || {
        printf 'error: R3bn report metadata drifted: %s\n' "$report" >&2
        exit 1
    }
}

verify_json_report() {
    local json_report=$1 tsv_report=$2 expected_profile=$3 json_keys tsv_keys
    if ! json_keys=$(json_report_keys "$json_report"); then
        printf 'error: R3bn JSONL validation failed: %s\n' "$json_report" >&2
        exit 1
    fi
    if ! tsv_keys=$(report_keys "$tsv_report"); then
        printf 'error: R3bn TSV key extraction failed: %s\n' "$tsv_report" >&2
        exit 1
    fi
    diff -u <(printf '%s\n' "$tsv_keys") <(printf '%s\n' "$json_keys")
    [[ "$(head -n 1 "$json_report")" == "$(expected_json_metadata "$expected_profile")" \
        && "$(json_report_summary "$json_report")" == "$(report_summary "$tsv_report")" ]] || {
        printf 'error: R3bn JSONL metadata or summary drifted: %s\n' "$json_report" >&2
        exit 1
    }
}

update_baseline() {
    local updates_tmp baseline_tmp entry
    updates_tmp=$(mktemp "$baseline.updates.XXXXXX")
    baseline_tmp=$(mktemp "$baseline.XXXXXX")
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
    ' "$updates_tmp" "$baseline" >"$baseline_tmp"
    chmod 644 "$baseline_tmp"
    mv -- "$baseline_tmp" "$baseline"
    rm -f -- "$updates_tmp"
}

pending_keys() {
    local key
    for key in "$@"; do
        [[ "$(read_value "$key")" == "PENDING" ]] && printf '%s\n' "$key"
    done
    return 0
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value parent_profile_sha256 205554c5686ef2ec77420984ce038d321411a11acabefd2c37d9b63b67fcba62
expect_value parent_features 82
expect_value candidate_profile_sha256 8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910
expect_value candidate_features 83
expect_value profile_negative_paths 828
expect_value tagged_paths 567
expect_value tagged_variants 1134
expect_value activation_paths 538
expect_value activation_variants 1076
expect_value reason_only_paths 13
expect_value reason_only_variants 26
expect_value host_config_paths 16
expect_value host_config_variants 32
expect_value transition_rows 1134
expect_value transition_activation_rows 1076
expect_value transition_reason_only_rows 26
expect_value transition_host_config_rows 32
expect_value full_variants 102037
expect_value full_non_iterator_rows 100903
expect_value before_full_runnable 57045
expect_value before_full_passes 56526
expect_value candidate_full_runnable 58121
expect_value candidate_full_passes 57602

for required in \
    "$parent_profile" "$candidate_profile" "$live_profile" "$manifest" \
    "$activation_manifest" "$reason_only_manifest" "$host_config_manifest"
do
    [[ -f "$required" ]] || { printf 'error: missing R3bn asset: %s\n' "$required" >&2; exit 1; }
done

tagged_inventory=$(
    git -C "$suite" grep -l -F 'iterator-helpers' -- 'test/**/*.js' \
        | while IFS= read -r test_path; do
            if metadata_list "$test_path" features | grep -Fxq 'iterator-helpers'; then
                printf '%s\n' "$test_path"
            fi
        done \
        | LC_ALL=C sort
)
verify_inventory tagged "$tagged_inventory"
verify_key_inventory tagged "$tagged_inventory"
diff -u <(printf '%s\n' "$tagged_inventory") <(manifest_paths "$manifest")
manifest_paths "$manifest" | LC_ALL=C sort -c
[[ "$(sha256_file "$manifest")" == "$(read_value tagged_manifest_sha256)" ]] || {
    echo "error: R3bn exhaustive manifest file drifted" >&2
    exit 1
}

reason_only_inventory=$(
    while IFS= read -r test_path; do
        if metadata_list "$test_path" features | grep -Fxq 'globalThis'; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory" | LC_ALL=C sort
)
create_realm_inventory=$(
    while IFS= read -r test_path; do
        if grep -Eq '[$]262[.]createRealm([^[:alnum:]_$]|$)' \
            < <(program_body "$test_path"); then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
is_html_dda_inventory=$(
    while IFS= read -r test_path; do
        if grep -Eq '[$]262[.]IsHTMLDDA([^[:alnum:]_$]|$)' \
            < <(program_body "$test_path"); then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
config_exclusions=$(
    awk '
        $0 == "[exclude]" { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$source_dir/test262.conf"
)
config_excluded_inventory=$(
    while IFS= read -r test_path; do
        if grep -Fxq "test262/$test_path" <<<"$config_exclusions"; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
host_config_inventory=$(
    printf '%s\n%s\n%s\n' \
        "$create_realm_inventory" "$is_html_dda_inventory" "$config_excluded_inventory" \
        | sed '/^$/d' \
        | LC_ALL=C sort -u
)
blocked_inventory=$(
    printf '%s\n%s\n' "$reason_only_inventory" "$host_config_inventory" \
        | sed '/^$/d' \
        | LC_ALL=C sort -u
)
activation_inventory=$(
    comm -23 \
        <(printf '%s\n' "$tagged_inventory") \
        <(printf '%s\n' "$blocked_inventory")
)

verify_inventory activation "$activation_inventory"
verify_key_inventory activation "$activation_inventory"
verify_inventory reason_only "$reason_only_inventory"
verify_key_inventory reason_only "$reason_only_inventory"
verify_inventory host_config "$host_config_inventory"
verify_key_inventory host_config "$host_config_inventory"
verify_inventory create_realm "$create_realm_inventory"
verify_inventory is_html_dda "$is_html_dda_inventory"
verify_inventory config_excluded "$config_excluded_inventory"
diff -u <(printf '%s\n' "$activation_inventory") <(manifest_paths "$activation_manifest")
diff -u <(printf '%s\n' "$reason_only_inventory") <(manifest_paths "$reason_only_manifest")
diff -u <(printf '%s\n' "$host_config_inventory") <(manifest_paths "$host_config_manifest")
for ledger in "$activation_manifest" "$reason_only_manifest" "$host_config_manifest"; do
    manifest_paths "$ledger" | LC_ALL=C sort -c
done
[[ "$(sha256_file "$activation_manifest")" == "$(read_value activation_manifest_sha256)" \
    && "$(sha256_file "$reason_only_manifest")" \
        == "$(read_value reason_only_manifest_sha256)" \
    && "$(sha256_file "$host_config_manifest")" \
        == "$(read_value host_config_manifest_sha256)" ]] || {
    echo "error: R3bn partition ledger file drifted" >&2
    exit 1
}
[[ -z "$(comm -12 \
    <(printf '%s\n' "$reason_only_inventory") \
    <(printf '%s\n' "$host_config_inventory"))" ]] || {
    echo "error: R3bn reason-only and host/config ledgers overlap" >&2
    exit 1
}
diff -u \
    <(printf '%s\n' "$tagged_inventory") \
    <(printf '%s\n%s\n%s\n' \
        "$activation_inventory" "$reason_only_inventory" "$host_config_inventory" \
        | LC_ALL=C sort -u)

metadata_feature_inventory=
metadata_include_inventory=
negative_inventory=
while IFS= read -r test_path; do
    metadata=$(metadata_block "$test_path")
    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "" | "flags: []") ;;
        *)
            printf 'error: R3bn path lost its sloppy+strict contract: %s: %s\n' \
                "$test_path" "$flag_line" >&2
            exit 1
            ;;
    esac
    if grep -Fq 'negative:' <<<"$metadata"; then
        negative_inventory+=$'\n'"$test_path"
    fi
    features=$(metadata_list "$test_path" features)
    grep -Fxq 'iterator-helpers' <<<"$features" || {
        printf 'error: R3bn path lost iterator-helpers metadata: %s\n' "$test_path" >&2
        exit 1
    }
    metadata_feature_inventory+=$'\n'"$features"
    metadata_include_inventory+=$'\n'"$(metadata_list "$test_path" includes)"
done <<<"$tagged_inventory"
metadata_feature_inventory=$(
    printf '%s\n' "$metadata_feature_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
metadata_include_inventory=$(
    printf '%s\n' "$metadata_include_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
negative_inventory=$(
    printf '%s\n' "$negative_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
[[ "$(printf '%s\n' "$metadata_feature_inventory" | wc -l | tr -d '[:space:]')" \
        == "$(read_value metadata_features)" \
    && "$(printf '%s\n' "$metadata_feature_inventory" | sha256_stream)" \
        == "$(read_value metadata_features_sha256)" \
    && "$(printf '%s\n' "$metadata_include_inventory" | wc -l | tr -d '[:space:]')" \
        == "$(read_value metadata_includes)" \
    && "$(printf '%s\n' "$metadata_include_inventory" | sha256_stream)" \
        == "$(read_value metadata_includes_sha256)" \
    && "$(printf '%s\n' "$negative_inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')" \
        == "$(read_value negative_paths)" ]] || {
    echo "error: R3bn metadata inventory drifted" >&2
    exit 1
}

parent_features=$(profile_section "$parent_profile" features | LC_ALL=C sort)
candidate_features=$(profile_section "$candidate_profile" features | LC_ALL=C sort)
live_features=$(profile_section "$live_profile" features | LC_ALL=C sort)
parent_negatives=$(profile_section "$parent_profile" audited-negative-tests)
candidate_negatives=$(profile_section "$candidate_profile" audited-negative-tests)
live_negatives=$(profile_section "$live_profile" audited-negative-tests)
parent_execution=$(profile_section "$parent_profile" execution)
candidate_execution=$(profile_section "$candidate_profile" execution)
live_execution=$(profile_section "$live_profile" execution)

[[ "$(sha256_file "$parent_profile")" == "$(read_value parent_profile_sha256)" \
    && "$(sha256_file "$candidate_profile")" == "$(read_value candidate_profile_sha256)" \
    && "$(printf '%s\n' "$parent_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value parent_features)" \
    && "$(printf '%s\n' "$parent_features" | sha256_stream)" \
        == "$(read_value parent_features_sha256)" \
    && "$(printf '%s\n' "$candidate_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value candidate_features)" \
    && "$(printf '%s\n' "$candidate_features" | sha256_stream)" \
        == "$(read_value candidate_features_sha256)" \
    && "$(printf '%s\n' "$candidate_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value profile_negative_paths)" \
    && "$(printf '%s\n' "$candidate_negatives" | sha256_stream)" \
        == "$(read_value profile_negative_sha256)" \
    && "$(printf '%s\n' "$candidate_execution" | wc -l | tr -d '[:space:]')" \
        == "$(read_value profile_execution_entries)" \
    && "$(printf '%s\n' "$candidate_execution" | sha256_stream)" \
        == "$(read_value profile_execution_sha256)" ]] || {
    echo "error: R3bn frozen profile identity drifted" >&2
    exit 1
}
diff -u <(printf '%s\n' "$parent_negatives") <(printf '%s\n' "$candidate_negatives")
diff -u <(printf '%s\n' "$parent_execution") <(printf '%s\n' "$candidate_execution")
diff -u \
    <(printf '%s\n' iterator-helpers) \
    <(comm -13 \
        <(printf '%s\n' "$parent_features") \
        <(printf '%s\n' "$candidate_features"))
[[ -z "$(comm -23 \
    <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))" ]] || {
    echo "error: R3bn candidate removed a frozen parent feature" >&2
    exit 1
}
grep -Fxq 'iterator-helpers' <<<"$candidate_features"
! grep -Fxq 'globalThis' <<<"$candidate_features"

# This is deliberately subset-based: later global admissions may grow the live
# profile without invalidating the frozen R3bn parent/candidate transition.
[[ -z "$(comm -23 \
    <(printf '%s\n' "$candidate_features") \
    <(printf '%s\n' "$live_features"))" \
    && -z "$(comm -23 \
        <(printf '%s\n' "$candidate_negatives") \
        <(printf '%s\n' "$live_negatives"))" \
    && "$candidate_execution" == "$live_execution" ]] || {
    echo "error: live global profile no longer contains the frozen R3bn candidate semantics" >&2
    exit 1
}

while IFS= read -r test_path; do
    features=$(metadata_list "$test_path" features | LC_ALL=C sort)
    [[ -z "$(comm -23 \
        <(printf '%s\n' "$features") \
        <(printf '%s\n' "$candidate_features"))" ]] || {
        printf 'error: R3bn activation path has another unsupported feature: %s\n' \
            "$test_path" >&2
        exit 1
    }
done <<<"$activation_inventory"
while IFS= read -r test_path; do
    features=$(metadata_list "$test_path" features | LC_ALL=C sort)
    diff -u \
        <(printf '%s\n' globalThis) \
        <(comm -23 \
            <(printf '%s\n' "$features") \
            <(printf '%s\n' "$candidate_features"))
done <<<"$reason_only_inventory"

if [[ "$mode" == check ]]; then
    printf 'R3bn inputs verified: parent %s -> candidate %s features; %s tagged = %s activation + %s globalThis reason-only + %s host/config paths\n' \
        "$(read_value parent_features)" \
        "$(read_value candidate_features)" \
        "$(read_value tagged_paths)" \
        "$(read_value activation_paths)" \
        "$(read_value reason_only_paths)" \
        "$(read_value host_config_paths)"
    exit 0
fi

tag_receipt_fields=(
    transition_receipt_sha256 transition_data_sha256
    before_runnable before_passes before_failures before_unsupported before_skipped
    before_nonpass_sha256 before_tsv_sha256 before_jsonl_sha256 before_summary
    candidate_runnable candidate_passes candidate_failures candidate_unsupported
    candidate_skipped candidate_nonpass_sha256 candidate_tsv_sha256
    candidate_jsonl_sha256 candidate_summary
)
tag_pending=$(pending_keys "${tag_receipt_fields[@]}")
if [[ -n "$tag_pending" && "$mode" != bless ]]; then
    printf 'error: R3bn tag baseline needs --bless after implementation: %s\n' \
        "$(tr '\n' ' ' <<<"$tag_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ -n "$tag_pending" \
    && "$(printf '%s\n' "$tag_pending" | wc -l | tr -d '[:space:]')" \
        != "${#tag_receipt_fields[@]}" ]]; then
    echo "error: R3bn tag baseline is only partially PENDING" >&2
    exit 1
fi
if [[ -z "$tag_pending" && "$mode" == bless ]]; then
    mode=tag
fi

rm -f -- "$before_report" "$before_json_report" "$candidate_report" "$candidate_json_report"
before_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$parent_profile" \
    --manifest "$manifest" \
    --report "$before_report" \
    --mode "$(read_value mode)" \
    --workers "$workers" \
    --timeout-ms "$(read_value timeout_ms)" \
    --allow-failures)
printf '%s\n' "$before_output"
candidate_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$candidate_profile" \
    --manifest "$manifest" \
    --report "$candidate_report" \
    --mode "$(read_value mode)" \
    --workers "$workers" \
    --timeout-ms "$(read_value timeout_ms)" \
    --allow-failures)
printf '%s\n' "$candidate_output"

verify_report_metadata \
    "$before_report" "$(read_value parent_profile_sha256)" "$(read_value tagged_variants)"
verify_report_metadata \
    "$candidate_report" "$(read_value candidate_profile_sha256)" "$(read_value tagged_variants)"
verify_json_report \
    "$before_json_report" "$before_report" "$(read_value parent_profile_sha256)"
verify_json_report \
    "$candidate_json_report" "$candidate_report" "$(read_value candidate_profile_sha256)"
diff -u <(printf '%s\n' "$tagged_inventory" | variant_keys) <(report_keys "$before_report")
diff -u <(report_keys "$before_report") <(report_keys "$candidate_report")

before_runnable=$(execution_runnable "$before_output")
before_passes=$(report_outcome_count "$before_report" '^pass$')
before_unsupported=$(report_outcome_count "$before_report" '^unsupported-')
before_skipped=$(report_outcome_count "$before_report" '^skipped-')
before_failures=$((
    $(read_value tagged_variants)
    - before_passes
    - before_unsupported
    - before_skipped
))
before_nonpass=$(report_nonpass_sha256 "$before_report")
before_summary=$(report_summary "$before_report")
candidate_runnable=$(execution_runnable "$candidate_output")
candidate_passes=$(report_outcome_count "$candidate_report" '^pass$')
candidate_unsupported=$(report_outcome_count "$candidate_report" '^unsupported-')
candidate_skipped=$(report_outcome_count "$candidate_report" '^skipped-')
candidate_failures=$((
    $(read_value tagged_variants)
    - candidate_passes
    - candidate_unsupported
    - candidate_skipped
))
candidate_nonpass=$(report_nonpass_sha256 "$candidate_report")
candidate_summary=$(report_summary "$candidate_report")

expected_before_summary="skipped-config-exclude=2 unsupported-feature=1102 unsupported-host-create-realm=22 unsupported-host-is-html-dda=8"
expected_candidate_summary="pass=1076 skipped-config-exclude=2 unsupported-feature=26 unsupported-host-create-realm=22 unsupported-host-is-html-dda=8"
[[ "$before_runnable" == 0 \
    && "$before_passes" == 0 \
    && "$before_failures" == 0 \
    && "$before_unsupported" == 1132 \
    && "$before_skipped" == 2 \
    && "$before_summary" == "$expected_before_summary" \
    && "$candidate_runnable" == 1076 \
    && "$candidate_passes" == 1076 \
    && "$candidate_failures" == 0 \
    && "$candidate_unsupported" == 56 \
    && "$candidate_skipped" == 2 \
    && "$candidate_summary" == "$expected_candidate_summary" ]] || {
    echo "error: R3bn tag report counts drifted" >&2
    exit 1
}

activation_before_rows=$(rows_for_paths "$activation_inventory" "$before_report")
activation_candidate_rows=$(rows_for_paths "$activation_inventory" "$candidate_report")
reason_before_rows=$(rows_for_paths "$reason_only_inventory" "$before_report")
reason_candidate_rows=$(rows_for_paths "$reason_only_inventory" "$candidate_report")
host_before_rows=$(rows_for_paths "$host_config_inventory" "$before_report")
host_candidate_rows=$(rows_for_paths "$host_config_inventory" "$candidate_report")
[[ "$(printf '%s\n' "$activation_candidate_rows" | awk -F'\t' '
        $7 == "pass" && $8 == "normal" && $9 == "" && $10 == "" {
            count++
        }
        END { print count + 0 }
    ')" \
        == "$(read_value activation_variants)" \
    && "$(printf '%s\n' "$activation_before_rows" | awk -F'\t' '
        $7 == "unsupported-feature" &&
        $8 == "selection" &&
        $9 == "EngineCapability" &&
        $10 == "quickjs-oxide does not declare Test262 feature support: iterator-helpers" {
            count++
        }
        END { print count + 0 }
    ')" == "$(read_value activation_variants)" \
    && "$(printf '%s\n' "$reason_before_rows" | awk -F'\t' '
        $7 == "unsupported-feature" &&
        $8 == "selection" &&
        $9 == "EngineCapability" &&
        $10 == "quickjs-oxide does not declare Test262 feature support: globalThis, iterator-helpers" {
            count++
        }
        END { print count + 0 }
    ')" == "$(read_value reason_only_variants)" \
    && "$(printf '%s\n' "$reason_candidate_rows" | awk -F'\t' '
        $7 == "unsupported-feature" &&
        $8 == "selection" &&
        $9 == "EngineCapability" &&
        $10 == "quickjs-oxide does not declare Test262 feature support: globalThis" {
            count++
        }
        END { print count + 0 }
    ')" == "$(read_value reason_only_variants)" ]] || {
    echo "error: R3bn activation or reason-only transition drifted" >&2
    exit 1
}
diff -u <(printf '%s\n' "$host_before_rows") <(printf '%s\n' "$host_candidate_rows")
[[ "$(printf '%s\n' "$host_candidate_rows" | awk -F'\t' '
        $7 == "unsupported-host-create-realm" { realm++ }
        $7 == "unsupported-host-is-html-dda" { html++ }
        $7 == "skipped-config-exclude" { config++ }
        END { print realm + 0, html + 0, config + 0 }
    ')" == "22 8 2" ]] || {
    echo "error: R3bn host/config classifications drifted" >&2
    exit 1
}

transition_tmp=$(mktemp "$transition_receipt.XXXXXX")
{
    printf '# R3bn exhaustive iterator-helpers global admission transition.\n'
    printf '# before_oxide_profile_sha256=%s\n' "$(read_value parent_profile_sha256)"
    printf '# after_oxide_profile_sha256=%s\n' "$(read_value candidate_profile_sha256)"
    printf '# manifest_sha256=%s\n' "$(read_value tagged_manifest_sha256)"
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
            for (i=1; i<=6; i++) {
                if ($i != field[key, i]) exit 4
            }
            print $1, $2, $3, $4, $5, $6,
                field[key, 7], field[key, 8], field[key, 9], field[key, 10],
                $7, $8, $9, $10
            after[key]=1
            after_count++
        }
        END {
            if (before_count != after_count) exit 5
            for (key in before) if (!(key in after)) exit 6
        }
    ' "$before_report" "$candidate_report"
} >"$transition_tmp"
transition_rows_actual=$(
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { count++ }
        END { print count + 0 }' "$transition_tmp"
)
transition_keys_actual=$(
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2
    }' "$transition_tmp" | LC_ALL=C sort | sha256_stream
)
transition_activation_actual=$(
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") &&
        $7 == "unsupported-feature" && $11 == "pass" { count++ }
        END { print count + 0 }
    ' "$transition_tmp"
)
transition_reason_actual=$(
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") &&
        $7 == "unsupported-feature" && $11 == "unsupported-feature" &&
        $10 != $14 { count++ }
        END { print count + 0 }
    ' "$transition_tmp"
)
transition_host_actual=$(
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            same=1
            for (i=7; i<=10; i++) if ($i != $(i+4)) same=0
            if (same) count++
        }
        END { print count + 0 }
    ' "$transition_tmp"
)
[[ "$transition_rows_actual" == "$(read_value transition_rows)" \
    && "$transition_keys_actual" == "$(read_value transition_keys_sha256)" \
    && "$transition_activation_actual" == "$(read_value transition_activation_rows)" \
    && "$transition_reason_actual" == "$(read_value transition_reason_only_rows)" \
    && "$transition_host_actual" == "$(read_value transition_host_config_rows)" ]] || {
    echo "error: R3bn exhaustive transition partition drifted" >&2
    exit 1
}
transition_sha=$(sha256_file "$transition_tmp")
transition_data_sha=$(
    awk '!/^#/ && !/^path\tvariant\t/' "$transition_tmp" | sha256_stream
)

before_tsv=$(sha256_file "$before_report")
before_jsonl=$(sha256_file "$before_json_report")
candidate_tsv=$(sha256_file "$candidate_report")
candidate_jsonl=$(sha256_file "$candidate_json_report")

if [[ "$mode" == bless ]]; then
    chmod 644 "$transition_tmp"
    mv -- "$transition_tmp" "$transition_receipt"
    update_baseline \
        "transition_receipt_sha256=$transition_sha" \
        "transition_data_sha256=$transition_data_sha" \
        "before_runnable=$before_runnable" \
        "before_passes=$before_passes" \
        "before_failures=$before_failures" \
        "before_unsupported=$before_unsupported" \
        "before_skipped=$before_skipped" \
        "before_nonpass_sha256=$before_nonpass" \
        "before_tsv_sha256=$before_tsv" \
        "before_jsonl_sha256=$before_jsonl" \
        "before_summary=$before_summary" \
        "candidate_runnable=$candidate_runnable" \
        "candidate_passes=$candidate_passes" \
        "candidate_failures=$candidate_failures" \
        "candidate_unsupported=$candidate_unsupported" \
        "candidate_skipped=$candidate_skipped" \
        "candidate_nonpass_sha256=$candidate_nonpass" \
        "candidate_tsv_sha256=$candidate_tsv" \
        "candidate_jsonl_sha256=$candidate_jsonl" \
        "candidate_summary=$candidate_summary"
    printf 'R3bn tag baseline blessed: %s activation passes, %s reason-only and %s host/config variants remain fail-closed\n' \
        "$candidate_passes" \
        "$(read_value reason_only_variants)" \
        "$(read_value host_config_variants)"
    exit 0
fi

[[ -f "$transition_receipt" \
    && "$transition_sha" == "$(read_value transition_receipt_sha256)" \
    && "$transition_data_sha" == "$(read_value transition_data_sha256)" \
    && "$before_runnable" == "$(read_value before_runnable)" \
    && "$before_passes" == "$(read_value before_passes)" \
    && "$before_failures" == "$(read_value before_failures)" \
    && "$before_unsupported" == "$(read_value before_unsupported)" \
    && "$before_skipped" == "$(read_value before_skipped)" \
    && "$before_nonpass" == "$(read_value before_nonpass_sha256)" \
    && "$before_tsv" == "$(read_value before_tsv_sha256)" \
    && "$before_jsonl" == "$(read_value before_jsonl_sha256)" \
    && "$before_summary" == "$(read_value before_summary)" \
    && "$candidate_runnable" == "$(read_value candidate_runnable)" \
    && "$candidate_passes" == "$(read_value candidate_passes)" \
    && "$candidate_failures" == "$(read_value candidate_failures)" \
    && "$candidate_unsupported" == "$(read_value candidate_unsupported)" \
    && "$candidate_skipped" == "$(read_value candidate_skipped)" \
    && "$candidate_nonpass" == "$(read_value candidate_nonpass_sha256)" \
    && "$candidate_tsv" == "$(read_value candidate_tsv_sha256)" \
    && "$candidate_jsonl" == "$(read_value candidate_jsonl_sha256)" \
    && "$candidate_summary" == "$(read_value candidate_summary)" ]] || {
    echo "error: R3bn tag or transition receipt drifted" >&2
    exit 1
}
cmp -s "$transition_tmp" "$transition_receipt" || {
    echo "error: R3bn checked-in transition receipt drifted" >&2
    exit 1
}
rm -f -- "$transition_tmp"

if [[ "$mode" == tag ]]; then
    printf 'R3bn global iterator-helpers tag gate is exact: %s/%s activation variants pass; %s total transition rows\n' \
        "$candidate_passes" "$(read_value activation_variants)" "$(read_value transition_rows)"
    exit 0
fi

full_receipt_fields=(
    full_keys_sha256
    candidate_full_tsv_sha256 candidate_full_jsonl_sha256
    full_non_iterator_tsv_data_sha256 full_non_iterator_jsonl_data_sha256
    before_full_iterator_tsv_data_sha256 before_full_iterator_jsonl_data_sha256
    candidate_full_iterator_tsv_data_sha256
    candidate_full_iterator_jsonl_data_sha256
)
full_pending=$(pending_keys "${full_receipt_fields[@]}")
if [[ -n "$full_pending" && "$mode" != bless-full ]]; then
    printf 'error: R3bn full baseline needs --bless-full after an exact join: %s\n' \
        "$(tr '\n' ' ' <<<"$full_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ -n "$full_pending" \
    && "$(printf '%s\n' "$full_pending" | wc -l | tr -d '[:space:]')" \
        != "${#full_receipt_fields[@]}" ]]; then
    echo "error: R3bn full baseline is only partially PENDING" >&2
    exit 1
fi
if [[ -z "$full_pending" && "$mode" == bless-full ]]; then
    mode=full
fi

rm -f -- \
    "$before_full_report" "$before_full_json_report" \
    "$candidate_full_report" "$candidate_full_json_report"
before_full_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$parent_profile" \
    --all \
    --report "$before_full_report" \
    --mode "$(read_value mode)" \
    --workers "$full_workers" \
    --timeout-ms "$(read_value timeout_ms)" \
    --allow-failures)
printf '%s\n' "$before_full_output"
[[ "$(sha256_file "$before_full_report")" == "$(read_value before_full_tsv_sha256)" \
    && "$(sha256_file "$before_full_json_report")" \
        == "$(read_value before_full_jsonl_sha256)" ]] || {
    echo "error: R3bn authoritative pre-admission full vector drifted" >&2
    exit 1
}
candidate_full_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$candidate_profile" \
    --all \
    --report "$candidate_full_report" \
    --mode "$(read_value mode)" \
    --workers "$full_workers" \
    --timeout-ms "$(read_value timeout_ms)" \
    --allow-failures)
printf '%s\n' "$candidate_full_output"

verify_report_metadata \
    "$before_full_report" "$(read_value parent_profile_sha256)" "$(read_value full_variants)"
verify_report_metadata \
    "$candidate_full_report" "$(read_value candidate_profile_sha256)" "$(read_value full_variants)"
verify_json_report \
    "$before_full_json_report" "$before_full_report" "$(read_value parent_profile_sha256)"
verify_json_report \
    "$candidate_full_json_report" "$candidate_full_report" "$(read_value candidate_profile_sha256)"
before_full_keys=$(report_keys "$before_full_report")
candidate_full_keys=$(report_keys "$candidate_full_report")
diff -u <(printf '%s\n' "$before_full_keys") <(printf '%s\n' "$candidate_full_keys")
full_keys_sha=$(printf '%s\n' "$before_full_keys" | sha256_stream)

before_full_runnable=$(execution_runnable "$before_full_output")
candidate_full_runnable=$(execution_runnable "$candidate_full_output")
before_full_passes=$(report_outcome_count "$before_full_report" '^pass$')
candidate_full_passes=$(report_outcome_count "$candidate_full_report" '^pass$')
before_full_summary=$(report_summary "$before_full_report")
candidate_full_summary=$(report_summary "$candidate_full_report")
[[ "$before_full_runnable" == "$(read_value before_full_runnable)" \
    && "$before_full_passes" == "$(read_value before_full_passes)" \
    && "$before_full_summary" == "$(read_value before_full_summary)" \
    && "$candidate_full_runnable" == "$(read_value candidate_full_runnable)" \
    && "$candidate_full_passes" == "$(read_value candidate_full_passes)" \
    && "$candidate_full_summary" == "$(read_value candidate_full_summary)" ]] || {
    echo "error: R3bn full summary or runnable count drifted" >&2
    exit 1
}

before_full_iterator_rows=$(rows_for_paths "$tagged_inventory" "$before_full_report")
candidate_full_iterator_rows=$(rows_for_paths "$tagged_inventory" "$candidate_full_report")
before_full_non_iterator_rows=$(rows_without_paths "$tagged_inventory" "$before_full_report")
candidate_full_non_iterator_rows=$(rows_without_paths "$tagged_inventory" "$candidate_full_report")
[[ "$(printf '%s\n' "$before_full_non_iterator_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_non_iterator_rows)" ]] || {
    echo "error: R3bn full non-Iterator row count drifted" >&2
    exit 1
}
diff -u <(printf '%s\n' "$before_full_iterator_rows") <(report_rows "$before_report")
diff -u <(printf '%s\n' "$candidate_full_iterator_rows") <(report_rows "$candidate_report")
diff -u \
    <(printf '%s\n' "$before_full_non_iterator_rows") \
    <(printf '%s\n' "$candidate_full_non_iterator_rows")

before_full_iterator_json=$(json_rows_for_paths "$tagged_inventory" "$before_full_json_report")
candidate_full_iterator_json=$(json_rows_for_paths "$tagged_inventory" "$candidate_full_json_report")
before_full_non_iterator_json=$(
    json_rows_without_paths "$tagged_inventory" "$before_full_json_report"
)
candidate_full_non_iterator_json=$(
    json_rows_without_paths "$tagged_inventory" "$candidate_full_json_report"
)
diff -u \
    <(printf '%s\n' "$before_full_iterator_json") \
    <(json_rows_for_paths "$tagged_inventory" "$before_json_report")
diff -u \
    <(printf '%s\n' "$candidate_full_iterator_json") \
    <(json_rows_for_paths "$tagged_inventory" "$candidate_json_report")
diff -u \
    <(printf '%s\n' "$before_full_non_iterator_json") \
    <(printf '%s\n' "$candidate_full_non_iterator_json")

before_full_tsv=$(sha256_file "$before_full_report")
before_full_jsonl=$(sha256_file "$before_full_json_report")
candidate_full_tsv=$(sha256_file "$candidate_full_report")
candidate_full_jsonl=$(sha256_file "$candidate_full_json_report")
[[ "$before_full_tsv" == "$(read_value before_full_tsv_sha256)" \
    && "$before_full_jsonl" == "$(read_value before_full_jsonl_sha256)" ]] || {
    echo "error: R3bn authoritative pre-admission full vector drifted" >&2
    exit 1
}
non_iterator_tsv_sha=$(printf '%s\n' "$before_full_non_iterator_rows" | sha256_stream)
non_iterator_json_sha=$(printf '%s\n' "$before_full_non_iterator_json" | sha256_stream)
before_iterator_tsv_sha=$(printf '%s\n' "$before_full_iterator_rows" | sha256_stream)
before_iterator_json_sha=$(printf '%s\n' "$before_full_iterator_json" | sha256_stream)
candidate_iterator_tsv_sha=$(
    printf '%s\n' "$candidate_full_iterator_rows" | sha256_stream
)
candidate_iterator_json_sha=$(
    printf '%s\n' "$candidate_full_iterator_json" | sha256_stream
)

if [[ "$mode" == bless-full ]]; then
    update_baseline \
        "full_keys_sha256=$full_keys_sha" \
        "candidate_full_tsv_sha256=$candidate_full_tsv" \
        "candidate_full_jsonl_sha256=$candidate_full_jsonl" \
        "full_non_iterator_tsv_data_sha256=$non_iterator_tsv_sha" \
        "full_non_iterator_jsonl_data_sha256=$non_iterator_json_sha" \
        "before_full_iterator_tsv_data_sha256=$before_iterator_tsv_sha" \
        "before_full_iterator_jsonl_data_sha256=$before_iterator_json_sha" \
        "candidate_full_iterator_tsv_data_sha256=$candidate_iterator_tsv_sha" \
        "candidate_full_iterator_jsonl_data_sha256=$candidate_iterator_json_sha"
    printf 'R3bn full transition blessed: %s non-Iterator rows identical; pass %s -> %s with exact %s-row Iterator join\n' \
        "$(read_value full_non_iterator_rows)" \
        "$before_full_passes" \
        "$candidate_full_passes" \
        "$(read_value transition_rows)"
    exit 0
fi

[[ "$full_keys_sha" == "$(read_value full_keys_sha256)" \
    && "$before_full_tsv" == "$(read_value before_full_tsv_sha256)" \
    && "$before_full_jsonl" == "$(read_value before_full_jsonl_sha256)" \
    && "$candidate_full_tsv" == "$(read_value candidate_full_tsv_sha256)" \
    && "$candidate_full_jsonl" == "$(read_value candidate_full_jsonl_sha256)" \
    && "$non_iterator_tsv_sha" == "$(read_value full_non_iterator_tsv_data_sha256)" \
    && "$non_iterator_json_sha" == "$(read_value full_non_iterator_jsonl_data_sha256)" \
    && "$before_iterator_tsv_sha" \
        == "$(read_value before_full_iterator_tsv_data_sha256)" \
    && "$before_iterator_json_sha" \
        == "$(read_value before_full_iterator_jsonl_data_sha256)" \
    && "$candidate_iterator_tsv_sha" \
        == "$(read_value candidate_full_iterator_tsv_data_sha256)" \
    && "$candidate_iterator_json_sha" \
        == "$(read_value candidate_full_iterator_jsonl_data_sha256)" ]] || {
    echo "error: R3bn full receipt drifted" >&2
    exit 1
}

printf 'R3bn full transition is exact: %s non-Iterator rows unchanged; %s -> %s pass across %s keys\n' \
    "$(read_value full_non_iterator_rows)" \
    "$before_full_passes" \
    "$candidate_full_passes" \
    "$(read_value full_variants)"
