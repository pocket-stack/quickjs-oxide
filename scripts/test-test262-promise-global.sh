#!/usr/bin/env bash
# Reproduce the R3bq global Promise capability closure and exact full join.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-promise-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
parent_profile=tests/test262-promise-global-parent.conf
candidate_profile=tests/test262-promise-global-candidate.conf
live_profile=compat/test262-oxide.conf
manifest=tests/test262-promise-global.txt
activation_manifest=tests/test262-promise-global-activation.txt
reason_only_manifest=tests/test262-promise-global-reason-only.txt
transition_receipt=tests/test262-promise-global-transitions.tsv
before_report=target/test262-promise-global-before.tsv
before_json_report=target/test262-promise-global-before.jsonl
candidate_report=target/test262-promise-global-candidate.tsv
candidate_json_report=target/test262-promise-global-candidate.jsonl
before_full_report=target/test262-promise-global-before-full.tsv
before_full_json_report=target/test262-promise-global-before-full.jsonl
candidate_full_report=target/test262-promise-global-candidate-full.tsv
candidate_full_json_report=target/test262-promise-global-candidate-full.jsonl
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
transition_tmp=
r3bq_previous_exit_trap=
r3bq_trap_capture=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-r3bq-trap.XXXXXX")
trap -p EXIT >"$r3bq_trap_capture"
while IFS= read -r r3bq_trap_line || [[ -n "$r3bq_trap_line" ]]; do
    [[ -z "$r3bq_previous_exit_trap" ]] || r3bq_previous_exit_trap+=$'\n'
    r3bq_previous_exit_trap+=$r3bq_trap_line
done <"$r3bq_trap_capture"
rm -f -- "$r3bq_trap_capture"
unset r3bq_trap_capture r3bq_trap_line

r3bq_cleanup_transition_tmp() {
    [[ -z "$transition_tmp" ]] || rm -f -- "$transition_tmp"
}

r3bq_run_previous_exit_trap() {
    local status=$1 r3bq_previous_command= r3bq_had_errexit=false
    trap() {
        [[ ${1-} == -- && ${3-} == EXIT ]] || return 2
        r3bq_previous_command=$2
    }
    eval "$r3bq_previous_exit_trap"
    unset -f trap
    [[ $- != *e* ]] || r3bq_had_errexit=true
    set +e
    (exit "$status")
    eval "$r3bq_previous_command"
    [[ "$r3bq_had_errexit" != true ]] || set -e
    return 0
}

r3bq_exit_trap() {
    local status=$1
    trap - EXIT
    r3bq_cleanup_transition_tmp
    if [[ -n "$r3bq_previous_exit_trap" ]]; then
        r3bq_run_previous_exit_trap "$status"
    fi
    exit "$status"
}

trap 'r3bq_exit_trap "$?"' EXIT

usage() {
    printf 'usage: %s [--check|--bless|--full|--bless-full]\n' "${0##*/}"
    printf '  --check       verify frozen profiles, manifests, and set algebra only\n'
    printf '  --bless       bless tag reports and the 452-row transition receipt\n'
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
        printf 'error: R3bq baseline identity drifted for %s: %s != %s\n' \
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
    expected_hash=$(read_value "${name}_paths_sha256")
    actual_count=$(printf '%s\n' "$inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$inventory" | sed '/^$/d' | sha256_stream)
    [[ "$actual_count" == "$expected_count" && "$actual_hash" == "$expected_hash" ]] || {
        printf 'error: R3bq %s inventory drifted\n' "$name" >&2
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
        printf 'error: R3bq %s variant-key inventory drifted\n' "$name" >&2
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
            printf "error: R3bq JSONL report %s: %s\n", report, message >"/dev/stderr"
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

json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3bq JSONL projection %s: %s\n", report, message \
                >"/dev/stderr"
            failed=1
            exit 2
        }
        function expect(token) {
            if (substr(line, position, length(token)) != token) {
                fail("expected " token " at column " position)
            }
            position+=length(token)
        }
        function string_value(    start, character, escape, digits, value) {
            expect("\"")
            start=position
            while (position <= length(line)) {
                character=substr(line, position, 1)
                if (character == "\"") {
                    value=substr(line, start, position - start)
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
                        position+=5
                    } else {
                        if (index("\"\\/bfnrt", escape) == 0) {
                            fail("invalid string escape")
                        }
                        position++
                    }
                    continue
                }
                if (character == "\t" || character == "\r") {
                    fail("unescaped control character")
                }
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
                if (key != name[i]) {
                    fail("unexpected field " key " at position " i)
                }
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

verify_promise_partition_transition() {
    local encoding=$1 kind=$2 before_rows=$3 candidate_rows=$4 expected_rows=$5
    if ! awk -F'\t' -v kind="$kind" -v expected_rows="$expected_rows" '
        function before_shape_ok() {
            return $7 == "unsupported-feature" &&
                $8 == "selection" &&
                $9 == "EngineCapability"
        }
        NR == FNR {
            key=$1 SUBSEP $2
            if (key in before) exit 2
            if (!before_shape_ok()) exit 3
            if (kind == "activation") {
                if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise") {
                    promise++
                } else if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise, Promise.prototype.finally") {
                    promise_finally++
                } else if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise.allSettled") {
                    all_settled++
                } else if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise.any") {
                    any++
                } else if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise.prototype.finally") {
                    finally++
                } else {
                    exit 4
                }
            } else if (kind == "reason-class") {
                if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise.allSettled, class") {
                    all_settled_class++
                } else if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise.any, class") {
                    any_class++
                } else if ($10 == "quickjs-oxide does not declare Test262 feature support: Promise.prototype.finally, class") {
                    finally_class++
                } else {
                    exit 5
                }
            } else if (kind == "reason-computed") {
                if ($10 != "quickjs-oxide does not declare Test262 feature support: Promise.any, computed-property-names") {
                    exit 6
                }
                any_computed++
            } else {
                exit 7
            }
            for (i=1; i<=10; i++) {
                before_field[key, i]=$i
            }
            before[key]=1
            before_count++
            next
        }
        {
            key=$1 SUBSEP $2
            if (!(key in before) || key in after) exit 8
            for (i=1; i<=6; i++) {
                if ($i != before_field[key, i]) exit 9
            }
            if (kind == "activation") {
                if (!($7 == "pass" && $8 == "normal" &&
                    $9 == "" && $10 == "")) exit 10
            } else {
                if (kind == "reason-class") {
                    expected_detail="quickjs-oxide does not declare Test262 feature support: class"
                } else {
                    expected_detail="quickjs-oxide does not declare Test262 feature support: computed-property-names"
                }
                if (!($7 == "unsupported-feature" &&
                    $8 == "selection" &&
                    $9 == "EngineCapability" &&
                    $10 == expected_detail)) exit 11
            }
            after[key]=1
            after_count++
        }
        END {
            if (before_count != expected_rows || after_count != expected_rows) exit 12
            for (key in before) if (!(key in after)) exit 13
            if (kind == "activation" &&
                !(promise == 6 && promise_finally == 2 &&
                  all_settled == 200 && any == 154 && finally == 54)) exit 14
            if (kind == "reason-class" &&
                !(all_settled_class == 4 && any_class == 6 &&
                  finally_class == 2)) exit 15
            if (kind == "reason-computed" && any_computed != 24) exit 16
        }
    ' <(printf '%s\n' "$before_rows") <(printf '%s\n' "$candidate_rows"); then
        printf 'error: R3bq %s %s semantic transition drifted\n' \
            "$encoding" "$kind" >&2
        exit 1
    fi
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
        printf 'error: R3bq report metadata drifted: %s\n' "$report" >&2
        exit 1
    }
}

