#!/usr/bin/env bash
# Reproduce the checksum-bound R3ch scoped $262.gc host-hook admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-host-gc-baseline.txt
profile=tests/test262-host-gc.conf
live_profile=compat/test262-oxide.conf
global_parent=tests/test262-host-gc-global-parent.conf
global_candidate=tests/test262-host-gc-global-candidate.conf
upstream=compat/upstream.toml
current_global_gate=scripts/test-test262-current-global.sh
universe=tests/test262-host-gc-universe.txt
activation=tests/test262-host-gc-activation.txt
create_realm=tests/test262-host-gc-create-realm-deferred.txt
parent_report=tests/test262-host-gc-parent.tsv
parent_json=tests/test262-host-gc-parent.jsonl
transition=tests/test262-host-gc-transitions.tsv
report=target/test262-host-gc.tsv
oracle_log=target/test262-host-gc-quickjs.log
workers=${TEST262_WORKERS:-8}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
parent_profile_sha=8be6c2a3892a62d89ed17df3f3d3b54e9e84fda8ef6be2bcdaa7d49044593990
live_profile_sha=01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75
historical_candidate_sha=c671ae022251a9a0f7d17cc851db7506d825c34854c69adedc6475d3da0f389f
profile_sha=496a4bc7538454b8b14e361d922679d401a5be0ed3af4b2be0cdbf1a3761ed99
profile_features_sha=ef3dd040cc4be53129ef57f369eb7bceaf46e51fa5ddd2c77b1677bbdf930fd8
universe_sha=4ab5a2feb62b100afd4aa5e6afd9f418c415142a1678d1c2c1c9de9d1553c630
universe_keys_sha=46c6aea579898a22ab416c3d6c68b696ecb95fa5d4d8a3e19b4e068a1e0d3610
metadata_projection_sha=60a4208e8f779a1e33de905f8501eec6b213968eff089ee14cdfd93d11abb176
source_hook_projection_sha=b4d3e0c2a96a32ea44d90c51056e57aec1888a1f0a55207ae4f0489c3036fe36
activation_sha=bdecdca8dbf0517221fbb8e403cacb0467add39c879b5659f71a7a838912fd74
activation_keys_sha=79ebef1024667e98fcb3ef3bfa4b11ec42cd142693d5bb6a60cb82136b985916
create_realm_sha=dc1778245ae4947fd29c7b2deb89c647ed57dd3bd85b84729ca4e7ea130a09a0
create_realm_keys_sha=33ec8c5bb476ae2de0c9031ac7ac2b020c107c2ea5276008f9a1bc37e9ccf0f0
parent_tsv_sha=6bfffe4686469f4e129aba88bf18b0ac13d1c97ef9d31d8afc013c03dd33e917
parent_json_sha=a3a354d2e1bd8bc030e7f4e96db94cfea5e33966c4481674639e82b4d16d3216
candidate_tsv_sha=78ba543fd816f68a82ce264353478bc94dc4dfa067663d88d80a44fc51699211
candidate_json_sha=a3ae18bfc094bb7957cf45adaa3efa42830163e2eed912f9b754f6fe60ee770e
successor_candidate_tsv_sha=df68f77260e3ce6828b42948d0974eeda05e5056aa89434380f82bbff2d98ad5
successor_candidate_json_sha=f3ef207701b629537b6f9529b9f3ee5de4387f1b448f482c1e2fb01e6bb6c45d
transition_sha=a1bad8f21025d99ff186c1f4823d97f0624ccf3b62f42f4d461a9f6a7c9f0252
transition_data_sha=d349016f4b72388e1324c32bee65b845538d524f276ed46cf177f155343649ee
baseline_sha=ee07e73b49c52eadae7198ec80780923237ef4d42b282b967e379e923989833a

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify authenticated inputs and QuickJS 28/28 only\n'
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
section() {
    awk -v wanted="[$2]" \
        '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' \
        "$1"
}
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
check_file() {
    local file=$1 count=$2 digest=$3
    [[ -f "$file" && "$(lines "$file")" == "$count" \
        && "$(sha "$file")" == "$digest" ]] \
        || die "authenticated input drifted: $file"
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
    variant_keys "$1" >"$4"
    [[ "$(lines "$4")" == "$2" && "$(sha "$4")" == "$3" ]] \
        || die "variant-key inventory drifted: $1"
}

