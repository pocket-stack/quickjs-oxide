#!/usr/bin/env bash
# Reproduce the complete resizable-arraybuffer capability admission and its
# exact parent/candidate Test262 joins.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-resizable-arraybuffer-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
parent_profile=tests/test262-resizable-arraybuffer-global-parent.conf
candidate_profile=tests/test262-resizable-arraybuffer-global-candidate.conf
live_profile=compat/test262-oxide.conf
universe_manifest=tests/test262-resizable-arraybuffer-universe.txt
activation_manifest=tests/test262-resizable-arraybuffer.txt
reason_only_manifest=tests/test262-resizable-arraybuffer-reason-only.txt
config_skipped_manifest=tests/test262-resizable-arraybuffer-config-skipped.txt
transition_receipt=tests/test262-resizable-arraybuffer-global-transitions.tsv
parent_report=target/test262-resizable-arraybuffer-global-parent.tsv
parent_json_report=target/test262-resizable-arraybuffer-global-parent.jsonl
candidate_report=target/test262-resizable-arraybuffer-global-candidate.tsv
candidate_json_report=target/test262-resizable-arraybuffer-global-candidate.jsonl
parent_full_report=target/test262-resizable-arraybuffer-global-parent-full.tsv
parent_full_json_report=target/test262-resizable-arraybuffer-global-parent-full.jsonl
candidate_full_report=target/test262-resizable-arraybuffer-global-candidate-full.tsv
candidate_full_json_report=target/test262-resizable-arraybuffer-global-candidate-full.jsonl
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
transition_tmp=

cleanup() {
    [[ -z "$transition_tmp" ]] || rm -f -- "$transition_tmp"
}
trap cleanup EXIT

usage() {
    printf 'usage: %s [--check|--bless|--full|--bless-full]\n' "${0##*/}"
    printf '  --check       verify profiles, partitions, QuickJS, and focused oracle\n'
    printf '  --bless       bless the 926-row tag transition receipt\n'
    printf '  --full        verify the exact 102037-row parent/candidate join\n'
    printf '  --bless-full  bless full receipts after the exact no-regression join\n'
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
[[ "$workers" =~ ^[1-9][0-9]*$ ]] || {
    echo "error: TEST262_WORKERS must be a positive integer" >&2
    exit 2
}
[[ "$full_workers" =~ ^[1-9][0-9]*$ ]] || {
    echo "error: TEST262_FULL_WORKERS must be a positive integer" >&2
    exit 2
}

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
        printf 'error: R3bu baseline identity drifted for %s: %s != %s\n' \
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

profile_section() {
    local profile=$1 section=$2
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
    done | LC_ALL=C sort
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

json_report_keys() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3bu JSONL report %s: %s\n", report, message >"/dev/stderr"
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
            if (!failed && metadata != 1) fail("expected one metadata record")
            if (!failed && summary != 1) fail("expected one summary record")
            if (!failed && summary_line != NR) fail("summary record is not last")
        }
    ' "$report" | LC_ALL=C sort
}