verify_json_report() {
    local json_report=$1 tsv_report=$2 expected_profile=$3 json_keys tsv_keys
    if ! json_keys=$(json_report_keys "$json_report"); then
        printf 'error: R3bq JSONL validation failed: %s\n' "$json_report" >&2
        exit 1
    fi
    if ! tsv_keys=$(report_keys "$tsv_report"); then
        printf 'error: R3bq TSV key extraction failed: %s\n' "$tsv_report" >&2
        exit 1
    fi
    diff -u <(printf '%s\n' "$tsv_keys") <(printf '%s\n' "$json_keys")
    [[ "$(head -n 1 "$json_report")" == "$(expected_json_metadata "$expected_profile")" \
        && "$(json_report_summary "$json_report")" == "$(report_summary "$tsv_report")" ]] || {
        printf 'error: R3bq JSONL metadata or summary drifted: %s\n' "$json_report" >&2
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
expect_value parent_profile tests/test262-promise-global-parent.conf
expect_value parent_profile_sha256 caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15
expect_value parent_features 84
expect_value candidate_profile tests/test262-promise-global-candidate.conf
expect_value candidate_profile_sha256 5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9
expect_value candidate_features 88
expect_value profile_negative_paths 828
expect_value profile_execution_entries 1
expect_value admitted_features 4
expect_value universe_paths 226
expect_value universe_variants 452
expect_value activation_paths 208
expect_value activation_variants 416
expect_value reason_only_paths 18
expect_value reason_only_variants 36
expect_value reason_only_class_variants 12
expect_value reason_only_computed_property_names_variants 24
expect_value metadata_features 14
expect_value metadata_includes 3
expect_value async_paths 134
expect_value async_variants 268
expect_value generated_paths 2
expect_value generated_variants 4
expect_value negative_paths 0
expect_value transition_rows 452
expect_value transition_activation_rows 416
expect_value transition_reason_only_rows 36
expect_value full_variants 102037
expect_value full_universe_rows 452
expect_value full_activation_rows 416
expect_value full_reason_only_rows 36
expect_value full_non_universe_rows 101585
expect_value full_changed_rows 452
expect_value full_outcome_changed_rows 416
expect_value full_detail_only_rows 36
expect_value previous_pass_regressions 0
expect_value before_full_runnable 58271
expect_value before_full_passes 57752
expect_value before_full_unsupported_feature 20373
expect_value expected_candidate_full_runnable 58687
expect_value expected_candidate_full_passes 58168
expect_value expected_candidate_full_unsupported_feature 19957

for required in \
    "$baseline" "$canonical_baseline" \
    "$parent_profile" "$candidate_profile" "$live_profile" \
    "$manifest" "$activation_manifest" "$reason_only_manifest"
do
    [[ -f "$required" ]] || {
        printf 'error: missing R3bq asset: %s\n' "$required" >&2
        exit 1
    }
done

admitted_features=$(
    printf '%s\n' \
        Promise \
        Promise.allSettled \
        Promise.any \
        Promise.prototype.finally
)
parent_features=$(profile_section "$parent_profile" features)
candidate_features=$(profile_section "$candidate_profile" features)
live_features=$(profile_section "$live_profile" features)
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
        == "$(read_value profile_execution_sha256)" \
    && "$(printf '%s\n' "$admitted_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value admitted_features)" \
    && "$(printf '%s\n' "$admitted_features" | sha256_stream)" \
        == "$(read_value admitted_features_sha256)" ]] || {
    echo "error: R3bq frozen profile identity drifted" >&2
    exit 1
}
diff -u <(printf '%s\n' "$parent_negatives") <(printf '%s\n' "$candidate_negatives")
diff -u <(printf '%s\n' "$parent_execution") <(printf '%s\n' "$candidate_execution")
diff -u \
    <(printf '%s\n' "$admitted_features") \
    <(comm -13 \
        <(printf '%s\n' "$parent_features") \
        <(printf '%s\n' "$candidate_features"))
[[ -z "$(comm -23 \
    <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))" ]] || {
    echo "error: R3bq candidate removed a frozen parent feature" >&2
    exit 1
}
[[ -z "$(comm -23 \
    <(printf '%s\n' "$candidate_features") \
    <(printf '%s\n' "$live_features"))" ]] || {
    echo "error: live profile removed an R3bq candidate feature" >&2
    exit 1
}
[[ -z "$(comm -23 \
    <(printf '%s\n' "$candidate_negatives") \
    <(printf '%s\n' "$live_negatives"))" ]] || {
    echo "error: live profile removed an R3bq candidate negative audit" >&2
    exit 1
}
diff -u <(printf '%s\n' "$candidate_execution") <(printf '%s\n' "$live_execution")
live_profile_sha256=$(sha256_file "$live_profile")
upstream_profile=$(
    awk -F'"' '$1 ~ /^oxide_profile_sha256 = / { print $2; found++ }
        END { if (found != 1) exit 1 }' compat/upstream.toml
)
[[ "$upstream_profile" == "$live_profile_sha256" ]] || {
    echo "error: compat/upstream.toml does not authenticate the live profile" >&2
    exit 1
}

