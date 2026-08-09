#!/usr/bin/env bash
# Reproduce and authenticate the R3eb-A default/import.meta canonical successor.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

baseline=tests/test262-import-meta-a-global-baseline.txt
parent_baseline=tests/test262-module-namespace-a-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
default_generator=scripts/generate-test262-module-default-a.mjs
default_manifest=tests/test262-module-default-a.txt
default_sources=tests/test262-module-default-a-sources.txt
default_edges=tests/test262-module-default-a-edges.tsv
default_closures=tests/test262-module-default-a-closures.tsv
default_ledger=tests/test262-module-default-a-ledger.tsv
default_negatives=tests/test262-module-default-a-negatives.txt
default_exclusions=tests/test262-module-default-a-exclusions.tsv
import_generator=scripts/generate-test262-import-meta-a.mjs
import_manifest=tests/test262-import-meta-a.txt
import_sources=tests/test262-import-meta-a-sources.txt
import_module_roots=tests/test262-import-meta-a-module-roots.txt
import_script_roots=tests/test262-import-meta-a-script-roots.txt
import_edges=tests/test262-import-meta-a-edges.tsv
import_closures=tests/test262-import-meta-a-closures.tsv
import_ledger=tests/test262-import-meta-a-ledger.tsv
import_variants=tests/test262-import-meta-a-variants.tsv
import_negatives=tests/test262-import-meta-a-negatives.txt
import_exclusions=tests/test262-import-meta-a-exclusions.tsv
candidate=tests/test262-import-meta-a-global-candidate.tsv
workers=${TEST262_WORKERS:-4}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
full_dir=${TEST262_FULL_REPORT_DIR:-$root/target/test262-import-meta-a-global-full}
full_candidate_a=${TEST262_FULL_CANDIDATE_A_REPORT:-$full_dir/candidate-a.tsv}
full_candidate_b=${TEST262_FULL_CANDIDATE_B_REPORT:-$full_dir/candidate-b.tsv}
runner_override=${TEST262_RUNNER:-}
baseline_lines=108
baseline_sha256=b1b8065a99f6fffd61b3aaa2feaa183b173cd1f9ab2561ad5771def99468a615

