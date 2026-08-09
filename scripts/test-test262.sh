#!/usr/bin/env bash
# Parameterized gate for the current, data-defined Test262 milestone.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
default_spec=dev-support/test262/current.conf

usage() {
    printf 'usage: %s [--spec FILE] [--check|--focused|--full]\n' "${0##*/}"
    printf '  --check    authenticate the spec and its frozen inputs and receipts\n'
    printf '  --focused  rerun and byte-compare the focused milestone receipt\n'
    printf '  --full     rerun and authenticate the complete Test262 result vector\n'
}

die() { echo "error: $*" >&2; exit 1; }

mode=check
spec_arg=$default_spec
mode_seen=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --spec)
            [[ $# -ge 2 ]] || { usage >&2; exit 2; }
            spec_arg=$2
            shift 2
            ;;
        --check|--focused|--full)
            [[ "$mode_seen" == false ]] || { usage >&2; exit 2; }
            mode=${1#--}
            mode_seen=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done

case $spec_arg in
    /*) spec=$spec_arg ;;
    *) spec=$root/$spec_arg ;;
esac
[[ -f "$spec" && ! -L "$spec" ]] || die "spec is not a regular file: $spec_arg"

required_keys='schema milestone quickjs test262 test262_patch_sha256 test262_config_sha256 test262_metadata_records test262_metadata_sha256 upstream upstream_lines upstream_sha256 profile profile_lines profile_sha256 manifest manifest_lines manifest_sha256 focused_tsv focused_tsv_lines focused_tsv_sha256 focused_jsonl focused_jsonl_lines focused_jsonl_sha256 mode timeout_ms focused_variants focused_eligible focused_runnable focused_passes focused_summary full_variants full_eligible full_runnable full_passes full_tsv_lines full_tsv_sha256 full_jsonl_lines full_jsonl_sha256 full_summary'

# Parse as inert data. In particular, this gate never sources or evaluates a spec.
awk -v required="$required_keys" '
    BEGIN {
        split(required, names, " ")
        for (i in names) wanted[names[i]]=1
    }
    {
        line=$0
        sub(/\r$/, "", line)
        if (line=="" || line ~ /^#/) next
        if (line !~ /^[a-z][a-z0-9_]*=/) {
            print "error: malformed Test262 spec line " NR > "/dev/stderr"
            failed=1
            next
        }
        key=line
        sub(/=.*/, "", key)
        value=substr(line, length(key)+2)
        if (!(key in wanted)) {
            print "error: unknown Test262 spec key: " key > "/dev/stderr"
            failed=1
        }
        if (++seen[key] != 1) {
            print "error: duplicate Test262 spec key: " key > "/dev/stderr"
            failed=1
        }
        if (value=="" || value ~ /^[[:space:]]/ || value ~ /[[:space:]]$/ ||
            value ~ /[^ -~]/) {
            print "error: invalid Test262 spec value for: " key > "/dev/stderr"
            failed=1
        }
    }
    END {
        for (key in wanted) {
            if (seen[key] != 1) {
                print "error: missing Test262 spec key: " key > "/dev/stderr"
                failed=1
            }
        }
        exit failed
    }
' "$spec" || exit 1

spec_value() {
    local wanted=$1
    awk -v wanted="$wanted" '
        $0 ~ ("^" wanted "=") {
            print substr($0, length(wanted)+2)
            found++
        }
        END { if (found != 1) exit 1 }
    ' "$spec"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die 'sha256sum or shasum is required'
    fi
}

line_count() { wc -l <"$1" | tr -d '[:space:]'; }

