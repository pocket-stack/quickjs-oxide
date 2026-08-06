#!/usr/bin/env bash
# Reproduce the R3dw profile-only global admission of `destructuring-assignment`.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-destructuring-assignment-global-baseline.txt
successor_baseline=tests/test262-module-static-core-baseline.txt
successor_gate=scripts/test-test262-module-static-core.sh
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-destructuring-assignment-global-parent.conf
candidate_profile=tests/test262-destructuring-assignment-global-candidate.conf
added_features=tests/test262-destructuring-assignment-global-added-features.txt
universe=tests/test262-destructuring-assignment-global-universe.txt
activation=tests/test262-destructuring-assignment-global-activation.txt
retained=tests/test262-destructuring-assignment-global-retained.txt
feature_skipped=tests/test262-destructuring-assignment-global-feature-skipped.txt
config_excluded=tests/test262-destructuring-assignment-global-config-excluded.txt
metadata_projection=tests/test262-destructuring-assignment-global-metadata.tsv
feature_partitions=tests/test262-destructuring-assignment-global-feature-partitions.tsv
host_hooks=tests/test262-destructuring-assignment-global-host-hooks.tsv
ledger=tests/test262-destructuring-assignment-global-ledger.tsv
source_ledger=tests/test262-destructuring-assignment-global-source.tsv
negative_manifest=tests/test262-destructuring-assignment-global-negative.txt
negative_ledger=tests/test262-destructuring-assignment-global-negative-ledger.tsv
quickjs_skipped=tests/test262-destructuring-assignment-global-quickjs-skipped.txt
quickjs_runnable=tests/test262-destructuring-assignment-global-quickjs-runnable.txt
quickjs_skips_ledger=tests/test262-destructuring-assignment-global-quickjs-skips.tsv
quickjs_receipt=tests/test262-destructuring-assignment-global-quickjs-receipt.txt
linux_receipt=tests/test262-destructuring-assignment-global-linux-receipt.txt
parent_report=tests/test262-destructuring-assignment-global-parent.tsv
candidate_report=tests/test262-destructuring-assignment-global-candidate.tsv
transition=tests/test262-destructuring-assignment-global-transitions.tsv
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
full_dir=${TEST262_FULL_REPORT_DIR:-target/test262-destructuring-assignment-global-full}
full_parent=${TEST262_FULL_PARENT_REPORT:-$full_dir/parent.tsv}
full_candidate_a=${TEST262_FULL_CANDIDATE_A_REPORT:-$full_dir/candidate-a.tsv}
full_candidate_b=${TEST262_FULL_CANDIDATE_B_REPORT:-$full_dir/candidate-b.tsv}
runner_override=${TEST262_RUNNER:-}

baseline_lines=223
baseline_sha=8944d20e029c55ab8038ea9207ea952d3e1f6aaeda75ada374e3aaacfc46723b
successor_lines=71
successor_sha=26713b79e1430244fc42479a2259d362e47e8fdb87a2e04eaf030b1dc45c77cb

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate inputs, metadata/source ledger, and pinned QuickJS\n'
    printf '  --full   additionally run parent once and candidate twice across all 102037 variants\n'
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
sha_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
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
successor_value() { value_from "$successor_baseline" "$1"; }
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
json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3dw JSONL projection %s: %s\n", report, message \
                >"/dev/stderr"
            exit 2
        }
        function expect(token) {
            if(substr(line,position,length(token))!=token) {
                fail("expected " token " at column " position)
            }
            position+=length(token)
        }
        function string_value(    character,escape,digits,value) {
            expect("\"");value=""
            while(position<=length(line)) {
                character=substr(line,position,1)
                if(character=="\""){position++;return value}
                if(character=="\\") {
                    position++
                    if(position>length(line))fail("unterminated escape")
                    escape=substr(line,position,1)
                    if(escape=="u") {
                        digits=substr(line,position+1,4)
                        if(length(digits)!=4||digits~/[^0123456789abcdefABCDEF]/) {
                            fail("invalid Unicode escape")
                        }
                        value=value "\\u" digits;position+=5
                    } else {
                        if(index("\"\\/bfnrt",escape)==0)fail("invalid string escape")
                        if(escape=="\"")value=value "\""
                        else if(escape=="/")value=value "/"
                        else if(escape=="b")value=value "\\u0008"
                        else if(escape=="f")value=value "\\u000c"
                        else value=value "\\" escape
                        position++
                    }
                    continue
                }
                if(character=="\t"||character=="\r")fail("unescaped control character")
                value=value character;position++
            }
            fail("unterminated string")
        }
        function project_result(    i,key,value) {
            line=$0;position=1;expect("{")
            for(i=1;i<=11;i++) {
                if(i!=1)expect(",")
                key=string_value()
                if(key!=name[i])fail("unexpected field " key " at position " i)
                expect(":");value=string_value()
                if(i==1) {
                    if(value!="result")fail("unexpected record kind")
                } else field[i-1]=value
            }
            expect("}")
            if(position!=length(line)+1)fail("trailing record data")
            print field[1],field[2],field[3],field[4],field[5],field[6], \
                field[7],field[8],field[9],field[10]
        }
        BEGIN {
            OFS="\t"
            name[1]="kind";name[2]="path";name[3]="variant";name[4]="flags"
            name[5]="features";name[6]="expected_phase";name[7]="expected_type"
            name[8]="outcome";name[9]="actual_phase";name[10]="actual_type"
            name[11]="detail"
        }
        /^\{"kind":"metadata",/{next}
        /^\{"kind":"result",/{project_result();next}
        /^\{"kind":"summary",/{next}
        {fail("unexpected record")}
    ' "$report"
}
verify_json_projection() {
    local report=$1 label=$2 json=${1%.tsv}.jsonl
    report_rows "$report" >"$tmp/$label.tsv-projection"
    json_result_projection "$json" >"$tmp/$label.json-projection" \
        || die "JSONL result projection failed: $json"
    diff -u "$tmp/$label.tsv-projection" "$tmp/$label.json-projection" \
        || die "JSONL/TSV projection drifted: $json"
}
rows_for_paths() {
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' "$1" "$2"
}
rows_without_paths() {
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&!($1 in wanted)' "$1" "$2"
}
ledger_keys() {
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        FNR==1{next}
        $1 in wanted{
            flags="," $3 ","
            if(index(flags,",onlyStrict,")){print $1 "\tstrict"}
            else if(index(flags,",module,")||index(flags,",noStrict,")||index(flags,",raw,")){
                print $1 "\tsloppy"
            } else {
                print $1 "\tsloppy";print $1 "\tstrict"
            }
        }
    ' "$1" "$ledger" | sort
}
toml_value() {
    awk -v wanted_section="[$1]" -v wanted_key="$2" '
        $0==wanted_section{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1);gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted_key)next
            answer=substr($0,separator+1);gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$upstream"
}

verify_report() {
    local report=$1 profile_sha=$2 role=$3 json=${1%.tsv}.jsonl
    verify_json_projection "$report" "$role-focused"
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$report")" == "$(value "${role}_report_lines")" \
        && "$(lines "$json")" == "$(value "${role}_jsonl_lines")" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
            == "$(value universe_variants)" \
        && "$(report_keys "$report" | sha_stream)" == "$(value universe_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${role}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${role}_summary")" \
        && "$(report_rows "$report" | sha_stream)" == "$(value "${role}_rows_sha256")" \
        && "$(sha "$report")" == "$(value "${role}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${role}_jsonl_sha256")" ]] \
        || die "classified report drifted: $report"
}