usage() {
    printf 'usage: %s [--check|--focused|--full]\n' "${0##*/}"
    printf '  --check    authenticate frozen inputs and the focused receipt\n'
    printf '  --focused  rerun the 65-variant candidate twice (default)\n'
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
parent_value() { value_from "$parent_baseline" "$1"; }
canonical_value() { value_from "$canonical_baseline" "$1"; }
check_file() {
    [[ -f "$1" && ! -L "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated R3eb-A input drifted: $1"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() {
    report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort
}
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
json_rows_for_keys() {
    local keys=$1 report=$2
    node - "$keys" "$report" <<'NODE'
const fs = require("node:fs");
const keys = new Set(
  fs.readFileSync(process.argv[2], "utf8").trimEnd().split("\n"),
);
for (const line of fs.readFileSync(process.argv[3], "utf8").trimEnd().split("\n")) {
  const record = JSON.parse(line);
  if (record.kind === "result" && keys.has(record.path + "\t" + record.variant)) {
    process.stdout.write(line + "\n");
  }
}
NODE
}
json_result_projection() {
    local report=$1
    node - "$report" <<'NODE'
const fs = require("node:fs");
const fields = [
  "path",
  "variant",
  "flags",
  "features",
  "expected_phase",
  "expected_type",
  "outcome",
  "actual_phase",
  "actual_type",
  "detail",
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
    process.stdout.write(
      fields.map((field) => escapeField(record[field])).join("\t") + "\n",
    );
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

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3eb-a.XXXXXX")
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

full_receipt_pending() {
    local field pending=0 complete=0
    for field in candidate_canonical_baseline_sha256 \
        full_candidate_tsv_sha256 full_candidate_jsonl_sha256 \
        full_candidate_replay_status; do
        if [[ "$(value "$field")" == PENDING ]]; then
            pending=$((pending + 1))
        else
            complete=$((complete + 1))
        fi
    done
    [[ "$pending" == 0 || "$complete" == 0 ]] \
        || die 'R3eb-A full receipt is only partially populated'
    [[ "$pending" != 0 ]]
}

check_file "$baseline" "$baseline_lines" "$baseline_sha256"
[[ "$(value schema)" == r3eb-a-test262-import-meta-global-v1 \
    && "$(value union_roots)" == 60 \
    && "$(value union_variants)" == 65 \
    && "$(value full_variants)" == 102037 \
    && "$(value full_outcome_changed)" == 64 \
    && "$(value full_detail_only)" == 0 \
    && "$(value full_unchanged)" == 101973 \
    && "$(value full_pass_gains)" == 64 \
    && "$(value full_pass_regressions)" == 0 \
    && "$(value full_candidate_runnable)" == 68261 \
    && "$(value full_candidate_passes)" == 68209 ]] \
    || die 'R3eb-A baseline contract drifted'

check_file "$parent_baseline" "$(value parent_baseline_lines)" \
    "$(value parent_baseline_sha256)"
[[ "$(value parent_gate)" == scripts/test-test262-module-namespace-a.sh \
    && -x "$(value parent_gate)" \
    && "$(parent_value schema)" == r3dz-a-test262-module-namespace-v1 \
    && "$(parent_value candidate_commit)" == "$(value parent_commit)" \
    && "$(parent_value candidate_tree)" == "$(value parent_tree)" \
    && "$(parent_value candidate_profile_sha256)" == "$(value parent_profile_sha256)" \
    && "$(parent_value full_candidate_runnable)" == "$(value full_parent_runnable)" \
    && "$(parent_value full_candidate_passes)" == "$(value full_parent_passes)" \
    && "$(parent_value full_candidate_tsv_sha256)" == "$(value full_parent_tsv_sha256)" \
    && "$(parent_value full_candidate_jsonl_sha256)" == "$(value full_parent_jsonl_sha256)" \
    && "$(parent_value full_candidate_summary)" == "$(value full_parent_summary)" ]] \
    || die 'R3eb-A does not checksum-bridge the frozen R3dz-A receipt'

parent_commit=$(value parent_commit)
candidate_commit=$(value candidate_commit)
for commit in "$parent_commit" "$candidate_commit"; do
    trusted_git -C "$root" cat-file -e "$commit^{commit}" 2>/dev/null \
        || die "authenticated R3eb-A commit is unavailable: $commit"
done
[[ "$(trusted_git -C "$root" rev-parse "$parent_commit^{tree}")" == "$(value parent_tree)" \
    && "$(trusted_git -C "$root" rev-parse "$candidate_commit^{tree}")" \
        == "$(value candidate_tree)" ]] \
    || die 'R3eb-A parent or candidate tree drifted'

# The receipt is bound to the complete Rust source tree at the admission
# commit. New documentation and this successor gate may be layered on top, but
# semantic or harness changes require a new canonical receipt.
trusted_git -C "$root" diff --quiet "$candidate_commit" -- src \
    || die 'live Rust source drifted from the R3eb-A admission commit'
[[ -z "$(trusted_git -C "$root" ls-files --others --exclude-standard -- src)" ]] \
    || die 'untracked Rust source is outside the R3eb-A admission commit'
for rel in "$profile" "$upstream" "$default_generator" "$import_generator" \
    "$(value parent_gate)"; do
    trusted_git -C "$root" show "$candidate_commit:$rel" >"$tmp/committed"
    cmp -s "$tmp/committed" "$rel" \
        || die "live R3eb-A harness input drifted from $candidate_commit: $rel"
done

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
    && "$(toml_value test262 oxide_profile)" == "$profile" \
    && "$(toml_value test262 oxide_profile_sha256)" \
        == "$(value candidate_profile_sha256)" ]] \
    || die 'pinned upstream identity drifted'

check_file "$default_generator" "$(value default_generator_lines)" \
    "$(value default_generator_sha256)"
check_file "$default_manifest" "$(value default_manifest_lines)" \
    "$(value default_manifest_sha256)"
check_file "$default_sources" "$(value default_sources_lines)" \
    "$(value default_sources_sha256)"
check_file "$default_edges" "$(value default_edges_lines)" \
    "$(value default_edges_sha256)"
check_file "$default_closures" "$(value default_closures_lines)" \
    "$(value default_closures_sha256)"
check_file "$default_ledger" "$(value default_ledger_lines)" \
    "$(value default_ledger_sha256)"
check_file "$default_negatives" "$(value default_negatives_lines)" \
    "$(value default_negatives_sha256)"
check_file "$default_exclusions" "$(value default_exclusions_lines)" \
    "$(value default_exclusions_sha256)"
check_file "$import_generator" "$(value import_meta_generator_lines)" \
    "$(value import_meta_generator_sha256)"
check_file "$import_manifest" "$(value import_meta_manifest_lines)" \
    "$(value import_meta_manifest_sha256)"
check_file "$import_sources" "$(value import_meta_sources_lines)" \
    "$(value import_meta_sources_sha256)"
check_file "$import_module_roots" "$(value import_meta_module_roots_lines)" \
    "$(value import_meta_module_roots_sha256)"
check_file "$import_script_roots" "$(value import_meta_script_roots_lines)" \
    "$(value import_meta_script_roots_sha256)"
check_file "$import_edges" "$(value import_meta_edges_lines)" \
    "$(value import_meta_edges_sha256)"
check_file "$import_closures" "$(value import_meta_closures_lines)" \
    "$(value import_meta_closures_sha256)"
check_file "$import_ledger" "$(value import_meta_ledger_lines)" \
    "$(value import_meta_ledger_sha256)"
check_file "$import_variants" "$(value import_meta_variants_lines)" \
    "$(value import_meta_variants_sha256)"
check_file "$import_negatives" "$(value import_meta_negatives_lines)" \
    "$(value import_meta_negatives_sha256)"
check_file "$import_exclusions" "$(value import_meta_exclusions_lines)" \
    "$(value import_meta_exclusions_sha256)"

node "$default_generator" >"$tmp/default-generator.log"
node "$import_generator" >"$tmp/import-generator.log"
grep -Fqx 'module-default-a: roots=38 sources=58 rooted_edges=45 negatives=5' \
    "$tmp/default-generator.log" \
    || die 'default module generator did not authenticate its complete cohort'
grep -Fqx \
    'import-meta-a: roots=22 sources=23 module_roots=17 script_roots=5 rooted_edges=1 variants=27 canaries=3' \
    "$tmp/import-generator.log" \
    || die 'import.meta generator did not authenticate its complete cohort'

trusted_git -C "$root" show "$parent_commit:$profile" >"$tmp/parent-profile.conf"
check_file "$tmp/parent-profile.conf" "$(value parent_profile_lines)" \
    "$(value parent_profile_sha256)"
for section in features audited-negative-tests; do
    profile_section "$section" "$tmp/parent-profile.conf" \
        | sort >"$tmp/parent-$section"
    profile_section "$section" "$profile" | sort >"$tmp/candidate-$section"
    comm -23 "$tmp/parent-$section" "$tmp/candidate-$section" \
        >"$tmp/removed-$section"
    [[ ! -s "$tmp/removed-$section" ]] \
        || die "R3eb-A profile removed an authenticated $section entry"
    comm -13 "$tmp/parent-$section" "$tmp/candidate-$section" \
        >"$tmp/added-$section"
done
printf '%s\n' 'import.meta' >"$tmp/expected-added-features"
{
    cat "$default_negatives"
    cat "$import_negatives"
} | sort >"$tmp/expected-added-negatives"
diff -u "$tmp/expected-added-features" "$tmp/added-features"
diff -u "$tmp/expected-added-negatives" "$tmp/added-audited-negative-tests"
[[ "$(lines "$tmp/added-features")" == "$(value profile_added_features)" \
    && "$(lines "$tmp/added-audited-negative-tests")" \
        == "$(value profile_added_negatives)" ]] \
    || die 'R3eb-A profile transition count drifted'

{
    cat "$default_manifest"
    cat "$import_manifest"
} | sort >"$tmp/union-manifest"
[[ "$(lines "$tmp/union-manifest")" == "$(value union_roots)" \
    && "$(sha "$tmp/union-manifest")" == "$(value union_manifest_sha256)" \
    && -z "$(uniq -d "$tmp/union-manifest")" ]] \
    || die 'R3eb-A 60-root union drifted'

sed $'s/$/\tsloppy/' "$default_manifest" >"$tmp/default-keys"
sed '1d' "$import_variants" | cut -f1,2 >"$tmp/import-keys"
{
    cat "$tmp/default-keys"
    cat "$tmp/import-keys"
} | sort >"$tmp/union-keys"
[[ "$(lines "$tmp/union-keys")" == "$(value union_variants)" \
    && "$(sha "$tmp/union-keys")" == "$(value union_keys_sha256)" \
    && -z "$(uniq -d "$tmp/union-keys")" ]] \
    || die 'R3eb-A 65-key union drifted'

already_pass_path=$(value historical_already_pass_path)
printf '%s\tsloppy\n' "$already_pass_path" >"$tmp/already-pass.keys"
grep -Fvx "$already_pass_path" "$default_manifest" \
    | sed $'s/$/\tsloppy/' >"$tmp/default-transition.keys"
awk -F'\t' 'NR>1&&$3=="module"{print $1 "\t" $2}' "$import_variants" \
    >"$tmp/import-module-transition.keys"
awk -F'\t' 'NR>1&&$3=="script"{print $1 "\t" $2}' "$import_variants" \
    >"$tmp/import-script-transition.keys"
{
    cat "$tmp/default-transition.keys"
    cat "$tmp/import-module-transition.keys"
} | sort >"$tmp/module-transition.keys"
sort "$tmp/import-script-transition.keys" >"$tmp/feature-transition.keys"
{
    cat "$tmp/module-transition.keys"
    cat "$tmp/feature-transition.keys"
} | sort >"$tmp/outcome-transition.keys"
{
    cat "$tmp/outcome-transition.keys"
    cat "$tmp/already-pass.keys"
} | sort >"$tmp/reconstructed-union.keys"
diff -u "$tmp/union-keys" "$tmp/reconstructed-union.keys"
[[ "$(lines "$tmp/module-transition.keys")" == 54 \
    && "$(lines "$tmp/feature-transition.keys")" == 10 \
    && "$(lines "$tmp/outcome-transition.keys")" \
        == "$(value full_outcome_changed)" \
    && "$(sha "$tmp/outcome-transition.keys")" \
        == "$(value full_outcome_keys_sha256)" ]] \
    || die 'R3eb-A historical transition partition drifted'

reverse_tsv() {
    local input=$1 output=$2 candidate_summary=$3 parent_summary=$4
    awk -F'\t' -v OFS='\t' \
        -v module_file="$tmp/module-transition.keys" \
        -v feature_file="$tmp/feature-transition.keys" \
        -v already_file="$tmp/already-pass.keys" \
        -v candidate_profile="$(value candidate_profile_sha256)" \
        -v parent_profile="$(value parent_profile_sha256)" \
        -v candidate_summary="$candidate_summary" \
        -v parent_summary="$parent_summary" '
        BEGIN {
            while ((getline row < module_file) > 0) {
                split(row, fields, "\t")
                module_key[fields[1] FS fields[2]]=1
            }
            close(module_file)
            while ((getline row < feature_file) > 0) {
                split(row, fields, "\t")
                feature_key[fields[1] FS fields[2]]=1
            }
            close(feature_file)
            while ((getline row < already_file) > 0) {
                split(row, fields, "\t")
                already_key[fields[1] FS fields[2]]=1
            }
            close(already_file)
        }
        /^# oxide_profile_sha256=/ {
            if (index($0,candidate_profile)==0) exit 2
            sub(candidate_profile,parent_profile)
            headers++
            print
            next
        }
        /^# summary / {
            if ($0!="# summary " candidate_summary) exit 3
            print "# summary " parent_summary
            summaries++
            next
        }
        !/^#/ && !($1=="path"&&$2=="variant") {
            key=$1 FS $2
            if (key in module_key) {
                if ($7!="pass") exit 4
                $7="unsupported-module"
                $8="selection"
                $9="ExecutionMode"
                $10="missing execution capabilities: module"
                modules++
            } else if (key in feature_key) {
                if ($7!="pass") exit 5
                $7="unsupported-feature"
                $8="selection"
                $9="EngineCapability"
                $10="quickjs-oxide does not declare Test262 feature support: import.meta"
                features++
            } else if (key in already_key) {
                if ($7!="pass") exit 6
                already++
            }
        }
        {print}
        END {
            if (headers!=1||summaries!=1||modules!=54||features!=10||already!=1) {
                exit 7
            }
        }
    ' "$input" >"$output" \
        || die "could not reverse R3eb-A TSV into R3dz-A: $input"
}

reverse_jsonl() {
    local input=$1 output=$2 parent_summary_json=$3
    awk -v module_file="$tmp/module-transition.keys" \
        -v feature_file="$tmp/feature-transition.keys" \
        -v already_file="$tmp/already-pass.keys" \
        -v candidate_profile="$(value candidate_profile_sha256)" \
        -v parent_profile="$(value parent_profile_sha256)" \
        -v parent_summary_json="$parent_summary_json" '
        BEGIN {
            while ((getline row < module_file) > 0) {
                split(row, fields, "\t")
                module_key[fields[1] SUBSEP fields[2]]=1
            }
            close(module_file)
            while ((getline row < feature_file) > 0) {
                split(row, fields, "\t")
                feature_key[fields[1] SUBSEP fields[2]]=1
            }
            close(feature_file)
            while ((getline row < already_file) > 0) {
                split(row, fields, "\t")
                already_key[fields[1] SUBSEP fields[2]]=1
            }
            close(already_file)
        }
        /^\{"kind":"metadata",/ {
            if (index($0,candidate_profile)==0) exit 2
            sub(candidate_profile,parent_profile)
            metadata++
            print
            next
        }
        /^\{"kind":"summary",/ {
            print parent_summary_json
            summaries++
            next
        }
        /^\{"kind":"result",/ {
            line=$0
            start=index(line,"\"path\":\"")
            if (!start) exit 3
            rest=substr(line,start+8)
            finish=index(rest,"\"")
            if (!finish) exit 3
            path=substr(rest,1,finish-1)
            start=index(line,"\"variant\":\"")
            if (!start) exit 3
            rest=substr(line,start+11)
            finish=index(rest,"\"")
            if (!finish) exit 3
            variant=substr(rest,1,finish-1)
            key=path SUBSEP variant
            if (key in module_key) {
                replacement="\"outcome\":\"unsupported-module\"," \
                    "\"actual_phase\":\"selection\"," \
                    "\"actual_type\":\"ExecutionMode\"," \
                    "\"detail\":\"missing execution capabilities: module\"}"
                if (!sub(/"outcome":"pass","actual_phase":.*}$/,replacement,line)) exit 4
                modules++
            } else if (key in feature_key) {
                replacement="\"outcome\":\"unsupported-feature\"," \
                    "\"actual_phase\":\"selection\"," \
                    "\"actual_type\":\"EngineCapability\"," \
                    "\"detail\":\"quickjs-oxide does not declare Test262 feature support: import.meta\"}"
                if (!sub(/"outcome":"pass","actual_phase":.*}$/,replacement,line)) exit 5
                features++
            } else if (key in already_key) {
                if (index(line,"\"outcome\":\"pass\"")==0) exit 6
                already++
            }
            print line
            next
        }
        {print}
        END {
            if (metadata!=1||summaries!=1||modules!=54||features!=10||already!=1) {
                exit 7
            }
        }
    ' "$input" >"$output" \
        || die "could not reverse R3eb-A JSONL into R3dz-A: $input"
}

verify_report_header() {
    local report=$1 expected_profile=$2 expected_rows=$3 expected_summary=$4
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" \
            == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" \
            == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" \
            == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$expected_profile" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
            == "$expected_rows" \
        && "$(report_summary "$report")" == "$expected_summary" \
        && "$(computed_summary "$report")" == "$expected_summary" ]] \
        || die "R3eb-A classified report contract drifted: $report"
}

