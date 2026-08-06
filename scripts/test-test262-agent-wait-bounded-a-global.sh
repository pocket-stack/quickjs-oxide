#!/usr/bin/env bash
# Reproduce the R3do Test262 agent bounded wait cohort A global admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-agent-wait-bounded-a-global-baseline.txt
predecessor_baseline=tests/test262-agent-broadcast-a-global-baseline.txt
predecessor_profile=tests/test262-agent-broadcast-a-global-candidate.conf
predecessor_universe=tests/test262-agent-broadcast-a-retained.txt
scoped_baseline=tests/test262-agent-wait-bounded-a-baseline.txt
scoped_gate=scripts/test-test262-agent-wait-bounded-a.sh
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-agent-wait-bounded-a-global-parent.conf
candidate_profile=tests/test262-agent-wait-bounded-a-global-candidate.conf
manifest=tests/test262-agent-wait-bounded-a-universe.txt
activation=tests/test262-agent-wait-bounded-a.txt
retained=tests/test262-agent-wait-bounded-a-retained.txt
quickjs_receipt=tests/test262-agent-wait-bounded-a-quickjs-receipt.txt
parent_report=tests/test262-agent-wait-bounded-a-global-parent.tsv
candidate_report=tests/test262-agent-wait-bounded-a-global-candidate.tsv
transition=tests/test262-agent-wait-bounded-a-global-transitions.tsv
parent_replay=target/test262-agent-wait-bounded-a-global-parent-replay.tsv
candidate_replay=target/test262-agent-wait-bounded-a-global-candidate-replay.tsv
preferred_parent_full=target/test262-agent-broadcast-a-global-candidate-full.tsv
generated_parent_full=target/test262-agent-wait-bounded-a-global-parent-full.tsv
candidate_full=target/test262-agent-wait-bounded-a-global-candidate-full.tsv
candidate_full_repeat=target/test262-agent-wait-bounded-a-global-candidate-full-repeat.tsv
oracle_log=target/test262-agent-wait-bounded-a-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}

baseline_lines=124
baseline_sha=42d8dc706c90075ebfe08a6bb003002b439fcaa3d1822f84411c761a2c811d0d

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate profiles, focused receipts, canonical state, and QuickJS\n'
    printf '  default  additionally replay the exact 43-path focused transition\n'
    printf '  --full   additionally run two independent complete candidate vectors\n'
}

mode=focused
case ${1-} in
    '') ;;
    --check) mode=check ;;
    --full) mode=full ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ && "$full_workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid Test262 worker count' >&2; exit 2; }
[[ "$reuse_full_reports" == false || "$reuse_full_reports" == true ]] \
    || { echo 'error: TEST262_REUSE_FULL_REPORTS must be true or false' >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
value_from() {
    awk -F= -v wanted="$2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
value() { value_from "$baseline" "$1"; }
predecessor_value() { value_from "$predecessor_baseline" "$1"; }
scoped_value() { value_from "$scoped_baseline" "$1"; }
canonical_value() { value_from "$canonical_baseline" "$1"; }
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
            printf "error: R3do JSONL projection %s: %s\n", report, message \
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
    json_result_projection "$json" >"$tmp/$label.json-projection.tsv" \
        || die "R3do JSONL result projection failed: $json"
    report_rows "$report" >"$tmp/$label.tsv-projection.tsv"
    diff -u "$tmp/$label.tsv-projection.tsv" "$tmp/$label.json-projection.tsv" \
        || die "R3do JSONL/TSV ten-field projection drifted: $json"
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
rows_for_paths() {
    awk -F'\t' 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' "$1" "$2"
}
rows_without_paths() {
    awk -F'\t' 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&!($1 in wanted)' "$1" "$2"
}
json_rows_for_paths() {
    awk 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        /^\{"kind":"result"/{
            if(!match($0,/"path":"[^"]*"/))exit 2
            path=substr($0,RSTART+8,RLENGTH-9)
            if(path in wanted)print
        }' "$1" "$2"
}
json_rows_without_paths() {
    awk 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        /^\{"kind":"result"/{
            if(!match($0,/"path":"[^"]*"/))exit 2
            path=substr($0,RSTART+8,RLENGTH-9)
            if(!(path in wanted))print
        }' "$1" "$2"
}
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
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value manifest_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value manifest_keys_sha256)" \
        && "$(report_rows "$report" | sha /dev/stdin)" == "$(value "${label}_rows_sha256")" \
        && "$(json_result_rows "$json" | lines /dev/stdin)" == "$(value manifest_variants)" \
        && "$(json_result_rows "$json" | sha /dev/stdin)" == "$(value "${label}_json_rows_sha256")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "focused report drifted: $report"
}

verify_full_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    verify_json_tsv_projection "$report" "$json" "$label"
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$report")" == "$(value full_tsv_lines)" \
        && "$(lines "$json")" == "$(value full_jsonl_lines)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value full_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "full report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3do Test262 agent bounded wait cohort A global transition.'
        echo "# parent_commit=$(value parent_commit)"
        echo "# parent_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_profile_sha256=$(value candidate_profile_sha256)"
        echo "# manifest_sha256=$(value manifest_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{
                if(!/^#/&&!($1=="path"&&$2=="variant")){
                    key=$1 FS $2;if(key in old)exit 2;old[key]=$0
                }
                next
            }
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old)||key in seen)exit 3
                split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 4
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 5}
        ' "$before" "$after"
    } >"$output"
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if($7!="pass"&&$11=="pass")gain++
        if($7=="pass"&&$11!="pass")regress++
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d",changed,outcome,detail,unchanged,gain,regress}' "$1"
}

