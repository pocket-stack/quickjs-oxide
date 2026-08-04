#!/usr/bin/env bash
# Reproduce the R3cl String.prototype.localeCompare runtime-parity receipt.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-string-locale-compare-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
predecessor_baseline=tests/test262-string-normalize-baseline.txt
successor_baseline=tests/test262-promise-try-with-resolvers-global-baseline.txt
successor_gate=scripts/test-test262-promise-try-with-resolvers-global.sh
upstream=compat/upstream.toml
profile=compat/test262-oxide.conf
universe=tests/test262-string-locale-compare-universe.txt
supplemental=tests/test262-string-locale-compare-supplemental.txt
manifest=tests/test262-string-locale-compare.txt
parent_report=tests/test262-string-locale-compare-parent.tsv
transition=tests/test262-string-locale-compare-transitions.tsv
candidate_report=target/test262-string-locale-compare-candidate.tsv
candidate_transition=target/test262-string-locale-compare-transitions.tsv
candidate_full=target/test262-string-locale-compare-full.tsv
preferred_parent_full=${TEST262_LOCALE_COMPARE_PARENT_FULL:-target/test262-string-normalize-full.tsv}
generated_parent_full=target/test262-string-locale-compare-parent-full.tsv
oracle_log=target/test262-string-locale-compare-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