check_profiles() {
    check_file "$parent_profile" "$(value parent_profile_lines)" \
        "$(value parent_profile_sha256)"
    check_file "$candidate_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    check_file "$live_profile" "$(value live_profile_lines)" \
        "$(value live_profile_sha256)"
    check_file "$added_features" "$(value added_features)" \
        "$(value added_features_sha256)"
    sort -c "$added_features" || die 'R3dw added features are not bytewise sorted'
    cmp -s "$candidate_profile" "$live_profile" \
        || die 'R3dw candidate is not byte-identical to the canonical profile'

    for role in parent candidate; do
        local profile=$parent_profile
        [[ "$role" == candidate ]] && profile=$candidate_profile
        for section in features audited-negative-tests execution host-agent-tests; do
            profile_section "$section" "$profile" >"$tmp/$role.$section"
            [[ "$section" == execution ]] || sort -c "$tmp/$role.$section" \
                || die "$role $section section is not bytewise sorted"
        done
    done
    [[ "$(lines "$tmp/parent.features")" == "$(value parent_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value parent_features_sha256)" \
        && "$(lines "$tmp/candidate.features")" == "$(value candidate_features)" \
        && "$(sha "$tmp/candidate.features")" == "$(value candidate_features_sha256)" \
        && "$(lines "$tmp/parent.audited-negative-tests")" \
            == "$(value parent_audited_negatives)" \
        && "$(sha "$tmp/parent.audited-negative-tests")" \
            == "$(value parent_audited_negatives_sha256)" \
        && "$(lines "$tmp/candidate.audited-negative-tests")" \
            == "$(value candidate_audited_negatives)" \
        && "$(sha "$tmp/candidate.audited-negative-tests")" \
            == "$(value candidate_audited_negatives_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value parent_execution_entries)" \
        && "$(lines "$tmp/candidate.execution")" == "$(value candidate_execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" \
        && "$(sha "$tmp/candidate.execution")" == "$(value execution_sha256)" \
        && "$(lines "$tmp/parent.host-agent-tests")" == "$(value parent_host_agent_paths)" \
        && "$(lines "$tmp/candidate.host-agent-tests")" == "$(value candidate_host_agent_paths)" \
        && "$(sha "$tmp/parent.host-agent-tests")" == "$(value host_agent_paths_sha256)" \
        && "$(sha "$tmp/candidate.host-agent-tests")" == "$(value host_agent_paths_sha256)" ]] \
        || die 'R3dw profile inventory drifted'
    comm -13 "$tmp/parent.features" "$tmp/candidate.features" >"$tmp/added.features"
    comm -23 "$tmp/parent.features" "$tmp/candidate.features" >"$tmp/removed.features"
    comm -13 "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" >"$tmp/added.negatives"
    comm -23 "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" >"$tmp/removed.negatives"
    [[ ! -s "$tmp/removed.features" && ! -s "$tmp/removed.negatives" \
        && "$(lines "$tmp/added.features")" == "$(value added_features)" \
        && "$(sha "$tmp/added.features")" == "$(value added_features_sha256)" ]] \
        || die 'R3dw profile removes capability or adds the wrong feature'
    diff -u "$added_features" "$tmp/added.features"
    [[ "$(lines "$tmp/added.negatives")" == "$(value added_negative_paths)" \
        && "$(sha "$tmp/added.negatives")" == "$(value added_negative_sha256)" ]] \
        || die 'R3dw adds the wrong negative provenance'
    diff -u "$negative_manifest" "$tmp/added.negatives"
    cmp -s "$tmp/parent.execution" "$tmp/candidate.execution" \
        || die 'R3dw changes async execution policy'
    cmp -s "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
        || die 'R3dw changes agent host policy'
}