check_file "$candidate" "$(value focused_candidate_tsv_lines)" \
    "$(value focused_candidate_tsv_sha256)"
check_file "${candidate%.tsv}.jsonl" "$(value focused_candidate_jsonl_lines)" \
    "$(value focused_candidate_jsonl_sha256)"
verify_report_header "$candidate" "$(value candidate_profile_sha256)" \
    "$(value union_variants)" "$(value focused_candidate_summary)"
report_keys "$candidate" >"$tmp/focused-candidate.keys"
diff -u "$tmp/union-keys" "$tmp/focused-candidate.keys"
[[ "$(report_count pass "$candidate")" == "$(value union_variants)" \
    && "$(report_runnable "$candidate")" == "$(value union_variants)" ]] \
    || die 'R3eb-A focused candidate is not exactly 65 runnable passes'
awk '
    /^\{"kind":"metadata",/{metadata++}
    /^\{"kind":"result",/{results++}
    /^\{"kind":"summary",/{summaries++}
    END{if(metadata!=1||results!=65||summaries!=1)exit 1}
' "${candidate%.tsv}.jsonl" \
    || die 'R3eb-A focused candidate JSONL shape drifted'
report_rows "$candidate" >"$tmp/focused-candidate.tsv-results"
json_result_projection "${candidate%.tsv}.jsonl" \
    >"$tmp/focused-candidate.json-results"
diff -u "$tmp/focused-candidate.tsv-results" \
    "$tmp/focused-candidate.json-results"
json_rows_for_keys "$tmp/union-keys" "${candidate%.tsv}.jsonl" \
    >"$tmp/focused-candidate.keyed-json-results"
awk '/^\{"kind":"result",/' "${candidate%.tsv}.jsonl" \
    >"$tmp/focused-candidate.all-json-results"
diff -u "$tmp/focused-candidate.all-json-results" \
    "$tmp/focused-candidate.keyed-json-results"

focused_parent=$tmp/focused-parent.tsv
focused_parent_json=${focused_parent%.tsv}.jsonl
reverse_tsv "$candidate" "$focused_parent" \
    "$(value focused_candidate_summary)" "$(value focused_parent_summary)"
reverse_jsonl "${candidate%.tsv}.jsonl" "$focused_parent_json" \
    '{"kind":"summary","outcomes":{"pass":1,"unsupported-feature":10,"unsupported-module":54}}'
check_file "$focused_parent" "$(value focused_parent_tsv_lines)" \
    "$(value focused_parent_tsv_sha256)"
check_file "$focused_parent_json" "$(value focused_parent_jsonl_lines)" \
    "$(value focused_parent_jsonl_sha256)"
verify_report_header "$focused_parent" "$(value parent_profile_sha256)" \
    "$(value union_variants)" "$(value focused_parent_summary)"

if full_receipt_pending; then
    full_pending=true
    check_file "$canonical_baseline" 8 "$(value parent_canonical_baseline_sha256)"
    [[ "$(canonical_value tsv_sha256)" == "$(value full_parent_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value full_parent_jsonl_sha256)" \
        && "$(canonical_value runnable)" == "$(value full_parent_runnable)" \
        && "$(canonical_value passes)" == "$(value full_parent_passes)" \
        && "$(canonical_value summary)" == "$(value full_parent_summary)" ]] \
        || die 'pending R3eb-A receipt is not anchored to the R3dz-A canonical baseline'
else
    full_pending=false
    [[ "$(value full_candidate_replay_status)" == passed-twice ]] \
        || die 'completed R3eb-A receipt does not certify two full replays'
    check_file "$canonical_baseline" 8 \
        "$(value candidate_canonical_baseline_sha256)"
    [[ "$(canonical_value tsv_sha256)" == "$(value full_candidate_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" \
            == "$(value full_candidate_jsonl_sha256)" \
        && "$(canonical_value runnable)" == "$(value full_candidate_runnable)" \
        && "$(canonical_value passes)" == "$(value full_candidate_passes)" \
        && "$(canonical_value summary)" == "$(value full_candidate_summary)" ]] \
        || die 'canonical Test262 baseline is not the admitted R3eb-A candidate'
fi

if [[ "$mode" == check ]]; then
    if [[ "$full_pending" == true ]]; then
        echo 'R3eb-A focused evidence authenticated: 65/65 current passes; exact R3dz parent is 1 pass, 10 feature gaps, and 54 module gaps; full receipt PENDING'
    else
        echo 'R3eb-A evidence authenticated: focused 65/65 and completed canonical successor'
    fi
    exit 0
fi

if [[ -n "$runner_override" ]]; then
    runner=$runner_override
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
    && "$(sha "$source_dir/tests/test262.patch")" \
        == "$(value test262_patch_sha256)" \
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

run_focused_candidate() {
    local output=$1
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$tmp/union-manifest" \
        --report "$output" --mode "$(value mode)" --workers "$workers" \
        --timeout-ms "$(value timeout_ms)"
}

focused_replay_a=$tmp/focused-candidate-a.tsv
focused_replay_b=$tmp/focused-candidate-b.tsv
run_focused_candidate "$focused_replay_a"
run_focused_candidate "$focused_replay_b"
diff -u "$candidate" "$focused_replay_a"
diff -u "${candidate%.tsv}.jsonl" "${focused_replay_a%.tsv}.jsonl"
if ! cmp -s "$focused_replay_a" "$focused_replay_b" \
    || ! cmp -s "${focused_replay_a%.tsv}.jsonl" \
        "${focused_replay_b%.tsv}.jsonl"; then
    die 'R3eb-A focused candidate replays are not byte-identical'
fi

if [[ "$mode" == focused ]]; then
    echo 'R3eb-A focused gate passed: current candidate 65/65 twice; exact R3dz parent reverse verified'
    exit 0
fi

for full_path in "$full_candidate_a" "$full_candidate_b"; do
    case $full_path in
        /*) ;;
        *) die 'R3eb-A full report paths must be absolute' ;;
    esac
done
if [[ "$reuse_full_reports" == true \
    && -z ${TEST262_FULL_REPORT_DIR+x} \
    && -z ${TEST262_FULL_CANDIDATE_A_REPORT+x} \
    && -z ${TEST262_FULL_CANDIDATE_B_REPORT+x} ]]; then
    die 'TEST262_REUSE_FULL_REPORTS=true requires explicit full report paths'
fi

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

candidate_full_json_summary='{"kind":"summary","outcomes":{"fail-parse":7,"fail-runtime":43,"pass":68209,"skipped-config-exclude":6700,"skipped-feature":11775,"timeout":2,"unsupported-feature":11338,"unsupported-module":571,"unsupported-negative-provenance":3392}}'

verify_full_candidate() {
    local report=$1 json label
    json=${report%.tsv}.jsonl
    label=$(basename "${report%.tsv}")
    [[ -f "$report" && ! -L "$report" \
        && -f "$json" && ! -L "$json" \
        && "$(lines "$report")" == "$(value full_report_lines)" \
        && "$(lines "$json")" == "$(value full_jsonl_lines)" ]] \
        || die "R3eb-A full report shape drifted: $report"
    verify_report_header "$report" "$(value candidate_profile_sha256)" \
        "$(value full_variants)" "$(value full_candidate_summary)"
    report_keys "$report" >"$tmp/$label.keys"
    [[ "$(lines "$tmp/$label.keys")" == "$(value full_variants)" \
        && "$(sha "$tmp/$label.keys")" == "$(value full_keys_sha256)" \
        && -z "$(uniq -d "$tmp/$label.keys")" \
        && "$(report_runnable "$report")" == "$(value full_candidate_runnable)" \
        && "$(report_count pass "$report")" == "$(value full_candidate_passes)" ]] \
        || die "R3eb-A full candidate outcome drifted: $report"
    [[ "$(sed -n '1p' "$json")" \
            == "$(sed -n '1p' "${candidate%.tsv}.jsonl")" \
        && "$(tail -n 1 "$json")" == "$candidate_full_json_summary" ]] \
        || die "R3eb-A full JSONL metadata or summary drifted: $json"
    awk -v expected="$(value full_variants)" '
        /^\{"kind":"metadata",/{metadata++}
        /^\{"kind":"result",/{results++}
        /^\{"kind":"summary",/{summaries++}
        END{
            if(metadata!=1||results!=expected||summaries!=1)exit 1
        }
    ' "$json" || die "R3eb-A full JSONL shape drifted: $json"
    report_rows "$report" >"$tmp/$label.tsv-results"
    json_result_projection "$json" >"$tmp/$label.json-results"
    diff -u "$tmp/$label.tsv-results" "$tmp/$label.json-results"
    if [[ "$full_pending" == false ]]; then
        [[ "$(sha "$report")" == "$(value full_candidate_tsv_sha256)" \
            && "$(sha "$json")" == "$(value full_candidate_jsonl_sha256)" ]] \
            || die "R3eb-A full candidate hash drifted: $report"
    fi
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
    || "${full_candidate_a%.tsv}.jsonl" \
        -ef "${full_candidate_b%.tsv}.jsonl" ]]; then
    die 'R3eb-A full candidate replays must be distinct files'
fi
if ! cmp -s "$full_candidate_a" "$full_candidate_b" \
    || ! cmp -s "${full_candidate_a%.tsv}.jsonl" \
        "${full_candidate_b%.tsv}.jsonl"; then
    die 'R3eb-A full candidate replays are not byte-identical'
fi

rows_for_keys "$tmp/union-keys" "$full_candidate_a" \
    >"$tmp/full-candidate-scope.tsv"
report_rows "$candidate" >"$tmp/focused-candidate-scope.tsv"
diff -u "$tmp/focused-candidate-scope.tsv" "$tmp/full-candidate-scope.tsv"
json_rows_for_keys "$tmp/union-keys" "${full_candidate_a%.tsv}.jsonl" \
    >"$tmp/full-candidate-scope.jsonl"
awk '/^\{"kind":"result",/' "${candidate%.tsv}.jsonl" \
    >"$tmp/focused-candidate-scope.jsonl"
diff -u "$tmp/focused-candidate-scope.jsonl" \
    "$tmp/full-candidate-scope.jsonl"

derived_parent_tsv=$tmp/full-parent.tsv
derived_parent_json=$tmp/full-parent.jsonl
reverse_tsv "$full_candidate_a" "$derived_parent_tsv" \
    "$(value full_candidate_summary)" "$(value full_parent_summary)"
reverse_jsonl "${full_candidate_a%.tsv}.jsonl" "$derived_parent_json" \
    '{"kind":"summary","outcomes":{"fail-parse":7,"fail-runtime":43,"pass":68145,"skipped-config-exclude":6700,"skipped-feature":11775,"timeout":2,"unsupported-feature":11348,"unsupported-module":625,"unsupported-negative-provenance":3392}}'
check_file "$derived_parent_tsv" "$(value full_report_lines)" \
    "$(value full_parent_tsv_sha256)"
check_file "$derived_parent_json" "$(value full_jsonl_lines)" \
    "$(value full_parent_jsonl_sha256)"
rows_for_keys "$tmp/union-keys" "$derived_parent_tsv" \
    >"$tmp/full-parent-scope.tsv"
report_rows "$focused_parent" >"$tmp/focused-parent-scope.tsv"
diff -u "$tmp/focused-parent-scope.tsv" "$tmp/full-parent-scope.tsv"
json_rows_for_keys "$tmp/union-keys" "$derived_parent_json" \
    >"$tmp/full-parent-scope.jsonl"
awk '/^\{"kind":"result",/' "$focused_parent_json" \
    >"$tmp/focused-parent-scope.jsonl"
diff -u "$tmp/focused-parent-scope.jsonl" "$tmp/full-parent-scope.jsonl"

: >"$tmp/full-outcome.keys"
: >"$tmp/full-detail.keys"
transition_counts=$(awk -F'\t' -v parent="$derived_parent_tsv" \
    -v outcome_keys="$tmp/full-outcome.keys" \
    -v detail_keys="$tmp/full-detail.keys" '
    FILENAME==parent {
        if (!/^#/&&!($1=="path"&&$2=="variant")) {
            old[$1 FS $2]=$0
            before++
        }
        next
    }
    !/^#/&&!($1=="path"&&$2=="variant") {
        key=$1 FS $2
        if (!(key in old)) exit 2
        split(old[key],prior,FS)
        for (i=1;i<=6;i++) if (prior[i]!=$i) exit 3
        if (prior[7]!="pass"&&$7=="pass") gains++
        if (prior[7]=="pass"&&$7!="pass") regressions++
        if (old[key]!=$0) {
            changed++
            if (prior[7]!=$7) {
                outcome++
                print $1 "\t" $2 > outcome_keys
            } else {
                detail++
                print $1 "\t" $2 > detail_keys
            }
        }
        seen[key]=1
    }
    END {
        for (key in old) if (!(key in seen)) exit 4
        printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d",
            changed,outcome,detail,before-changed,gains,regressions
    }
' "$derived_parent_tsv" "$full_candidate_a") \
    || die 'R3eb-A full exact join failed'
expected_counts="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) gains=$(value full_pass_gains) regressions=$(value full_pass_regressions)"
[[ "$transition_counts" == "$expected_counts" ]] \
    || die "R3eb-A full transition drifted: $transition_counts"
sort "$tmp/full-outcome.keys" >"$tmp/full-outcome.sorted.keys"
sort "$tmp/full-detail.keys" >"$tmp/full-detail.sorted.keys"
[[ "$(lines "$tmp/full-outcome.sorted.keys")" == "$(value full_outcome_changed)" \
    && "$(sha "$tmp/full-outcome.sorted.keys")" \
        == "$(value full_outcome_keys_sha256)" \
    && "$(lines "$tmp/full-detail.sorted.keys")" == "$(value full_detail_only)" \
    && "$(sha "$tmp/full-detail.sorted.keys")" \
        == "$(value full_detail_only_keys_sha256)" ]] \
    || die 'R3eb-A full transition key partition drifted'
diff -u "$tmp/outcome-transition.keys" "$tmp/full-outcome.sorted.keys"
[[ ! -s "$tmp/full-detail.sorted.keys" ]] \
    || die 'R3eb-A unexpectedly changed a full-report detail-only row'

actual_tsv_sha=$(sha "$full_candidate_a")
actual_jsonl_sha=$(sha "${full_candidate_a%.tsv}.jsonl")
prospective_baseline=$tmp/test262-full-baseline.txt
{
    printf 'schema=test262-canonical-classified-v2\n'
    printf 'timeout_ms=%s\n' "$(value timeout_ms)"
    printf 'variants=%s\n' "$(value full_variants)"
    printf 'runnable=%s\n' "$(value full_candidate_runnable)"
    printf 'passes=%s\n' "$(value full_candidate_passes)"
    printf 'tsv_sha256=%s\n' "$actual_tsv_sha"
    printf 'jsonl_sha256=%s\n' "$actual_jsonl_sha"
    printf 'summary=%s\n' "$(value full_candidate_summary)"
} >"$prospective_baseline"
prospective_baseline_sha=$(sha "$prospective_baseline")

if [[ "$full_pending" == true ]]; then
    printf '%s\n' \
        'R3eb-A full semantics verified, but the frozen receipt is PENDING.' \
        "full_candidate_tsv_sha256=$actual_tsv_sha" \
        "full_candidate_jsonl_sha256=$actual_jsonl_sha" \
        "candidate_canonical_baseline_sha256=$prospective_baseline_sha" \
        'full_candidate_replay_status=passed-twice' >&2
    die 'populate the four PENDING fields and admit the prospective canonical baseline'
fi

[[ "$actual_tsv_sha" == "$(value full_candidate_tsv_sha256)" \
    && "$actual_jsonl_sha" == "$(value full_candidate_jsonl_sha256)" \
    && "$prospective_baseline_sha" \
        == "$(value candidate_canonical_baseline_sha256)" ]] \
    || die 'R3eb-A completed full receipt values drifted'
printf 'R3eb-A canonical successor passed (%s): 64 gains, one historical pass, 101973 unchanged; candidate=%s json=%s\n' \
    "$full_receipt_kind" "$actual_tsv_sha" "$actual_jsonl_sha"