repo_path() {
    local key=$1
    local relative
    relative=$(spec_value "$key")
    case "/$relative/" in
        *//*|*'/./'*|*'/../'*|*\\*) die "unsafe repository path in $key: $relative" ;;
    esac
    case $relative in
        /*|'') die "unsafe repository path in $key: $relative" ;;
    esac
    printf '%s/%s\n' "$root" "$relative"
}

check_file() {
    local path=$1
    local expected_lines=$2
    local expected_sha=$3
    local label=$4
    [[ -f "$path" && ! -L "$path" ]] || die "$label is not a regular file: $path"
    [[ "$(line_count "$path")" == "$expected_lines" ]] \
        || die "$label line count drifted: $path"
    [[ "$(sha256_file "$path")" == "$expected_sha" ]] \
        || die "$label checksum drifted: $path"
}

toml_value() {
    local section=$1
    local key=$2
    local file=$3
    awk -v wanted_section="[$section]" -v wanted_key="$key" '
        $0==wanted_section { inside=1; next }
        /^\[/ { inside=0 }
        inside {
            candidate=$0
            sub(/=.*/, "", candidate)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", candidate)
            if (candidate != wanted_key) next
            value=substr($0, index($0, "=")+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
            if (value ~ /^".*"$/) value=substr(value, 2, length(value)-2)
            print value
            found++
        }
        END { if (found != 1) exit 1 }
    ' "$file"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1=="path" && $2=="variant")' "$1"
}

report_header() {
    local report=$1
    local key=$2
    awk -v wanted="# $key=" '
        index($0, wanted)==1 { print substr($0, length(wanted)+1); found++ }
        END { if (found != 1) exit 1 }
    ' "$report"
}

report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
report_variants() { report_rows "$1" | awk 'END { print NR+0 }'; }
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection" { count++ } END { print count+0 }'
}
report_passes() {
    report_rows "$1" | awk -F'\t' '$7=="pass" { count++ } END { print count+0 }'
}
computed_summary() {
    report_rows "$1" | awk -F'\t' '{ print $7 }' | sort | uniq -c | awk '
        { output=output (NR==1 ? "" : " ") $2 "=" $1 }
        END { print output }
    '
}

verify_json_projection() {
    local report=$1
    local json=$2
    local expected_variants=$3
    local expected_summary=$4
    node - "$json" "$expected_variants" "$expected_summary" \
        "$(spec_value quickjs)" "$(spec_value test262)" \
        "$(spec_value test262_patch_sha256)" "$(spec_value test262_config_sha256)" \
        "$(spec_value test262_metadata_sha256)" "$(spec_value profile_sha256)" \
        "$(spec_value mode)" <<'NODE' >"$tmp/json.rows"
const fs = require("node:fs");
const [path, variantsText, expectedSummary, quickjs, test262, patch, config,
  metadataHash, profileHash, mode] = process.argv.slice(2);
const lines = fs.readFileSync(path, "utf8").trimEnd().split("\n");
const records = lines.map((line) => JSON.parse(line));
const variants = Number(variantsText);
if (records.length !== variants + 2) process.exit(2);
const metadata = records[0];
if (metadata.kind !== "metadata" || metadata.schema !== 2 ||
    metadata.quickjs !== quickjs || metadata.test262 !== test262 ||
    metadata.test262_patch_sha256 !== patch ||
    metadata.test262_config_sha256 !== config ||
    metadata.test262_metadata_sha256 !== metadataHash ||
    metadata.oxide_profile_sha256 !== profileHash ||
    metadata.profile !== "test262-canonical-classified-v2" ||
    metadata.mode !== mode) process.exit(3);
const summary = records.at(-1);
if (summary.kind !== "summary") process.exit(4);
const actualSummary = Object.entries(summary.outcomes)
  .map(([name, count]) => `${name}=${count}`).join(" ");
if (actualSummary !== expectedSummary) process.exit(5);
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
for (const record of records.slice(1, -1)) {
  if (record.kind !== "result" || fields.some((field) => typeof record[field] !== "string")) {
    process.exit(6);
  }
  process.stdout.write(fields.map((field) => escapeField(record[field])).join("\t") + "\n");
}
NODE
    report_rows "$report" >"$tmp/tsv.rows"
    cmp -s "$tmp/tsv.rows" "$tmp/json.rows" \
        || die "TSV and JSONL result vectors differ: $report"
}

