#!/usr/bin/env bash
# Reproduce the checksum-bound scoped $262.createRealm host-hook admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-create-realm-baseline.txt
live_profile=compat/test262-oxide.conf
profile=tests/test262-create-realm.conf
upstream=compat/upstream.toml
universe=tests/test262-create-realm-universe.txt
config_excluded=tests/test262-create-realm-config-excluded.txt
config_skipped=tests/test262-create-realm-config-skipped-feature.txt
reason_only=tests/test262-create-realm-reason-only.txt
oracle_envelope=tests/test262-create-realm-oracle-envelope.txt
supplemental_feature=tests/test262-create-realm-supplemental-feature.txt
activation=tests/test262-create-realm-activation.txt
core_sync=tests/test262-create-realm-core-sync.txt
existing_host=tests/test262-create-realm-existing-host-composition.txt
async_paths=tests/test262-create-realm-async.txt
quickjs_receipt=tests/test262-create-realm-quickjs-receipt.txt
parent_report=tests/test262-create-realm-parent.tsv
parent_json=tests/test262-create-realm-parent.jsonl
transition=tests/test262-create-realm-transitions.tsv
report=target/test262-create-realm.tsv
envelope_report=target/test262-create-realm-envelope.tsv
universe_report=target/test262-create-realm-universe.tsv
oracle_log=target/test262-create-realm-quickjs.log
workers=${TEST262_WORKERS:-8}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
quickjs_source_sha=b376e839b322978313d929fd20663b11ba58b75df5a46c126dd19ea2fa70ad2a
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
live_profile_sha=01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75
profile_sha=7d27ac5117879670609206a0fa7d459a2c050d230ba489979dcae0aa9911fd30
baseline_sha=902d4e27efa0e590b36c12eb093387a13294f690ea9ae14ca4a56220d98f53b9

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify authenticated inputs, parent receipt, and QuickJS 152/152\n'
}

