#!/usr/bin/env bash
# Authenticate the R3dm scoped Test262 agent Stage A implementation receipt.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-agent-stage-a-baseline.txt
upstream=compat/upstream.toml
atomics_ledger=tests/test262-atomics-universe.tsv
universe=tests/test262-agent-stage-a-universe.txt
activation=tests/test262-agent-stage-a.txt
retained=tests/test262-agent-stage-a-retained.txt
parent_profile=tests/test262-agent-stage-a-parent.conf
candidate_profile=tests/test262-agent-stage-a-candidate.conf
parent_report=tests/test262-agent-stage-a-parent.tsv
candidate_report=tests/test262-agent-stage-a-candidate.tsv
transition=tests/test262-agent-stage-a-transitions.tsv
quickjs_receipt=tests/test262-agent-stage-a-quickjs-receipt.txt
oracle_log=target/test262-agent-stage-a-quickjs.log
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  replay the exact scoped Stage A implementation receipt\n'
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
    || { echo 'error: TEST262_RUNNER override is forbidden for R3dm Stage A' >&2; exit 2; }

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
json_rows() { awk '/^\{"kind":"result"/' "$1"; }
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
        && "$(report_keys "$report" | sha /dev/stdin)" == 25b3648efc7b45d5aec9c950ced7abed66dd6eac1c6e5c94e38f6ff44cad2cb0 \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "scoped report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive Test262 agent Stage A transition.'
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
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dm-scoped.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM

check_file "$baseline" 78 \
    56acd44b0b63ed1354f3e53c97d84b382be4b478245df0520668060c099b523f
check_file "$atomics_ledger" "$(value atomics_ledger_lines)" \
    "$(value atomics_ledger_sha256)"
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
awk -F'\t' '$2=="shared-agent"{print $1}' "$atomics_ledger" >"$tmp/universe.txt"
diff -u "$universe" "$tmp/universe.txt"
sort -u "$activation" "$retained" >"$tmp/partition.txt"
diff -u "$universe" "$tmp/partition.txt"
[[ -z "$(comm -12 "$activation" "$retained")" ]] \
    || die 'Stage A activation and retained partitions overlap'

for section in features audited-negative-tests host-agent-tests; do
    profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
    profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
done
diff -u "$tmp/parent.features" "$tmp/candidate.features"
diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
[[ "$(lines "$tmp/parent.features")" == "$(value profile_features)" \
    && "$(sha "$tmp/parent.features")" == "$(value profile_features_sha256)" \
    && ! -s "$tmp/parent.host-agent-tests" \
    && "$(lines "$tmp/candidate.host-agent-tests")" == "$(value agent_allowlist_paths)" \
    && "$(sha "$tmp/candidate.host-agent-tests")" == "$(value agent_allowlist_sha256)" ]] \
    || die 'Stage A scoped profile inventory drifted'
diff -u "$activation" "$tmp/candidate.host-agent-tests"

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
check_file "$suite/$(head -n 1 "$activation")" 66 \
    "$(value activation_source_sha256)"
grep -Fq '$262.agent.start' "$suite/$(head -n 1 "$activation")" \
    && grep -Fq '$262.agent.report' "$suite/$(head -n 1 "$activation")" \
    && grep -Fq '$262.agent.getReport' "$suite/$(head -n 1 "$activation")" \
    && grep -Fq '$262.agent.leaving' "$suite/$(head -n 1 "$activation")" \
    || die 'Stage A activation source shape drifted'
! grep -Eq '\$262\.agent\.(broadcast|receiveBroadcast)|Atomics\.waitAsync' \
    "$suite/$(head -n 1 "$activation")" \
    || die 'Stage A activation leaked later agent semantics'

[[ "$(toml_test262_value commit)" == "$(value test262)" \
    && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 inputs drifted'

cargo test --quiet --locked --lib test262_agent -- --test-threads=1
cargo check --quiet --locked --target wasm32-unknown-unknown --lib
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
    || die 'Stage A parent replay drifted'
cmp -s "$candidate_report" "$tmp/candidate.tsv" \
    && cmp -s "${candidate_report%.tsv}.jsonl" "$tmp/candidate.jsonl" \
    || die 'Stage A candidate replay drifted'
verify_report "$parent_report" "$(value parent_profile_sha256)" parent
verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate
[[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
    && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
    && "$(report_count unsupported-host-agent "$candidate_report")" == \
        "$(value candidate_retained_unsupported)" ]] \
    || die 'Stage A scoped outcome counts drifted'

make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
[[ "$(report_rows "$transition" | sha /dev/stdin)" == \
        "$(value transition_data_sha256)" ]] \
    || die 'Stage A transition rows drifted'
counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if($7=="pass"&&$11!="pass")regress++
    if(different){changed++;if($11=="pass")gain++}else unchanged++
}END{printf "%d %d %d %d",changed,gain,unchanged,regress}' "$transition")
[[ "$counts" == "$(value changed) $(value pass_changes) $(value unchanged) $(value regressions)" ]] \
    || die "Stage A transition counts drifted: $counts"

for rejected in "$retained" Cargo.toml; do
    if run_report "$candidate_profile" "$rejected" "$tmp/rejected.tsv" 1 \
        >/dev/null 2>&1; then
        die "Stage A profile accepted a non-Stage-A manifest: $rejected"
    fi
done
if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$root/$candidate_profile" --all --report "$tmp/rejected.tsv" \
    --mode both --workers 1 --allow-failures >/dev/null 2>&1; then
    die 'Stage A scoped profile accepted --all'
fi
if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$root/$candidate_profile" \
    --test test/built-ins/Atomics/wait/good-views.js \
    --report "$tmp/rejected.tsv" --mode both --workers 1 --allow-failures \
    >/dev/null 2>&1; then
    die 'Stage A scoped profile accepted --test'
fi

for run in $(seq 1 "$(value stability_runs)"); do
    run_report "$candidate_profile" "$activation" "$tmp/stability-$run.tsv" 1
    [[ "$(sha "$tmp/stability-$run.tsv")" == "$(value activation_tsv_sha256)" \
        && "$(sha "$tmp/stability-$run.jsonl")" == "$(value activation_jsonl_sha256)" \
        && "$(report_count pass "$tmp/stability-$run.tsv")" == 2 ]] \
        || die "Stage A stability replay $run drifted"
done

[[ -x "$source_dir/run-test262" ]] \
    || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
if ! (cd -- "$source_dir" && ./run-test262 -m -c test262.conf -a -T "$workers" \
    -f test262/test/built-ins/Atomics/wait/good-views.js) \
    >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS could not execute Stage A'
fi
grep -Fq 'Average memory statistics for 2 tests:' "$oracle_log" \
    && ! grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || die 'pinned QuickJS no longer passes Stage A 2/2'

printf 'R3dm scoped agent Stage A verified: Oxide 2/2, pinned QuickJS 2/2, retained 116 fail closed.\n'
