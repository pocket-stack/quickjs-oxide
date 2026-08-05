#!/usr/bin/env bash
# Reproduce the R3cv Array.prototype.flat/flatMap global Test262 admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-array-flatten-global-baseline.txt
predecessor_baseline=tests/test262-eval-wtf8-source-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-array-flatten-global-parent.conf
candidate_profile=tests/test262-array-flatten-global-candidate.conf
added_features=tests/test262-array-flatten-global-added-features.txt
manifest=tests/test262-array-flatten-global.txt
parent_report=tests/test262-array-flatten-global-parent.tsv
candidate_report=tests/test262-array-flatten-global-candidate.tsv
transition=tests/test262-array-flatten-global-transitions.tsv
parent_replay=target/test262-array-flatten-global-parent-replay.tsv
candidate_replay=target/test262-array-flatten-global-candidate-replay.tsv
preferred_parent_full=${TEST262_ARRAY_FLATTEN_PARENT_FULL:-target/test262-eval-wtf8-source-full.tsv}
generated_parent_full=target/test262-array-flatten-global-parent-full.tsv
candidate_full=target/test262-array-flatten-global-candidate-full.tsv
oracle_log=target/test262-array-flatten-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

baseline_lines=92
baseline_sha=e0cfd8b8250308e5f2658372c4703bf114b411a983db0d8d297b06175d68754f
predecessor_lines=85
predecessor_sha=d48300712eb54c05946ec7adb2117682621aab2e340a779d297f2eb425a20d1a

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen profiles, receipts, metadata, and QuickJS\n'
    printf '  --full   additionally replay and join the exact 102037-row candidate\n'
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
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$(value manifest_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value manifest_keys_sha256)" \
        && "$(report_rows "$report" | sha /dev/stdin)" == "$(value "${label}_rows_sha256")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "classified report drifted: $report"
}

verify_full_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$(value full_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "full classified report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3cv Array.prototype.flat/flatMap global admission transition.'
        echo "# parent_commit=$(value parent_commit)"
        echo "# parent_oxide_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_oxide_profile_sha256=$(value candidate_profile_sha256)"
        echo "# manifest_sha256=$(value manifest_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 3}
        ' "$before" "$after"
    } >"$output"
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if($7=="pass"&&$11!="pass")regress++
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,unchanged,regress}' "$1"
}

check_profiles() {
    check_file "$parent_profile" "$(value parent_profile_lines)" "$(value parent_profile_sha256)"
    check_file "$candidate_profile" "$(value candidate_profile_lines)" "$(value candidate_profile_sha256)"
    check_file "$live_profile" "$(value candidate_profile_lines)" "$(value candidate_profile_sha256)"
    cmp -s "$candidate_profile" "$live_profile" \
        || die 'live Test262 profile is not byte-identical to the R3cv candidate'
    for section in features audited-negative-tests execution; do
        profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
        profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
    done
    [[ "$(lines "$tmp/parent.features")" == "$(value parent_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value parent_features_sha256)" \
        && "$(lines "$tmp/candidate.features")" == "$(value candidate_features)" \
        && "$(sha "$tmp/candidate.features")" == "$(value candidate_features_sha256)" \
        && "$(lines "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests)" \
        && "$(sha "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" ]] \
        || die 'R3cv profile section inventory drifted'
    diff -u "$added_features" <(comm -13 "$tmp/parent.features" "$tmp/candidate.features")
    [[ -z "$(comm -23 "$tmp/parent.features" "$tmp/candidate.features")" ]] \
        || die 'R3cv candidate removed a parent feature'
    diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
    diff -u "$tmp/parent.execution" "$tmp/candidate.execution"
}