verify_manifest() {
    local name=$1 file=$2
    check_file "$file" "$(value "${name}_paths")" "$(value "${name}_sha256")"
    sort -c "$file" || die "manifest is not sorted: $file"
    [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
    ledger_keys "$file" >"$tmp/$name.keys"
    [[ "$(lines "$tmp/$name.keys")" == "$(value "${name}_variants")" \
        && "$(sha "$tmp/$name.keys")" == "$(value "${name}_keys_sha256")" ]] \
        || die "manifest variant keys drifted: $file"
}

check_manifests_and_receipts() {
    verify_manifest universe "$universe"
    verify_manifest activation "$activation"
    verify_manifest retained "$retained"
    verify_manifest feature_skipped "$feature_skipped"
    verify_manifest config_excluded "$config_excluded"
    verify_manifest negative "$negative_manifest"
    verify_manifest quickjs_skipped "$quickjs_skipped"
    verify_manifest quickjs_runnable "$quickjs_runnable"
    check_file "$metadata_projection" "$(value metadata_projection_lines)" \
        "$(value metadata_projection_sha256)"
    check_file "$feature_partitions" "$(value feature_partitions_lines)" \
        "$(value feature_partitions_sha256)"
    check_file "$host_hooks" "$(value host_hooks_lines)" \
        "$(value host_hooks_sha256)"
    check_file "$ledger" "$(value ledger_lines)" "$(value ledger_sha256)"
    check_file "$source_ledger" "$(value source_ledger_lines)" \
        "$(value source_ledger_sha256)"
    check_file "$negative_ledger" "$(value negative_ledger_lines)" \
        "$(value negative_ledger_sha256)"
    check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
        "$(value quickjs_receipt_sha256)"
    check_file "$linux_receipt" "$(value linux_receipt_lines)" \
        "$(value linux_receipt_sha256)"
    check_file "$quickjs_skips_ledger" "$(value quickjs_skips_ledger_lines)" \
        "$(value quickjs_skips_ledger_sha256)"
    check_file "$parent_report" "$(value parent_report_lines)" \
        "$(value parent_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
        "$(value parent_jsonl_sha256)"
    check_file "$candidate_report" "$(value candidate_report_lines)" \
        "$(value candidate_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
        "$(value candidate_jsonl_sha256)"
    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    sort "$activation" "$retained" "$feature_skipped" "$config_excluded" \
        >"$tmp/universe.partition"
    diff -u "$universe" "$tmp/universe.partition"
    [[ -z "$(uniq -d "$tmp/universe.partition")" ]] \
        || die 'R3dw universe partitions overlap'
    [[ -z "$(comm -23 "$negative_manifest" "$activation")" \
        && -z "$(comm -12 "$negative_manifest" "$retained")" ]] \
        || die 'R3dw audited negatives are not an exact activation subset'
    sort "$activation" "$retained" >"$tmp/runnable.partition"
    diff -u "$quickjs_runnable" "$tmp/runnable.partition"
    sort "$feature_skipped" "$config_excluded" >"$tmp/skipped.partition"
    diff -u "$quickjs_skipped" "$tmp/skipped.partition"
    sort "$quickjs_skipped" "$quickjs_runnable" >"$tmp/quickjs.partition"
    diff -u "$universe" "$tmp/quickjs.partition"
    [[ -z "$(comm -12 "$quickjs_skipped" "$quickjs_runnable")" ]] \
        || die 'QuickJS runnable and skipped manifests overlap'

    [[ "$(sed -n '1p' "$ledger")" \
            == $'path\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tvariants\tsource_sha256' \
        && "$(sed -n '2,$p' "$ledger" | sha_stream)" == "$(value ledger_rows_sha256)" \
        && "$(awk -F'\t' 'NR>1{s+=$7}END{print s+0}' "$ledger")" \
            == "$(value ledger_variants)" \
        && "$(awk -F'\t' 'NR>1&&$5!=""{n++}END{print n+0}' "$ledger")" \
            == "$(value raw_negative_paths)" ]] \
        || die 'R3dw source/metadata ledger drifted'
    sed -n '2,$p' "$ledger" | cut -f1 | diff -u "$universe" -
    sed -n '2,$p' "$ledger" | cut -f1-6 | diff -u "$metadata_projection" -
    awk -F'\t' 'NR>1{print $1 "\t" $8}' "$ledger" | diff -u "$source_ledger" -
    {
        sed -n '1p' "$ledger"
        awk -F'\t' 'NR==FNR{wanted[$0]=1;next}FNR>1&&($1 in wanted)' \
            "$negative_manifest" "$ledger"
    } >"$tmp/negative-ledger.tsv"
    diff -u "$negative_ledger" "$tmp/negative-ledger.tsv"
    comm -23 "$activation" "$negative_manifest" >"$tmp/normal"
    ledger_keys "$tmp/normal" >"$tmp/normal.keys"
    [[ "$(lines "$tmp/normal")" == "$(value normal_paths)" \
        && "$(lines "$tmp/normal.keys")" == "$(value normal_variants)" \
        && "$(awk -F'\t' 'NR==FNR{wanted[$0]=1;next}FNR>1&&($1 in wanted)&&($5!=""||$6!=""){n++}END{print n+0}' "$tmp/normal" "$ledger")" == 0 ]] \
        || die 'R3dw normal activation partition drifted'
    : >"$tmp/negative-generated"
    : >"$tmp/negative-only-strict"
    awk -F'\t' -v generated_paths="$(value negative_generated_paths)" \
        -v generated_variants="$(value negative_generated_variants)" \
        -v only_strict_paths="$(value negative_only_strict_paths)" \
        -v only_strict_variants="$(value negative_only_strict_variants)" \
        -v generated_file="$tmp/negative-generated" \
        -v only_strict_file="$tmp/negative-only-strict" '
        NR==FNR{wanted[$0]=1;next}FNR>1&&($1 in wanted){
        if($5!="parse"||$6!="SyntaxError")exit 2
        if(index(","$4",",",destructuring-assignment,")==0)exit 3
        if($3=="generated"&&$7==2){print $1 >generated_file;generated++;variants+=2}
        else if($3=="generated,onlyStrict"&&$7==1){print $1 >only_strict_file;only_strict++;strict_variants++}
        else exit 4
    }END{if(generated!=generated_paths||variants!=generated_variants \
        ||only_strict!=only_strict_paths||strict_variants!=only_strict_variants)exit 5}' \
        "$negative_manifest" "$ledger" \
        || die 'R3dw audited negative metadata drifted'
    [[ "$(sha "$tmp/negative-generated")" == "$(value negative_generated_sha256)" \
        && "$(sha "$tmp/negative-only-strict")" \
            == "$(value negative_only_strict_sha256)" ]] \
        || die 'R3dw audited negative flag partitions drifted'
    awk -F'\t' 'NR==FNR{wanted[$0]=1;next}FNR>1&&($1 in wanted){
        if($3!="module"||$5!="parse"||$6!="SyntaxError"||$7!=1)exit 2
        if(index(","$4",",",import.meta,")==0||index(","$4",",",destructuring-assignment,")==0)exit 3
        count++
    }END{if(count!=4)exit 4}' "$retained" "$ledger" \
        || die 'R3dw retained module metadata drifted'
    {
        printf 'feature\tpaths\tvariants\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{wanted[$0]=1;next} FNR==1{next}
            {
                count=split($4,features,",");matched=0
                for(i=1;i<=count;i++)if(features[i] in wanted){
                    paths[features[i]]++;variants[features[i]]+=$7;matched++
                }
                if(matched!=1)exit 2
            }
            END{for(feature in paths)print feature,paths[feature],variants[feature]}' \
            "$added_features" "$ledger" | sort
    } >"$tmp/feature-partitions.tsv"
    diff -u "$feature_partitions" "$tmp/feature-partitions.tsv"
    cut -f1 "$host_hooks" | diff -u "$universe" -
    [[ "$(awk -F'\t' '$2=="$262.createRealm"{n++}END{print n+0}' "$host_hooks")" \
            == "$(value host_create_realm_paths)" \
        && "$(awk -F'\t' '$2==""{n++}END{print n+0}' "$host_hooks")" \
            == "$(value host_hook_free_paths)" ]] \
        || die 'R3dw frozen host-hook inventory drifted'
    awk -F'\t' '$2!=""{exit 2}' "$host_hooks" \
        || die 'R3dw universe requires a host hook'

    for key in quickjs raw_paths raw_variants activation_paths activation_variants \
        retained_module_paths retained_module_variants raw_passes activation_passes \
        failures replays replay_status raw_log_lines activation_log_lines \
        log_projection_lines raw_log_projection_sha256 \
        activation_log_projection_sha256 raw_linux_arm64_log_sha256 \
        activation_linux_arm64_log_sha256 raw_macos_arm64_log_sha256 \
        activation_macos_arm64_log_sha256; do
        [[ "$(value_from "$quickjs_receipt" "$key")" \
                == "$(value "quickjs_receipt_$key")" ]] \
            || die "R3dw QuickJS receipt field drifted: $key"
    done
    [[ "$(value_from "$quickjs_receipt" quickjs)" == "$(value quickjs)" \
        && "$(value_from "$quickjs_receipt" raw_paths)" == "$(value universe_paths)" \
        && "$(value_from "$quickjs_receipt" raw_variants)" == "$(value universe_variants)" \
        && "$(value_from "$quickjs_receipt" activation_paths)" == "$(value activation_paths)" \
        && "$(value_from "$quickjs_receipt" activation_variants)" == "$(value activation_variants)" \
        && "$(value_from "$quickjs_receipt" retained_module_paths)" == "$(value retained_paths)" \
        && "$(value_from "$quickjs_receipt" retained_module_variants)" == "$(value retained_variants)" \
        && "$(value_from "$quickjs_receipt" raw_passes)" \
            == "$(value quickjs_raw_executed_variants)" \
        && "$(value_from "$quickjs_receipt" activation_passes)" \
            == "$(value quickjs_activation_executed_variants)" \
        && "$(value_from "$quickjs_receipt" failures)" == 0 \
        && "$(value_from "$quickjs_receipt" replays)" == 2 \
        && "$(value_from "$quickjs_receipt" replay_status)" == passed-twice \
        && "$(value_from "$quickjs_receipt" raw_log_lines)" \
            == "$(value quickjs_raw_log_lines)" \
        && "$(value_from "$quickjs_receipt" activation_log_lines)" \
            == "$(value quickjs_activation_log_lines)" \
        && "$(value_from "$quickjs_receipt" log_projection_lines)" \
            == "$(value quickjs_log_projection_lines)" \
        && "$(value_from "$quickjs_receipt" raw_log_projection_sha256)" \
            == "$(value quickjs_raw_log_projection_sha256)" \
        && "$(value_from "$quickjs_receipt" activation_log_projection_sha256)" \
            == "$(value quickjs_activation_log_projection_sha256)" \
        && "$(value_from "$quickjs_receipt" raw_linux_arm64_log_sha256)" \
            == "$(value quickjs_raw_linux_arm64_log_sha256)" \
        && "$(value_from "$quickjs_receipt" activation_linux_arm64_log_sha256)" \
            == "$(value quickjs_activation_linux_arm64_log_sha256)" \
        && "$(value_from "$quickjs_receipt" raw_macos_arm64_log_sha256)" \
            == "$(value quickjs_raw_macos_arm64_log_sha256)" \
        && "$(value_from "$quickjs_receipt" activation_macos_arm64_log_sha256)" \
            == "$(value quickjs_activation_macos_arm64_log_sha256)" ]] \
        || die 'R3dw QuickJS receipt is not cross-bound to the primary inventory'
    for key in image image_id os arch rustc cargo quickjs_raw_log_sha256 \
        quickjs_activation_log_sha256 parent_runnable parent_passes \
        parent_tsv_sha256 parent_jsonl_sha256 candidate_runnable candidate_passes \
        candidate_tsv_sha256 candidate_jsonl_sha256 candidate_replays \
        candidate_replay_status native_parity; do
        [[ "$(value_from "$linux_receipt" "$key")" \
                == "$(value "linux_receipt_$key")" ]] \
            || die "R3dw Linux receipt field drifted: $key"
    done
    [[ "$(value_from "$linux_receipt" quickjs_raw_log_sha256)" \
            == "$(value quickjs_raw_linux_arm64_log_sha256)" \
        && "$(value_from "$linux_receipt" quickjs_activation_log_sha256)" \
            == "$(value quickjs_activation_linux_arm64_log_sha256)" \
        && "$(value_from "$linux_receipt" parent_tsv_sha256)" \
            == "$(value full_parent_tsv_sha256)" \
        && "$(value_from "$linux_receipt" parent_jsonl_sha256)" \
            == "$(value full_parent_jsonl_sha256)" \
        && "$(value_from "$linux_receipt" parent_runnable)" \
            == "$(value full_parent_runnable)" \
        && "$(value_from "$linux_receipt" parent_passes)" \
            == "$(value full_parent_passes)" \
        && "$(value_from "$linux_receipt" candidate_tsv_sha256)" \
            == "$(value full_candidate_tsv_sha256)" \
        && "$(value_from "$linux_receipt" candidate_jsonl_sha256)" \
            == "$(value full_candidate_jsonl_sha256)" \
        && "$(value_from "$linux_receipt" candidate_runnable)" \
            == "$(value full_candidate_runnable)" \
        && "$(value_from "$linux_receipt" candidate_passes)" \
            == "$(value full_candidate_passes)" \
        && "$(value_from "$linux_receipt" candidate_replays)" \
            == "$(value full_candidate_replays)" \
        && "$(value_from "$linux_receipt" candidate_replay_status)" \
            == "$(value full_candidate_replay_status)" \
        && "$(value_from "$linux_receipt" native_parity)" == byte-identical ]] \
        || die 'R3dw Linux/native parity receipt is not bound to canonical reports'

    verify_report "$parent_report" "$(value parent_profile_sha256)" parent
    verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
    [[ "$(awk -F'\t' '!/^#/&&$1!="path"{print}' "$transition" | sha_stream)" \
            == "$(value transition_data_sha256)" ]] \
        || die 'R3dw transition data drifted'
}

