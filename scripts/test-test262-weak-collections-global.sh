#!/usr/bin/env bash
# Reproduce the exhaustive WeakMap/WeakSet/symbol-key/upsert admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-weak-collections-global-baseline.txt
parent=tests/test262-weak-collections-global-parent.conf
candidate=tests/test262-weak-collections-global-candidate.conf
universe=tests/test262-weak-collections-global-universe.txt
activation=tests/test262-weak-collections-global-activation.txt
reason_only=tests/test262-weak-collections-global-reason-only.txt
supplemental=tests/test262-weak-collections-global-supplemental.txt
transition=tests/test262-weak-collections-global-transitions.tsv
parent_report=target/test262-weak-collections-global-parent.tsv
candidate_report=target/test262-weak-collections-global-candidate.tsv
parent_full=target/test262-weak-collections-global-parent-full.tsv
candidate_full=target/test262-weak-collections-global-candidate-full.tsv
oracle_log=target/test262-weak-collections-global-quickjs.log
draft=target/test262-weak-collections-global-baseline.draft.txt
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
parent_sha=f229cd652dd5b38ed3a0387a089eab974148d404bd166e8b4c0eb2cb0fa7a2c1
candidate_sha=3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef
universe_sha=d0bd5c21db1165cd72618168ce5592b78a6909be5f2cd0813fa15ed6a3c17cc1
activation_sha=7c9604ea45edd1c6f08875c0d2c9ece8c5166517bba127d950519ee0024c3f92
reason_sha=191be63fb08e41c5a51fc49e6c91e77aef25c5aca4d0374a30609059c813816c
supplemental_sha=1bc60b219226fb285211dcfd7f62bd00c25fdb03faaba7302d29a3a5f8dc2ca1
transition_sha=7d18cef62b857b175c34529b9147da6404b95114b12440cfa1e36212ffa6cf31
universe_keys_sha=2bf72c55541b84e9a4f0dac4a6eba4c6b073d5154801ae0cbce9d94a7472e319
activation_keys_sha=920d30c0e48f75ae77c39b89b32bf1b23d89cfce88ccb05a09ab51ffa430f184
reason_keys_sha=63086bdb2ec2f1beefff2d5473f660ef3e4595f9d38884f478158d83da79ac85
supplemental_keys_sha=bf71c49538be73565ce213c694a1122040008bf601520abe1a8e4622943f664b
all_keys_sha=69f0826f8f362d15c99b47e0fdd0aeb7dba2693f67abb255546f25cda026c797

usage() {
    printf 'usage: %s [--check|--full|--bless]\n' "${0##*/}"
    printf '  --check  verify authenticated inputs and the pinned QuickJS oracle\n'
    printf '  --full   additionally run and join both exact 102037-row profiles\n'
    printf '  --bless  write a reviewed tag-receipt draft under target/ only\n'
}

mode=tag
case ${1-} in
    '') ;;
    --check) mode=check ;;
    --full) mode=full ;;
    --bless) mode=bless ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] || { echo 'error: invalid TEST262_WORKERS' >&2; exit 2; }