universe_inventory=$(
    git -C "$suite" grep -l -E \
        'Promise(\.allSettled|\.any|\.prototype\.finally)?' -- 'test/**/*.js' \
        | while IFS= read -r test_path; do
            features=$(metadata_list "$test_path" features | LC_ALL=C sort)
            if [[ -n "$(comm -12 \
                    <(printf '%s\n' "$features") \
                    <(printf '%s\n' "$admitted_features"))" ]]; then
                printf '%s\n' "$test_path"
            fi
        done \
        | LC_ALL=C sort -u
)
verify_inventory universe "$universe_inventory"
verify_key_inventory universe "$universe_inventory"
diff -u <(printf '%s\n' "$universe_inventory") <(manifest_paths "$manifest")
manifest_paths "$manifest" | LC_ALL=C sort -c
[[ "$(sha256_file "$manifest")" == "$(read_value universe_manifest_sha256)" ]] || {
    echo "error: R3bq exhaustive manifest file drifted" >&2
    exit 1
}

activation_inventory=
reason_only_inventory=
reason_class_inventory=
reason_computed_inventory=
metadata_feature_inventory=
metadata_include_inventory=
async_inventory=
generated_inventory=
negative_inventory=
config_excluded_inventory=
config_exclusions=$(
    awk '
        $0 == "[exclude]" { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$source_dir/test262.conf"
)
while IFS= read -r test_path; do
    [[ -z "$test_path" ]] && continue
    metadata=$(metadata_block "$test_path")
    features=$(metadata_list "$test_path" features | LC_ALL=C sort)
    [[ -n "$(comm -12 \
            <(printf '%s\n' "$features") \
            <(printf '%s\n' "$admitted_features"))" ]] || {
        printf 'error: R3bq path lost every admitted Promise tag: %s\n' \
            "$test_path" >&2
        exit 1
    }
    missing=$(comm -23 \
        <(printf '%s\n' "$features") \
        <(printf '%s\n' "$candidate_features"))
    case "$missing" in
        "")
            activation_inventory+=$'\n'"$test_path"
            ;;
        class)
            reason_only_inventory+=$'\n'"$test_path"
            reason_class_inventory+=$'\n'"$test_path"
            ;;
        computed-property-names)
            reason_only_inventory+=$'\n'"$test_path"
            reason_computed_inventory+=$'\n'"$test_path"
            ;;
        *)
            printf 'error: R3bq unexpected residual feature set for %s: %s\n' \
                "$test_path" "$missing" >&2
            exit 1
            ;;
    esac
    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "" | "flags: []") ;;
        "flags: [async]") async_inventory+=$'\n'"$test_path" ;;
        "flags: [generated]") generated_inventory+=$'\n'"$test_path" ;;
        *)
            printf 'error: R3bq path lost its two-variant contract: %s: %s\n' \
                "$test_path" "$flag_line" >&2
            exit 1
            ;;
    esac
    if grep -Fq 'negative:' <<<"$metadata"; then
        negative_inventory+=$'\n'"$test_path"
    fi
    if grep -Eq '[$]262[.]([[:alnum:]_$]+)' < <(program_body "$test_path"); then
        printf 'error: R3bq tagged path unexpectedly requires a $262 host hook: %s\n' \
            "$test_path" >&2
        exit 1
    fi
    if grep -Fxq "test262/$test_path" <<<"$config_exclusions"; then
        config_excluded_inventory+=$'\n'"$test_path"
    fi
    metadata_feature_inventory+=$'\n'"$features"
    metadata_include_inventory+=$'\n'"$(metadata_list "$test_path" includes)"
done <<<"$universe_inventory"

