#!/usr/bin/env bash
# Reproduce and authenticate the R3ed-A module static parse-negative successor.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

baseline=tests/test262-module-static-negative-a-global-baseline.txt
predecessor_baseline=tests/test262-module-decl-position-a-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
generator=scripts/generate-test262-module-static-negative-a.mjs
prepare_suite=scripts/prepare-test262.sh
oracle_builder=scripts/build-quickjs-oracle.sh
manifest=tests/test262-module-static-negative-a.txt
ledger=tests/test262-module-static-negative-a-ledger.tsv
requests=tests/test262-module-static-negative-a-requests.tsv
variants=tests/test262-module-static-negative-a-variants.tsv
negatives=tests/test262-module-static-negative-a-negatives.txt
exclusions=tests/test262-module-static-negative-a-exclusions.tsv
provenance=tests/test262-module-static-negative-a-provenance.tsv
quickjs_projection=tests/test262-module-static-negative-a-quickjs-projection.txt
candidate=tests/test262-module-static-negative-a-global-candidate.tsv
workers=${TEST262_WORKERS:-4}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
full_dir=${TEST262_FULL_REPORT_DIR:-$root/target/test262-module-static-negative-a-global-full}
full_candidate_a=${TEST262_FULL_CANDIDATE_A_REPORT:-$full_dir/candidate-a.tsv}
full_candidate_b=${TEST262_FULL_CANDIDATE_B_REPORT:-$full_dir/candidate-b.tsv}
runner_override=${TEST262_RUNNER:-}
baseline_lines=102
baseline_sha256=9a6b39b91aee092d1280aa2e007b7902d58369c965d79538b7c4d390744a867c