verify_report() {
    local report=$1
    local json=$2
    local prefix=$3
    local expected_variants expected_eligible expected_runnable expected_passes
    local expected_summary expected_tsv_lines expected_jsonl_lines
    local expected_tsv_sha expected_jsonl_sha
    expected_variants=$(spec_value "${prefix}_variants")
    expected_eligible=$(spec_value "${prefix}_eligible")
    expected_runnable=$(spec_value "${prefix}_runnable")
    expected_passes=$(spec_value "${prefix}_passes")
    expected_summary=$(spec_value "${prefix}_summary")
    expected_tsv_lines=$(spec_value "${prefix}_tsv_lines")
    expected_jsonl_lines=$(spec_value "${prefix}_jsonl_lines")
    expected_tsv_sha=$(spec_value "${prefix}_tsv_sha256")
    expected_jsonl_sha=$(spec_value "${prefix}_jsonl_sha256")

    check_file "$report" "$expected_tsv_lines" "$expected_tsv_sha" "$prefix TSV receipt"
    check_file "$json" "$expected_jsonl_lines" "$expected_jsonl_sha" "$prefix JSONL receipt"
    [[ "$(report_header "$report" oxide_profile_sha256)" == "$(spec_value profile_sha256)" \
        && "$(report_header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(report_header "$report" mode)" == "$(spec_value mode)" ]] \
        || die "$prefix report metadata drifted"
    [[ "$(report_variants "$report")" == "$expected_variants" \
        && "$(report_runnable "$report")" == "$expected_eligible" \
        && "$expected_eligible" == "$expected_runnable" \
        && "$(report_passes "$report")" == "$expected_passes" \
        && "$(report_summary "$report")" == "$expected_summary" \
        && "$(computed_summary "$report")" == "$expected_summary" ]] \
        || die "$prefix classified result vector drifted"
    verify_json_projection "$report" "$json" "$expected_variants" "$expected_summary"
}