activation_inventory=$(
    printf '%s\n' "$activation_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
reason_only_inventory=$(
    printf '%s\n' "$reason_only_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
reason_class_inventory=$(
    printf '%s\n' "$reason_class_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
reason_computed_inventory=$(
    printf '%s\n' "$reason_computed_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
async_inventory=$(printf '%s\n' "$async_inventory" | sed '/^$/d' | LC_ALL=C sort -u)
generated_inventory=$(
    printf '%s\n' "$generated_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
negative_inventory=$(
    printf '%s\n' "$negative_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
config_excluded_inventory=$(
    printf '%s\n' "$config_excluded_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
metadata_feature_inventory=$(
    printf '%s\n' "$metadata_feature_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
metadata_include_inventory=$(
    printf '%s\n' "$metadata_include_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)

verify_inventory activation "$activation_inventory"
verify_key_inventory activation "$activation_inventory"
verify_inventory reason_only "$reason_only_inventory"
verify_key_inventory reason_only "$reason_only_inventory"
diff -u <(printf '%s\n' "$activation_inventory") <(manifest_paths "$activation_manifest")
diff -u <(printf '%s\n' "$reason_only_inventory") <(manifest_paths "$reason_only_manifest")
for ledger in "$activation_manifest" "$reason_only_manifest"; do
    manifest_paths "$ledger" | LC_ALL=C sort -c
done
[[ "$(sha256_file "$activation_manifest")" \
        == "$(read_value activation_manifest_sha256)" \
    && "$(sha256_file "$reason_only_manifest")" \
        == "$(read_value reason_only_manifest_sha256)" ]] || {
    echo "error: R3bq partition ledger file drifted" >&2
    exit 1
}
diff -u \
    <(printf '%s\n' "$universe_inventory") \
    <(printf '%s\n%s\n' "$activation_inventory" "$reason_only_inventory" \
        | sed '/^$/d' | LC_ALL=C sort -u)

[[ "$(printf '%s\n' "$reason_class_inventory" | variant_keys | wc -l \
            | tr -d '[:space:]')" == "$(read_value reason_only_class_variants)" \
    && "$(printf '%s\n' "$reason_computed_inventory" | variant_keys | wc -l \
            | tr -d '[:space:]')" \
        == "$(read_value reason_only_computed_property_names_variants)" \
    && "$(printf '%s\n' "$metadata_feature_inventory" | wc -l \
            | tr -d '[:space:]')" == "$(read_value metadata_features)" \
    && "$(printf '%s\n' "$metadata_feature_inventory" | sha256_stream)" \
        == "$(read_value metadata_features_sha256)" \
    && "$(printf '%s\n' "$metadata_include_inventory" | wc -l \
            | tr -d '[:space:]')" == "$(read_value metadata_includes)" \
    && "$(printf '%s\n' "$metadata_include_inventory" | sha256_stream)" \
        == "$(read_value metadata_includes_sha256)" \
    && "$(printf '%s\n' "$async_inventory" | wc -l | tr -d '[:space:]')" \
        == "$(read_value async_paths)" \
    && "$(printf '%s\n' "$async_inventory" | variant_keys | wc -l \
            | tr -d '[:space:]')" == "$(read_value async_variants)" \
    && "$(printf '%s\n' "$generated_inventory" | wc -l | tr -d '[:space:]')" \
        == "$(read_value generated_paths)" \
    && "$(printf '%s\n' "$generated_inventory" | variant_keys | wc -l \
            | tr -d '[:space:]')" == "$(read_value generated_variants)" \
    && "$(printf '%s\n' "$negative_inventory" | sed '/^$/d' | wc -l \
            | tr -d '[:space:]')" == "$(read_value negative_paths)" \
    && -z "$config_excluded_inventory" ]] || {
    echo "error: R3bq metadata inventory drifted" >&2
    exit 1
}

run_promise_oracles() {
    if [[ "$mode" == check ]]; then
        "$script_dir/test-r3m-promise-jobs-oracle.sh" --check
        "$script_dir/test-r3o-promise-finally-oracle.sh" --check
        "$script_dir/test-r3q-promise-aggregates-oracle.sh" --check
        return
    fi
    cargo build --locked --release --quiet --bin qjs
    "$script_dir/test-r3m-promise-jobs-oracle.sh" --oxide "$root/target/release/qjs"
    "$script_dir/test-r3o-promise-finally-oracle.sh" --oxide "$root/target/release/qjs"
    "$script_dir/test-r3q-promise-aggregates-oracle.sh" --oxide "$root/target/release/qjs"
}
run_promise_oracles

if [[ "$mode" == check ]]; then
    printf 'R3bq inputs verified: %s tagged paths = %s activation + %s reason-only; parent %s -> candidate %s features\n' \
        "$(read_value universe_paths)" \
        "$(read_value activation_paths)" \
        "$(read_value reason_only_paths)" \
        "$(read_value parent_features)" \
        "$(read_value candidate_features)"
    exit 0
fi
tag_receipt_fields=(
    transition_receipt_sha256 transition_data_sha256
    before_tag_nonpass_sha256 before_tag_tsv_sha256 before_tag_jsonl_sha256
    candidate_tag_nonpass_sha256 candidate_tag_tsv_sha256 candidate_tag_jsonl_sha256
)
tag_pending=$(pending_keys "${tag_receipt_fields[@]}")
if [[ -n "$tag_pending" && "$mode" != bless ]]; then
    printf 'error: R3bq tag baseline needs --bless after implementation: %s\n' \
        "$(tr '\n' ' ' <<<"$tag_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ -n "$tag_pending" \
    && "$(printf '%s\n' "$tag_pending" | wc -l | tr -d '[:space:]')" \
        != "${#tag_receipt_fields[@]}" ]]; then
    echo "error: R3bq tag baseline is only partially PENDING" >&2
    exit 1
fi
if [[ -z "$tag_pending" && "$mode" == bless ]]; then
    mode=tag
fi

rm -f -- "$before_report" "$before_json_report"
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
verify_report_metadata \
    "$before_report" "$(read_value parent_profile_sha256)" "$(read_value universe_variants)"
verify_json_report \
    "$before_json_report" "$before_report" "$(read_value parent_profile_sha256)"
diff -u <(printf '%s\n' "$universe_inventory" | variant_keys) <(report_keys "$before_report")

before_runnable=$(execution_runnable "$before_output")
before_summary=$(report_summary "$before_report")
[[ "$before_runnable" == "$(read_value expected_before_tag_runnable)" \
    && "$before_summary" == "$(read_value expected_before_tag_summary)" ]] || {
    echo "error: R3bq parent tag count drifted" >&2
    exit 1
}
if ! report_rows "$before_report" | awk -F'\t' '
    $7 != "unsupported-feature" ||
    $8 != "selection" ||
    $9 != "EngineCapability" {
        exit 2
    }
    {
        detail[$10]++
    }
    END {
        if (NR != 452 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise"] != 6 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise, Promise.prototype.finally"] != 2 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.allSettled"] != 200 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.allSettled, class"] != 4 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.any"] != 154 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.any, class"] != 6 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.any, computed-property-names"] != 24 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.prototype.finally"] != 54 ||
            detail["quickjs-oxide does not declare Test262 feature support: Promise.prototype.finally, class"] != 2 ||
            length(detail) != 9) {
            exit 3
        }
    }
'; then
    echo "error: R3bq parent tag vector is not the exact frozen rejection set" >&2
    exit 1
fi

# Authenticate the immutable parent completely before executing the candidate.
rm -f -- "$candidate_report" "$candidate_json_report"
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
    "$candidate_report" "$(read_value candidate_profile_sha256)" \
    "$(read_value universe_variants)"
verify_json_report \
    "$candidate_json_report" "$candidate_report" "$(read_value candidate_profile_sha256)"
diff -u <(report_keys "$before_report") <(report_keys "$candidate_report")

candidate_runnable=$(execution_runnable "$candidate_output")
candidate_summary=$(report_summary "$candidate_report")
[[ "$candidate_runnable" == "$(read_value expected_candidate_tag_runnable)" \
    && "$candidate_summary" == "$(read_value expected_candidate_tag_summary)" ]] || {
    echo "error: R3bq candidate tag count drifted" >&2
    exit 1
}
activation_before_rows=$(rows_for_paths "$activation_inventory" "$before_report")
activation_candidate_rows=$(rows_for_paths "$activation_inventory" "$candidate_report")
reason_class_before_rows=$(rows_for_paths "$reason_class_inventory" "$before_report")
reason_class_candidate_rows=$(
    rows_for_paths "$reason_class_inventory" "$candidate_report"
)
reason_computed_before_rows=$(
    rows_for_paths "$reason_computed_inventory" "$before_report"
)
reason_computed_candidate_rows=$(
    rows_for_paths "$reason_computed_inventory" "$candidate_report"
)
verify_promise_partition_transition \
    TSV activation "$activation_before_rows" "$activation_candidate_rows" \
    "$(read_value activation_variants)"
verify_promise_partition_transition \
    TSV reason-class "$reason_class_before_rows" "$reason_class_candidate_rows" \
    "$(read_value reason_only_class_variants)"
verify_promise_partition_transition \
    TSV reason-computed "$reason_computed_before_rows" \
    "$reason_computed_candidate_rows" \
    "$(read_value reason_only_computed_property_names_variants)"

before_tag_json_projection=$(json_result_projection "$before_json_report")
candidate_tag_json_projection=$(json_result_projection "$candidate_json_report")
diff -u \
    <(report_rows "$before_report") \
    <(printf '%s\n' "$before_tag_json_projection")
diff -u \
    <(report_rows "$candidate_report") \
    <(printf '%s\n' "$candidate_tag_json_projection")
activation_before_json_projection=$(
    projection_rows_for_paths "$activation_inventory" "$before_tag_json_projection"
)
activation_candidate_json_projection=$(
    projection_rows_for_paths "$activation_inventory" "$candidate_tag_json_projection"
)
reason_class_before_json_projection=$(
    projection_rows_for_paths "$reason_class_inventory" "$before_tag_json_projection"
)
reason_class_candidate_json_projection=$(
    projection_rows_for_paths "$reason_class_inventory" "$candidate_tag_json_projection"
)
reason_computed_before_json_projection=$(
    projection_rows_for_paths "$reason_computed_inventory" "$before_tag_json_projection"
)
reason_computed_candidate_json_projection=$(
    projection_rows_for_paths "$reason_computed_inventory" \
        "$candidate_tag_json_projection"
)
verify_promise_partition_transition \
    JSONL activation "$activation_before_json_projection" \
    "$activation_candidate_json_projection" "$(read_value activation_variants)"
verify_promise_partition_transition \
    JSONL reason-class "$reason_class_before_json_projection" \
    "$reason_class_candidate_json_projection" \
    "$(read_value reason_only_class_variants)"
verify_promise_partition_transition \
    JSONL reason-computed "$reason_computed_before_json_projection" \
    "$reason_computed_candidate_json_projection" \
    "$(read_value reason_only_computed_property_names_variants)"

transition_tmp=$(mktemp "$transition_receipt.XXXXXX")
{
    printf '# R3bq exhaustive Promise global admission transition.\n'
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
transition_counts=$(
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            changed=0
            for (i=7; i<=10; i++) if ($i != $(i+4)) changed=1
            if (!changed) unchanged++
            if ($7 == "unsupported-feature" && $11 == "pass") activation++
            else if ($7 == "unsupported-feature" &&
                     $11 == "unsupported-feature" &&
                     $10 != $14) reason++
            else other++
        }
        END {
            print activation + 0, reason + 0, unchanged + 0, other + 0
        }
    ' "$transition_tmp"
)
[[ "$transition_rows_actual" == "$(read_value transition_rows)" \
    && "$transition_keys_actual" == "$(read_value universe_keys_sha256)" \
    && "$transition_counts" \
        == "$(read_value transition_activation_rows) $(read_value transition_reason_only_rows) 0 0" ]] || {
    echo "error: R3bq transition partition drifted" >&2
    exit 1
}

transition_sha=$(sha256_file "$transition_tmp")
transition_data_sha=$(
    awk '!/^#/ && !/^path\tvariant\t/' "$transition_tmp" | sha256_stream
)
before_nonpass=$(report_nonpass_sha256 "$before_report")
candidate_nonpass=$(report_nonpass_sha256 "$candidate_report")
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
        "before_tag_nonpass_sha256=$before_nonpass" \
        "before_tag_tsv_sha256=$before_tsv" \
        "before_tag_jsonl_sha256=$before_jsonl" \
        "candidate_tag_nonpass_sha256=$candidate_nonpass" \
        "candidate_tag_tsv_sha256=$candidate_tsv" \
        "candidate_tag_jsonl_sha256=$candidate_jsonl"
    printf 'R3bq tag baseline blessed: %s activation passes; %s reason-only variants remain fail-closed\n' \
        "$(read_value activation_variants)" "$(read_value reason_only_variants)"
    exit 0
fi

[[ -f "$transition_receipt" \
    && "$transition_sha" == "$(read_value transition_receipt_sha256)" \
    && "$transition_data_sha" == "$(read_value transition_data_sha256)" \
    && "$before_nonpass" == "$(read_value before_tag_nonpass_sha256)" \
    && "$before_tsv" == "$(read_value before_tag_tsv_sha256)" \
    && "$before_jsonl" == "$(read_value before_tag_jsonl_sha256)" \
    && "$candidate_nonpass" == "$(read_value candidate_tag_nonpass_sha256)" \
    && "$candidate_tsv" == "$(read_value candidate_tag_tsv_sha256)" \
    && "$candidate_jsonl" == "$(read_value candidate_tag_jsonl_sha256)" ]] || {
    echo "error: R3bq tag or transition receipt drifted" >&2
    exit 1
}
cmp -s "$transition_tmp" "$transition_receipt" || {
    echo "error: R3bq checked-in transition receipt drifted" >&2
    exit 1
}
rm -f -- "$transition_tmp"

if [[ "$mode" == tag ]]; then
    printf 'R3bq global Promise tag gate is exact: %s/%s activation variants pass; %s reason-only variants remain\n' \
        "$(read_value activation_variants)" \
        "$(read_value activation_variants)" \
        "$(read_value reason_only_variants)"
    exit 0
fi

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
    rm -f -- "$updates_tmp"
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
        expected_runnable=$(read_value expected_candidate_full_runnable)
        expected_passes=$(read_value expected_candidate_full_passes)
        expected_tsv=$(read_value candidate_full_tsv_sha256)
        expected_jsonl=$(read_value candidate_full_jsonl_sha256)
        expected_summary=$(read_value expected_candidate_full_summary)
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
        printf 'error: R3bq canonical full baseline is not the expected %s vector\n' \
            "$state" >&2
        exit 1
    }
}

full_receipt_fields=(
    candidate_full_tsv_sha256 candidate_full_jsonl_sha256
    full_activation_before_tsv_data_sha256
    full_activation_before_jsonl_data_sha256
    full_activation_candidate_tsv_data_sha256
    full_activation_candidate_jsonl_data_sha256
    full_reason_only_before_tsv_data_sha256
    full_reason_only_before_jsonl_data_sha256
    full_reason_only_candidate_tsv_data_sha256
    full_reason_only_candidate_jsonl_data_sha256
    full_non_universe_tsv_data_sha256
    full_non_universe_jsonl_data_sha256
)
full_pending=$(pending_keys "${full_receipt_fields[@]}")
full_pending_count=$(
    printf '%s\n' "$full_pending" | sed '/^$/d' | wc -l | tr -d '[:space:]'
)
if [[ "$full_pending_count" != 0 \
    && "$full_pending_count" != "${#full_receipt_fields[@]}" ]]; then
    echo "error: R3bq whole-corpus receipt is only partially PENDING" >&2
    exit 1
fi
if [[ "$full_pending_count" != 0 && "$mode" != bless-full ]]; then
    printf 'error: R3bq full baseline requires --bless-full after an exact join: %s\n' \
        "$(tr '\n' ' ' <<<"$full_pending" | sed 's/[[:space:]]*$//')" >&2
    exit 1
fi
if [[ "$full_pending_count" == 0 && "$mode" == bless-full ]]; then
    mode=full
fi

if [[ "$mode" == bless-full ]]; then
    cmp -s "$candidate_profile" "$live_profile" || {
        echo "error: R3bq can bless the canonical vector only while its candidate is live" >&2
        exit 1
    }
    verify_canonical_baseline parent
else
    verify_canonical_baseline candidate
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

rm -f -- "$before_full_report" "$before_full_json_report"
before_full_output=$(run_full "$parent_profile" "$before_full_report")
printf '%s\n' "$before_full_output"
verify_report_metadata \
    "$before_full_report" "$(read_value parent_profile_sha256)" \
    "$(read_value full_variants)"
verify_json_report \
    "$before_full_json_report" "$before_full_report" \
    "$(read_value parent_profile_sha256)"
before_full_keys=$(report_keys "$before_full_report")
[[ "$(printf '%s\n' "$before_full_keys" | sha256_stream)" \
        == "$(read_value full_keys_sha256)" ]] || {
    echo "error: R3bq complete Test262 key inventory drifted" >&2
    exit 1
}
before_full_runnable=$(execution_runnable "$before_full_output")
before_full_passes=$(report_outcome_count "$before_full_report" '^pass$')
before_full_unsupported=$(
    report_outcome_count "$before_full_report" '^unsupported-feature$'
)
before_full_summary=$(report_summary "$before_full_report")
before_full_tsv=$(sha256_file "$before_full_report")
before_full_jsonl=$(sha256_file "$before_full_json_report")
[[ "$before_full_runnable" == "$(read_value before_full_runnable)" \
    && "$before_full_passes" == "$(read_value before_full_passes)" \
    && "$before_full_unsupported" \
        == "$(read_value before_full_unsupported_feature)" \
    && "$before_full_summary" == "$(read_value before_full_summary)" \
    && "$before_full_tsv" == "$(read_value before_full_tsv_sha256)" \
    && "$before_full_jsonl" == "$(read_value before_full_jsonl_sha256)" ]] || {
    echo "error: R3bq authoritative historical parent full vector drifted" >&2
    exit 1
}

# Do not execute the candidate until every parent identity, key, count, and byte
# receipt above has authenticated the immutable historical vector.
rm -f -- "$candidate_full_report" "$candidate_full_json_report"
candidate_full_output=$(run_full "$candidate_profile" "$candidate_full_report")
printf '%s\n' "$candidate_full_output"
verify_report_metadata \
    "$candidate_full_report" "$(read_value candidate_profile_sha256)" \
    "$(read_value full_variants)"
verify_json_report \
    "$candidate_full_json_report" "$candidate_full_report" \
    "$(read_value candidate_profile_sha256)"
candidate_full_keys=$(report_keys "$candidate_full_report")
diff -u \
    <(printf '%s\n' "$before_full_keys") \
    <(printf '%s\n' "$candidate_full_keys")
candidate_full_runnable=$(execution_runnable "$candidate_full_output")
candidate_full_passes=$(report_outcome_count "$candidate_full_report" '^pass$')
candidate_full_unsupported=$(
    report_outcome_count "$candidate_full_report" '^unsupported-feature$'
)
candidate_full_summary=$(report_summary "$candidate_full_report")
candidate_full_tsv=$(sha256_file "$candidate_full_report")
candidate_full_jsonl=$(sha256_file "$candidate_full_json_report")
[[ "$candidate_full_runnable" == "$(read_value expected_candidate_full_runnable)" \
    && "$candidate_full_passes" == "$(read_value expected_candidate_full_passes)" \
    && "$candidate_full_unsupported" \
        == "$(read_value expected_candidate_full_unsupported_feature)" \
    && "$candidate_full_summary" \
        == "$(read_value expected_candidate_full_summary)" ]] || {
    echo "error: R3bq candidate full summary or admission counts drifted" >&2
    exit 1
}

universe_paths=$(manifest_paths "$manifest")
activation_paths=$(manifest_paths "$activation_manifest")
reason_only_paths=$(manifest_paths "$reason_only_manifest")
before_activation_rows=$(rows_for_paths "$activation_paths" "$before_full_report")
candidate_activation_rows=$(
    rows_for_paths "$activation_paths" "$candidate_full_report"
)
before_reason_rows=$(rows_for_paths "$reason_only_paths" "$before_full_report")
candidate_reason_rows=$(rows_for_paths "$reason_only_paths" "$candidate_full_report")
before_reason_class_rows=$(
    rows_for_paths "$reason_class_inventory" "$before_full_report"
)
candidate_reason_class_rows=$(
    rows_for_paths "$reason_class_inventory" "$candidate_full_report"
)
before_reason_computed_rows=$(
    rows_for_paths "$reason_computed_inventory" "$before_full_report"
)
candidate_reason_computed_rows=$(
    rows_for_paths "$reason_computed_inventory" "$candidate_full_report"
)
before_universe_rows=$(rows_for_paths "$universe_paths" "$before_full_report")
candidate_universe_rows=$(rows_for_paths "$universe_paths" "$candidate_full_report")
before_non_universe_rows=$(rows_without_paths "$universe_paths" "$before_full_report")
candidate_non_universe_rows=$(
    rows_without_paths "$universe_paths" "$candidate_full_report"
)

[[ "$(printf '%s\n' "$before_activation_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_activation_rows)" \
    && "$(printf '%s\n' "$candidate_activation_rows" | wc -l \
            | tr -d '[:space:]')" == "$(read_value full_activation_rows)" \
    && "$(printf '%s\n' "$before_reason_rows" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_reason_only_rows)" \
    && "$(printf '%s\n' "$candidate_reason_rows" | wc -l \
            | tr -d '[:space:]')" == "$(read_value full_reason_only_rows)" \
    && "$(printf '%s\n' "$before_universe_rows" | wc -l \
            | tr -d '[:space:]')" == "$(read_value full_universe_rows)" \
    && "$(printf '%s\n' "$candidate_universe_rows" | wc -l \
            | tr -d '[:space:]')" == "$(read_value full_universe_rows)" \
    && "$(printf '%s\n' "$before_non_universe_rows" | wc -l \
            | tr -d '[:space:]')" == "$(read_value full_non_universe_rows)" \
    && "$(printf '%s\n' "$candidate_non_universe_rows" | wc -l \
            | tr -d '[:space:]')" == "$(read_value full_non_universe_rows)" ]] || {
    echo "error: R3bq full activation/reason/non-universe partition drifted" >&2
    exit 1
}
verify_promise_partition_transition \
    full-TSV activation "$before_activation_rows" "$candidate_activation_rows" \
    "$(read_value full_activation_rows)"
verify_promise_partition_transition \
    full-TSV reason-class "$before_reason_class_rows" \
    "$candidate_reason_class_rows" "$(read_value reason_only_class_variants)"
verify_promise_partition_transition \
    full-TSV reason-computed "$before_reason_computed_rows" \
    "$candidate_reason_computed_rows" \
    "$(read_value reason_only_computed_property_names_variants)"
diff -u <(report_rows "$before_report") <(printf '%s\n' "$before_universe_rows")
diff -u \
    <(report_rows "$candidate_report") \
    <(printf '%s\n' "$candidate_universe_rows")
diff -u \
    <(printf '%s\n' "$before_non_universe_rows") \
    <(printf '%s\n' "$candidate_non_universe_rows")

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
            if (before[key] == current) {
                unchanged++
            } else {
                changed++
                if (old[1] != $7) {
                    outcome_changed++
                    if (!(old[1] == "unsupported-feature" &&
                        old[2] == "selection" &&
                        old[3] == "EngineCapability" &&
                        $7 == "pass" && $8 == "normal" &&
                        $9 == "" && $10 == "")) exit 5
                } else {
                    detail_only++
                    if (!(old[1] == "unsupported-feature" &&
                        old[2] == "selection" &&
                        old[3] == "EngineCapability" &&
                        $7 == "unsupported-feature" &&
                        $8 == "selection" &&
                        $9 == "EngineCapability" &&
                        ($10 == "quickjs-oxide does not declare Test262 feature support: class" ||
                         $10 == "quickjs-oxide does not declare Test262 feature support: computed-property-names"))) {
                        exit 6
                    }
                }
            }
            after[key]=1
            after_count++
        }
        END {
            if (before_count != after_count) exit 7
            for (key in before) if (!(key in after)) exit 8
            print before_count + 0, changed + 0, outcome_changed + 0,
                detail_only + 0, unchanged + 0, regressions + 0
        }
    ' "$before_full_report" "$candidate_full_report"
) || {
    echo "error: R3bq complete before/after keyed join failed" >&2
    exit 1
}
read -r full_rows changed_rows outcome_changed_rows detail_only_rows \
    unchanged_rows previous_pass_regressions <<<"$join_counts"