check_profiles() {
    check_file "$parent_profile" "$(value parent_profile_lines)" \
        "$(value parent_profile_sha256)"
    check_file "$candidate_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    check_file "$live_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    cmp -s "$candidate_profile" "$live_profile" \
        || die 'live profile is not byte-identical to the R3do candidate'
    cmp -s "$parent_profile" "$predecessor_profile" \
        || die 'R3do parent is not byte-identical to the R3dn live profile'

    local section
    for section in features audited-negative-tests execution host-agent-tests; do
        profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
        profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
    done
    [[ "$(lines "$tmp/parent.features")" == "$(value profile_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value profile_features_sha256)" \
        && "$(lines "$tmp/candidate.features")" == "$(value profile_features)" \
        && "$(sha "$tmp/candidate.features")" == "$(value profile_features_sha256)" \
        && "$(lines "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests)" \
        && "$(sha "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests_sha256)" \
        && "$(sha "$tmp/candidate.audited-negative-tests")" == "$(value audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" \
        && "$(sha "$tmp/candidate.execution")" == "$(value execution_sha256)" \
        && "$(lines "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_paths)" \
        && "$(sha "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_sha256)" \
        && "$(lines "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_paths)" \
        && "$(sha "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_sha256)" ]] \
        || die 'R3do profile inventory drifted'
    diff -u "$tmp/parent.features" "$tmp/candidate.features"
    diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
    diff -u "$tmp/parent.execution" "$tmp/candidate.execution"
    comm -23 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
        >"$tmp/removed-agent-paths"
    [[ ! -s "$tmp/removed-agent-paths" ]] || die 'R3do removed a prior agent admission'
    comm -13 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
        >"$tmp/agent-delta"
    check_file "$tmp/agent-delta" "$(value agent_allowlist_delta_paths)" \
        "$(value agent_allowlist_delta_sha256)"
    diff -u "$activation" "$tmp/agent-delta"
}

check_manifest_and_sources() {
    check_file "$predecessor_universe" "$(predecessor_value retained_paths)" \
        "$(predecessor_value retained_sha256)"
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    check_file "$activation" "$(value activation_paths)" "$(value activation_sha256)"
    check_file "$retained" "$(value retained_paths)" "$(value retained_sha256)"
    check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
        "$(value quickjs_receipt_sha256)"
    local file path
    cmp -s "$predecessor_universe" "$manifest" \
        || die 'R3do universe is not byte-identical to the R3dn retained manifest'
    for file in "$manifest" "$activation" "$retained"; do
        sort -c "$file" || die "R3do manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] || die "R3do manifest contains duplicates: $file"
    done
    sort -u "$activation" "$retained" >"$tmp/partition"
    diff -u "$manifest" "$tmp/partition"
    [[ -z "$(comm -12 "$activation" "$retained")" \
        && "activation=$(lines "$activation") retained=$(lines "$retained")" == \
            "$(value manifest_partition)" ]] \
        || die 'R3do manifest partition drifted'

    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata drifted'
    while IFS= read -r path; do
        printf '%s\t%s\n' "$path" "$(sha "$suite/$path")"
    done <"$activation" >"$tmp/source-ledger.tsv"
    check_file "$tmp/source-ledger.tsv" "$(value activation_source_ledger_lines)" \
        "$(value activation_source_ledger_sha256)"
}

