#!/usr/bin/env bash
# Reproduce the R3cj global admission of the residual DataView method and
# concrete numeric TypedArray metadata tags.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-binary-data-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
parent=tests/test262-binary-data-global-parent.conf
candidate=tests/test262-binary-data-global-candidate.conf
live_profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
added_features=tests/test262-binary-data-global-added-features.txt
universe=tests/test262-binary-data-global-universe.txt
activation=tests/test262-binary-data-global-activation.txt
authenticated=tests/test262-binary-data-global-authenticated.txt
supplemental=tests/test262-binary-data-global-supplemental.txt
reason_only=tests/test262-binary-data-global-reason-only.txt
config_skipped=tests/test262-binary-data-global-config-skipped.txt
transition=tests/test262-binary-data-global-transitions.tsv
activation_report=target/test262-binary-data-global-activation.tsv
parent_report=target/test262-binary-data-global-parent.tsv
candidate_report=target/test262-binary-data-global-candidate.tsv
r3ci_parent_full=target/test262-realm-hosts-global-candidate-full.tsv
generated_parent_full=target/test262-binary-data-global-parent-full.tsv
candidate_full=target/test262-binary-data-global-candidate-full.tsv
oracle_log=target/test262-binary-data-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen profiles, manifests, and receipts\n'
    printf '  --full   additionally replay the candidate and exact 102037-row join\n'
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
value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$baseline"
}
canonical_value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$canonical_baseline"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
section() {
    awk -v wanted="[$2]" \
        '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' \
        "$1"
}
toml_test262_value() {
    awk -v wanted="$2" '
        $0=="[test262]"{inside=1;next} /^\[/{inside=0}
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
verify_report() {
    local report=$1 profile_sha=$2 rows=$3 keys_sha=$4 label=$5
    local json=${report%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$rows" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$keys_sha" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" \
        && "$(report_summary "$report")" == "$(computed_summary "$report")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" ]] \
        || die "classified report receipt drifted: $report"
}
variant_keys() {
    awk 'NF&&$1!~/^#/{print $0 "\tsloppy";print $0 "\tstrict"}' "$1" | sort
}
make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3cj DataView-method/concrete-TypedArray global admission transition.'
        echo "# before_oxide_profile_sha256=$(value parent_oxide_profile_sha256)"
        echo "# after_oxide_profile_sha256=$(value candidate_oxide_profile_sha256)"
        echo "# manifest_sha256=$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
            !/^#/&&!($1=="path"&&$2=="variant"){
                split(old[$1 FS $2],a,FS)
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
            }' "$before" "$after"
    } >"$output"
}
transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' "$1"
}

check_profiles() {
    check_file "$parent" "$(value parent_profile_lines)" "$(value parent_oxide_profile_sha256)"
    check_file "$candidate" "$(value candidate_profile_lines)" "$(value candidate_oxide_profile_sha256)"
    check_file "$live_profile" "$(value candidate_profile_lines)" "$(value candidate_oxide_profile_sha256)"
    cmp -s "$candidate" "$live_profile" \
        || die 'live Test262 profile is not byte-identical to the R3cj candidate'
    section "$parent" features >"$tmp/parent.features"
    section "$candidate" features >"$tmp/candidate.features"
    section "$parent" audited-negative-tests >"$tmp/parent.negatives"
    section "$candidate" audited-negative-tests >"$tmp/candidate.negatives"
    section "$parent" execution >"$tmp/parent.execution"
    section "$candidate" execution >"$tmp/candidate.execution"
    [[ "$(lines "$tmp/parent.features")" == "$(value parent_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value parent_features_sha256)" \
        && "$(lines "$tmp/candidate.features")" == "$(value candidate_features)" \
        && "$(sha "$tmp/candidate.features")" == "$(value candidate_features_sha256)" \
        && "$(lines "$tmp/parent.negatives")" == "$(value audited_negative_tests)" \
        && "$(sha "$tmp/parent.negatives")" == "$(value audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" ]] \
        || die 'binary-data global profile sections drifted'
    diff -u "$added_features" <(comm -13 "$tmp/parent.features" "$tmp/candidate.features")
    [[ -z "$(comm -23 "$tmp/parent.features" "$tmp/candidate.features")" ]] \
        || die 'R3cj candidate removed a parent feature'
    diff -u "$tmp/parent.negatives" "$tmp/candidate.negatives"
    diff -u "$tmp/parent.execution" "$tmp/candidate.execution"
}

check_manifests() {
    check_file "$added_features" "$(value added_features)" "$(value added_features_sha256)"
    for spec in \
        "universe:$universe" "activation:$activation" \
        "authenticated:$authenticated" "supplemental:$supplemental" \
        "reason_only:$reason_only" "config_skipped:$config_skipped"
    do
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
    cat "$authenticated" "$supplemental" | sort >"$tmp/activation.partition"
    diff -u "$activation" "$tmp/activation.partition"
    [[ -z "$(uniq -d "$tmp/activation.partition")" ]] \
        || die 'authenticated and supplemental activation partitions overlap'
    cat "$activation" "$reason_only" "$config_skipped" | sort >"$tmp/universe.partition"
    diff -u "$universe" "$tmp/universe.partition"
    [[ -z "$(uniq -d "$tmp/universe.partition")" ]] \
        || die 'binary-data universe partitions overlap'
}

