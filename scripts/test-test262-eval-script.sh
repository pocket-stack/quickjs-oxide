#!/usr/bin/env bash
# Reproduce the checksum-bound scoped $262.evalScript host-hook admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-eval-script-baseline.txt
live_profile=compat/test262-oxide.conf
staging_profile=tests/test262-realm-hosts-global-parent.conf
profile=tests/test262-eval-script.conf
upstream=compat/upstream.toml
current_global_gate=scripts/test-test262-current-global.sh
universe=tests/test262-eval-script-universe.txt
activation=tests/test262-eval-script-activation.txt
reason_only=tests/test262-eval-script-reason-only.txt
config_excluded=tests/test262-eval-script-config-excluded.txt
config_skipped=tests/test262-eval-script-config-skipped-feature.txt
parent_report=tests/test262-eval-script-parent.tsv
parent_json=tests/test262-eval-script-parent.jsonl
transition=tests/test262-eval-script-transitions.tsv
staging_report=target/test262-eval-script-staging.tsv
report=target/test262-eval-script.tsv
oracle_log=target/test262-eval-script-quickjs.log
workers=${TEST262_WORKERS:-8}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
live_profile_sha=01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75
staging_profile_sha=c671ae022251a9a0f7d17cc851db7506d825c34854c69adedc6475d3da0f389f
profile_sha=12e106bf0b0e3ea7ed24ee81ead260185864ab69ddead54559f7fe9aed65fc4e
universe_sha=cb1dda026b6f952cb77f0dbcdf2127ebcde8397789a63e4b4d7fb341e5b52afe
universe_keys_sha=7c12f7134a108d37196d5fbc8d7d86c1b14a907665ceebe2d9c3e961c0d6f5fe
metadata_projection_sha=e8b87a5208fa9ba636a67aa2403a28aed04b1c39352b6abd50fcb8a96ed7091a
source_hook_projection_sha=e8b2c0d5dda852189658e3dee8557f2e43a10c1af2dd7608e794a7ff6be261b0
create_realm_sha=51baaf9409286fd52aebb409c83700c2da4be506b588c9ea1dc7c32e9e5c7355
empty_sha=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
oracle_receipt_sha=31d9d85ebb2eca92df006c4b6ac3d25cd798aea07e76c53728088d5f4bcaf981
parent_tsv_sha=2d35a58ed87619a5081b77f5d42be0bb0b961389e321befcc00616fe8ad72a05
parent_json_sha=38df30f995c4ba195d4c64372e604ec8436a8a0a2abd0b6c0bc83dbeecefb33b
staging_tsv_sha=cc46f1a926b4f9a06e44d8810abd060efe8a13ad28b498d3c1fba0a91579ece5
staging_json_sha=d286d0e164f535005cd4c2ca90eba9f48cba944420f08b3228d4ca3d813da4d6
candidate_tsv_sha=218c010c3910bb1f6c77ae48fab93764e16383d47c1026d603780188725d0036
candidate_json_sha=aff095a3941aaa81e5716d592ec082ad9145142801fcfcb2c430890e9ce3a883
transition_sha=8717ada533f527bcdcf1674284c0de8a3436c108843383474f9115ec47e0cf80
transition_data_sha=9a437e388fff2ac7faeccd46f7775170fb8c5dc561872a60b66b4c0cc498b1ea
baseline_sha=d3b7953c7e4ed313548460167640e49f8f0d3318409d5b0c0959821a6608f107

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify authenticated inputs and pinned QuickJS 44/44 only\n'
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
check_file() {
    local file=$1 count=$2 digest=$3
    [[ -f "$file" && "$(lines "$file")" == "$count" \
        && "$(sha "$file")" == "$digest" ]] \
        || die "authenticated input drifted: $file"
}
check_current_live_profile() {
    local actual_profile_sha
    [[ -f "$live_profile" ]] || die "missing live Test262 profile: $live_profile"
    actual_profile_sha=$(sha "$live_profile")
    [[ "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" \
            == "$actual_profile_sha" ]] \
        || die 'compat/upstream.toml does not authenticate the current live profile'
    if [[ "$actual_profile_sha" == "$live_profile_sha" ]]; then
        check_file "$live_profile" 1274 "$live_profile_sha"
    else
        "$current_global_gate" --check >/dev/null
    fi
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
        function has(list,item){return index("," list ",", "," item ",")!=0}
        NR==FNR{wanted[$0]=1;next}
        $1 in wanted{
            if(has($3,"module")||has($3,"noStrict")||has($3,"raw"))print $1 "\tsloppy"
            else if(has($3,"onlyStrict"))print $1 "\tstrict"
            else{print $1 "\tsloppy";print $1 "\tstrict"}
        }
    ' "$1" "$metadata_tsv" | sort
}

