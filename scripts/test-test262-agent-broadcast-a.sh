#!/usr/bin/env bash
# Authenticate the R3dn scoped Test262 agent broadcast cohort A receipt.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-agent-broadcast-a-baseline.txt
upstream=compat/upstream.toml
predecessor_gate=scripts/test-test262-agent-stage-a.sh
predecessor_baseline=tests/test262-agent-stage-a-baseline.txt
predecessor_retained=tests/test262-agent-stage-a-retained.txt
stage_a_activation=tests/test262-agent-stage-a.txt
universe=tests/test262-agent-broadcast-a-universe.txt
activation=tests/test262-agent-broadcast-a.txt
retained=tests/test262-agent-broadcast-a-retained.txt
parent_profile=tests/test262-agent-broadcast-a-parent.conf
candidate_profile=tests/test262-agent-broadcast-a-candidate.conf
parent_report=tests/test262-agent-broadcast-a-parent.tsv
candidate_report=tests/test262-agent-broadcast-a-candidate.tsv
transition=tests/test262-agent-broadcast-a-transitions.tsv
quickjs_receipt=tests/test262-agent-broadcast-a-quickjs-receipt.txt
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  replay the exact scoped broadcast cohort A receipt\n'
}

case ${1-} in
    ''|--check) ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid Test262 worker count' >&2; exit 2; }
[[ -z ${TEST262_RUNNER+x} ]] \
    || { echo 'error: TEST262_RUNNER override is forbidden for R3dn broadcast A' >&2; exit 2; }

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
toml_test262_value() {
    awk -v wanted="$1" '
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
    ' "$upstream"
}

verify_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == both \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value universe_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value report_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "scoped report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive Test262 agent broadcast cohort A transition.'
        echo "# parent_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_profile_sha256=$(value candidate_profile_sha256)"
        echo "# universe_sha256=$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{
                if(!/^#/&&!($1=="path"&&$2=="variant")){key=$1 FS $2;old[key]=$0}
                next
            }
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old)||key in seen)exit 2
                split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 4}
        ' "$before" "$after"
    } >"$output"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dn-scoped.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM

check_file "$baseline" 92 \
    977cb89120daa603aee8b73ab87a4e180052011adeac2253271837ff4b74c84b
[[ "$(value scope_semantics)" == fixed-sab-generation-broadcast \
    && "$(value fixed_sab_broadcast)" == true \
    && "$(value ordinary_array_buffer_broadcast)" == fail-closed \
    && "$(value growable_sab_broadcast)" == fail-closed \
    && "$(value broadcast_ack)" == before-callback \
    && "$(value receiver_rearm)" == promise-job-next-generation \
    && "$(value worker_slot_after_exit)" == retained-no-auto-ack \
    && "$(value worker_exception)" == surfaced-by-join \
    && "$(value host_timeout)" == none \
    && "$(value thread_stack_bytes)" == 2097152 \
    && "$(value worker_runtime)" == fresh-per-agent \
    && "$(value worker_can_block)" == true \
    && "$(value agent_host_admission)" == exact-path-source-sha256-metadata \
    && "$(value worker_flag)" == allow-agent-host ]] \
    || die 'R3dn scoped semantic boundary drifted'

check_file "$upstream" 28 "$(value upstream_sha256)"
check_file "$predecessor_gate" 303 "$(value predecessor_gate_sha256)"
check_file "$predecessor_baseline" 78 "$(value predecessor_baseline_sha256)"
check_file "$predecessor_retained" 58 "$(value predecessor_retained_sha256)"
check_file "$universe" "$(value universe_paths)" "$(value universe_sha256)"
check_file "$activation" "$(value activation_paths)" "$(value activation_sha256)"
check_file "$retained" "$(value retained_paths)" "$(value retained_sha256)"
check_file "$parent_profile" "$(value parent_profile_lines)" \
    "$(value parent_profile_sha256)"
check_file "$candidate_profile" "$(value candidate_profile_lines)" \
    "$(value candidate_profile_sha256)"
check_file "$parent_report" "$(value parent_tsv_lines)" \
    "$(value parent_tsv_sha256)"
check_file "${parent_report%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
    "$(value parent_jsonl_sha256)"
check_file "$candidate_report" "$(value candidate_tsv_lines)" \
    "$(value candidate_tsv_sha256)"
check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
    "$(value candidate_jsonl_sha256)"