check_only=false
case ${1-} in
    '') ;;
    --check) check_only=true ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid TEST262_WORKERS' >&2; exit 2; }

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
section() {
    awk -v wanted="[$2]" \
        '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' \
        "$1"
}
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
toml_quickjs_value() {
    awk -v wanted="$2" '
        $0=="[quickjs]"{inside=1;next} /^\[/{inside=0}
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
    local file=$1 expected_lines=$2 expected_sha=$3
    [[ -f "$file" && "$(lines "$file")" == "$expected_lines" \
        && "$(sha "$file")" == "$expected_sha" ]] \
        || die "authenticated input drifted: $file"
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
    awk -F'\t' '
        function has(list,value){return index("," list ",", "," value ",")!=0}
        NR==FNR{wanted[$0]=1;next}
        $1 in wanted{
            if(has($3,"module")||has($3,"noStrict")||has($3,"raw"))print $1 "\tsloppy"
            else if(has($3,"onlyStrict"))print $1 "\tstrict"
            else{print $1 "\tsloppy";print $1 "\tstrict"}
        }
    ' "$1" "$metadata_tsv" | sort
}
check_keys() {
    local manifest=$1 expected_count=$2 expected_sha=$3 output=$4
    variant_keys "$manifest" >"$output"
    [[ "$(lines "$output")" == "$expected_count" \
        && "$(sha "$output")" == "$expected_sha" ]] \
        || die "variant-key inventory drifted: $manifest"
}

json_result_projection() {
    awk -v report="$1" '
        function fail(message){
            printf "error: createRealm JSONL projection %s: %s\n",report,message >"/dev/stderr"
            failed=1;exit 2
        }
        function expect(token){
            if(substr(line,position,length(token))!=token)fail("expected " token)
            position+=length(token)
        }
        function string_value(    character,escape,digits,result){
            expect("\"");result=""
            while(position<=length(line)){
                character=substr(line,position,1)
                if(character=="\""){position++;return result}
                if(character=="\\"){
                    position++;escape=substr(line,position,1)
                    if(escape=="u"){
                        digits=substr(line,position+1,4)
                        if(length(digits)!=4||digits~/[^0123456789abcdefABCDEF]/)fail("invalid Unicode escape")
                        result=result "\\u" digits;position+=5
                    }else{
                        if(index("\"\\/bfnrt",escape)==0)fail("invalid escape")
                        if(escape=="\"")result=result "\""
                        else if(escape=="/")result=result "/"
                        else if(escape=="b")result=result "\\u0008"
                        else if(escape=="f")result=result "\\u000c"
                        else result=result "\\" escape
                        position++
                    }
                    continue
                }
                if(character=="\t"||character=="\r")fail("unescaped control")
                result=result character;position++
            }
            fail("unterminated string")
        }
        function project(    i,key,result){
            line=$0;position=1;expect("{")
            for(i=1;i<=11;i++){
                if(i!=1)expect(",")
                key=string_value();if(key!=name[i])fail("unexpected field " key)
                expect(":");result=string_value()
                if(i==1){if(result!="result")fail("unexpected record kind")}
                else field[i-1]=result
            }
            expect("}");if(position!=length(line)+1)fail("trailing data")
            print field[1],field[2],field[3],field[4],field[5],field[6],field[7],field[8],field[9],field[10]
        }
        BEGIN{
            OFS="\t";name[1]="kind";name[2]="path";name[3]="variant"
            name[4]="flags";name[5]="features";name[6]="expected_phase"
            name[7]="expected_type";name[8]="outcome";name[9]="actual_phase"
            name[10]="actual_type";name[11]="detail"
        }
        /^\{"kind":"metadata",/{next}
        /^\{"kind":"result",/{project();next}
        /^\{"kind":"summary",/{next}
        {fail("unexpected record")}
    ' "$1"
}
json_summary() {
    tail -n 1 "$1" | awk '
        /^\{"kind":"summary","outcomes":\{.*\}\}$/ {
            sub(/^\{"kind":"summary","outcomes":\{/ ,"");sub(/\}\}$/ ,"")
            gsub(/":/ ,"=");gsub(/"/ ,"");gsub(/,/ ," ");print;found++
        }
        END{if(found!=1)exit 1}'
}
expected_json_metadata() {
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"test262-canonical-classified-v2","mode":"both"}\n' \
        "$quickjs" "$test262" "$patch_sha" "$config_sha" "$metadata_sha" "$profile_sha"
}
verify_report() {
    local tsv=$1 json=$2 expected_variants=$3 expected_tsv_sha=$4
    local expected_json_sha=$5 expected_keys_sha=$6 projection=$tmp/projection.$$.tsv
    [[ -f "$tsv" && -f "$json" \
        && "$(header "$tsv" quickjs)" == "$quickjs" \
        && "$(header "$tsv" test262)" == "$test262" \
        && "$(header "$tsv" test262_patch_sha256)" == "$patch_sha" \
        && "$(header "$tsv" test262_config_sha256)" == "$config_sha" \
        && "$(header "$tsv" test262_metadata_sha256)" == "$metadata_sha" \
        && "$(header "$tsv" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$tsv" profile)" == test262-canonical-classified-v2 \
        && "$(header "$tsv" mode)" == both \
        && "$(lines "$tsv")" == "$((expected_variants + 11))" \
        && "$(lines "$json")" == "$((expected_variants + 2))" \
        && "$(sha "$tsv")" == "$expected_tsv_sha" \
        && "$(sha "$json")" == "$expected_json_sha" \
        && "$(report_summary "$tsv")" == "$(computed_summary "$tsv")" \
        && "$(head -n 1 "$json")" == "$(expected_json_metadata)" \
        && "$(json_summary "$json")" == "$(report_summary "$tsv")" ]] \
        || die "report identity drifted: $tsv"
    json_result_projection "$json" >"$projection" \
        || die "JSONL projection failed: $json"
    diff -u <(report_rows "$tsv") "$projection" \
        || die "JSONL/TSV projection drifted: $json"
    [[ "$(report_keys "$tsv" | sha /dev/stdin)" == "$expected_keys_sha" ]] \
        || die "report key inventory drifted: $tsv"
}

check_static_inputs() {
    check_file "$baseline" "$(value baseline_lines)" "$baseline_sha"
    check_file "$live_profile" 1274 "$live_profile_sha"
    check_file "$profile" 16 "$profile_sha"
    check_file "$universe" "$(value universe_paths)" "$(value universe_sha256)"
    check_file "$config_excluded" "$(value config_excluded_paths)" \
        "$(value config_excluded_sha256)"
    check_file "$config_skipped" "$(value config_skipped_feature_paths)" \
        "$(value config_skipped_feature_sha256)"
    check_file "$reason_only" "$(value reason_only_paths)" "$(value reason_only_sha256)"
    check_file "$oracle_envelope" "$(value oracle_envelope_paths)" \
        "$(value oracle_envelope_sha256)"
    check_file "$supplemental_feature" "$(value supplemental_feature_paths)" \
        "$(value supplemental_feature_sha256)"
    check_file "$activation" "$(value activation_paths)" "$(value activation_sha256)"
    check_file "$core_sync" "$(value core_sync_paths)" "$(value core_sync_sha256)"
    check_file "$existing_host" "$(value existing_host_composition_paths)" \
        "$(value existing_host_composition_sha256)"
    check_file "$async_paths" "$(value async_paths)" "$(value async_sha256)"
    check_file "$quickjs_receipt" 10 "$(value quickjs_receipt_sha256)"
    check_file "$parent_report" 163 "$(value parent_tsv_sha256)"
    check_file "$parent_json" 154 "$(value parent_jsonl_sha256)"
    for manifest in "$universe" "$config_excluded" "$config_skipped" \
            "$reason_only" "$oracle_envelope" "$supplemental_feature" \
            "$activation" "$core_sync" "$existing_host" "$async_paths"; do
        sort -c "$manifest" || die "manifest is not bytewise sorted: $manifest"
    done
    [[ "$(value quickjs)" == "$quickjs" \
        && "$(value quickjs_source_sha256)" == "$quickjs_source_sha" \
        && "$(value test262)" == "$test262" \
        && "$(value test262_patch_sha256)" == "$patch_sha" \
        && "$(value test262_config_sha256)" == "$config_sha" \
        && "$(value test262_metadata_sha256)" == "$metadata_sha" \
        && "$(value live_oxide_profile)" == "$live_profile" \
        && "$(value live_oxide_profile_sha256)" == "$live_profile_sha" \
        && "$(value scoped_oxide_profile)" == "$profile" \
        && "$(value scoped_oxide_profile_sha256)" == "$profile_sha" \
        && "$(section "$profile" features | lines /dev/stdin)" \
            == "$(value scoped_profile_features)" \
        && "$(section "$profile" features | sha /dev/stdin)" \
            == "$(value scoped_profile_features_sha256)" \
        && "$(value source_audited_admission_tag)" == host-create-realm-required \
        && "$(section "$profile" features | grep -Fxc host-create-realm-required)" == 1 \
        && -z "$(section "$profile" audited-negative-tests)" \
        && "$(section "$profile" execution)" == async=true ]] \
        || die 'baseline/profile identity drifted'
    [[ "$(toml_test262_value "$upstream" repository)" \
            == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$test262" \
        && "$(toml_test262_value "$upstream" patch_sha256)" == "$patch_sha" \
        && "$(toml_test262_value "$upstream" config_sha256)" == "$config_sha" \
        && "$(toml_test262_value "$upstream" test_count)" == 53125 \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" == "$metadata_sha" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" == "$live_profile_sha" \
        && "$(toml_quickjs_value "$upstream" version)" == "$quickjs" \
        && "$(toml_quickjs_value "$upstream" source_sha256)" == "$quickjs_source_sha" ]] \
        || die 'compat/upstream.toml Test262 identity drifted'
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-create-realm.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_static_inputs

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
expected_status=$' M harness/atomicsHelper.js\n M harness/regExpUtils.js'
actual_status=$(git -C "$suite" status --porcelain=v1 --untracked-files=all | sort)
[[ "$(basename -- "$source_dir")" == "quickjs-$quickjs" \
    && "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" == "$test262" \
    && "$(sha "$source_dir/tests/test262.patch")" == "$patch_sha" \
    && "$(sha "$source_dir/test262.conf")" == "$config_sha" \
    && "$actual_status" == "$expected_status" ]] \
    || die 'prepared QuickJS/Test262 identity drifted'
git -C "$suite" apply --reverse --check "$source_dir/tests/test262.patch" \
    || die 'prepared Test262 patch is not reverse-applicable'
git -C "$suite" diff --no-ext-diff --no-color --no-renames \
    --abbrev=7 --src-prefix=a/ --dst-prefix=b/ -- \
    harness/atomicsHelper.js harness/regExpUtils.js \
    | cmp -s - "$source_dir/tests/test262.patch" \
    || die 'prepared Test262 harness diff drifted'

metadata_bin=$tmp/metadata.bin
metadata_tsv=$tmp/metadata.tsv
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(sha "$metadata_bin")" == "$metadata_sha" ]] \
    || die 'pinned Test262 metadata drifted'
tr '\0' '\t' <"$metadata_bin" >"$metadata_tsv"
[[ "$(lines "$metadata_tsv")" == 53125 ]] \
    || die 'pinned metadata record count drifted'

for spec in \
    "$universe:universe_variants:universe_keys_sha256" \
    "$config_excluded:config_excluded_variants:config_excluded_keys_sha256" \
    "$config_skipped:config_skipped_feature_variants:config_skipped_feature_keys_sha256" \
    "$reason_only:reason_only_variants:reason_only_keys_sha256" \
    "$oracle_envelope:oracle_envelope_variants:oracle_envelope_keys_sha256" \
    "$supplemental_feature:supplemental_feature_variants:supplemental_feature_keys_sha256" \
    "$activation:activation_variants:activation_keys_sha256" \
    "$core_sync:core_sync_variants:core_sync_keys_sha256" \
    "$existing_host:existing_host_composition_variants:existing_host_composition_keys_sha256" \
    "$async_paths:async_variants:async_keys_sha256"; do
    IFS=: read -r manifest count_key sha_key <<<"$spec"
    check_keys "$manifest" "$(value "$count_key")" "$(value "$sha_key")" \
        "$tmp/$(basename "$manifest").keys"
done

awk -F'\t' '
    NR==FNR{wanted[$0]=1;next}
    $1 in wanted{
        count=split($4,features,",")
        for(i=1;i<=count;i++)if(features[i]!="")seen[features[i]]=1
    }
    END{for(feature in seen)print feature}
' "$activation" "$metadata_tsv" >"$tmp/activation-features.txt"
echo host-create-realm-required >>"$tmp/activation-features.txt"
sort -u -o "$tmp/activation-features.txt" "$tmp/activation-features.txt"
diff -u <(section "$profile" features) "$tmp/activation-features.txt" \
    || die 'scoped createRealm profile is not the exact declared union plus source-audited admission tag'

awk -F'\t' 'NR==FNR{wanted[$0]=1;next}$1 in wanted{print}' \
    "$universe" "$metadata_tsv" | sort >"$tmp/metadata-projection.tsv"
[[ "$(lines "$tmp/metadata-projection.tsv")" == 281 \
    && "$(sha "$tmp/metadata-projection.tsv")" == "$(value metadata_projection_sha256)" ]] \
    || die 'createRealm metadata projection drifted'
awk -F'\t' '$5!=""||$6!=""||index(","$3",",",module,")||index(","$3",",",raw,"){exit 1}' \
    "$tmp/metadata-projection.tsv" \
    || die 'createRealm universe gained module/raw/negative metadata'

git -C "$suite" grep -l -F '$262.createRealm' -- 'test/**/*.js' | sort \
    >"$tmp/source.paths"
[[ "$(lines "$tmp/source.paths")" == "$(value universe_paths)" \
    && "$(sha "$tmp/source.paths")" == "$(value universe_sha256)" ]] \
    || die 'direct-source createRealm universe drifted'
diff -u "$universe" "$tmp/source.paths"

: >"$tmp/source-hooks.tsv"
: >"$tmp/call-sites.tsv"
while IFS= read -r test_path; do
    grep -Eo '\$262\.[A-Za-z_$][A-Za-z0-9_$]*' "$suite/$test_path" | sort -u \
        | sed "s#^#$test_path\t#" >>"$tmp/source-hooks.tsv"
    grep -nF '$262.createRealm' "$suite/$test_path" \
        | sed "s#^#$test_path:#" >>"$tmp/call-sites.tsv"
done <"$universe"
sort -o "$tmp/source-hooks.tsv" "$tmp/source-hooks.tsv"
sort -o "$tmp/call-sites.tsv" "$tmp/call-sites.tsv"
[[ "$(lines "$tmp/source-hooks.tsv")" == "$(value source_hook_projection_lines)" \
    && "$(sha "$tmp/source-hooks.tsv")" == "$(value source_hook_projection_sha256)" \
    && "$(lines "$tmp/call-sites.tsv")" == "$(value direct_call_sites)" \
    && "$(sha "$tmp/call-sites.tsv")" == "$(value direct_call_projection_sha256)" ]] \
    || die 'createRealm source projection drifted'
awk -F'\t' '$2!="$262.createRealm"&&$2!="$262.detachArrayBuffer"&&$2!="$262.gc"{exit 1}' \
    "$tmp/source-hooks.tsv" || die 'createRealm universe gained an unknown direct host hook'

git -C "$suite" grep -l -F '$262.evalScript' -- 'test/**/*.js' | sort \
    >"$tmp/eval-script.paths"
check_keys "$tmp/eval-script.paths" "$(value direct_eval_script_variants)" \
    "$(value direct_eval_script_keys_sha256)" "$tmp/eval-script.keys"
[[ "$(lines "$tmp/eval-script.paths")" == "$(value direct_eval_script_paths)" \
    && "$(sha "$tmp/eval-script.paths")" == "$(value direct_eval_script_sha256)" \
    && -z "$(comm -12 "$universe" "$tmp/eval-script.paths")" ]] \
    || die 'createRealm/evalScript direct-source boundary drifted'
if git -C "$suite" grep -l -F '$262.global' -- 'test/**/*.js' \
        >"$tmp/global.paths"; then
    sort -o "$tmp/global.paths" "$tmp/global.paths"
else
    [[ $? == 1 ]] || die 'failed to scan direct $262.global paths'
fi
[[ "$(lines "$tmp/global.paths")" == "$(value direct_global_paths)" \
    && "$(sha "$tmp/global.paths")" == "$(value direct_global_sha256)" ]] \
    || die 'direct $262.global source boundary drifted'

cat "$config_excluded" "$config_skipped" "$reason_only" "$activation" | sort \
    >"$tmp/universe-partition.paths"
diff -u "$universe" "$tmp/universe-partition.paths" \
    || die 'createRealm universe partition drifted'
[[ -z "$(cat "$config_excluded" "$config_skipped" "$reason_only" "$activation" \
    | sort | uniq -d)" ]] || die 'createRealm universe partitions overlap'
cat "$core_sync" "$existing_host" "$async_paths" | sort \
    >"$tmp/activation-partition.paths"
diff -u "$activation" "$tmp/activation-partition.paths" \
    || die 'createRealm activation partition drifted'
cat "$activation" "$supplemental_feature" | sort >"$tmp/oracle-envelope.paths"
diff -u "$oracle_envelope" "$tmp/oracle-envelope.paths" \
    || die 'createRealm oracle envelope partition drifted'
[[ "$(comm -12 "$activation" "$supplemental_feature" | lines /dev/stdin)" == 0 \
    && "$(comm -12 "$reason_only" "$supplemental_feature" | lines /dev/stdin)" == 1 ]] \
    || die 'supplemental feature path classification drifted'

awk -F'\t' '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    NR==FNR{wanted[$0]=1;next}
    $1 in wanted&&has($3,"async"){print $1}
' "$activation" "$metadata_tsv" | sort >"$tmp/derived-async.paths"
diff -u "$async_paths" "$tmp/derived-async.paths"
awk -F'\t' 'NR==FNR{wanted[$0]=1;next}$1 in wanted&&$2!="$262.createRealm"{print $1}' \
    "$activation" "$tmp/source-hooks.tsv" >"$tmp/derived-aux.paths"
awk -F'\t' '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    NR==FNR{wanted[$0]=1;next}
    $1 in wanted&&(has($2,"detachArrayBuffer.js")||has($4,"host-gc-required")){print $1}
' "$activation" "$metadata_tsv" >>"$tmp/derived-aux.paths"
sort -u -o "$tmp/derived-aux.paths" "$tmp/derived-aux.paths"
diff -u "$existing_host" "$tmp/derived-aux.paths"
cat "$existing_host" "$async_paths" | sort -u >"$tmp/non-core.paths"
comm -23 "$activation" "$tmp/non-core.paths" >"$tmp/derived-core.paths"
diff -u "$core_sync" "$tmp/derived-core.paths"

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$oracle_envelope"
if ! (cd "$source_dir" && ./run-test262 -m -c test262.conf -a \
        -T "$workers" -f "${files[@]}") >"$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS failed the createRealm oracle envelope'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || [[ "$(grep -Fc 'Average memory statistics for 152 tests:' "$oracle_log")" != 1 \
        || "$(grep -Fc 'Test262:AsyncTestComplete' "$oracle_log")" != 2 ]]; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS createRealm oracle receipt drifted'
fi
{
    echo '# Pinned QuickJS oracle receipt for the scoped $262.createRealm envelope.'
    echo "quickjs=$quickjs"
    echo "test262=$test262"
    echo "oracle_envelope_sha256=$(value oracle_envelope_sha256)"
    echo "paths=$(value oracle_envelope_paths)"
    echo "variants=$(value oracle_envelope_variants)"
    echo 'async_completions=2'
    echo 'failed=0'
    echo 'skipped_feature=0'
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
diff -u "$quickjs_receipt" "$tmp/quickjs-receipt.txt"

verify_report "$parent_report" "$parent_json" 152 \
    "$(value parent_tsv_sha256)" "$(value parent_jsonl_sha256)" \
    "$(value oracle_envelope_keys_sha256)"
[[ "$(report_summary "$parent_report")" == 'unsupported-host-create-realm=152' \
    && "$(report_runnable "$parent_report")" == 0 \
    && "$(report_count unsupported-host-create-realm "$parent_report")" == 152 ]] \
    || die 'historical createRealm parent receipt semantics drifted'
report_rows "$parent_report" | awk -F'\t' \
    '$7!="unsupported-host-create-realm"||$8!="selection"||$9!="HostCapability"||
     $10!="missing execution capabilities: create-realm"{exit 1}' \
    || die 'historical createRealm parent row drifted'

if "$check_only"; then
    check_static_inputs
    echo 'Test262 createRealm inputs verified: direct 281/545 partitioned; pinned QuickJS passes the 152-variant envelope; formal activation is 150 variants with 2 supplemental feature rows.'
    exit 0
fi

run_report() {
    local manifest=$1 output=$2 run_workers=$3
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" \
        --report "$output" --mode both --timeout-ms 30000 \
        --workers "$run_workers" --allow-failures >/dev/null
}

run_report "$universe" "$universe_report" "$workers"
verify_report "$universe_report" "${universe_report%.tsv}.jsonl" 545 \
    "$(value universe_candidate_tsv_sha256)" \
    "$(value universe_candidate_jsonl_sha256)" \
    "$(value universe_keys_sha256)"
[[ "$(report_summary "$universe_report")" \
        == 'pass=150 skipped-config-exclude=22 skipped-feature=33 unsupported-feature=340' \
    && "$(report_runnable "$universe_report")" == 150 ]] \
    || die 'createRealm universe candidate summary drifted'

{
    awk '{print $0 "\tconfig-excluded"}' "$config_excluded"
    awk '{print $0 "\tconfig-skipped"}' "$config_skipped"
    awk '{print $0 "\treason-only"}' "$reason_only"
    awk '{print $0 "\tactivation"}' "$activation"
} | sort >"$tmp/classes.tsv"
partition_counts=$(awk -F'\t' '
    NR==FNR{class[$1]=$2;next}
    /^#/||($1=="path"&&$2=="variant"){next}
    {
        kind=class[$1]
        if(kind=="config-excluded"){
            if($7!="skipped-config-exclude"||$8!="selection"||$9!=""||
                $10!="QuickJS config excludes this test")exit 2
            excluded++
        }else if(kind=="config-skipped"){
            if($7!="skipped-feature"||$8!="selection"||$9!=""||
                index($10,"QuickJS config skips feature ")!=1)exit 3
            skipped++
        }else if(kind=="reason-only"){
            if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
                index($10,"quickjs-oxide does not declare Test262 feature support: ")!=1)exit 4
            reason++
        }else if(kind=="activation"){
            if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 5
            active++
        }else exit 6
    }
    END{printf "excluded=%d skipped=%d reason=%d activation=%d",excluded,skipped,reason,active}
' "$tmp/classes.tsv" "$universe_report") \
    || die 'createRealm universe candidate partition semantics drifted'
[[ "$partition_counts" == 'excluded=22 skipped=33 reason=340 activation=150' ]] \
    || die "createRealm universe candidate partition counts drifted: $partition_counts"

run_report "$oracle_envelope" "$envelope_report" "$workers"

verify_report "$envelope_report" "${envelope_report%.tsv}.jsonl" 152 \
    "$(value envelope_candidate_tsv_sha256)" \
    "$(value envelope_candidate_jsonl_sha256)" \
    "$(value oracle_envelope_keys_sha256)"
[[ "$(report_summary "$envelope_report")" == 'pass=150 unsupported-feature=2' \
    && "$(report_runnable "$envelope_report")" == 150 \
    && "$(report_count pass "$envelope_report")" == 150 \
    && "$(report_count unsupported-feature "$envelope_report")" == 2 ]] \
    || die 'createRealm oracle-envelope candidate semantics drifted'

run_report "$activation" "$report" "$workers"
repeat_report=$tmp/repeat.tsv
run_report "$activation" "$repeat_report" 1
cmp -s "$report" "$repeat_report" \
    && cmp -s "${report%.tsv}.jsonl" "${repeat_report%.tsv}.jsonl" \
    || die 'createRealm candidate receipts are not deterministic across worker counts'
verify_report "$report" "${report%.tsv}.jsonl" 150 \
    "$(value candidate_tsv_sha256)" "$(value candidate_jsonl_sha256)" \
    "$(value activation_keys_sha256)"
[[ "$(report_summary "$report")" == 'pass=150' \
    && "$(report_runnable "$report")" == 150 \
    && "$(report_count pass "$report")" == 150 ]] \
    || die 'createRealm activation candidate semantics drifted'
awk -F'\t' 'NR==FNR{wanted[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&$1 in wanted{print}' \
    "$oracle_envelope" "$universe_report" >"$tmp/universe-envelope.tsv"
diff -u <(report_rows "$envelope_report") "$tmp/universe-envelope.tsv" \
    || die 'oracle-envelope receipt disagrees with universe receipt'
awk -F'\t' 'NR==FNR{wanted[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&$1 in wanted{print}' \
    "$activation" "$envelope_report" >"$tmp/envelope-activation.tsv"
diff -u <(report_rows "$report") "$tmp/envelope-activation.tsv" \
    || die 'activation receipt disagrees with oracle-envelope receipt'

generated=$tmp/transitions.tsv
{
    echo '# Scoped $262.createRealm host-hook admission transition.'
    echo "# before_oxide_profile_sha256=$profile_sha"
    echo "# after_oxide_profile_sha256=$profile_sha"
    echo "# manifest_sha256=$(value oracle_envelope_sha256)"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' 'BEGIN{OFS="\t"}
        NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old))exit 2
            split(old[key],a,FS);for(i=1;i<=6;i++)if($i!=a[i])exit 3
            print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
            seen[key]=1
        }' "$parent_report" "$envelope_report"
} >"$generated"
[[ "$(sha "$generated")" == "$(value transition_sha256)" \
    && "$(report_rows "$generated" | sha /dev/stdin)" \
        == "$(value transition_data_sha256)" ]] \
    || die 'generated createRealm transition checksum drifted'
diff -u "$transition" "$generated"
transition_counts=$(awk -F'\t' '
    NR==FNR{class[$0]="activation";next}
    FILENAME==ARGV[2]{class[$0]="supplemental";next}
    /^#/||($1=="path"&&$2=="variant"){next}
    {
        if($7!="unsupported-host-create-realm"||$8!="selection"||$9!="HostCapability"||
            $10!="missing execution capabilities: create-realm")exit 2
        if(class[$1]=="activation"){
            if($11!="pass"||$12!="normal"||$13!=""||$14!="")exit 3
            passed++
        }else if(class[$1]=="supplemental"){
            if($11!="unsupported-feature"||$12!="selection"||$13!="EngineCapability"||
                $14!="quickjs-oxide does not declare Test262 feature support: Atomics, SharedArrayBuffer")exit 4
            feature++
        }else exit 5
        changed++
    }
    END{printf "pass=%d feature=%d changed=%d outcome=%d detail=0 unchanged=0",passed,feature,changed,changed}
' "$activation" "$supplemental_feature" "$generated") \
    || die 'createRealm transition semantics drifted'
[[ "$transition_counts" == 'pass=150 feature=2 changed=152 outcome=152 detail=0 unchanged=0' ]] \
    || die "createRealm transition partition drifted: $transition_counts"

check_static_inputs
echo 'Test262 createRealm gate passes: direct 281/545 certified; QuickJS oracle 152/152; Oxide activates 150/150 with 2 supplemental feature rows kept fail-closed.'