usage() {
    printf 'usage: %s [--check|--focused|--full]\n' "${0##*/}"
    printf '  --check    authenticate frozen inputs and receipts\n'
    printf '  --focused  rerun the 67-path Oxide and QuickJS cohorts twice (default)\n'
    printf '  --full     additionally run or reuse two complete 102037-row reports\n'
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
predecessor_value() { value_from "$predecessor_baseline" "$1"; }
canonical_value() { value_from "$canonical_baseline" "$1"; }
check_file() {
    [[ -f "$1" && ! -L "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated R3ed-A input drifted: $1"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
rows_for_keys() {
    local keys=$1 report=$2
    awk -F'\t' '
        NR==FNR{wanted[$1 FS $2]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&(($1 FS $2) in wanted){print}
    ' "$keys" "$report"
}
json_result_projection() {
    node - "$1" <<'NODE'
const fs = require("node:fs");
const fields = [
  "path", "variant", "flags", "features", "expected_phase",
  "expected_type", "outcome", "actual_phase", "actual_type", "detail",
];
function escapeField(value) {
  let output = "";
  for (const character of value) {
    if (character === "\\") output += "\\\\";
    else if (character === "\t") output += "\\t";
    else if (character === "\n") output += "\\n";
    else if (character === "\r") output += "\\r";
    else if (/\p{Cc}/u.test(character)) {
      output += "\\u" + character.codePointAt(0).toString(16).padStart(4, "0");
    } else output += character;
  }
  return output;
}
for (const line of fs.readFileSync(process.argv[2], "utf8").trimEnd().split("\n")) {
  const record = JSON.parse(line);
  if (record.kind === "result") {
    process.stdout.write(fields.map((field) => escapeField(record[field])).join("\t") + "\n");
  }
}
NODE
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

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3ed-a.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$tmp/git-home" "$tmp/git-config"

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

reverse_tsv() {
    local input=$1 output=$2 candidate_summary=$3 parent_summary=$4
    awk -F'\t' -v OFS='\t' \
        -v keys_file="$tmp/focused.keys" \
        -v candidate_profile="$(value candidate_profile_sha256)" \
        -v parent_profile="$(value parent_profile_sha256)" \
        -v candidate_summary="$candidate_summary" \
        -v parent_summary="$parent_summary" '
        BEGIN {
            while ((getline row < keys_file) > 0) {
                split(row, fields, "\t"); wanted[fields[1] FS fields[2]]=1
            }
            close(keys_file)
        }
        /^# oxide_profile_sha256=/ {
            if(index($0,candidate_profile)==0)exit 2
            sub(candidate_profile,parent_profile);headers++;print;next
        }
        /^# summary / {
            if($0!="# summary " candidate_summary)exit 3
            print "# summary " parent_summary;summaries++;next
        }
        !/^#/&&!($1=="path"&&$2=="variant") {
            key=$1 FS $2
            if(key in wanted) {
                if($7!="pass"||$8!="parse"||$9!="SyntaxError")exit 4
                $7="unsupported-module";$8="selection";$9="ExecutionMode"
                $10="missing execution capabilities: module";changed++
            }
        }
        {print}
        END{if(headers!=1||summaries!=1||changed!=67)exit 5}
    ' "$input" >"$output" || die "could not reverse R3ed-A TSV: $input"
}

reverse_jsonl() {
    local input=$1 output=$2 candidate_summary_json=$3 parent_summary_json=$4
    node - "$input" "$output" "$tmp/focused.keys" \
        "$(value candidate_profile_sha256)" "$(value parent_profile_sha256)" \
        "$candidate_summary_json" "$parent_summary_json" <<'NODE'
const fs = require("node:fs");
const [input, output, keyFile, candidateProfile, parentProfile,
  candidateSummaryJson, parentSummaryJson] = process.argv.slice(2);
const keys = new Set(fs.readFileSync(keyFile, "utf8").trimEnd().split("\n"));
const expectedSummary = JSON.stringify(JSON.parse(candidateSummaryJson));
const parentSummary = JSON.parse(parentSummaryJson);
let metadata = 0;
let summary = 0;
let changed = 0;
const outputLines = fs.readFileSync(input, "utf8").trimEnd().split("\n").map((line) => {
  const record = JSON.parse(line);
  if (record.kind === "metadata") {
    if (record.oxide_profile_sha256 !== candidateProfile) process.exit(2);
    record.oxide_profile_sha256 = parentProfile;
    metadata += 1;
  } else if (record.kind === "result" && keys.has(`${record.path}\t${record.variant}`)) {
    if (record.outcome !== "pass" || record.actual_phase !== "parse" ||
        record.actual_type !== "SyntaxError") process.exit(3);
    record.outcome = "unsupported-module";
    record.actual_phase = "selection";
    record.actual_type = "ExecutionMode";
    record.detail = "missing execution capabilities: module";
    changed += 1;
  } else if (record.kind === "summary") {
    if (JSON.stringify(record.outcomes) !== expectedSummary) process.exit(4);
    record.outcomes = parentSummary;
    summary += 1;
  }
  return JSON.stringify(record);
});
if (metadata !== 1 || summary !== 1 || changed !== 67) process.exit(5);
fs.writeFileSync(output, `${outputLines.join("\n")}\n`);
NODE
}

check_file "$baseline" "$baseline_lines" "$baseline_sha256"
[[ "$(value schema)" == r3ed-a-test262-module-static-negative-global-v1 \
    && "$(value roots)" == 67 \
    && "$(value empty_feature_roots)" == 57 \
    && "$(value export_star_roots)" == 4 \
    && "$(value generator_roots)" == 3 \
    && "$(value let_roots)" == 1 \
    && "$(value let_const_roots)" == 1 \
    && "$(value new_target_roots)" == 1 \
    && "$(value request_roots)" == 13 \
    && "$(value block_list_flag_roots)" == 7 \
    && "$(value exclusion_canaries)" == 25 \
    && "$(value focused_variants)" == 67 \
    && "$(value profile_added_features)" == 0 \
    && "$(value profile_added_negatives)" == 67 \
    && "$(value full_variants)" == 102037 \
    && "$(value full_outcome_changed)" == 67 \
    && "$(value full_detail_only)" == 0 \
    && "$(value full_unchanged)" == 101970 \
    && "$(value full_pass_gains)" == 67 \
    && "$(value full_pass_regressions)" == 0 \
    && "$(value full_candidate_runnable)" == 68414 \
    && "$(value full_candidate_passes)" == 68362 ]] \
    || die 'R3ed-A baseline contract drifted'

check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
    "$(value predecessor_baseline_sha256)"
[[ "$(predecessor_value schema)" == r3ec-a-test262-module-decl-position-global-v1 \
    && "$(predecessor_value candidate_profile_sha256)" == "$(value parent_profile_sha256)" \
    && "$(predecessor_value full_candidate_runnable)" == "$(value full_parent_runnable)" \
    && "$(predecessor_value full_candidate_passes)" == "$(value full_parent_passes)" \
    && "$(predecessor_value full_candidate_tsv_sha256)" == "$(value full_parent_tsv_sha256)" \
    && "$(predecessor_value full_candidate_jsonl_sha256)" == "$(value full_parent_jsonl_sha256)" \
    && "$(predecessor_value full_candidate_summary)" == "$(value full_parent_summary)" \
    && "$(predecessor_value candidate_canonical_baseline_sha256)" \
        == "$(value parent_canonical_baseline_sha256)" \
    && "$(predecessor_value full_keys_sha256)" == "$(value full_keys_sha256)" ]] \
    || die 'R3ed-A does not checksum-bridge the frozen R3ec-A receipt'

parent_commit=$(value parent_commit)
candidate_commit=$(value candidate_commit)
for commit in "$parent_commit" "$candidate_commit"; do
    trusted_git -C "$root" cat-file -e "$commit^{commit}" 2>/dev/null \
        || die "authenticated R3ed-A commit is unavailable: $commit"
done
[[ "$(trusted_git -C "$root" rev-parse "$parent_commit^{tree}")" == "$(value parent_tree)" \
    && "$(trusted_git -C "$root" rev-parse "$candidate_commit^{tree}")" \
        == "$(value candidate_tree)" ]] \
    || die 'R3ed-A parent or candidate tree drifted'
[[ "$(trusted_git -C "$root" rev-parse "$candidate_commit^")" == "$parent_commit" ]] \
    || die 'R3ed-A admission commit is not a direct child of its frozen parent'
predecessor_candidate=$(predecessor_value candidate_commit)
[[ "$(trusted_git -C "$root" rev-parse "$predecessor_candidate^{tree}")" \
        == "$(predecessor_value candidate_tree)" ]] \
    || die 'R3ec-A predecessor candidate tree drifted'
trusted_git -C "$root" merge-base --is-ancestor \
    "$predecessor_candidate" "$parent_commit" \
    || die 'R3ec-A semantic candidate is not an ancestor of the R3ed-A parent layer'
trusted_git -C "$root" diff --quiet "$predecessor_candidate" "$parent_commit" -- src compat \
    || die 'semantic inputs drifted between R3ec-A and the R3ed-A parent layer'

trusted_git -C "$root" diff --quiet "$candidate_commit" -- \
    src compat "$generator" "$prepare_suite" "$oracle_builder" "$manifest" \
    "$ledger" "$requests" "$variants" "$negatives" "$exclusions" \
    "$provenance" "$quickjs_projection" \
    || die 'live R3ed-A semantic or authenticated evidence input drifted'
[[ -z "$(trusted_git -C "$root" ls-files --others --exclude-standard -- src compat)" ]] \
    || die 'untracked semantic input is outside the R3ed-A admission commit'

check_file "$profile" "$(value candidate_profile_lines)" \
    "$(value candidate_profile_sha256)"
check_file "$upstream" "$(value candidate_upstream_lines)" \
    "$(value candidate_upstream_sha256)"
[[ "$(toml_value quickjs version)" == "$(value quickjs)" \
    && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
    && "$(toml_value test262 commit)" == "$(value test262)" \
    && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
    && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256)" \
        == "$(value test262_metadata_sha256)" \
    && "$(toml_value test262 oxide_profile_sha256)" \
        == "$(value candidate_profile_sha256)" \
    && "$(toml_value test262 oxide_profile)" == "$profile" ]] \
    || die 'pinned upstream identity drifted'

trusted_git -C "$root" show "$parent_commit:$profile" >"$tmp/parent-profile.conf"
check_file "$tmp/parent-profile.conf" "$(value parent_profile_lines)" \
    "$(value parent_profile_sha256)"
for section in features audited-negative-tests execution host-agent-tests; do
    for role in parent candidate; do
        if [[ "$role" == parent ]]; then
            source_profile=$tmp/parent-profile.conf
        else
            source_profile=$profile
        fi
        profile_section "$section" "$source_profile" >"$tmp/$role-$section"
        sort -c "$tmp/$role-$section" \
            || die "R3ed-A $role profile section is not bytewise sorted: $section"
        [[ -z "$(uniq -d "$tmp/$role-$section")" ]] \
            || die "R3ed-A $role profile section contains duplicates: $section"
    done
done
cmp -s "$tmp/parent-features" "$tmp/candidate-features" \
    || die 'R3ed-A changed the feature set'
cmp -s "$tmp/parent-execution" "$tmp/candidate-execution" \
    || die 'R3ed-A changed execution policy'
cmp -s "$tmp/parent-host-agent-tests" "$tmp/candidate-host-agent-tests" \
    || die 'R3ed-A changed agent-host admission'
comm -23 "$tmp/parent-audited-negative-tests" \
    "$tmp/candidate-audited-negative-tests" >"$tmp/removed-negatives"
comm -13 "$tmp/parent-audited-negative-tests" \
    "$tmp/candidate-audited-negative-tests" >"$tmp/added-negatives"
[[ ! -s "$tmp/removed-negatives" ]] || die 'R3ed-A removed an audited negative'
diff -u "$negatives" "$tmp/added-negatives"
[[ "$(lines "$tmp/added-negatives")" == "$(value profile_added_negatives)" ]] \
    || die 'R3ed-A audited-negative delta count drifted'

for spec in \
    "$generator:$(value generator_lines):$(value generator_sha256)" \
    "$manifest:$(value manifest_lines):$(value manifest_sha256)" \
    "$ledger:$(value ledger_lines):$(value ledger_sha256)" \
    "$requests:$(value requests_lines):$(value requests_sha256)" \
    "$variants:$(value variants_lines):$(value variants_sha256)" \
    "$negatives:$(value negatives_lines):$(value negatives_sha256)" \
    "$exclusions:$(value exclusions_lines):$(value exclusions_sha256)" \
    "$provenance:$(value provenance_lines):$(value provenance_sha256)" \
    "$quickjs_projection:$(value quickjs_projection_lines):$(value quickjs_projection_sha256)" \
    "$candidate:$(value focused_candidate_tsv_lines):$(value focused_candidate_tsv_sha256)" \
    "${candidate%.tsv}.jsonl:$(value focused_candidate_jsonl_lines):$(value focused_candidate_jsonl_sha256)"; do
    IFS=: read -r path expected_lines expected_sha <<<"$spec"
    check_file "$path" "$expected_lines" "$expected_sha"
done

node "$generator" >"$tmp/generator.log"
grep -Fqx \
    'module-static-negative-a: roots=67 empty=57 export-star=4 generators=3 let=1 let-const=1 new-target=1 requests=13 block-list-flags=7 canaries=25' \
    "$tmp/generator.log" || die 'static parse-negative generator contract drifted'

sed '1d' "$variants" | cut -f1,2 >"$tmp/focused.keys"
[[ "$(lines "$tmp/focused.keys")" == 67 \
    && "$(sha "$tmp/focused.keys")" == "$(value focused_keys_sha256)" \
    && -z "$(uniq -d "$tmp/focused.keys")" ]] \
    || die 'R3ed-A focused key set drifted'

verify_focused_report() {
    local report=$1 json=${1%.tsv}.jsonl
    [[ "$(header "$report" oxide_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_summary "$report")" == "$(value focused_candidate_summary)" \
        && "$(computed_summary "$report")" == "$(value focused_candidate_summary)" \
        && "$(report_count pass "$report")" == 67 \
        && "$(report_runnable "$report")" == 67 ]] \
        || die "R3ed-A focused candidate outcome drifted: $report"
    report_keys "$report" >"$tmp/report.keys"
    diff -u "$tmp/focused.keys" "$tmp/report.keys"
    report_rows "$report" >"$tmp/report.rows"
    json_result_projection "$json" >"$tmp/report.json.rows"
    diff -u "$tmp/report.rows" "$tmp/report.json.rows"
}

verify_focused_report "$candidate"
reverse_tsv "$candidate" "$tmp/focused-parent.tsv" \
    "$(value focused_candidate_summary)" "$(value focused_parent_summary)"
reverse_jsonl "${candidate%.tsv}.jsonl" "$tmp/focused-parent.jsonl" \
    '{"pass":67}' '{"unsupported-module":67}'
check_file "$tmp/focused-parent.tsv" "$(value focused_parent_tsv_lines)" \
    "$(value focused_parent_tsv_sha256)"
check_file "$tmp/focused-parent.jsonl" "$(value focused_parent_jsonl_lines)" \
    "$(value focused_parent_jsonl_sha256)"

check_file "$canonical_baseline" "$(value candidate_canonical_baseline_lines)" \
    "$(value candidate_canonical_baseline_sha256)"
[[ "$(canonical_value schema)" == test262-canonical-classified-v2 \
    && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
    && "$(canonical_value variants)" == "$(value full_variants)" \
    && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
    && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value full_candidate_jsonl_sha256)" \
    && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
    || die 'canonical Test262 baseline is not the R3ed-A candidate'

if [[ "$mode" == check ]]; then
    echo 'R3ed-A module static parse-negative receipt inputs are authenticated.'
    exit 0
fi

if [[ -n "$runner_override" ]]; then
    runner=$runner_override
    [[ "$runner" == /* ]] || die 'TEST262_RUNNER must be an absolute path'
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
fi
[[ -f "$runner" && -x "$runner" && ! -L "$runner" ]] \
    || die 'R3ed-A runner is not an executable regular file'

suite=$("$script_dir/prepare-test262.sh")
[[ -n "$suite" && "$suite" == /* && -d "$suite/test" && ! -L "$suite" ]] \
    || die 'prepare-test262.sh did not return one authenticated suite path'
source_dir=$(CDPATH='' cd -- "$suite/.." && pwd)
[[ "$(trusted_git -C "$suite" rev-parse HEAD)" == "$(value test262)" \
    && "$(sha "$source_dir/tests/test262.patch")" \
        == "$(value test262_patch_sha256)" \
    && "$(sha "$source_dir/test262.conf")" == "$(value test262_config_sha256)" ]] \
    || die 'prepared Test262/QuickJS inputs drifted'
quickjs_source=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
[[ "$quickjs_source" == "$source_dir" && -f "$quickjs_source/run-test262" \
    && -x "$quickjs_source/run-test262" && ! -L "$quickjs_source/run-test262" ]] \
    || die 'authenticated QuickJS Test262 oracle path drifted'

metadata_records=$tmp/test262-metadata.bin
"$runner" --suite "$suite" --validate-metadata "$metadata_records" \
    >"$tmp/metadata-audit.log"
check_file "$metadata_records" "$(value test262_metadata_records)" \
    "$(value test262_metadata_sha256)"
grep -Fqx "Test262 metadata: files=$(value test262_metadata_records)" \
    "$tmp/metadata-audit.log" \
    || die 'Test262 metadata audit did not cover the pinned checkout'

run_focused_candidate() {
    local output=$1
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode "$(value mode)" --workers "$workers" \
        --timeout-ms "$(value timeout_ms)" --allow-failures
}
for replay in a b; do
    run_focused_candidate "$tmp/focused-$replay.tsv"
    verify_focused_report "$tmp/focused-$replay.tsv"
    cmp -s "$candidate" "$tmp/focused-$replay.tsv" \
        || die "R3ed-A focused TSV replay $replay drifted"
    cmp -s "${candidate%.tsv}.jsonl" "$tmp/focused-$replay.jsonl" \
        || die "R3ed-A focused JSONL replay $replay drifted"
done
cmp -s "$tmp/focused-a.tsv" "$tmp/focused-b.tsv" \
    || die 'R3ed-A focused TSV replays differ'
cmp -s "$tmp/focused-a.jsonl" "$tmp/focused-b.jsonl" \
    || die 'R3ed-A focused JSONL replays differ'

quickjs_args=(-m -c test262.conf -a -T 1 -f)
while IFS= read -r relative; do
    quickjs_args+=("test262/$relative")
done <"$manifest"
for replay in a b; do
    log=$tmp/quickjs-$replay.log
    (cd "$quickjs_source" && ./run-test262 "${quickjs_args[@]}") >"$log" 2>&1 \
        || die "pinned QuickJS rejected the R3ed-A cohort: $log"
    ! grep -Fq 'FAILED' "$log" \
        || die "pinned QuickJS reported a failed R3ed-A test: $log"
    [[ "$(grep -Fxc 'Average memory statistics for 67 tests:' "$log")" == 1 ]] \
        || die "pinned QuickJS did not execute exactly 67 R3ed-A tests: $log"
    {
        awk '/^test262\.conf:.* ignoring testdir=/{print}' "$log"
        awk '/^SyntaxError:/{count[$0]++} END{for(line in count) print count[line], line}' \
            "$log" | sort -k3
        awk '/^Average memory statistics for [0-9]+ tests:/{print}' "$log"
    } >"$log.projection"
    diff -u "$quickjs_projection" "$log.projection"
done
cmp -s "$tmp/quickjs-a.log.projection" "$tmp/quickjs-b.log.projection" \
    || die 'pinned QuickJS semantic projections are not byte-identical'

if [[ "$mode" != full ]]; then
    echo 'R3ed-A focused gate passed: Oxide 67/67 twice, QuickJS 67/67 twice, parent 67 unsupported, adjacent frontiers closed.'
    exit 0
fi

for full_path in "$full_candidate_a" "$full_candidate_b"; do
    case $full_path in
        /*) ;;
        *) die 'R3ed-A full report paths must be absolute' ;;
    esac
done
[[ "$full_candidate_a" != "$full_candidate_b" \
    && "${full_candidate_a%.tsv}.jsonl" != "${full_candidate_b%.tsv}.jsonl" ]] \
    || die 'R3ed-A full candidate A/B paths must be distinct'
if [[ "$reuse_full_reports" == true \
    && -z ${TEST262_FULL_REPORT_DIR+x} \
    && -z ${TEST262_FULL_CANDIDATE_A_REPORT+x} \
    && -z ${TEST262_FULL_CANDIDATE_B_REPORT+x} ]]; then
    die 'TEST262_REUSE_FULL_REPORTS=true requires explicit full report paths'
fi
for full_path in "$full_candidate_a" "${full_candidate_a%.tsv}.jsonl" \
    "$full_candidate_b" "${full_candidate_b%.tsv}.jsonl"; do
    for protected in "$root/$candidate" "$root/${candidate%.tsv}.jsonl" \
        "$root/$canonical_baseline" "$root/$baseline" "$root/$predecessor_baseline" \
        "$root/$profile" "$root/$upstream"; do
        [[ "$full_path" != "$protected" ]] \
            || die "R3ed-A full report path aliases a protected input: $full_path"
        if [[ -e "$full_path" && "$full_path" -ef "$protected" ]]; then
            die "R3ed-A existing full report aliases a protected input inode: $full_path"
        fi
    done
done
if [[ "$reuse_full_reports" == true ]]; then
    full_receipt_kind=authenticated-reuse
    for report in "$full_candidate_a" "$full_candidate_b"; do
        [[ -f "$report" && -f "${report%.tsv}.jsonl" ]] \
            || die "requested reusable R3ed-A full report is missing: $report"
    done
else
    full_receipt_kind=live-rerun
    for report in "$full_candidate_a" "$full_candidate_b"; do
        mkdir -p "$(dirname "$report")"
        rm -f -- "$report" "${report%.tsv}.jsonl"
        "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
            --oxide-profile "$profile" --all --report "$report" \
            --mode "$(value mode)" --workers "$full_workers" \
            --timeout-ms "$(value timeout_ms)" --allow-failures
    done
fi

if [[ "$full_candidate_a" -ef "$full_candidate_b" \
    || "${full_candidate_a%.tsv}.jsonl" -ef "${full_candidate_b%.tsv}.jsonl" ]]; then
    die 'R3ed-A full candidate replays must be distinct files'
fi
for full_path in "$full_candidate_a" "${full_candidate_a%.tsv}.jsonl" \
    "$full_candidate_b" "${full_candidate_b%.tsv}.jsonl"; do
    for protected in "$root/$candidate" "$root/${candidate%.tsv}.jsonl" \
        "$root/$canonical_baseline" "$root/$baseline" "$root/$predecessor_baseline" \
        "$root/$profile" "$root/$upstream"; do
        if [[ "$full_path" -ef "$protected" ]]; then
            die "R3ed-A full report aliases a protected input inode: $full_path"
        fi
    done
done

verify_full_candidate() {
    local report=$1 json=${1%.tsv}.jsonl
    check_file "$report" "$(value full_report_lines)" \
        "$(value full_candidate_tsv_sha256)"
    check_file "$json" "$(value full_jsonl_lines)" \
        "$(value full_candidate_jsonl_sha256)"
    [[ "$(header "$report" oxide_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_summary "$report")" == "$(value full_candidate_summary)" \
        && "$(computed_summary "$report")" == "$(value full_candidate_summary)" \
        && "$(report_runnable "$report")" == "$(value full_candidate_runnable)" \
        && "$(report_count pass "$report")" == "$(value full_candidate_passes)" ]] \
        || die "R3ed-A full candidate outcome drifted: $report"
    report_keys "$report" >"$tmp/full.keys"
    [[ "$(lines "$tmp/full.keys")" == "$(value full_variants)" \
        && "$(sha "$tmp/full.keys")" == "$(value full_keys_sha256)" \
        && -z "$(uniq -d "$tmp/full.keys")" ]] \
        || die "R3ed-A full key set drifted: $report"
    report_rows "$report" >"$tmp/full.rows"
    json_result_projection "$json" >"$tmp/full.json.rows"
    diff -u "$tmp/full.rows" "$tmp/full.json.rows"
    rows_for_keys "$tmp/focused.keys" "$report" >"$tmp/full.focused.rows"
    report_rows "$candidate" >"$tmp/candidate.focused.rows"
    diff -u "$tmp/candidate.focused.rows" "$tmp/full.focused.rows"
}

verify_full_candidate "$full_candidate_a"
verify_full_candidate "$full_candidate_b"
cmp -s "$full_candidate_a" "$full_candidate_b" \
    || die 'R3ed-A full TSV replays are not byte-identical'
cmp -s "${full_candidate_a%.tsv}.jsonl" "${full_candidate_b%.tsv}.jsonl" \
    || die 'R3ed-A full JSONL replays are not byte-identical'

reverse_tsv "$full_candidate_a" "$tmp/full-parent.tsv" \
    "$(value full_candidate_summary)" "$(value full_parent_summary)"
reverse_jsonl "${full_candidate_a%.tsv}.jsonl" "$tmp/full-parent.jsonl" \
    "$(value full_candidate_summary_json)" "$(value full_parent_summary_json)"
check_file "$tmp/full-parent.tsv" "$(value full_report_lines)" \
    "$(value full_parent_tsv_sha256)"
check_file "$tmp/full-parent.jsonl" "$(value full_jsonl_lines)" \
    "$(value full_parent_jsonl_sha256)"

printf 'R3ed-A canonical successor passed (%s): 67 static module parse-negative gains, zero regressions, and two distinct byte-identical full reports.\n' \
    "$full_receipt_kind"
