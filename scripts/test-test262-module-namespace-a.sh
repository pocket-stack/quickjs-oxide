#!/usr/bin/env bash
# Reproduce and authenticate the R3dz-A natural module-namespace cohort.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

baseline=tests/test262-module-namespace-a-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
manifest=tests/test262-module-namespace-a.txt
sources=tests/test262-module-namespace-a-sources.txt
edges=tests/test262-module-namespace-a-edges.tsv
ledger=tests/test262-module-namespace-a-ledger.tsv
closures=tests/test262-module-namespace-a-closures.tsv
implementation_manifest=tests/test262-module-namespace-a-implementation.txt
candidate=tests/test262-module-namespace-a-candidate.tsv
parent=tests/test262-module-namespace-a-parent.tsv
unlisted_manifest=tests/test262-module-namespace-a-unlisted.txt
unlisted=tests/test262-module-namespace-a-unlisted.tsv
quickjs_projection=tests/test262-module-namespace-a-quickjs-projection.txt
profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
workers=${TEST262_WORKERS:-4}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

baseline_lines=118
baseline_sha256=df55c74f91c7ece8fe92769d88fa1a75108a4e52cf3ede7d1f9233dce9bc979c

usage() {
    printf 'usage: %s [--check|--focused|--full]\n' "${0##*/}"
    printf '  --check    authenticate frozen inputs without running the cohort\n'
    printf '  --focused  rerun the focused Oxide and QuickJS receipts (default)\n'
    printf '  --full     additionally replay and exact-join two whole-suite reports\n'
}