for key in upstream profile manifest focused_tsv focused_jsonl; do
    relative=$(spec_value "$key")
    case "/$relative/" in
        *//*|*'/./'*|*'/../'*|*\\*) die "unsafe repository path in $key: $relative" ;;
    esac
    case $relative in
        /*|'') die "unsafe repository path in $key: $relative" ;;
    esac
done

for key in upstream_lines profile_lines manifest_lines focused_tsv_lines \
    focused_jsonl_lines test262_metadata_records timeout_ms focused_variants \
    focused_eligible focused_runnable focused_passes full_variants full_eligible \
    full_runnable full_passes full_tsv_lines full_jsonl_lines; do
    value=$(spec_value "$key")
    [[ "$value" =~ ^(0|[1-9][0-9]*)$ ]] \
        || die "non-canonical numeric Test262 spec value for $key"
done
for key in test262_patch_sha256 test262_config_sha256 test262_metadata_sha256 \
    upstream_sha256 profile_sha256 manifest_sha256 focused_tsv_sha256 \
    focused_jsonl_sha256 full_tsv_sha256 full_jsonl_sha256; do
    value=$(spec_value "$key")
    [[ "$value" =~ ^[0-9a-f]{64}$ ]] || die "invalid SHA-256 in Test262 spec for $key"
done
[[ "$(spec_value schema)" == test262-gate-v1 \
    && "$(spec_value mode)" == both \
    && "$(spec_value focused_eligible)" == "$(spec_value focused_runnable)" \
    && "$(spec_value full_eligible)" == "$(spec_value full_runnable)" ]] \
    || die 'unsupported Test262 gate spec contract'

validate_summary_contract() {
    local prefix=$1
    local variants eligible passes tsv_lines jsonl_lines summary
    variants=$(spec_value "${prefix}_variants")
    eligible=$(spec_value "${prefix}_eligible")
    passes=$(spec_value "${prefix}_passes")
    tsv_lines=$(spec_value "${prefix}_tsv_lines")
    jsonl_lines=$(spec_value "${prefix}_jsonl_lines")
    summary=$(spec_value "${prefix}_summary")
    awk -v summary="$summary" -v variants="$variants" -v eligible="$eligible" \
        -v passes="$passes" '
        BEGIN {
            count=split(summary, fields, " ")
            for (i=1; i<=count; i++) {
                if (fields[i] !~ /^[a-z][a-z0-9-]*=[0-9]+$/) exit 1
                split(fields[i], pair, "=")
                if (seen[pair[1]]++ || (i>1 && previous >= pair[1])) exit 1
                previous=pair[1]
                total+=pair[2]
                if (pair[1] == "pass") actual_passes=pair[2]
                if (pair[1] ~ /^(skipped|unsupported)-/) ineligible+=pair[2]
            }
            if (total != variants || actual_passes != passes ||
                variants-ineligible != eligible) exit 1
        }
    ' || die "$prefix summary/count contract is inconsistent"
    ((tsv_lines == variants + 11 && jsonl_lines == variants + 2)) \
        || die "$prefix report line-count contract is inconsistent"
}

validate_summary_contract focused
validate_summary_contract full

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-test262.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

upstream=$(repo_path upstream)
profile=$(repo_path profile)
manifest=$(repo_path manifest)
focused_tsv=$(repo_path focused_tsv)
focused_jsonl=$(repo_path focused_jsonl)
output_dir=$root/target
[[ ! -e "$output_dir" || (-d "$output_dir" && ! -L "$output_dir") ]] \
    || die 'target output directory must not be a symbolic link'
full_report=$output_dir/test262-full.tsv
full_json=$output_dir/test262-full.jsonl

check_file "$upstream" "$(spec_value upstream_lines)" \
    "$(spec_value upstream_sha256)" 'upstream pin'
check_file "$profile" "$(spec_value profile_lines)" \
    "$(spec_value profile_sha256)" 'Oxide profile'
check_file "$manifest" "$(spec_value manifest_lines)" \
    "$(spec_value manifest_sha256)" 'focused manifest'
[[ "$(toml_value quickjs version "$upstream")" == "$(spec_value quickjs)" \
    && "$(toml_value test262 commit "$upstream")" == "$(spec_value test262)" \
    && "$(toml_value test262 patch_sha256 "$upstream")" \
        == "$(spec_value test262_patch_sha256)" \
    && "$(toml_value test262 config_sha256 "$upstream")" \
        == "$(spec_value test262_config_sha256)" \
    && "$(toml_value test262 test_count "$upstream")" \
        == "$(spec_value test262_metadata_records)" \
    && "$(toml_value test262 metadata_records_sha256 "$upstream")" \
        == "$(spec_value test262_metadata_sha256)" \
    && "$(toml_value test262 oxide_profile "$upstream")" == "$(spec_value profile)" \
    && "$(toml_value test262 oxide_profile_sha256 "$upstream")" \
        == "$(spec_value profile_sha256)" ]] \
    || die 'upstream pin and Test262 gate spec disagree'

sort "$manifest" >"$tmp/manifest.sorted"
cmp -s "$manifest" "$tmp/manifest.sorted" || die 'focused manifest is not bytewise sorted'
[[ -z "$(uniq -d "$manifest")" ]] || die 'focused manifest contains duplicate paths'
verify_report "$focused_tsv" "$focused_jsonl" focused
report_rows "$focused_tsv" | cut -f1 >"$tmp/focused.paths"
cmp -s "$manifest" "$tmp/focused.paths" \
    || die 'focused receipt paths do not exactly match the manifest'

if [[ "$mode" == check ]]; then
    printf '%s Test262 spec and frozen receipts are authenticated.\n' "$(spec_value milestone)"
    exit 0
fi

workers=${TEST262_WORKERS:-2}
[[ "$workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: TEST262_WORKERS must be a positive integer' >&2; exit 2; }
runner_override=${TEST262_RUNNER:-}
if [[ -n "$runner_override" ]]; then
    runner=$runner_override
    [[ "$runner" == /* ]] || die 'TEST262_RUNNER must be an absolute path'
else
    target_dir=${CARGO_TARGET_DIR:-$root/target}
    case $target_dir in
        /*) ;;
        *) target_dir=$root/$target_dir ;;
    esac
    build_host=$(rustc -vV | awk '$1=="host:" { print $2; found++ } END { if (found!=1) exit 1 }')
    cargo build --locked --release --target "$build_host" \
        --target-dir "$target_dir" --bin run-test262
    runner=$target_dir/$build_host/release/run-test262
fi
[[ -f "$runner" && -x "$runner" && ! -L "$runner" ]] \
    || die 'run-test262 is not an executable regular file'

suite=$("$script_dir/prepare-test262.sh")
[[ -n "$suite" && "$suite" == /* && -d "$suite/test" && -d "$suite/harness" \
    && ! -L "$suite" ]] \
    || die 'prepare-test262.sh did not return one authenticated suite path'
source_dir=$(CDPATH='' cd -- "$suite/.." && pwd)
metadata_records=$tmp/test262-metadata.bin
"$runner" --suite "$suite" --validate-metadata "$metadata_records" \
    >"$tmp/metadata-audit.log"
check_file "$metadata_records" "$(spec_value test262_metadata_records)" \
    "$(spec_value test262_metadata_sha256)" 'Test262 metadata vector'
grep -Fqx "Test262 metadata: files=$(spec_value test262_metadata_records)" \
    "$tmp/metadata-audit.log" || die 'Test262 metadata audit output drifted'

if [[ "$mode" == focused ]]; then
    replay=$tmp/focused.tsv
    run_output=$("$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$replay" \
        --mode "$(spec_value mode)" --workers "$workers" \
        --timeout-ms "$(spec_value timeout_ms)" --allow-failures)
    printf '%s\n' "$run_output"
    verify_report "$replay" "${replay%.tsv}.jsonl" focused
    cmp -s "$focused_tsv" "$replay" || die 'focused TSV replay is not byte-identical'
    cmp -s "$focused_jsonl" "${replay%.tsv}.jsonl" \
        || die 'focused JSONL replay is not byte-identical'
    printf '%s focused Test262 vector matches: %s pass of %s eligible variants.\n' \
        "$(spec_value milestone)" "$(spec_value focused_passes)" \
        "$(spec_value focused_eligible)"
    exit 0
fi

for protected in "$spec" "$upstream" "$profile" "$manifest" "$focused_tsv" "$focused_jsonl"; do
    [[ "$full_report" != "$protected" && "$full_json" != "$protected" ]] \
        || die "full output aliases protected input: $protected"
    if [[ -e "$full_report" && "$full_report" -ef "$protected" ]] \
        || [[ -e "$full_json" && "$full_json" -ef "$protected" ]]; then
        die "full output aliases protected input inode: $protected"
    fi
done
mkdir -p "$output_dir"
[[ -d "$output_dir" && ! -L "$output_dir" ]] \
    || die 'target output directory must be a real directory'
rm -f -- "$full_report" "$full_json"
run_output=$("$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --all --report "$full_report" \
    --mode "$(spec_value mode)" --workers "$workers" \
    --timeout-ms "$(spec_value timeout_ms)" --allow-failures)
printf '%s\n' "$run_output"
execution_line=$(printf '%s\n' "$run_output" | \
    awk '/^execution: runnable=/ { print; found++ } END { if (found!=1) exit 1 }')
actual_runnable=${execution_line#*runnable=}
actual_runnable=${actual_runnable%% *}
[[ "$actual_runnable" == "$(spec_value full_runnable)" ]] \
    || die 'full runner eligible/runnable count drifted'
verify_report "$full_report" "$full_json" full
printf '%s complete Test262 vector matches: %s pass of %s eligible (%s total) variants.\n' \
    "$(spec_value milestone)" "$(spec_value full_passes)" \
    "$(spec_value full_eligible)" "$(spec_value full_variants)"