check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
    "$(value quickjs_receipt_sha256)"

for file in "$universe" "$activation" "$retained"; do
    sort -c "$file" || die "manifest is not bytewise sorted: $file"
    [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
done
diff -u "$predecessor_retained" "$universe"
sort -u "$activation" "$retained" >"$tmp/partition.txt"
diff -u "$universe" "$tmp/partition.txt"
[[ -z "$(comm -12 "$activation" "$retained")" ]] \
    || die 'broadcast A activation and retained partitions overlap'

for section in features audited-negative-tests host-agent-tests; do
    profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
    profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
done
diff -u "$tmp/parent.features" "$tmp/candidate.features"
diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
[[ "$(lines "$tmp/parent.features")" == "$(value profile_features)" \
    && "$(sha "$tmp/parent.features")" == "$(value profile_features_sha256)" \
    && "$(lines "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_paths)" \
    && "$(sha "$tmp/parent.host-agent-tests")" == "$(value parent_agent_allowlist_sha256)" \
    && "$(lines "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_paths)" \
    && "$(sha "$tmp/candidate.host-agent-tests")" == "$(value candidate_agent_allowlist_sha256)" ]] \
    || die 'broadcast A scoped profile inventory drifted'
diff -u "$stage_a_activation" "$tmp/parent.host-agent-tests"
[[ -z "$(comm -23 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests")" ]] \
    || die 'broadcast A candidate removed a historical agent admission'
comm -13 "$tmp/parent.host-agent-tests" "$tmp/candidate.host-agent-tests" \
    >"$tmp/agent-allowlist-delta.txt"
check_file "$tmp/agent-allowlist-delta.txt" "$(value agent_allowlist_delta_paths)" \
    "$(value agent_allowlist_delta_sha256)"
diff -u "$activation" "$tmp/agent-allowlist-delta.txt"

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
while IFS= read -r test_path; do
    source=$suite/$test_path
    [[ -f "$source" ]] || die "pinned activation source is missing: $test_path"
    printf '%s\t%s\n' "$test_path" "$(sha "$source")"
    [[ "$(grep -Fo '$262.agent.start' "$source" | lines /dev/stdin)" == 1 \
        && "$(grep -Fo '$262.agent.receiveBroadcast' "$source" | lines /dev/stdin)" == 1 \
        && "$(grep -Fo '$262.agent.safeBroadcast' "$source" | lines /dev/stdin)" == 1 \
        && "$(grep -Fo 'new SharedArrayBuffer' "$source" | lines /dev/stdin)" == 1 \
        && "$(grep -Ec '^\$262\.agent\.safeBroadcast\([^,()]+\);$' "$source")" == 1 ]] \
        || die "broadcast A activation source shape drifted: $test_path"
done <"$activation" >"$tmp/source-ledger.tsv"
check_file "$tmp/source-ledger.tsv" "$(value activation_source_ledger_lines)" \
    "$(value activation_source_ledger_sha256)"

[[ "$(toml_test262_value commit)" == "$(value test262)" \
    && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 inputs drifted'

cargo test --quiet --locked --bin run-test262 \
    cli_tests::agent_broadcast_a_profiles_require_exact_58_path_universe_or_all \
    -- --exact --test-threads=1
cargo test --quiet --locked --lib test262_agent -- --test-threads=1
RUSTFLAGS='-D warnings' cargo check --quiet --locked --target wasm32-unknown-unknown --lib
RUSTFLAGS='-D warnings' cargo clippy --quiet --locked --target wasm32-unknown-unknown \
    --lib -- -D warnings
cargo build --quiet --locked --release --bin run-test262
target_dir=${CARGO_TARGET_DIR:-target}
case $target_dir in /*) ;; *) target_dir=$root/$target_dir ;; esac
runner=$target_dir/release/run-test262

"$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
[[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
    && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata drifted'

run_report() {
    local profile=$1 selected=$2 output=$3 run_workers=${4:-$workers}
    case $output in /*) ;; *) output=$root/$output ;; esac
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --manifest "$root/$selected" \
        --report "$output" --mode both --timeout-ms 30000 \
        --workers "$run_workers" --allow-failures >/dev/null
}

run_report "$parent_profile" "$universe" "$tmp/parent.tsv"
run_report "$candidate_profile" "$universe" "$tmp/candidate.tsv"
cmp -s "$parent_report" "$tmp/parent.tsv" \
    && cmp -s "${parent_report%.tsv}.jsonl" "$tmp/parent.jsonl" \
    || die 'broadcast A parent replay drifted'
cmp -s "$candidate_report" "$tmp/candidate.tsv" \
    && cmp -s "${candidate_report%.tsv}.jsonl" "$tmp/candidate.jsonl" \
    || die 'broadcast A candidate replay drifted'
verify_report "$parent_report" "$(value parent_profile_sha256)" parent
verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
[[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
    && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
    && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
    && "$(report_count unsupported-host-agent "$candidate_report")" == \
        "$(value candidate_retained_unsupported)" ]] \
    || die 'broadcast A scoped outcome counts drifted'

make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
[[ "$(report_rows "$transition" | sha /dev/stdin)" == \
        "$(value transition_data_sha256)" ]] \
    || die 'broadcast A transition rows drifted'
counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if($7=="pass"&&$11!="pass")regress++
    if(different){changed++;if($11=="pass")gain++}else unchanged++
}END{printf "%d %d %d %d",changed,gain,unchanged,regress}' "$transition")
[[ "$counts" == "$(value changed) $(value pass_changes) $(value unchanged) $(value regressions)" ]] \
    || die "broadcast A transition counts drifted: $counts"

for rejected in "$activation" "$retained" Cargo.toml; do
    if run_report "$candidate_profile" "$rejected" "$tmp/rejected.tsv" 1 \
        >/dev/null 2>&1; then
        die "broadcast A profile accepted a non-universe manifest: $rejected"
    fi
done
if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$root/$candidate_profile" \
    --test test/built-ins/Atomics/notify/notify-with-no-agents-waiting.js \
    --report "$tmp/rejected.tsv" --mode both --workers 1 --allow-failures \
    >/dev/null 2>&1; then
    die 'broadcast A scoped profile accepted --test'
fi

active_variants=0
stability_passes=0
for run in $(seq 1 "$(value stability_runs)"); do
    run_report "$candidate_profile" "$universe" "$tmp/stability-$run.tsv" 1
    runnable=$(report_runnable "$tmp/stability-$run.tsv")
    passes=$(report_count pass "$tmp/stability-$run.tsv")
    [[ "$(sha "$tmp/stability-$run.tsv")" == "$(value stability_tsv_sha256)" \
        && "$(sha "$tmp/stability-$run.jsonl")" == "$(value stability_jsonl_sha256)" \
        && "$runnable" == "$(value candidate_runnable)" \
        && "$passes" == "$(value candidate_passes)" ]] \
        || die "broadcast A stability replay $run drifted"
    active_variants=$((active_variants + runnable))
    stability_passes=$((stability_passes + passes))
done
[[ "$active_variants" == "$(value stability_active_variants)" \
    && "$stability_passes" == "$(value stability_passes)" ]] \
    || die 'broadcast A aggregate stability counts drifted'

[[ -x "$source_dir/run-test262" ]] \
    || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$activation"
if ! (cd -- "$source_dir" && ./run-test262 -m -c test262.conf -a -T 1 \
    -f "${quickjs_files[@]}") >"$tmp/quickjs.log" 2>&1; then
    tail -n 100 "$tmp/quickjs.log" >&2
    die 'pinned QuickJS could not execute broadcast cohort A'
fi
grep -Fq 'Average memory statistics for 30 tests:' "$tmp/quickjs.log" \
    && ! grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
        "$tmp/quickjs.log" \
    || die 'pinned QuickJS no longer passes broadcast cohort A 30/30'
{
    echo '# Pinned QuickJS oracle receipt for Test262 agent broadcast cohort A.'
    echo "quickjs=$(value quickjs)"
    echo "quickjs_source_sha256=$(value quickjs_source_sha256)"
    echo "test262=$(value test262)"
    echo "activation_manifest_sha256=$(value activation_sha256)"
    echo "activation_source_ledger_sha256=$(value activation_source_ledger_sha256)"
    echo "paths=$(value quickjs_paths)"
    echo "variants=$(value activation_variants)"
    echo "passes=$(value quickjs_passes)"
    echo 'failed=0'
    echo 'skipped_feature=0'
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
diff -u "$quickjs_receipt" "$tmp/quickjs-receipt.txt"

printf 'R3dn scoped agent broadcast A verified: Oxide 30/30, pinned QuickJS 30/30, retained 86 fail closed, stability 600/600.\n'