check_static_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$predecessor_baseline" "$predecessor_lines" "$predecessor_sha"
    check_file "$added_features" "$(value added_features)" "$(value added_features_sha256)"
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    sort -c "$manifest" || die 'R3cv manifest is not bytewise sorted'
    [[ -z "$(uniq -d "$manifest")" ]] || die 'R3cv manifest contains duplicates'
    check_profiles
    check_file "$parent_report" "$(value parent_focused_lines)" "$(value parent_focused_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" "$(value parent_focused_jsonl_lines)" "$(value parent_focused_jsonl_sha256)"
    check_file "$candidate_report" "$(value candidate_focused_lines)" "$(value candidate_focused_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_focused_jsonl_lines)" "$(value candidate_focused_jsonl_sha256)"
    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    verify_report "$parent_report" "$(value parent_profile_sha256)" parent_focused
    verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate_focused
    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" parent_oxide_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(header "$transition" candidate_oxide_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) regressions=$(value full_pass_regressions)" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
        || die 'R3cv transition or upstream binding drifted'
    [[ "$(predecessor_value profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(predecessor_value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value candidate_full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value candidate_full_summary)" == "$(value parent_full_summary)" \
        && "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" \
        && "$(( $(value candidate_full_runnable) - $(value parent_full_runnable) ))" == "$(value manifest_variants)" \
        && "$(( $(value candidate_full_passes) - $(value parent_full_passes) ))" == "$(value manifest_variants)" \
        && "$(( $(value parent_full_unsupported_feature) - $(value candidate_full_unsupported_feature) ))" == "$(value manifest_variants)" \
        && "$(( $(value full_changed) + $(value full_unchanged) ))" == "$(value full_variants)" \
        && "$(value full_changed)" == "$(value manifest_variants)" \
        && "$(value full_outcome_changed)" == "$(value manifest_variants)" \
        && "$(value full_detail_only)" == 0 \
        && "$(value full_pass_regressions)" == 0 ]] \
        || die 'R3cv full-vector anchors drifted'
}

verify_focused_semantics() {
    [[ "$(report_runnable "$parent_report")" == "$(value parent_focused_runnable)" \
        && "$(report_count unsupported-feature "$parent_report")" == "$(value parent_focused_unsupported_feature)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_focused_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_focused_passes)" ]] \
        || die 'R3cv focused outcome counts drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")&&
        !($2~/^(sloppy|strict)$/&&$7=="unsupported-feature"&&$8=="selection"&&
          $9=="EngineCapability"&&$10~/^quickjs-oxide does not declare Test262 feature support: /){exit 2}' \
        "$parent_report" || die 'R3cv parent selection frontier drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")&&
        !($2~/^(sloppy|strict)$/&&$7=="pass"&&$8=="normal"&&$9==""&&$10==""){exit 2}' \
        "$candidate_report" || die 'R3cv candidate semantics drifted'
}

check_metadata() {
    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata drifted'
    tr '\0' '\t' <"$tmp/metadata.bin" >"$tmp/metadata.tsv"
    awk -F'\t' '
        function has(list,value){return index("," list ",","," value ",")!=0}
        has($4,"Array.prototype.flat")||has($4,"Array.prototype.flatMap"){
            if(!($3==""||$3=="onlyStrict")||$5!=""||$6!="")bad=1
            print $1
        }
        END{if(bad)exit 2}
    ' "$tmp/metadata.tsv" | sort -u >"$tmp/derived.manifest" \
        || die 'R3cv metadata envelope gained unsupported flags or negatives'
    diff -u "$manifest" "$tmp/derived.manifest"
}

verify_quickjs() {
    local path
    local -a files=()
    while IFS= read -r path; do files+=("test262/$path"); done <"$manifest"
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3cv manifest'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_variants) tests:" "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the R3cv manifest'
    fi
}

run_report() {
    local profile=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$workers" \
        --allow-failures >/dev/null
}

run_full_report() {
    local profile=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --all --report "$output" --mode both \
        --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures >/dev/null
}

