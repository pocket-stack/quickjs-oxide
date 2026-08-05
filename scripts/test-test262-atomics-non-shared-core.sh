#!/usr/bin/env bash
# Reproduce the R3de manifest-scoped non-shared Atomics evidence gate.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-atomics-non-shared-core-baseline.txt
predecessor_baseline=tests/test262-atomics-metadata-gaps-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
global_profile=compat/test262-oxide.conf
scoped_profile=tests/test262-atomics-non-shared-core.conf
core=tests/test262-atomics-non-shared-core.txt
deferred=tests/test262-atomics-shared-deferred.txt
universe=tests/test262-atomics-universe.tsv
parent_report=tests/test262-atomics-non-shared-core-parent.tsv
candidate_report=tests/test262-atomics-non-shared-core.tsv
transition=tests/test262-atomics-non-shared-core-transitions.tsv
parent_replay=target/test262-atomics-non-shared-core-parent-replay.tsv
candidate_replay=target/test262-atomics-non-shared-core-replay.tsv
full_report=target/test262-atomics-non-shared-core-full.tsv
oracle_log=target/test262-atomics-non-shared-core-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full=${TEST262_REUSE_FULL_REPORT:-false}
runner_override=${TEST262_RUNNER:-}

baseline_lines=133
baseline_sha=3ba60debc3bd6fec6339a0d61f1b9ffca7f00176067a32eff42e62da95790e6b
predecessor_lines=106
predecessor_sha=cc6169a0ee5e5a69c647a405b9d9c334471130b7d5c4267845a419bcca49f6a9
canonical_lines=8
canonical_sha=3b59afb90b22434a6ae2fcdec94b67b3c7c3b74142d3cc73e5c315c9aa50e5a3

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate frozen inputs/receipts and both QuickJS partitions\n'
    printf '  default  additionally replay the global parent and scoped candidate\n'
    printf '  --full   additionally replay the unchanged 102037-row global vector (canonical: TEST262_FULL_WORKERS=2)\n'
    printf '           TEST262_REUSE_FULL_REPORT=true reauthenticates an existing full report\n'
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
    || { echo 'error: invalid Test262 worker count' >&2; exit 2; }
