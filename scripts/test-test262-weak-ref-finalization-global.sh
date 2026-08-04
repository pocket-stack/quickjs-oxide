#!/usr/bin/env bash
# Reproduce the exhaustive WeakRef/FinalizationRegistry global admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-weak-ref-finalization-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
parent=tests/test262-weak-ref-finalization-global-parent.conf
candidate=tests/test262-weak-ref-finalization-global-candidate.conf
live_profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
added_features=tests/test262-weak-ref-finalization-candidate-features.txt
universe=tests/test262-weak-ref-finalization-universe.txt
activation=tests/test262-weak-ref-finalization-activation.txt
for_of=tests/test262-weak-ref-finalization-for-of-blocker.txt
create_realm=tests/test262-weak-ref-finalization-create-realm-blockers.txt
transition=tests/test262-weak-ref-finalization-global-transitions.tsv
parent_report=target/test262-weak-ref-finalization-global-parent.tsv
candidate_report=target/test262-weak-ref-finalization-global-candidate.tsv
parent_full=target/test262-weak-ref-finalization-global-parent-full.tsv
candidate_full=target/test262-weak-ref-finalization-global-candidate-full.tsv
oracle_log=target/test262-weak-ref-finalization-global-quickjs.log
baseline_draft=target/test262-weak-ref-finalization-global-baseline.draft.txt
transition_draft=target/test262-weak-ref-finalization-global-transitions.draft.tsv
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
parent_sha=3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef
candidate_sha=8be6c2a3892a62d89ed17df3f3d3b54e9e84fda8ef6be2bcdaa7d49044593990
added_features_sha=0a462001d5a51db3b103ccdadfad17076941c5a5f7f163d767bedec5fc471406
parent_features_sha=a892ce31bef675386670419a9410e6086c24f1edd9f8e14f6c793d8bfb07503b
candidate_features_sha=82f8c1c3f217e45d3e02b60776bad5ec8268b8270a608990906802c38c8ce139
audited_negative_tests_sha=709b3f86b0820c524cdd645a2993e7e17ae65f840936d388b9d7c890c2970412
execution_sha=e26ec9bb60b6289635c1ab1347a0e7c7372cc5c329998c9c1504299da452acd8
universe_sha=0325512882ba3d93d225423b62b76b9d8bebc7266a427ed6e05be3b70559c060
universe_keys_sha=f4beb592d73342a4d694430d8b13a04122b03f61e7c9a79d2e24476e002910a9
activation_sha=de660ae31e700129f9668760e92cd0e712fcbbe753d4f31d321790645428b848
activation_keys_sha=f04acfd7dcc3c8aaf9e06f4734089eb61bf1cf0ffc99d47cf80c5f98ab35e5de
for_of_sha=b08463b0d3b1aeca28a1520dc7e01f9e18d595296197ab2767747f931134b8ea
for_of_keys_sha=446fe46a6dcb2c3b55272ff2545eb6d4197051cfc09b30fab7121f0a7ca8a521
create_realm_sha=21948f4d14d8fd58cd020972aaefe9ed0e02c8d41f9a4ea839d9b1ccd74757f0
create_realm_keys_sha=5ff830450906569e072bc03701c10edb9748124ee28a8a8fe08c788dd628416a
all_keys_sha=69f0826f8f362d15c99b47e0fdd0aeb7dba2693f67abb255546f25cda026c797
historical_parent_full_tsv_sha=e0b0be534f07a34bc7a9e18f4c3bae8c9360dd62c89176f96bf3234c5895b6ec
historical_parent_full_jsonl_sha=8227cb6d19fc2f814bdb016308cf1003be6c91ebe01145ccc3c719f6e38ac6bf
canonical_full_tsv_sha=c919dd56fc37f2946d729ee9a9a6958fc91c3f95366843ffae258953145e5a4f
canonical_full_jsonl_sha=342c22edd7cfdc4edf2b5085455c8586095bb4abc5b59d55cc4657c5ff954459
canonical_full_summary='fail-parse=11 fail-runtime=110 pass=64628 skipped-config-exclude=6700 skipped-feature=11775 timeout=2 unsupported-feature=13866 unsupported-host-agent=118 unsupported-host-can-block-false=4 unsupported-host-create-realm=490 unsupported-host-eval-script=44 unsupported-host-gc=26 unsupported-host-is-html-dda=84 unsupported-module=679 unsupported-negative-provenance=3451 unsupported-parser=26 unsupported-runtime=23'
baseline_sha=d1758cc0bdcb82c06b63335f8becdd75496be6f100305afd70d9c58c9edf2e2d

usage() {
    printf 'usage: %s [--check|--full|--bless]\n' "${0##*/}"
    printf '  --check  verify authenticated inputs and the pinned QuickJS oracle\n'
    printf '  --full   additionally run and join both exact 102037-row profiles\n'
    printf '  --bless  bootstrap reviewed focused/full receipt drafts under target/\n'
}