derive_metadata_and_sources() {
    local quickjs_parser=$source_dir/quickjs.c
    check_file "$quickjs_parser" "$(value quickjs_parser_lines)" \
        "$(value quickjs_parser_sha256)"
    grep -Fq 'static int __exception js_parse_property_name(JSParseState *s,' \
        "$quickjs_parser" \
        && grep -Fq 'static __exception int get_lvalue(JSParseState *s,' \
            "$quickjs_parser" \
        && grep -Fq 'static int js_parse_destructuring_element(JSParseState *s, int tok, int is_arg,' \
            "$quickjs_parser" \
        && grep -Fq 'static __exception int js_parse_assign_expr2(JSParseState *s, int parse_flags)' \
            "$quickjs_parser" \
        || die 'R3dw pinned QuickJS parser reference drifted'
    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned exhaustive Test262 metadata drifted'
    tr '\0' '\t' <"$tmp/metadata.bin" >"$tmp/metadata.tsv"
    awk -F'\t' 'function has(list,value){return index(","list",",","value",")!=0}
        NR==FNR{wanted[$0]=1;next}
        {for(feature in wanted)if(has($4,feature)){print $1;next}}' \
        "$added_features" "$tmp/metadata.tsv" | sort -u >"$tmp/universe"
    diff -u "$universe" "$tmp/universe"
    awk -F'\t' 'function has(list,value){return index(","list",",","value",")!=0}
        NR==FNR{wanted[$0]=1;next}
        {for(feature in wanted)if(has($4,feature)){print;next}}' \
        "$added_features" "$tmp/metadata.tsv" | sort \
        >"$tmp/metadata-projection.tsv"
    diff -u "$metadata_projection" "$tmp/metadata-projection.tsv"

    if command -v sha256sum >/dev/null 2>&1; then
        while IFS= read -r test_path; do printf '%s\0' "$suite/$test_path"; done <"$universe" \
            | xargs -0 sha256sum >"$tmp/source-hashes.raw"
    else
        while IFS= read -r test_path; do printf '%s\0' "$suite/$test_path"; done <"$universe" \
            | xargs -0 shasum -a 256 >"$tmp/source-hashes.raw"
    fi
    awk -v prefix="$suite/" 'BEGIN{OFS="\t"}
        {at=index($0,prefix);if(!at)exit 2;print substr($0,at+length(prefix)),$1}' \
        "$tmp/source-hashes.raw" | sort >"$tmp/source-hashes.tsv"
    diff -u "$source_ledger" "$tmp/source-hashes.tsv"
    awk -F'\t' 'BEGIN{OFS="\t"}
        FILENAME==ARGV[1]{hash[$1]=$2;next}
        FILENAME==ARGV[2]{wanted[$0]=1;next}
        function has(list,value){return index(","list",",","value",")!=0}
        {
            matched=0
            for(feature in wanted)if(has($4,feature))matched++
            if(!matched)next
            if(matched!=1)exit 4
            variants=(has($3,"module")||has($3,"noStrict")||has($3,"raw")||has($3,"onlyStrict"))?1:2
            if(!($1 in hash))exit 2
            print $1,$2,$3,$4,$5,$6,variants,hash[$1];seen[$1]=1
        }
        END{for(test_path in hash)if(!(test_path in seen))exit 3}
    ' "$tmp/source-hashes.tsv" "$added_features" "$tmp/metadata.tsv" \
        | sort >"$tmp/ledger.rows"
    {
        printf 'path\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tvariants\tsource_sha256\n'
        sed -n '1,$p' "$tmp/ledger.rows"
    } >"$tmp/ledger.tsv"
    diff -u "$ledger" "$tmp/ledger.tsv"

    : >"$tmp/host-hooks.tsv"
    while IFS= read -r test_path; do
        local source=$suite/$test_path
        if grep -Eq '\$262\.|\$DONE' "$source"; then
            die "R3dw source requires a host or async hook: $test_path"
        fi
        printf '%s\t\n' "$test_path" >>"$tmp/host-hooks.tsv"
    done <"$universe"
    diff -u "$host_hooks" "$tmp/host-hooks.tsv"

    awk '$0=="[features]"{inside=1;next}/^\[/{inside=0}
        inside&&/=skip$/{sub(/=skip$/,"");print}' "$source_dir/test262.conf" \
        | sort -u >"$tmp/quickjs-skip-features"
    awk '$0=="[exclude]"{inside=1;next}/^\[/{inside=0}
        inside{sub(/#.*/,"");gsub(/^[[:space:]]+|[[:space:]]+$/,"");if(NF)print}' \
        "$source_dir/test262.conf" | sort -u >"$tmp/quickjs-excludes"
    : >"$tmp/feature-skipped.raw"
    : >"$tmp/config-excluded.raw"
    : >"$tmp/quickjs-skip-occurrences"
    awk -F'\t' -v skipped="$tmp/feature-skipped.raw" \
        -v occurrences="$tmp/quickjs-skip-occurrences" '
        NR==FNR{skip[$1]=1;next} FNR==1{next}
        {
            count=split($4,features,",");matched=0
            for(i=1;i<=count;i++)if(features[i] in skip){
                print $1 > skipped
                print "feature\t" features[i] "\t" $1 "\t" $7 >> occurrences
                matched++
            }
            if(matched>1)exit 2
        }
    ' "$tmp/quickjs-skip-features" "$tmp/ledger.tsv"
    awk -F'\t' -v skipped="$tmp/config-excluded.raw" \
        -v occurrences="$tmp/quickjs-skip-occurrences" '
        NR==FNR{exclude[++count]=$1;next} FNR==1{next}
        {
            matched=0
            for(i=1;i<=count;i++){
                normalized=exclude[i];sub(/^test262\//,"",normalized)
                if(substr(normalized,length(normalized),1)=="/")hit=index($1,normalized)==1
                else hit=$1==normalized
                if(hit){
                    print $1 > skipped
                    print "config-exclude\t" exclude[i] "\t" $1 "\t" $7 >> occurrences
                    matched++
                }
            }
            if(matched>1)exit 2
        }
    ' "$tmp/quickjs-excludes" "$tmp/ledger.tsv"
    sort -u "$tmp/feature-skipped.raw" >"$tmp/feature-skipped-all"
    sort -u "$tmp/config-excluded.raw" >"$tmp/config-excluded"
    comm -23 "$tmp/feature-skipped-all" "$tmp/config-excluded" \
        >"$tmp/feature-skipped"
    diff -u "$feature_skipped" "$tmp/feature-skipped"
    diff -u "$config_excluded" "$tmp/config-excluded"
    [[ -z "$(comm -12 "$tmp/feature-skipped" "$tmp/config-excluded")" ]] \
        || die 'R3dw QuickJS feature and config skip partitions overlap'
    awk -F'\t' 'NR==FNR{excluded[$1]=1;next}
        !($1=="feature"&&($3 in excluded)){print}' \
        "$tmp/config-excluded" "$tmp/quickjs-skip-occurrences" \
        >"$tmp/quickjs-skip-occurrences.filtered"
    sort -u "$tmp/feature-skipped" "$tmp/config-excluded" >"$tmp/quickjs-skipped"
    comm -23 "$universe" "$tmp/quickjs-skipped" >"$tmp/quickjs-runnable"
    diff -u "$quickjs_skipped" "$tmp/quickjs-skipped"
    diff -u "$quickjs_runnable" "$tmp/quickjs-runnable"
    {
        printf 'kind\tselector\tpaths\tvariants\n'
        awk -F'\t' '{paths[$1 SUBSEP $2 SUBSEP $3]=1;variants[$1 SUBSEP $2]+=$4}
            END{for(group in variants){split(group,g,SUBSEP);n=0;for(key in paths){split(key,a,SUBSEP);if(a[1]==g[1]&&a[2]==g[2])n++}
                print g[1] "\t" g[2] "\t" n "\t" variants[group]}}' \
            "$tmp/quickjs-skip-occurrences.filtered" | sort
    } >"$tmp/quickjs-skips.tsv"
    diff -u "$quickjs_skips_ledger" "$tmp/quickjs-skips.tsv"
}

derive_activation_and_retained() {
    awk -F'\t' '!/^#/&&$1!="path"&&$7=="pass"{print $1}' \
        "$candidate_report" | sort -u >"$tmp/activation"
    diff -u "$activation" "$tmp/activation"
    awk -F'\t' '!/^#/&&$1!="path"&&$7=="unsupported-module"{print $1}' \
        "$candidate_report" | sort -u >"$tmp/retained"
    diff -u "$retained" "$tmp/retained"
}

verify_quickjs_pair() {
    local label=$1 manifest=$2 variants=$3
    local log_a=$root/target/test262-destructuring-assignment-global-quickjs-$label-a.log
    local log_b=$root/target/test262-destructuring-assignment-global-quickjs-$label-b.log
    local test_path platform_log_sha= projection
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$manifest"
    rm -f -- "$log_a" "$log_b"
    for log in "$log_a" "$log_b"; do
        if ! (cd -- "$source_dir" \
            && ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
            >"$log" 2>&1; then
            tail -n 100 "$log" >&2
            die "pinned QuickJS could not execute the R3dw $label universe"
        fi
        if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$log" \
            || ! grep -Fq "Average memory statistics for $variants tests:" "$log" \
            || [[ "$(lines "$log")" != "$(value "quickjs_${label}_log_lines")" ]]; then
            tail -n 100 "$log" >&2
            die "pinned QuickJS R3dw $label result drifted"
        fi
        projection=$tmp/$(basename "$log").projection
        awk '/^test262\.conf:.* ignoring testdir=/{print}
            /^Average memory statistics for [0-9]+ tests:/{print}' "$log" >"$projection"
        [[ "$(lines "$projection")" == "$(value quickjs_log_projection_lines)" \
            && "$(sha "$projection")" \
                == "$(value "quickjs_${label}_log_projection_sha256")" ]] \
            || die "pinned QuickJS R3dw $label semantic projection drifted"
    done
    [[ ! -L "$log_a" && ! -L "$log_b" && ! "$log_a" -ef "$log_b" ]] \
        || die "R3dw QuickJS $label replay logs must be distinct regular files"
    cmp -s "$log_a" "$log_b" \
        || die "R3dw QuickJS $label replays are not byte-identical"
    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) platform_log_sha=$(value "quickjs_${label}_macos_arm64_log_sha256") ;;
        Linux-aarch64) platform_log_sha=$(value "quickjs_${label}_linux_arm64_log_sha256") ;;
    esac
    [[ -z "$platform_log_sha" || "$(sha "$log_a")" == "$platform_log_sha" ]] \
        || die "R3dw QuickJS $label platform receipt drifted"
}

verify_quickjs() {
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    verify_quickjs_pair raw "$quickjs_runnable" "$(value quickjs_raw_executed_variants)"
    verify_quickjs_pair activation "$activation" \
        "$(value quickjs_activation_executed_variants)"
}

transition_counts() {
    awk -F'\t' '!/^#/&&$1!="path"{
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
        if($7!="pass"&&$11=="pass")gains++
        if($7=="pass"&&$11!="pass")regressions++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d", \
        changed,outcome,detail,unchanged,gains,regressions}' "$1"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        printf '%s\n' '# Exhaustive R3dw global destructuring-assignment admission transition.'
        printf '# parent_profile_sha256=%s\n' "$(value parent_profile_sha256)"
        printf '# candidate_profile_sha256=%s\n' "$(value candidate_profile_sha256)"
        printf '# universe_sha256=%s\n' "$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10;seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 3}
        ' "$before" "$after"
    } >"$output"
}

verify_focused_semantics() {
    [[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
        && "$(report_count pass "$parent_report")" == "$(value parent_passes)" \
        && "$(report_count unsupported-feature "$parent_report")" \
            == "$(value parent_unsupported_feature)" \
        && "$(report_count unsupported-module "$parent_report")" \
            == "$(value parent_unsupported_module)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
        && "$(report_count unsupported-feature "$candidate_report")" \
            == "$(value candidate_unsupported_feature)" \
        && "$(report_count unsupported-module "$candidate_report")" \
            == "$(value candidate_unsupported_module)" ]] \
        || die 'R3dw focused outcome counts drifted'
    awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted){
            if(!($7=="unsupported-feature"&&$8=="selection"&&$9=="EngineCapability" \
                &&$10=="quickjs-oxide does not declare Test262 feature support: destructuring-assignment"))exit 2
            count++
        }END{if(count!=213)exit 3}' "$activation" "$parent_report" \
        || die 'R3dw parent capability frontier drifted'
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted)&&
        !($7=="pass"&&$8==$5&&$9==$6){exit 2}' \
        "$activation" "$candidate_report" \
        || die 'R3dw candidate activation semantics drifted'
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted){
            if(!($7=="unsupported-module"&&$8=="selection"&&$9=="ExecutionMode") \
                ||$10!="missing execution capabilities: module")exit 2
            variants++
        }
        END{if(variants!=4)exit 3}' \
        "$retained" "$candidate_report" \
        || die 'R3dw retained module frontier drifted'
    rows_for_paths "$retained" "$parent_report" >"$tmp/retained-parent.rows"
    rows_for_paths "$retained" "$candidate_report" >"$tmp/retained-candidate.rows"
    diff -u "$tmp/retained-parent.rows" "$tmp/retained-candidate.rows"
    rows_for_paths "$negative_manifest" "$candidate_report" >"$tmp/negative.rows"
    [[ "$(lines "$tmp/negative.rows")" == "$(value negative_variants)" \
        && "$(awk -F'\t' '$7=="pass"&&$8=="parse"&&$9=="SyntaxError"{n++}END{print n+0}' "$tmp/negative.rows")" \
            == "$(value negative_variants)" \
        && "$(awk -F'\t' '$10=="invalid destructuring target"{n++}END{print n+0}' "$tmp/negative.rows")" == 82 \
        && "$(awk -F'\t' '$10=="unexpected token in expression: \047=\047"{n++}END{print n+0}' "$tmp/negative.rows")" == 82 ]] \
        || die 'R3dw negative parse receipt drifted'
    local expected="changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)"
    [[ "$(transition_counts "$transition")" == "$expected" ]] \
        || die 'R3dw focused transition counts drifted'
}

