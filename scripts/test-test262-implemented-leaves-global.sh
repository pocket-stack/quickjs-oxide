#!/usr/bin/env bash
# Reproduce the R3dv global admission of eight already-implemented Test262 leaf tags.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-implemented-leaves-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-implemented-leaves-global-parent.conf
candidate_profile=tests/test262-implemented-leaves-global-candidate.conf
added_features=tests/test262-implemented-leaves-global-added-features.txt
universe=tests/test262-implemented-leaves-global-universe.txt
activation=tests/test262-implemented-leaves-global-activation.txt
retained=tests/test262-implemented-leaves-global-retained.txt
feature_skipped=tests/test262-implemented-leaves-global-feature-skipped.txt
config_excluded=tests/test262-implemented-leaves-global-config-excluded.txt
metadata_projection=tests/test262-implemented-leaves-global-metadata.tsv
feature_partitions=tests/test262-implemented-leaves-global-feature-partitions.tsv
host_hooks=tests/test262-implemented-leaves-global-host-hooks.tsv
ledger=tests/test262-implemented-leaves-global-ledger.tsv
quickjs_skipped=tests/test262-implemented-leaves-global-quickjs-skipped.txt
quickjs_runnable=tests/test262-implemented-leaves-global-quickjs-runnable.txt
quickjs_skips_ledger=tests/test262-implemented-leaves-global-quickjs-skips.tsv
quickjs_receipt=tests/test262-implemented-leaves-global-quickjs-receipt.txt
parent_report=tests/test262-implemented-leaves-global-parent.tsv
candidate_report=tests/test262-implemented-leaves-global-candidate.tsv
transition=tests/test262-implemented-leaves-global-transitions.tsv
for_of_oracle=tests/oracle_for_of.rs
caller_oracle=tests/oracle_function_debug_accessors.rs
json_oracle=tests/oracle_json_parse.rs
lexer_source=src/lexer.rs
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
full_dir=${TEST262_FULL_REPORT_DIR:-target/test262-implemented-leaves-global-full}
full_parent=${TEST262_FULL_PARENT_REPORT:-$full_dir/parent.tsv}
full_candidate_a=${TEST262_FULL_CANDIDATE_A_REPORT:-$full_dir/candidate-a.tsv}
full_candidate_b=${TEST262_FULL_CANDIDATE_B_REPORT:-$full_dir/candidate-b.tsv}
runner_override=${TEST262_RUNNER:-}