[[ "$full_workers" =~ ^[1-9][0-9]*$ ]] || { echo 'error: invalid TEST262_FULL_WORKERS' >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
sha() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
    else shasum -a 256 "$1" | awk '{print $1}'; fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
manifest_paths() { awk 'NF && $1 !~ /^#/ {print}' "$1"; }
section() {
    awk -v wanted="[$2]" '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' "$1"
}
header() {
    awk -F= -v wanted="# $2" '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' "$1"
}
value() {
    awk -F= -v wanted="$1" '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' "$baseline"
}
report_rows() { awk -F'\t' '!/^#/ && !($1=="path"&&$2=="variant")' "$1"; }
check_file() {
    local file=$1 count=$2 digest=$3
    [[ -f "$file" ]] || die "missing gate input: $file"
    [[ "$(lines "$file")" == "$count" && "$(sha "$file")" == "$digest" ]] \
        || die "authenticated input drifted: $file"
}
variant_keys() {
    awk -F'\t' '
        function has(list,value){return index("," list ",", "," value ",")!=0}
        NR==FNR{wanted[$0]=1;next}
        $1 in wanted {
            if(has($3,"module")||has($3,"noStrict")||has($3,"raw")) print $1 "\tsloppy"
            else if(has($3,"onlyStrict")) print $1 "\tstrict"
            else {print $1 "\tsloppy"; print $1 "\tstrict"}
        }
    ' "$1" "$metadata_tsv" | sort
}
check_keys() {
    local paths=$1 count=$2 digest=$3 output=$4
    variant_keys "$paths" >"$output"
    [[ "$(lines "$output")" == "$count" && "$(sha "$output")" == "$digest" ]] \
        || die "variant-key inventory drifted: $paths"
}
check_report() {
    local report=$1 profile_sha=$2 summary=$3 label=$4
    [[ -f "$report" ]] || die "missing report: $report"
    [[ "$(header "$report" quickjs)" == "$quickjs" \
        && "$(header "$report" test262)" == "$test262" \
        && "$(header "$report" test262_patch_sha256)" == "$patch_sha" \
        && "$(header "$report" test262_config_sha256)" == "$config_sha" \
        && "$(header "$report" test262_metadata_sha256)" == "$metadata_sha" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == both \
        && "$(lines <(report_rows "$report"))" == 306 \
        && "$(report_rows "$report" | awk -F'\t' '{print $1 "\t" $2}' | sort | sha /dev/stdin)" == "$universe_keys_sha" \
        && "$(tail -n 1 "$report")" == "# summary $summary" ]] \
        || die "classified report drifted: $report"
    local json=${report%.tsv}.jsonl
    [[ -f "$json" && "$(lines "$json")" == 308 \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "report receipt drifted: $label"
}
run_report() {
    local profile=$1 report=$2 selection=$3 pool=$4
    local -a selected
    if [[ "$selection" == --all ]]; then selected=(--all)
    else selected=(--manifest "$universe"); fi
    rm -f -- "$report" "${report%.tsv}.jsonl"
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" "${selected[@]}" \
        --report "$report" --mode both \
        --workers "$pool" --timeout-ms 30000 --allow-failures
}

cd -- "$root"
[[ -f "$baseline" ]] || die "missing gate baseline: $baseline"
while IFS=: read -r key expected; do
    [[ "$(value "$key")" == "$expected" ]] || die "baseline identity drifted: $key"
done <<EOF
quickjs:$quickjs
test262:$test262
test262_patch_sha256:$patch_sha
test262_config_sha256:$config_sha
test262_metadata_records:53125
test262_metadata_sha256:$metadata_sha
schema:test262-canonical-classified-v2
mode:both
timeout_ms:30000
parent_oxide_profile_sha256:$parent_sha
candidate_oxide_profile_sha256:$candidate_sha
universe_sha256:$universe_sha
universe_keys_sha256:$universe_keys_sha
activation_sha256:$activation_sha
activation_keys_sha256:$activation_keys_sha
reason_only_sha256:$reason_sha
reason_only_keys_sha256:$reason_keys_sha
supplemental_sha256:$supplemental_sha
supplemental_keys_sha256:$supplemental_keys_sha
transition_receipt_sha256:$transition_sha
full_keys_sha256:$all_keys_sha
EOF
check_file "$parent" 1265 "$parent_sha"
check_file "$candidate" 1269 "$candidate_sha"
check_file "$universe" 154 "$universe_sha"
check_file "$activation" 147 "$activation_sha"
check_file "$reason_only" 7 "$reason_sha"
check_file "$supplemental" 4 "$supplemental_sha"
check_file "$transition" 311 "$transition_sha"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-weak-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
pfeatures=$tmp/parent.features
cfeatures=$tmp/candidate.features
section "$parent" features | sort >"$pfeatures"
section "$candidate" features | sort >"$cfeatures"
[[ "$(lines "$pfeatures")" == 95 \
    && "$(sha "$pfeatures")" == 07b67a0c074630e5da2e8f402fb66a3f6f0f3cba0f1593e3063f88f6cc5fcf6b \
    && "$(lines "$cfeatures")" == 99 \
    && "$(sha "$cfeatures")" == a892ce31bef675386670419a9410e6086c24f1edd9f8e14f6c793d8bfb07503b ]] \
    || die 'profile feature inventories drifted'
printf '%s\n' WeakMap WeakSet symbols-as-weakmap-keys upsert | sort >"$tmp/added"
diff -u "$tmp/added" <(comm -13 "$pfeatures" "$cfeatures")
[[ -z "$(comm -23 "$pfeatures" "$cfeatures")" ]] || die 'candidate removed a parent feature'
for name in audited-negative-tests execution; do
    section "$parent" "$name" >"$tmp/parent.$name"
    section "$candidate" "$name" >"$tmp/candidate.$name"
    diff -u "$tmp/parent.$name" "$tmp/candidate.$name"
done
[[ "$(lines "$tmp/parent.audited-negative-tests")" == 1157 \
    && "$(sha "$tmp/parent.audited-negative-tests")" == 709b3f86b0820c524cdd645a2993e7e17ae65f840936d388b9d7c890c2970412 \
    && "$(lines "$tmp/parent.execution")" == 1 \
    && "$(sha "$tmp/parent.execution")" == e26ec9bb60b6289635c1ab1347a0e7c7372cc5c329998c9c1504299da452acd8 ]] \
    || die 'non-feature profile sections drifted'

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
[[ "$(basename -- "$source_dir")" == "quickjs-$quickjs" \
    && "$(git -C "$suite" rev-parse 'HEAD^{commit}')" == "$test262" \
    && "$(sha "$source_dir/tests/test262.patch")" == "$patch_sha" \
    && "$(sha "$source_dir/test262.conf")" == "$config_sha" ]] \
    || die 'prepared QuickJS/Test262 inputs drifted'
metadata_bin=$tmp/metadata.bin
metadata_tsv=$tmp/metadata.tsv
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(lines <(tr '\0' '\t' <"$metadata_bin"))" == 53125 \
    && "$(sha "$metadata_bin")" == "$metadata_sha" ]] || die 'pinned metadata inventory drifted'
tr '\0' '\t' <"$metadata_bin" >"$metadata_tsv"

derived=$tmp/universe.paths
awk -F'\t' '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    has($4,"WeakMap")||has($4,"WeakSet")||has($4,"symbols-as-weakmap-keys")||has($4,"upsert"){print $1}
' "$metadata_tsv" | sort -u >"$derived"
[[ "$(lines "$derived")" == 154 && "$(sha "$derived")" == "$universe_sha" ]] \
    || die 'four-tag metadata universe drifted'
diff -u "$universe" "$derived"

awk -F'\t' 'NR==FNR{wanted[$0]=1;next} $1 in wanted {
    if($5!=""||$6!=""||($3!=""&&$3!="generated"&&$3!="onlyStrict")) print $1
}' "$derived" "$metadata_tsv" >"$tmp/invalid-metadata"
[[ ! -s "$tmp/invalid-metadata" ]] || die 'tag universe gained negative or unsupported metadata'
awk -F'\t' 'NR==FNR{w[$0]=1;next}$1 in w{n=split($4,a,",");for(i=1;i<=n;i++)if(a[i]!="")print a[i]}' \
    "$derived" "$metadata_tsv" | sort -u >"$tmp/features"
awk -F'\t' 'NR==FNR{w[$0]=1;next}$1 in w{n=split($2,a,",");for(i=1;i<=n;i++)if(a[i]!="")print a[i]}' \
    "$derived" "$metadata_tsv" | sort -u >"$tmp/includes"
awk -F'\t' 'NR==FNR{w[$0]=1;next}$1 in w&&$3=="onlyStrict"{print $1}' \
    "$derived" "$metadata_tsv" | sort >"$tmp/only-strict"
awk -F'\t' 'NR==FNR{w[$0]=1;next}$1 in w&&$3=="generated"{print $1}' \
    "$derived" "$metadata_tsv" | sort >"$tmp/generated"
[[ "$(lines "$tmp/features")" == 12 && "$(sha "$tmp/features")" == 658329632e3b9b9bfb5e52c2b3cdc9d599624524d1c885f0f1450d9e83dd6cba \
    && "$(lines "$tmp/includes")" == 3 && "$(sha "$tmp/includes")" == 4668b897190c3996c2141090fb75cc70a398fec78262fa08f96c8826acfe6f40 \
    && "$(lines "$tmp/only-strict")" == 2 && "$(sha "$tmp/only-strict")" == a8228daf9dbb84e6a3f60756a30318076b6b657c499caa521679a4bbe5c6cd79 \
    && "$(lines "$tmp/generated")" == 4 && "$(sha "$tmp/generated")" == dfbba072c6c953b1e24e909499b11151dbe2368a23c80e1223f7a81c9dc70ed2 ]] \
    || die 'tag metadata surface drifted'

: >"$tmp/activation"; : >"$tmp/reason"
awk -F'\t' -v yes="$tmp/activation" -v no="$tmp/reason" '
    NR==FNR{supported[$0]=1;next}
    function has(list,value){return index("," list ",", "," value ",")!=0}
    has($4,"WeakMap")||has($4,"WeakSet")||has($4,"symbols-as-weakmap-keys")||has($4,"upsert") {
        missing=0;n=split($4,f,",");for(i=1;i<=n;i++)if(!(f[i] in supported))missing=1
        print $1 > (missing ? no : yes)
    }
' "$cfeatures" "$metadata_tsv"
sort -u -o "$tmp/activation" "$tmp/activation"
sort -u -o "$tmp/reason" "$tmp/reason"
diff -u "$activation" "$tmp/activation"
diff -u "$reason_only" "$tmp/reason"
check_keys "$derived" 306 "$universe_keys_sha" "$tmp/universe.keys"
check_keys "$tmp/activation" 292 "$activation_keys_sha" "$tmp/activation.keys"
check_keys "$tmp/reason" 14 "$reason_keys_sha" "$tmp/reason.keys"
check_keys "$supplemental" 7 "$supplemental_keys_sha" "$tmp/supplemental.keys"

manifest_paths tests/test262-weak-collections.txt | sort -u >"$tmp/weak"
manifest_paths tests/test262-map.txt | sort -u >"$tmp/map"
comm -12 "$tmp/activation" "$tmp/weak" >"$tmp/weak-coverage"
comm -12 "$tmp/activation" "$tmp/map" >"$tmp/map-coverage"
check_keys "$tmp/weak-coverage" "$(value weak_coverage_variants)" \
    "$(value weak_coverage_keys_sha256)" "$tmp/weak-coverage.keys"
check_keys "$tmp/map-coverage" "$(value map_coverage_variants)" \
    "$(value map_coverage_keys_sha256)" "$tmp/map-coverage.keys"
[[ "$(lines "$tmp/weak-coverage")" == "$(value weak_coverage_paths)" \
    && "$(lines "$tmp/map-coverage")" == "$(value map_coverage_paths)" \
    && -z "$(comm -12 "$tmp/weak-coverage" "$tmp/map-coverage")" \
    && -z "$(comm -12 "$tmp/weak-coverage" "$supplemental")" \
    && -z "$(comm -12 "$tmp/map-coverage" "$supplemental")" ]] \
    || die 'focused/supplemental boundary drifted'
{ cat "$tmp/weak-coverage"; cat "$tmp/map-coverage"; cat "$supplemental"; } \
    | sort -u >"$tmp/coverage"
diff -u "$activation" "$tmp/coverage"

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r path; do files+=("test262/$path"); done <"$derived"
if ! (cd "$source_dir"; ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2; die 'pinned QuickJS failed the four-tag universe'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || ! grep -Fq 'Average memory statistics for 306 tests:' "$oracle_log"; then
    tail -n 100 "$oracle_log" >&2; die 'pinned QuickJS oracle receipt drifted'
fi

if [[ "$mode" == check ]]; then
    echo 'Weak collections global inputs verified: QuickJS passes 306 variants; Oxide activation is 292 + 14 reason-only.'
    exit 0
fi

run_report "$parent" "$parent_report" --manifest "$workers"
run_report "$candidate" "$candidate_report" --manifest "$workers"
check_report "$parent_report" "$parent_sha" 'unsupported-feature=306' parent
check_report "$candidate_report" "$candidate_sha" 'pass=292 unsupported-feature=14' candidate

generated_transition=$tmp/transitions.tsv
{
    echo '# R3ce exhaustive weak collections global admission transition.'
    echo "# before_oxide_profile_sha256=$parent_sha"
    echo "# after_oxide_profile_sha256=$candidate_sha"
    echo "# manifest_sha256=$universe_sha"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' 'BEGIN{OFS="\t"}
        NR==FNR{if(!/^#/&& !($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
        !/^#/&& !($1=="path"&&$2=="variant") {
            split(old[$1 FS $2],a,FS)
            print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
        }
    ' "$parent_report" "$candidate_report"
} >"$generated_transition"
[[ "$(sha "$generated_transition")" == "$transition_sha" ]] || die 'generated transition checksum drifted'
diff -u "$transition" "$generated_transition"

{ awk '{print $0 "\tactivation"}' "$activation"; awk '{print $0 "\treason"}' "$reason_only"; } \
    >"$tmp/classes"
counts=$(awk -F'\t' '
    NR==FNR{class[$1]=$2;next}
    /^#/||($1=="path"&&$2=="variant"){next}
    function has(list,value){return index("," list ",", "," value ",")!=0}
    function add(value){if(value!="") missing=missing (missing=="" ? "" : ", ") value}
    {
        missing="";if(has($4,"FinalizationRegistry"))add("FinalizationRegistry")
        if(has($4,"WeakMap"))add("WeakMap");if(has($4,"WeakRef"))add("WeakRef")
        if(has($4,"WeakSet"))add("WeakSet");if(has($4,"symbols-as-weakmap-keys"))add("symbols-as-weakmap-keys")
        if(has($4,"upsert"))add("upsert")
        before="quickjs-oxide does not declare Test262 feature support: " missing
        remaining="";if(has($4,"FinalizationRegistry"))remaining="FinalizationRegistry"
        if(has($4,"WeakRef"))remaining=remaining (remaining=="" ? "" : ", ") "WeakRef"
        after=remaining=="" ? "" : "quickjs-oxide does not declare Test262 feature support: " remaining
        if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||$10!=before)exit 2
        if(class[$1]=="activation") {
            if($11!="pass"||$12!="normal"||$13!=""||$14!="")exit 3; active++
        } else if(class[$1]=="reason") {
            if($11!="unsupported-feature"||$12!="selection"||$13!="EngineCapability"||$14!=after)exit 4; reason++
        } else exit 5
    }
    END{printf "activation=%d reason-only=%d",active,reason}
' "$tmp/classes" "$transition") || die 'transition semantics drifted'
[[ "$counts" == 'activation=292 reason-only=14' ]] || die "transition partition drifted: $counts"

if [[ "$mode" == bless ]]; then
    {
        echo 'schema=test262-weak-collections-global-receipt-v1'
        echo "parent_profile_sha256=$parent_sha"
        echo "candidate_profile_sha256=$candidate_sha"
        echo "universe_paths=154"
        echo "universe_variants=306"
        echo "activation_variants=292"
        echo "reason_only_variants=14"
        echo "parent_tsv_sha256=$(sha "$parent_report")"
        echo "parent_jsonl_sha256=$(sha "${parent_report%.tsv}.jsonl")"
        echo "candidate_tsv_sha256=$(sha "$candidate_report")"
        echo "candidate_jsonl_sha256=$(sha "${candidate_report%.tsv}.jsonl")"
        echo "transition_sha256=$transition_sha"
    } >"$draft"
    echo "Reviewed receipt draft written only to $draft"
    exit 0
fi

if [[ "$mode" == full ]]; then
    run_report "$parent" "$parent_full" --all "$full_workers"
    run_report "$candidate" "$candidate_full" --all "$full_workers"
    for spec in "$parent_full:$parent_sha" "$candidate_full:$candidate_sha"; do
        file=${spec%%:*}; profile_sha=${spec#*:}
        [[ "$(header "$file" oxide_profile_sha256)" == "$profile_sha" \
            && "$(lines <(report_rows "$file"))" == 102037 \
            && "$(report_rows "$file" | awk -F'\t' '{print $1 "\t" $2}' | sort | sha /dev/stdin)" == "$all_keys_sha" ]] \
            || die "full report identity drifted: $file"
    done
    [[ "$(sha "$parent_full")" == "$(value parent_full_tsv_sha256)" \
        && "$(sha "${parent_full%.tsv}.jsonl")" == "$(value parent_full_jsonl_sha256)" \
        && "$(sha "$candidate_full")" == "$(value candidate_full_tsv_sha256)" \
        && "$(sha "${candidate_full%.tsv}.jsonl")" == "$(value candidate_full_jsonl_sha256)" \
        && "$(tail -n 1 "$candidate_full")" == "# summary $(value candidate_full_summary)" ]] \
        || die 'full report receipts drifted'
    report_rows "$parent_report" >"$tmp/parent.focused"
    report_rows "$candidate_report" >"$tmp/candidate.focused"
    awk -F'\t' 'NR==FNR{w[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in w)' \
        "$derived" "$parent_full" >"$tmp/parent.full-focused"
    awk -F'\t' 'NR==FNR{w[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in w)' \
        "$derived" "$candidate_full" >"$tmp/candidate.full-focused"
    diff -u "$tmp/parent.focused" "$tmp/parent.full-focused"
    diff -u "$tmp/candidate.focused" "$tmp/candidate.full-focused"
    join_counts=$(awk -F'\t' -v keys="$tmp/universe.keys" -v parent="$parent_full" '
        FILENAME==keys{admitted[$0]=1;next}
        FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
        !/^#/&&!($1=="path"&&$2=="variant") {
            key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
            for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            changed=old[key]!=$0;if(a[7]=="pass"&&$7!="pass")regress++
            if(key in admitted){if(!changed)exit 4;changes++;if(a[7]!=$7)outcomes++;else details++}
            else if(changed)exit 5
            seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 6;printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changes,outcomes,details,length(old)-changes,regress}
    ' "$tmp/universe.keys" "$parent_full" "$candidate_full") || die 'full parent/candidate join drifted'
    [[ "$join_counts" == 'changed=306 outcome=292 detail=14 unchanged=101731 regressions=0' ]] \
        || die "full no-regression delta drifted: $join_counts"
    echo 'Weak collections global full gate passes: 102037 rows, 306 exact changes, no pass regression.'
    exit 0
fi

echo 'Weak collections global gate passes: QuickJS 306/306; Oxide 0/306 -> 292/306 with 14 reason-only variants.'
