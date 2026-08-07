#!/usr/bin/env bash
# Reproduce and authenticate the R3dy-A exact static-module graph milestone.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

baseline=tests/test262-module-loader-linker-a-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
manifest=tests/test262-module-loader-linker-a.txt
sources=tests/test262-module-loader-linker-a-sources.txt
negatives=tests/test262-module-loader-linker-a-negatives.txt
edges=tests/test262-module-loader-linker-a-edges.tsv
ledger=tests/test262-module-loader-linker-a-ledger.tsv
implementation_manifest=tests/test262-module-loader-linker-a-implementation.txt
candidate=tests/test262-module-loader-linker-a-candidate.tsv
parent=tests/test262-module-loader-linker-a-parent.tsv
unlisted_manifest=tests/test262-module-loader-linker-a-unlisted.txt
unlisted=tests/test262-module-loader-linker-a-unlisted.tsv
quickjs_projection=tests/test262-module-loader-linker-a-quickjs-projection.txt
profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
workers=${TEST262_WORKERS:-4}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

baseline_lines=82
baseline_sha256=b3632ff9ae52187862f91c40ab8682b28e6aa87efbfc25053bacd9901708ef03

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate the exact pinned graph without executing it\n'
    printf '  --full   rerun and authenticate two complete 102037-variant candidates\n'
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
    || { echo 'error: Test262 worker counts must be positive integers' >&2; exit 2; }
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
sha_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}
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
check_file() {
    [[ -f "$1" && ! -L "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated R3dy-A input drifted: $1"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
toml_value() {
    awk -v wanted_section="[$1]" -v wanted_key="$2" '
        $0==wanted_section{inside=1;next}
        /^\[/{inside=0}
        inside{
            split($0,pieces,"=");key=pieces[1]
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted_key)next
            value=substr($0,index($0,"=")+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
            if(value~/^".*"$/)value=substr(value,2,length(value)-2)
            print value;found++
        }
        END{if(found!=1)exit 1}
    ' "$upstream"
}
profile_section() {
    awk -v wanted="[$1]" '
        $0==wanted{inside=1;next}
        /^\[/{inside=0}
        inside&&NF&&$1!~/^#/{print}
    ' "$2"
}
strip_profile_section() {
    awk -v wanted="[$1]" '
        $0==wanted{print;inside=1;next}
        inside&&/^\[/{inside=0}
        !inside{print}
    ' "$2"
}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dy-a.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/git-home" "$tmp/git-config"
candidate_replay_a=$tmp/candidate-replay-a.tsv
candidate_replay_b=$tmp/candidate-replay-b.tsv
unlisted_replay=$tmp/unlisted-replay.tsv
quickjs_log_a=$tmp/quickjs-a.log
quickjs_log_b=$tmp/quickjs-b.log
if [[ -n ${TEST262_FULL_REPORT_DIR:-} ]]; then
    full_dir=$TEST262_FULL_REPORT_DIR
else
    full_dir=$tmp/full
fi
full_candidate_a=${TEST262_FULL_CANDIDATE_A_REPORT:-$full_dir/candidate-a.tsv}
full_candidate_b=${TEST262_FULL_CANDIDATE_B_REPORT:-$full_dir/candidate-b.tsv}
if [[ "$reuse_full_reports" == true \
    && -z ${TEST262_FULL_REPORT_DIR:-} \
    && -z ${TEST262_FULL_CANDIDATE_A_REPORT:-} \
    && -z ${TEST262_FULL_CANDIDATE_B_REPORT:-} ]]; then
    die 'TEST262_REUSE_FULL_REPORTS=true requires explicit full report paths'
fi

trusted_git() {
    env -i \
        "PATH=$PATH" \
        "HOME=$tmp/git-home" \
        "XDG_CONFIG_HOME=$tmp/git-config" \
        "TMPDIR=${TMPDIR:-/tmp}" \
        "LC_ALL=C" \
        "GIT_CONFIG_NOSYSTEM=1" \
        "GIT_NO_REPLACE_OBJECTS=1" \
        "GIT_NO_LAZY_FETCH=1" \
        "GIT_ATTR_NOSYSTEM=1" \
        "GIT_TERMINAL_PROMPT=0" \
        git --no-replace-objects \
        -c core.hooksPath=/dev/null \
        -c core.fsmonitor=false \
        -c core.attributesFile=/dev/null \
        "$@"
}

check_file "$baseline" "$baseline_lines" "$baseline_sha256"
[[ "$(value schema)" == r3dy-a-test262-module-loader-linker-v1 ]] \
    || die 'R3dy-A baseline schema drifted'
check_file "$canonical_baseline" 8 "$(value candidate_canonical_baseline_sha256)"
[[ "$(canonical_value schema)" == test262-canonical-classified-v2 \
    && "$(canonical_value variants)" == "$(value full_variants)" \
    && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
    && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
    && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
    || die 'canonical Test262 baseline is not the admitted R3dy-A candidate'
[[ "$(value full_candidate_replay_status)" == passed-twice \
    && "$(value full_candidate_replays)" == 2 ]] \
    || die 'R3dy-A full replay certificate drifted'

check_file "$manifest" "$(value manifest_lines)" "$(value manifest_sha256)"
check_file "$sources" "$(value sources_lines)" "$(value sources_sha256)"
check_file "$negatives" "$(value negative_lines)" "$(value negative_sha256)"
check_file "$edges" "$(value edges_lines)" "$(value edges_sha256)"
check_file "$ledger" "$(value ledger_lines)" "$(value ledger_sha256)"
check_file "$implementation_manifest" \
    "$(value candidate_implementation_manifest_lines)" \
    "$(value candidate_implementation_manifest_sha256)"
check_file "$parent" "$(value parent_tsv_lines)" "$(value parent_tsv_sha256)"
check_file "${parent%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
    "$(value parent_jsonl_sha256)"
check_file "$unlisted_manifest" "$(value unlisted_manifest_lines)" \
    "$(value unlisted_manifest_sha256)"
check_file "$unlisted" "$(value unlisted_tsv_lines)" "$(value unlisted_tsv_sha256)"
check_file "${unlisted%.tsv}.jsonl" "$(value unlisted_jsonl_lines)" \
    "$(value unlisted_jsonl_sha256)"
check_file "$quickjs_projection" "$(value quickjs_projection_lines)" \
    "$(value quickjs_projection_sha256)"

for sorted in "$manifest" "$sources" "$negatives"; do
    sort -c "$sorted" || die "R3dy-A manifest is not bytewise sorted: $sorted"
    [[ -z "$(uniq -d "$sorted")" ]] || die "R3dy-A manifest has duplicates: $sorted"
done
sort -c "$implementation_manifest" \
    || die 'R3dy-A implementation manifest is not bytewise sorted'
[[ -z "$(uniq -d "$implementation_manifest")" ]] \
    || die 'R3dy-A implementation manifest contains duplicates'
[[ -z "$(comm -23 "$manifest" "$sources")" \
    && -z "$(comm -23 "$negatives" "$manifest")" ]] \
    || die 'R3dy-A roots, sources, and negative subsets are inconsistent'
sed '1d' "$ledger" | cut -f1 | diff -u "$sources" -
awk -F'\t' '$3=="root"{print $1}' "$ledger" | diff -u "$manifest" -
awk -F'\t' 'NR>1{roles[$3]++} END{if(roles["root"]!=4||roles["fixture"]!=5)exit 1}' \
    "$ledger" || die 'R3dy-A ledger is not exactly four roots and five fixtures'
{
    cat "$manifest"
    sed '1d' "$edges" | cut -f5
} | sort -u >"$tmp/derived-sources"
diff -u "$sources" "$tmp/derived-sources"
awk -F'\t' 'NR==1{next}
    {
        key=$1 SUBSEP $2
        if($3!=seen[key]++)exit 1
        roots[$1]=1
    }
    END{for(root in roots)count++;if(count!=4||NR!=6)exit 1}
' "$edges" || die 'R3dy-A request indices or graph-root count drifted'

[[ "$(toml_value quickjs version)" == "$(value quickjs)" \
    && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
    && "$(toml_value test262 commit)" == "$(value test262)" \
    && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
    && "$(toml_value test262 oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
    || die 'compat/upstream.toml no longer names the R3dy-A pinned inputs'
[[ "$(sha "$profile")" == "$(value candidate_profile_sha256)" ]] \
    || die 'live Test262 capability profile drifted from R3dy-A'

parent_commit=$(value parent_commit)
trusted_git -C "$root" cat-file -e "$parent_commit^{commit}" 2>/dev/null \
    || die "R3dy-A parent commit is unavailable: $parent_commit"
[[ "$(trusted_git -C "$root" rev-parse "$parent_commit^{tree}")" == "$(value parent_tree)" ]] \
    || die 'R3dy-A parent tree drifted'
for rel in src/bin/run_test262.rs src/bin/run_test262/capabilities.rs \
    src/bin/run_test262/execution.rs src/bin/run_test262/requirements.rs \
    compat/test262-oxide.conf "$canonical_baseline"; do
    trusted_git -C "$root" show "$parent_commit:$rel" >"$tmp/parent-${rel//\//_}"
done
check_file "$tmp/parent-src_bin_run_test262.rs" \
    "$(value parent_runner_lines)" "$(value parent_runner_sha256)"
check_file "$tmp/parent-src_bin_run_test262_capabilities.rs" \
    "$(value parent_capabilities_lines)" "$(value parent_capabilities_sha256)"
check_file "$tmp/parent-src_bin_run_test262_execution.rs" \
    "$(value parent_execution_lines)" "$(value parent_execution_sha256)"
check_file "$tmp/parent-src_bin_run_test262_requirements.rs" \
    "$(value parent_requirements_lines)" "$(value parent_requirements_sha256)"
check_file "$tmp/parent-compat_test262-oxide.conf" \
    "$(value parent_profile_lines)" "$(value parent_profile_sha256)"
check_file "$tmp/parent-tests_test262-full-baseline.txt" 8 \
    "$(value parent_canonical_baseline_sha256)"
! grep -Fq 'FIXTURE_GRAPH_MODULE_ADMISSIONS' \
    "$tmp/parent-src_bin_run_test262_requirements.rs" \
    || die 'R3dy-A parent unexpectedly contains fixture-graph admission'
grep -Fq 'DEPENDENCY_FREE_MODULE_ADMISSIONS' \
    "$tmp/parent-src_bin_run_test262_requirements.rs" \
    || die 'R3dy-A parent no longer proves the R3dx exact module frontier'
grep -Fq "\"$(value candidate_profile_sha256)\"" src/bin/run_test262.rs \
    || die 'candidate runner does not pin the live R3dy-A profile hash'

while IFS= read -r rel; do
    [[ -f "$rel" && ! -L "$rel" ]] || die "R3dy-A implementation path is unsafe: $rel"
    printf '%s\t%s\t%s\n' "$rel" "$(lines "$rel")" "$(sha "$rel")"
done <"$implementation_manifest" >"$tmp/candidate-implementation.tsv"
current_implementation_sha=$(sha_stream <"$tmp/candidate-implementation.tsv")
implementation_frozen=false
case $(value candidate_implementation_status) in
    frozen)
        implementation_frozen=true
        [[ "$(value candidate_implementation_stream_sha256)" == "$current_implementation_sha" ]] \
            || die 'R3dy-A candidate implementation source inventory drifted'
        ;;
    pending-final-source-refresh)
        [[ "$(value candidate_implementation_stream_sha256)" == PENDING_FINAL_REFRESH ]] \
            || die 'R3dy-A pending implementation certificate is internally inconsistent'
        ;;
    *) die 'R3dy-A candidate implementation status drifted' ;;
esac

strip_profile_section audited-negative-tests "$tmp/parent-compat_test262-oxide.conf" \
    >"$tmp/parent-profile-without-negatives"
strip_profile_section audited-negative-tests "$profile" \
    >"$tmp/candidate-profile-without-negatives"
diff -u "$tmp/parent-profile-without-negatives" "$tmp/candidate-profile-without-negatives"
profile_section audited-negative-tests "$tmp/parent-compat_test262-oxide.conf" \
    | sort >"$tmp/parent-negatives"
profile_section audited-negative-tests "$profile" | sort >"$tmp/candidate-negatives"
comm -23 "$tmp/parent-negatives" "$tmp/candidate-negatives" >"$tmp/removed-negatives"
[[ ! -s "$tmp/removed-negatives" ]] || die 'R3dy-A removed a historical audited negative'
comm -13 "$tmp/parent-negatives" "$tmp/candidate-negatives" >"$tmp/added-negatives"
diff -u "$negatives" "$tmp/added-negatives"

verify_report_header() {
    local report=$1 expected_profile=$2 expected_rows=$3 expected_summary=$4
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$expected_profile" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == both \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$expected_rows" \
        && "$(report_summary "$report")" == "$expected_summary" ]] \
        || die "R3dy-A classified report contract drifted: $report"
}

verify_report_header "$parent" "$(value parent_profile_sha256)" 4 'unsupported-module=4'
verify_report_header "$unlisted" "$(value candidate_profile_sha256)" 1 'unsupported-module=1'
report_rows "$parent" | cut -f1 | diff -u "$manifest" -
awk -F'\t' '
    !/^#/&&!($1=="path"&&$2=="variant"){
        if($2!="sloppy"||$7!="unsupported-module"||$8!="selection"||
           $9!="ExecutionMode"||$10!="missing execution capabilities: module")exit 1
        count++
    }
    END{if(count!=4)exit 1}
' "$parent" || die 'R3dy-A parent receipt is not exactly four unsupported modules'
[[ "$(report_rows "$unlisted")" == $'test/language/module-code/instn-resolve-empty-export.js\tsloppy\tmodule\t\tresolution\tSyntaxError\tunsupported-module\tselection\tExecutionMode\tmissing execution capabilities: module' ]] \
    || die 'R3dy-A unlisted graph-root canary drifted'

candidate_frozen=true
if [[ "$(value candidate_tsv_sha256)" == PENDING_FINAL_REFRESH ]]; then
    candidate_frozen=false
else
    check_file "$candidate" "$(value candidate_tsv_lines)" "$(value candidate_tsv_sha256)"
    check_file "${candidate%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
        "$(value candidate_jsonl_sha256)"
    verify_report_header "$candidate" "$(value candidate_profile_sha256)" 4 'pass=4'
    report_rows "$candidate" | cut -f1 | diff -u "$manifest" -
    awk -F'\t' -v normal="$(value normal_tests)" \
        -v runtime="$(value runtime_type_error_tests)" \
        -v resolution="$(value resolution_syntax_error_tests)" '
        !/^#/&&!($1=="path"&&$2=="variant"){
            if($2!="sloppy"||$7!="pass")exit 1
            if($5=="normal"&&$6==""&&$8=="normal"&&$9=="")normal_count++
            else if($5=="runtime"&&$6=="TypeError"&&$8=="runtime"&&$9=="TypeError")runtime_count++
            else if($5=="resolution"&&$6=="SyntaxError"&&$8=="resolution"&&$9=="SyntaxError")resolution_count++
            else exit 1
        }
        END{if(normal_count!=normal||runtime_count!=runtime||resolution_count!=resolution)exit 1}
    ' "$candidate" || die 'R3dy-A candidate phase classification drifted'
    awk -F'\t' '
        NR==FNR{
            if(/^#/||($1=="path"&&$2=="variant"))next
            before[$1]=$0;before_count++;next
        }
        /^#/||($1=="path"&&$2=="variant"){next}
        {
            if(!($1 in before))exit 1
            split(before[$1],old,"\t")
            for(i=2;i<=6;i++)if(old[i]!=$i)exit 1
            if(old[7]!="unsupported-module"||$7!="pass")exit 1
            seen[$1]=1;after_count++
        }
        END{
            if(before_count!=4||after_count!=4)exit 1
            for(test in before)if(!(test in seen))exit 1
        }
    ' "$parent" "$candidate" || die 'R3dy-A transition is not exactly four unsupported-to-pass rows'
fi

if [[ -n ${TEST262_RUNNER:-} ]]; then
    runner=$TEST262_RUNNER
    [[ "$runner" == /* && -x "$runner" && ! -L "$runner" ]] \
        || die 'TEST262_RUNNER must name an absolute executable regular file'
else
    target_dir=${CARGO_TARGET_DIR:-$root/target}
    case $target_dir in
        /*) ;;
        *) target_dir=$root/$target_dir ;;
    esac
    build_host=$(rustc -vV | awk '$1=="host:"{print $2;found++} END{if(found!=1)exit 1}')
    cargo build --locked --release --target "$build_host" \
        --target-dir "$target_dir" --bin run-test262
    runner=$target_dir/$build_host/release/run-test262
    [[ -x "$runner" && ! -L "$runner" ]] || die 'release run-test262 binary is missing or unsafe'
fi

suite=$("$script_dir/prepare-test262.sh")
[[ -n "$suite" && "$suite" == /* && -d "$suite/test" && ! -L "$suite" ]] \
    || die 'prepare-test262.sh did not return one authenticated suite path'
source_dir=$(CDPATH='' cd -- "$suite/.." && pwd)
[[ "$(trusted_git -C "$suite" rev-parse HEAD)" == "$(value test262)" \
    && "$(sha "$source_dir/tests/test262.patch")" == "$(value test262_patch_sha256)" \
    && "$(sha "$source_dir/test262.conf")" == "$(value test262_config_sha256)" ]] \
    || die 'prepared Test262/QuickJS test inputs drifted'

metadata_records=$tmp/test262-metadata.bin
"$runner" --suite "$suite" --validate-metadata "$metadata_records" >"$tmp/metadata-audit.log"
check_file "$metadata_records" "$(value test262_metadata_records)" \
    "$(value test262_metadata_sha256)"
grep -Fqx "Test262 metadata: files=$(value test262_metadata_records)" \
    "$tmp/metadata-audit.log" || die 'Test262 metadata audit did not cover the pinned checkout'
tr '\000' '\t' <"$metadata_records" >"$tmp/test262-metadata.tsv"
awk -F'\t' '$3=="root"{print $1 "\t" $4 "\t" $5 "\t" $6 "\t" $7 "\t" $8}' \
    "$ledger" >"$tmp/expected-root-metadata.tsv"
awk -F'\t' 'NR==FNR{wanted[$1]=1;next} $1 in wanted{print}' \
    "$manifest" "$tmp/test262-metadata.tsv" >"$tmp/actual-root-metadata.tsv"
diff -u "$tmp/expected-root-metadata.tsv" "$tmp/actual-root-metadata.tsv"

while IFS= read -r record; do
    rel=$(printf '%s\n' "$record" | cut -f1)
    expected_source=$(printf '%s\n' "$record" | cut -f9)
    expected_frontmatter=$(printf '%s\n' "$record" | cut -f10)
    source_file=$suite/$rel
    [[ -f "$source_file" && ! -L "$source_file" ]] \
        || die "pinned R3dy-A source is missing or unsafe: $rel"
    [[ "$(sha "$source_file")" == "$expected_source" ]] \
        || die "pinned R3dy-A source drifted: $rel"
    awk '/\/\*---/{inside=1} inside{print} /---\*\//{if(inside)exit}' \
        "$source_file" >"$tmp/frontmatter"
    [[ "$(sha "$tmp/frontmatter")" == "$expected_frontmatter" ]] \
        || die "complete pinned R3dy-A frontmatter drifted: $rel"
done < <(sed '1d' "$ledger")

unlisted_path=$(sed -n '1p' "$unlisted_manifest")
[[ "$(sha "$suite/$unlisted_path")" == "$(value unlisted_source_sha256)" ]] \
    || die 'R3dy-A unlisted root source drifted'
awk '/\/\*---/{inside=1} inside{print} /---\*\//{if(inside)exit}' \
    "$suite/$unlisted_path" >"$tmp/unlisted-frontmatter"
[[ "$(sha "$tmp/unlisted-frontmatter")" == "$(value unlisted_frontmatter_sha256)" ]] \
    || die 'R3dy-A unlisted root frontmatter drifted'

if [[ "$mode" == check ]]; then
    if [[ "$candidate_frozen" == false ]]; then
        echo 'R3dy-A graph inputs authenticated: 4 roots, 9 sources, 5 ordered edges; focused receipt and final implementation hashes remain pending'
    elif [[ "$implementation_frozen" == false ]]; then
        printf 'R3dy-A graph and focused evidence authenticated; final implementation stream remains pending (current=%s)\n' \
            "$current_implementation_sha"
    else
        echo 'R3dy-A graph evidence authenticated: pinned inputs, predecessor, focused receipts, phases, and unlisted canary'
    fi
    exit 0
fi

[[ "$candidate_frozen" == true ]] \
    || die 'focused candidate receipt is PENDING_FINAL_REFRESH; regenerate and freeze it before running the focused gate'

run_focused_candidate() {
    local output=$1
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode both --workers "$workers" --timeout-ms "$(value timeout_ms)"
}
run_focused_candidate "$candidate_replay_a"
run_focused_candidate "$candidate_replay_b"
diff -u "$candidate" "$candidate_replay_a"
diff -u "${candidate%.tsv}.jsonl" "${candidate_replay_a%.tsv}.jsonl"
if ! cmp -s "$candidate_replay_a" "$candidate_replay_b" \
    || ! cmp -s "${candidate_replay_a%.tsv}.jsonl" "${candidate_replay_b%.tsv}.jsonl"; then
    die 'R3dy-A focused Oxide replays are not byte-identical'
fi

"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$unlisted_manifest" --report "$unlisted_replay" \
    --mode both --workers 1 --timeout-ms "$(value timeout_ms)" --allow-failures
diff -u "$unlisted" "$unlisted_replay"
diff -u "${unlisted%.tsv}.jsonl" "${unlisted_replay%.tsv}.jsonl"
worker_result=$("$runner" --worker-one --suite "$suite" \
    --test "$unlisted_path" --variant sloppy)
[[ "$worker_result" == $'runner-error\thost\t\tunsupported test reached worker' ]] \
    || die 'direct worker admitted the unlisted module-graph root'

drift_root=test/language/module-code/eval-gtbndng-indirect-update.js
drift_fixture=test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js
mkdir -p "$tmp/source-drift/$(dirname "$drift_root")"
cp "$suite/$drift_root" "$tmp/source-drift/$drift_root"
cp "$suite/$drift_fixture" "$tmp/source-drift/$drift_fixture"
printf '\n// R3dy-A nested source-drift canary.\n' >>"$tmp/source-drift/$drift_fixture"
worker_result=$("$runner" --worker-one --suite "$tmp/source-drift" \
    --test "$drift_root" --variant sloppy)
expected_prefix=$'runner-error\thost\t\tfixture graph module source drifted for test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js: expected SHA-256 86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b, found '
found_source_sha=${worker_result#"$expected_prefix"}
[[ "$worker_result" == "$expected_prefix"* \
    && "$found_source_sha" =~ ^[0-9a-f]{64}$ \
    && "$found_source_sha" != 86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b ]] \
    || die 'direct worker did not fail closed on nested fixture source drift'

quickjs_source=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
[[ "$quickjs_source" == "$source_dir" && -x "$quickjs_source/run-test262" ]] \
    || die 'authenticated QuickJS Test262 oracle path drifted'
quickjs_args=(-m -c test262.conf -a -T 1 -f)
while IFS= read -r rel; do
    quickjs_args+=("test262/$rel")
done <"$manifest"
for log in "$quickjs_log_a" "$quickjs_log_b"; do
    (cd "$quickjs_source" && ./run-test262 "${quickjs_args[@]}") >"$log" 2>&1 \
        || die "pinned QuickJS rejected the R3dy-A cohort: $log"
    ! grep -Fq 'FAILED' "$log" || die "pinned QuickJS reported a failed R3dy-A test: $log"
    [[ "$(grep -Fxc 'Average memory statistics for 4 tests:' "$log")" == 1 ]] \
        || die "pinned QuickJS did not execute exactly four R3dy-A tests: $log"
    awk '/^test262\.conf:.* ignoring testdir=/{print}
         /^TypeError$/{print}
         /^SyntaxError:/{print}
         /^Average memory statistics for [0-9]+ tests:/{print}' \
        "$log" >"$log.projection"
    diff -u "$quickjs_projection" "$log.projection"
done
cmp -s "$quickjs_log_a.projection" "$quickjs_log_b.projection" \
    || die 'pinned QuickJS semantic projections are not byte-identical'

if [[ "$mode" != full ]]; then
    if [[ "$implementation_frozen" == false ]]; then
        printf 'R3dy-A focused behavior passed twice: Oxide 4/4, QuickJS 4/4 twice, exact phases, unlisted and nested-drift fail-closed; final implementation stream remains pending (current=%s)\n' \
            "$current_implementation_sha"
    else
        echo 'R3dy-A module loader/linker focused gate passed: Oxide 4/4 twice and QuickJS 4/4 twice'
    fi
    exit 0
fi

run_full_candidate() {
    local output=$1 json=${1%.tsv}.jsonl
    mkdir -p "$(dirname "$output")"
    rm -f -- "$output" "$json"
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --all --report "$output" \
        --mode "$(value mode)" --workers "$full_workers" \
        --timeout-ms "$(value timeout_ms)" --allow-failures
}

verify_full_candidate() {
    local report=$1 label
    label=$(basename "${report%.tsv}")
    check_file "$report" "$(value full_report_lines)" \
        "$(value full_candidate_tsv_sha256)"
    check_file "${report%.tsv}.jsonl" "$(value full_jsonl_lines)" \
        "$(value full_candidate_jsonl_sha256)"
    report_keys "$report" >"$tmp/$label.keys"
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$tmp/$label.keys")" == "$(value full_variants)" \
        && "$(sha "$tmp/$label.keys")" == "$(value full_keys_sha256)" \
        && -z "$(uniq -d "$tmp/$label.keys")" \
        && "$(report_summary "$report")" == "$(value full_candidate_summary)" \
        && "$(report_runnable "$report")" == "$(value full_candidate_runnable)" \
        && "$(report_count pass "$report")" == "$(value full_candidate_passes)" ]] \
        || die "R3dy-A full candidate outcome drifted: $report"
}

if [[ "$reuse_full_reports" == false ]]; then
    full_receipt_kind=live-rerun
    run_full_candidate "$full_candidate_a"
    run_full_candidate "$full_candidate_b"
else
    full_receipt_kind=authenticated-reuse
fi
verify_full_candidate "$full_candidate_a"
verify_full_candidate "$full_candidate_b"
if [[ "$full_candidate_a" -ef "$full_candidate_b" \
    || "${full_candidate_a%.tsv}.jsonl" -ef "${full_candidate_b%.tsv}.jsonl" ]]; then
    die 'R3dy-A full candidate replays must be distinct files'
fi
if ! cmp -s "$full_candidate_a" "$full_candidate_b" \
    || ! cmp -s "${full_candidate_a%.tsv}.jsonl" "${full_candidate_b%.tsv}.jsonl"; then
    die 'R3dy-A full candidate replays are not byte-identical'
fi

derived_parent_tsv=$tmp/full-parent.tsv
derived_parent_json=$tmp/full-parent.jsonl
awk -F'\t' -v OFS='\t' -v candidate_profile="$(value candidate_profile_sha256)" \
    -v parent_profile="$(value parent_profile_sha256)" \
    -v summary="$(value full_parent_summary)" '
    NR==FNR{wanted[$1]=1;manifest_count++;next}
    /^# oxide_profile_sha256=/ {
        if(index($0,candidate_profile)==0)exit 2
        sub(candidate_profile,parent_profile);headers++;print;next
    }
    /^# summary / {print "# summary " summary;summaries++;next}
    !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted) {
        if($7!="pass")exit 3
        $7="unsupported-module";$8="selection";$9="ExecutionMode"
        $10="missing execution capabilities: module";changed++
    }
    {print}
    END{if(manifest_count!=4||headers!=1||summaries!=1||changed!=4)exit 4}
' "$manifest" "$full_candidate_a" >"$derived_parent_tsv" \
    || die 'R3dy-A could not reverse the full TSV candidate into R3dx'
awk -v candidate_profile="$(value candidate_profile_sha256)" \
    -v parent_profile="$(value parent_profile_sha256)" '
    NR==FNR{wanted[$1]=1;manifest_count++;next}
    /^\{"kind":"metadata",/ {
        if(index($0,candidate_profile)==0)exit 2
        sub(candidate_profile,parent_profile);headers++;print;next
    }
    /^\{"kind":"summary",/ {
        print "{\"kind\":\"summary\",\"outcomes\":{\"fail-parse\":7,\"fail-runtime\":43,\"pass\":68104,\"skipped-config-exclude\":6700,\"skipped-feature\":11775,\"timeout\":2,\"unsupported-feature\":11348,\"unsupported-module\":666,\"unsupported-negative-provenance\":3392}}"
        summaries++;next
    }
    {
        hit=0
        for(path in wanted)if(index($0,"\"path\":\"" path "\"")>0){hit=1;break}
        if(hit){
            if(!sub(/"outcome":"pass","actual_phase":"[^"]*","actual_type":"[^"]*","detail":"[^"]*"/,"\"outcome\":\"unsupported-module\",\"actual_phase\":\"selection\",\"actual_type\":\"ExecutionMode\",\"detail\":\"missing execution capabilities: module\""))exit 3
            changed++
        }
        print
    }
    END{if(manifest_count!=4||headers!=1||summaries!=1||changed!=4)exit 4}
' "$manifest" "${full_candidate_a%.tsv}.jsonl" >"$derived_parent_json" \
    || die 'R3dy-A could not reverse the full JSONL candidate into R3dx'
check_file "$derived_parent_tsv" "$(value full_report_lines)" \
    "$(value full_parent_tsv_sha256)"
check_file "$derived_parent_json" "$(value full_jsonl_lines)" \
    "$(value full_parent_jsonl_sha256)"

awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
    !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted){print}' \
    "$manifest" "$full_candidate_a" >"$tmp/full-scope.rows"
diff -u <(report_rows "$candidate") "$tmp/full-scope.rows"
[[ "$(lines "$tmp/full-scope.rows")" == "$(value full_scope_variants)" \
    && "$(( $(value full_variants) - $(value full_scope_variants) ))" == "$(value full_outside_variants)" ]] \
    || die 'R3dy-A full scope certificate drifted'

printf 'R3dy-A full A/B admission gate passed (%s): candidate=%s json=%s; reverse-derived predecessor matches R3dx canonical hashes.\n' \
    "$full_receipt_kind" "$(sha "$full_candidate_a")" \
    "$(sha "${full_candidate_a%.tsv}.jsonl")"