[[ "$full_rows" == "$(read_value full_variants)" \
    && "$changed_rows" == "$(read_value full_changed_rows)" \
    && "$outcome_changed_rows" == "$(read_value full_outcome_changed_rows)" \
    && "$detail_only_rows" == "$(read_value full_detail_only_rows)" \
    && "$unchanged_rows" == "$(read_value full_non_universe_rows)" \
    && "$previous_pass_regressions" \
        == "$(read_value previous_pass_regressions)" ]] || {
    echo "error: R3bq full join is not exactly 416 outcome changes, 36 detail-only changes, and zero pass regressions" >&2
    exit 1
}

before_universe_json_rows=$(
    json_rows_for_paths "$universe_paths" "$before_full_json_report"
)
candidate_universe_json_rows=$(
    json_rows_for_paths "$universe_paths" "$candidate_full_json_report"
)
before_non_universe_json_rows=$(
    json_rows_without_paths "$universe_paths" "$before_full_json_report"
)
candidate_non_universe_json_rows=$(
    json_rows_without_paths "$universe_paths" "$candidate_full_json_report"
)
diff -u \
    <(json_rows_for_paths "$universe_paths" "$before_json_report") \
    <(printf '%s\n' "$before_universe_json_rows")
diff -u \
    <(json_rows_for_paths "$universe_paths" "$candidate_json_report") \
    <(printf '%s\n' "$candidate_universe_json_rows")
