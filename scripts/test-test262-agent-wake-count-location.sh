#!/usr/bin/env bash
# Authenticate the R3dp scoped Test262 agent wake/count/location receipt.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-agent-wake-count-location-baseline.txt
upstream=compat/upstream.toml
predecessor_gate=scripts/test-test262-agent-wait-bounded-a.sh
predecessor_baseline=tests/test262-agent-wait-bounded-a-baseline.txt
predecessor_retained=tests/test262-agent-wait-bounded-a-retained.txt
predecessor_profile=tests/test262-agent-wait-bounded-a-candidate.conf
universe=tests/test262-agent-wake-count-location-universe.txt
activation=tests/test262-agent-wake-count-location.txt
retained=tests/test262-agent-wake-count-location-retained.txt
parent_profile=tests/test262-agent-wake-count-location-parent.conf
candidate_profile=tests/test262-agent-wake-count-location-candidate.conf
parent_report=tests/test262-agent-wake-count-location-parent.tsv
candidate_report=tests/test262-agent-wake-count-location-candidate.tsv
transition=tests/test262-agent-wake-count-location-transitions.tsv
quickjs_receipt=tests/test262-agent-wake-count-location-quickjs-receipt.txt
parallel_workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  replay the exact R3dp scoped wake/count/location receipt\n'
}

case ${1-} in
    ''|--check) ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$parallel_workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid Test262 worker count' >&2; exit 2; }
[[ -z ${TEST262_RUNNER+x} ]] \
    || { echo 'error: TEST262_RUNNER override is forbidden for R3dp wake/count/location' >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$baseline"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
check_file() {
    [[ -f "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated input drifted: $1"
}
profile_section() {
    awk -v wanted="[$1]" '
        $0==wanted{inside=1;next} /^\[/{inside=0}
        inside&&NF&&$1!~/^#/{print}
    ' "$2"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
json_result_rows() { awk '/^\{"kind":"result"/' "$1"; }
json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3dp JSONL projection %s: %s\n", report, message \
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
                        value=value "\\u" digits
                        position+=5
                    } else {
                        if (index("\"\\/bfnrt", escape) == 0) {
                            fail("invalid string escape")
                        }
                        if (escape == "\"") value=value "\""
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
                if (key != name[i]) fail("unexpected field " key " at position " i)
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
verify_json_tsv_projection() {
    local report=$1 json=$2 label=$3
    json_result_projection "$json" >"$tmp/$label.json-result-rows.tsv" \
        || die "R3dp JSONL result projection failed: $json"
    report_rows "$report" >"$tmp/$label.tsv-result-rows.tsv"
    diff -u "$tmp/$label.tsv-result-rows.tsv" "$tmp/$label.json-result-rows.tsv" \
        || die "R3dp JSONL/TSV ten-field projection drifted: $json"
}
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
select_manifest_rows() {
    awk -F'\t' 'NR==FNR{wanted[$0]=1;next} $1 in wanted' "$1" "$2"
}
fixed_count() { grep -Fo -- "$1" "$2" | lines /dev/stdin; }
toml_test262_value() {
    awk -v wanted="$1" '
        $0=="[test262]"{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1);gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted)next
            answer=substr($0,separator+1);gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$upstream"
}

verify_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    verify_json_tsv_projection "$report" "$json" "$label"
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == both \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value universe_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value report_keys_sha256)" \
        && "$(lines "$tmp/$label.json-result-rows.tsv")" == "$(value universe_variants)" \
        && "$(awk -F'\t' '{print $1 "\t" $2}' "$tmp/$label.json-result-rows.tsv" | sort | sha /dev/stdin)" == "$(value report_keys_sha256)" \
        && "$(awk -F'\t' '{print $1 "\t" $2}' "$tmp/$label.json-result-rows.tsv" | sort -u | lines /dev/stdin)" == "$(value universe_variants)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "scoped report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive Test262 agent wake/count/location transition.'
        echo '# milestone=R3dp'
        echo "# parent_commit=$(value parent_commit)"
        echo "# parent_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_profile_sha256=$(value candidate_profile_sha256)"
        echo "# universe_sha256=$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{
                if(!/^#/&&!($1=="path"&&$2=="variant")){key=$1 FS $2;old[key]=$0}
                next
            }
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old)||key in seen)exit 2
                split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 4}
        ' "$before" "$after"
    } >"$output"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dp-scoped.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM

check_file "$baseline" 113 \
    6780d507d3449e81f3f7285e336846b9ba6b4117121eb51b812d91068dce71b2
[[ "$(value milestone)" == R3dp \
    && "$(value milestone_kind)" == scoped-runner-admission \
    && "$(value scope_semantics)" == notify-wake-count-and-waiter-location \
    && "$(value runtime_waiter_changes)" == none \
    && "$(value global_promotion)" == false \
    && "$(value profile_selection)" == exact-universe-or-all \
    && "$(value submanifest_selection)" == fail-closed \
    && "$(value single_test_selection)" == fail-closed \
    && "$(value coordinator_revalidation)" == exact-path-source-sha256-metadata \
    && "$(value worker_revalidation)" == exact-path-source-sha256-metadata \
    && "$(value predecessor_gate_replayed)" == true \
    && "$parallel_workers" == "$(value parallel_workers)" ]] \
    || die 'R3dp scoped admission boundary drifted'

[[ -f "$upstream" ]] || die "missing pinned upstream metadata: $upstream"
check_file "$predecessor_gate" "$(value predecessor_gate_lines)" \
    "$(value predecessor_gate_sha256)"
check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
    "$(value predecessor_baseline_sha256)"
check_file "$predecessor_retained" "$(value predecessor_retained_lines)" \
    "$(value predecessor_retained_sha256)"
check_file "$predecessor_profile" "$(value predecessor_profile_lines)" \
    "$(value predecessor_profile_sha256)"
check_file "$universe" "$(value universe_paths)" "$(value universe_sha256)"
check_file "$activation" "$(value activation_paths)" "$(value activation_sha256)"
check_file "$retained" "$(value retained_paths)" "$(value retained_sha256)"
check_file "$parent_profile" "$(value parent_profile_lines)" \
    "$(value parent_profile_sha256)"
check_file "$candidate_profile" "$(value candidate_profile_lines)" \
    "$(value candidate_profile_sha256)"
check_file "$parent_report" "$(value parent_tsv_lines)" "$(value parent_tsv_sha256)"
check_file "${parent_report%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
    "$(value parent_jsonl_sha256)"
check_file "$candidate_report" "$(value candidate_tsv_lines)" \
    "$(value candidate_tsv_sha256)"
check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
    "$(value candidate_jsonl_sha256)"
check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
    "$(value quickjs_receipt_sha256)"