check_inputs() {
    check_profiles
    check_manifests
    check_file "$transition" 405 "$(value transition_sha256)"
    [[ "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(toml_test262_value "$upstream" repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$(value test262)" \
        && "$(toml_test262_value "$upstream" patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value "$upstream" config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" == "$(value candidate_oxide_profile_sha256)" ]] \
        || die 'binary-data upstream or transition binding drifted'
    [[ "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
        || die 'canonical Test262 baseline does not identify the R3cj candidate'
}

run_report() {
    local profile=$1 output=$2 manifest=$3 pool=$4
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode both --timeout-ms 30000 --workers "$pool" --allow-failures >/dev/null
}
run_full() {
    local profile=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --all --report "$output" --mode both \
        --timeout-ms 30000 --workers "$full_workers" --allow-failures >/dev/null
}
verify_quickjs() {
    local test_path
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$activation"
    [[ -x "$source_dir/run-test262" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    if ! (
        cd -- "$source_dir"
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}"
    ) >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3cj activation'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_activation_variants) tests:" "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the R3cj activation'
    fi
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-binary-data-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_inputs
if [[ "$mode" == check ]]; then
    echo 'R3cj binary-data global inputs verified: 200 paths, 400 variants, exact 104-to-122 feature delta.'
    exit 0
fi

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
"$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
[[ "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata drifted'
tr '\0' '\t' <"$tmp/metadata.bin" >"$tmp/metadata.tsv"
awk -F'\t' -v features="$added_features" '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    NR==FNR{wanted[$0]=1;next}
    {
        matched=0
        for(feature in wanted)if(has($4,feature))matched=1
        if(!matched)next
        if(has($3,"module")||has($3,"raw")||has($3,"onlyStrict")||
            has($3,"noStrict")||$5!=""||$6!="")bad=1
        print $1
    }
    END{if(bad)exit 1}
' "$added_features" "$tmp/metadata.tsv" | sort -u >"$tmp/derived-universe"
diff -u "$universe" "$tmp/derived-universe"
[[ "$(lines "$tmp/derived-universe")" == "$(value universe_paths)" \
    && "$(sha "$tmp/derived-universe")" == "$(value universe_sha256)" ]] \
    || die 'R3cj source-derived metadata universe drifted'

verify_quickjs
run_report "$candidate" "$activation_report" "$activation" "$workers"
verify_report "$activation_report" "$(value candidate_oxide_profile_sha256)" \
    "$(value activation_variants)" "$(value activation_keys_sha256)" oxide_activation
[[ "$(report_runnable "$activation_report")" == "$(value oxide_activation_runnable)" \
    && "$(report_count pass "$activation_report")" == "$(value oxide_activation_passes)" ]] \
    || die 'Oxide R3cj activation is not 386/386'

run_report "$parent" "$parent_report" "$universe" "$workers"
run_report "$candidate" "$candidate_report" "$universe" "$workers"
verify_report "$parent_report" "$(value parent_oxide_profile_sha256)" \
    "$(value universe_variants)" "$(value universe_keys_sha256)" parent_focused
verify_report "$candidate_report" "$(value candidate_oxide_profile_sha256)" \
    "$(value universe_variants)" "$(value universe_keys_sha256)" candidate_focused
[[ "$(report_runnable "$parent_report")" == "$(value parent_focused_runnable)" \
    && "$(report_runnable "$candidate_report")" == "$(value candidate_focused_runnable)" \
    && "$(report_count pass "$candidate_report")" == "$(value candidate_focused_passes)" ]] \
    || die 'R3cj focused runnable/pass counts drifted'
make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
[[ "$(sha "$tmp/transition.tsv")" == "$(value transition_sha256)" \
    && "$(report_rows "$tmp/transition.tsv" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
    && "$(transition_counts "$tmp/transition.tsv")" \
        == 'changed=396 outcome=386 detail=10 unchanged=4' ]] \
    || die 'R3cj focused transition semantics drifted'

if [[ "$mode" != full ]]; then
    check_inputs
    echo 'R3cj focused gate passes: QuickJS 386/386, Oxide 386/386, 10 reason-only and 4 config-skip variants retained.'
    exit 0
fi

parent_full=$r3ci_parent_full
if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
    || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
    || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
    parent_full=$generated_parent_full
    run_full "$parent" "$parent_full"
fi
if [[ "$reuse_full_reports" == false ]]; then
    run_full "$candidate" "$candidate_full"
fi
verify_report "$parent_full" "$(value parent_oxide_profile_sha256)" \
    "$(value full_variants)" "$(value full_keys_sha256)" parent_full
verify_report "$candidate_full" "$(value candidate_oxide_profile_sha256)" \
    "$(value full_variants)" "$(value full_keys_sha256)" candidate_full
[[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" \
    && "$(report_count unsupported-feature "$parent_full")" == "$(value parent_full_unsupported_feature)" \
    && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" \
    && "$(report_count unsupported-feature "$candidate_full")" == "$(value candidate_full_unsupported_feature)" ]] \
    || die 'R3cj full receipt counts drifted'

diff -u <(report_rows "$parent_report") \
    <(awk -F'\t' 'NR==FNR{p[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in p)' "$universe" "$parent_full")
diff -u <(report_rows "$candidate_report") \
    <(awk -F'\t' 'NR==FNR{p[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in p)' "$universe" "$candidate_full")

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
' "$parent_full" "$candidate_full") || die 'R3cj full join failed'
[[ "$join_counts" \
    == 'changed=396 outcome=386 detail=10 unchanged=101641 regressions=0' ]] \
    || die "R3cj full no-regression join drifted: $join_counts"
check_inputs
echo 'R3cj full gate passes: 102037 rows, 386 new passes, 10 reason-only changes, zero pass regressions.'