diff -u \
    <(printf '%s\n' "$before_non_universe_json_rows") \
    <(printf '%s\n' "$candidate_non_universe_json_rows")

before_full_json_projection=$(json_result_projection "$before_full_json_report")
candidate_full_json_projection=$(json_result_projection "$candidate_full_json_report")
before_activation_json_projection=$(
    projection_rows_for_paths "$activation_paths" "$before_full_json_projection"
)
candidate_activation_json_projection=$(
    projection_rows_for_paths "$activation_paths" "$candidate_full_json_projection"
)
before_reason_json_projection=$(
    projection_rows_for_paths "$reason_only_paths" "$before_full_json_projection"
)
candidate_reason_json_projection=$(
    projection_rows_for_paths "$reason_only_paths" "$candidate_full_json_projection"
)
before_reason_class_json_projection=$(
    projection_rows_for_paths "$reason_class_inventory" "$before_full_json_projection"
)
candidate_reason_class_json_projection=$(
    projection_rows_for_paths "$reason_class_inventory" \
        "$candidate_full_json_projection"
)
before_reason_computed_json_projection=$(
    projection_rows_for_paths "$reason_computed_inventory" \
        "$before_full_json_projection"
)
candidate_reason_computed_json_projection=$(
    projection_rows_for_paths "$reason_computed_inventory" \
        "$candidate_full_json_projection"
)
before_universe_json_projection=$(
    projection_rows_for_paths "$universe_paths" "$before_full_json_projection"
)
candidate_universe_json_projection=$(
    projection_rows_for_paths "$universe_paths" "$candidate_full_json_projection"
)
before_non_universe_json_projection=$(
    projection_rows_without_paths "$universe_paths" "$before_full_json_projection"
)
candidate_non_universe_json_projection=$(
    projection_rows_without_paths "$universe_paths" "$candidate_full_json_projection"
)
diff -u \
    <(printf '%s\n' "$before_tag_json_projection") \
    <(printf '%s\n' "$before_universe_json_projection")