json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3bu JSONL projection %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        function expect(token) {
            if (substr(line, position, length(token)) != token) {
                fail("expected " token " at column " position)
            }
            position+=length(token)
        }
        # Return the JSON string in the runner TSV field encoding. JSON and
        # TSV escape backslashes/control characters differently, so retaining
        # the raw JSON spelling would make a cross-format comparison unsound.
        function string_value(    character, escape, digits, value) {
            expect("\"")
            value=""
            while (position <= length(line)) {
                character=substr(line, position, 1)
                if (character == "\"") {
                    position++
                    return value
                }
                if (character == "\\") {
                    position++
                    if (position > length(line)) fail("unterminated escape")
                    escape=substr(line, position, 1)
                    if (escape == "u") {
                        digits=substr(line, position + 1, 4)
                        if (length(digits) != 4 ||
                            digits ~ /[^0123456789abcdefABCDEF]/) {
                            fail("invalid Unicode escape")
                        }
                        # The runner emits \u escapes only for U+0000..001F;
                        # its TSV codec spells those as the same lowercase
                        # escape instead of materializing control bytes.
                        if (digits !~ /^00[01][0123456789abcdefABCDEF]$/) {
                            fail("unexpected non-control Unicode escape")
                        }
                        value=value "\\u" tolower(digits)
                        position+=5
                    } else {
                        if (index("\"\\/bfnrt", escape) == 0) {
                            fail("invalid string escape")
                        }
                        if (escape == "\"") value=value "\""
                        else if (escape == "\\") value=value "\\\\"
                        else if (escape == "/") value=value "/"
                        else if (escape == "b") value=value "\\u0008"
                        else if (escape == "f") value=value "\\u000c"
                        else value=value "\\" escape
                        position++
                    }
                    continue
                }
                if (character == "\t" || character == "\r") {
                    fail("unescaped control character")
                }
                value=value character
                position++
            }
            fail("unterminated string")
        }
        function project_result(    i, key, value) {
            line=$0
            position=1
            expect("{")
            for (i=1; i<=11; i++) {
                if (i != 1) expect(",")
                key=string_value()
                if (key != name[i]) fail("unexpected field " key)
                expect(":")
                value=string_value()
                if (i == 1) {
                    if (value != "result") fail("unexpected record kind")
                } else {
                    field[i - 1]=value
                }
            }
            expect("}")
            if (position != length(line) + 1) fail("trailing record data")
            print field[1], field[2], field[3], field[4], field[5],
                field[6], field[7], field[8], field[9], field[10]
        }
        BEGIN {
            OFS="\t"
            name[1]="kind"
            name[2]="path"
            name[3]="variant"
            name[4]="flags"
            name[5]="features"
            name[6]="expected_phase"
            name[7]="expected_type"
            name[8]="outcome"
            name[9]="actual_phase"
            name[10]="actual_type"
            name[11]="detail"
        }
        /^\{"kind":"metadata",/ { next }
        /^\{"kind":"result",/ { project_result(); next }
        /^\{"kind":"summary",/ { next }
        { fail("unexpected record") }
    ' "$report"
}

projection_rows_for_paths() {
    local paths=$1 projection=$2
    awk -F'\t' '
        NR == FNR { if ($0 != "") wanted[$0]=1; next }
        $1 in wanted { print }
    ' <(printf '%s\n' "$paths") <(printf '%s\n' "$projection")
}

projection_rows_without_paths() {
    local paths=$1 projection=$2
    awk -F'\t' '
        NR == FNR { if ($0 != "") blocked[$0]=1; next }
        !($1 in blocked) { print }
    ' <(printf '%s\n' "$paths") <(printf '%s\n' "$projection")
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
    local report=$1 pattern=$2
    report_rows "$report" | awk -F'\t' -v pattern="$pattern" '
        $7 ~ pattern { count++ }
        END { print count + 0 }
    '
}

report_nonpass_sha256() {
    report_rows "$1" | awk -F'\t' '$7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' | sha256_stream
}

report_summary() {
    tail -n 1 "$1" | sed 's/^# summary //'
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

verify_report() {
    local report=$1 json_report=$2 expected_profile=$3 expected_rows=$4
    local tsv_keys json_keys
    [[ -f "$json_report" \
        && "$(read_header "$report" quickjs)" == "$(read_value quickjs)" \
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
            == "$expected_rows" ]] || {
        printf 'error: R3bu report metadata drifted: %s\n' "$report" >&2
        exit 1
    }
    tsv_keys=$(report_keys "$report")
    json_keys=$(json_report_keys "$json_report")
    diff -u <(printf '%s\n' "$tsv_keys") <(printf '%s\n' "$json_keys")
    [[ "$(head -n 1 "$json_report")" \
            == "$(expected_json_metadata "$expected_profile")" \
        && "$(json_report_summary "$json_report")" \
            == "$(report_summary "$report")" ]] || {
        printf 'error: R3bu JSONL metadata drifted: %s\n' "$json_report" >&2
        exit 1
    }
}

verify_partition_transition() {
    local encoding=$1 kind=$2 before_rows=$3 candidate_rows=$4 expected_rows=$5
    if ! awk -F'\t' -v kind="$kind" -v expected="$expected_rows" '
        function remove_feature(detail,    prefix, values, n, i, kept, found, result) {
            prefix="quickjs-oxide does not declare Test262 feature support: "
            if (index(detail, prefix) != 1) return "!bad-prefix!"
            detail=substr(detail, length(prefix) + 1)
            n=split(detail, values, /, /)
            result=""
            for (i=1; i<=n; i++) {
                if (values[i] == "resizable-arraybuffer") {
                    found++
                } else {
                    kept++
                    result=result (result == "" ? "" : ", ") values[i]
                }
            }
            if (found != 1) return "!bad-feature-count!"
            remaining=kept
            return result == "" ? "" : prefix result
        }
        NR == FNR {
            key=$1 SUBSEP $2
            if (key in before) exit 2
            for (i=1; i<=10; i++) old[key, i]=$i
            if (kind == "config") {
                if ($7 != "skipped-feature" || $8 != "selection") exit 3
            } else {
                if (!($7 == "unsupported-feature" && $8 == "selection" &&
                    $9 == "EngineCapability")) exit 4
                expected_detail[key]=remove_feature($10)
                remaining_count[key]=remaining
                if (expected_detail[key] ~ /^!bad-/) exit 5
                if (kind == "activation" && remaining != 0) exit 6
                if (kind == "reason" && remaining == 0) exit 7
            }
            before[key]=1
            before_count++
            next
        }
        {
            key=$1 SUBSEP $2
            if (!(key in before) || key in after) exit 8
            for (i=1; i<=6; i++) if ($i != old[key, i]) exit 9
            if (kind == "activation") {
                if (!($7 == "pass" && $8 == "normal" &&
                    $9 == "" && $10 == "")) exit 10
            } else if (kind == "reason") {
                if (!($7 == "unsupported-feature" && $8 == "selection" &&
                    $9 == "EngineCapability" &&
                    $10 == expected_detail[key])) exit 11
            } else {
                for (i=7; i<=10; i++) if ($i != old[key, i]) exit 12
            }
            after[key]=1
            after_count++
        }
        END {
            if (before_count != expected || after_count != expected) exit 13
            for (key in before) if (!(key in after)) exit 14
        }
    ' <(printf '%s\n' "$before_rows") \
        <(printf '%s\n' "$candidate_rows"); then
        printf 'error: R3bu %s %s transition drifted\n' \
            "$encoding" "$kind" >&2
        exit 1
    fi
}

verify_manifest() {
    local prefix=$1 file=$2 paths keys count
    paths=$(manifest_paths "$file")
    keys=$(printf '%s\n' "$paths" | variant_keys)
    count=$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    printf '%s\n' "$paths" | LC_ALL=C sort -c
    [[ "$count" == "$(read_value "${prefix}_paths")" \
        && "$(printf '%s\n' "$paths" | sha256_stream)" \
            == "$(read_value "${prefix}_sha256")" \
        && "$(printf '%s\n' "$keys" | wc -l | tr -d '[:space:]')" \
            == "$(read_value "${prefix}_variants")" \
        && "$(printf '%s\n' "$keys" | sha256_stream)" \
            == "$(read_value "${prefix}_keys_sha256")" ]] || {
        printf 'error: R3bu %s manifest drifted\n' "$prefix" >&2
        exit 1
    }
}

update_values() {
    local file=$1 updates_tmp output_tmp entry
    shift
    updates_tmp=$(mktemp "$file.updates.XXXXXX")
    output_tmp=$(mktemp "$file.output.XXXXXX")
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
    rm -f -- "$updates_tmp"
}

pending_keys() {
    local key
    for key in "$@"; do
        [[ "$(read_value "$key")" == PENDING ]] && printf '%s\n' "$key"
    done
    return 0
}

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
expect_value parent_oxide_profile_sha256 ed80ab5aed86c606a1d7b5c1854b78ab1bb3c517cf0c6898a89e9f8d19135000
expect_value candidate_oxide_profile_sha256 e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898
expect_value parent_features 89
expect_value candidate_features 90
expect_value added_features 1
expect_value audited_negative_tests 828
expect_value universe_paths 463
expect_value universe_variants 926
expect_value activation_paths 381
expect_value activation_variants 762
expect_value reason_only_paths 80
expect_value reason_only_variants 160
expect_value config_skipped_paths 2
expect_value config_skipped_variants 4
expect_value global_transition_receipt tests/test262-resizable-arraybuffer-global-transitions.tsv
expect_value global_transition_rows 926
expect_value global_transition_activation_rows 762
expect_value global_transition_reason_only_rows 160
expect_value global_transition_unchanged_config_rows 4
expect_value global_expected_parent_tag_runnable 0
expect_value global_expected_parent_tag_summary 'skipped-feature=4 unsupported-feature=922'
expect_value global_expected_candidate_tag_runnable 762
expect_value global_expected_candidate_tag_summary 'pass=762 skipped-feature=4 unsupported-feature=160'
expect_value global_full_variants 102037
expect_value global_full_universe_rows 926
expect_value global_full_activation_rows 762
expect_value global_full_reason_only_rows 160
expect_value global_full_config_skipped_rows 4
expect_value global_full_non_universe_rows 101111
expect_value global_full_changed_rows 922
expect_value global_full_outcome_changed_rows 762
expect_value global_full_detail_only_rows 160
expect_value global_full_unchanged_rows 101115
expect_value global_previous_pass_regressions 0
expect_value global_parent_full_runnable 58825
expect_value global_parent_full_passes 58306
expect_value global_parent_full_unsupported_feature 19819
expect_value parent_full_tsv_sha256 789b1d116e10dbeb7607faf4bbbcb5df818a6e588799d156579b5047238b0379
expect_value parent_full_jsonl_sha256 d1476490e0f53bb1397ce432c813c781e51130cfd97da22e1fdc8edc10f95a8f
expect_value global_expected_candidate_full_runnable 59587
expect_value global_expected_candidate_full_passes 59068
expect_value global_expected_candidate_full_unsupported_feature 19057

for required in \
    "$baseline" "$canonical_baseline" \
    "$parent_profile" "$candidate_profile" "$live_profile" \
    "$universe_manifest" "$activation_manifest" \
    "$reason_only_manifest" "$config_skipped_manifest"
do
    [[ -f "$required" ]] || {
        printf 'error: missing R3bu asset: %s\n' "$required" >&2
        exit 1
    }
done

parent_features=$(profile_section "$parent_profile" features)
candidate_features=$(profile_section "$candidate_profile" features)
live_features=$(profile_section "$live_profile" features)
parent_negatives=$(profile_section "$parent_profile" audited-negative-tests)
candidate_negatives=$(profile_section "$candidate_profile" audited-negative-tests)
live_negatives=$(profile_section "$live_profile" audited-negative-tests)
parent_execution=$(profile_section "$parent_profile" execution)
candidate_execution=$(profile_section "$candidate_profile" execution)
live_execution=$(profile_section "$live_profile" execution)
added_features=resizable-arraybuffer

[[ "$(sha256_file "$parent_profile")" \
        == "$(read_value parent_oxide_profile_sha256)" \
    && "$(sha256_file "$candidate_profile")" \
        == "$(read_value candidate_oxide_profile_sha256)" \
    && "$(printf '%s\n' "$parent_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value parent_features)" \
    && "$(printf '%s\n' "$parent_features" | sha256_stream)" \
        == "$(read_value parent_features_sha256)" \
    && "$(printf '%s\n' "$candidate_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value candidate_features)" \
    && "$(printf '%s\n' "$candidate_features" | sha256_stream)" \
        == "$(read_value candidate_features_sha256)" \
    && "$(printf '%s\n' "$candidate_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value audited_negative_tests)" \
    && "$(printf '%s\n' "$candidate_negatives" | sha256_stream)" \
        == "$(read_value audited_negative_tests_sha256)" \
    && "$(printf '%s\n' "$added_features" | sha256_stream)" \
        == "$(read_value added_features_sha256)" ]] || {
    echo "error: R3bu frozen profile identity drifted" >&2
    exit 1
}
diff -u <(printf '%s\n' "$parent_negatives") \
    <(printf '%s\n' "$candidate_negatives")
diff -u <(printf '%s\n' "$parent_execution") \
    <(printf '%s\n' "$candidate_execution")
diff -u <(printf '%s\n' "$added_features") \
    <(comm -13 <(printf '%s\n' "$parent_features") \
        <(printf '%s\n' "$candidate_features"))
[[ -z "$(comm -23 <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))" ]] || {
    echo "error: R3bu candidate removed a parent feature" >&2
    exit 1
}
[[ -z "$(comm -23 <(printf '%s\n' "$candidate_features") \
    <(printf '%s\n' "$live_features"))" ]] || {
    echo "error: live profile removed an R3bu candidate feature" >&2
    exit 1
}
[[ -z "$(comm -23 <(printf '%s\n' "$candidate_negatives") \
    <(printf '%s\n' "$live_negatives"))" ]] || {
    echo "error: live profile removed an R3bu negative audit" >&2
    exit 1
}
diff -u <(printf '%s\n' "$candidate_execution") \
    <(printf '%s\n' "$live_execution")

live_profile_sha256=$(sha256_file "$live_profile")
upstream_profile=$(awk -F'"' '
    $1 ~ /^oxide_profile_sha256 = / { print $2; found++ }
    END { if (found != 1) exit 1 }
' compat/upstream.toml)
[[ "$upstream_profile" == "$live_profile_sha256" ]] || {
    echo "error: compat/upstream.toml does not authenticate the live profile" >&2
    exit 1
}

verify_manifest universe "$universe_manifest"
verify_manifest activation "$activation_manifest"
verify_manifest reason_only "$reason_only_manifest"
verify_manifest config_skipped "$config_skipped_manifest"
universe_paths=$(manifest_paths "$universe_manifest")
activation_paths=$(manifest_paths "$activation_manifest")
reason_only_paths=$(manifest_paths "$reason_only_manifest")
config_skipped_paths=$(manifest_paths "$config_skipped_manifest")
partition_paths=$(printf '%s\n%s\n%s\n' \
    "$activation_paths" "$reason_only_paths" "$config_skipped_paths" \
    | LC_ALL=C sort)
[[ "$(printf '%s\n' "$partition_paths" | uniq -d | wc -l \
        | tr -d '[:space:]')" == 0 ]] || {
    echo "error: R3bu path partitions overlap" >&2
    exit 1
}
diff -u <(printf '%s\n' "$universe_paths") \
    <(printf '%s\n' "$partition_paths")

if [[ "$mode" == check ]]; then
    "$script_dir/test-test262-resizable-arraybuffer.sh" --check
    printf 'R3bu inputs verified: %s paths = %s activation + %s reason-only + %s config-skip\n' \
        "$(read_value universe_paths)" "$(read_value activation_paths)" \
        "$(read_value reason_only_paths)" "$(read_value config_skipped_paths)"
    exit 0
fi

# The global gate is downstream of the complete focused QuickJS and Oxide
# certification, so a tag receipt cannot be blessed in isolation.
TEST262_WORKERS="$workers" \
    "$script_dir/test-test262-resizable-arraybuffer.sh"

tag_receipt_fields=(
    global_transition_receipt_sha256 global_transition_data_sha256
    global_parent_tag_nonpass_sha256 global_parent_tag_tsv_sha256
    global_parent_tag_jsonl_sha256 global_candidate_tag_nonpass_sha256
    global_candidate_tag_tsv_sha256 global_candidate_tag_jsonl_sha256
)
tag_pending=$(pending_keys "${tag_receipt_fields[@]}")
tag_pending_count=$(printf '%s\n' "$tag_pending" | sed '/^$/d' \
    | wc -l | tr -d '[:space:]')
if [[ "$tag_pending_count" != 0 \
    && "$tag_pending_count" != "${#tag_receipt_fields[@]}" ]]; then
    echo "error: R3bu tag receipt is only partially PENDING" >&2
    exit 1
fi
if [[ "$tag_pending_count" != 0 && "$mode" != bless ]]; then
    printf 'error: R3bu tag baseline needs --bless: %s\n' \
        "$(tr '\n' ' ' <<<"$tag_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
[[ "$tag_pending_count" != 0 || "$mode" != bless ]] || mode=tag

rm -f -- "$parent_report" "$parent_json_report"
parent_output=$(run_tag "$parent_profile" "$parent_report")
printf '%s\n' "$parent_output"
verify_report "$parent_report" "$parent_json_report" \
    "$(read_value parent_oxide_profile_sha256)" \
    "$(read_value universe_variants)"
diff -u <(printf '%s\n' "$universe_paths" | variant_keys) \
    <(report_keys "$parent_report")
parent_runnable=$(execution_runnable "$parent_output")
parent_summary=$(report_summary "$parent_report")
[[ "$parent_runnable" == "$(read_value global_expected_parent_tag_runnable)" \
    && "$parent_summary" == "$(read_value global_expected_parent_tag_summary)" ]] || {
    echo "error: R3bu parent tag vector drifted" >&2
    exit 1
}

rm -f -- "$candidate_report" "$candidate_json_report"
candidate_output=$(run_tag "$candidate_profile" "$candidate_report")
printf '%s\n' "$candidate_output"
verify_report "$candidate_report" "$candidate_json_report" \
    "$(read_value candidate_oxide_profile_sha256)" \
    "$(read_value universe_variants)"
diff -u <(report_keys "$parent_report") <(report_keys "$candidate_report")
candidate_runnable=$(execution_runnable "$candidate_output")
candidate_summary=$(report_summary "$candidate_report")
[[ "$candidate_runnable" \
        == "$(read_value global_expected_candidate_tag_runnable)" \
    && "$candidate_summary" \
        == "$(read_value global_expected_candidate_tag_summary)" ]] || {
    echo "error: R3bu candidate tag vector drifted" >&2
    exit 1
}

parent_activation_rows=$(rows_for_paths "$activation_paths" "$parent_report")
candidate_activation_rows=$(rows_for_paths "$activation_paths" "$candidate_report")
parent_reason_rows=$(rows_for_paths "$reason_only_paths" "$parent_report")
candidate_reason_rows=$(rows_for_paths "$reason_only_paths" "$candidate_report")
parent_config_rows=$(rows_for_paths "$config_skipped_paths" "$parent_report")
candidate_config_rows=$(rows_for_paths "$config_skipped_paths" "$candidate_report")
verify_partition_transition TSV activation "$parent_activation_rows" \
    "$candidate_activation_rows" "$(read_value activation_variants)"
verify_partition_transition TSV reason "$parent_reason_rows" \
    "$candidate_reason_rows" "$(read_value reason_only_variants)"
verify_partition_transition TSV config "$parent_config_rows" \
    "$candidate_config_rows" "$(read_value config_skipped_variants)"

parent_json_projection=$(json_result_projection "$parent_json_report")
candidate_json_projection=$(json_result_projection "$candidate_json_report")
diff -u <(report_rows "$parent_report") \
    <(printf '%s\n' "$parent_json_projection")
diff -u <(report_rows "$candidate_report") \
    <(printf '%s\n' "$candidate_json_projection")
verify_partition_transition JSONL activation \
    "$(projection_rows_for_paths "$activation_paths" "$parent_json_projection")" \
    "$(projection_rows_for_paths "$activation_paths" "$candidate_json_projection")" \
    "$(read_value activation_variants)"
verify_partition_transition JSONL reason \
    "$(projection_rows_for_paths "$reason_only_paths" "$parent_json_projection")" \
    "$(projection_rows_for_paths "$reason_only_paths" "$candidate_json_projection")" \
    "$(read_value reason_only_variants)"
verify_partition_transition JSONL config \
    "$(projection_rows_for_paths "$config_skipped_paths" "$parent_json_projection")" \
    "$(projection_rows_for_paths "$config_skipped_paths" "$candidate_json_projection")" \
    "$(read_value config_skipped_variants)"

transition_tmp=$(mktemp "$transition_receipt.XXXXXX")
{
    printf '# R3bu exhaustive resizable-arraybuffer global admission transition.\n'
    printf '# before_oxide_profile_sha256=%s\n' \
        "$(read_value parent_oxide_profile_sha256)"
    printf '# after_oxide_profile_sha256=%s\n' \
        "$(read_value candidate_oxide_profile_sha256)"
    printf '# manifest_sha256=%s\n' "$(read_value universe_sha256)"
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
    ' "$parent_report" "$candidate_report"
} >"$transition_tmp"

transition_counts=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") {
        rows++
        changed=0
        for (i=7; i<=10; i++) if ($i != $(i+4)) changed=1
        if (!changed && $7 == "skipped-feature") config++
        else if ($7 == "unsupported-feature" && $11 == "pass") activation++
        else if ($7 == "unsupported-feature" &&
                 $11 == "unsupported-feature" && $10 != $14) reason++
        else other++
    }
    END { print rows + 0, activation + 0, reason + 0, config + 0, other + 0 }
' "$transition_tmp")
[[ "$transition_counts" == \
    "$(read_value global_transition_rows) $(read_value global_transition_activation_rows) $(read_value global_transition_reason_only_rows) $(read_value global_transition_unchanged_config_rows) 0" ]] || {
    echo "error: R3bu transition receipt partition drifted" >&2
    exit 1
}

transition_sha=$(sha256_file "$transition_tmp")
transition_data_sha=$(awk '!/^#/ && !/^path\tvariant\t/' "$transition_tmp" \
    | sha256_stream)
parent_nonpass=$(report_nonpass_sha256 "$parent_report")
candidate_nonpass=$(report_nonpass_sha256 "$candidate_report")
parent_tsv=$(sha256_file "$parent_report")
parent_jsonl=$(sha256_file "$parent_json_report")
candidate_tsv=$(sha256_file "$candidate_report")
candidate_jsonl=$(sha256_file "$candidate_json_report")

if [[ "$mode" == bless ]]; then
    chmod 644 "$transition_tmp"
    mv -- "$transition_tmp" "$transition_receipt"
    transition_tmp=
    update_values "$baseline" \
        "global_transition_receipt_sha256=$transition_sha" \
        "global_transition_data_sha256=$transition_data_sha" \
        "global_parent_tag_nonpass_sha256=$parent_nonpass" \
        "global_parent_tag_tsv_sha256=$parent_tsv" \
        "global_parent_tag_jsonl_sha256=$parent_jsonl" \
        "global_candidate_tag_nonpass_sha256=$candidate_nonpass" \
        "global_candidate_tag_tsv_sha256=$candidate_tsv" \
        "global_candidate_tag_jsonl_sha256=$candidate_jsonl"
    printf 'R3bu tag baseline blessed: %s activation, %s reason-only, %s unchanged config variants\n' \
        "$(read_value global_transition_activation_rows)" \
        "$(read_value global_transition_reason_only_rows)" \
        "$(read_value global_transition_unchanged_config_rows)"
    exit 0
fi

[[ -f "$transition_receipt" \
    && "$transition_sha" == "$(read_value global_transition_receipt_sha256)" \
    && "$transition_data_sha" == "$(read_value global_transition_data_sha256)" \
    && "$parent_nonpass" == "$(read_value global_parent_tag_nonpass_sha256)" \
    && "$parent_tsv" == "$(read_value global_parent_tag_tsv_sha256)" \
    && "$parent_jsonl" == "$(read_value global_parent_tag_jsonl_sha256)" \
    && "$candidate_nonpass" \
        == "$(read_value global_candidate_tag_nonpass_sha256)" \
    && "$candidate_tsv" == "$(read_value global_candidate_tag_tsv_sha256)" \
    && "$candidate_jsonl" \
        == "$(read_value global_candidate_tag_jsonl_sha256)" ]] || {
    echo "error: R3bu tag or transition receipt drifted" >&2
    exit 1
}
cmp -s "$transition_tmp" "$transition_receipt" || {
    echo "error: R3bu checked-in transition receipt drifted" >&2
    exit 1
}
rm -f -- "$transition_tmp"
transition_tmp=

if [[ "$mode" == tag ]]; then
    printf 'R3bu tag gate is exact: %s outcome changes, %s detail-only changes, %s unchanged skips\n' \
        "$(read_value global_transition_activation_rows)" \
        "$(read_value global_transition_reason_only_rows)" \
        "$(read_value global_transition_unchanged_config_rows)"
    exit 0
fi

full_receipt_fields=(
    global_candidate_full_tsv_sha256 global_candidate_full_jsonl_sha256
    global_activation_parent_tsv_data_sha256
    global_activation_parent_jsonl_data_sha256
    global_activation_candidate_tsv_data_sha256
    global_activation_candidate_jsonl_data_sha256
    global_reason_only_parent_tsv_data_sha256
    global_reason_only_parent_jsonl_data_sha256
    global_reason_only_candidate_tsv_data_sha256
    global_reason_only_candidate_jsonl_data_sha256
    global_config_skipped_tsv_data_sha256
    global_config_skipped_jsonl_data_sha256
    global_non_universe_tsv_data_sha256
    global_non_universe_jsonl_data_sha256
)
full_pending=$(pending_keys "${full_receipt_fields[@]}")
full_pending_count=$(printf '%s\n' "$full_pending" | sed '/^$/d' \
    | wc -l | tr -d '[:space:]')
if [[ "$full_pending_count" != 0 \
    && "$full_pending_count" != "${#full_receipt_fields[@]}" ]]; then
    echo "error: R3bu full receipt is only partially PENDING" >&2
    exit 1
fi
if [[ "$full_pending_count" != 0 && "$mode" != bless-full ]]; then
    printf 'error: R3bu full baseline needs --bless-full: %s\n' \
        "$(tr '\n' ' ' <<<"$full_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
[[ "$full_pending_count" != 0 || "$mode" != bless-full ]] || mode=full

if [[ "$mode" == bless-full ]]; then
    cmp -s "$candidate_profile" "$live_profile" || {
        echo "error: R3bu can bless full receipts only while its candidate is live" >&2
        exit 1
    }
    expected_canonical_state=parent
else
    expected_canonical_state=candidate
fi

if [[ "$expected_canonical_state" == parent ]]; then
    canonical_runnable=$(read_value global_parent_full_runnable)
    canonical_passes=$(read_value global_parent_full_passes)
    canonical_tsv=$(read_value parent_full_tsv_sha256)
    canonical_jsonl=$(read_value parent_full_jsonl_sha256)
    canonical_summary=$(read_value global_parent_full_summary)
else
    canonical_runnable=$(read_value global_expected_candidate_full_runnable)
    canonical_passes=$(read_value global_expected_candidate_full_passes)
    canonical_tsv=$(read_value global_candidate_full_tsv_sha256)
    canonical_jsonl=$(read_value global_candidate_full_jsonl_sha256)
    canonical_summary=$(read_value global_expected_candidate_full_summary)
fi
[[ "$(read_value_from "$canonical_baseline" schema)" == "$(read_value schema)" \
    && "$(read_value_from "$canonical_baseline" timeout_ms)" \
        == "$(read_value timeout_ms)" \
    && "$(read_value_from "$canonical_baseline" variants)" \
        == "$(read_value global_full_variants)" \
    && "$(read_value_from "$canonical_baseline" runnable)" == "$canonical_runnable" \
    && "$(read_value_from "$canonical_baseline" passes)" == "$canonical_passes" \
    && "$(read_value_from "$canonical_baseline" tsv_sha256)" == "$canonical_tsv" \
    && "$(read_value_from "$canonical_baseline" jsonl_sha256)" == "$canonical_jsonl" \
    && "$(read_value_from "$canonical_baseline" summary)" == "$canonical_summary" ]] || {
    printf 'error: canonical full baseline is not the expected R3bu %s vector\n' \
        "$expected_canonical_state" >&2
    exit 1
}

rm -f -- "$parent_full_report" "$parent_full_json_report"
parent_full_output=$(run_full "$parent_profile" "$parent_full_report")
printf '%s\n' "$parent_full_output"
verify_report "$parent_full_report" "$parent_full_json_report" \
    "$(read_value parent_oxide_profile_sha256)" \
    "$(read_value global_full_variants)"
parent_full_keys=$(report_keys "$parent_full_report")
[[ "$(printf '%s\n' "$parent_full_keys" | sha256_stream)" \
        == "$(read_value global_full_keys_sha256)" \
    && "$(execution_runnable "$parent_full_output")" \
        == "$(read_value global_parent_full_runnable)" \
    && "$(report_outcome_count "$parent_full_report" '^pass$')" \
        == "$(read_value global_parent_full_passes)" \
    && "$(report_outcome_count "$parent_full_report" '^unsupported-feature$')" \
        == "$(read_value global_parent_full_unsupported_feature)" \
    && "$(report_summary "$parent_full_report")" \
        == "$(read_value global_parent_full_summary)" \
    && "$(sha256_file "$parent_full_report")" \
        == "$(read_value parent_full_tsv_sha256)" \
    && "$(sha256_file "$parent_full_json_report")" \
        == "$(read_value parent_full_jsonl_sha256)" ]] || {
    echo "error: R3bu authoritative parent full vector drifted" >&2
    exit 1
}

rm -f -- "$candidate_full_report" "$candidate_full_json_report"
candidate_full_output=$(run_full "$candidate_profile" "$candidate_full_report")
printf '%s\n' "$candidate_full_output"
verify_report "$candidate_full_report" "$candidate_full_json_report" \
    "$(read_value candidate_oxide_profile_sha256)" \
    "$(read_value global_full_variants)"
diff -u <(printf '%s\n' "$parent_full_keys") \
    <(report_keys "$candidate_full_report")
candidate_full_runnable=$(execution_runnable "$candidate_full_output")
candidate_full_passes=$(report_outcome_count "$candidate_full_report" '^pass$')
candidate_full_unsupported=$(report_outcome_count "$candidate_full_report" \
    '^unsupported-feature$')
candidate_full_summary=$(report_summary "$candidate_full_report")
candidate_full_tsv=$(sha256_file "$candidate_full_report")
candidate_full_jsonl=$(sha256_file "$candidate_full_json_report")
[[ "$candidate_full_runnable" \
        == "$(read_value global_expected_candidate_full_runnable)" \
    && "$candidate_full_passes" \
        == "$(read_value global_expected_candidate_full_passes)" \
    && "$candidate_full_unsupported" \
        == "$(read_value global_expected_candidate_full_unsupported_feature)" \
    && "$candidate_full_summary" \
        == "$(read_value global_expected_candidate_full_summary)" ]] || {
    echo "error: R3bu candidate full vector drifted" >&2
    exit 1
}

parent_full_activation=$(rows_for_paths "$activation_paths" "$parent_full_report")
candidate_full_activation=$(rows_for_paths "$activation_paths" "$candidate_full_report")
parent_full_reason=$(rows_for_paths "$reason_only_paths" "$parent_full_report")
candidate_full_reason=$(rows_for_paths "$reason_only_paths" "$candidate_full_report")
parent_full_config=$(rows_for_paths "$config_skipped_paths" "$parent_full_report")
candidate_full_config=$(rows_for_paths "$config_skipped_paths" "$candidate_full_report")
parent_full_universe=$(rows_for_paths "$universe_paths" "$parent_full_report")
candidate_full_universe=$(rows_for_paths "$universe_paths" "$candidate_full_report")
parent_non_universe=$(rows_without_paths "$universe_paths" "$parent_full_report")
candidate_non_universe=$(rows_without_paths "$universe_paths" "$candidate_full_report")

[[ "$(printf '%s\n' "$parent_full_universe" | wc -l | tr -d '[:space:]')" \
        == "$(read_value global_full_universe_rows)" \
    && "$(printf '%s\n' "$candidate_full_universe" | wc -l \
        | tr -d '[:space:]')" == "$(read_value global_full_universe_rows)" \
    && "$(printf '%s\n' "$parent_non_universe" | wc -l \
        | tr -d '[:space:]')" == "$(read_value global_full_non_universe_rows)" \
    && "$(printf '%s\n' "$candidate_non_universe" | wc -l \
        | tr -d '[:space:]')" == "$(read_value global_full_non_universe_rows)" ]] || {
    echo "error: R3bu full universe partition drifted" >&2
    exit 1
}
verify_partition_transition full-TSV activation "$parent_full_activation" \
    "$candidate_full_activation" "$(read_value global_full_activation_rows)"
verify_partition_transition full-TSV reason "$parent_full_reason" \
    "$candidate_full_reason" "$(read_value global_full_reason_only_rows)"
verify_partition_transition full-TSV config "$parent_full_config" \
    "$candidate_full_config" "$(read_value global_full_config_skipped_rows)"
diff -u <(report_rows "$parent_report") <(printf '%s\n' "$parent_full_universe")
diff -u <(report_rows "$candidate_report") \
    <(printf '%s\n' "$candidate_full_universe")
diff -u <(printf '%s\n' "$parent_non_universe") \
    <(printf '%s\n' "$candidate_non_universe")

join_counts=$(awk -F'\t' '
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
        if (before[key] == current) {
            unchanged++
        } else {
            changed++
            if (old[1] != $7) outcome_changed++
            else detail_only++
        }
        after[key]=1
        after_count++
    }
    END {
        if (before_count != after_count) exit 5
        for (key in before) if (!(key in after)) exit 6
        print before_count + 0, changed + 0, outcome_changed + 0,
            detail_only + 0, unchanged + 0, regressions + 0
    }
' "$parent_full_report" "$candidate_full_report") || {
    echo "error: R3bu complete keyed join failed" >&2
    exit 1
}
read -r full_rows changed_rows outcome_changed_rows detail_only_rows \
    unchanged_rows pass_regressions <<<"$join_counts"
[[ "$full_rows" == "$(read_value global_full_variants)" \
    && "$changed_rows" == "$(read_value global_full_changed_rows)" \
    && "$outcome_changed_rows" \
        == "$(read_value global_full_outcome_changed_rows)" \
    && "$detail_only_rows" == "$(read_value global_full_detail_only_rows)" \
    && "$unchanged_rows" == "$(read_value global_full_unchanged_rows)" \
    && "$pass_regressions" \
        == "$(read_value global_previous_pass_regressions)" ]] || {
    echo "error: R3bu full keyed join count drifted" >&2
    exit 1
}

parent_full_json_projection=$(json_result_projection "$parent_full_json_report")
candidate_full_json_projection=$(json_result_projection "$candidate_full_json_report")
diff -u <(report_rows "$parent_full_report") \
    <(printf '%s\n' "$parent_full_json_projection")
diff -u <(report_rows "$candidate_full_report") \
    <(printf '%s\n' "$candidate_full_json_projection")
parent_activation_json=$(projection_rows_for_paths "$activation_paths" \
    "$parent_full_json_projection")
candidate_activation_json=$(projection_rows_for_paths "$activation_paths" \
    "$candidate_full_json_projection")
parent_reason_json=$(projection_rows_for_paths "$reason_only_paths" \
    "$parent_full_json_projection")
candidate_reason_json=$(projection_rows_for_paths "$reason_only_paths" \
    "$candidate_full_json_projection")
parent_config_json=$(projection_rows_for_paths "$config_skipped_paths" \
    "$parent_full_json_projection")
candidate_config_json=$(projection_rows_for_paths "$config_skipped_paths" \
    "$candidate_full_json_projection")
parent_non_universe_json=$(projection_rows_without_paths "$universe_paths" \
    "$parent_full_json_projection")
candidate_non_universe_json=$(projection_rows_without_paths "$universe_paths" \
    "$candidate_full_json_projection")
verify_partition_transition full-JSONL activation "$parent_activation_json" \
    "$candidate_activation_json" "$(read_value global_full_activation_rows)"
verify_partition_transition full-JSONL reason "$parent_reason_json" \
    "$candidate_reason_json" "$(read_value global_full_reason_only_rows)"
verify_partition_transition full-JSONL config "$parent_config_json" \
    "$candidate_config_json" "$(read_value global_full_config_skipped_rows)"
diff -u <(printf '%s\n' "$parent_non_universe_json") \
    <(printf '%s\n' "$candidate_non_universe_json")
diff -u <(json_rows_for_paths "$universe_paths" "$parent_json_report") \
    <(json_rows_for_paths "$universe_paths" "$parent_full_json_report")
diff -u <(json_rows_for_paths "$universe_paths" "$candidate_json_report") \
    <(json_rows_for_paths "$universe_paths" "$candidate_full_json_report")
diff -u <(json_rows_without_paths "$universe_paths" "$parent_full_json_report") \
    <(json_rows_without_paths "$universe_paths" "$candidate_full_json_report")

activation_parent_tsv_sha=$(printf '%s\n' "$parent_full_activation" | sha256_stream)
activation_parent_json_sha=$(printf '%s\n' "$parent_activation_json" | sha256_stream)
activation_candidate_tsv_sha=$(printf '%s\n' "$candidate_full_activation" | sha256_stream)
activation_candidate_json_sha=$(printf '%s\n' "$candidate_activation_json" | sha256_stream)
reason_parent_tsv_sha=$(printf '%s\n' "$parent_full_reason" | sha256_stream)
reason_parent_json_sha=$(printf '%s\n' "$parent_reason_json" | sha256_stream)
reason_candidate_tsv_sha=$(printf '%s\n' "$candidate_full_reason" | sha256_stream)
reason_candidate_json_sha=$(printf '%s\n' "$candidate_reason_json" | sha256_stream)
config_tsv_sha=$(printf '%s\n' "$parent_full_config" | sha256_stream)
config_json_sha=$(printf '%s\n' "$parent_config_json" | sha256_stream)
non_universe_tsv_sha=$(printf '%s\n' "$parent_non_universe" | sha256_stream)
non_universe_json_sha=$(printf '%s\n' "$parent_non_universe_json" | sha256_stream)

if [[ "$mode" == bless-full ]]; then
    update_values "$baseline" \
        "global_candidate_full_tsv_sha256=$candidate_full_tsv" \
        "global_candidate_full_jsonl_sha256=$candidate_full_jsonl" \
        "global_activation_parent_tsv_data_sha256=$activation_parent_tsv_sha" \
        "global_activation_parent_jsonl_data_sha256=$activation_parent_json_sha" \
        "global_activation_candidate_tsv_data_sha256=$activation_candidate_tsv_sha" \
        "global_activation_candidate_jsonl_data_sha256=$activation_candidate_json_sha" \
        "global_reason_only_parent_tsv_data_sha256=$reason_parent_tsv_sha" \
        "global_reason_only_parent_jsonl_data_sha256=$reason_parent_json_sha" \
        "global_reason_only_candidate_tsv_data_sha256=$reason_candidate_tsv_sha" \
        "global_reason_only_candidate_jsonl_data_sha256=$reason_candidate_json_sha" \
        "global_config_skipped_tsv_data_sha256=$config_tsv_sha" \
        "global_config_skipped_jsonl_data_sha256=$config_json_sha" \
        "global_non_universe_tsv_data_sha256=$non_universe_tsv_sha" \
        "global_non_universe_jsonl_data_sha256=$non_universe_json_sha"
    update_values "$canonical_baseline" \
        "runnable=$candidate_full_runnable" \
        "passes=$candidate_full_passes" \
        "tsv_sha256=$candidate_full_tsv" \
        "jsonl_sha256=$candidate_full_jsonl" \
        "summary=$candidate_full_summary"
    printf 'R3bu full transition blessed: %s outcome + %s detail-only changes, %s rows unchanged, zero regressions\n' \
        "$outcome_changed_rows" "$detail_only_rows" "$unchanged_rows"
    printf 'Run ./scripts/test-test262-full.sh for the independent repeat.\n'
    exit 0
fi

for entry in \
    "global_candidate_full_tsv_sha256:$candidate_full_tsv" \
    "global_candidate_full_jsonl_sha256:$candidate_full_jsonl" \
    "global_activation_parent_tsv_data_sha256:$activation_parent_tsv_sha" \
    "global_activation_parent_jsonl_data_sha256:$activation_parent_json_sha" \
    "global_activation_candidate_tsv_data_sha256:$activation_candidate_tsv_sha" \
    "global_activation_candidate_jsonl_data_sha256:$activation_candidate_json_sha" \
    "global_reason_only_parent_tsv_data_sha256:$reason_parent_tsv_sha" \
    "global_reason_only_parent_jsonl_data_sha256:$reason_parent_json_sha" \
    "global_reason_only_candidate_tsv_data_sha256:$reason_candidate_tsv_sha" \
    "global_reason_only_candidate_jsonl_data_sha256:$reason_candidate_json_sha" \
    "global_config_skipped_tsv_data_sha256:$config_tsv_sha" \
    "global_config_skipped_jsonl_data_sha256:$config_json_sha" \
    "global_non_universe_tsv_data_sha256:$non_universe_tsv_sha" \
    "global_non_universe_jsonl_data_sha256:$non_universe_json_sha"
do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: R3bu full receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done

printf 'R3bu full transition is exact: %s outcome + %s detail-only changes, %s rows unchanged, zero regressions\n' \
    "$outcome_changed_rows" "$detail_only_rows" "$unchanged_rows"