run_report() {
    local profile=$1 output=$2 scope=$3 pool=$4
    local -a args=(--suite "$suite" --config "$source_dir/test262.conf"
        --oxide-profile "$profile" --report "$output" --mode "$(value mode)"
        --timeout-ms "$(value timeout_ms)" --workers "$pool" --allow-failures)
    if [[ "$scope" == full ]]; then args+=(--all); else args+=(--manifest "$universe"); fi
    "$runner" "${args[@]}" >/dev/null
}

verify_full_report() {
    local report=$1 profile_sha=$2 role=$3
    local json=${report%.tsv}.jsonl
    verify_json_projection "$report" "$(basename "${report%.tsv}")-full"
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" \
            == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" \
            == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" \
            == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$report")" == "$(value full_report_lines)" \
        && "$(lines "$json")" == "$(value full_jsonl_lines)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
            == "$(value full_variants)" \
        && "$(report_keys "$report" | sha_stream)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${role}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${role}_summary")" \
        && "$(report_runnable "$report")" == "$(value "${role}_runnable")" \
        && "$(report_count pass "$report")" == "$(value "${role}_passes")" \
        && "$(sha "$report")" == "$(value "${role}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${role}_jsonl_sha256")" ]] \
        || die "full classified report drifted: $report"
}