diff -u \
    <(printf '%s\n' "$candidate_tag_json_projection") \
    <(printf '%s\n' "$candidate_universe_json_projection")
diff -u \
    <(printf '%s\n' "$before_non_universe_json_projection") \
    <(printf '%s\n' "$candidate_non_universe_json_projection")
verify_promise_partition_transition \
    full-JSONL activation "$before_activation_json_projection" \
    "$candidate_activation_json_projection" "$(read_value full_activation_rows)"
verify_promise_partition_transition \
    full-JSONL reason-class "$before_reason_class_json_projection" \
    "$candidate_reason_class_json_projection" \
    "$(read_value reason_only_class_variants)"
verify_promise_partition_transition \
    full-JSONL reason-computed "$before_reason_computed_json_projection" \
    "$candidate_reason_computed_json_projection" \
    "$(read_value reason_only_computed_property_names_variants)"

activation_before_tsv_sha=$(printf '%s\n' "$before_activation_rows" | sha256_stream)
activation_before_json_sha=$(
    printf '%s\n' "$before_activation_json_projection" | sha256_stream
)
activation_candidate_tsv_sha=$(
    printf '%s\n' "$candidate_activation_rows" | sha256_stream
)
activation_candidate_json_sha=$(
    printf '%s\n' "$candidate_activation_json_projection" | sha256_stream
)
reason_before_tsv_sha=$(printf '%s\n' "$before_reason_rows" | sha256_stream)
reason_before_json_sha=$(
    printf '%s\n' "$before_reason_json_projection" | sha256_stream
)
reason_candidate_tsv_sha=$(printf '%s\n' "$candidate_reason_rows" | sha256_stream)
reason_candidate_json_sha=$(
    printf '%s\n' "$candidate_reason_json_projection" | sha256_stream
)
non_universe_tsv_sha=$(
    printf '%s\n' "$before_non_universe_rows" | sha256_stream
)
non_universe_json_sha=$(
    printf '%s\n' "$before_non_universe_json_projection" | sha256_stream
)