baseline_lines=182
baseline_sha=a1c52e7d282ccc4348d8d7b43649a9570198c3d7a8cb55855c8f9efc98416743

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
            printf "error: R3dv JSONL projection %s: %s\n", report, message \
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
    sort -c "$added_features" || die 'R3dv added features are not bytewise sorted'
    cmp -s "$candidate_profile" "$live_profile" \
        || die 'R3dv candidate is not byte-identical to the canonical profile'

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
        || die 'R3dv profile inventory drifted'
    comm -13 "$tmp/parent.features" "$tmp/candidate.features" >"$tmp/added.features"
    comm -23 "$tmp/parent.features" "$tmp/candidate.features" >"$tmp/removed.features"
    comm -13 "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" >"$tmp/added.negatives"
    comm -23 "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" >"$tmp/removed.negatives"
    [[ ! -s "$tmp/removed.features" && ! -s "$tmp/removed.negatives" \
        && "$(lines "$tmp/added.features")" == "$(value added_features)" \
        && "$(sha "$tmp/added.features")" == "$(value added_features_sha256)" ]] \
        || die 'R3dv profile removes capability or adds the wrong feature'
    diff -u "$added_features" "$tmp/added.features"
    [[ ! -s "$tmp/added.negatives" ]] \
        || die 'R3dv must not add negative provenance'
    cmp -s "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" \
        || die 'R3dv changes audited negative provenance'
    cmp -s "$tmp/parent.execution" "$tmp/candidate.execution" \
        || die 'R3dv changes async execution policy'
    cmp -s "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
        || die 'R3dv changes agent host policy'
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
    verify_manifest quickjs_skipped "$quickjs_skipped"
    verify_manifest quickjs_runnable "$quickjs_runnable"
    check_file "$metadata_projection" "$(value metadata_projection_lines)" \
        "$(value metadata_projection_sha256)"
    check_file "$feature_partitions" "$(value feature_partitions_lines)" \
        "$(value feature_partitions_sha256)"
    check_file "$host_hooks" "$(value host_hooks_lines)" \
        "$(value host_hooks_sha256)"
    check_file "$ledger" "$(value ledger_lines)" "$(value ledger_sha256)"
    check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
        "$(value quickjs_receipt_sha256)"
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
    check_file "$for_of_oracle" "$(value for_of_oracle_lines)" \
        "$(value for_of_oracle_sha256)"
    check_file "$caller_oracle" "$(value caller_oracle_lines)" \
        "$(value caller_oracle_sha256)"
    check_file "$json_oracle" "$(value json_oracle_lines)" \
        "$(value json_oracle_sha256)"
    check_file "$lexer_source" "$(value lexer_source_lines)" \
        "$(value lexer_source_sha256)"
    grep -Fq 'fn for_of_values_match_pinned_quickjs()' "$for_of_oracle" \
        || die 'R3dv for-of QuickJS differential evidence drifted'
    grep -Fq 'const ORACLE_PROBE: &str' "$caller_oracle" \
        && grep -Fq 'fn function_debug_accessors_and_strip_modes_match_quickjs_oracle()' \
            "$caller_oracle" \
        || die 'R3dv caller QuickJS source oracle evidence drifted'
    grep -Fq 'fn json_parse_semantics_match_pinned_quickjs()' "$json_oracle" \
        && grep -Fq "'\\u{2028}' | '\\u{2029}'" "$lexer_source" \
        || die 'R3dv JSON/lexer differential evidence drifted'

    sort "$activation" "$retained" "$feature_skipped" "$config_excluded" \
        >"$tmp/universe.partition"
    diff -u "$universe" "$tmp/universe.partition"
    [[ -z "$(uniq -d "$tmp/universe.partition")" ]] \
        || die 'R3dv universe partitions overlap'
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
        && "$(awk -F'\t' 'NR>1&&$5!=""{n++}END{print n+0}' "$ledger")" == 0 ]] \
        || die 'R3dv source/metadata ledger drifted'
    sed -n '2,$p' "$ledger" | cut -f1 | diff -u "$universe" -
    sed -n '2,$p' "$ledger" | cut -f1-6 | diff -u "$metadata_projection" -
    awk -F'\t' '
        FILENAME==ARGV[1]{supported[$0]=1;next}
        FILENAME==ARGV[2]{retained[$0]=1;next}
        FILENAME==ARGV[3]{target[$0]=1;next}
        FNR==1{next}
        {
            count=split($4,features,",");matched=0;unknown=0;leaf=""
            for(i=1;i<=count;i++){
                if(features[i] in target){matched++;leaf=features[i]}
                if(!(features[i] in supported)&&!(features[i] in target)){
                    if(features[i]!="destructuring-assignment")exit 2
                    unknown++
                }
            }
            if(matched!=1||length($8)!=64||$8~/[^0-9a-f]/||$5!=""||$6!="")exit 3
            if(leaf=="caller"){
                if($3!="noStrict"||$7!=1)exit 4
            } else if($3!=""||$7!=2)exit 5
            if($1 in retained){if(unknown!=1)exit 6}else if(unknown!=0)exit 7
        }
    ' "$tmp/parent.features" "$retained" "$added_features" "$ledger" \
        || die 'R3dv ledger carries invalid leaf, co-feature, flag, or negative metadata'
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
        || die 'R3dv frozen host-hook inventory drifted'
    awk -F'\t' '$2!=""{exit 2}' "$host_hooks" \
        || die 'R3dv universe requires a host hook'

    for key in quickjs universe_paths universe_variants feature_skipped_paths \
        feature_skipped_variants config_excluded_paths config_excluded_variants \
        runnable_paths executed_variants passes failures replays replay_status \
        log_lines log_projection_lines log_projection_sha256 \
        linux_arm64_log_sha256 macos_arm64_log_sha256; do
        [[ "$(value_from "$quickjs_receipt" "$key")" \
                == "$(value "quickjs_receipt_$key")" ]] \
            || die "R3dv QuickJS receipt field drifted: $key"
    done

    verify_report "$parent_report" "$(value parent_profile_sha256)" parent
    verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
    [[ "$(awk -F'\t' '!/^#/&&$1!="path"{print}' "$transition" | sha_stream)" \
            == "$(value transition_data_sha256)" ]] \
        || die 'R3dv transition data drifted'
}