[[ "$reuse_full" == false || "$reuse_full" == true ]] \
    || { echo 'error: TEST262_REUSE_FULL_REPORT must be true or false' >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
value_from() {
    awk -F= -v wanted="$2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
value() { value_from "$baseline" "$1"; }
predecessor_value() { value_from "$predecessor_baseline" "$1"; }
canonical_value() { value_from "$canonical_baseline" "$1"; }
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
json_result_rows() { awk '/^\{"kind":"result"/' "$1"; }
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
rows_for_paths() {
    awk -F'\t' 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' "$1" "$2"
}
json_rows_for_paths() {
    awk 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        /^\{"kind":"result"/{
            if(!match($0,/"path":"[^"]*"/))exit 2
            path=substr($0,RSTART+8,RLENGTH-9)
            if(path in wanted)print
        }' "$1" "$2"
}
metadata_block() {
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$1"
}
metadata_features() {
    metadata_block "$1" | awk '
        /^features:[[:space:]]*\[/ {
            sub(/^features:[[:space:]]*\[/, "");sub(/\][[:space:]]*$/, "")
            count=split($0,values,/,[[:space:]]*/)
            for(i=1;i<=count;i++)if(values[i]!="")print values[i]
            exit
        }
        /^features:[[:space:]]*$/ {inside=1;next}
        inside&&/^[[:space:]]*-[[:space:]]*/ {
            sub(/^[[:space:]]*-[[:space:]]*/, "");print;next
        }
        inside {exit}
    '
}
csv_has() {
    case ,$1, in
        *,$2,*) return 0 ;;
        *) return 1 ;;
    esac
}
source_or_include_evaluates_sab() {
    local test_path=$1 includes=$2
    if grep -Eq 'new[[:space:]]+SharedArrayBuffer' "$suite/$test_path"; then
        return 0
    fi
    csv_has "$includes" testAtomics.js \
        && grep -Eq 'testWithAtomicsNonViewValues[[:space:]]*\(' "$suite/$test_path"
}
category_count() {
    awk -F'\t' -v wanted="$1" 'NR>1&&$2==wanted{count++} END{print count+0}' \
        "$universe"
}
ledger_source_rows_for_paths() {
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next}
        FNR>1&&($1 in wanted){print $1 "\t" $6}' "$1" "$universe"
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
    local report=$1 profile_sha=$2 prefix=$3 json=${1%.tsv}.jsonl
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value core_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value core_keys_sha256)" \
        && "$(report_rows "$report" | sha /dev/stdin)" == "$(value "${prefix}_rows_sha256")" \
        && "$(json_result_rows "$json" | lines /dev/stdin)" == "$(value core_variants)" \
        && "$(json_result_rows "$json" | sha /dev/stdin)" == "$(value "${prefix}_json_rows_sha256")" \
        && "$(report_summary "$report")" == "$(value "${prefix}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${prefix}_summary")" ]] \
        || die "classified report drifted: $report"
}

check_profiles() {
    check_file "$global_profile" "$(value global_profile_lines)" \
        "$(value global_profile_sha256)"
    check_file "$scoped_profile" "$(value scoped_profile_lines)" \
        "$(value scoped_profile_sha256)"
    profile_section features "$global_profile" >"$tmp/global.features"
    profile_section features "$scoped_profile" >"$tmp/scoped.features"
    profile_section audited-negative-tests "$scoped_profile" >"$tmp/scoped.negatives"
    profile_section execution "$scoped_profile" >"$tmp/scoped.execution"
    comm -23 "$tmp/scoped.features" "$tmp/global.features" >"$tmp/scoped.only"
    [[ "$(lines "$tmp/global.features")" == "$(value global_profile_features)" \
        && "$(sha "$tmp/global.features")" == "$(value global_profile_features_sha256)" \
        && "$(lines "$tmp/scoped.features")" == "$(value scoped_profile_features)" \
        && "$(sha "$tmp/scoped.features")" == "$(value scoped_profile_features_sha256)" \
        && "$(lines "$tmp/scoped.only")" == "$(value scoped_only_features)" \
        && "$(sha "$tmp/scoped.only")" == "$(value scoped_only_features_sha256)" \
        && ! -s "$tmp/scoped.negatives" && ! -s "$tmp/scoped.execution" ]] \
        || die 'R3de profile inventory drifted'
    diff -u <(printf '%s\n' Atomics Atomics.pause SharedArrayBuffer) "$tmp/scoped.only"
}

check_manifests_and_sources() {
    check_file "$core" "$(value core_paths)" "$(value core_paths_sha256)"
    check_file "$deferred" "$(value deferred_paths)" "$(value deferred_paths_sha256)"
    sort -c "$core" || die 'R3de core manifest is not bytewise sorted'
    sort -c "$deferred" || die 'R3de deferred manifest is not bytewise sorted'
    [[ -z "$(uniq -d "$core")" && -z "$(uniq -d "$deferred")" \
        && -z "$(comm -12 "$core" "$deferred")" ]] \
        || die 'R3de manifests contain duplicates or overlap'
    cat "$core" "$deferred" | sort >"$tmp/combined.paths"
    [[ "$(lines "$tmp/combined.paths")" == "$(value combined_paths)" \
        && "$(sha "$tmp/combined.paths")" == "$(value combined_paths_sha256)" ]] \
        || die 'R3de manifest partition drifted'

    : >"$tmp/source-audit.tsv"
    : >"$tmp/core.features"
    : >"$tmp/deferred.features"
    : >"$tmp/core.sab-metadata"
    local test_path source
    while IFS= read -r test_path; do
        source=$suite/$test_path
        [[ -f "$source" ]] || die "pinned R3de source is missing: $test_path"
        printf '%s\t%s\n' "$test_path" "$(sha "$source")" >>"$tmp/source-audit.tsv"
        metadata_features "$test_path" >>"$tmp/core.features"
        if metadata_features "$test_path" | grep -Fxq SharedArrayBuffer; then
            printf '%s\n' "$test_path" >>"$tmp/core.sab-metadata"
        fi
        ! grep -Eq 'new[[:space:]]+SharedArrayBuffer' "$source" \
            || die "core path evaluates SharedArrayBuffer: $test_path"
    done <"$core"
    while IFS= read -r test_path; do
        source=$suite/$test_path
        [[ -f "$source" ]] || die "pinned deferred source is missing: $test_path"
        printf '%s\t%s\n' "$test_path" "$(sha "$source")" >>"$tmp/source-audit.tsv"
        metadata_features "$test_path" >>"$tmp/deferred.features"
        grep -Eq 'new[[:space:]]+SharedArrayBuffer' "$source" \
            || die "deferred path no longer evaluates SharedArrayBuffer: $test_path"
    done <"$deferred"
    sort -u "$tmp/core.features" -o "$tmp/core.features"
    sort -u "$tmp/deferred.features" -o "$tmp/deferred.features"
    cat "$tmp/core.features" "$tmp/deferred.features" | sort -u >"$tmp/combined.features"
    [[ "$(sha "$tmp/source-audit.tsv")" == "$(value source_audit_projection_sha256)" \
        && "$(sha "$tmp/core.features")" == "$(value core_metadata_features_sha256)" \
        && "$(sha "$tmp/deferred.features")" == "$(value deferred_metadata_features_sha256)" \
        && "$(sha "$tmp/combined.features")" == "$(value combined_metadata_features_sha256)" \
        && "$(lines "$tmp/core.sab-metadata")" == 1 \
        && "$(<"$tmp/core.sab-metadata")" == "$(value metadata_only_sab_path)" ]] \
        || die 'R3de metadata or source boundary drifted'
    diff -u "$tmp/scoped.features" "$tmp/core.features"
}

check_atomics_universe() {
    check_file "$universe" "$(value universe_ledger_lines)" \
        "$(value universe_ledger_sha256)"
    [[ "$(head -n 1 "$universe")" == \
        $'path\tcategory\tincludes\tflags\tfeatures\tsource_sha256' ]] \
        || die 'R3de Atomics universe ledger header drifted'

    tail -n +2 "$universe" | cut -f1 >"$tmp/universe.paths"
    tail -n +2 "$universe" | cut -f1,6 >"$tmp/universe.source.tsv"
    sort -c "$tmp/universe.paths" || die 'R3de Atomics universe is not bytewise sorted'
    [[ -z "$(uniq -d "$tmp/universe.paths")" \
        && "$(lines "$tmp/universe.paths")" == "$(value universe_paths)" \
        && "$(sha "$tmp/universe.paths")" == "$(value universe_paths_sha256)" \
        && "$(sha "$tmp/universe.source.tsv")" == \
            "$(value universe_source_projection_sha256)" ]] \
        || die 'R3de Atomics universe path or source projection drifted'

    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata inventory drifted'
    tr '\0' '\t' <"$tmp/metadata.bin" | awk -F'\t' '
        index("," $4 ",", ",Atomics,") ||
        index("," $4 ",", ",Atomics.pause,") {
            print $1 "\t" $2 "\t" $3 "\t" $4
        }' >"$tmp/universe.metadata.tsv"
    awk -F'\t' 'NR>1{print $1 "\t" $3 "\t" $4 "\t" $5}' "$universe" \
        >"$tmp/universe.ledger-metadata.tsv"
    cmp -s "$tmp/universe.metadata.tsv" "$tmp/universe.ledger-metadata.tsv" \
        || die 'R3de Atomics universe no longer exhausts pinned metadata'
    awk -F'\t' 'NR>1&&index("," $5 ",", ",Atomics.pause,"){print $1}' \
        "$universe" >"$tmp/universe.pause.paths"
    [[ "$(awk -F'\t' 'NR>1&&("," $4 ",") ~ \
            /,(raw|module|noStrict|onlyStrict),/{count++} END{print count+0}' \
            "$universe")" == 0 \
        && "$(value universe_variants)" -eq \
            $(( $(value universe_paths) * 2 )) \
        && "$(lines "$tmp/universe.pause.paths")" == "$(value universe_pause_paths)" \
        && "$(sha "$tmp/universe.pause.paths")" == \
            "$(value universe_pause_paths_sha256)" \
        && "$(value universe_pause_variants)" -eq \
            $(( $(value universe_pause_paths) * 2 )) ]] \
        || die 'R3de Atomics universe variant projection drifted'
    [[ "$(sha "$suite/harness/testAtomics.js")" == \
        "$(value test_atomics_harness_sha256)" ]] \
        || die 'R3de testAtomics.js harness drifted'

    : >"$tmp/universe.safe.paths"
    : >"$tmp/universe.shared.paths"
    : >"$tmp/universe.direct-sab.paths"
    : >"$tmp/universe.indirect-sab.paths"
    awk -F'\t' 'NR>1{print $1 "|" $2 "|" $3 "|" $4 "|" $5 "|" $6}' "$universe" \
        >"$tmp/universe.pipe"
    local test_path category includes flags features pinned_source source
    while IFS='|' read -r test_path category includes flags features pinned_source; do
        source=$suite/$test_path
        [[ -f "$source" && "$(sha "$source")" == "$pinned_source" ]] \
            || die "R3de Atomics universe source drifted: $test_path"
        case $category in
            shared-*)
                source_or_include_evaluates_sab "$test_path" "$includes" \
                    || die "shared Atomics SAB evidence drifted: $test_path"
                if ! grep -Eq 'new[[:space:]]+SharedArrayBuffer' "$source"; then
                    printf '%s\n' "$test_path" >>"$tmp/universe.indirect-sab.paths"
                else
                    printf '%s\n' "$test_path" >>"$tmp/universe.direct-sab.paths"
                fi
                ;;
        esac
        case $category in
            non-shared-no-sab-tag)
                grep -Fxq "$test_path" "$core" \
                    || die "non-shared Atomics path escaped the core: $test_path"
                ! csv_has "$features" SharedArrayBuffer \
                    && ! csv_has "$features" Atomics.waitAsync \
                    && ! grep -Eq 'new[[:space:]]+SharedArrayBuffer' "$source" \
                    || die "non-shared Atomics evidence drifted: $test_path"
                printf '%s\n' "$test_path" >>"$tmp/universe.safe.paths"
                ;;
            non-shared-metadata-only-sab)
                [[ "$test_path" == "$(value metadata_only_sab_path)" ]] \
                    && grep -Fxq "$test_path" "$core" \
                    && csv_has "$features" SharedArrayBuffer \
                    && ! csv_has "$features" Atomics.waitAsync \
                    && ! grep -Eq 'new[[:space:]]+SharedArrayBuffer' "$source" \
                    || die "metadata-only SAB evidence drifted: $test_path"
                printf '%s\n' "$test_path" >>"$tmp/universe.safe.paths"
                ;;
            shared-no-extra-host)
                ! grep -Eq '\$262\.(agent|createRealm|canBlock)' "$source" \
                    || die "shared Atomics evidence drifted: $test_path"
                printf '%s\n' "$test_path" >>"$tmp/universe.shared.paths"
                ;;
            shared-agent)
                grep -Fq '$262.agent' "$source" \
                    || die "Atomics agent evidence drifted: $test_path"
                printf '%s\n' "$test_path" >>"$tmp/universe.shared.paths"
                ;;
            shared-can-block-false)
                [[ "$test_path" == \
                        test/built-ins/Atomics/wait/cannot-suspend-throws.js \
                    || "$test_path" == \
                        test/built-ins/Atomics/wait/bigint/cannot-suspend-throws.js ]] \
                    && [[ "$flags" == CanBlockIsFalse ]] \
                    || die "Atomics CanBlock evidence drifted: $test_path"
                printf '%s\n' "$test_path" >>"$tmp/universe.shared.paths"
                ;;
            wait-async)
                csv_has "$features" Atomics.waitAsync \
                    && [[ "$test_path" == test/built-ins/Atomics/waitAsync/* ]] \
                    || die "Atomics.waitAsync evidence drifted: $test_path"
                ;;
            *) die "unknown R3de Atomics universe category: $category" ;;
        esac
    done <"$tmp/universe.pipe"

    [[ "$(category_count non-shared-no-sab-tag)" == \
            "$(value universe_non_shared_no_sab_tag)" \
        && "$(category_count non-shared-metadata-only-sab)" == \
            "$(value universe_non_shared_metadata_only_sab)" \
        && "$(category_count shared-no-extra-host)" == \
            "$(value universe_shared_no_extra_host)" \
        && "$(category_count shared-agent)" == "$(value universe_shared_agent)" \
        && "$(category_count shared-can-block-false)" == \
            "$(value universe_shared_can_block_false)" \
        && "$(category_count wait-async)" == "$(value universe_wait_async)" \
        && "$(lines "$tmp/universe.direct-sab.paths")" == \
            "$(value universe_direct_sab_paths)" \
        && "$(sha "$tmp/universe.direct-sab.paths")" == \
            "$(value universe_direct_sab_paths_sha256)" \
        && "$(lines "$tmp/universe.indirect-sab.paths")" == \
            "$(value universe_indirect_sab_paths)" \
        && "$(sha "$tmp/universe.indirect-sab.paths")" == \
            "$(value universe_indirect_sab_paths_sha256)" \
        && "$(lines "$tmp/universe.safe.paths")" == "$(value universe_safe_paths)" \
        && "$(sha "$tmp/universe.safe.paths")" == \
            "$(value universe_safe_paths_sha256)" \
        && "$(lines "$tmp/universe.shared.paths")" == \
            "$(value universe_shared_paths)" \
        && "$(sha "$tmp/universe.shared.paths")" == \
            "$(value universe_shared_paths_sha256)" ]] \
        || die 'R3de Atomics universe category counts drifted'
    grep -Fvx "$(predecessor_value source_detached_path)" "$core" \
        >"$tmp/core.metadata.paths"
    cmp -s "$tmp/core.metadata.paths" "$tmp/universe.safe.paths" \
        || die 'R3de green manifest no longer matches the metadata audit'

    local detached_path cross_path detached_source cross_source supported feature
    detached_path=$(predecessor_value source_detached_path)
    cross_path=$(predecessor_value source_cross_path)
    detached_source=$suite/$detached_path
    cross_source=$suite/$cross_path
    [[ "$(lines "$detached_source")" == "$(predecessor_value source_detached_lines)" \
        && "$(sha "$detached_source")" == \
            "$(predecessor_value source_detached_sha256)" \
        && "$(lines "$cross_source")" == "$(predecessor_value source_cross_lines)" \
        && "$(sha "$cross_source")" == "$(predecessor_value source_cross_sha256)" ]] \
        && ! grep -Fxq "$detached_path" "$tmp/universe.paths" \
        && ! grep -Fxq "$cross_path" "$tmp/universe.paths" \
        && grep -Fxq "$detached_path" "$core" \
        && ! grep -Fxq "$cross_path" "$core" \
        && ! grep -Fxq "$cross_path" "$deferred" \
        && grep -Fq '$262.createRealm' "$cross_source" \
        && grep -Fq 'SharedArrayBuffer' "$cross_source" \
        || die 'R3de metadata-less staging boundary drifted'

    : >"$tmp/broad.metadata.paths"
    : >"$tmp/broad.metadata.safe.paths"
    : >"$tmp/broad.metadata.hidden.paths"
    while IFS='|' read -r test_path category includes flags features pinned_source; do
        csv_has "$features" Atomics || continue
        supported=true
        while IFS= read -r feature; do
            [[ "$feature" == Atomics ]] && continue
            if ! grep -Fxq "$feature" "$tmp/global.features"; then
                supported=false
                break
            fi
        done < <(printf '%s\n' "$features" | tr ',' '\n')
        [[ "$supported" == true ]] || continue
        printf '%s\n' "$test_path" >>"$tmp/broad.metadata.paths"
        case $category in
            non-shared-*) printf '%s\n' "$test_path" >>"$tmp/broad.metadata.safe.paths" ;;
            shared-*) printf '%s\n' "$test_path" >>"$tmp/broad.metadata.hidden.paths" ;;
            *) die "broad Atomics metadata closure escaped its audited categories: $test_path" ;;
        esac
    done <"$tmp/universe.pipe"
    ledger_source_rows_for_paths "$tmp/broad.metadata.paths" \
        >"$tmp/broad.metadata-source.tsv"
    ledger_source_rows_for_paths "$tmp/broad.metadata.safe.paths" \
        >"$tmp/broad.metadata.safe-source.tsv"
    ledger_source_rows_for_paths "$tmp/broad.metadata.hidden.paths" \
        >"$tmp/broad.metadata.hidden-source.tsv"
    [[ "$(lines "$tmp/broad.metadata.paths")" == \
            "$(value broad_atomics_metadata_paths)" \
        && "$(value broad_atomics_metadata_variants)" -eq \
            $(( $(value broad_atomics_metadata_paths) * 2 )) \
        && "$(sha "$tmp/broad.metadata.paths")" == \
            "$(value broad_atomics_metadata_paths_sha256)" \
        && "$(sha "$tmp/broad.metadata-source.tsv")" == \
            "$(value broad_atomics_metadata_source_projection_sha256)" \
        && "$(lines "$tmp/broad.metadata.safe.paths")" == \
            "$(value broad_atomics_metadata_safe_paths)" \
        && "$(value broad_atomics_metadata_safe_variants)" -eq \
            $(( $(value broad_atomics_metadata_safe_paths) * 2 )) \
        && "$(sha "$tmp/broad.metadata.safe.paths")" == \
            "$(value broad_atomics_metadata_safe_paths_sha256)" \
        && "$(sha "$tmp/broad.metadata.safe-source.tsv")" == \
            "$(value broad_atomics_metadata_safe_source_projection_sha256)" \
        && "$(lines "$tmp/broad.metadata.hidden.paths")" == \
            "$(value broad_atomics_metadata_hidden_shared_paths)" \
        && "$(value broad_atomics_metadata_hidden_shared_variants)" -eq \
            $(( $(value broad_atomics_metadata_hidden_shared_paths) * 2 )) \
        && "$(sha "$tmp/broad.metadata.hidden.paths")" == \
            "$(value broad_atomics_metadata_hidden_shared_paths_sha256)" \
        && "$(sha "$tmp/broad.metadata.hidden-source.tsv")" == \
            "$(value broad_atomics_metadata_hidden_source_projection_sha256)" ]] \
        || die 'R3de broad Atomics metadata closure drifted'

    local host_preempted supplemental
    host_preempted=$(value broad_atomics_host_preempted_path)
    supplemental=$(value broad_atomics_supplemental_path)
    [[ "$supplemental" == "$detached_path" \
        && "$host_preempted" == test/built-ins/Atomics/wait/good-views.js \
        && "$(awk -F'\t' -v path="$host_preempted" \
            '$1==path{print $2 "\t" $3;found++} END{if(found!=1)exit 1}' \
            "$universe")" == $'shared-agent\tatomicsHelper.js' ]] \
        || die 'R3de broad Atomics precedence exceptions drifted'
    grep -Fvx "$host_preempted" "$tmp/broad.metadata.paths" \
        >"$tmp/broad.transition.paths.tmp"
    printf '%s\n' "$supplemental" >>"$tmp/broad.transition.paths.tmp"
    sort "$tmp/broad.transition.paths.tmp" >"$tmp/broad.transition.paths"
    cp "$tmp/broad.metadata.safe.paths" "$tmp/broad.transition.safe.paths.tmp"
    printf '%s\n' "$supplemental" >>"$tmp/broad.transition.safe.paths.tmp"
    sort "$tmp/broad.transition.safe.paths.tmp" >"$tmp/broad.transition.safe.paths"
    grep -Fvx "$host_preempted" "$tmp/broad.metadata.hidden.paths" \
        >"$tmp/broad.transition.hidden.paths"
    awk -F'\t' -v path="$host_preempted" '$1!=path' \
        "$tmp/broad.metadata-source.tsv" >"$tmp/broad.transition-source.tsv.tmp"
    printf '%s\t%s\n' "$supplemental" "$(sha "$detached_source")" \
        >>"$tmp/broad.transition-source.tsv.tmp"
    sort "$tmp/broad.transition-source.tsv.tmp" >"$tmp/broad.transition-source.tsv"
    cp "$tmp/broad.metadata.safe-source.tsv" \
        "$tmp/broad.transition.safe-source.tsv.tmp"
    printf '%s\t%s\n' "$supplemental" "$(sha "$detached_source")" \
        >>"$tmp/broad.transition.safe-source.tsv.tmp"
    sort "$tmp/broad.transition.safe-source.tsv.tmp" \
        >"$tmp/broad.transition.safe-source.tsv"
    awk -F'\t' -v path="$host_preempted" '$1!=path' \
        "$tmp/broad.metadata.hidden-source.tsv" \
        >"$tmp/broad.transition.hidden-source.tsv"
    comm -23 "$tmp/broad.metadata.paths" "$tmp/broad.transition.paths" \
        >"$tmp/broad.metadata-only.paths"
    comm -13 "$tmp/broad.metadata.paths" "$tmp/broad.transition.paths" \
        >"$tmp/broad.transition-only.paths"
    [[ "$(<"$tmp/broad.metadata-only.paths")" == "$host_preempted" \
        && "$(<"$tmp/broad.transition-only.paths")" == "$supplemental" \
        && "$(lines "$tmp/broad.transition.paths")" == \
            "$(value broad_atomics_transition_paths)" \
        && "$(value broad_atomics_transition_variants)" -eq \
            $(( $(value broad_atomics_transition_paths) * 2 )) \
        && "$(sha "$tmp/broad.transition.paths")" == \
            "$(value broad_atomics_transition_paths_sha256)" \
        && "$(sha "$tmp/broad.transition-source.tsv")" == \
            "$(value broad_atomics_transition_source_projection_sha256)" \
        && "$(lines "$tmp/broad.transition.safe.paths")" == \
            "$(value broad_atomics_transition_safe_paths)" \
        && "$(value broad_atomics_transition_safe_variants)" -eq \
            $(( $(value broad_atomics_transition_safe_paths) * 2 )) \
        && "$(sha "$tmp/broad.transition.safe.paths")" == \
            "$(value broad_atomics_transition_safe_paths_sha256)" \
        && "$(sha "$tmp/broad.transition.safe-source.tsv")" == \
            "$(value broad_atomics_transition_safe_source_projection_sha256)" \
        && "$(lines "$tmp/broad.transition.hidden.paths")" == \
            "$(value broad_atomics_transition_hidden_shared_paths)" \
        && "$(value broad_atomics_transition_hidden_shared_variants)" -eq \
            $(( $(value broad_atomics_transition_hidden_shared_paths) * 2 )) \
        && "$(sha "$tmp/broad.transition.hidden.paths")" == \
            "$(value broad_atomics_transition_hidden_shared_paths_sha256)" \
        && "$(sha "$tmp/broad.transition.hidden-source.tsv")" == \
            "$(value broad_atomics_transition_hidden_source_projection_sha256)" ]] \
        || die 'R3de broad Atomics precedence-aware transition drifted'
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if($7!="pass"&&$11=="pass")gain++
        if($7=="pass"&&$11!="pass")regress++
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d",changed,outcome,detail,unchanged,gain,regress}' "$1"
}

check_receipts() {
    check_file "$parent_report" "$(value parent_report_lines)" "$(value parent_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
        "$(value parent_jsonl_sha256)"
    check_file "$candidate_report" "$(value candidate_report_lines)" \
        "$(value candidate_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
        "$(value candidate_jsonl_sha256)"
    verify_report "$parent_report" "$(value global_profile_sha256)" parent
    verify_report "$candidate_report" "$(value scoped_profile_sha256)" candidate
    [[ "$(report_runnable "$parent_report")" == 0 \
        && "$(report_count unsupported-feature "$parent_report")" == "$(value core_variants)" \
        && "$(report_runnable "$candidate_report")" == "$(value core_variants)" \
        && "$(report_count pass "$candidate_report")" == "$(value core_variants)" ]] \
        || die 'R3de focused semantic counts drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability")exit 2
    }' "$parent_report" || die 'R3de parent outcomes drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2
    }' "$candidate_report" || die 'R3de candidate outcomes drifted'

    report_rows "$parent_report" >"$tmp/parent.rows.tsv"
    report_rows "$candidate_report" >"$tmp/candidate.rows.tsv"
    awk -F'\t' -v OFS='\t' '
        NR==FNR{
            key=$1 FS $2
            if(key in before)exit 2
            before[key]=$0;order[++before_count]=key
            next
        }
        {
            key=$1 FS $2
            if(!(key in before)||key in after)exit 2
            after[key]=$0;after_count++
        }
        END{
            if(before_count!=after_count)exit 2
            for(row=1;row<=before_count;row++){
                key=order[row]
                if(!(key in after))exit 2
                split(before[key],left,FS);split(after[key],right,FS)
                for(field=1;field<=6;field++)if(left[field]!=right[field])exit 2
                print left[1],left[2],left[3],left[4],left[5],left[6], \
                    left[7],left[8],left[9],left[10], \
                    right[7],right[8],right[9],right[10]
            }
        }' "$tmp/parent.rows.tsv" "$tmp/candidate.rows.tsv" \
        >"$tmp/joined-transition.tsv" \
        || die 'R3de focused reports cannot be joined exactly'

    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    report_rows "$transition" >"$tmp/frozen-transition.tsv"
    cmp -s "$tmp/joined-transition.tsv" "$tmp/frozen-transition.tsv" \
        || die 'R3de transition is not the exact parent/candidate join'
    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" parent_profile_sha256)" == "$(value global_profile_sha256)" \
        && "$(header "$transition" candidate_profile_sha256)" == "$(value scoped_profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value core_paths_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)" ]] \
        || die 'R3de transition drifted'
}

check_history_and_upstream() {
    check_file "$predecessor_baseline" "$predecessor_lines" "$predecessor_sha"
    check_file "$canonical_baseline" "$canonical_lines" "$canonical_sha"
    [[ "$(value milestone_kind)" == scoped-evidence-only \
        && "$(predecessor_value profile_sha256)" == "$(value global_profile_sha256)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == "$(value full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == "$(value full_jsonl_sha256)" \
        && "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value full_runnable)" \
        && "$(canonical_value passes)" == "$(value full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value full_summary)" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$global_profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value global_profile_sha256)" ]] \
        || die 'R3de historical or pinned-upstream bridge drifted'
}

verify_fail_closed_selection() {
    local common=(--suite "$suite" --config "$source_dir/test262.conf"
        --oxide-profile "$root/$scoped_profile" --report "$tmp/rejected.tsv"
        --mode both --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures)
    if "$runner" "${common[@]}" --manifest "$root/$deferred" >/dev/null 2>&1; then
        die 'scoped profile accepted the deferred SAB manifest'
    fi
    if "$runner" "${common[@]}" --all >/dev/null 2>&1; then
        die 'scoped profile accepted --all'
    fi
    if "$runner" "${common[@]}" --test test/built-ins/Atomics/add/descriptor.js \
        >/dev/null 2>&1; then
        die 'scoped profile accepted --test'
    fi
}

verify_quickjs() {
    local test_path
    local -a files=()
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$core"
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$deferred"
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3de partitions'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_passes) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes all R3de variants'
    fi
}

run_focused_report() {
    local profile=$1 output=$2 allow_failures=$3
    local -a args=(--suite "$suite" --config "$source_dir/test262.conf"
        --oxide-profile "$root/$profile" --manifest "$root/$core"
        --report "$root/$output" --mode both --timeout-ms "$(value timeout_ms)"
        --workers "$workers")
    [[ "$allow_failures" == false ]] || args+=(--allow-failures)
    "$runner" "${args[@]}" >/dev/null
}

replay_focused() {
    run_focused_report "$global_profile" "$parent_replay" true
    run_focused_report "$scoped_profile" "$candidate_replay" false
    cmp -s "$parent_report" "$parent_replay" \
        && cmp -s "${parent_report%.tsv}.jsonl" "${parent_replay%.tsv}.jsonl" \
        || die 'R3de parent replay drifted'
    cmp -s "$candidate_report" "$candidate_replay" \
        && cmp -s "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl" \
        || die 'R3de candidate replay drifted'
}

replay_full() {
    if [[ "$reuse_full" == false ]]; then
        rm -f -- "$full_report" "${full_report%.tsv}.jsonl"
        "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
            --oxide-profile "$root/$global_profile" --all --report "$root/$full_report" \
            --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
            --allow-failures >/dev/null
    fi
    local json=${full_report%.tsv}.jsonl
    [[ -f "$full_report" && -f "$json" ]] \
        || die 'R3de full global reports are missing'
    [[ "$(lines "$full_report")" == "$(value full_tsv_lines)" \
        && "$(lines "$json")" == "$(value full_jsonl_lines)" \
        && "$(sha "$full_report")" == "$(value full_tsv_sha256)" \
        && "$(sha "$json")" == "$(value full_jsonl_sha256)" \
        && "$(header "$full_report" oxide_profile_sha256)" == "$(value global_profile_sha256)" \
        && "$(report_rows "$full_report" | lines /dev/stdin)" == "$(value full_variants)" \
        && "$(report_keys "$full_report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_runnable "$full_report")" == "$(value full_runnable)" \
        && "$(report_count pass "$full_report")" == "$(value full_passes)" \
        && "$(report_summary "$full_report")" == "$(value full_summary)" \
        && "$(computed_summary "$full_report")" == "$(value full_summary)" ]] \
        || die 'R3de full global replay drifted'
    rows_for_paths "$core" "$full_report" >"$tmp/full.core.tsv"
    report_rows "$parent_report" >"$tmp/parent.core.tsv"
    json_rows_for_paths "$core" "$json" >"$tmp/full.core.jsonl"
    json_result_rows "${parent_report%.tsv}.jsonl" >"$tmp/parent.core.jsonl"
    diff -u "$tmp/parent.core.tsv" "$tmp/full.core.tsv"
    diff -u "$tmp/parent.core.jsonl" "$tmp/full.core.jsonl"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3de.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

if [[ -n "$runner_override" ]]; then
    runner=$runner_override
else
    cargo build --quiet --locked --release --bin run-test262
    target_dir=${CARGO_TARGET_DIR:-target}
    case $target_dir in
        /*) ;;
        *) target_dir=$root/$target_dir ;;
    esac
    runner=$target_dir/release/run-test262
fi
[[ -x "$runner" ]] || die "Test262 runner is not executable: $runner"

check_file "$baseline" "$baseline_lines" "$baseline_sha"
check_profiles
check_manifests_and_sources
check_receipts
check_history_and_upstream
check_atomics_universe
verify_fail_closed_selection
verify_quickjs

case $mode in
    check) ;;
    focused) replay_focused ;;
    full) replay_focused; replay_full ;;
esac

printf 'R3de non-shared Atomics: core=%s/%s QuickJS=%s deferred=%s global-profile=unchanged\n' \
    "$(value core_variants)" "$(value core_variants)" \
    "$(value quickjs_passes)" "$(value deferred_variants)"