verify_full_join() {
    local parent=$1 candidate=$2 counts expected
    rows_for_paths "$universe" "$parent" >"$tmp/full-parent-scope.rows"
    rows_for_paths "$universe" "$candidate" >"$tmp/full-candidate-scope.rows"
    rows_without_paths "$universe" "$parent" >"$tmp/full-parent-outside.rows"
    rows_without_paths "$universe" "$candidate" >"$tmp/full-candidate-outside.rows"
    diff -u <(report_rows "$parent_report") "$tmp/full-parent-scope.rows"
    diff -u <(report_rows "$candidate_report") "$tmp/full-candidate-scope.rows"
    diff -u "$tmp/full-parent-outside.rows" "$tmp/full-candidate-outside.rows"
    [[ "$(lines "$tmp/full-parent-scope.rows")" == "$(value full_scope_variants)" \
        && "$(lines "$tmp/full-parent-outside.rows")" == "$(value full_outside_variants)" ]] \
        || die 'R3dw full scope partition drifted'
    counts=$(awk -F'\t' -v parent="$parent" '
        FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;before++}next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
            for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            if(a[7]=="pass"&&$7!="pass")regressions++
            if(a[7]!="pass"&&$7=="pass")gains++
            if(old[key]!=$0){changed++;if(a[7]!=$7)outcome++;else detail++};seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 4
            printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d", \
                changed,outcome,detail,before-changed,gains,regressions}
    ' "$parent" "$candidate") || die 'R3dw full exact join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) gains=$(value full_pass_gains) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3dw full transition drifted: $counts"
}