mode=focused
case ${1-} in
    '') ;;
    --check) mode=check ;;
    --focused) mode=focused ;;
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
sha_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
value_from() {
    local file=$1 wanted=$2
    awk -F= -v wanted="$wanted" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$file"
}
value() { value_from "$baseline" "$1"; }
canonical_value() { value_from "$canonical_baseline" "$1"; }
check_file() {
    [[ -f "$1" && ! -L "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated R3dz-A input drifted: $1"
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

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dz-a.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
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
[[ "$(value schema)" == r3dz-a-test262-module-namespace-v1 ]] \
    || die 'R3dz-A baseline schema drifted'
check_file "$(value predecessor_baseline)" \
    "$(value predecessor_baseline_lines)" \
    "$(value predecessor_baseline_sha256)"
[[ "$(value canonical_full_baseline)" == "$canonical_baseline" ]] \
    || die 'R3dz-A canonical baseline path drifted'
check_file "$canonical_baseline" 8 "$(value candidate_canonical_baseline_sha256)"
[[ "$(canonical_value schema)" == test262-canonical-classified-v2 \
    && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
    && "$(canonical_value variants)" == "$(value full_variants)" \
    && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
    && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
    && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
    || die 'canonical Test262 baseline is not the admitted R3dz-A candidate'
[[ "$(value candidate_replay_status)" == passed-twice \
    && "$(value candidate_replays)" == 2 \
    && "$(value quickjs_replay_status)" == passed-twice \
    && "$(value quickjs_replays)" == 2 \
    && "$(value full_candidate_replay_status)" == passed-twice \
    && "$(value full_candidate_replays)" == 2 ]] \
    || die 'R3dz-A replay certificate drifted'

check_file "$manifest" "$(value manifest_lines)" "$(value manifest_sha256)"
check_file "$sources" "$(value sources_lines)" "$(value sources_sha256)"
check_file "$edges" "$(value edges_lines)" "$(value edges_sha256)"
check_file "$ledger" "$(value ledger_lines)" "$(value ledger_sha256)"
check_file "$closures" "$(value closures_lines)" "$(value closures_sha256)"
check_file "$implementation_manifest" \
    "$(value candidate_implementation_manifest_lines)" \
    "$(value candidate_implementation_manifest_sha256)"
check_file "$parent" "$(value parent_tsv_lines)" "$(value parent_tsv_sha256)"
check_file "${parent%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
    "$(value parent_jsonl_sha256)"
check_file "$candidate" "$(value candidate_tsv_lines)" "$(value candidate_tsv_sha256)"
check_file "${candidate%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
    "$(value candidate_jsonl_sha256)"
check_file "$unlisted_manifest" "$(value unlisted_manifest_lines)" \
    "$(value unlisted_manifest_sha256)"
check_file "$unlisted" "$(value unlisted_tsv_lines)" "$(value unlisted_tsv_sha256)"
check_file "${unlisted%.tsv}.jsonl" "$(value unlisted_jsonl_lines)" \
    "$(value unlisted_jsonl_sha256)"
check_file "$quickjs_projection" "$(value quickjs_projection_lines)" \
    "$(value quickjs_projection_sha256)"

for sorted in "$manifest" "$sources" "$implementation_manifest"; do
    sort -c "$sorted" || die "R3dz-A input is not bytewise sorted: $sorted"
    [[ -z "$(uniq -d "$sorted")" ]] || die "R3dz-A input has duplicates: $sorted"
done
[[ "$(head -n 1 "$edges")" == $'root_path\tbase_path\trequest_index\tspecifier\tnormalized_path' ]] \
    || die 'R3dz-A edge ledger header drifted'
[[ "$(head -n 1 "$ledger")" == $'path\troot_path\trole\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256' ]] \
    || die 'R3dz-A source ledger header drifted'
[[ "$(head -n 1 "$closures")" == $'root_path\tclosure_files\trequest_edges' ]] \
    || die 'R3dz-A closure ledger header drifted'

sed '1d' "$ledger" | cut -f1 | diff -u "$sources" -
awk -F'\t' '$3=="root"{print $1}' "$ledger" | diff -u "$manifest" -
awk -F'\t' 'NR>1{print $2}' "$ledger" | sort -u >"$tmp/ledger-owners"
diff -u "$manifest" "$tmp/ledger-owners"
awk -F'\t' -v roots="$(value roots)" -v fixtures="$(value fixtures)" '
    NR>1{roles[$3]++;records++}
    END{
        if(records!=roots+fixtures||roles["root"]!=roots||
           roles["fixture"]!=fixtures)exit 1
    }
' "$ledger" || die 'R3dz-A ledger role counts drifted'
awk -F'\t' -v roots="$(value roots)" -v requests="$(value request_edges)" \
    -v self="$(value self_edges)" '
    NR==1{next}
    {
        key=$1 SUBSEP $2
        if($3!=seen[key]++)exit 1
        roots_seen[$1]=1
        if($2==$5)self_seen++
        if($4!~/^\.\//)exit 1
    }
    END{
        for(root in roots_seen)root_count++
        if(root_count!=roots||NR-1!=requests||self_seen!=self)exit 1
    }
' "$edges" || die 'R3dz-A edge indices, roots, or self-edge counts drifted'
awk -F'\t' -v roots="$(value roots)" -v files="$(value source_closure)" \
    -v requests="$(value request_edges)" -v maximum="$(value max_closure_files)" '
    NR>1{
        roots_seen++
        files_seen+=$2
        requests_seen+=$3
        if($2>max_seen)max_seen=$2
    }
    END{
        if(roots_seen!=roots||files_seen!=files||
           requests_seen!=requests||max_seen!=maximum)exit 1
    }
' "$closures" || die 'R3dz-A aggregate closure certificate drifted'
sed '1d' "$closures" | cut -f1 | diff -u "$manifest" -

[[ "$(toml_value quickjs version)" == "$(value quickjs)" \
    && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
    && "$(toml_value test262 commit)" == "$(value test262)" \
    && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
    && "$(toml_value test262 oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
    || die 'compat/upstream.toml no longer names the R3dz-A pinned inputs'
[[ "$(sha "$profile")" == "$(value candidate_profile_sha256)" ]] \
    || die 'live Test262 capability profile drifted from R3dz-A'

parent_commit=$(value parent_commit)
candidate_commit=$(value candidate_commit)
for commit in "$parent_commit" "$candidate_commit"; do
    trusted_git -C "$root" cat-file -e "$commit^{commit}" 2>/dev/null \
        || die "R3dz-A authenticated commit is unavailable: $commit"
done
[[ "$(trusted_git -C "$root" rev-parse "$parent_commit^{tree}")" == "$(value parent_tree)" \
    && "$(trusted_git -C "$root" rev-parse "$candidate_commit^{tree}")" == "$(value candidate_tree)" ]] \
    || die 'R3dz-A parent or candidate tree drifted'

commit_paths=(
    src/bin/run_test262.rs
    src/bin/run_test262/capabilities.rs
    src/bin/run_test262/execution.rs
    src/bin/run_test262/requirements.rs
    compat/test262-oxide.conf
    compat/upstream.toml
)
for side in parent candidate; do
    if [[ "$side" == parent ]]; then
        commit=$parent_commit
    else
        commit=$candidate_commit
    fi
    for rel in "${commit_paths[@]}"; do
        trusted_git -C "$root" show "$commit:$rel" >"$tmp/$side-${rel//\//_}"
    done
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
check_file "$tmp/parent-compat_upstream.toml" \
    "$(value parent_upstream_lines)" "$(value parent_upstream_sha256)"
check_file "$tmp/candidate-src_bin_run_test262.rs" \
    "$(value candidate_runner_lines)" "$(value candidate_runner_sha256)"
check_file "$tmp/candidate-src_bin_run_test262_capabilities.rs" \
    "$(value candidate_capabilities_lines)" "$(value candidate_capabilities_sha256)"
check_file "$tmp/candidate-src_bin_run_test262_execution.rs" \
    "$(value candidate_execution_lines)" "$(value candidate_execution_sha256)"
check_file "$tmp/candidate-src_bin_run_test262_requirements.rs" \
    "$(value candidate_requirements_lines)" "$(value candidate_requirements_sha256)"
check_file "$tmp/candidate-compat_test262-oxide.conf" \
    "$(value candidate_profile_lines)" "$(value candidate_profile_sha256)"
check_file "$tmp/candidate-compat_upstream.toml" \
    "$(value candidate_upstream_lines)" "$(value candidate_upstream_sha256)"

capture_diff_body() {
    local before=$1 after=$2 output=$3 status
    if diff -u "$before" "$after" >"$output.full"; then
        die "R3dz-A expected one authenticated delta: $before -> $after"
    else
        status=$?
        [[ "$status" == 1 ]] \
            || die "R3dz-A could not compare authenticated inputs: $before -> $after"
    fi
    sed '1,2d' "$output.full" >"$output"
}
capture_diff_body "$tmp/parent-compat_test262-oxide.conf" \
    "$tmp/candidate-compat_test262-oxide.conf" "$tmp/profile.diff"
printf '%s\n' \
    '@@ -107,6 +107,7 @@' \
    ' destructuring-binding' \
    ' error-cause' \
    ' exponentiation' \
    '+export-star-as-namespace-from-module' \
    ' for-in-order' \
    ' for-of' \
    ' generators' >"$tmp/expected-profile.diff"
diff -u "$tmp/expected-profile.diff" "$tmp/profile.diff"
sort "$tmp/parent-compat_test262-oxide.conf" >"$tmp/parent-profile.sorted"
sort "$tmp/candidate-compat_test262-oxide.conf" >"$tmp/candidate-profile.sorted"
comm -23 "$tmp/parent-profile.sorted" "$tmp/candidate-profile.sorted" \
    >"$tmp/profile.removed"
comm -13 "$tmp/parent-profile.sorted" "$tmp/candidate-profile.sorted" \
    >"$tmp/profile.added"
[[ ! -s "$tmp/profile.removed" \
    && "$(sed -n '1p' "$tmp/profile.added")" \
        == export-star-as-namespace-from-module \
    && "$(lines "$tmp/profile.added")" == 1 ]] \
    || die 'R3dz-A profile delta is not one exact feature addition'

capture_diff_body "$tmp/parent-compat_upstream.toml" \
    "$tmp/candidate-compat_upstream.toml" "$tmp/upstream.diff"
printf '%s\n' \
    '@@ -19,7 +19,7 @@' \
    ' test_count = 53125' \
    ' metadata_records_sha256 = "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a"' \
    ' oxide_profile = "compat/test262-oxide.conf"' \
    '-oxide_profile_sha256 = "e31b7f24a57354865899e16dc83ae9a149180914f9c572cb3241fa9d59f9d634"' \
    '+oxide_profile_sha256 = "f076aed49c304be872dadc43bd08d2890ac8261df1dbac520bf42d0e3b077a7c"' \
    ' expected_errors = "test262_errors.txt"' \
    ' ' \
    ' [test262_es5]' >"$tmp/expected-upstream.diff"
diff -u "$tmp/expected-upstream.diff" "$tmp/upstream.diff"

for rel in "${commit_paths[@]}"; do
    diff -u "$tmp/candidate-${rel//\//_}" "$rel"
done
! grep -Fq 'NAMESPACE_MODULE_ROOT_ADMISSIONS' \
    "$tmp/parent-src_bin_run_test262_requirements.rs" \
    || die 'R3dz-A parent unexpectedly contains the namespace admission'
grep -Fq 'NAMESPACE_MODULE_ROOT_ADMISSIONS' \
    "$tmp/candidate-src_bin_run_test262_requirements.rs" \
    || die 'R3dz-A candidate no longer contains the namespace admission'
grep -Fq "\"$(value candidate_profile_sha256)\"" \
    "$tmp/candidate-src_bin_run_test262.rs" \
    || die 'R3dz-A candidate runner no longer pins its profile'

while IFS= read -r rel; do
    [[ -f "$rel" && ! -L "$rel" ]] \
        || die "R3dz-A implementation path is missing or unsafe: $rel"
    printf '%s\t%s\t%s\n' "$rel" "$(lines "$rel")" "$(sha "$rel")"
done <"$implementation_manifest" >"$tmp/candidate-implementation.tsv"
[[ "$(sha_stream <"$tmp/candidate-implementation.tsv")" \
    == "$(value candidate_implementation_stream_sha256)" ]] \
    || die 'R3dz-A candidate implementation source inventory drifted'

trusted_git -C "$root" show "$parent_commit:$canonical_baseline" \
    >"$tmp/parent-canonical-baseline"
check_file "$tmp/parent-canonical-baseline" 8 \
    "$(value parent_canonical_baseline_sha256)"
[[ "$(value_from "$tmp/parent-canonical-baseline" tsv_sha256)" \
        == "$(value full_parent_tsv_sha256)" \
    && "$(value_from "$tmp/parent-canonical-baseline" jsonl_sha256)" \
        == "$(value full_parent_jsonl_sha256)" \
    && "$(value_from "$tmp/parent-canonical-baseline" runnable)" \
        == "$(value full_parent_runnable)" \
    && "$(value_from "$tmp/parent-canonical-baseline" passes)" \
        == "$(value full_parent_passes)" \
    && "$(value_from "$tmp/parent-canonical-baseline" summary)" \
        == "$(value full_parent_summary)" ]] \
    || die 'R3dz-A predecessor canonical baseline drifted'

if [[ -n ${TEST262_RUNNER:-} ]]; then
    runner=$TEST262_RUNNER
    [[ "$runner" == /* && -f "$runner" && -x "$runner" && ! -L "$runner" ]] \
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
    [[ -f "$runner" && -x "$runner" && ! -L "$runner" ]] \
        || die 'release run-test262 binary is missing or unsafe'
fi

suite=$("$script_dir/prepare-test262.sh")
[[ -n "$suite" && "$suite" == /* && -d "$suite/test" && ! -L "$suite" ]] \
    || die 'prepare-test262.sh did not return one authenticated suite path'
source_dir=$(CDPATH='' cd -- "$suite/.." && pwd)
[[ "$(trusted_git -C "$suite" rev-parse HEAD)" == "$(value test262)" \
    && "$(sha "$source_dir/tests/test262.patch")" == "$(value test262_patch_sha256)" \
    && "$(sha "$source_dir/test262.conf")" == "$(value test262_config_sha256)" ]] \
    || die 'prepared Test262/QuickJS inputs drifted'

metadata_records=$tmp/test262-metadata.bin
"$runner" --suite "$suite" --validate-metadata "$metadata_records" \
    >"$tmp/metadata-audit.log"
check_file "$metadata_records" "$(value test262_metadata_records)" \
    "$(value test262_metadata_sha256)"
grep -Fqx "Test262 metadata: files=$(value test262_metadata_records)" \
    "$tmp/metadata-audit.log" \
    || die 'Test262 metadata audit did not cover the pinned checkout'
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
        || die "pinned R3dz-A source is missing or unsafe: $rel"
    [[ "$(sha "$source_file")" == "$expected_source" ]] \
        || die "pinned R3dz-A source drifted: $rel"
    awk '/\/\*---/{inside=1} inside{print} /---\*\//{if(inside)exit}' \
        "$source_file" >"$tmp/frontmatter"
    [[ "$(sha "$tmp/frontmatter")" == "$expected_frontmatter" ]] \
        || die "complete pinned R3dz-A frontmatter drifted: $rel"
done < <(sed '1d' "$ledger")

unlisted_path=$(sed -n '1p' "$unlisted_manifest")
[[ "$unlisted_path" == test/language/module-code/ambiguous-export-bindings/namespace-unambiguous-if-export-star-as-from.js \
    && -z "$(comm -12 "$manifest" "$unlisted_manifest")" \
    && "$(sha "$suite/$unlisted_path")" == "$(value unlisted_source_sha256)" ]] \
    || die 'R3dz-A adjacent unlisted root drifted'
awk '/\/\*---/{inside=1} inside{print} /---\*\//{if(inside)exit}' \
    "$suite/$unlisted_path" >"$tmp/unlisted-frontmatter"
[[ "$(sha "$tmp/unlisted-frontmatter")" == "$(value unlisted_frontmatter_sha256)" ]] \
    || die 'R3dz-A unlisted root frontmatter drifted'
awk -F'\t' -v wanted="$unlisted_path" '
    $1==wanted{
        if($2!=""||$3!="module"||$4!=""||$5!=""||$6!="")exit 1
        found++
    }
    END{if(found!=1)exit 1}
' "$tmp/test262-metadata.tsv" || die 'R3dz-A unlisted root metadata drifted'

find "$suite/test/language/module-code/namespace" -type f -name '*.js' \
    ! -name '*_FIXTURE.js' -print \
    | sed "s#^$suite/##" | sort >"$tmp/natural-namespace-roots"
[[ "$(lines "$tmp/natural-namespace-roots")" == 36 ]] \
    || die 'pinned Test262 namespace subtree is no longer exactly 36 natural roots'
{
    cat "$tmp/natural-namespace-roots"
    printf '%s\n' \
        'test/language/module-code/ambiguous-export-bindings/omitted-from-namespace.js'
} | sort >"$tmp/natural-roots"
diff -u "$manifest" "$tmp/natural-roots"

printf 'root_path\tbase_path\trequest_index\tspecifier\tnormalized_path\n' \
    >"$tmp/derived-edges.tsv"
while IFS= read -r record; do
    rel=$(printf '%s\n' "$record" | cut -f1)
    owner=$(printf '%s\n' "$record" | cut -f2)
    sed -En \
        -e "s/.*[[:space:]]from[[:space:]]*['\"]([^'\"]+)['\"].*/\\1/p" \
        -e "s/^[[:space:]]*import[[:space:]]*['\"]([^'\"]+)['\"].*/\\1/p" \
        "$suite/$rel" | awk '!seen[$0]++' >"$tmp/requests"
    request_index=0
    while IFS= read -r specifier; do
        case $specifier in
            ./*) ;;
            *) die "R3dz-A static request is not an exact relative child: $rel -> $specifier" ;;
        esac
        [[ "$specifier" != *'/../'* && "$specifier" != './../'* \
            && "$specifier" != *'/./'* ]] \
            || die "R3dz-A static request escapes lexical child normalization: $specifier"
        normalized=${rel%/*}/${specifier#./}
        printf '%s\t%s\t%s\t%s\t%s\n' \
            "$owner" "$rel" "$request_index" "$specifier" "$normalized" \
            >>"$tmp/derived-edges.tsv"
        request_index=$((request_index + 1))
    done <"$tmp/requests"
done < <(sed '1d' "$ledger")
diff -u "$edges" "$tmp/derived-edges.tsv"

{
    cat "$manifest"
    sed '1d' "$edges" | cut -f5
} | sort -u >"$tmp/derived-source-union"
diff -u "$sources" "$tmp/derived-source-union"

while IFS= read -r root_path; do
    awk -F'\t' -v root_path="$root_path" '
        NR==1{next}
        {count++;base[count]=$2;target[count]=$5}
        END{
            reached[root_path]=1
            do {
                changed=0
                for(i=1;i<=count;i++){
                    if((base[i] in reached)&&!(target[i] in reached)){
                        reached[target[i]]=1
                        changed=1
                    }
                }
            } while(changed)
            for(path in reached)print path
        }
    ' "$edges" | sort >"$tmp/reachable"
    awk -F'\t' -v root_path="$root_path" \
        'NR>1&&$2==root_path{print $1}' "$ledger" | sort >"$tmp/owned"
    diff -u "$tmp/owned" "$tmp/reachable"
    derived_files=$(lines "$tmp/reachable")
    derived_requests=$(awk -F'\t' '
        NR==FNR{reachable[$1]=1;next}
        FNR>1&&($2 in reachable){count++}
        END{print count+0}
    ' "$tmp/reachable" "$edges")
    expected_closure=$(awk -F'\t' -v root_path="$root_path" \
        '$1==root_path{print $2 "\t" $3;found++} END{if(found!=1)exit 1}' \
        "$closures")
    [[ "$derived_files"$'\t'"$derived_requests" == "$expected_closure" ]] \
        || die "R3dz-A per-root closure drifted: $root_path"
done <"$manifest"

verify_report_header() {
    local report=$1 expected_profile=$2 expected_rows=$3 expected_summary=$4
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$expected_profile" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$expected_rows" \
        && "$(report_summary "$report")" == "$expected_summary" ]] \
        || die "R3dz-A classified report contract drifted: $report"
}

verify_report_header "$parent" "$(value parent_profile_sha256)" \
    "$(value roots)" "$(value parent_summary)"
verify_report_header "$candidate" "$(value candidate_profile_sha256)" \
    "$(value roots)" "$(value candidate_summary)"
verify_report_header "$unlisted" "$(value candidate_profile_sha256)" \
    1 "$(value unlisted_summary)"
report_rows "$parent" | cut -f1 | diff -u "$manifest" -
report_rows "$candidate" | cut -f1 | diff -u "$manifest" -
awk -F'\t' '
    !/^#/&&!($1=="path"&&$2=="variant"){
        if($2!="sloppy"||$3!="module"||$5!="normal"||$6!=""||
           $7!="unsupported-module"||$8!="selection"||
           $9!="ExecutionMode"||$10!="missing execution capabilities: module")exit 1
        count++
    }
    END{if(count!=37)exit 1}
' "$parent" || die 'R3dz-A parent receipt is not exactly 37 unsupported modules'
awk -F'\t' '
    !/^#/&&!($1=="path"&&$2=="variant"){
        if($2!="sloppy"||$3!="module"||$5!="normal"||$6!=""||
           $7!="pass"||$8!="normal"||$9!=""||$10!="")exit 1
        count++
    }
    END{if(count!=37)exit 1}
' "$candidate" || die 'R3dz-A candidate receipt is not exactly 37 normal passes'
awk -F'\t' '
    NR==FNR{
        if(/^#/||($1=="path"&&$2=="variant"))next
        old[$1]=$0;before++;next
    }
    /^#/||($1=="path"&&$2=="variant"){next}
    {
        if(!($1 in old))exit 1
        split(old[$1],prior,"\t")
        for(i=2;i<=6;i++)if(prior[i]!=$i)exit 1
        if(prior[7]!="unsupported-module"||$7!="pass")exit 1
        seen[$1]=1;after++
    }
    END{
        if(before!=37||after!=37)exit 1
        for(path in old)if(!(path in seen))exit 1
    }
' "$parent" "$candidate" \
    || die 'R3dz-A focused transition is not exactly 37 unsupported-to-pass rows'
[[ "$(report_rows "$unlisted")" == "$unlisted_path"$'\tsloppy\tmodule\t\tnormal\t\tunsupported-module\tselection\tExecutionMode\tmissing execution capabilities: module' ]] \
    || die 'R3dz-A adjacent unlisted report drifted'
awk '
    /^\{"kind":"metadata",/{metadata++}
    /^\{"kind":"result",/{results++}
    /^\{"kind":"summary",/{summaries++}
    END{if(metadata!=1||results!=37||summaries!=1)exit 1}
' "${parent%.tsv}.jsonl" || die 'R3dz-A parent JSONL shape drifted'
awk '
    /^\{"kind":"metadata",/{metadata++}
    /^\{"kind":"result",/{results++}
    /^\{"kind":"summary",/{summaries++}
    END{if(metadata!=1||results!=37||summaries!=1)exit 1}
' "${candidate%.tsv}.jsonl" || die 'R3dz-A candidate JSONL shape drifted'

if [[ "$mode" == check ]]; then
    echo 'R3dz-A module namespace evidence authenticated: 37/37 roots, 48-source union closure, 46 requests, predecessor and canonical full baselines'
    exit 0
fi

run_focused_candidate() {
    local output=$1
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode "$(value mode)" --workers "$workers" \
        --timeout-ms "$(value timeout_ms)"
}
run_focused_candidate "$candidate_replay_a"
run_focused_candidate "$candidate_replay_b"
diff -u "$candidate" "$candidate_replay_a"
diff -u "${candidate%.tsv}.jsonl" "${candidate_replay_a%.tsv}.jsonl"
if ! cmp -s "$candidate_replay_a" "$candidate_replay_b" \
    || ! cmp -s "${candidate_replay_a%.tsv}.jsonl" \
        "${candidate_replay_b%.tsv}.jsonl"; then
    die 'R3dz-A focused Oxide replays are not byte-identical'
fi

"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$unlisted_manifest" \
    --report "$unlisted_replay" --mode "$(value mode)" --workers 1 \
    --timeout-ms "$(value timeout_ms)" --allow-failures
diff -u "$unlisted" "$unlisted_replay"
diff -u "${unlisted%.tsv}.jsonl" "${unlisted_replay%.tsv}.jsonl"
worker_result=$("$runner" --worker-one --suite "$suite" \
    --test "$unlisted_path" --variant sloppy)
[[ "$worker_result" == $'runner-error\thost\t\tunsupported test reached worker' ]] \
    || die 'direct worker admitted the adjacent unlisted namespace root'

drift_root=$(value drift_root)
drift_fixture=$(value drift_fixture)
while IFS= read -r rel; do
    mkdir -p "$tmp/source-drift/${rel%/*}"
    cp "$suite/$rel" "$tmp/source-drift/$rel"
done < <(awk -F'\t' -v root_path="$drift_root" \
    'NR>1&&$2==root_path{print $1}' "$ledger")
printf '\n// R3dz-A nested fixture drift canary.\n' \
    >>"$tmp/source-drift/$drift_fixture"
[[ "$(sha "$tmp/source-drift/$drift_fixture")" == "$(value drift_mutated_sha256)" ]] \
    || die 'R3dz-A nested fixture canary mutation drifted'
worker_result=$("$runner" --worker-one --suite "$tmp/source-drift" \
    --test "$drift_root" --variant sloppy)
printf -v expected_drift \
    'runner-error\thost\t\tfixture graph module source drifted for %s: expected SHA-256 %s, found %s' \
    "$drift_fixture" "$(value drift_fixture_sha256)" \
    "$(value drift_mutated_sha256)"
[[ "$worker_result" == "$expected_drift" ]] \
    || die 'direct worker did not fail closed on exact nested fixture drift'

quickjs_source=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
[[ "$quickjs_source" == "$source_dir" && -f "$quickjs_source/run-test262" \
    && -x "$quickjs_source/run-test262" && ! -L "$quickjs_source/run-test262" ]] \
    || die 'authenticated QuickJS Test262 oracle path drifted'
quickjs_args=(-m -c test262.conf -a -T 1 -f)
while IFS= read -r rel; do
    quickjs_args+=("test262/$rel")
done <"$manifest"
for log in "$quickjs_log_a" "$quickjs_log_b"; do
    (cd "$quickjs_source" && ./run-test262 "${quickjs_args[@]}") >"$log" 2>&1 \
        || die "pinned QuickJS rejected the R3dz-A cohort: $log"
    ! grep -Fq 'FAILED' "$log" \
        || die "pinned QuickJS reported a failed R3dz-A test: $log"
    [[ "$(grep -Fxc 'Average memory statistics for 37 tests:' "$log")" == 1 ]] \
        || die "pinned QuickJS did not execute exactly 37 R3dz-A tests: $log"
    awk '/^test262\.conf:.* ignoring testdir=/{print}
         /^Average memory statistics for [0-9]+ tests:/{print}' \
        "$log" >"$log.projection"
    diff -u "$quickjs_projection" "$log.projection"
done
cmp -s "$quickjs_log_a.projection" "$quickjs_log_b.projection" \
    || die 'pinned QuickJS semantic projections are not byte-identical'

if [[ "$mode" != full ]]; then
    echo 'R3dz-A module namespace focused gate passed: Oxide 37/37 twice, QuickJS 37/37 twice, unlisted and nested drift fail closed'
    exit 0
fi

for full_path in "$full_candidate_a" "$full_candidate_b"; do
    case $full_path in
        /*) ;;
        *) die 'R3dz-A full report paths must be absolute' ;;
    esac
done

run_full_candidate() {
    local output=$1 json
    json=${output%.tsv}.jsonl
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
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$tmp/$label.keys")" == "$(value full_variants)" \
        && "$(sha "$tmp/$label.keys")" == "$(value full_keys_sha256)" \
        && -z "$(uniq -d "$tmp/$label.keys")" \
        && "$(report_summary "$report")" == "$(value full_candidate_summary)" \
        && "$(report_runnable "$report")" == "$(value full_candidate_runnable)" \
        && "$(report_count pass "$report")" == "$(value full_candidate_passes)" ]] \
        || die "R3dz-A full candidate outcome drifted: $report"
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
    die 'R3dz-A full candidate replays must be distinct files'
fi
if ! cmp -s "$full_candidate_a" "$full_candidate_b" \
    || ! cmp -s "${full_candidate_a%.tsv}.jsonl" \
        "${full_candidate_b%.tsv}.jsonl"; then
    die 'R3dz-A full candidate replays are not byte-identical'
fi

derived_parent_tsv=$tmp/full-parent.tsv
derived_parent_json=$tmp/full-parent.jsonl
awk -F'\t' -v OFS='\t' \
    -v candidate_profile="$(value candidate_profile_sha256)" \
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
        $10="missing execution capabilities: module";outcomes++
    }
    !/^#/&&!($1=="path"&&$2=="variant")&&
        $4~/export-star-as-namespace-from-module/&&
        $7=="unsupported-feature"&&
        $10=="quickjs-oxide does not declare Test262 feature support: dynamic-import" {
        $10=$10 ", export-star-as-namespace-from-module";details++
    }
    {print}
    END{
        if(manifest_count!=37||headers!=1||summaries!=1||
           outcomes!=37||details!=8)exit 4
    }
' "$manifest" "$full_candidate_a" >"$derived_parent_tsv" \
    || die 'R3dz-A could not reverse the full TSV candidate into R3dy-A'
awk -v candidate_profile="$(value candidate_profile_sha256)" \
    -v parent_profile="$(value parent_profile_sha256)" '
    NR==FNR{wanted[$1]=1;manifest_count++;next}
    /^\{"kind":"metadata",/ {
        if(index($0,candidate_profile)==0)exit 2
        sub(candidate_profile,parent_profile);headers++;print;next
    }
    /^\{"kind":"summary",/ {
        print "{\"kind\":\"summary\",\"outcomes\":{\"fail-parse\":7,\"fail-runtime\":43,\"pass\":68108,\"skipped-config-exclude\":6700,\"skipped-feature\":11775,\"timeout\":2,\"unsupported-feature\":11348,\"unsupported-module\":662,\"unsupported-negative-provenance\":3392}}"
        summaries++;next
    }
    {
        hit=0
        for(path in wanted){
            if(index($0,"\"path\":\"" path "\"")>0){hit=1;break}
        }
        if(hit){
            if(!sub(/\"outcome\":\"pass\",\"actual_phase\":\"normal\",\"actual_type\":\"\",\"detail\":\"\"/,"\"outcome\":\"unsupported-module\",\"actual_phase\":\"selection\",\"actual_type\":\"ExecutionMode\",\"detail\":\"missing execution capabilities: module\""))exit 3
            outcomes++
        } else if(index($0,"\"features\":\"export-star-as-namespace-from-module,dynamic-import\"")>0&&
            index($0,"\"outcome\":\"unsupported-feature\"")>0&&
            sub(/\"detail\":\"quickjs-oxide does not declare Test262 feature support: dynamic-import\"/,"\"detail\":\"quickjs-oxide does not declare Test262 feature support: dynamic-import, export-star-as-namespace-from-module\"")){
            details++
        }
        print
    }
    END{
        if(manifest_count!=37||headers!=1||summaries!=1||
           outcomes!=37||details!=8)exit 4
    }
' "$manifest" "${full_candidate_a%.tsv}.jsonl" >"$derived_parent_json" \
    || die 'R3dz-A could not reverse the full JSONL candidate into R3dy-A'
check_file "$derived_parent_tsv" "$(value full_report_lines)" \
    "$(value full_parent_tsv_sha256)"
check_file "$derived_parent_json" "$(value full_jsonl_lines)" \
    "$(value full_parent_jsonl_sha256)"

awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
    !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted){print}' \
    "$manifest" "$full_candidate_a" >"$tmp/full-candidate-scope.tsv"
awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
    !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted){print}' \
    "$manifest" "$derived_parent_tsv" >"$tmp/full-parent-scope.tsv"
report_rows "$candidate" >"$tmp/focused-candidate-scope.tsv"
report_rows "$parent" >"$tmp/focused-parent-scope.tsv"
diff -u "$tmp/focused-candidate-scope.tsv" "$tmp/full-candidate-scope.tsv"
diff -u "$tmp/focused-parent-scope.tsv" "$tmp/full-parent-scope.tsv"

awk -v manifest="$manifest" '
    BEGIN{
        while((getline path < manifest)>0)wanted[path]=1
        close(manifest)
    }
    /^\{"kind":"result",/ {
        for(path in wanted){
            if(index($0,"\"path\":\"" path "\"")>0){print;break}
        }
    }
' "${full_candidate_a%.tsv}.jsonl" >"$tmp/full-candidate-scope.jsonl"
awk -v manifest="$manifest" '
    BEGIN{
        while((getline path < manifest)>0)wanted[path]=1
        close(manifest)
    }
    /^\{"kind":"result",/ {
        for(path in wanted){
            if(index($0,"\"path\":\"" path "\"")>0){print;break}
        }
    }
' "$derived_parent_json" >"$tmp/full-parent-scope.jsonl"
awk '/^\{"kind":"result",/' "${candidate%.tsv}.jsonl" \
    >"$tmp/focused-candidate-scope.jsonl"
awk '/^\{"kind":"result",/' "${parent%.tsv}.jsonl" \
    >"$tmp/focused-parent-scope.jsonl"
diff -u "$tmp/focused-candidate-scope.jsonl" "$tmp/full-candidate-scope.jsonl"
diff -u "$tmp/focused-parent-scope.jsonl" "$tmp/full-parent-scope.jsonl"

transition_counts=$(awk -F'\t' -v parent="$derived_parent_tsv" \
    -v outcome_keys="$tmp/full-outcome.keys" \
    -v detail_keys="$tmp/full-detail.keys" '
    FILENAME==parent{
        if(!/^#/&&!($1=="path"&&$2=="variant")){
            old[$1 FS $2]=$0;before++
        }
        next
    }
    !/^#/&&!($1=="path"&&$2=="variant"){
        key=$1 FS $2
        if(!(key in old))exit 2
        split(old[key],prior,FS)
        for(i=1;i<=6;i++)if(prior[i]!=$i)exit 3
        if(prior[7]!="pass"&&$7=="pass")gains++
        if(prior[7]=="pass"&&$7!="pass")regressions++
        if(old[key]!=$0){
            changed++
            if(prior[7]!=$7){
                outcome++
                print $1 "\t" $2 > outcome_keys
            } else {
                detail++
                print $1 "\t" $2 > detail_keys
            }
        }
        seen[key]=1
    }
    END{
        for(key in old)if(!(key in seen))exit 4
        printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d",
            changed,outcome,detail,before-changed,gains,regressions
    }
' "$derived_parent_tsv" "$full_candidate_a") \
    || die 'R3dz-A full exact join failed'
expected_counts="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) gains=$(value full_pass_gains) regressions=$(value full_pass_regressions)"
[[ "$transition_counts" == "$expected_counts" ]] \
    || die "R3dz-A full transition drifted: $transition_counts"
sort "$tmp/full-outcome.keys" >"$tmp/full-outcome.sorted.keys"
sort "$tmp/full-detail.keys" >"$tmp/full-detail.sorted.keys"
[[ "$(lines "$tmp/full-outcome.sorted.keys")" == "$(value full_outcome_changed)" \
    && "$(sha "$tmp/full-outcome.sorted.keys")" == "$(value full_outcome_keys_sha256)" \
    && "$(lines "$tmp/full-detail.sorted.keys")" == "$(value full_detail_only)" \
    && "$(sha "$tmp/full-detail.sorted.keys")" == "$(value full_detail_only_keys_sha256)" ]] \
    || die 'R3dz-A full transition key partition drifted'
sed $'s/$/\tsloppy/' "$manifest" >"$tmp/expected-outcome.keys"
diff -u "$tmp/expected-outcome.keys" "$tmp/full-outcome.sorted.keys"
[[ "$(lines "$tmp/full-candidate-scope.tsv")" == "$(value full_scope_variants)" \
    && "$(( $(value full_variants) - $(value full_scope_variants) ))" \
        == "$(value full_outside_variants)" ]] \
    || die 'R3dz-A full scope partition drifted'

printf 'R3dz-A full A/B admission gate passed (%s): candidate=%s json=%s; exact reverse-derived predecessor matches R3dy-A.\n' \
    "$full_receipt_kind" "$(sha "$full_candidate_a")" \
    "$(sha "${full_candidate_a%.tsv}.jsonl")"