baseline_lines=70
baseline_sha=de6a11a24acbb1ac3327c9d3a17b654dbd2e31ac91d915ae310eccbd7b5c9194
predecessor_baseline_lines=68
predecessor_baseline_sha=dec902e83fb7564825788c1a7dff9281341f1b05f92d645666f149aeae93c5e2

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen manifests, parent receipts, and canonical binding\n'
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
canonical_value() { value_from "$canonical_baseline" "$1"; }
predecessor_value() { value_from "$predecessor_baseline" "$1"; }
successor_value() { value_from "$successor_baseline" "$1"; }
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
toml_test262_value() {
    awk -v wanted="$2" '
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
    ' "$1"
}
check_file() {
    [[ -f "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated input drifted: $1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
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
variant_keys() {
    awk 'NF&&$1!~/^#/{print $0 "\tsloppy";print $0 "\tstrict"}' "$1" | sort
}
candidate_certified() {
    [[ "$(value candidate_commit)" != pending \
        && "$(value candidate_focused_tsv_sha256)" != pending \
        && "$(value candidate_focused_jsonl_sha256)" != pending \
        && "$(value candidate_focused_summary)" != pending \
        && "$(value transition_sha256)" != pending \
        && "$(value transition_data_sha256)" != pending \
        && "$(value candidate_full_tsv_sha256)" != pending \
        && "$(value candidate_full_jsonl_sha256)" != pending \
        && "$(value candidate_full_summary)" != pending ]]
}
verify_report_shape() {
    local report=$1 rows=$2 keys_sha=$3 expected_summary=$4
    local json=${report%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$(value oxide_profile_sha256)" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$rows" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$keys_sha" \
        && "$(report_summary "$report")" == "$(computed_summary "$report")" \
        && "$(report_summary "$report")" == "$expected_summary" ]] \
        || die "classified report shape drifted: $report"
}
verify_frozen_report() {
    local report=$1 rows=$2 keys_sha=$3 label=$4
    verify_report_shape "$report" "$rows" "$keys_sha" "$(value "${label}_summary")"
    [[ "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "${report%.tsv}.jsonl")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "classified report receipt drifted: $report"
}
make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3cl String.prototype.localeCompare runtime transition.'
        echo "# parent_commit=$(value parent_commit)"
        echo "# candidate_commit=$(value candidate_commit)"
        echo "# oxide_profile_sha256=$(value oxide_profile_sha256)"
        echo "# manifest_sha256=$(value gate_sha256)"
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
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' "$1"
}
parent_summary_json() {
    awk -v summary="$(value parent_full_summary)" 'BEGIN {
        count=split(summary,items," ")
        printf "%s","{\"kind\":\"summary\",\"outcomes\":{";
        for(item=1;item<=count;item++){
            separator=index(items[item],"=")
            key=substr(items[item],1,separator-1)
            value=substr(items[item],separator+1)
            printf "%s\"%s\":%s",(item==1 ? "" : ","),key,value
        }
        print "}}"
    }'
}

# The live runner cannot replay its own pre-localeCompare parent. Reconstruct
# R3ck by substituting the tracked 30-row parent receipt into the fresh
# candidate. Exact R3ck hashes authenticate every byte, including the 102,007
# rows outside this focused gate.
reconstruct_parent_full() {
    local candidate=$1 output=$2
    local candidate_json=${candidate%.tsv}.jsonl
    local output_json=${output%.tsv}.jsonl
    [[ -f "$candidate" && -f "$candidate_json" ]] \
        || die 'cannot reconstruct localeCompare parent before replaying the candidate full vector'

    awk -F'\t' -v parent="$parent_report" \
        -v summary="# summary $(value parent_full_summary)" '
        FILENAME==parent{
            if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0
            next
        }
        /^# summary /{print summary;next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2
            if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "$parent_report" "$candidate" >"$output" \
        || die 'could not reconstruct localeCompare parent TSV'

    awk -v parent="${parent_report%.tsv}.jsonl" \
        -v summary="$(parent_summary_json)" '
        function field(line,name, value){
            value=line
            sub(".*\\\"" name "\\\":\\\"","",value)
            sub("\\\".*","",value)
            return value
        }
        FILENAME==parent{
            if($0~/^\{\"kind\":\"result\"/){
                key=field($0,"path") SUBSEP field($0,"variant")
                old[key]=$0
            }
            next
        }
        /^\{\"kind\":\"summary\"/{print summary;next}
        /^\{\"kind\":\"result\"/{
            key=field($0,"path") SUBSEP field($0,"variant")
            if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "${parent_report%.tsv}.jsonl" "$candidate_json" >"$output_json" \
        || die 'could not reconstruct localeCompare parent JSONL'

    [[ "$(sha "$output")" == "$(value parent_full_tsv_sha256)" \
        && "$(sha "$output_json")" == "$(value parent_full_jsonl_sha256)" ]] \
        || die 'reconstructed localeCompare parent does not match canonical R3ck'
}

check_manifests() {
    for spec in universe:$universe supplemental:$supplemental gate:$manifest; do
        prefix=${spec%%:*}
        file=${spec#*:}
        check_file "$file" "$(value "${prefix}_paths")" "$(value "${prefix}_sha256")"
        sort -c "$file" || die "manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
        variant_keys "$file" >"$tmp/$prefix.keys"
        [[ "$(lines "$tmp/$prefix.keys")" == "$(value "${prefix}_variants")" \
            && "$(sha "$tmp/$prefix.keys")" == "$(value "${prefix}_keys_sha256")" ]] \
            || die "manifest variant keys drifted: $file"
    done
    cat "$universe" "$supplemental" | sort >"$tmp/gate.partition"
    diff -u "$manifest" "$tmp/gate.partition"
    [[ -z "$(uniq -d "$tmp/gate.partition")" ]] || die 'localeCompare manifests overlap'
}

bridge_r3cm_successor() {
    [[ "$(canonical_value tsv_sha256)" != "$(value candidate_full_tsv_sha256)" ]] \
        || return 0

    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$successor_baseline" 117 \
        2e553253d317438316a9cad9c9e8ea60f8bbe5db6809dee8c9530b9de3fba369
    [[ -x "$successor_gate" ]] \
        || die 'missing R3cm Promise proposal successor gate'
    [[ "$(successor_value quickjs)" == "$(value quickjs)" \
        && "$(successor_value test262)" == "$(value test262)" \
        && "$(successor_value test262_patch_sha256)" \
            == "$(value test262_patch_sha256)" \
        && "$(successor_value test262_config_sha256)" \
            == "$(value test262_config_sha256)" \
        && "$(successor_value test262_metadata_sha256)" \
            == "$(value test262_metadata_sha256)" \
        && "$(successor_value schema)" == "$(value schema)" \
        && "$(successor_value mode)" == "$(value mode)" \
        && "$(successor_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(successor_value runtime_commit)" \
            == 7c8f7c0fe82390b6aa4f24721b57df01b6ecde66 \
        && "$(successor_value parent_profile_sha256)" \
            == "$(value oxide_profile_sha256)" \
        && "$(successor_value full_variants)" == "$(value full_variants)" \
        && "$(successor_value full_keys_sha256)" == "$(value full_keys_sha256)" \
        && "$(successor_value parent_full_runnable)" \
            == "$(value anticipated_candidate_full_runnable)" \
        && "$(successor_value parent_full_passes)" \
            == "$(value anticipated_candidate_full_passes)" \
        && "$(successor_value parent_full_tsv_sha256)" \
            == "$(value candidate_full_tsv_sha256)" \
        && "$(successor_value parent_full_jsonl_sha256)" \
            == "$(value candidate_full_jsonl_sha256)" \
        && "$(successor_value parent_full_summary)" \
            == "$(value candidate_full_summary)" \
        && "$(successor_value full_changed)" == 36 \
        && "$(successor_value full_outcome_changed)" == 32 \
        && "$(successor_value full_detail_only)" == 4 \
        && "$(successor_value full_unchanged)" == 102001 \
        && "$(successor_value full_pass_regressions)" == 0 ]] \
        || die 'R3cm successor does not checksum-bridge the historical R3cl receipt'
    case $mode in
        check) "$successor_gate" --check ;;
        focused) "$successor_gate" ;;
        full) "$successor_gate" --full ;;
    esac
    echo 'Historical R3cl localeCompare receipt is checksum-bridged through the R3cm Promise proposal admission.'
    exit 0
}

check_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$predecessor_baseline" "$predecessor_baseline_lines" "$predecessor_baseline_sha"
    check_file "$profile" "$(value oxide_profile_lines)" "$(value oxide_profile_sha256)"
    check_file "$parent_report" 41 "$(value parent_focused_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" 32 "$(value parent_focused_jsonl_sha256)"
    check_manifests
    verify_frozen_report "$parent_report" "$(value gate_variants)" \
        "$(value gate_keys_sha256)" parent_focused
    [[ "$(toml_test262_value "$upstream" repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$(value test262)" \
        && "$(toml_test262_value "$upstream" patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value "$upstream" config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value "$upstream" test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" == "$(value oxide_profile_sha256)" ]] \
        || die 'localeCompare upstream binding drifted'
    [[ "$(predecessor_value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value candidate_full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value candidate_full_summary)" == "$(value parent_full_summary)" ]] \
        || die 'R3ck predecessor does not checksum-bind the localeCompare parent'
    [[ "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" ]] \
        || die 'canonical Test262 baseline schema drifted'
    if candidate_certified; then
        check_file "$transition" 36 "$(value transition_sha256)"
        [[ "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
            && "$(value candidate_focused_summary)" == "$(value anticipated_candidate_focused_summary)" \
            && "$(value candidate_full_summary)" == "$(value anticipated_candidate_full_summary)" \
            && "$(canonical_value runnable)" == "$(value anticipated_candidate_full_runnable)" \
            && "$(canonical_value passes)" == "$(value anticipated_candidate_full_passes)" \
            && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
            && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
            && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
            || die 'canonical Test262 baseline does not identify the localeCompare candidate'
    else
        [[ "$(canonical_value runnable)" == "$(value parent_full_runnable)" \
            && "$(canonical_value passes)" == "$(value parent_full_passes)" \
            && "$(canonical_value tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
            && "$(canonical_value jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
            && "$(canonical_value summary)" == "$(value parent_full_summary)" ]] \
            || die 'canonical Test262 baseline does not identify the localeCompare parent'
    fi
}

run_report() {
    local output=$1 scope=$2 pool=$3
    local -a args=(--suite "$suite" --config "$source_dir/test262.conf"
        --oxide-profile "$profile" --report "$output" --mode both
        --timeout-ms "$(value timeout_ms)" --workers "$pool" --allow-failures)
    if [[ "$scope" == full ]]; then args+=(--all); else args+=(--manifest "$manifest"); fi
    "$runner" "${args[@]}" >/dev/null
}
verify_quickjs() {
    local test_path
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$manifest"
    [[ -x "$source_dir/run-test262" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    if ! (cd -- "$source_dir" && ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the localeCompare gate'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_passes) tests:" "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the localeCompare gate'
    fi
}

cd -- "$root"
bridge_r3cm_successor
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-string-locale-compare.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_inputs
if [[ "$mode" == check ]]; then
    if candidate_certified; then
        echo 'R3cl localeCompare inputs verified: 15 paths, 30 variants, checksum-bound R3ck parent and candidate.'
    else
        echo 'R3cl localeCompare inputs verified: 15 paths, 30 variants, checksum-bound R3ck parent; candidate pending.'
    fi
    exit 0
fi

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
"$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
[[ "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata drifted'
find "$suite/test/built-ins/String/prototype/localeCompare" -type f -name '*.js' \
    | sed "s#^$suite/##" | sort >"$tmp/derived-universe"
diff -u "$universe" "$tmp/derived-universe"
while IFS= read -r test_path; do
    [[ -f "$suite/$test_path" ]] || die "localeCompare supplemental test disappeared: $test_path"
done <"$supplemental"

verify_quickjs
run_report "$candidate_report" focused "$workers"
verify_frozen_report "$parent_report" "$(value gate_variants)" \
    "$(value gate_keys_sha256)" parent_focused
verify_report_shape "$candidate_report" "$(value gate_variants)" \
    "$(value gate_keys_sha256)" "$(value anticipated_candidate_focused_summary)"
[[ "$(report_runnable "$parent_report")" == "$(value parent_focused_runnable)" \
    && "$(report_count pass "$parent_report")" == "$(value parent_focused_passes)" \
    && "$(report_count fail-runtime "$parent_report")" == "$(value parent_focused_fail_runtime)" \
    && "$(report_runnable "$candidate_report")" == "$(value anticipated_candidate_focused_runnable)" \
    && "$(report_count pass "$candidate_report")" == "$(value anticipated_candidate_focused_passes)" \
    && "$(report_count fail-runtime "$candidate_report")" == "$(value anticipated_candidate_focused_fail_runtime)" ]] \
    || die 'localeCompare focused runnable/pass counts drifted'
make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
expected_focused="changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged)"
[[ "$(transition_counts "$tmp/transition.tsv")" == "$expected_focused" ]] \
    || die 'localeCompare focused transition semantics drifted'
if candidate_certified; then
    verify_frozen_report "$candidate_report" "$(value gate_variants)" \
        "$(value gate_keys_sha256)" candidate_focused
    diff -u "$transition" "$tmp/transition.tsv"
    [[ "$(sha "$tmp/transition.tsv")" == "$(value transition_sha256)" \
        && "$(report_rows "$tmp/transition.tsv" | sha /dev/stdin)" == "$(value transition_data_sha256)" ]] \
        || die 'localeCompare focused receipt hashes drifted'
else
    cp "$tmp/transition.tsv" "$candidate_transition"
    echo "candidate_focused_tsv_sha256=$(sha "$candidate_report")"
    echo "candidate_focused_jsonl_sha256=$(sha "${candidate_report%.tsv}.jsonl")"
    echo "transition_sha256=$(sha "$candidate_transition")"
    echo "transition_data_sha256=$(report_rows "$candidate_transition" | sha /dev/stdin)"
fi

if [[ "$mode" != full ]]; then
    check_inputs
    echo 'R3cl localeCompare focused semantics pass: QuickJS 30/30, Oxide 30/30, 26 new passes.'
    exit 0
fi

if [[ "$reuse_full_reports" == false ]]; then run_report "$candidate_full" full "$full_workers"; fi
parent_full=$preferred_parent_full
if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
    || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
    || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
    parent_full=$generated_parent_full
    reconstruct_parent_full "$candidate_full" "$parent_full"
fi
verify_frozen_report "$parent_full" "$(value full_variants)" \
    "$(value full_keys_sha256)" parent_full
verify_report_shape "$candidate_full" "$(value full_variants)" \
    "$(value full_keys_sha256)" "$(value anticipated_candidate_full_summary)"
[[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" \
    && "$(report_count fail-runtime "$parent_full")" == "$(value parent_full_fail_runtime)" \
    && "$(report_runnable "$candidate_full")" == "$(value anticipated_candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value anticipated_candidate_full_passes)" \
    && "$(report_count fail-runtime "$candidate_full")" == "$(value anticipated_candidate_full_fail_runtime)" ]] \
    || die 'localeCompare full receipt counts drifted'
diff -u <(report_rows "$parent_report") \
    <(awk -F'\t' 'NR==FNR{p[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in p)' "$manifest" "$parent_full")
diff -u <(report_rows "$candidate_report") \
    <(awk -F'\t' 'NR==FNR{p[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in p)' "$manifest" "$candidate_full")
join_counts=$(awk -F'\t' -v parent="$parent_full" '
    FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;before++}next}
    !/^#/&&!($1=="path"&&$2=="variant"){
        key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
        for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
        different=old[key]!=$0;if(a[7]=="pass"&&$7!="pass")regress++
        if(different){changed++;if(a[7]!=$7)outcome++;else detail++}
        seen[key]=1
    }
    END{for(key in old)if(!(key in seen))exit 4
        printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
' "$parent_full" "$candidate_full") || die 'localeCompare full exact join failed'
expected_join="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
[[ "$join_counts" == "$expected_join" ]] \
    || die "localeCompare full no-regression join drifted: $join_counts"
if candidate_certified; then
    verify_frozen_report "$candidate_full" "$(value full_variants)" \
        "$(value full_keys_sha256)" candidate_full
else
    echo "candidate_full_tsv_sha256=$(sha "$candidate_full")"
    echo "candidate_full_jsonl_sha256=$(sha "${candidate_full%.tsv}.jsonl")"
fi
check_inputs
echo 'R3cl localeCompare full semantics pass: 102037 rows, 26 new passes, 102011 unchanged, zero pass regressions.'