bridge_r3dx_successor() {
    [[ "$(canonical_value tsv_sha256)" != "$(value full_candidate_tsv_sha256)" ]] \
        || return 0

    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$parent_profile" "$(value parent_profile_lines)" \
        "$(value parent_profile_sha256)"
    check_file "$candidate_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    check_manifests_and_receipts

    check_file "$successor_baseline" "$successor_lines" "$successor_sha"
    check_file "$canonical_baseline" 8 \
        "$(successor_value candidate_canonical_baseline_sha256)"
    [[ -x "$successor_gate" \
        && "$(successor_value quickjs)" == "$(value quickjs)" \
        && "$(successor_value test262)" == "$(value test262)" \
        && "$(successor_value test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(successor_value test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(successor_value test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(successor_value parent_commit)" == 0aad838791d2f1c13f8a0ceb9e2d69f7b5260c31 \
        && "$(successor_value parent_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(successor_value parent_canonical_baseline_sha256)" \
            == "$(value canonical_full_baseline_sha256)" \
        && "$(successor_value full_variants)" == "$(value full_variants)" \
        && "$(successor_value full_keys_sha256)" == "$(value full_keys_sha256)" \
        && "$(successor_value full_parent_runnable)" == "$(value full_candidate_runnable)" \
        && "$(successor_value full_parent_passes)" == "$(value full_candidate_passes)" \
        && "$(successor_value full_parent_tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
        && "$(successor_value full_parent_jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
        && "$(successor_value full_parent_summary)" == "$(value full_candidate_summary)" \
        && "$(successor_value full_candidate_runnable)" == "$(canonical_value runnable)" \
        && "$(successor_value full_candidate_passes)" == "$(canonical_value passes)" \
        && "$(successor_value full_candidate_tsv_sha256)" == "$(canonical_value tsv_sha256)" \
        && "$(successor_value full_candidate_jsonl_sha256)" == "$(canonical_value jsonl_sha256)" \
        && "$(successor_value full_candidate_summary)" == "$(canonical_value summary)" \
        && "$(successor_value full_changed)" == 13 \
        && "$(successor_value full_outcome_changed)" == 13 \
        && "$(successor_value full_detail_only)" == 0 \
        && "$(successor_value full_unchanged)" == 102024 \
        && "$(successor_value full_pass_gains)" == 13 \
        && "$(successor_value full_pass_regressions)" == 0 \
        && "$(successor_value full_candidate_replay_status)" == passed-twice \
        && "$(successor_value full_candidate_replays)" == 2 ]] \
        || die 'R3dx successor does not checksum-bridge the historical R3dw receipt'
    [[ "$(sha "$candidate_profile")" == "$(successor_value parent_profile_sha256)" \
        && "$(sha "$live_profile")" == "$(successor_value candidate_profile_sha256)" ]] \
        || die 'R3dx profiles do not bridge the historical R3dw candidate to live state'

    case $mode in
        check) "$successor_gate" --check ;;
        focused) "$successor_gate" ;;
        full) "$successor_gate" --full ;;
    esac
    echo 'Historical R3dw destructuring-assignment receipt is checksum-bridged through the R3dx static-module milestone.'
    exit 0
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-destructuring-assignment-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
bridge_r3dx_successor
check_file "$baseline" "$baseline_lines" "$baseline_sha"
check_file "$upstream" "$(value upstream_lines)" "$(value upstream_sha256)"
[[ "$(value live_profile)" == "$live_profile" \
    && "$(value upstream)" == "$upstream" \
    && "$(value parent_profile)" == "$parent_profile" \
    && "$(value candidate_profile)" == "$candidate_profile" \
    && "$(value canonical_full_baseline)" == "$canonical_baseline" \
    && "$(toml_value quickjs version)" == "$(value quickjs)" \
    && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
    && "$(toml_value test262 repository)" == https://github.com/tc39/test262.git \
    && "$(toml_value test262 commit)" == "$(value test262)" \
    && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
    && "$(toml_value test262 oxide_profile)" == "$live_profile" \
    && "$(toml_value test262 oxide_profile_sha256)" == "$(value live_profile_sha256)" \
    && "$(value live_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
    || die 'pinned upstream identity drifted'

parent_commit=$(value parent_commit)
parent_profile_snapshot=$tmp/parent-commit-profile.conf
parent_canonical_snapshot=$tmp/parent-commit-canonical-baseline.txt
git cat-file -e "${parent_commit}^{commit}" 2>/dev/null \
    || die "R3dw parent commit is unavailable: $parent_commit"
git show "${parent_commit}:${live_profile}" >"$parent_profile_snapshot" 2>/dev/null \
    || die "R3dw parent commit has no $live_profile snapshot"
cmp -s "$parent_profile" "$parent_profile_snapshot" \
    || die 'R3dw parent profile does not match its recorded commit'
git show "${parent_commit}:${canonical_baseline}" \
    >"$parent_canonical_snapshot" 2>/dev/null \
    || die "R3dw parent commit has no $canonical_baseline snapshot"
check_file "$parent_canonical_snapshot" \
    "$(value parent_canonical_baseline_lines)" \
    "$(value parent_canonical_baseline_sha256)"
[[ "$(value_from "$parent_canonical_snapshot" schema)" == "$(value schema)" \
    && "$(value_from "$parent_canonical_snapshot" timeout_ms)" == "$(value timeout_ms)" \
    && "$(value_from "$parent_canonical_snapshot" variants)" == "$(value full_variants)" \
    && "$(value_from "$parent_canonical_snapshot" runnable)" \
        == "$(value full_parent_runnable)" \
    && "$(value_from "$parent_canonical_snapshot" passes)" \
        == "$(value full_parent_passes)" \
    && "$(value_from "$parent_canonical_snapshot" tsv_sha256)" \
        == "$(value full_parent_tsv_sha256)" \
    && "$(value_from "$parent_canonical_snapshot" jsonl_sha256)" \
        == "$(value full_parent_jsonl_sha256)" \
    && "$(value_from "$parent_canonical_snapshot" summary)" \
        == "$(value full_parent_summary)" ]] \
    || die 'R3dw parent canonical receipt does not match its recorded commit'
[[ "$(sha "$canonical_baseline")" == "$(value canonical_full_baseline_sha256)" \
    && "$(canonical_value schema)" == "$(value schema)" \
    && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
    && "$(canonical_value variants)" == "$(value full_variants)" \
    && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
    && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
    && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
    || die 'canonical full baseline is not the admitted R3dw candidate'
check_profiles
check_manifests_and_receipts

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ -n "$runner_override" ]]; then
    [[ -x "$runner_override" ]] || die "TEST262_RUNNER is not executable: $runner_override"
    runner=$runner_override
else
    target_dir=${CARGO_TARGET_DIR:-$root/target}
    case $target_dir in
        /*) ;;
        *) target_dir=$root/$target_dir ;;
    esac
    cargo build --locked --release --quiet --bin run-test262
    runner=$target_dir/release/run-test262
fi
derive_metadata_and_sources
derive_activation_and_retained
verify_quickjs
if [[ "$mode" == check ]]; then
    echo 'R3dw destructuring-assignment inputs verified: raw 141 paths / 217 variants; activation 137 / 213; QuickJS passed both twice.'
    exit 0
fi

run_report "$parent_profile" "$tmp/parent.tsv" focused "$workers"
run_report "$candidate_profile" "$tmp/candidate.tsv" focused "$workers"
verify_report "$tmp/parent.tsv" "$(value parent_profile_sha256)" parent
verify_report "$tmp/candidate.tsv" "$(value candidate_profile_sha256)" candidate
diff -u "$parent_report" "$tmp/parent.tsv"
diff -u "${parent_report%.tsv}.jsonl" "$tmp/parent.jsonl"
diff -u "$candidate_report" "$tmp/candidate.tsv"
diff -u "${candidate_report%.tsv}.jsonl" "$tmp/candidate.jsonl"
make_transition "$tmp/parent.tsv" "$tmp/candidate.tsv" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
verify_focused_semantics
if [[ "$mode" != full ]]; then
    echo 'R3dw destructuring-assignment focused semantics pass: 213 pass, 4 module variants retained unsupported.'
    exit 0
fi

mkdir -p "$full_dir"
if [[ "$reuse_full_reports" == false ]]; then
    run_report "$parent_profile" "$full_parent" full "$full_workers"
    run_report "$candidate_profile" "$full_candidate_a" full "$full_workers"
    run_report "$candidate_profile" "$full_candidate_b" full "$full_workers"
fi
verify_full_report "$full_parent" "$(value parent_profile_sha256)" full_parent
verify_full_report "$full_candidate_a" "$(value candidate_profile_sha256)" \
    full_candidate
verify_full_report "$full_candidate_b" "$(value candidate_profile_sha256)" \
    full_candidate
[[ ! -L "$full_candidate_a" && ! -L "$full_candidate_b" \
    && ! -L "${full_candidate_a%.tsv}.jsonl" \
    && ! -L "${full_candidate_b%.tsv}.jsonl" ]] \
    || die 'R3dw candidate full receipts must not be symbolic links'
if [[ "$full_candidate_a" -ef "$full_candidate_b" \
    || "${full_candidate_a%.tsv}.jsonl" -ef "${full_candidate_b%.tsv}.jsonl" ]]; then
    die 'R3dw candidate full receipts must be distinct files'
fi
cmp -s "$full_candidate_a" "$full_candidate_b" \
    && cmp -s "${full_candidate_a%.tsv}.jsonl" "${full_candidate_b%.tsv}.jsonl" \
    || die 'R3dw candidate full replays are not byte-identical'
[[ "$(value full_candidate_replay_status)" == passed-twice \
    && "$(value full_candidate_replays)" == 2 ]] \
    || die 'R3dw candidate replay certificate drifted'
verify_full_join "$full_parent" "$full_candidate_a"
printf 'R3dw destructuring-assignment full semantics pass: parent=%s candidate=%s candidate_json=%s\n' \
    "$(sha "$full_parent")" "$(sha "$full_candidate_a")" \
    "$(sha "${full_candidate_a%.tsv}.jsonl")"