derive_metadata_and_sources() {
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
            die "R3dv source requires a host or async hook: $test_path"
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
        || die 'R3dv QuickJS feature and config skip partitions overlap'
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
    awk -F'\t' '!/^#/&&$1!="path"&&$7=="unsupported-feature"{print $1}' \
        "$candidate_report" | sort -u >"$tmp/retained"
    diff -u "$retained" "$tmp/retained"
}

verify_quickjs() {
    local log_a=$root/target/test262-implemented-leaves-global-quickjs-a.log
    local log_b=$root/target/test262-implemented-leaves-global-quickjs-b.log test_path
    local platform_log_sha=
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$quickjs_runnable"
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    rm -f -- "$log_a" "$log_b"
    for log in "$log_a" "$log_b"; do
        if ! (cd -- "$source_dir" \
            && ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
            >"$log" 2>&1; then
            tail -n 100 "$log" >&2
            die 'pinned QuickJS could not execute the implemented leaf universe'
        fi
        if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$log" \
            || ! grep -Fq "Average memory statistics for $(value quickjs_executed_variants) tests:" "$log" \
            || [[ "$(lines "$log")" != "$(value quickjs_log_lines)" ]]; then
            tail -n 100 "$log" >&2
            die 'pinned QuickJS implemented leaf result drifted'
        fi
        awk '/^test262\.conf:.* ignoring testdir=/{print}
            /^Average memory statistics for [0-9]+ tests:/{print}' "$log" \
            >"$tmp/$(basename "$log").projection"
        [[ "$(lines "$tmp/$(basename "$log").projection")" \
                == "$(value quickjs_log_projection_lines)" \
            && "$(sha "$tmp/$(basename "$log").projection")" \
                == "$(value quickjs_log_projection_sha256)" ]] \
            || die 'pinned QuickJS semantic log projection drifted'
    done
    [[ ! -L "$log_a" && ! -L "$log_b" && ! "$log_a" -ef "$log_b" ]] \
        || die 'R3dv QuickJS replay logs must be distinct regular files'
    cmp -s "$log_a" "$log_b" || die 'R3dv QuickJS replays are not byte-identical'
    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) platform_log_sha=$(value quickjs_macos_arm64_log_sha256) ;;
        Linux-aarch64) platform_log_sha=$(value quickjs_linux_arm64_log_sha256) ;;
    esac
    [[ -z "$platform_log_sha" || "$(sha "$log_a")" == "$platform_log_sha" ]] \
        || die 'R3dv QuickJS platform receipt drifted'
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
        printf '%s\n' '# Exhaustive R3dv global implemented leaf feature admission transition.'
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
        && "$(report_count skipped-feature "$parent_report")" \
            == "$(value parent_skipped_feature)" \
        && "$(report_count skipped-config-exclude "$parent_report")" \
            == "$(value parent_skipped_config_exclude)" \
        && "$(report_count unsupported-feature "$parent_report")" \
            == "$(value parent_unsupported_feature)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
        && "$(report_count skipped-feature "$candidate_report")" \
            == "$(value candidate_skipped_feature)" \
        && "$(report_count skipped-config-exclude "$candidate_report")" \
            == "$(value candidate_skipped_config_exclude)" \
        && "$(report_count unsupported-feature "$candidate_report")" \
            == "$(value candidate_unsupported_feature)" ]] \
        || die 'R3dv focused outcome counts drifted'
    awk -F'\t' '
        FILENAME==ARGV[1]{target[$0]=1;next}
        FILENAME==ARGV[2]{retained[$0]=1;next}
        FILENAME==ARGV[3]{
            if(FNR==1)next
            count=split($4,features,",");matched=0
            for(i=1;i<=count;i++)if(features[i] in target){
                leaf[$1]=features[i];matched++
            }
            if(matched!=1)exit 2
            next
        }
        !/^#/&&$1!="path"{
            if(!($7=="unsupported-feature"&&$8=="selection"&&$9=="EngineCapability"))exit 3
            prefix="quickjs-oxide does not declare Test262 feature support: "
            if(index($10,prefix)!=1)exit 4
            missing=substr($10,length(prefix)+1)
            count=split(missing,features,", ")
            expected=($1 in retained)?2:1
            if(count!=expected)exit 5
            matched=0;extra=0
            for(i=1;i<=count;i++){
                if(features[i]==leaf[$1])matched++
                else if(($1 in retained)&&features[i]=="destructuring-assignment")extra++
                else exit 6
            }
            if(matched!=1||extra!=(($1 in retained)?1:0))exit 7
        }' "$added_features" "$retained" "$ledger" "$parent_report" \
        || die 'R3dv parent leaf frontier drifted'
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted)&&
        !($7=="pass"&&$8==$5&&$9==$6){exit 2}' \
        "$activation" "$candidate_report" \
        || die 'R3dv candidate activation semantics drifted'
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted){
            if(!($7=="unsupported-feature"&&$8=="selection"&&$9=="EngineCapability") \
                ||$10!="quickjs-oxide does not declare Test262 feature support: destructuring-assignment")exit 2
            variants++
        }
        END{if(variants!=4)exit 3}' \
        "$retained" "$candidate_report" \
        || die 'R3dv retained feature frontier drifted'
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted)&&$7!="skipped-feature"{exit 2}' \
        "$feature_skipped" "$candidate_report" \
        || die 'R3dv QuickJS feature-skip semantics drifted'
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&$1!="path"&&($1 in wanted)&&$7!="skipped-config-exclude"{exit 2}' \
        "$config_excluded" "$candidate_report" \
        || die 'R3dv QuickJS config-exclude semantics drifted'
    local expected="changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)"
    [[ "$(transition_counts "$transition")" == "$expected" ]] \
        || die 'R3dv focused transition counts drifted'
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
        || die 'R3dv full scope partition drifted'
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
    ' "$parent" "$candidate") || die 'R3dv full exact join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) gains=$(value full_pass_gains) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3dv full transition drifted: $counts"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-implemented-leaves-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_file "$baseline" "$baseline_lines" "$baseline_sha"