check_receipts() {
    check_file "$parent_report" "$(value parent_report_lines)" \
        "$(value parent_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
        "$(value parent_jsonl_sha256)"
    check_file "$candidate_report" "$(value candidate_report_lines)" \
        "$(value candidate_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
        "$(value candidate_jsonl_sha256)"
    verify_report "$parent_report" "$(value parent_profile_sha256)" parent
    verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
    [[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
        && "$(report_count pass "$parent_report")" == "$(value parent_passes)" \
        && "$(report_count unsupported-host-agent "$parent_report")" == \
            "$(value parent_unsupported_host_agent)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
        && "$(report_count unsupported-host-agent "$candidate_report")" == \
            "$(value candidate_unsupported_host_agent)" ]] \
        || die 'R3do focused outcome counts drifted'

    rows_for_paths "$activation" "$parent_report" >"$tmp/parent.activation"
    rows_for_paths "$activation" "$candidate_report" >"$tmp/candidate.activation"
    rows_for_paths "$retained" "$parent_report" >"$tmp/parent.retained"
    rows_for_paths "$retained" "$candidate_report" >"$tmp/candidate.retained"
    [[ "$(lines "$tmp/parent.activation")" == "$(value activation_variants)" \
        && "$(lines "$tmp/candidate.activation")" == "$(value activation_variants)" \
        && "$(lines "$tmp/parent.retained")" == "$(value retained_variants)" \
        && "$(lines "$tmp/candidate.retained")" == "$(value retained_variants)" ]] \
        || die 'R3do focused report partition drifted'
    awk -F'\t' '{if($7!="unsupported-host-agent"||$8!="selection"||
        $9!="HostCapability"||$10!="missing execution capabilities: agent")exit 2}' \
        "$tmp/parent.activation" || die 'R3do parent activation frontier drifted'
    awk -F'\t' '{if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2}' \
        "$tmp/candidate.activation" || die 'R3do candidate activation drifted'
    diff -u "$tmp/parent.retained" "$tmp/candidate.retained"

    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" parent_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(header "$transition" candidate_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)" ]] \
        || die 'R3do focused transition drifted'
}