mode=focused
case ${1-} in
    '') ;;
    --check) mode=check ;;
    --full) mode=full ;;
    --bless) mode=bless ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid TEST262_WORKERS' >&2; exit 2; }
[[ "$full_workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid TEST262_FULL_WORKERS' >&2; exit 2; }

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
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
kv_value() {
    awk -F= -v wanted="$2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
value() { kv_value "$baseline" "$1"; }
canonical_value() { kv_value "$canonical_baseline" "$1"; }
toml_test262_value() {
    awk -v wanted="$2" '
        $0 == "[test262]" { inside=1; next }
        /^\[/ { inside=0 }
        inside {
            line=$0
            separator=index(line, "=")
            if (!separator) next
            key=substr(line, 1, separator - 1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if (key != wanted) next
            answer=substr(line, separator + 1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if (answer ~ /^".*"$/) answer=substr(answer, 2, length(answer) - 2)
            print answer
            found++
        }
        END { if (found != 1) exit 1 }
    ' "$1"
}
report_rows() { awk -F'\t' '!/^#/ && !($1=="path"&&$2=="variant")' "$1"; }
report_keys() {
    report_rows "$1" | awk -F'\t' '{ print $1 "\t" $2 }' | sort
}
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_report_summary() {
    report_rows "$1" | awk -F'\t' '{ print $7 }' | sort | uniq -c | awk '
        { output=output (NR == 1 ? "" : " ") $2 "=" $1 }
        END { print output }
    '
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" '$7==wanted{count++} END{print count+0}'
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
check_file() {
    local file=$1 count=$2 digest=$3
    [[ -f "$file" ]] || die "missing gate input: $file"
    [[ "$(lines "$file")" == "$count" && "$(sha "$file")" == "$digest" ]] \
        || die "authenticated input drifted: $file"
}
check_live_admission_binding() {
    check_file "$live_profile" 1271 "$candidate_sha"
    cmp -s "$candidate" "$live_profile" \
        || die 'live Test262 profile is not byte-identical to the admitted candidate'
    [[ "$(toml_test262_value "$upstream" repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$test262" \
        && "$(toml_test262_value "$upstream" shallow_since)" == 2025-09-01 \
        && "$(toml_test262_value "$upstream" patch)" == tests/test262.patch \
        && "$(toml_test262_value "$upstream" patch_sha256)" == "$patch_sha" \
        && "$(toml_test262_value "$upstream" config)" == test262.conf \
        && "$(toml_test262_value "$upstream" config_sha256)" == "$config_sha" \
        && "$(toml_test262_value "$upstream" test_count)" == 53125 \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" == "$metadata_sha" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" == "$candidate_sha" ]] \
        && [[ "$(toml_test262_value "$upstream" expected_errors)" == test262_errors.txt ]] \
        || die 'compat/upstream.toml Test262 identity does not match the admission certificate'
}
check_canonical_baseline_identity() {
    [[ -f "$canonical_baseline" && "$(lines "$canonical_baseline")" == 8 \
        && "$(canonical_value schema)" == test262-canonical-classified-v2 \
        && "$(canonical_value timeout_ms)" == 30000 \
        && "$(canonical_value variants)" == 102037 \
        && "$(canonical_value runnable)" == 64800 \
        && "$(canonical_value passes)" == 64628 \
        && "$(canonical_value tsv_sha256)" == "$canonical_full_tsv_sha" \
        && "$(canonical_value jsonl_sha256)" == "$canonical_full_jsonl_sha" \
        && "$(canonical_value summary)" == "$canonical_full_summary" ]] \
        || die 'canonical Test262 full baseline does not identify R3cg candidate output'
}
check_authenticated_inputs() {
    check_file "$parent" 1269 "$parent_sha"
    check_file "$candidate" 1271 "$candidate_sha"
    check_live_admission_binding
    check_file "$added_features" 2 "$added_features_sha"
    check_file "$universe" 82 "$universe_sha"
    check_file "$activation" 79 "$activation_sha"
    check_file "$for_of" 1 "$for_of_sha"
    check_file "$create_realm" 2 "$create_realm_sha"
    check_canonical_baseline_identity
    if [[ "$mode" != bless ]]; then
        check_file "$baseline" 82 "$baseline_sha"
        check_file "$transition" 169 "$(value transition_receipt_sha256)"
    fi
}
check_prepared_suite_identity() {
    local expected_status actual_status
    expected_status=$' M harness/atomicsHelper.js\n M harness/regExpUtils.js'
    [[ -d "$suite/.git" \
        && "$(basename -- "$source_dir")" == "quickjs-$quickjs" \
        && "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" == "$test262" \
        && "$(sha "$source_dir/tests/test262.patch")" == "$patch_sha" \
        && "$(sha "$source_dir/test262.conf")" == "$config_sha" ]] \
        || die 'prepared QuickJS/Test262 identity drifted'
    actual_status=$(git -C "$suite" status --porcelain=v1 --untracked-files=all | sort)
    [[ "$actual_status" == "$expected_status" ]] \
        || die 'prepared Test262 checkout status drifted'
    git -C "$suite" apply --reverse --check "$source_dir/tests/test262.patch" \
        || die 'prepared Test262 patch is no longer reverse-applicable'
    git -C "$suite" diff --no-ext-diff --no-color --no-renames \
        --abbrev=7 --src-prefix=a/ --dst-prefix=b/ -- \
        harness/atomicsHelper.js harness/regExpUtils.js \
        | cmp -s - "$source_dir/tests/test262.patch" \
        || die 'prepared Test262 harness diff no longer matches the pinned patch'
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
json_report_keys() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3cg JSONL report %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        /^\{"kind":"metadata",/ {
            metadata++
            if (NR != 1) fail("metadata record is not first")
            next
        }
        /^\{"kind":"result",/ {
            results++
            if (!match($0, /"path":"[^"]*"/)) fail("result is missing path")
            path=substr($0, RSTART, RLENGTH)
            sub(/^"path":"/, "", path)
            sub(/"$/, "", path)
            if (!match($0, /"variant":"[^"]*"/)) fail("result is missing variant")
            variant=substr($0, RSTART, RLENGTH)
            sub(/^"variant":"/, "", variant)
            sub(/"$/, "", variant)
            key=path "\t" variant
            if (seen[key]++) fail("duplicate result key")
            print key
            next
        }
        /^\{"kind":"summary",/ {
            summary++
            summary_line=NR
            next
        }
        { fail("unexpected record") }
        END {
            if (!failed && metadata != 1) fail("expected exactly one metadata record")
            if (!failed && summary != 1) fail("expected exactly one summary record")
            if (!failed && summary_line != NR) fail("summary record is not last")
        }
    ' "$report" | sort
}
json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3cg JSONL projection %s: %s\n", report, message \
                >"/dev/stderr"
            failed=1
            exit 2
        }
        function expect(token) {
            if (substr(line, position, length(token)) != token) {
                fail("expected " token " at column " position)
            }
            position+=length(token)
        }
        function string_value(    character, escape, digits, value) {
            expect("\"")
            value=""
            while (position <= length(line)) {
                character=substr(line, position, 1)
                if (character == "\"") {
                    position++
                    return value
                }
                if (character == "\\") {
                    position++
                    if (position > length(line)) fail("unterminated escape")
                    escape=substr(line, position, 1)
                    if (escape == "u") {
                        digits=substr(line, position + 1, 4)
                        if (length(digits) != 4 ||
                            digits ~ /[^0123456789abcdefABCDEF]/) {
                            fail("invalid Unicode escape")
                        }
                        value=value "\\u" digits
                        position+=5
                    } else {
                        if (index("\"\\/bfnrt", escape) == 0) {
                            fail("invalid string escape")
                        }
                        if (escape == "\"") value=value "\""
                        else if (escape == "/") value=value "/"
                        else if (escape == "b") value=value "\\u0008"
                        else if (escape == "f") value=value "\\u000c"
                        else value=value "\\" escape
                        position++
                    }
                    continue
                }
                if (character == "\t" || character == "\r") {
                    fail("unescaped control character")
                }
                value=value character
                position++
            }
            fail("unterminated string")
        }
        function project_result(    i, key, value) {
            line=$0
            position=1
            expect("{")
            for (i=1; i<=11; i++) {
                if (i != 1) expect(",")
                key=string_value()
                if (key != name[i]) fail("unexpected field " key " at position " i)
                expect(":")
                value=string_value()
                if (i == 1) {
                    if (value != "result") fail("unexpected record kind")
                } else {
                    field[i - 1]=value
                }
            }
            expect("}")
            if (position != length(line) + 1) fail("trailing record data")
            print field[1], field[2], field[3], field[4], field[5],
                field[6], field[7], field[8], field[9], field[10]
        }
        BEGIN {
            OFS="\t"
            name[1]="kind"
            name[2]="path"
            name[3]="variant"
            name[4]="flags"
            name[5]="features"
            name[6]="expected_phase"
            name[7]="expected_type"
            name[8]="outcome"
            name[9]="actual_phase"
            name[10]="actual_type"
            name[11]="detail"
        }
        /^\{"kind":"metadata",/ { next }
        /^\{"kind":"result",/ { project_result(); next }
        /^\{"kind":"summary",/ { next }
        { fail("unexpected record") }
    ' "$report"
}
json_report_summary() {
    local report=$1
    tail -n 1 "$report" | awk -v report="$report" '
        function fail(message) {
            printf "error: R3cg JSONL summary %s: %s\n", report, message \
                >"/dev/stderr"
            failed=1
            exit 2
        }
        {
            prefix="{\"kind\":\"summary\",\"outcomes\":{"
            suffix="}}"
            if (substr($0, 1, length(prefix)) != prefix ||
                substr($0, length($0) - length(suffix) + 1) != suffix) {
                fail("malformed summary envelope")
            }
            body=substr($0, length(prefix) + 1,
                length($0) - length(prefix) - length(suffix))
            if (body == "") fail("empty outcomes object")
            count=split(body, entries, ",")
            output=""
            previous=""
            for (i=1; i<=count; i++) {
                if (entries[i] !~ /^"[a-z0-9-]+":(0|[1-9][0-9]*)$/) {
                    fail("outcome entry is not canonical name:integer")
                }
                separator=index(entries[i], "\":")
                outcome=substr(entries[i], 2, separator - 2)
                number=substr(entries[i], separator + 2)
                if (seen[outcome]++) fail("duplicate outcome")
                if (previous != "" && !(previous < outcome)) {
                    fail("outcomes are not in canonical order")
                }
                output=output (i == 1 ? "" : " ") outcome "=" number
                previous=outcome
            }
            print output
            records++
        }
        END { if (!failed && records != 1) fail("expected one summary record") }
    '
}
expected_json_metadata() {
    local profile_sha=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"test262-canonical-classified-v2","mode":"both"}\n' \
        "$quickjs" "$test262" "$patch_sha" "$config_sha" "$metadata_sha" \
        "$profile_sha"
}
verify_json_report() {
    local json=$1 report=$2 profile_sha=$3 rows=$4 keys_sha=$5
    local json_keys json_projection
    json_keys=$(mktemp "$tmp/json-keys.XXXXXX")
    json_projection=$(mktemp "$tmp/json-projection.XXXXXX")
    json_report_keys "$json" >"$json_keys" \
        || die "JSONL record structure drifted: $json"
    json_result_projection "$json" >"$json_projection" \
        || die "JSONL result projection failed: $json"
    [[ "$(lines "$json_keys")" == "$rows" \
        && "$(sha "$json_keys")" == "$keys_sha" \
        && "$(head -n 1 "$json")" == "$(expected_json_metadata "$profile_sha")" \
        && "$(json_report_summary "$json")" == "$(report_summary "$report")" ]] \
        || die "JSONL metadata, key inventory, or summary drifted: $json"
    if ! diff -u <(report_rows "$report") "$json_projection"; then
        die "JSONL/TSV ten-field projection drifted: $json"
    fi
    rm -f -- "$json_keys" "$json_projection"
}
check_report_identity() {
    local report=$1 profile_sha=$2 rows=$3 keys_sha=$4
    local json=${report%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(head -n 1 "$report")" == '# quickjs-oxide Test262 outcome vector v2' \
        && "$(sed -n '10p' "$report")" == $'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\toutcome\tactual_phase\tactual_type\tdetail' \
        && "$(header "$report" quickjs)" == "$quickjs" \
        && "$(header "$report" test262)" == "$test262" \
        && "$(header "$report" test262_patch_sha256)" == "$patch_sha" \
        && "$(header "$report" test262_config_sha256)" == "$config_sha" \
        && "$(header "$report" test262_metadata_sha256)" == "$metadata_sha" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == both \
        && "$(lines "$report")" == "$((rows + 11))" \
        && "$(lines <(report_rows "$report"))" == "$rows" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$keys_sha" \
        && "$(report_summary "$report")" == "$(computed_report_summary "$report")" \
        && "$(lines "$json")" == "$((rows + 2))" ]] \
        || die "classified report identity drifted: $report"
    verify_json_report "$json" "$report" "$profile_sha" "$rows" "$keys_sha"
}
check_report_receipt() {
    local report=$1 label=$2
    [[ "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "${report%.tsv}.jsonl")" == "$(value "${label}_jsonl_sha256")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" ]] \
        || die "report receipt drifted: $label"
}
check_canonical_full_report() {
    local report=$1
    check_canonical_baseline_identity
    [[ "$(lines <(report_rows "$report"))" == "$(canonical_value variants)" \
        && "$(report_runnable "$report")" == "$(canonical_value runnable)" \
        && "$(report_count pass "$report")" == "$(canonical_value passes)" \
        && "$(sha "$report")" == "$(canonical_value tsv_sha256)" \
        && "$(sha "${report%.tsv}.jsonl")" == "$(canonical_value jsonl_sha256)" \
        && "$(report_summary "$report")" == "$(canonical_value summary)" ]] \
        || die 'candidate full report does not match the canonical Test262 baseline'
}
run_report() {
    local profile=$1 report=$2 scope=$3 pool=$4
    local -a selected
    if [[ "$scope" == full ]]; then selected=(--all)
    else selected=(--manifest "$universe"); fi
    rm -f -- "$report" "${report%.tsv}.jsonl"
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" "${selected[@]}" \
        --report "$report" --mode both --workers "$pool" \
        --timeout-ms 30000 --allow-failures
}

cd -- "$root"
if [[ "$mode" != bless ]]; then
    [[ -f "$baseline" ]] || die "missing gate baseline: $baseline"
    while IFS=: read -r key expected; do
        [[ "$(value "$key")" == "$expected" ]] \
            || die "baseline identity drifted: $key"
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
live_oxide_profile:$live_profile
live_oxide_profile_sha256:$candidate_sha
canonical_full_baseline:$canonical_baseline
parent_features:99
parent_features_sha256:$parent_features_sha
candidate_features:101
candidate_features_sha256:$candidate_features_sha
added_features:2
added_features_sha256:$added_features_sha
audited_negative_tests:1157
audited_negative_tests_sha256:$audited_negative_tests_sha
universe_paths:82
universe_sha256:$universe_sha
universe_variants:164
universe_keys_sha256:$universe_keys_sha
activation_paths:79
activation_sha256:$activation_sha
activation_variants:158
activation_keys_sha256:$activation_keys_sha
for_of_blocker_paths:1
for_of_blocker_sha256:$for_of_sha
for_of_blocker_variants:2
for_of_blocker_keys_sha256:$for_of_keys_sha
create_realm_blocker_paths:2
create_realm_blockers_sha256:$create_realm_sha
create_realm_blocker_variants:4
create_realm_blockers_keys_sha256:$create_realm_keys_sha
parent_focused_variants:164
parent_focused_runnable:0
parent_focused_passes:0
parent_focused_unsupported_feature:160
parent_focused_unsupported_host_create_realm:4
candidate_focused_variants:164
candidate_focused_runnable:158
candidate_focused_passes:158
candidate_focused_unsupported_feature:2
candidate_focused_unsupported_host_create_realm:4
transition_data_sha256:13ee058545f39ceb9c442270dbc59c0ccda0e6b32a4109692090b36ac7942e60
full_variants:102037
full_keys_sha256:$all_keys_sha
focused_changed:160
focused_outcome_changes:158
focused_detail_changes:2
focused_unchanged:4
full_changed:160
full_outcome_changes:158
full_detail_changes:2
full_unchanged:101877
full_pass_regressions:0
parent_full_runnable:64642
parent_full_passes:64470
candidate_full_runnable:64800
candidate_full_passes:64628
EOF
fi
check_authenticated_inputs
for sorted in "$added_features" "$universe" "$activation" "$for_of" "$create_realm"; do
    sort -c "$sorted"
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-weak-ref-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
pfeatures=$tmp/parent.features
cfeatures=$tmp/candidate.features
section "$parent" features | sort >"$pfeatures"
section "$candidate" features | sort >"$cfeatures"
[[ "$(lines "$pfeatures")" == 99 && "$(sha "$pfeatures")" == "$parent_features_sha" \
    && "$(lines "$cfeatures")" == 101 && "$(sha "$cfeatures")" == "$candidate_features_sha" ]] \
    || die 'profile feature inventories drifted'
diff -u "$added_features" <(comm -13 "$pfeatures" "$cfeatures")
[[ -z "$(comm -23 "$pfeatures" "$cfeatures")" ]] \
    || die 'candidate removed a parent feature'
for name in audited-negative-tests execution; do
    section "$parent" "$name" >"$tmp/parent.$name"
    section "$candidate" "$name" >"$tmp/candidate.$name"
    diff -u "$tmp/parent.$name" "$tmp/candidate.$name"
done
[[ "$(lines "$tmp/parent.audited-negative-tests")" == 1157 \
    && "$(sha "$tmp/parent.audited-negative-tests")" == "$audited_negative_tests_sha" \
    && "$(lines "$tmp/parent.execution")" == 1 \
    && "$(sha "$tmp/parent.execution")" == "$execution_sha" ]] \
    || die 'non-feature profile sections drifted'

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
check_prepared_suite_identity
metadata_bin=$tmp/metadata.bin
metadata_tsv=$tmp/metadata.tsv
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(lines <(tr '\0' '\t' <"$metadata_bin"))" == 53125 \
    && "$(sha "$metadata_bin")" == "$metadata_sha" ]] \
    || die 'pinned metadata inventory drifted'
tr '\0' '\t' <"$metadata_bin" >"$metadata_tsv"

derived=$tmp/universe.paths
awk -F'\t' '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    has($4,"WeakRef")||has($4,"FinalizationRegistry"){print $1}
' "$metadata_tsv" | sort -u >"$derived"
[[ "$(lines "$derived")" == 82 && "$(sha "$derived")" == "$universe_sha" ]] \
    || die 'WeakRef/FinalizationRegistry metadata universe drifted'
diff -u "$universe" "$derived"

awk -F'\t' 'NR==FNR{wanted[$0]=1;next} $1 in wanted {
    if($5!=""||$6!=""||($3!=""&&$3!="generated"))print $1
}' "$derived" "$metadata_tsv" >"$tmp/invalid-metadata"
[[ ! -s "$tmp/invalid-metadata" ]] \
    || die 'weak-reference universe gained negative or unsupported metadata'

derived_for_of=$tmp/for-of.paths
awk -F'\t' '
    function has(list,value){return index("," list ",", "," value ",")!=0}
    NR==FNR{wanted[$0]=1;next}$1 in wanted&&has($4,"for-of"){print $1}
' "$derived" "$metadata_tsv" | sort -u >"$derived_for_of"
diff -u "$for_of" "$derived_for_of"
derived_create_realm=$tmp/create-realm.paths
: >"$derived_create_realm"
while IFS= read -r path; do
    source=$suite/$path
    [[ -f "$source" ]] || die "pinned Test262 path is missing: $path"
    grep -Fq '$262.createRealm' "$source" && printf '%s\n' "$path" >>"$derived_create_realm"
    if awk '{gsub(/\$262\.createRealm/,"");if($0~/\$262\./||$0~/\$DONE/)found=1}
        END{exit !found}' "$source"; then
        die "unexpected non-createRealm or async host dependency: $path"
    fi
done <"$derived"
sort -u -o "$derived_create_realm" "$derived_create_realm"
diff -u "$create_realm" "$derived_create_realm"
{ cat "$derived_for_of"; cat "$derived_create_realm"; } | sort -u >"$tmp/blockers.paths"
comm -23 "$derived" "$tmp/blockers.paths" >"$tmp/activation.paths"
diff -u "$activation" "$tmp/activation.paths"
[[ -z "$(comm -12 "$derived_for_of" "$derived_create_realm")" ]] \
    || die 'for-of and createRealm blocker sets overlap'

check_keys "$derived" 164 "$universe_keys_sha" "$tmp/universe.keys"
check_keys "$activation" 158 "$activation_keys_sha" "$tmp/activation.keys"
check_keys "$for_of" 2 "$for_of_keys_sha" "$tmp/for-of.keys"
check_keys "$create_realm" 4 "$create_realm_keys_sha" "$tmp/create-realm.keys"
{ cat "$tmp/activation.keys"; cat "$tmp/for-of.keys"; cat "$tmp/create-realm.keys"; } \
    | sort >"$tmp/partition.keys"
diff -u "$tmp/universe.keys" "$tmp/partition.keys"

awk -F'\t' '
    NR==FNR{supported[$0]=1;next}
    function has(list,value){return index("," list ",", "," value ",")!=0}
    has($4,"WeakRef")||has($4,"FinalizationRegistry") {
        missing=0;n=split($4,f,",");for(i=1;i<=n;i++)if(!(f[i] in supported))missing=1
        if(missing)print $1
    }
' "$cfeatures" "$metadata_tsv" | sort -u >"$tmp/missing-feature.paths"
diff -u "$tmp/blockers.paths" "$tmp/missing-feature.paths"

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r path; do files+=("test262/$path"); done <"$derived"
if ! (cd "$source_dir"; ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS failed the WeakRef/FinalizationRegistry universe'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || ! grep -Fq 'Average memory statistics for 164 tests:' "$oracle_log"; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS oracle receipt drifted'
fi
if [[ "$mode" == check ]]; then
    check_authenticated_inputs
    check_prepared_suite_identity
    echo 'WeakRef/FinalizationRegistry global inputs verified: QuickJS passes 164 variants; 158 activate, 2 retain for-of, 4 retain createRealm.'
    exit 0
fi

run_report "$parent" "$parent_report" focused "$workers"
run_report "$candidate" "$candidate_report" focused "$workers"
check_report_identity "$parent_report" "$parent_sha" 164 "$universe_keys_sha"
check_report_identity "$candidate_report" "$candidate_sha" 164 "$universe_keys_sha"
[[ "$(report_summary "$parent_report")" \
        == 'unsupported-feature=160 unsupported-host-create-realm=4' \
    && "$(report_summary "$candidate_report")" \
        == 'pass=158 unsupported-feature=2 unsupported-host-create-realm=4' ]] \
    || die 'focused report summaries drifted'
if [[ "$mode" != bless ]]; then
    check_report_receipt "$parent_report" parent_focused
    check_report_receipt "$candidate_report" candidate_focused
fi

generated_transition=$tmp/transitions.tsv
{
    echo '# R3cg exhaustive WeakRef/FinalizationRegistry global admission transition.'
    echo "# before_oxide_profile_sha256=$parent_sha"
    echo "# after_oxide_profile_sha256=$candidate_sha"
    echo "# manifest_sha256=$universe_sha"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' 'BEGIN{OFS="\t"}
        NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            split(old[$1 FS $2],a,FS)
            print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
        }
    ' "$parent_report" "$candidate_report"
} >"$generated_transition"
{ awk '{print $0 "\tactivation"}' "$activation";
  awk '{print $0 "\tfor-of"}' "$for_of";
  awk '{print $0 "\tcreate-realm"}' "$create_realm"; } >"$tmp/classes"
focused_counts=$(awk -F'\t' '
    NR==FNR{class[$1]=$2;next}
    /^#/||($1=="path"&&$2=="variant"){next}
    {
        kind=class[$1];different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if(kind=="activation"){
            if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
                $11!="pass"||$12!="normal"||$13!=""||$14!="")exit 2
            activation++
        } else if(kind=="for-of"){
            if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
                $11!="unsupported-feature"||$12!="selection"||$13!="EngineCapability"||
                $14!="quickjs-oxide does not declare Test262 feature support: for-of"||!different)exit 3
            forof++
        } else if(kind=="create-realm"){
            if($7!="unsupported-host-create-realm"||$8!="selection"||$9!="HostCapability"||
                $10!="missing execution capabilities: create-realm"||different)exit 4
            realm++
        } else exit 5
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    }
    END{printf "activation=%d forof=%d realm=%d changed=%d outcome=%d detail=%d unchanged=%d",activation,forof,realm,changed,outcome,detail,unchanged}
' "$tmp/classes" "$generated_transition") || die 'focused transition semantics drifted'
[[ "$focused_counts" == 'activation=158 forof=2 realm=4 changed=160 outcome=158 detail=2 unchanged=4' ]] \
    || die "focused transition partition drifted: $focused_counts"
if [[ "$mode" != bless ]]; then
    [[ "$(sha "$generated_transition")" == "$(value transition_receipt_sha256)" ]] \
        || die 'generated transition checksum drifted'
    diff -u "$transition" "$generated_transition"
fi
check_authenticated_inputs
check_prepared_suite_identity

run_full=false
[[ "$mode" == full || "$mode" == bless ]] && run_full=true
if "$run_full"; then
    run_report "$parent" "$parent_full" full "$full_workers"
    check_report_identity "$parent_full" "$parent_sha" 102037 "$all_keys_sha"
    [[ "$(sha "$parent_full")" == "$historical_parent_full_tsv_sha" \
        && "$(sha "${parent_full%.tsv}.jsonl")" == "$historical_parent_full_jsonl_sha" ]] \
        || die 'authoritative previous-live parent full receipt drifted'
    run_report "$candidate" "$candidate_full" full "$full_workers"
    check_report_identity "$candidate_full" "$candidate_sha" 102037 "$all_keys_sha"
    check_canonical_full_report "$candidate_full"
    report_rows "$parent_report" >"$tmp/parent.focused"
    report_rows "$candidate_report" >"$tmp/candidate.focused"
    awk -F'\t' 'NR==FNR{w[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in w)' \
        "$derived" "$parent_full" >"$tmp/parent.full-focused"
    awk -F'\t' 'NR==FNR{w[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in w)' \
        "$derived" "$candidate_full" >"$tmp/candidate.full-focused"
    diff -u "$tmp/parent.focused" "$tmp/parent.full-focused"
    diff -u "$tmp/candidate.focused" "$tmp/candidate.full-focused"
    { awk -F'\t' '{print $1 "\t" $2 "\tchanged"}' "$tmp/activation.keys";
      awk -F'\t' '{print $1 "\t" $2 "\tchanged"}' "$tmp/for-of.keys";
      awk -F'\t' '{print $1 "\t" $2 "\tdeferred"}' "$tmp/create-realm.keys"; } \
        | sort >"$tmp/full.classes"
    join_counts=$(awk -F'\t' -v classes="$tmp/full.classes" -v parent="$parent_full" '
        FILENAME==classes{class[$1 FS $2]=$3;next}
        FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;before++}next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
            for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            different=old[key]!=$0;if(a[7]=="pass"&&$7!="pass")regress++
            if(class[key]=="changed"){
                if(!different)exit 4;changed++;if(a[7]!=$7)outcome++;else detail++
            } else if(different)exit 5
            seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 6;printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
    ' "$tmp/full.classes" "$parent_full" "$candidate_full") \
        || die 'full parent/candidate join drifted'
    [[ "$join_counts" == 'changed=160 outcome=158 detail=2 unchanged=101877 regressions=0' ]] \
        || die "full no-regression delta drifted: $join_counts"
    check_authenticated_inputs
    check_prepared_suite_identity
    if [[ "$mode" == full ]]; then
        check_report_receipt "$parent_full" parent_full
        check_report_receipt "$candidate_full" candidate_full
        echo 'WeakRef/FinalizationRegistry global full gate passes: 102037 rows, 160 exact changes, no pass regression.'
        exit 0
    fi
fi

if [[ "$mode" == bless ]]; then
    cp "$generated_transition" "$transition_draft"
    {
        echo '# Checksum-bound R3cg global admission for WeakRef and FinalizationRegistry.'
        echo 'quickjs=2026-06-04'
        echo "test262=$test262"
        echo "test262_patch_sha256=$patch_sha"
        echo "test262_config_sha256=$config_sha"
        echo 'test262_metadata_records=53125'
        echo "test262_metadata_sha256=$metadata_sha"
        echo 'schema=test262-canonical-classified-v2'
        echo 'mode=both'
        echo 'timeout_ms=30000'
        echo "live_oxide_profile=$live_profile"
        echo "live_oxide_profile_sha256=$candidate_sha"
        echo "canonical_full_baseline=$canonical_baseline"
        echo
        echo "parent_oxide_profile_sha256=$parent_sha"
        echo "candidate_oxide_profile_sha256=$candidate_sha"
        echo 'parent_features=99'
        echo "parent_features_sha256=$parent_features_sha"
        echo 'candidate_features=101'
        echo "candidate_features_sha256=$candidate_features_sha"
        echo 'added_features=2'
        echo "added_features_sha256=$added_features_sha"
        echo 'audited_negative_tests=1157'
        echo "audited_negative_tests_sha256=$audited_negative_tests_sha"
        echo
        echo 'universe_paths=82'
        echo "universe_sha256=$universe_sha"
        echo 'universe_variants=164'
        echo "universe_keys_sha256=$universe_keys_sha"
        echo 'activation_paths=79'
        echo "activation_sha256=$activation_sha"
        echo 'activation_variants=158'
        echo "activation_keys_sha256=$activation_keys_sha"
        echo 'for_of_blocker_paths=1'
        echo "for_of_blocker_sha256=$for_of_sha"
        echo 'for_of_blocker_variants=2'
        echo "for_of_blocker_keys_sha256=$for_of_keys_sha"
        echo 'create_realm_blocker_paths=2'
        echo "create_realm_blockers_sha256=$create_realm_sha"
        echo 'create_realm_blocker_variants=4'
        echo "create_realm_blockers_keys_sha256=$create_realm_keys_sha"
        echo
        echo 'parent_focused_variants=164'
        echo "parent_focused_runnable=$(report_runnable "$parent_report")"
        echo "parent_focused_passes=$(report_count pass "$parent_report")"
        echo "parent_focused_unsupported_feature=$(report_count unsupported-feature "$parent_report")"
        echo "parent_focused_unsupported_host_create_realm=$(report_count unsupported-host-create-realm "$parent_report")"
        echo "parent_focused_tsv_sha256=$(sha "$parent_report")"
        echo "parent_focused_jsonl_sha256=$(sha "${parent_report%.tsv}.jsonl")"
        echo "parent_focused_summary=$(report_summary "$parent_report")"
        echo 'candidate_focused_variants=164'
        echo "candidate_focused_runnable=$(report_runnable "$candidate_report")"
        echo "candidate_focused_passes=$(report_count pass "$candidate_report")"
        echo "candidate_focused_unsupported_feature=$(report_count unsupported-feature "$candidate_report")"
        echo "candidate_focused_unsupported_host_create_realm=$(report_count unsupported-host-create-realm "$candidate_report")"
        echo "candidate_focused_tsv_sha256=$(sha "$candidate_report")"
        echo "candidate_focused_jsonl_sha256=$(sha "${candidate_report%.tsv}.jsonl")"
        echo "candidate_focused_summary=$(report_summary "$candidate_report")"
        echo "transition_receipt_sha256=$(sha "$generated_transition")"
        echo "transition_data_sha256=$(report_rows "$generated_transition" | sha /dev/stdin)"
        echo 'focused_changed=160'
        echo 'focused_outcome_changes=158'
        echo 'focused_detail_changes=2'
        echo 'focused_unchanged=4'
        echo
        echo 'full_variants=102037'
        echo "full_keys_sha256=$all_keys_sha"
        echo 'full_changed=160'
        echo 'full_outcome_changes=158'
        echo 'full_detail_changes=2'
        echo 'full_unchanged=101877'
        echo 'full_pass_regressions=0'
        echo "parent_full_runnable=$(report_runnable "$parent_full")"
        echo "parent_full_passes=$(report_count pass "$parent_full")"
        echo "parent_full_tsv_sha256=$(sha "$parent_full")"
        echo "parent_full_jsonl_sha256=$(sha "${parent_full%.tsv}.jsonl")"
        echo "parent_full_summary=$(report_summary "$parent_full")"
        echo "candidate_full_runnable=$(report_runnable "$candidate_full")"
        echo "candidate_full_passes=$(report_count pass "$candidate_full")"
        echo "candidate_full_tsv_sha256=$(sha "$candidate_full")"
        echo "candidate_full_jsonl_sha256=$(sha "${candidate_full%.tsv}.jsonl")"
        echo "candidate_full_summary=$(report_summary "$candidate_full")"
    } >"$baseline_draft"
    echo "Reviewed baseline draft: $baseline_draft"
    echo "Reviewed transition draft: $transition_draft"
    exit 0
fi

echo 'WeakRef/FinalizationRegistry global gate passes: QuickJS 164/164; Oxide 0/164 -> 158/164 with 2 detail-only and 4 host-priority rows unchanged.'