json_result_projection() {
    awk -v report="$1" '
        function fail(message){
            printf "error: evalScript JSONL projection %s: %s\n",report,message >"/dev/stderr"
            exit 2
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
            sub(/^\{"kind":"summary","outcomes":\{/,"");sub(/\}\}$/,"")
            gsub(/":/,"=");gsub(/"/,"");gsub(/,/," ");print;found++
        }
        END{if(found!=1)exit 1}'
}
expected_json_metadata() {
    local profile_digest=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"test262-canonical-classified-v2","mode":"both"}\n' \
        "$quickjs" "$test262" "$patch_sha" "$config_sha" "$metadata_sha" "$profile_digest"
}
verify_report() {
    local tsv=$1 json=$2 profile_digest=$3 expected_tsv=$4 expected_json=$5
    local projection=$tmp/projection.$$.tsv
    [[ -f "$tsv" && -f "$json" \
        && "$(header "$tsv" quickjs)" == "$quickjs" \
        && "$(header "$tsv" test262)" == "$test262" \
        && "$(header "$tsv" test262_patch_sha256)" == "$patch_sha" \
        && "$(header "$tsv" test262_config_sha256)" == "$config_sha" \
        && "$(header "$tsv" test262_metadata_sha256)" == "$metadata_sha" \
        && "$(header "$tsv" oxide_profile_sha256)" == "$profile_digest" \
        && "$(header "$tsv" profile)" == test262-canonical-classified-v2 \
        && "$(header "$tsv" mode)" == both \
        && "$(lines "$tsv")" == 55 && "$(lines "$json")" == 46 \
        && "$(sha "$tsv")" == "$expected_tsv" \
        && "$(sha "$json")" == "$expected_json" \
        && "$(report_summary "$tsv")" == "$(computed_summary "$tsv")" \
        && "$(head -n 1 "$json")" == "$(expected_json_metadata "$profile_digest")" \
        && "$(json_summary "$json")" == "$(report_summary "$tsv")" \
        && "$(report_keys "$tsv" | sha /dev/stdin)" == "$universe_keys_sha" ]] \
        || die "report identity drifted: $tsv"
    [[ "$(report_keys "$tsv" | uniq -d | wc -l | tr -d '[:space:]')" == 0 ]] \
        || die "report contains duplicate keys: $tsv"
    json_result_projection "$json" >"$projection" \
        || die "JSONL projection failed: $json"
    diff -u <(report_rows "$tsv") "$projection" \
        || die "JSONL/TSV ten-field projection drifted: $json"
}

