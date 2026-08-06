#!/usr/bin/env bash
# Reproduce and authenticate the R3dx dependency-free static-module milestone.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

baseline=tests/test262-module-static-core-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
manifest=tests/test262-module-static-core.txt
negatives=tests/test262-module-static-core-negatives.txt
ledger=tests/test262-module-static-core-ledger.tsv
candidate=tests/test262-module-static-core-candidate.tsv
parent=tests/test262-module-static-core-parent.tsv
unlisted_manifest=tests/test262-module-static-core-unlisted.txt
unlisted=tests/test262-module-static-core-unlisted.tsv
quickjs_projection=tests/test262-module-static-core-quickjs-projection.txt
profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
workers=${TEST262_WORKERS:-4}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

baseline_lines=71
baseline_sha256=26713b79e1430244fc42479a2259d362e47e8fdb87a2e04eaf030b1dc45c77cb

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate frozen inputs and the pinned Test262 checkout\n'
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
        || die "authenticated R3dx input drifted: $1"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
report_keys() {
    report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
sha_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
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

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dx.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/git-home" "$tmp/git-config"
candidate_replay=$tmp/candidate-replay.tsv
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
[[ "$(value canonical_full_baseline)" == "$canonical_baseline" ]] \
    || die 'R3dx canonical baseline path drifted'
check_file "$canonical_baseline" 8 "$(value candidate_canonical_baseline_sha256)"
[[ "$(canonical_value schema)" == test262-canonical-classified-v2 \
    && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
    && "$(canonical_value variants)" == "$(value full_variants)" \
    && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
    && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
    && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
    || die 'canonical Test262 baseline is not the admitted R3dx candidate'
check_file "$manifest" "$(value manifest_lines)" "$(value manifest_sha256)"
check_file "$negatives" "$(value negative_lines)" "$(value negative_sha256)"
check_file "$ledger" "$(value ledger_lines)" "$(value ledger_sha256)"
check_file "$candidate" "$(value candidate_tsv_lines)" "$(value candidate_tsv_sha256)"
check_file "${candidate%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
    "$(value candidate_jsonl_sha256)"
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

sort -c "$manifest" || die 'R3dx manifest is not bytewise sorted'
[[ -z "$(uniq -d "$manifest")" ]] || die 'R3dx manifest contains duplicate paths'
sort -c "$negatives" || die 'R3dx negative manifest is not bytewise sorted'
[[ -z "$(comm -23 "$negatives" "$manifest")" ]] \
    || die 'R3dx negative manifest escapes the 13-path cohort'
[[ "$(sed '1d' "$ledger" | cut -f1 | diff -u "$manifest" - >/dev/null; echo $?)" == 0 ]] \
    || die 'R3dx ledger paths do not exactly equal the manifest'

[[ "$(toml_value quickjs version)" == "$(value quickjs)" \
    && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
    && "$(toml_value test262 commit)" == "$(value test262)" \
    && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
    && "$(toml_value test262 oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
    || die 'compat/upstream.toml no longer names the R3dx pinned inputs'
[[ "$(sha "$profile")" == "$(value candidate_profile_sha256)" ]] \
    || die 'live Test262 capability profile drifted from R3dx'

parent_commit=$(value parent_commit)
trusted_git -C "$root" cat-file -e "$parent_commit^{commit}" 2>/dev/null \
    || die "R3dx parent commit is unavailable: $parent_commit"
[[ "$(trusted_git -C "$root" rev-parse "$parent_commit^{tree}")" == "$(value parent_tree)" ]] \
    || die 'R3dx parent tree drifted'
for rel in src/bin/run_test262.rs src/bin/run_test262/execution.rs \
    src/bin/run_test262/requirements.rs compat/test262-oxide.conf \
    "$canonical_baseline"; do
    trusted_git -C "$root" show "$parent_commit:$rel" >"$tmp/parent-${rel//\//_}"
done
check_file "$tmp/parent-src_bin_run_test262.rs" \
    "$(lines "$tmp/parent-src_bin_run_test262.rs")" "$(value parent_runner_sha256)"
check_file "$tmp/parent-src_bin_run_test262_execution.rs" \
    "$(lines "$tmp/parent-src_bin_run_test262_execution.rs")" "$(value parent_execution_sha256)"
check_file "$tmp/parent-src_bin_run_test262_requirements.rs" \
    "$(lines "$tmp/parent-src_bin_run_test262_requirements.rs")" "$(value parent_requirements_sha256)"
check_file "$tmp/parent-compat_test262-oxide.conf" \
    "$(lines "$tmp/parent-compat_test262-oxide.conf")" "$(value parent_profile_sha256)"
check_file "$tmp/parent-tests_test262-full-baseline.txt" 8 \
    "$(value parent_canonical_baseline_sha256)"
grep -Fq 'if metadata.is_module() || (metadata.is_async() && !options.allow_async_host) {' \
    "$tmp/parent-src_bin_run_test262_execution.rs" \
    || die 'R3dx parent no longer proves the historical all-module worker rejection'
! grep -Fq 'is_exact_dependency_free_module_test' \
    "$tmp/parent-src_bin_run_test262_requirements.rs" \
    || die 'R3dx parent unexpectedly contains the module admission table'

strip_profile_section audited-negative-tests "$tmp/parent-compat_test262-oxide.conf" \
    >"$tmp/parent-profile-without-negatives"
strip_profile_section audited-negative-tests "$profile" \
    >"$tmp/candidate-profile-without-negatives"
diff -u "$tmp/parent-profile-without-negatives" "$tmp/candidate-profile-without-negatives"
profile_section audited-negative-tests "$tmp/parent-compat_test262-oxide.conf" \
    | sort >"$tmp/parent-negatives"
profile_section audited-negative-tests "$profile" | sort >"$tmp/candidate-negatives"
comm -23 "$tmp/parent-negatives" "$tmp/candidate-negatives" >"$tmp/removed-negatives"
[[ ! -s "$tmp/removed-negatives" ]] || die 'R3dx removed a historical audited negative'
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
        || die "R3dx classified report contract drifted: $report"
}

verify_report_header "$candidate" "$(value candidate_profile_sha256)" 13 'pass=13'
verify_report_header "$parent" "$(value parent_profile_sha256)" 13 'unsupported-module=13'
verify_report_header "$unlisted" "$(value candidate_profile_sha256)" 1 'unsupported-module=1'

report_rows "$candidate" | cut -f1 >"$tmp/candidate-paths"
report_rows "$parent" | cut -f1 >"$tmp/parent-paths"
diff -u "$manifest" "$tmp/candidate-paths"
diff -u "$manifest" "$tmp/parent-paths"
awk -F'\t' -v normal="$(value normal_tests)" \
    -v parse="$(value parse_syntax_error_tests)" \
    -v runtime="$(value runtime_test262_error_tests)" '
    !/^#/&&!($1=="path"&&$2=="variant"){
        if($2!="sloppy"||$7!="pass")exit 1
        if($5=="normal"&&$6==""&&$8=="normal"&&$9=="")normal_count++
        else if($5=="parse"&&$6=="SyntaxError"&&$8=="parse"&&$9=="SyntaxError")parse_count++
        else if($5=="runtime"&&$6=="Test262Error"&&$8=="runtime"&&$9=="Test262Error")runtime_count++
        else exit 1
    }
    END{if(normal_count!=normal||parse_count!=parse||runtime_count!=runtime)exit 1}
' "$candidate" || die 'R3dx candidate is not exactly 9 normal, 3 parse, and 1 runtime pass'
awk -F'\t' '
    !/^#/&&!($1=="path"&&$2=="variant"){
        if($2!="sloppy"||$7!="unsupported-module"||$8!="selection"||
           $9!="ExecutionMode"||$10!="missing execution capabilities: module")exit 1
        count++
    }
    END{if(count!=13)exit 1}
' "$parent" || die 'R3dx parent receipt is not exactly 13 unsupported modules'
[[ "$(report_rows "$unlisted")" == $'test/language/eval-code/indirect/export.js\tsloppy\tmodule\t\tnormal\t\tunsupported-module\tselection\tExecutionMode\tmissing execution capabilities: module' ]] \
    || die 'R3dx unlisted coordinator canary drifted'
[[ "$(tail -n 1 "${candidate%.tsv}.jsonl")" == '{"kind":"summary","outcomes":{"pass":13}}' \
    && "$(tail -n 1 "${parent%.tsv}.jsonl")" == '{"kind":"summary","outcomes":{"unsupported-module":13}}' \
    && "$(tail -n 1 "${unlisted%.tsv}.jsonl")" == '{"kind":"summary","outcomes":{"unsupported-module":1}}' ]] \
    || die 'R3dx JSONL receipt summaries drifted'

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
        if(old[7]!="unsupported-module"||old[8]!="selection"||old[9]!="ExecutionMode"||
           old[10]!="missing execution capabilities: module"||$7!="pass")exit 1
        seen[$1]=1;after_count++
    }
    END{
        if(before_count!=13||after_count!=13)exit 1
        for(test in before)if(!(test in seen))exit 1
    }
' "$parent" "$candidate" || die 'R3dx parent-to-candidate transition is not exactly 13 unsupported-to-pass rows'

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

suite=$("$script_dir/prepare-test262.sh")
[[ -n "$suite" && "$suite" == /* && -d "$suite/test" && ! -L "$suite" ]] \
    || die 'prepare-test262.sh did not return one authenticated suite path'
[[ "$(trusted_git -C "$suite" rev-parse HEAD)" == "$(value test262)" ]] \
    || die 'prepared Test262 checkout is not at the pinned commit'
source_dir=$(CDPATH='' cd -- "$suite/.." && pwd)
[[ "$(sha "$source_dir/tests/test262.patch")" == "$(value test262_patch_sha256)" \
    && "$(sha "$source_dir/test262.conf")" == "$(value test262_config_sha256)" ]] \
    || die 'prepared QuickJS Test262 patch or config drifted'

metadata_records="$tmp/test262-metadata.bin"
"$runner" --suite "$suite" --validate-metadata "$metadata_records" >"$tmp/metadata-audit.log"
check_file "$metadata_records" "$(value test262_metadata_records)" \
    "$(value test262_metadata_sha256)"
grep -Fqx "Test262 metadata: files=$(value test262_metadata_records)" "$tmp/metadata-audit.log" \
    || die 'Test262 metadata audit did not cover the full pinned checkout'
tr '\000' '\t' <"$metadata_records" >"$tmp/test262-metadata.tsv"
sed '1d' "$ledger" | cut -f1-6 >"$tmp/expected-metadata.tsv"
awk -F'\t' 'NR==FNR{wanted[$1]=1;next} $1 in wanted{print}' \
    "$manifest" "$tmp/test262-metadata.tsv" >"$tmp/actual-metadata.tsv"
diff -u "$tmp/expected-metadata.tsv" "$tmp/actual-metadata.tsv"

while IFS= read -r record; do
    rel=$(printf '%s\n' "$record" | cut -f1)
    expected_source=$(printf '%s\n' "$record" | cut -f7)
    expected_frontmatter=$(printf '%s\n' "$record" | cut -f8)
    source_file=$suite/$rel
    [[ -f "$source_file" && ! -L "$source_file" ]] \
        || die "pinned R3dx source is missing or unsafe: $rel"
    [[ "$(sha "$source_file")" == "$expected_source" ]] \
        || die "pinned R3dx source drifted: $rel"
    awk '/\/\*---/{inside=1} inside{print} /---\*\//{if(inside)exit}' \
        "$source_file" >"$tmp/frontmatter"
    [[ "$(sha "$tmp/frontmatter")" == "$expected_frontmatter" ]] \
        || die "complete pinned R3dx frontmatter drifted: $rel"
done < <(sed '1d' "$ledger")

unlisted_path=$(sed -n '1p' "$unlisted_manifest")
unlisted_source=$suite/$unlisted_path
[[ "$(sha "$unlisted_source")" == "$(value unlisted_source_sha256)" ]] \
    || die 'R3dx unlisted module canary source drifted'
awk '/\/\*---/{inside=1} inside{print} /---\*\//{if(inside)exit}' \
    "$unlisted_source" >"$tmp/unlisted-frontmatter"
[[ "$(sha "$tmp/unlisted-frontmatter")" == "$(value unlisted_frontmatter_sha256)" ]] \
    || die 'R3dx unlisted module canary frontmatter drifted'
awk -F'\t' -v wanted="$unlisted_path" '$1==wanted{print;found++} END{if(found!=1)exit 1}' \
    "$tmp/test262-metadata.tsv" >"$tmp/unlisted-metadata"
[[ "$(cat "$tmp/unlisted-metadata")" == $'test/language/eval-code/indirect/export.js\t\tmodule\t\t\t' ]] \
    || die 'R3dx unlisted module canary metadata drifted'

if [[ "$mode" == check ]]; then
    echo 'R3dx module static-core evidence authenticated: 13 sources/frontmatters/full metadata, parent transition, and unlisted canary'
    exit 0
fi

"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$manifest" --report "$candidate_replay" \
    --mode both --workers "$workers" --timeout-ms 30000
diff -u "$candidate" "$candidate_replay"
diff -u "${candidate%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl"

"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$unlisted_manifest" --report "$unlisted_replay" \
    --mode both --workers 1 --timeout-ms 30000 --allow-failures
diff -u "$unlisted" "$unlisted_replay"
diff -u "${unlisted%.tsv}.jsonl" "${unlisted_replay%.tsv}.jsonl"

worker_result=$("$runner" --worker-one --suite "$suite" \
    --test "$unlisted_path" --variant sloppy)
[[ "$worker_result" == $'runner-error\thost\t\tunsupported test reached worker' ]] \
    || die 'direct worker admitted the unlisted dependency-free module canary'

admitted_path=$(sed -n '1p' "$manifest")
mkdir -p "$tmp/source-drift/$(dirname "$admitted_path")"
cp "$suite/$admitted_path" "$tmp/source-drift/$admitted_path"
printf '\n// R3dx source-drift canary.\n' >>"$tmp/source-drift/$admitted_path"
worker_result=$("$runner" --worker-one --suite "$tmp/source-drift" \
    --test "$admitted_path" --variant sloppy)
expected_prefix=$'runner-error\thost\t\tdependency-free module source drifted for test/language/comments/hashbang/module.js: expected SHA-256 5fe73a40369e7cbd61f4061b027c9b508d6f1752fc83b29a4f1e4af7e8471926, found '
found_source_sha=${worker_result#"$expected_prefix"}
[[ "$worker_result" == "$expected_prefix"* \
    && "$found_source_sha" =~ ^[0-9a-f]{64}$ \
    && "$found_source_sha" != 5fe73a40369e7cbd61f4061b027c9b508d6f1752fc83b29a4f1e4af7e8471926 ]] \
    || die 'direct worker did not fail closed on admitted-module source drift'

mkdir -p "$tmp/metadata-drift/$(dirname "$admitted_path")" "$tmp/fake-bin"
sed 's/flags: \[module, raw\]/flags: [module]/' "$suite/$admitted_path" \
    >"$tmp/metadata-drift/$admitted_path"
printf '%s\n' \
    '#!/bin/sh' \
    'dd of=/dev/null 2>/dev/null' \
    "printf '%s  -\\n' 5fe73a40369e7cbd61f4061b027c9b508d6f1752fc83b29a4f1e4af7e8471926" \
    >"$tmp/fake-bin/sha256sum"
chmod +x "$tmp/fake-bin/sha256sum"
worker_result=$(PATH="$tmp/fake-bin:$PATH" "$runner" --worker-one \
    --suite "$tmp/metadata-drift" --test "$admitted_path" --variant sloppy)
[[ "$worker_result" == $'runner-error\thost\t\tdependency-free module metadata shape drifted for test/language/comments/hashbang/module.js' ]] \
    || die 'direct worker did not fail closed on admitted-module metadata drift'

quickjs_source=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
[[ "$quickjs_source" == "$source_dir" && -x "$quickjs_source/run-test262" ]] \
    || die 'authenticated QuickJS Test262 oracle path drifted'
# Pinned QuickJS documents that worker output may be reordered. Keep this
# small diagnostic cohort single-threaded so its semantic projection is
# byte-stable on every supported host.
quickjs_args=(-m -c test262.conf -a -T 1 -f)
while IFS= read -r rel; do
    quickjs_args+=("test262/$rel")
done <"$manifest"
for log in "$quickjs_log_a" "$quickjs_log_b"; do
    (cd "$quickjs_source" && ./run-test262 "${quickjs_args[@]}") >"$log" 2>&1 \
        || die "pinned QuickJS rejected the R3dx cohort: $log"
    ! grep -Fq 'FAILED' "$log" || die "pinned QuickJS reported a failed R3dx test: $log"
    [[ "$(grep -Fxc 'Average memory statistics for 13 tests:' "$log")" == 1 ]] \
        || die "pinned QuickJS did not execute exactly 13 R3dx tests: $log"
    awk '/^test262\.conf:.* ignoring testdir=/{print}
         /^SyntaxError:/{print}
         /^Throw:/{print}
         /^Average memory statistics for [0-9]+ tests:/{print}' \
        "$log" >"$log.projection"
    diff -u "$quickjs_projection" "$log.projection"
done
diff -u "$quickjs_log_a.projection" "$quickjs_log_b.projection"

if [[ "$mode" != full ]]; then
    echo 'R3dx module static-core focused gate passed: Oxide 13/13, QuickJS 13/13 twice, parent 13 unsupported, and all canaries rejected'
    exit 0
fi

rows_for_manifest() {
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted){print}' "$1" "$2"
}

rows_without_manifest() {
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&!($1 in wanted){print}' "$1" "$2"
}

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
    local report=$1 json=${1%.tsv}.jsonl label
    label=$(basename "${report%.tsv}")
    check_file "$report" "$(value full_report_lines)" \
        "$(value full_candidate_tsv_sha256)"
    check_file "$json" "$(value full_jsonl_lines)" \
        "$(value full_candidate_jsonl_sha256)"
    report_keys "$report" >"$tmp/$label.keys"
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$tmp/$label.keys")" == "$(value full_variants)" \
        && "$(sha "$tmp/$label.keys")" == "$(value full_keys_sha256)" \
        && -z "$(uniq -d "$tmp/$label.keys")" \
        && "$(report_summary "$report")" == "$(value full_candidate_summary)" \
        && "$(report_runnable "$report")" == "$(value full_candidate_runnable)" \
        && "$(report_count pass "$report")" == "$(value full_candidate_passes)" ]] \
        || die "R3dx full candidate drifted: $report"
}

derive_historical_full_parent() {
    local full_candidate=$1 derived_tsv=$2 derived_json=$3
    local candidate_profile parent_profile
    candidate_profile=$(value candidate_profile_sha256)
    parent_profile=$(value parent_profile_sha256)

    awk -F'\t' -v OFS='\t' -v candidate_profile="$candidate_profile" \
        -v parent_profile="$parent_profile" -v summary="$(value full_parent_summary)" '
        NR==FNR{wanted[$1]=1;manifest_count++;next}
        /^# oxide_profile_sha256=/ {
            if(index($0,candidate_profile)==0)exit 2
            sub(candidate_profile,parent_profile);profile_headers++;print;next
        }
        /^# summary / {print "# summary " summary;summaries++;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted) {
            if($7!="pass")exit 3
            $7="unsupported-module";$8="selection";$9="ExecutionMode"
            $10="missing execution capabilities: module";changed++
        }
        {print}
        END{if(manifest_count!=13||profile_headers!=1||summaries!=1||changed!=13)exit 4}
    ' "$manifest" "$full_candidate" >"$derived_tsv" \
        || die 'R3dx could not reverse the full TSV candidate into its parent'

    awk -v candidate_profile="$candidate_profile" -v parent_profile="$parent_profile" '
        NR==FNR{wanted[$1]=1;manifest_count++;next}
        /^\{"kind":"metadata",/ {
            if(index($0,candidate_profile)==0)exit 2
            sub(candidate_profile,parent_profile);profile_headers++;print;next
        }
        /^\{"kind":"summary",/ {
            print "{\"kind\":\"summary\",\"outcomes\":{\"fail-parse\":7,\"fail-runtime\":43,\"pass\":68091,\"skipped-config-exclude\":6700,\"skipped-feature\":11775,\"timeout\":2,\"unsupported-feature\":11348,\"unsupported-module\":679,\"unsupported-negative-provenance\":3392}}"
            summaries++;next
        }
        {
            hit=0
            for(path in wanted) {
                if(index($0,"\"path\":\"" path "\"")>0){hit=1;break}
            }
            if(hit) {
                if(!sub(/\"outcome\":\"pass\",\"actual_phase\":\"[^\"]*\",\"actual_type\":\"[^\"]*\",\"detail\":\"[^\"]*\"/,"\"outcome\":\"unsupported-module\",\"actual_phase\":\"selection\",\"actual_type\":\"ExecutionMode\",\"detail\":\"missing execution capabilities: module\""))exit 3
                changed++
            }
            print
        }
        END{if(manifest_count!=13||profile_headers!=1||summaries!=1||changed!=13)exit 4}
    ' "$manifest" "${full_candidate%.tsv}.jsonl" >"$derived_json" \
        || die 'R3dx could not reverse the full JSONL candidate into its parent'
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
    die 'R3dx full candidate replays must be distinct files'
fi
if ! cmp -s "$full_candidate_a" "$full_candidate_b" \
    || ! cmp -s "${full_candidate_a%.tsv}.jsonl" "${full_candidate_b%.tsv}.jsonl"; then
    die 'R3dx full candidate replays are not byte-identical'
fi
[[ "$(value full_candidate_replay_status)" == passed-twice \
    && "$(value full_candidate_replays)" == 2 ]] \
    || die 'R3dx full replay certificate drifted'

derived_parent_tsv=$tmp/full-parent.tsv
derived_parent_json=$tmp/full-parent.jsonl
derive_historical_full_parent "$full_candidate_a" \
    "$derived_parent_tsv" "$derived_parent_json"
check_file "$derived_parent_tsv" "$(value full_report_lines)" \
    "$(value full_parent_tsv_sha256)"
check_file "$derived_parent_json" "$(value full_jsonl_lines)" \
    "$(value full_parent_jsonl_sha256)"
[[ "$(report_runnable "$derived_parent_tsv")" == "$(value full_parent_runnable)" \
    && "$(report_count pass "$derived_parent_tsv")" == "$(value full_parent_passes)" \
    && "$(report_summary "$derived_parent_tsv")" == "$(value full_parent_summary)" ]] \
    || die 'R3dx reverse-derived historical full summary drifted'

rows_for_manifest "$manifest" "$full_candidate_a" >"$tmp/full-candidate-scope.rows"
rows_for_manifest "$manifest" "$derived_parent_tsv" >"$tmp/full-parent-scope.rows"
rows_without_manifest "$manifest" "$full_candidate_a" >"$tmp/full-candidate-outside.rows"
rows_without_manifest "$manifest" "$derived_parent_tsv" >"$tmp/full-parent-outside.rows"
diff -u <(report_rows "$candidate") "$tmp/full-candidate-scope.rows"
diff -u <(report_rows "$parent") "$tmp/full-parent-scope.rows"
diff -u "$tmp/full-parent-outside.rows" "$tmp/full-candidate-outside.rows"
[[ "$(lines "$tmp/full-candidate-scope.rows")" == "$(value full_scope_variants)" \
    && "$(lines "$tmp/full-candidate-outside.rows")" == "$(value full_outside_variants)" \
    && "$(value full_changed)" == 13 \
    && "$(value full_outcome_changed)" == 13 \
    && "$(value full_detail_only)" == 0 \
    && "$(value full_unchanged)" == "$(value full_outside_variants)" \
    && "$(value full_pass_gains)" == 13 \
    && "$(value full_pass_regressions)" == 0 ]] \
    || die 'R3dx full exact-join certificate drifted'

printf 'R3dx module static-core full gate passed (%s): candidate=%s json=%s; reverse-derived parent matches R3dw canonical hashes\n' \
    "$full_receipt_kind" "$(sha "$full_candidate_a")" \
    "$(sha "${full_candidate_a%.tsv}.jsonl")"