for file in "$universe" "$activation" "$retained"; do
    sort -c "$file" || die "manifest is not bytewise sorted: $file"
    [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
done
diff -u "$predecessor_retained" "$universe"
sort -u "$activation" "$retained" >"$tmp/partition.txt"
diff -u "$universe" "$tmp/partition.txt"
[[ -z "$(comm -12 "$activation" "$retained")" ]] \
    || die 'wake/count/location activation and retained partitions overlap'

for section in features audited-negative-tests host-agent-tests; do
    profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
    profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
done
diff -u "$predecessor_profile" "$parent_profile"
diff -u "$tmp/parent.features" "$tmp/candidate.features"
diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
[[ "$(lines "$tmp/parent.features")" == "$(value profile_features)" \
    && "$(sha "$tmp/parent.features")" == "$(value profile_features_sha256)" \
    && "$(lines "$tmp/parent.audited-negative-tests")" == "$(value profile_audited_negatives)" \
    && "$(sha "$tmp/parent.audited-negative-tests")" == "$(value profile_audited_negatives_sha256)" \
    && "$(lines "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_paths)" \
    && "$(sha "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_sha256)" \
    && "$(lines "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_paths)" \
    && "$(sha "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_sha256)" ]] \
    || die 'wake/count/location scoped profile inventory drifted'
[[ -z "$(comm -23 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests")" ]] \
    || die 'wake/count/location candidate removed a historical agent admission'
comm -13 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
    >"$tmp/agent-allowlist-delta.txt"
check_file "$tmp/agent-allowlist-delta.txt" "$(value agent_allowlist_delta_paths)" \
    "$(value agent_allowlist_delta_sha256)"
diff -u "$activation" "$tmp/agent-allowlist-delta.txt"

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
: >"$tmp/source-ledger.tsv"
: >"$tmp/metadata-ledger.tsv"
notify_location_paths=0
notify_default_count_paths=0
notify_explicit_count_paths=0
notify_default_index_paths=0
wait_infinite_timeout_paths=0
wait_location_paths=0
wait_before_timeout_paths=0
wait_default_index_paths=0
while IFS= read -r test_path; do
    source=$suite/$test_path
    [[ -f "$source" ]] || die "pinned activation source is missing: $test_path"
    printf '%s\t%s\n' "$test_path" "$(sha "$source")" >>"$tmp/source-ledger.tsv"
    features=$(sed -n 's/^features: \[\(.*\)\]$/\1/p' "$source" | tr -d '[:space:]')
    includes=$(sed -n 's/^includes: \[\(.*\)\]$/\1/p' "$source" | tr -d '[:space:]')
    [[ -n "$features" && "$includes" == atomicsHelper.js \
        && "$(grep -Ec '^flags:|^negative:' "$source")" == 0 ]] \
        || die "wake/count/location metadata shape drifted: $test_path"
    printf '%s\tflags=-\tfeatures=%s\tincludes=%s\tnegative=-\n' \
        "$test_path" "$features" "$includes" >>"$tmp/metadata-ledger.tsv"
    for token in '$262.agent.start(' '$262.agent.receiveBroadcast(' \
        '$262.agent.safeBroadcast(' '$262.agent.waitUntil(' \
        '$262.agent.leaving(' 'Atomics.wait(' 'Atomics.notify(' \
        'new SharedArrayBuffer('; do
        grep -Fq "$token" "$source" \
            || die "wake/count/location source shape drifted ($token): $test_path"
    done
    ! grep -Fq '$262.agent.broadcast(' "$source" \
        || die "direct broadcast escaped the scoped source boundary: $test_path"
    compact=$(tr -d '[:space:]' <"$source")
    case $test_path in
        test/built-ins/Atomics/notify/bigint/notify-all-on-loc.js)
            notify_location_paths=$((notify_location_paths + 1))
            [[ "$compact" == *'constWAIT_FAKE=1;'* \
                && "$compact" == *'Atomics.notify(i64a,WAIT_INDEX),NUMAGENT'* \
                && "$compact" == *'Btimed-out'* ]] \
                || die "BigInt notify-location semantics drifted"
            ;;
        test/built-ins/Atomics/notify/notify-all-on-loc.js)
            notify_location_paths=$((notify_location_paths + 1))
            [[ "$compact" == *'constWAIT_FAKE=1;'* \
                && "$compact" == *'Atomics.notify(i32a,WAIT_INDEX),NUMAGENT'* \
                && "$compact" == *'Btimed-out'* ]] \
                || die "Int32 notify-location semantics drifted"
            ;;
        test/built-ins/Atomics/notify/count-defaults-to-infinity-missing.js)
            notify_default_count_paths=$((notify_default_count_paths + 1))
            [[ "$compact" == *'constNUMAGENT=4;'* \
                && "$compact" == *'Atomics.notify(i32a,WAIT_INDEX/*,countmissing*/),NUMAGENT'* ]] \
                || die "missing notify-count semantics drifted"
            ;;
        test/built-ins/Atomics/notify/count-defaults-to-infinity-undefined.js)
            notify_default_count_paths=$((notify_default_count_paths + 1))
            [[ "$compact" == *'constNUMAGENT=4;'* \
                && "$compact" == *'Atomics.notify(i32a,WAIT_INDEX,undefined),NUMAGENT'* ]] \
                || die "undefined notify-count semantics drifted"
            ;;
        test/built-ins/Atomics/notify/notify-all.js)
            notify_default_count_paths=$((notify_default_count_paths + 1))
            [[ "$compact" == *'constNUMAGENT=3;'* \
                && "$compact" == *'Atomics.notify(i32a,WAIT_INDEX),NUMAGENT'* ]] \
                || die "notify-all semantics drifted"
            ;;
        test/built-ins/Atomics/notify/notify-one.js)
            notify_explicit_count_paths=$((notify_explicit_count_paths + 1))
            [[ "$compact" == *'constNOTIFYCOUNT=1;'* \
                && "$compact" == *'Atomics.notify(i32a,0,NOTIFYCOUNT),NOTIFYCOUNT'* \
                && "$compact" == *'$262.agent.trySleep(TIMEOUT)'* ]] \
                || die "notify-one semantics drifted"
            ;;
        test/built-ins/Atomics/notify/notify-two.js)
            notify_explicit_count_paths=$((notify_explicit_count_paths + 1))
            [[ "$compact" == *'constNOTIFYCOUNT=2;'* \
                && "$compact" == *'Atomics.notify(i32a,0,NOTIFYCOUNT),NOTIFYCOUNT'* \
                && "$compact" == *'$262.agent.trySleep(TIMEOUT)'* ]] \
                || die "notify-two semantics drifted"
            ;;
        test/built-ins/Atomics/notify/notify-renotify-noop.js)
            notify_explicit_count_paths=$((notify_explicit_count_paths + 1))
            [[ "$(fixed_count 'Atomics.notify(i32a, 0, 1)' "$source")" == 4 \
                && "$compact" == *'Atomics.notify(i32a,0,1),1'* \
                && "$compact" == *'Atomics.notify(i32a,0,1),0'* ]] \
                || die "notify-renotify semantics drifted"
            ;;
        test/built-ins/Atomics/notify/undefined-index-defaults-to-zero.js)
            notify_default_index_paths=$((notify_default_index_paths + 1))
            [[ "$compact" == *'Atomics.notify(i32a,undefined,1)'* \
                && "$compact" == *'Atomics.notify(i32a/*,defaultvaluesused*/)'* ]] \
                || die "notify default-index semantics drifted"
            ;;
        test/built-ins/Atomics/wait/bigint/nan-for-timeout.js)
            wait_infinite_timeout_paths=$((wait_infinite_timeout_paths + 1))
            [[ "$compact" == *'Atomics.wait(i64a,0,0n,NaN)'* \
                && "$compact" == *'Atomics.notify(i64a,0),1'* ]] \
                || die "BigInt NaN timeout semantics drifted"
            ;;
        test/built-ins/Atomics/wait/nan-for-timeout.js)
            wait_infinite_timeout_paths=$((wait_infinite_timeout_paths + 1))
            [[ "$compact" == *'Atomics.wait(i32a,0,0,NaN)'* \
                && "$compact" == *'Atomics.notify(i32a,0),1'* ]] \
                || die "Int32 NaN timeout semantics drifted"
            ;;
        test/built-ins/Atomics/wait/undefined-for-timeout.js)
            wait_infinite_timeout_paths=$((wait_infinite_timeout_paths + 1))
            [[ "$(fixed_count 'Atomics.wait(i32a' "$source")" == 2 \
                && "$compact" == *'Atomics.wait(i32a,0,0,undefined)'* \
                && "$compact" == *'Atomics.wait(i32a,0,0)'* ]] \
                || die "undefined wait-timeout semantics drifted"
            ;;
        test/built-ins/Atomics/wait/bigint/waiterlist-block-indexedposition-wake.js)
            wait_location_paths=$((wait_location_paths + 1))
            [[ "$compact" == *'Atomics.wait(i64a,0,0n,Infinity)'* \
                && "$compact" == *'Atomics.wait(i64a,2,0n,Infinity)'* \
                && "$compact" == *'Atomics.notify(i64a,1),0'* \
                && "$compact" == *'Atomics.notify(i64a,3),0'* ]] \
                || die "BigInt waiter-location semantics drifted"
            ;;
        test/built-ins/Atomics/wait/waiterlist-block-indexedposition-wake.js)
            wait_location_paths=$((wait_location_paths + 1))
            [[ "$compact" == *'Atomics.wait(i32a,0,0,Infinity)'* \
                && "$compact" == *'Atomics.wait(i32a,2,0,Infinity)'* \
                && "$compact" == *'Atomics.notify(i32a,1),0'* \
                && "$compact" == *'Atomics.notify(i32a,3),0'* ]] \
                || die "Int32 waiter-location semantics drifted"
            ;;
        test/built-ins/Atomics/wait/bigint/was-woken-before-timeout.js)
            wait_before_timeout_paths=$((wait_before_timeout_paths + 1))
            [[ "$compact" == *'$262.agent.timeouts.huge'* \
                && "$(fixed_count '$262.agent.monotonicNow()' "$source")" == 2 \
                && "$compact" == *'lapse<TIMEOUT'* \
                && "$compact" == *'Atomics.notify(i64a,0),1'* ]] \
                || die "BigInt wake-before-timeout semantics drifted"
            ;;
        test/built-ins/Atomics/wait/was-woken-before-timeout.js)
            wait_before_timeout_paths=$((wait_before_timeout_paths + 1))
            [[ "$compact" == *'$262.agent.timeouts.huge'* \
                && "$(fixed_count '$262.agent.monotonicNow()' "$source")" == 2 \
                && "$compact" == *'lapse<TIMEOUT'* \
                && "$compact" == *'Atomics.notify(i32a,0),1'* ]] \
                || die "Int32 wake-before-timeout semantics drifted"
            ;;
        test/built-ins/Atomics/wait/undefined-index-defaults-to-zero.js)
            wait_default_index_paths=$((wait_default_index_paths + 1))
            [[ "$compact" == *'Atomics.wait(i32a,undefined,0,'* \
                && "$compact" == *'Atomics.notify(i32a,0),1'* \
                && "$compact" == *'Atomics.notify(i32a,0),0'* ]] \
                || die "wait default-index semantics drifted"
            ;;
        *) die "unexpected wake/count/location activation path: $test_path" ;;
    esac