check_inputs() {
    check_file "$baseline" 80 "$baseline_sha"
    check_file "$staging_profile" 1272 "$staging_profile_sha"
    check_file "$profile" 8 "$profile_sha"
    check_file "$universe" 31 "$universe_sha"
    check_file "$activation" 31 "$universe_sha"
    check_file "$reason_only" 0 "$empty_sha"
    check_file "$config_excluded" 0 "$empty_sha"
    check_file "$config_skipped" 0 "$empty_sha"
    check_file "$parent_report" 55 "$parent_tsv_sha"
    check_file "$parent_json" 46 "$parent_json_sha"
    check_file "$transition" 49 "$transition_sha"
    check_current_live_profile
    sort -c "$universe"
    sort -c "$activation"
    [[ -z "$(uniq -d "$universe")" && -z "$(uniq -d "$activation")" ]] \
        || die 'evalScript manifests contain duplicate paths'
    cmp -s "$universe" "$activation" \
        || die 'featureless evalScript universe is not its exact activation cohort'
    [[ "$(value quickjs)" == "$quickjs" \
        && "$(value test262)" == "$test262" \
        && "$(value live_oxide_profile_sha256)" == "$live_profile_sha" \
        && "$(value runtime_staging_oxide_profile)" == "$staging_profile" \
        && "$(value runtime_staging_oxide_profile_sha256)" == "$staging_profile_sha" \
        && "$(value candidate_oxide_profile_sha256)" == "$profile_sha" \
        && "$(value universe_sha256)" == "$universe_sha" \
        && "$(value universe_keys_sha256)" == "$universe_keys_sha" \
        && "$(value source_hook_projection_sha256)" == "$source_hook_projection_sha" \
        && "$(value metadata_projection_sha256)" == "$metadata_projection_sha" \
        && "$(value create_realm_sha256)" == "$create_realm_sha" \
        && "$(value parent_tsv_sha256)" == "$parent_tsv_sha" \
        && "$(value parent_jsonl_sha256)" == "$parent_json_sha" \
        && "$(value runtime_staging_tsv_sha256)" == "$staging_tsv_sha" \
        && "$(value runtime_staging_jsonl_sha256)" == "$staging_json_sha" \
        && "$(value candidate_tsv_sha256)" == "$candidate_tsv_sha" \
        && "$(value candidate_jsonl_sha256)" == "$candidate_json_sha" \
        && "$(value transition_sha256)" == "$transition_sha" \
        && "$(value transition_data_sha256)" == "$transition_data_sha" \
        && "$(value quickjs_receipt_sha256)" == "$oracle_receipt_sha" ]] \
        || die 'evalScript baseline identity drifted'
    [[ "$(section "$staging_profile" features | wc -l | tr -d '[:space:]')" == 102 \
        && "$(section "$staging_profile" audited-negative-tests | wc -l | tr -d '[:space:]')" == 1157 \
        && "$(section "$staging_profile" execution)" == async=true \
        && -z "$(section "$staging_profile" features \
            | grep -Ex 'cross-realm|host-(create-realm|eval-script)-required' || true)" ]] \
        || die 'runtime staging profile semantics drifted'
    [[ "$(section "$profile" features)" == host-eval-script-required \
        && -z "$(section "$profile" audited-negative-tests)" \
        && -z "$(section "$profile" execution)" ]] \
        || die 'scoped evalScript profile semantics drifted'
    [[ "$(toml_test262_value "$upstream" repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$test262" \
        && "$(toml_test262_value "$upstream" shallow_since)" == 2025-09-01 \
        && "$(toml_test262_value "$upstream" patch_sha256)" == "$patch_sha" \
        && "$(toml_test262_value "$upstream" config_sha256)" == "$config_sha" \
        && "$(toml_test262_value "$upstream" test_count)" == 53125 \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" == "$metadata_sha" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" \
            == "$(sha "$live_profile")" ]] \
        || die 'compat/upstream.toml Test262 identity drifted'
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-eval-script.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_inputs
verify_report "$parent_report" "$parent_json" "$staging_profile_sha" \
    "$parent_tsv_sha" "$parent_json_sha"
[[ "$(report_summary "$parent_report")" == 'unsupported-host-eval-script=44' \
    && "$(report_runnable "$parent_report")" == 0 \
    && "$(report_count unsupported-host-eval-script "$parent_report")" == 44 \
    && "$(report_rows "$parent_report" | awk -F'\t' '
        $7!="unsupported-host-eval-script"||$8!="selection"||
        $9!="HostCapability"||$10!="missing execution capabilities: eval-script"{bad++}
        END{print bad+0}')" == 0 ]] \
    || die 'historical evalScript parent receipt semantics drifted'

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

metadata_projection=$tmp/metadata-projection.tsv
awk -F'\t' 'NR==FNR{wanted[$0]=1;next}$1 in wanted{print}' \
    "$universe" "$metadata_tsv" | sort >"$metadata_projection"
[[ "$(lines "$metadata_projection")" == 31 \
    && "$(sha "$metadata_projection")" == "$metadata_projection_sha" ]] \
    || die 'evalScript metadata projection drifted'
awk -F'\t' '
    $2!=""&&$2!="propertyHelper.js"&&$2!="fnGlobalObject.js,propertyHelper.js"{exit 1}
    $3!=""&&$3!="noStrict"&&$3!="generated,noStrict"{exit 2}
    $4!=""||$5!=""||$6!=""{exit 3}
    {flags[$3]++;includes[$2]++}
    END{
        if(flags[""]!=13||flags["noStrict"]!=2||flags["generated,noStrict"]!=16)exit 4
        if(includes[""]!=13||includes["propertyHelper.js"]!=2||
            includes["fnGlobalObject.js,propertyHelper.js"]!=16)exit 5
    }' "$metadata_projection" \
    || die 'evalScript metadata gained features, negatives, modes, or includes'
cut -f1 "$metadata_projection" >"$tmp/metadata.paths"
diff -u "$universe" "$tmp/metadata.paths"

git -C "$suite" grep -l -F '$262.evalScript' -- 'test/**/*.js' \
    | sort >"$tmp/source.paths"
[[ "$(lines "$tmp/source.paths")" == 31 \
    && "$(sha "$tmp/source.paths")" == "$universe_sha" ]] \
    || die 'direct $262.evalScript source universe drifted'
diff -u "$universe" "$tmp/source.paths"
git -C "$suite" grep -El '\$262[[:space:]]*\.[[:space:]]*evalScript([^[:alnum:]_$]|$)' \
    -- 'test/**/*.js' | sort >"$tmp/flexible-source.paths"
diff -u "$universe" "$tmp/flexible-source.paths" \
    || die 'evalScript source spelling inventory drifted'
[[ "$(git -C "$suite" grep -o -F '$262.evalScript' -- 'test/**/*.js' | wc -l | tr -d '[:space:]')" == 87 ]] \
    || die 'direct $262.evalScript call-site count drifted'

: >"$tmp/source-hooks.tsv"
while IFS= read -r test_path; do
    grep -Eo '\$262\.[A-Za-z_]+' "$suite/$test_path" | sort -u \
        | sed "s#^#$test_path\t#" >>"$tmp/source-hooks.tsv"
    if grep -Eq '\$262[[:space:]]*\[' "$suite/$test_path"; then
        die "evalScript path gained computed host access: $test_path"
    fi
done <"$universe"
[[ "$(lines "$tmp/source-hooks.tsv")" == 31 \
    && "$(sha "$tmp/source-hooks.tsv")" == "$source_hook_projection_sha" \
    && -z "$(awk -F'\t' '$2!="$262.evalScript"{print}' "$tmp/source-hooks.tsv")" ]] \
    || die 'evalScript source hook projection drifted'

git -C "$suite" grep -l -F '$262.createRealm' -- 'test/**/*.js' \
    | sort >"$tmp/create-realm.paths"
[[ "$(lines "$tmp/create-realm.paths")" == 281 \
    && "$(sha "$tmp/create-realm.paths")" == "$create_realm_sha" \
    && -z "$(comm -12 "$universe" "$tmp/create-realm.paths")" ]] \
    || die 'evalScript/createRealm direct-source boundary drifted'

variant_keys "$universe" >"$tmp/universe.keys"
[[ "$(lines "$tmp/universe.keys")" == 44 \
    && "$(sha "$tmp/universe.keys")" == "$universe_keys_sha" ]] \
    || die 'evalScript variant-key inventory drifted'
diff -u "$tmp/universe.keys" <(report_keys "$parent_report")
{ cat "$activation"; cat "$reason_only"; cat "$config_excluded"; cat "$config_skipped"; } \
    | sort >"$tmp/partition.paths"
diff -u "$universe" "$tmp/partition.paths" \
    || die 'evalScript activation/reason/config partition is not exhaustive and disjoint'

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$universe"
if ! (cd "$source_dir" && ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS failed the evalScript universe'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log"; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS evalScript oracle reported a failure or feature skip'
fi
grep -F 'Average memory statistics for 44 tests:' "$oracle_log" \
    >"$tmp/quickjs.receipt"
[[ "$(lines "$tmp/quickjs.receipt")" == 1 \
    && "$(sha "$tmp/quickjs.receipt")" == "$oracle_receipt_sha" ]] \
    || die 'pinned QuickJS evalScript oracle receipt drifted'

if "$check_only"; then
    check_inputs
    echo 'Test262 evalScript inputs verified: direct 31 paths/44 variants, zero createRealm overlap, QuickJS 44/44.'
    exit 0
fi

run_candidate() {
    local run_profile=$1 output=$2 run_workers=$3
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$run_profile" --manifest "$activation" \
        --report "$output" --mode both --timeout-ms 30000 \
        --workers "$run_workers" --allow-failures >/dev/null
}
run_candidate "$staging_profile" "$staging_report" "$workers"
verify_report "$staging_report" "${staging_report%.tsv}.jsonl" \
    "$staging_profile_sha" "$staging_tsv_sha" "$staging_json_sha"
[[ "$(report_summary "$staging_report")" == 'unsupported-feature=44' \
    && "$(report_runnable "$staging_report")" == 0 \
    && "$(report_count unsupported-feature "$staging_report")" == 44 \
    && "$(report_rows "$staging_report" | awk -F'\t' '
        $7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
        $10!="quickjs-oxide does not declare Test262 feature support: host-eval-script-required"{bad++}
        END{print bad+0}')" == 0 ]] \
    || die 'runtime staging profile did not keep evalScript globally closed'

run_candidate "$profile" "$report" "$workers"
repeat_report=$tmp/repeat.tsv
repeat_workers=1
[[ "$workers" == 1 ]] && repeat_workers=2
run_candidate "$profile" "$repeat_report" "$repeat_workers"
cmp -s "$report" "$repeat_report" \
    && cmp -s "${report%.tsv}.jsonl" "${repeat_report%.tsv}.jsonl" \
    || die 'focused evalScript receipts are not repeatable across worker counts'
verify_report "$report" "${report%.tsv}.jsonl" "$profile_sha" \
    "$candidate_tsv_sha" "$candidate_json_sha"
[[ "$(report_summary "$report")" == 'pass=44' \
    && "$(report_runnable "$report")" == 44 \
    && "$(report_count pass "$report")" == 44 \
    && "$(report_rows "$report" | awk -F'\t' '
        $7!="pass"||$8!="normal"||$9!=""||$10!=""{bad++} END{print bad+0}')" == 0 ]] \
    || die 'candidate evalScript receipt semantics drifted'

generated=$tmp/transitions.tsv
{
    echo '# Scoped Test262 $262.evalScript host-hook admission transition.'
    echo "# before_oxide_profile_sha256=$staging_profile_sha"
    echo "# after_oxide_profile_sha256=$profile_sha"
    echo "# manifest_sha256=$universe_sha"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' 'BEGIN{OFS="\t"}
        NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;count[$1 FS $2]++}next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(count[key]!=1)exit 2;split(old[key],a,FS)
            for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
            seen[key]++
        }
        END{for(key in count)if(seen[key]!=1)exit 4}' \
        "$parent_report" "$report"
} >"$generated" || die 'evalScript transition join was not bijective'
[[ "$(sha "$generated")" == "$transition_sha" \
    && "$(report_rows "$generated" | sha /dev/stdin)" == "$transition_data_sha" ]] \
    || die 'generated evalScript transition checksum drifted'
diff -u "$transition" "$generated"
counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if($7!="unsupported-host-eval-script"||$8!="selection"||
        $9!="HostCapability"||$10!="missing execution capabilities: eval-script"||
        $11!="pass"||$12!="normal"||$13!=""||$14!="")exit 2
    if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
} END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' \
    "$generated") || die 'evalScript transition semantics drifted'
[[ "$counts" == 'changed=44 outcome=44 detail=0 unchanged=0' ]] \
    || die "evalScript transition partition drifted: $counts"

check_inputs
echo 'Test262 evalScript gate passes: QuickJS 44/44; Oxide 0/44 -> 44/44, zero createRealm overlap, deterministic receipts.'