check_history_and_canonical() {
    local parent_commit parent_snapshot parent_canonical_snapshot
    check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
        "$(value predecessor_baseline_sha256)"
    check_file "$scoped_baseline" "$(value scoped_baseline_lines)" \
        "$(value scoped_baseline_sha256)"
    check_file "$scoped_gate" "$(value scoped_gate_lines)" "$(value scoped_gate_sha256)"
    check_file "$canonical_baseline" "$(value canonical_baseline_lines)" \
        "$(value canonical_candidate_sha256)"
    check_file "$upstream" "$(value upstream_lines)" "$(value upstream_sha256)"
    parent_commit=$(value parent_commit)
    parent_snapshot=$tmp/parent-commit-profile.conf
    git cat-file -e "${parent_commit}^{commit}" 2>/dev/null \
        || die "R3do parent commit is unavailable: $parent_commit"
    git show "${parent_commit}:${live_profile}" >"$parent_snapshot" 2>/dev/null \
        || die "R3do parent commit has no $live_profile snapshot"
    cmp -s "$parent_profile" "$parent_snapshot" \
        || die 'R3do parent profile does not match its recorded commit'
    parent_canonical_snapshot=$tmp/parent-commit-canonical-baseline.txt
    git show "${parent_commit}:${canonical_baseline}" >"$parent_canonical_snapshot" 2>/dev/null \
        || die "R3do parent commit has no $canonical_baseline snapshot"
    check_file "$parent_canonical_snapshot" "$(value canonical_baseline_lines)" \
        "$(value canonical_parent_sha256)"
    [[ "$(predecessor_value candidate_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(predecessor_value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value candidate_full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value candidate_full_unsupported_host_agent)" == \
            "$(value parent_full_unsupported_host_agent)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == \
            "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == \
            "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value candidate_full_summary)" == \
            "$(value parent_full_summary)" \
        && "$(scoped_value universe_sha256)" == "$(value manifest_sha256)" \
        && "$(scoped_value activation_sha256)" == "$(value activation_sha256)" \
        && "$(scoped_value retained_sha256)" == "$(value retained_sha256)" \
        && "$(scoped_value candidate_passes)" == "$(value candidate_passes)" \
        && "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value metadata_records_sha256)" == \
            "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == \
            "$(value candidate_profile_sha256)" ]] \
        || die 'R3do predecessor, scoped, canonical, or upstream bridge drifted'
}

run_focused_report() {
    local profile=$1 output=$2 run_workers=${3:-1}
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --manifest "$root/$manifest" \
        --report "$output" --mode both --timeout-ms "$(value timeout_ms)" \
        --workers "$run_workers" --allow-failures >/dev/null
}

verify_profile_handshake() {
    local label profile expected rejected
    for label in parent candidate; do
        if [[ "$label" == parent ]]; then
            profile=$parent_profile; expected=$parent_report
        else
            profile=$candidate_profile; expected=$candidate_report
        fi
        run_focused_report "$profile" "$tmp/$label.tsv"
        if ! cmp -s "$expected" "$tmp/$label.tsv" \
            || ! cmp -s "${expected%.tsv}.jsonl" "$tmp/$label.jsonl"; then
            die "R3do runner failed the exact $label profile handshake"
        fi
        for rejected in "$activation" "$retained" "$predecessor_universe" Cargo.toml; do
            if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
                --oxide-profile "$root/$profile" --manifest "$root/$rejected" \
                --report "$tmp/rejected.tsv" --mode both \
                --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
                >/dev/null 2>&1; then
                die "R3do $label profile accepted a non-R3do manifest: $rejected"
            fi
        done
        if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
            --oxide-profile "$root/$profile" \
            --test test/built-ins/Atomics/wait/value-not-equal.js \
            --report "$tmp/rejected.tsv" --mode both \
            --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
            >/dev/null 2>&1; then
            die "R3do $label profile accepted --test"
        fi
    done
    make_transition "$tmp/parent.tsv" "$tmp/candidate.tsv" "$tmp/transition.tsv"
    diff -u "$transition" "$tmp/transition.tsv"
}

replay_focused() {
    run_focused_report "$parent_profile" "$root/$parent_replay" "$workers"
    run_focused_report "$candidate_profile" "$root/$candidate_replay" "$workers"
    if ! cmp -s "$parent_report" "$parent_replay" \
        || ! cmp -s "${parent_report%.tsv}.jsonl" "${parent_replay%.tsv}.jsonl"; then
        die 'R3do parent focused replay drifted'
    fi
    if ! cmp -s "$candidate_report" "$candidate_replay" \
        || ! cmp -s "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl"; then
        die 'R3do candidate focused replay drifted'
    fi
    make_transition "$parent_replay" "$candidate_replay" "$tmp/replayed-transition.tsv"
    diff -u "$transition" "$tmp/replayed-transition.tsv"
}

verify_quickjs() {
    local path files=()
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r path; do files+=("test262/$path"); done <"$activation"
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3do activation'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value activation_variants) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the R3do activation'
    fi
}

run_full_report() {
    local profile=$1 output=$2
    case $output in /*) ;; *) output=$root/$output ;; esac
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --all --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures >/dev/null
}

verify_full_join() {
    local parent=$1 candidate=$2 label=$3 counts expected
    local parent_json=${parent%.tsv}.jsonl candidate_json=${candidate%.tsv}.jsonl
    rows_for_paths "$activation" "$parent" >"$tmp/$label.parent.scope"
    rows_for_paths "$activation" "$candidate" >"$tmp/$label.candidate.scope"
    rows_without_paths "$activation" "$parent" >"$tmp/$label.parent.outside"
    rows_without_paths "$activation" "$candidate" >"$tmp/$label.candidate.outside"
    json_rows_for_paths "$activation" "$parent_json" >"$tmp/$label.parent.scope.json"
    json_rows_for_paths "$activation" "$candidate_json" >"$tmp/$label.candidate.scope.json"
    json_rows_without_paths "$activation" "$parent_json" >"$tmp/$label.parent.outside.json"
    json_rows_without_paths "$activation" "$candidate_json" >"$tmp/$label.candidate.outside.json"
    rows_for_paths "$activation" "$parent_report" >"$tmp/focused.parent"
    rows_for_paths "$activation" "$candidate_report" >"$tmp/focused.candidate"
    json_rows_for_paths "$activation" "${parent_report%.tsv}.jsonl" \
        >"$tmp/focused.parent.json"
    json_rows_for_paths "$activation" "${candidate_report%.tsv}.jsonl" \
        >"$tmp/focused.candidate.json"
    [[ "$(lines "$tmp/$label.parent.scope")" == "$(value full_scope_rows)" \
        && "$(lines "$tmp/$label.candidate.scope")" == "$(value full_scope_rows)" \
        && "$(lines "$tmp/$label.parent.outside")" == "$(value full_outside_rows)" \
        && "$(lines "$tmp/$label.candidate.outside")" == "$(value full_outside_rows)" ]] \
        || die "R3do $label full partition row counts drifted"
    diff -u "$tmp/focused.parent" "$tmp/$label.parent.scope"
    diff -u "$tmp/focused.candidate" "$tmp/$label.candidate.scope"
    diff -u "$tmp/focused.parent.json" "$tmp/$label.parent.scope.json"
    diff -u "$tmp/focused.candidate.json" "$tmp/$label.candidate.scope.json"
    diff -u "$tmp/$label.parent.outside" "$tmp/$label.candidate.outside"
    diff -u "$tmp/$label.parent.outside.json" "$tmp/$label.candidate.outside.json"

    counts=$(awk -F'\t' -v parent="$parent" '
        FILENAME==parent{
            if(!/^#/&&!($1=="path"&&$2=="variant")){
                key=$1 FS $2;if(key in old)exit 2;old[key]=$0;before++
            }
            next
        }
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old)||key in seen)exit 3
            split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 4
            if(a[7]!="pass"&&$7=="pass")gain++
            if(a[7]=="pass"&&$7!="pass")regress++
            if(old[key]!=$0){changed++;if(a[7]!=$7)outcome++;else detail++}
            seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 5
            printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d",changed,outcome,detail,before-changed,gain,regress}
    ' "$parent" "$candidate") || die "R3do $label exact full join failed"
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) gains=$(value full_pass_gains) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3do $label full transition drifted: $counts"
}

replay_full() {
    local parent_full=$preferred_parent_full
    if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
        || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
        || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
        parent_full=$generated_parent_full
        if [[ "$reuse_full_reports" == false || ! -f "$parent_full" \
            || ! -f "${parent_full%.tsv}.jsonl" \
            || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
            || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
            run_full_report "$parent_profile" "$parent_full"
        fi
    fi
    if [[ "$reuse_full_reports" == false || ! -f "$candidate_full" \
        || ! -f "${candidate_full%.tsv}.jsonl" \
        || "$(sha "$candidate_full")" != "$(value candidate_full_tsv_sha256)" \
        || "$(sha "${candidate_full%.tsv}.jsonl")" != "$(value candidate_full_jsonl_sha256)" ]]; then
        run_full_report "$candidate_profile" "$candidate_full"
    fi
    if [[ "$reuse_full_reports" == false || ! -f "$candidate_full_repeat" \
        || ! -f "${candidate_full_repeat%.tsv}.jsonl" \
        || "$(sha "$candidate_full_repeat")" != "$(value candidate_full_tsv_sha256)" \
        || "$(sha "${candidate_full_repeat%.tsv}.jsonl")" != "$(value candidate_full_jsonl_sha256)" ]]; then
        run_full_report "$candidate_profile" "$candidate_full_repeat"
    fi
    verify_full_report "$parent_full" "$(value parent_profile_sha256)" parent_full
    verify_full_report "$candidate_full" "$(value candidate_profile_sha256)" candidate_full
    verify_full_report "$candidate_full_repeat" "$(value candidate_profile_sha256)" candidate_full
    if ! cmp -s "$candidate_full" "$candidate_full_repeat" \
        || ! cmp -s "${candidate_full%.tsv}.jsonl" "${candidate_full_repeat%.tsv}.jsonl"; then
        die 'R3do independent candidate full runs are not byte-identical'
    fi
    [[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
        && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" \
        && "$(report_count unsupported-host-agent "$parent_full")" == \
            "$(value parent_full_unsupported_host_agent)" \
        && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
        && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" \
        && "$(report_count unsupported-host-agent "$candidate_full")" == \
            "$(value candidate_full_unsupported_host_agent)" ]] \
        || die 'R3do full outcome counts drifted'
    verify_full_join "$parent_full" "$candidate_full" first
    verify_full_join "$parent_full" "$candidate_full_repeat" repeat
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3do-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

if [[ -n "$runner_override" ]]; then
    runner=$runner_override
else
    cargo build --quiet --locked --release --bin run-test262
    target_dir=${CARGO_TARGET_DIR:-target}
    case $target_dir in /*) ;; *) target_dir=$root/$target_dir ;; esac
    runner=$target_dir/release/run-test262
fi
[[ -x "$runner" ]] || die "Test262 runner is not executable: $runner"

check_file "$baseline" "$baseline_lines" "$baseline_sha"
check_profiles
check_manifest_and_sources
check_receipts
check_history_and_canonical
verify_profile_handshake
verify_quickjs

case $mode in
    check) ;;
    focused) replay_focused ;;
    full) replay_focused; replay_full ;;
esac

if [[ "$mode" == full ]]; then
    printf 'R3do agent bounded wait A full: 44 new passes, 101993 outside rows unchanged, two byte-identical runs\n'
else
    printf 'R3do agent bounded wait A global: 44/44 activated pass, 42 retained agent diagnostics\n'
fi