if [[ "$mode" == bless-full ]]; then
    update_values "$baseline" \
        "candidate_full_tsv_sha256=$candidate_full_tsv" \
        "candidate_full_jsonl_sha256=$candidate_full_jsonl" \
        "full_activation_before_tsv_data_sha256=$activation_before_tsv_sha" \
        "full_activation_before_jsonl_data_sha256=$activation_before_json_sha" \
        "full_activation_candidate_tsv_data_sha256=$activation_candidate_tsv_sha" \
        "full_activation_candidate_jsonl_data_sha256=$activation_candidate_json_sha" \
        "full_reason_only_before_tsv_data_sha256=$reason_before_tsv_sha" \
        "full_reason_only_before_jsonl_data_sha256=$reason_before_json_sha" \
        "full_reason_only_candidate_tsv_data_sha256=$reason_candidate_tsv_sha" \
        "full_reason_only_candidate_jsonl_data_sha256=$reason_candidate_json_sha" \
        "full_non_universe_tsv_data_sha256=$non_universe_tsv_sha" \
        "full_non_universe_jsonl_data_sha256=$non_universe_json_sha"
    update_values "$canonical_baseline" \
        "runnable=$candidate_full_runnable" \
        "passes=$candidate_full_passes" \
        "tsv_sha256=$candidate_full_tsv" \
        "jsonl_sha256=$candidate_full_jsonl" \
        "summary=$candidate_full_summary"
    printf 'R3bq full transition blessed: %s outcome changes, %s detail-only changes, %s unchanged; zero previous-pass regressions\n' \
        "$outcome_changed_rows" "$detail_only_rows" "$unchanged_rows"
    printf 'Run ./scripts/test-test262-full.sh for the independent canonical repeat.\n'
    exit 0
fi

for entry in \
    "candidate_full_tsv_sha256:$candidate_full_tsv" \
    "candidate_full_jsonl_sha256:$candidate_full_jsonl" \
    "full_activation_before_tsv_data_sha256:$activation_before_tsv_sha" \
    "full_activation_before_jsonl_data_sha256:$activation_before_json_sha" \
    "full_activation_candidate_tsv_data_sha256:$activation_candidate_tsv_sha" \
    "full_activation_candidate_jsonl_data_sha256:$activation_candidate_json_sha" \
    "full_reason_only_before_tsv_data_sha256:$reason_before_tsv_sha" \
    "full_reason_only_before_jsonl_data_sha256:$reason_before_json_sha" \
    "full_reason_only_candidate_tsv_data_sha256:$reason_candidate_tsv_sha" \
    "full_reason_only_candidate_jsonl_data_sha256:$reason_candidate_json_sha" \
    "full_non_universe_tsv_data_sha256:$non_universe_tsv_sha" \
    "full_non_universe_jsonl_data_sha256:$non_universe_json_sha"
do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: R3bq full receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done
verify_canonical_baseline candidate

printf 'R3bq full transition is exact: %s outcome changes, %s detail-only changes, %s unchanged; zero previous-pass regressions\n' \
    "$outcome_changed_rows" "$detail_only_rows" "$unchanged_rows"