check_file "$upstream" "$(value upstream_lines)" "$(value upstream_sha256)"
[[ "$(toml_value quickjs version)" == "$(value quickjs)" \
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
    || die "R3dv parent commit is unavailable: $parent_commit"
git show "${parent_commit}:${live_profile}" >"$parent_profile_snapshot" 2>/dev/null \
    || die "R3dv parent commit has no $live_profile snapshot"
cmp -s "$parent_profile" "$parent_profile_snapshot" \
    || die 'R3dv parent profile does not match its recorded commit'
git show "${parent_commit}:${canonical_baseline}" \
    >"$parent_canonical_snapshot" 2>/dev/null \
    || die "R3dv parent commit has no $canonical_baseline snapshot"
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
    || die 'R3dv parent canonical receipt does not match its recorded commit'
[[ "$(sha "$canonical_baseline")" == "$(value canonical_full_baseline_sha256)" \
    && "$(canonical_value schema)" == "$(value schema)" \
    && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
    && "$(canonical_value variants)" == "$(value full_variants)" \
    && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
    && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
    && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
    || die 'canonical full baseline is not the admitted R3dv candidate'
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
    echo 'R3dv implemented leaf inputs verified: 46 paths / 69 variants; QuickJS 69/69 executed with no skips.'
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
    echo 'R3dv implemented leaf focused semantics pass: 65 pass, 4 destructuring-assignment variants retained unsupported.'
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
    || die 'R3dv candidate full receipts must not be symbolic links'
if [[ "$full_candidate_a" -ef "$full_candidate_b" \
    || "${full_candidate_a%.tsv}.jsonl" -ef "${full_candidate_b%.tsv}.jsonl" ]]; then
    die 'R3dv candidate full receipts must be distinct files'
fi
cmp -s "$full_candidate_a" "$full_candidate_b" \
    && cmp -s "${full_candidate_a%.tsv}.jsonl" "${full_candidate_b%.tsv}.jsonl" \
    || die 'R3dv candidate full replays are not byte-identical'
[[ "$(value full_candidate_replay_status)" == passed-twice \
    && "$(value full_candidate_replays)" == 2 ]] \
    || die 'R3dv candidate replay certificate drifted'
verify_full_join "$full_parent" "$full_candidate_a"
printf 'R3dv implemented leaf full semantics pass: parent=%s candidate=%s candidate_json=%s\n' \
    "$(sha "$full_parent")" "$(sha "$full_candidate_a")" \
    "$(sha "${full_candidate_a%.tsv}.jsonl")"