done <"$activation"
check_file "$tmp/source-ledger.tsv" "$(value activation_source_ledger_lines)" \
    "$(value activation_source_ledger_sha256)"
check_file "$tmp/metadata-ledger.tsv" "$(value activation_metadata_ledger_lines)" \
    "$(value activation_metadata_ledger_sha256)"
[[ "$notify_location_paths" == "$(value notify_location_paths)" \
    && "$notify_default_count_paths" == "$(value notify_default_count_paths)" \
    && "$notify_explicit_count_paths" == "$(value notify_explicit_count_paths)" \
    && "$notify_default_index_paths" == "$(value notify_default_index_paths)" \
    && "$wait_infinite_timeout_paths" == "$(value wait_infinite_timeout_paths)" \
    && "$wait_location_paths" == "$(value wait_location_paths)" \
    && "$wait_before_timeout_paths" == "$(value wait_before_timeout_paths)" \
    && "$wait_default_index_paths" == "$(value wait_default_index_paths)" ]] \
    || die 'wake/count/location semantic source inventory drifted'

[[ "$(toml_test262_value commit)" == "$(value test262)" \
    && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 inputs drifted'

if [[ -z ${CARGO_TARGET_DIR+x} ]]; then
    export CARGO_TARGET_DIR=$root/target/r3dp-build
fi
case $CARGO_TARGET_DIR in /*) ;; *) CARGO_TARGET_DIR=$root/$CARGO_TARGET_DIR ;; esac
export CARGO_TARGET_DIR

cargo test --quiet --locked --bin run-test262 \
    cli_tests::agent_wake_count_location_profiles_require_exact_21_path_universe_or_all \
    -- --exact --test-threads=1
cargo test --quiet --locked --bin run-test262 \
    capabilities::tests::agent_wake_count_location_profiles_add_only_the_exact_activation_allowlist \
    -- --exact --test-threads=1
cargo test --quiet --locked --bin run-test262 \
    requirements::tests::agent_host_admission_ledger_is_exact_sorted_and_metadata_frozen \
    -- --exact --test-threads=1
cargo test --quiet --locked --bin run-test262 \
    execution::tests::agent_host_worker_flag_revalidates_path_and_source \
    -- --exact --test-threads=1
cargo build --quiet --locked --release --bin run-test262
runner=$CARGO_TARGET_DIR/release/run-test262

"$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
[[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
    && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata drifted'

run_report() {
    local profile=$1 selected=$2 output=$3 run_workers=${4:-$parallel_workers}
    case $output in /*) ;; *) output=$root/$output ;; esac
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --manifest "$root/$selected" \
        --report "$output" --mode both --timeout-ms 30000 \
        --workers "$run_workers" --allow-failures >/dev/null
}

run_report "$parent_profile" "$universe" "$tmp/parent.tsv"
run_report "$candidate_profile" "$universe" "$tmp/candidate.tsv"
cmp -s "$parent_report" "$tmp/parent.tsv" \
    && cmp -s "${parent_report%.tsv}.jsonl" "$tmp/parent.jsonl" \
    || die 'wake/count/location parent replay drifted'
cmp -s "$candidate_report" "$tmp/candidate.tsv" \
    && cmp -s "${candidate_report%.tsv}.jsonl" "$tmp/candidate.jsonl" \
    || die 'wake/count/location candidate replay drifted'
verify_report "$parent_report" "$(value parent_profile_sha256)" parent
verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
[[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
    && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
    && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
    && "$(report_count unsupported-host-agent "$candidate_report")" == \
        "$(value candidate_retained_unsupported)" ]] \
    || die 'wake/count/location scoped outcome counts drifted'

for format in tsv json; do
    parent_rows=$tmp/parent.$format-result-rows.tsv
    candidate_rows=$tmp/candidate.$format-result-rows.tsv
    select_manifest_rows "$activation" "$parent_rows" >"$tmp/parent.$format.activation.tsv"
    select_manifest_rows "$activation" "$candidate_rows" >"$tmp/candidate.$format.activation.tsv"
    select_manifest_rows "$retained" "$parent_rows" >"$tmp/parent.$format.retained.tsv"
    select_manifest_rows "$retained" "$candidate_rows" >"$tmp/candidate.$format.retained.tsv"
    [[ "$(lines "$tmp/parent.$format.activation.tsv")" == "$(value activation_variants)" \
        && "$(lines "$tmp/candidate.$format.activation.tsv")" == "$(value activation_variants)" \
        && "$(lines "$tmp/parent.$format.retained.tsv")" == "$(value retained_variants)" \
        && "$(lines "$tmp/candidate.$format.retained.tsv")" == "$(value retained_variants)" \
        && "$(awk -F'\t' '$7=="unsupported-host-agent"{count++}END{print count+0}' "$tmp/parent.$format.activation.tsv")" == "$(value activation_variants)" \
        && "$(awk -F'\t' '$7=="pass"{count++}END{print count+0}' "$tmp/candidate.$format.activation.tsv")" == "$(value activation_variants)" ]] \
        || die "wake/count/location $format activation/retained partition drifted"
    diff -u "$tmp/parent.$format.retained.tsv" "$tmp/candidate.$format.retained.tsv"
    diff -u <(cut -f1-6 "$tmp/parent.$format.activation.tsv") \
        <(cut -f1-6 "$tmp/candidate.$format.activation.tsv")
done

awk -F'\t' 'NR==FNR{active[$0]=1;next}
    !/^#/&&!($1=="path"&&$2=="variant")&&($1 in active){
        print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6
    }' "$activation" "$candidate_report" >"$tmp/activation-report-metadata.tsv"
check_file "$tmp/activation-report-metadata.tsv" \
    "$(value activation_report_metadata_rows)" \
    "$(value activation_report_metadata_sha256)"

make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
[[ "$(report_rows "$transition" | sha /dev/stdin)" == \
        "$(value transition_data_sha256)" ]] \
    || die 'wake/count/location transition rows drifted'
counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if($7=="pass"&&$11!="pass")regress++
    if(different){changed++;if($11=="pass")gain++}else unchanged++
}END{printf "%d %d %d %d",changed,gain,unchanged,regress}' "$transition")
[[ "$counts" == "$(value changed) $(value pass_changes) $(value unchanged) $(value regressions)" ]] \
    || die "wake/count/location transition counts drifted: $counts"

for profile in "$parent_profile" "$candidate_profile"; do
    for rejected in "$activation" "$retained" "$predecessor_retained" Cargo.toml; do
        if run_report "$profile" "$rejected" "$tmp/rejected.tsv" 1 >/dev/null 2>&1; then
            die "wake/count/location profile accepted a non-universe manifest: $profile / $rejected"
        fi
    done
    if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" \
        --test test/built-ins/Atomics/notify/bigint/notify-all-on-loc.js \
        --report "$tmp/rejected.tsv" --mode both --workers 1 --allow-failures \
        >/dev/null 2>&1; then
        die "wake/count/location scoped profile accepted --test: $profile"
    fi
done

single_passes=0
single_retained=0
for run in $(seq 1 "$(value single_worker_runs)"); do
    run_report "$candidate_profile" "$universe" "$tmp/single-$run.tsv" 1
    passes=$(report_count pass "$tmp/single-$run.tsv")
    unsupported=$(report_count unsupported-host-agent "$tmp/single-$run.tsv")
    [[ "$(sha "$tmp/single-$run.tsv")" == "$(value single_worker_tsv_sha256)" \
        && "$(sha "$tmp/single-$run.jsonl")" == "$(value single_worker_jsonl_sha256)" \
        && "$passes" == "$(value candidate_passes)" \
        && "$unsupported" == "$(value candidate_retained_unsupported)" ]] \
        || die "wake/count/location single-worker replay $run drifted"
    single_passes=$((single_passes + passes))
    single_retained=$((single_retained + unsupported))
done
[[ "$single_passes" == "$(value single_worker_activation_passes)" \
    && "$single_retained" == "$(value single_worker_retained_outcomes)" ]] \
    || die 'wake/count/location single-worker aggregate drifted'

parallel_passes=0
parallel_retained=0
for run in $(seq 1 "$(value parallel_runs)"); do
    run_report "$candidate_profile" "$universe" "$tmp/parallel-$run.tsv" "$parallel_workers"
    passes=$(report_count pass "$tmp/parallel-$run.tsv")
    unsupported=$(report_count unsupported-host-agent "$tmp/parallel-$run.tsv")
    [[ "$(sha "$tmp/parallel-$run.tsv")" == "$(value parallel_tsv_sha256)" \
        && "$(sha "$tmp/parallel-$run.jsonl")" == "$(value parallel_jsonl_sha256)" \
        && "$passes" == "$(value candidate_passes)" \
        && "$unsupported" == "$(value candidate_retained_unsupported)" ]] \
        || die "wake/count/location parallel replay $run drifted"
    parallel_passes=$((parallel_passes + passes))
    parallel_retained=$((parallel_retained + unsupported))
done
[[ "$parallel_passes" == "$(value parallel_activation_passes)" \
    && "$parallel_retained" == "$(value parallel_retained_outcomes)" ]] \
    || die 'wake/count/location parallel aggregate drifted'

[[ -x "$source_dir/run-test262" ]] || die 'authenticated QuickJS run-test262 oracle is missing'
quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$activation"
if ! (cd -- "$source_dir" && ./run-test262 -m -c test262.conf -a -T 1 \
    -f "${quickjs_files[@]}") >"$tmp/quickjs.log" 2>&1; then
    tail -n 100 "$tmp/quickjs.log" >&2
    die 'pinned QuickJS could not execute wake/count/location cohort'
fi
grep -Fq 'Average memory statistics for 34 tests:' "$tmp/quickjs.log" \
    && ! grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
        "$tmp/quickjs.log" \
    || die 'pinned QuickJS no longer passes wake/count/location cohort 34/34'
{
    echo '# Pinned QuickJS oracle receipt for Test262 agent wake/count/location cohort.'
    echo "quickjs=$(value quickjs)"
    echo "quickjs_source_sha256=$(value quickjs_source_sha256)"
    echo "test262=$(value test262)"
    echo "activation_manifest_sha256=$(value activation_sha256)"
    echo "activation_source_ledger_sha256=$(value activation_source_ledger_sha256)"
    echo "activation_metadata_ledger_sha256=$(value activation_metadata_ledger_sha256)"
    echo "paths=$(value quickjs_paths)"
    echo "variants=$(value quickjs_variants)"
    echo "passes=$(value quickjs_passes)"
    echo 'failed=0'
    echo 'skipped_feature=0'
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
diff -u "$quickjs_receipt" "$tmp/quickjs-receipt.txt"

TEST262_WORKERS="$parallel_workers" "$predecessor_gate" --check >/dev/null

printf 'R3dp scoped agent wake/count/location verified: Oxide 34/34, retained 8 fail closed, pinned QuickJS 34/34, stability 1700 + 680 passes.\n'
