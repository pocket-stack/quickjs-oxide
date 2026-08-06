#!/usr/bin/env bash
set -euo pipefail

# Authenticate the frozen R3dq Test262 agent FIFO wake-order receipt.
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-agent-fifo-wake-order-baseline.txt
upstream=compat/upstream.toml
predecessor_gate=scripts/test-test262-agent-wake-count-location.sh
predecessor_baseline=tests/test262-agent-wake-count-location-baseline.txt
predecessor_profile=tests/test262-agent-wake-count-location-candidate.conf
predecessor_activation=tests/test262-agent-wake-count-location.txt
predecessor_universe=tests/test262-agent-wake-count-location-universe.txt
predecessor_retained=tests/test262-agent-wake-count-location-retained.txt
universe=tests/test262-agent-fifo-wake-order-universe.txt
activation=tests/test262-agent-fifo-wake-order.txt
parent_profile=tests/test262-agent-fifo-wake-order-parent.conf
candidate_profile=tests/test262-agent-fifo-wake-order-candidate.conf
parent_report=tests/test262-agent-fifo-wake-order-parent.tsv
candidate_report=tests/test262-agent-fifo-wake-order-candidate.tsv
transition=tests/test262-agent-fifo-wake-order-transitions.tsv
quickjs_receipt=tests/test262-agent-fifo-wake-order-quickjs-receipt.txt
parallel_workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  replay the exact R3dq FIFO wake-order receipt\n'
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
    || { echo 'error: TEST262_RUNNER override is forbidden for R3dq' >&2; exit 2; }

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
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' "$1"
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
toml_value() {
    awk -v section="[$1]" -v wanted="$2" '
        $0==section{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted)next
            answer=substr($0,separator+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$upstream"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk \
        '{out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}

# Parse only the exact ordered schema-2 result shape. This deliberately avoids
# jq and rejects unknown, reordered, missing, malformed, or trailing fields.
json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3dq JSONL projection %s: %s\n",report,message >"/dev/stderr"
            exit 2
        }
        function expect(token) {
            if(substr(line,pos,length(token))!=token)
                fail("expected " token " at column " pos)
            pos+=length(token)
        }
        function string_value(    c,e,digits,v) {
            expect("\"");v=""
            while(pos<=length(line)) {
                c=substr(line,pos,1)
                if(c=="\""){pos++;return v}
                if(c=="\\") {
                    pos++;if(pos>length(line))fail("unterminated escape")
                    e=substr(line,pos,1)
                    if(e=="u") {
                        digits=substr(line,pos+1,4)
                        if(length(digits)!=4||digits~/[^0123456789abcdefABCDEF]/)
                            fail("invalid Unicode escape")
                        v=v "\\u" digits;pos+=5
                    } else {
                        if(index("\"\\/bfnrt",e)==0)fail("invalid string escape")
                        if(e=="\"")v=v "\"";else if(e=="/")v=v "/"
                        else if(e=="b")v=v "\\u0008";else if(e=="f")v=v "\\u000c"
                        else v=v "\\" e
                        pos++
                    }
                    continue
                }
                if(c=="\t"||c=="\r")fail("unescaped control character")
                v=v c;pos++
            }
            fail("unterminated string")
        }
        function project(    i,key,v) {
            line=$0;pos=1;expect("{")
            for(i=1;i<=11;i++) {
                if(i!=1)expect(",")
                key=string_value();if(key!=name[i])fail("unexpected field " key)
                expect(":");v=string_value()
                if(i==1){if(v!="result")fail("unexpected record kind")}
                else field[i-1]=v
            }
            expect("}");if(pos!=length(line)+1)fail("trailing record data")
            print field[1],field[2],field[3],field[4],field[5],field[6],field[7],field[8],field[9],field[10]
        }
        BEGIN {
            OFS="\t";name[1]="kind";name[2]="path";name[3]="variant"
            name[4]="flags";name[5]="features";name[6]="expected_phase"
            name[7]="expected_type";name[8]="outcome";name[9]="actual_phase"
            name[10]="actual_type";name[11]="detail"
        }
        /^\{"kind":"metadata",/{next}
        /^\{"kind":"result",/{project();next}
        /^\{"kind":"summary",/{next}
        {fail("unexpected record")}
    ' "$report"
}

verify_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    json_result_projection "$json" >"$tmp/$label.json-result-rows.tsv" \
        || die "JSONL projection failed: $json"
    report_rows "$report" >"$tmp/$label.tsv-result-rows.tsv"
    cmp -s "$tmp/$label.tsv-result-rows.tsv" "$tmp/$label.json-result-rows.tsv" \
        || die "JSONL/TSV ordered ten-field bytes drifted: $json"
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
        && "$(sha "$tmp/$label.tsv-result-rows.tsv")" == "$(value "${label}_result_rows_sha256")" \
        && "$(lines "$tmp/$label.json-result-rows.tsv")" == "$(value universe_variants)" \
        && "$(awk -F'\t' '{print $1 "\t" $2}' "$tmp/$label.json-result-rows.tsv" | sort | sha /dev/stdin)" == "$(value report_keys_sha256)" \
        && "$(awk -F'\t' '{print $1 "\t" $2}' "$tmp/$label.json-result-rows.tsv" | sort -u | lines /dev/stdin)" == "$(value universe_variants)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "focused report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive Test262 agent FIFO wake-order transition.'
        echo '# milestone=R3dq'
        echo "# parent_commit=$(value parent_commit)"
        echo "# parent_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_profile_sha256=$(value candidate_profile_sha256)"
        echo "# universe_sha256=$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR {
                if(!/^#/&&!($1=="path"&&$2=="variant")){key=$1 FS $2;old[key]=$0}
                next
            }
            !/^#/&&!($1=="path"&&$2=="variant") {
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
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dq-scoped.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM

check_file "$baseline" 112 \
    8ddbe467283762b4815b82b875b752979d967c1a7ca40c77f56d0bc6c52fda57
[[ "$(value milestone)" == R3dq \
    && "$(value milestone_kind)" == scoped-runner-admission \
    && "$(value scope_semantics)" == fifo-wake-order \
    && "$(value runtime_waiter_changes)" == none \
    && "$(value global_promotion)" == false \
    && "$(value profile_selection)" == exact-universe-or-all \
    && "$(value submanifest_selection)" == fail-closed \
    && "$(value single_test_selection)" == fail-closed \
    && "$(value coordinator_revalidation)" == exact-path-source-sha256-metadata \
    && "$(value worker_revalidation)" == exact-path-source-sha256-metadata \
    && "$(value predecessor_gate_replayed)" == true \
    && "$parallel_workers" == "$(value parallel_workers)" ]] \
    || die 'R3dq scoped admission boundary drifted'

[[ -f "$upstream" \
    && "$(toml_value quickjs version)" == "$(value quickjs)" \
    && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
    && "$(toml_value test262 commit)" == "$(value test262)" \
    && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256)" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned QuickJS/Test262 inputs drifted'

check_file "$predecessor_gate" "$(value predecessor_gate_lines)" \
    "$(value predecessor_gate_sha256)"
check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
    "$(value predecessor_baseline_sha256)"
check_file "$predecessor_profile" "$(value predecessor_profile_lines)" \
    "$(value predecessor_profile_sha256)"
check_file "$predecessor_activation" "$(value predecessor_activation_lines)" \
    "$(value predecessor_activation_sha256)"
check_file "$predecessor_universe" "$(value predecessor_universe_lines)" \
    "$(value predecessor_universe_sha256)"
check_file "$predecessor_retained" "$(value predecessor_retained_lines)" \
    "$(value predecessor_retained_sha256)"
check_file "$universe" "$(value universe_paths)" "$(value universe_sha256)"
check_file "$activation" "$(value activation_paths)" "$(value activation_sha256)"
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

for file in "$universe" "$activation" "$predecessor_activation" \
    "$predecessor_universe" "$predecessor_retained"; do
    sort -c "$file" || die "manifest is not bytewise sorted: $file"
    [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
done
cmp -s "$universe" "$activation" \
    || die 'R3dq canonical universe and activation are not byte-identical'
cmp -s "$universe" "$predecessor_retained" \
    || die 'R3dq exact4 universe is not the frozen R3dp retained cohort'
[[ "$(value universe_paths)" == 4 \
    && "$(value activation_paths)" == 4 \
    && "$(value universe_variants)" == 8 \
    && "$(value activation_variants)" == 8 ]] \
    || die 'R3dq exact4 manifest cardinality drifted'

for section in features audited-negative-tests host-agent-tests; do
    profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
    profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
done
cmp -s "$predecessor_profile" "$parent_profile" \
    || die 'R3dq parent is not byte-identical to the R3dp candidate profile'
cmp -s "$tmp/parent.features" "$tmp/candidate.features"
cmp -s "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
[[ "$(lines "$tmp/parent.features")" == "$(value profile_features)" \
    && "$(sha "$tmp/parent.features")" == "$(value profile_features_sha256)" \
    && "$(lines "$tmp/parent.audited-negative-tests")" == "$(value profile_audited_negatives)" \
    && "$(sha "$tmp/parent.audited-negative-tests")" == "$(value profile_audited_negatives_sha256)" \
    && "$(lines "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_paths)" \
    && "$(sha "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_sha256)" \
    && "$(lines "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_paths)" \
    && "$(sha "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_sha256)" ]] \
    || die 'R3dq profile inventory drifted'
[[ -z "$(comm -23 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests")" ]] \
    || die 'R3dq candidate removed a historical agent admission'
comm -13 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
    >"$tmp/allowlist-delta.txt"
check_file "$tmp/allowlist-delta.txt" "$(value agent_allowlist_delta_paths)" \
    "$(value agent_allowlist_delta_sha256)"
cmp -s "$activation" "$tmp/allowlist-delta.txt" \
    || die 'R3dq candidate allowlist delta is not exact4'

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
: >"$tmp/source-ledger.tsv"
: >"$tmp/metadata-ledger.tsv"
: >"$tmp/admission-ledger.tsv"
notify_order_paths=0
waiter_fifo_paths=0
bigint_fifo_paths=0
int32_fifo_paths=0
while IFS= read -r test_path; do
    source=$suite/$test_path
    [[ -f "$source" ]] || die "pinned activation source is missing: $test_path"
    source_sha=$(sha "$source")
    features=$(sed -n 's/^features: \[\(.*\)\]$/\1/p' "$source" | tr -d '[:space:]')
    includes=$(sed -n 's/^includes: \[\(.*\)\]$/\1/p' "$source" | tr -d '[:space:]')
    [[ -n "$features" && "$includes" == atomicsHelper.js \
        && "$(grep -Ec '^flags:|^negative:' "$source")" == 0 ]] \
        || die "R3dq activation metadata shape drifted: $test_path"
    printf '%s\t%s\n' "$test_path" "$source_sha" >>"$tmp/source-ledger.tsv"
    printf '%s\tflags=-\tfeatures=%s\tincludes=%s\tnegative=-\n' \
        "$test_path" "$features" "$includes" >>"$tmp/metadata-ledger.tsv"
    printf '%s\t%s\tflags=-\tfeatures=%s\tincludes=%s\tnegative=-\n' \
        "$test_path" "$source_sha" "$features" "$includes" >>"$tmp/admission-ledger.tsv"
    case $test_path in
        test/built-ins/Atomics/notify/notify-in-order-one-time.js|\
        test/built-ins/Atomics/notify/notify-in-order.js)
            notify_order_paths=$((notify_order_paths + 1)) ;;
        test/built-ins/Atomics/wait/bigint/waiterlist-order-of-operations-is-fifo.js)
            waiter_fifo_paths=$((waiter_fifo_paths + 1)); bigint_fifo_paths=$((bigint_fifo_paths + 1)) ;;
        test/built-ins/Atomics/wait/waiterlist-order-of-operations-is-fifo.js)
            waiter_fifo_paths=$((waiter_fifo_paths + 1)); int32_fifo_paths=$((int32_fifo_paths + 1)) ;;
        *) die "unexpected R3dq activation path: $test_path" ;;
    esac
done <"$activation"
check_file "$tmp/source-ledger.tsv" "$(value activation_source_ledger_lines)" \
    "$(value activation_source_ledger_sha256)"
check_file "$tmp/metadata-ledger.tsv" "$(value activation_metadata_ledger_lines)" \
    "$(value activation_metadata_ledger_sha256)"
check_file "$tmp/admission-ledger.tsv" "$(value activation_admission_ledger_lines)" \
    "$(value activation_admission_ledger_sha256)"
[[ "$notify_order_paths" == "$(value notify_order_paths)" \
    && "$waiter_fifo_paths" == "$(value waiter_fifo_paths)" \
    && "$bigint_fifo_paths" == "$(value bigint_fifo_paths)" \
    && "$int32_fifo_paths" == "$(value int32_fifo_paths)" ]] \
    || die 'R3dq semantic path inventory drifted'

if [[ -z ${CARGO_TARGET_DIR+x} ]]; then
    export CARGO_TARGET_DIR=$root/target/r3dq-build
fi
case $CARGO_TARGET_DIR in /*) ;; *) CARGO_TARGET_DIR=$root/$CARGO_TARGET_DIR ;; esac
export CARGO_TARGET_DIR

cargo test --quiet --locked --bin run-test262 \
    cli_tests::agent_fifo_wake_order_profiles_require_exact_4_path_universe_or_all \
    -- --exact --test-threads=1
cargo test --quiet --locked --bin run-test262 \
    capabilities::tests::agent_fifo_wake_order_profiles_add_only_the_exact_activation_allowlist \
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
    || die 'R3dq focused parent replay drifted'
cmp -s "$candidate_report" "$tmp/candidate.tsv" \
    && cmp -s "${candidate_report%.tsv}.jsonl" "$tmp/candidate.jsonl" \
    || die 'R3dq focused candidate replay drifted'
verify_report "$parent_report" "$(value parent_profile_sha256)" parent
verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
[[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
    && "$(report_count unsupported-host-agent "$parent_report")" == "$(value universe_variants)" \
    && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
    && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
    && "$(report_count pass "$candidate_report")" == "$(value oxide_passes)" \
    && "$(report_count unsupported-host-agent "$candidate_report")" == 0 ]] \
    || die 'R3dq focused outcome counts drifted'

awk -F'\t' 'NR==FNR{active[$0]=1;next}
    !/^#/&&!($1=="path"&&$2=="variant")&&($1 in active){
        print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6
    }' "$activation" "$candidate_report" >"$tmp/activation-report-metadata.tsv"
check_file "$tmp/activation-report-metadata.tsv" \
    "$(value activation_report_metadata_rows)" \
    "$(value activation_report_metadata_sha256)"

make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
cmp -s "$transition" "$tmp/transition.tsv" \
    || die 'R3dq transition replay drifted'
[[ "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" ]] \
    || die 'R3dq transition rows drifted'
counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if($7=="pass"&&$11!="pass")regress++
    if(different){changed++;if($11=="pass")gain++}else unchanged++
}END{printf "%d %d %d %d",changed,gain,unchanged,regress}' "$transition")
[[ "$counts" == "$(value changed) $(value pass_changes) $(value unchanged) $(value regressions)" \
    && "$(value changed) $(value pass_changes) $(value unchanged) $(value regressions)" == '8 8 0 0' ]] \
    || die "R3dq transition counts drifted: $counts"

# The profile verifier is path-canonical, not merely byte-content based. Reject
# the same-byte activation/R3dp-retained paths as well as the R3dp 17/21 sets.
for profile in "$parent_profile" "$candidate_profile"; do
    for rejected in "$activation" "$predecessor_activation" \
        "$predecessor_universe" "$predecessor_retained" Cargo.toml; do
        if run_report "$profile" "$rejected" "$tmp/rejected.tsv" 1 \
            >/dev/null 2>&1; then
            die "R3dq profile accepted non-canonical manifest: $profile / $rejected"
        fi
    done
    if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" \
        --test test/built-ins/Atomics/notify/notify-in-order.js \
        --report "$tmp/rejected.tsv" --mode both --workers 1 --allow-failures \
        >/dev/null 2>&1; then
        die "R3dq scoped profile accepted --test: $profile"
    fi
done

single_passes=0
for run in $(seq 1 "$(value single_worker_runs)"); do
    run_report "$candidate_profile" "$universe" "$tmp/single-$run.tsv" 1
    passes=$(report_count pass "$tmp/single-$run.tsv")
    [[ "$(sha "$tmp/single-$run.tsv")" == "$(value single_worker_tsv_sha256)" \
        && "$(sha "$tmp/single-$run.jsonl")" == "$(value single_worker_jsonl_sha256)" \
        && "$passes" == "$(value candidate_passes)" \
        && "$(report_count unsupported-host-agent "$tmp/single-$run.tsv")" == 0 ]] \
        || die "R3dq workers=1 replay $run drifted"
    single_passes=$((single_passes + passes))
done
[[ "$single_passes" == "$(value single_worker_passes)" \
    && "$(value single_worker_runs)" == 100 \
    && "$(value single_worker_passes)" == 800 ]] \
    || die 'R3dq workers=1 aggregate drifted'

parallel_passes=0
for run in $(seq 1 "$(value parallel_runs)"); do
    run_report "$candidate_profile" "$universe" "$tmp/parallel-$run.tsv" \
        "$parallel_workers"
    passes=$(report_count pass "$tmp/parallel-$run.tsv")
    [[ "$(sha "$tmp/parallel-$run.tsv")" == "$(value parallel_tsv_sha256)" \
        && "$(sha "$tmp/parallel-$run.jsonl")" == "$(value parallel_jsonl_sha256)" \
        && "$passes" == "$(value candidate_passes)" \
        && "$(report_count unsupported-host-agent "$tmp/parallel-$run.tsv")" == 0 ]] \
        || die "R3dq workers=$parallel_workers replay $run drifted"
    parallel_passes=$((parallel_passes + passes))
done
[[ "$parallel_passes" == "$(value parallel_passes)" \
    && "$(value parallel_workers)" == 8 \
    && "$(value parallel_runs)" == 32 \
    && "$(value parallel_passes)" == 256 ]] \
    || die 'R3dq workers=8 aggregate drifted'

[[ -x "$source_dir/run-test262" ]] || die 'authenticated QuickJS run-test262 oracle is missing'
quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$activation"
[[ "${#quickjs_files[@]}" == "$(value quickjs_paths)" ]] \
    || die 'R3dq QuickJS exact4 path vector drifted'
quickjs_passes=0
for run in $(seq 1 "$(value quickjs_stability_runs)"); do
    if ! (cd -- "$source_dir" && ./run-test262 -m -c test262.conf -a -T 1 \
        -f "${quickjs_files[@]}") >"$tmp/quickjs-$run.log" 2>&1; then
        tail -n 100 "$tmp/quickjs-$run.log" >&2
        die "pinned QuickJS R3dq execution $run failed"
    fi
    grep -Fq "Average memory statistics for $(value quickjs_variants_per_run) tests:" \
        "$tmp/quickjs-$run.log" \
        && ! grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
            "$tmp/quickjs-$run.log" \
        || die "pinned QuickJS R3dq execution $run did not pass exact4/8"
    quickjs_passes=$((quickjs_passes + $(value quickjs_variants_per_run)))
done
[[ "$(value quickjs_stability_runs)" == 100 \
    && "$(value quickjs_variants_per_run)" == 8 \
    && "$quickjs_passes" == "$(value quickjs_stability_passes)" \
    && "$(value quickjs_stability_passes)" == 800 ]] \
    || die 'pinned QuickJS R3dq stability aggregate drifted'

{
    echo '# Pinned QuickJS oracle receipt for Test262 agent FIFO wake-order cohort.'
    echo "quickjs=$(value quickjs)"
    echo "quickjs_source_sha256=$(value quickjs_source_sha256)"
    echo "test262=$(value test262)"
    echo "activation_manifest_sha256=$(value activation_sha256)"
    echo "activation_source_ledger_sha256=$(value activation_source_ledger_sha256)"
    echo "activation_metadata_ledger_sha256=$(value activation_metadata_ledger_sha256)"
    echo "paths=$(value quickjs_paths)"
    echo "variants_per_run=$(value quickjs_variants_per_run)"
    echo "stability_runs=$(value quickjs_stability_runs)"
    echo "stability_passes=$quickjs_passes"
    echo 'failed=0'
    echo 'skipped_feature=0'
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
cmp -s "$quickjs_receipt" "$tmp/quickjs-receipt.txt" \
    || die 'pinned QuickJS R3dq receipt drifted'

TEST262_WORKERS="$parallel_workers" "$predecessor_gate" --check >/dev/null

printf 'R3dq scoped agent FIFO wake order verified: Oxide 8/8, stability 800 + 256 passes; pinned QuickJS 100x8; R3dp replayed.\n'