verify_full_join() {
    local parent=$1 candidate=$2 counts expected
    local parent_json=${parent%.tsv}.jsonl candidate_json=${candidate%.tsv}.jsonl
    rows_for_paths "$manifest" "$parent" >"$tmp/parent.universe"
    rows_for_paths "$manifest" "$candidate" >"$tmp/candidate.universe"
    rows_without_paths "$manifest" "$parent" >"$tmp/parent.non-universe"
    rows_without_paths "$manifest" "$candidate" >"$tmp/candidate.non-universe"
    report_rows "$parent_report" >"$tmp/focused.parent"
    report_rows "$candidate_report" >"$tmp/focused.candidate"
    json_rows_for_paths "$manifest" "$parent_json" >"$tmp/parent.universe.json"
    json_rows_for_paths "$manifest" "$candidate_json" >"$tmp/candidate.universe.json"
    json_rows_without_paths "$manifest" "$parent_json" >"$tmp/parent.non-universe.json"
    json_rows_without_paths "$manifest" "$candidate_json" >"$tmp/candidate.non-universe.json"
    awk '/^\{"kind":"result"/' "${parent_report%.tsv}.jsonl" >"$tmp/focused.parent.json"
    awk '/^\{"kind":"result"/' "${candidate_report%.tsv}.jsonl" >"$tmp/focused.candidate.json"
    [[ "$(lines "$tmp/parent.universe")" == "$(value full_universe_rows)" \
        && "$(lines "$tmp/candidate.universe")" == "$(value full_universe_rows)" \
        && "$(sha "$tmp/parent.universe")" == "$(value full_parent_universe_rows_sha256)" \
        && "$(sha "$tmp/candidate.universe")" == "$(value full_candidate_universe_rows_sha256)" \
        && "$(lines "$tmp/parent.non-universe")" == "$(value full_non_universe_rows)" \
        && "$(lines "$tmp/candidate.non-universe")" == "$(value full_non_universe_rows)" \
        && "$(sha "$tmp/parent.non-universe")" == "$(value full_non_universe_rows_sha256)" \
        && "$(sha "$tmp/candidate.non-universe")" == "$(value full_non_universe_rows_sha256)" \
        && "$(sha "$tmp/parent.universe.json")" == "$(value full_parent_universe_json_rows_sha256)" \
        && "$(sha "$tmp/candidate.universe.json")" == "$(value full_candidate_universe_json_rows_sha256)" \
        && "$(sha "$tmp/parent.non-universe.json")" == "$(value full_non_universe_json_rows_sha256)" \
        && "$(sha "$tmp/candidate.non-universe.json")" == "$(value full_non_universe_json_rows_sha256)" ]] \
        || die 'R3cv full universe partition drifted'
    diff -u "$tmp/focused.parent" "$tmp/parent.universe"
    diff -u "$tmp/focused.candidate" "$tmp/candidate.universe"
    diff -u "$tmp/parent.non-universe" "$tmp/candidate.non-universe"
    diff -u "$tmp/focused.parent.json" "$tmp/parent.universe.json"
    diff -u "$tmp/focused.candidate.json" "$tmp/candidate.universe.json"
    diff -u "$tmp/parent.non-universe.json" "$tmp/candidate.non-universe.json"
    counts=$(awk -F'\t' -v parent="$parent" '
        FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;before++}next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
            for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            if(a[7]=="pass"&&$7!="pass")regress++
            if(old[key]!=$0){changed++;if(a[7]!=$7)outcome++;else detail++}
            seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 4
            printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
    ' "$parent" "$candidate") || die 'R3cv exact full join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3cv full transition drifted: $counts"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-array-flatten.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
check_static_inputs
verify_focused_semantics
make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
check_metadata
verify_quickjs
if [[ "$mode" == check ]]; then
    echo 'R3cv inputs verified: 35 paths, 69 variants, QuickJS clean, exact two-feature delta.'
    exit 0
fi

run_report "$parent_profile" "$parent_replay"
run_report "$candidate_profile" "$candidate_replay"
diff -u "$parent_report" "$parent_replay"
diff -u "${parent_report%.tsv}.jsonl" "${parent_replay%.tsv}.jsonl"
diff -u "$candidate_report" "$candidate_replay"
diff -u "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl"
make_transition "$parent_replay" "$candidate_replay" "$tmp/replayed-transition.tsv"
diff -u "$transition" "$tmp/replayed-transition.tsv"
if [[ "$mode" != full ]]; then
    echo 'R3cv focused gate passes: QuickJS clean, Oxide 69/69, zero regressions.'
    exit 0
fi

parent_full=$preferred_parent_full
if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
    || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
    || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
    parent_full=$generated_parent_full
    run_full_report "$parent_profile" "$parent_full"
fi
if [[ "$reuse_full_reports" == false ]]; then
    run_full_report "$candidate_profile" "$candidate_full"
fi
verify_full_report "$parent_full" "$(value parent_profile_sha256)" parent_full
verify_full_report "$candidate_full" "$(value candidate_profile_sha256)" candidate_full
[[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" \
    && "$(report_count unsupported-feature "$parent_full")" == "$(value parent_full_unsupported_feature)" \
    && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" \
    && "$(report_count unsupported-feature "$candidate_full")" == "$(value candidate_full_unsupported_feature)" ]] \
    || die 'R3cv full outcome counts drifted'
verify_full_join "$parent_full" "$candidate_full"
check_static_inputs
echo 'R3cv full gate passes: 102037 rows, 69 new passes, 101968 byte-identical non-universe rows, zero regressions.'