json_result_projection() {
    awk -v report="$1" '
        function fail(message){
            printf "error: host-gc JSONL projection %s: %s\n",report,message >"/dev/stderr"
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
            sub(/^\{"kind":"summary","outcomes":\{/,"");sub(/\}\}$/,"")
            gsub(/":/,"=");gsub(/"/,"");gsub(/,/," ");print;found++
        }
        END{if(found!=1)exit 1}'
}
expected_json_metadata() {
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"test262-canonical-classified-v2","mode":"both"}\n' \
        "$quickjs" "$test262" "$patch_sha" "$config_sha" "$metadata_sha" "$1"
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
        && "$(lines "$tsv")" == 39 && "$(lines "$json")" == 30 \
        && "$(sha "$tsv")" == "$expected_tsv" \
        && "$(sha "$json")" == "$expected_json" \
        && "$(report_summary "$tsv")" == "$(computed_summary "$tsv")" \
        && "$(head -n 1 "$json")" == "$(expected_json_metadata "$profile_digest")" \
        && "$(json_summary "$json")" == "$(report_summary "$tsv")" ]] \
        || die "report identity drifted: $tsv"
    json_result_projection "$json" >"$projection" \
        || die "JSONL projection failed: $json"
    diff -u <(report_rows "$tsv") "$projection" \
        || die "JSONL/TSV ten-field projection drifted: $json"
    [[ "$(report_keys "$tsv" | sha /dev/stdin)" == "$universe_keys_sha" ]] \
        || die "report key inventory drifted: $tsv"
}

check_inputs() {
    check_file "$baseline" 68 "$baseline_sha"
    check_file "$profile" 6 "$profile_sha"
    check_file "$global_parent" 1271 "$parent_profile_sha"
    check_file "$global_candidate" 1272 "$historical_candidate_sha"
    check_file "$universe" 15 "$universe_sha"
    check_file "$activation" 14 "$activation_sha"
    check_file "$create_realm" 1 "$create_realm_sha"
    check_file "$parent_report" 39 "$parent_tsv_sha"
    check_file "$parent_json" 30 "$parent_json_sha"
    check_file "$transition" 33 "$transition_sha"
    check_current_live_profile
    for file in "$universe" "$activation" "$create_realm"; do sort -c "$file"; done
    [[ "$(value quickjs)" == "$quickjs" \
        && "$(value test262)" == "$test262" \
        && "$(value parent_oxide_profile_sha256)" == "$parent_profile_sha" \
        && "$(value live_oxide_profile_sha256)" == "$live_profile_sha" \
        && "$(value historical_global_candidate_profile)" == "$global_candidate" \
        && "$(value historical_global_candidate_profile_sha256)" == "$historical_candidate_sha" \
        && "$(value candidate_tsv_sha256)" == "$candidate_tsv_sha" \
        && "$(value candidate_jsonl_sha256)" == "$candidate_json_sha" \
        && "$(value successor_candidate_tsv_sha256)" == "$successor_candidate_tsv_sha" \
        && "$(value successor_candidate_jsonl_sha256)" == "$successor_candidate_json_sha" \
        && "$(value successor_candidate_summary)" == 'pass=26 unsupported-feature=2' \
        && "$(value transition_sha256)" == "$transition_sha" \
        && "$(value transition_data_sha256)" == "$transition_data_sha" ]] \
        || die 'focused baseline identity drifted'
    [[ "$(section "$profile" features | wc -l | tr -d '[:space:]')" == 1 \
        && "$(section "$profile" features | sha /dev/stdin)" == "$profile_features_sha" \
        && "$(section "$profile" features)" == host-gc-required \
        && -z "$(section "$profile" audited-negative-tests)" \
        && -z "$(section "$profile" execution)" ]] \
        || die 'scoped host-gc profile semantics drifted'
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
check_inputs
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-host-gc.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM

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
awk -F'\t' '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    has($4,"host-gc-required"){print}
' "$metadata_tsv" | sort >"$metadata_projection"
[[ "$(lines "$metadata_projection")" == 15 \
    && "$(sha "$metadata_projection")" == "$metadata_projection_sha" ]] \
    || die 'host-gc metadata projection drifted'
awk -F'\t' '($2!=""&&$2!="sm/non262-generators-shell.js")||
    ($3!=""&&$3!="noStrict")||$4!="host-gc-required"||$5!=""||$6!=""{exit 1}' \
    "$metadata_projection" || die 'host-gc metadata gained unsupported structure'
cut -f1 "$metadata_projection" >"$tmp/metadata.paths"
diff -u "$universe" "$tmp/metadata.paths"

git -C "$suite" grep -l -F '$262.gc' -- 'test/**/*.js' | sort >"$tmp/source.paths"
[[ "$(lines "$tmp/source.paths")" == 15 \
    && "$(sha "$tmp/source.paths")" == "$universe_sha" ]] \
    || die 'host-gc source universe drifted'
diff -u "$universe" "$tmp/source.paths"

: >"$tmp/source-hooks.tsv"
: >"$tmp/create-realm.paths"
while IFS= read -r test_path; do
    grep -Eo '\$262\.[A-Za-z_]+' "$suite/$test_path" | sort -u \
        | sed "s#^#$test_path\t#" >>"$tmp/source-hooks.tsv"
    grep -Fq '$262.createRealm' "$suite/$test_path" \
        && printf '%s\n' "$test_path" >>"$tmp/create-realm.paths"
done <"$universe"
[[ "$(sha "$tmp/source-hooks.tsv")" == "$source_hook_projection_sha" ]] \
    || die 'host-gc source hook projection drifted'
awk -F'\t' '$2!="$262.gc"&&$2!="$262.detachArrayBuffer"&&$2!="$262.createRealm"{exit 1}' \
    "$tmp/source-hooks.tsv" || die 'host-gc universe gained an unknown host hook'
sort -u -o "$tmp/create-realm.paths" "$tmp/create-realm.paths"
diff -u "$create_realm" "$tmp/create-realm.paths"
comm -23 "$universe" "$create_realm" >"$tmp/activation.paths"
diff -u "$activation" "$tmp/activation.paths"

check_keys "$universe" 28 "$universe_keys_sha" "$tmp/universe.keys"
check_keys "$activation" 26 "$activation_keys_sha" "$tmp/activation.keys"
check_keys "$create_realm" 2 "$create_realm_keys_sha" "$tmp/create-realm.keys"
{ cat "$tmp/activation.keys"; cat "$tmp/create-realm.keys"; } | sort >"$tmp/partition.keys"
diff -u "$tmp/universe.keys" "$tmp/partition.keys"

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$universe"
if ! (cd "$source_dir" && ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS failed the host-gc universe'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || ! grep -Fq 'Average memory statistics for 28 tests:' "$oracle_log"; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS host-gc oracle receipt drifted'
fi

verify_report "$parent_report" "$parent_json" "$parent_profile_sha" \
    "$parent_tsv_sha" "$parent_json_sha"
[[ "$(report_summary "$parent_report")" \
        == 'unsupported-host-create-realm=2 unsupported-host-gc=26' \
    && "$(report_runnable "$parent_report")" == 0 \
    && "$(report_count unsupported-host-gc "$parent_report")" == 26 \
    && "$(report_count unsupported-host-create-realm "$parent_report")" == 2 ]] \
    || die 'historical parent receipt semantics drifted'

if "$check_only"; then
    check_inputs
    echo 'Test262 host-gc inputs verified: QuickJS passes 28/28; 26 activation and 2 createRealm-deferred variants authenticated.'
    exit 0
fi

run_candidate() {
    local output=$1 run_workers=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$universe" \
        --report "$output" --mode both --timeout-ms 30000 \
        --workers "$run_workers" --allow-failures >/dev/null
}
run_candidate "$report" "$workers"
repeat_report=$tmp/repeat.tsv
run_candidate "$repeat_report" 1
cmp -s "$report" "$repeat_report" \
    && cmp -s "${report%.tsv}.jsonl" "${repeat_report%.tsv}.jsonl" \
    || die 'focused host-gc receipts are not repeatable across worker counts'
verify_report "$report" "${report%.tsv}.jsonl" "$profile_sha" \
    "$successor_candidate_tsv_sha" "$successor_candidate_json_sha"
[[ "$(report_summary "$report")" == 'pass=26 unsupported-feature=2' \
    && "$(report_runnable "$report")" == 26 \
    && "$(report_count pass "$report")" == 26 \
    && "$(report_count unsupported-feature "$report")" == 2 ]] \
    || die 'R3ci successor host-gc receipt semantics drifted'

counts=$(awk -F'\t' '
    NR==FNR{class[$0]="activation";next}
    FILENAME==ARGV[2]{class[$0]="create-realm";next}
    /^#/||($1=="path"&&$2=="variant"){next}
    {
        kind=class[$1]
        if(kind=="activation"){
            if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2
            activation++
        }else if(kind=="create-realm"){
            if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
                $10!="quickjs-oxide does not declare Test262 feature support: host-create-realm-required")exit 3
            realm++
        }else exit 4
    }
    END{printf "activation=%d realm=%d",activation,realm}
' "$activation" "$create_realm" "$report") || die 'R3ci successor host-gc semantics drifted'
[[ "$counts" == 'activation=26 realm=2' ]] \
    || die "R3ci successor host-gc partition drifted: $counts"

check_inputs
echo 'Test262 host-gc gate passes: QuickJS 28/28; Oxide passes 26 and keeps 2 createRealm rows feature-gated in the current runtime.'
