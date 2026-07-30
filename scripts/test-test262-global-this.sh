#!/usr/bin/env bash
# Reproduce the R3bo focused globalThis admission gate.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-global-this-baseline.txt
parent_profile=tests/test262-global-this-parent.conf
candidate_profile=tests/test262-global-this-candidate.conf
universe_manifest=tests/test262-global-this.txt
activation_manifest=tests/test262-global-this-activation.txt
deferred_manifest=tests/test262-global-this-deferred.txt
transition_receipt=tests/test262-global-this-transitions.tsv
before_report=target/test262-global-this-before.tsv
before_json_report=target/test262-global-this-before.jsonl
candidate_report=target/test262-global-this-candidate.tsv
candidate_json_report=target/test262-global-this-candidate.jsonl
quickjs_log=target/test262-global-this-quickjs.log
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check|--bless]\n' "${0##*/}"
    printf '  --check  verify frozen inputs and the pinned QuickJS oracle; skip Oxide\n'
    printf '  --bless  record receipts only after the exact green before/after join\n'
}

check_only=false
bless=false
case ${1-} in
    "") ;;
    --check) check_only=true ;;
    --bless) bless=true ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }

read_value() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$baseline"
}

expect_value() {
    local key=$1 expected=$2
    [[ "$(read_value "$key")" == "$expected" ]] || {
        printf 'error: R3bo baseline identity drifted for %s\n' "$key" >&2
        exit 1
    }
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    else
        shasum -a 256 | awk '{ print $1 }'
    fi
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$1"
}

profile_section() {
    local profile=$1 section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

metadata_block() {
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$1"
}

metadata_list() {
    local test_path=$1 key=$2
    metadata_block "$test_path" | awk -v key="$key" '
        $0 ~ ("^" key ":[[:space:]]*\\[") {
            sub("^[^:]+:[[:space:]]*\\[", "")
            sub(/\][[:space:]]*$/, "")
            count=split($0, values, /,[[:space:]]*/)
            for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            exit
        }
        $0 ~ ("^" key ":[[:space:]]*$") { inside=1; next }
        inside && /^[[:space:]]+-[[:space:]]+/ {
            sub(/^[[:space:]]+-[[:space:]]+/, "")
            print
            next
        }
        inside { exit }
    '
}

variant_keys() {
    local test_path flags
    while IFS= read -r test_path; do
        [[ -z "$test_path" ]] && continue
        flags=$(metadata_list "$test_path" flags)
        if grep -Fxq raw <<<"$flags" \
            || grep -Fxq module <<<"$flags" \
            || grep -Fxq noStrict <<<"$flags"; then
            printf '%s\tsloppy\n' "$test_path"
        elif grep -Fxq onlyStrict <<<"$flags"; then
            printf '%s\tstrict\n' "$test_path"
        else
            printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path"
        fi
    done | LC_ALL=C sort
}

verify_inventory() {
    local name=$1 inventory=$2 expected_count expected_hash actual_count actual_hash
    expected_count=$(read_value "${name}_paths")
    expected_hash=$(read_value "${name}_sha256")
    actual_count=$(printf '%s\n' "$inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$inventory" | sed '/^$/d' | sha256_stream)
    [[ "$actual_count" == "$expected_count" && "$actual_hash" == "$expected_hash" ]] || {
        printf 'error: R3bo %s path inventory drifted\n' "$name" >&2
        exit 1
    }
}

verify_key_inventory() {
    local name=$1 inventory=$2 keys expected_count expected_hash actual_count actual_hash
    keys=$(printf '%s\n' "$inventory" | variant_keys)
    expected_count=$(read_value "${name}_variants")
    expected_hash=$(read_value "${name}_keys_sha256")
    actual_count=$(printf '%s\n' "$keys" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$keys" | sed '/^$/d' | sha256_stream)
    [[ "$actual_count" == "$expected_count" && "$actual_hash" == "$expected_hash" ]] || {
        printf 'error: R3bo %s variant-key inventory drifted\n' "$name" >&2
        exit 1
    }
}

read_header() {
    local report=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' "$1"
}

report_keys() {
    report_rows "$1" | awk -F'\t' '{ print $1 "\t" $2 }' | LC_ALL=C sort
}

json_report_keys() {
    awk -v report="$1" '
        function fail(message) {
            printf "error: R3bo JSONL report %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        /^\{"kind":"metadata",/ {
            metadata++
            if (NR != 1) fail("metadata record is not first")
            next
        }
        /^\{"kind":"result",/ {
            if (summary) fail("result appears after summary")
            if (!match($0, /"path":"[^"]*"/)) fail("result is missing path")
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!match($0, /"variant":"[^"]*"/)) fail("result is missing variant")
            variant=substr($0, RSTART + 11, RLENGTH - 12)
            key=path "\t" variant
            if (seen[key]++) fail("duplicate result key")
            print key
            results++
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
    ' "$1" | LC_ALL=C sort
}

json_report_summary() {
    tail -n 1 "$1" | awk '
        /^\{"kind":"summary","outcomes":\{.*\}\}$/ {
            sub(/^\{"kind":"summary","outcomes":\{/, "")
            sub(/\}\}$/, "")
            gsub(/":/, "=")
            gsub(/"/, "")
            gsub(/,/, " ")
            print
            found=1
        }
        END { if (!found) exit 1 }
    '
}

expected_json_metadata() {
    local profile_hash=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"%s","mode":"%s"}\n' \
        "$(read_value quickjs)" \
        "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" \
        "$profile_hash" \
        "$(read_value schema)" \
        "$(read_value mode)"
}

execution_runnable() {
    printf '%s\n' "$1" | awk '
        /^execution: runnable=/ {
            sub(/^execution: runnable=/, "")
            sub(/ .*/, "")
            print
            found=1
        }
        END { if (!found) exit 1 }
    '
}

report_count() {
    local report=$1 expression=$2
    report_rows "$report" | awk -F'\t' -v expression="$expression" '
        expression == "pass" && $7 == "pass" { count++ }
        expression == "unsupported" && $7 ~ /^unsupported-/ { count++ }
        expression == "skipped" && $7 ~ /^skipped-/ { count++ }
        expression == "failure" &&
            $7 != "pass" && $7 !~ /^unsupported-/ && $7 !~ /^skipped-/ { count++ }
        END { print count + 0 }
    '
}

report_nonpass_sha256() {
    report_rows "$1" | awk -F'\t' '$7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' | sha256_stream
}

report_summary() {
    tail -n 1 "$1" | sed 's/^# summary //'
}

verify_report_shape() {
    local report=$1 json_report=$2 profile_hash=$3 expected_variants=$4
    local expected_keys tsv_keys json_keys
    expected_keys=$(manifest_paths "$activation_manifest" | variant_keys)
    tsv_keys=$(report_keys "$report")
    if ! json_keys=$(json_report_keys "$json_report"); then
        printf 'error: R3bo JSONL validation failed: %s\n' "$json_report" >&2
        exit 1
    fi
    [[ "$(read_header "$report" quickjs)" == "$(read_value quickjs)" \
        && "$(read_header "$report" test262)" == "$(read_value test262)" \
        && "$(read_header "$report" test262_patch_sha256)" \
            == "$(read_value test262_patch_sha256)" \
        && "$(read_header "$report" test262_config_sha256)" \
            == "$(read_value test262_config_sha256)" \
        && "$(read_header "$report" test262_metadata_sha256)" \
            == "$(read_value test262_metadata_sha256)" \
        && "$(read_header "$report" oxide_profile_sha256)" == "$profile_hash" \
        && "$(read_header "$report" profile)" == "$(read_value schema)" \
        && "$(read_header "$report" mode)" == "$(read_value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" \
            == "$expected_variants" ]] || {
        printf 'error: R3bo report metadata drifted: %s\n' "$report" >&2
        exit 1
    }
    diff -u <(printf '%s\n' "$expected_keys") <(printf '%s\n' "$tsv_keys")
    diff -u <(printf '%s\n' "$expected_keys") <(printf '%s\n' "$json_keys")
    [[ "$(head -n 1 "$json_report")" == "$(expected_json_metadata "$profile_hash")" \
        && "$(json_report_summary "$json_report")" == "$(report_summary "$report")" ]] || {
        printf 'error: R3bo JSONL metadata or summary drifted: %s\n' "$json_report" >&2
        exit 1
    }
}

update_baseline() {
    local updates_tmp baseline_tmp entry
    updates_tmp=$(mktemp "$baseline.updates.XXXXXX")
    baseline_tmp=$(mktemp "$baseline.XXXXXX")
    for entry in "$@"; do
        printf '%s\n' "$entry"
    done >"$updates_tmp"
    awk -F= '
        NR == FNR {
            key=$1
            sub(/^[^=]*=/, "")
            replacement[key]=$0
            next
        }
        $1 in replacement {
            print $1 "=" replacement[$1]
            seen[$1]=1
            next
        }
        { print }
        END {
            for (key in replacement) if (!(key in seen)) {
                print "missing baseline key: " key >"/dev/stderr"
                bad=1
            }
            if (bad) exit 1
        }
    ' "$updates_tmp" "$baseline" >"$baseline_tmp"
    chmod 644 "$baseline_tmp"
    mv -- "$baseline_tmp" "$baseline"
    unlink "$updates_tmp"
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value parent_profile tests/test262-global-this-parent.conf
expect_value parent_profile_sha256 8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910
expect_value candidate_profile tests/test262-global-this-candidate.conf
expect_value candidate_profile_sha256 caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15
expect_value universe_paths 148
expect_value universe_variants 165
expect_value activation_paths 135
expect_value activation_variants 150
expect_value deferred_paths 13
expect_value deferred_variants 15
expect_value quickjs_passes 150

for required in \
    "$parent_profile" "$candidate_profile" "$universe_manifest" \
    "$activation_manifest" "$deferred_manifest"
do
    [[ -f "$required" ]] || {
        printf 'error: missing R3bo asset: %s\n' "$required" >&2
        exit 1
    }
done

[[ "$(sha256_file "$parent_profile")" == "$(read_value parent_profile_sha256)" \
    && "$(sha256_file "$candidate_profile")" == "$(read_value candidate_profile_sha256)" ]] || {
    echo "error: R3bo frozen profile bytes drifted" >&2
    exit 1
}

parent_features=$(profile_section "$parent_profile" features)
candidate_features=$(profile_section "$candidate_profile" features)
parent_negatives=$(profile_section "$parent_profile" audited-negative-tests)
candidate_negatives=$(profile_section "$candidate_profile" audited-negative-tests)
parent_execution=$(profile_section "$parent_profile" execution)
candidate_execution=$(profile_section "$candidate_profile" execution)
diff -u <(printf '%s\n' globalThis) \
    <(comm -13 <(printf '%s\n' "$parent_features") <(printf '%s\n' "$candidate_features"))
[[ -z "$(comm -23 \
    <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))" ]] || {
    echo "error: R3bo candidate lost a parent feature" >&2
    exit 1
}
diff -u <(printf '%s\n' "$parent_negatives") <(printf '%s\n' "$candidate_negatives")
diff -u <(printf '%s\n' "$parent_execution") <(printf '%s\n' "$candidate_execution")
for tuple in \
    "parent_features:$parent_features" \
    "candidate_features:$candidate_features" \
    "profile_negative:$candidate_negatives" \
    "profile_execution:$candidate_execution"
do
    name=${tuple%%:*}
    inventory=${tuple#*:}
    case "$name" in
        parent_features | candidate_features) expected_count=$(read_value "$name") ;;
        profile_negative) expected_count=$(read_value profile_negative_paths) ;;
        profile_execution) expected_count=$(read_value profile_execution_entries) ;;
    esac
    expected_hash=$(read_value "${name}_sha256")
    [[ "$(printf '%s\n' "$inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')" \
            == "$expected_count" \
        && "$(printf '%s\n' "$inventory" | sed '/^$/d' | sha256_stream)" \
            == "$expected_hash" ]] || {
        printf 'error: R3bo %s profile section drifted\n' "$name" >&2
        exit 1
    }
done

tagged_inventory=$(
    git -C "$suite" grep -l -F 'globalThis' -- 'test/**/*.js' \
        | while IFS= read -r test_path; do
            if metadata_list "$test_path" features | grep -Fxq globalThis; then
                printf '%s\n' "$test_path"
            fi
        done \
        | LC_ALL=C sort
)

skipped_features=$(
    awk '
        $0 == "[features]" { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ && /=skip$/ {
            sub(/=skip$/, "")
            print
        }
    ' "$source_dir/test262.conf" | LC_ALL=C sort
)
module_inventory=
config_inventory=
while IFS= read -r test_path; do
    flags=$(metadata_list "$test_path" flags)
    if grep -Fxq module <<<"$flags"; then
        module_inventory+=$'\n'"$test_path"
        continue
    fi
    features=$(metadata_list "$test_path" features)
    if grep -Fxf <(printf '%s\n' "$skipped_features") <<<"$features" >/dev/null; then
        config_inventory+=$'\n'"$test_path"
    fi
done <<<"$tagged_inventory"
module_inventory=$(printf '%s\n' "$module_inventory" | sed '/^$/d' | LC_ALL=C sort)
config_inventory=$(printf '%s\n' "$config_inventory" | sed '/^$/d' | LC_ALL=C sort)
deferred_inventory=$(
    printf '%s\n%s\n' "$module_inventory" "$config_inventory" \
        | sed '/^$/d' \
        | LC_ALL=C sort -u
)
activation_inventory=$(
    comm -23 <(printf '%s\n' "$tagged_inventory") <(printf '%s\n' "$deferred_inventory")
)

for name in universe activation deferred module config; do
    case "$name" in
        universe) inventory=$tagged_inventory ;;
        activation) inventory=$activation_inventory ;;
        deferred) inventory=$deferred_inventory ;;
        module) inventory=$module_inventory ;;
        config) inventory=$config_inventory ;;
    esac
    verify_inventory "$name" "$inventory"
    verify_key_inventory "$name" "$inventory"
done
[[ -z "$(comm -12 \
    <(printf '%s\n' "$module_inventory") \
    <(printf '%s\n' "$config_inventory"))" ]] || {
    echo "error: R3bo module and config deferrals overlap" >&2
    exit 1
}
diff -u <(printf '%s\n' "$tagged_inventory") \
    <(printf '%s\n%s\n' "$activation_inventory" "$deferred_inventory" \
        | sed '/^$/d' | LC_ALL=C sort -u)
diff -u <(printf '%s\n' "$tagged_inventory") <(manifest_paths "$universe_manifest")
diff -u <(printf '%s\n' "$activation_inventory") <(manifest_paths "$activation_manifest")
diff -u <(printf '%s\n' "$deferred_inventory") <(manifest_paths "$deferred_manifest")
for tuple in \
    "universe:$universe_manifest" \
    "activation:$activation_manifest" \
    "deferred:$deferred_manifest"
do
    name=${tuple%%:*}
    ledger=${tuple#*:}
    manifest_paths "$ledger" | LC_ALL=C sort -c
    [[ "$(sha256_file "$ledger")" == "$(read_value "${name}_manifest_sha256")" ]] || {
        printf 'error: R3bo %s manifest file drifted\n' "$name" >&2
        exit 1
    }
done

plain_inventory=
no_strict_inventory=
async_inventory=
flag_rows=
flag_names=
negative_inventory=
host_requirement_inventory=
feature_inventory=
include_inventory=
while IFS= read -r test_path; do
    flags=$(metadata_list "$test_path" flags | LC_ALL=C sort)
    canonical_flags=$(printf '%s\n' "$flags" | sed '/^$/d' | paste -sd, -)
    flag_rows+=$'\n'"$test_path"$'\t'"$canonical_flags"
    flag_names+=$'\n'"$flags"
    case "$canonical_flags" in
        "")
            plain_inventory+=$'\n'"$test_path"
            ;;
        generated,noStrict)
            no_strict_inventory+=$'\n'"$test_path"
            ;;
        async,generated,noStrict)
            no_strict_inventory+=$'\n'"$test_path"
            async_inventory+=$'\n'"$test_path"
            ;;
        module | async,module | module,raw) ;;
        *)
            printf 'error: R3bo flags drifted for %s: %s\n' \
                "$test_path" "$canonical_flags" >&2
            exit 1
            ;;
    esac
    features=$(metadata_list "$test_path" features)
    includes=$(metadata_list "$test_path" includes)
    feature_inventory+=$'\n'"$features"
    include_inventory+=$'\n'"$includes"
    if metadata_block "$test_path" | grep -Eq '^negative:'; then
        negative_inventory+=$'\n'"$test_path"
    fi
done <<<"$tagged_inventory"
plain_inventory=$(printf '%s\n' "$plain_inventory" | sed '/^$/d' | LC_ALL=C sort)
no_strict_inventory=$(printf '%s\n' "$no_strict_inventory" | sed '/^$/d' | LC_ALL=C sort)
async_inventory=$(printf '%s\n' "$async_inventory" | sed '/^$/d' | LC_ALL=C sort)
flag_rows=$(printf '%s\n' "$flag_rows" | sed '/^$/d' | LC_ALL=C sort)
flag_names=$(printf '%s\n' "$flag_names" | sed '/^$/d' | LC_ALL=C sort -u)
negative_inventory=$(printf '%s\n' "$negative_inventory" | sed '/^$/d' | LC_ALL=C sort)
feature_inventory=$(printf '%s\n' "$feature_inventory" | sed '/^$/d' | LC_ALL=C sort -u)
include_inventory=$(printf '%s\n' "$include_inventory" | sed '/^$/d' | LC_ALL=C sort -u)

verify_inventory both "$plain_inventory"
verify_key_inventory both "$plain_inventory"
verify_inventory no_strict "$no_strict_inventory"
verify_key_inventory no_strict "$no_strict_inventory"
[[ "$(printf '%s\n' "$flag_rows" | sha256_stream)" == "$(read_value flags_sha256)" \
    && "$(printf '%s\n' "$flag_names" | wc -l | tr -d '[:space:]')" \
        == "$(read_value metadata_flags)" \
    && "$(printf '%s\n' "$flag_names" | sha256_stream)" \
        == "$(read_value metadata_flags_sha256)" ]] || {
    echo "error: R3bo metadata flag inventory drifted" >&2
    exit 1
}
for entry in \
    "plain_flags_paths:$(printf '%s\n' "$flag_rows" | awk -F'\t' '$2 == "" { n++ } END { print n + 0 }')" \
    "generated_no_strict_paths:$(printf '%s\n' "$flag_rows" | awk -F'\t' '$2 == "generated,noStrict" { n++ } END { print n + 0 }')" \
    "async_generated_no_strict_paths:$(printf '%s\n' "$flag_rows" | awk -F'\t' '$2 == "async,generated,noStrict" { n++ } END { print n + 0 }')" \
    "module_plain_paths:$(printf '%s\n' "$flag_rows" | awk -F'\t' '$2 == "module" { n++ } END { print n + 0 }')" \
    "module_async_paths:$(printf '%s\n' "$flag_rows" | awk -F'\t' '$2 == "async,module" { n++ } END { print n + 0 }')" \
    "module_raw_paths:$(printf '%s\n' "$flag_rows" | awk -F'\t' '$2 == "module,raw" { n++ } END { print n + 0 }')"
do
    name=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$name")" ]] || {
        printf 'error: R3bo flag class drifted: %s\n' "$name" >&2
        exit 1
    }
done

[[ "$(printf '%s\n' "$feature_inventory" | wc -l | tr -d '[:space:]')" \
        == "$(read_value metadata_features)" \
    && "$(printf '%s\n' "$feature_inventory" | sha256_stream)" \
        == "$(read_value metadata_features_sha256)" \
    && "$(printf '%s\n' "$include_inventory" | wc -l | tr -d '[:space:]')" \
        == "$(read_value metadata_includes)" \
    && "$(printf '%s\n' "$include_inventory" | sha256_stream)" \
        == "$(read_value metadata_includes_sha256)" ]] || {
    echo "error: R3bo complete metadata inventory drifted" >&2
    exit 1
}
verify_inventory negative "$negative_inventory"
[[ -z "$(comm -23 \
    <(printf '%s\n' "$negative_inventory") \
    <(printf '%s\n' "$module_inventory"))" ]] || {
    echo "error: R3bo negative tests escaped the deferred module partition" >&2
    exit 1
}
while IFS= read -r test_path; do
    negative_metadata=$(metadata_block "$test_path")
    [[ "$(grep -c '^[[:space:]]*phase: resolution$' <<<"$negative_metadata")" == 1 \
        && "$(grep -c '^[[:space:]]*type: SyntaxError$' <<<"$negative_metadata")" == 1 ]] || {
        printf 'error: R3bo deferred negative metadata drifted: %s\n' "$test_path" >&2
        exit 1
    }
done <<<"$negative_inventory"

activation_features=
activation_includes=
activation_negatives=
while IFS= read -r test_path; do
    activation_features+=$'\n'"$(metadata_list "$test_path" features)"
    activation_includes+=$'\n'"$(metadata_list "$test_path" includes)"
    if metadata_block "$test_path" | grep -Eq '^negative:'; then
        activation_negatives+=$'\n'"$test_path"
    fi
    if grep -Eq '[$]262[.]' "$suite/$test_path"; then
        host_requirement_inventory+=$'\n'"$test_path"
        continue
    fi
    while IFS= read -r include; do
        if [[ -n "$include" ]] \
            && grep -Eq '[$]262[.]' "$suite/harness/$include"; then
            host_requirement_inventory+=$'\n'"$test_path"
            break
        fi
    done < <(metadata_list "$test_path" includes)
done <<<"$activation_inventory"
activation_features=$(printf '%s\n' "$activation_features" | sed '/^$/d' | LC_ALL=C sort -u)
activation_includes=$(printf '%s\n' "$activation_includes" | sed '/^$/d' | LC_ALL=C sort -u)
activation_negatives=$(printf '%s\n' "$activation_negatives" | sed '/^$/d' | LC_ALL=C sort -u)
host_requirement_inventory=$(
    printf '%s\n' "$host_requirement_inventory" | sed '/^$/d' | LC_ALL=C sort -u
)
[[ "$(printf '%s\n' "$activation_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_metadata_features)" \
    && "$(printf '%s\n' "$activation_features" | sha256_stream)" \
        == "$(read_value activation_metadata_features_sha256)" \
    && "$(printf '%s\n' "$activation_includes" | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_metadata_includes)" \
    && "$(printf '%s\n' "$activation_includes" | sha256_stream)" \
        == "$(read_value activation_metadata_includes_sha256)" \
    && -z "$(comm -23 \
        <(printf '%s\n' "$activation_features") \
        <(printf '%s\n' "$candidate_features"))" ]] || {
    echo "error: R3bo activation metadata exceeds the candidate profile" >&2
    exit 1
}
diff -u <(printf '%s\n' globalThis) \
    <(comm -23 \
        <(printf '%s\n' "$activation_features") \
        <(printf '%s\n' "$parent_features"))
[[ "$(printf '%s\n' "$activation_negatives" | sed '/^$/d' | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_negative_paths)" \
    && "$(printf '%s\n' "$host_requirement_inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_host_requirement_paths)" \
    && "$(comm -12 \
        <(printf '%s\n' "$activation_inventory") \
        <(printf '%s\n' "$async_inventory") | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_async_paths)" \
    && "$(comm -12 \
        <(printf '%s\n' "$activation_inventory") \
        <(printf '%s\n' "$no_strict_inventory") | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_no_strict_paths)" \
    && "$(comm -12 \
        <(printf '%s\n' "$activation_inventory") \
        <(printf '%s\n' "$plain_inventory") | wc -l | tr -d '[:space:]')" \
        == "$(read_value activation_two_mode_paths)" ]] || {
    echo "error: R3bo activation composition drifted" >&2
    exit 1
}

error_file="$source_dir/test262_errors.txt"
[[ "$(sha256_file "$error_file")" == "$(read_value quickjs_error_file_sha256)" ]] || {
    echo "error: pinned QuickJS expected-error ledger drifted" >&2
    exit 1
}
expected_error_paths=$(
    awk -F: '{ path=$1; sub(/^test262\//, "", path); print path }' "$error_file" \
        | LC_ALL=C sort -u
)
expected_error_overlap=$(
    comm -12 <(printf '%s\n' "$tagged_inventory") <(printf '%s\n' "$expected_error_paths")
)
[[ "$(printf '%s\n' "$expected_error_overlap" | sed '/^$/d' | wc -l | tr -d '[:space:]')" \
        == "$(read_value quickjs_expected_error_paths)" ]] || {
    echo "error: globalThis universe overlaps the QuickJS expected-error ledger" >&2
    exit 1
}

runner="$source_dir/run-test262"
[[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done < <(manifest_paths "$activation_manifest")
if ! quickjs_output=$(cd -- "$source_dir" \
    && ./run-test262 -m -T 1 -c test262.conf -a -f "${quickjs_files[@]}" 2>&1); then
    printf '%s\n' "$quickjs_output" >&2
    echo "error: pinned QuickJS could not execute the R3bo activation" >&2
    exit 1
fi
printf '%s\n' "$quickjs_output" >"$quickjs_log"
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$quickjs_output" \
    || ! grep -Fq \
        "Average memory statistics for $(read_value quickjs_passes) tests:" \
        <<<"$quickjs_output"; then
    printf '%s\n' "$quickjs_output" >&2
    echo "error: pinned QuickJS no longer passes all 150 R3bo variants" >&2
    exit 1
fi

if "$check_only"; then
    printf 'globalThis inputs verified: %s paths/%s variants = %s/%s activation + %s/%s module/config deferred; QuickJS passes all %s variants\n' \
        "$(read_value universe_paths)" "$(read_value universe_variants)" \
        "$(read_value activation_paths)" "$(read_value activation_variants)" \
        "$(read_value deferred_paths)" "$(read_value deferred_variants)" \
        "$(read_value quickjs_passes)"
    exit 0
fi

receipt_keys=(
    transition_data_sha256 transition_receipt_sha256 \
    before_runnable before_passes before_failures before_unsupported before_skipped \
    before_nonpass_sha256 before_tsv_sha256 before_jsonl_sha256 before_summary \
    candidate_runnable candidate_passes candidate_failures candidate_unsupported \
    candidate_skipped candidate_nonpass_sha256 candidate_tsv_sha256 \
    candidate_jsonl_sha256 candidate_summary
)
pending=()
for key in "${receipt_keys[@]}"; do
    [[ "$(read_value "$key")" == PENDING ]] && pending+=("$key")
done
if [[ ${#pending[@]} -ne 0 && ${#pending[@]} -ne ${#receipt_keys[@]} ]]; then
    printf 'error: R3bo baseline is partially pending: %s\n' "${pending[*]}" >&2
    exit 1
fi
if [[ ${#pending[@]} -eq 0 ]]; then
    bless=false
elif ! "$bless"; then
    printf 'error: R3bo Oxide baseline requires --bless: %s\n' "${pending[*]}" >&2
    exit 1
fi

run_oxide() {
    local profile=$1 report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --manifest "$activation_manifest" \
        --report "$report" \
        --mode "$(read_value mode)" \
        --workers "$workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

for stale_report in \
    "$before_report" "$before_json_report" "$candidate_report" "$candidate_json_report"
do
    [[ ! -e "$stale_report" ]] || unlink "$stale_report"
done
before_output=$(run_oxide "$parent_profile" "$before_report")
candidate_output=$(run_oxide "$candidate_profile" "$candidate_report")
printf '%s\n%s\n' "$before_output" "$candidate_output"
verify_report_shape \
    "$before_report" "$before_json_report" \
    "$(read_value parent_profile_sha256)" "$(read_value activation_variants)"
verify_report_shape \
    "$candidate_report" "$candidate_json_report" \
    "$(read_value candidate_profile_sha256)" "$(read_value activation_variants)"

before_runnable=$(execution_runnable "$before_output")
candidate_runnable=$(execution_runnable "$candidate_output")
before_passes=$(report_count "$before_report" pass)
before_failures=$(report_count "$before_report" failure)
before_unsupported=$(report_count "$before_report" unsupported)
before_skipped=$(report_count "$before_report" skipped)
candidate_passes=$(report_count "$candidate_report" pass)
candidate_failures=$(report_count "$candidate_report" failure)
candidate_unsupported=$(report_count "$candidate_report" unsupported)
candidate_skipped=$(report_count "$candidate_report" skipped)

[[ "$before_runnable" == 0 \
    && "$before_passes" == 0 \
    && "$before_failures" == 0 \
    && "$before_unsupported" == "$(read_value activation_variants)" \
    && "$before_skipped" == 0 \
    && "$candidate_runnable" == "$(read_value activation_variants)" \
    && "$candidate_passes" == "$(read_value activation_variants)" \
    && "$candidate_unsupported" == 0 \
    && "$candidate_skipped" == 0 ]] || {
    echo "error: R3bo before/candidate outcome counts are not the exact 0-to-150 activation" >&2
    exit 1
}
if ! report_rows "$before_report" | awk -F'\t' '
    $5 != "normal" || $6 != "" ||
    $7 != "unsupported-feature" || $8 != "selection" ||
    $9 != "EngineCapability" ||
    $10 != "quickjs-oxide does not declare Test262 feature support: globalThis" {
        exit 1
    }
'; then
    echo "error: R3bo parent is not the exact globalThis-only rejection vector" >&2
    exit 1
fi
if ! report_rows "$candidate_report" | awk -F'\t' '
    $5 != "normal" || $6 != "" || $7 != "pass" ||
    $8 != "normal" || $9 != "" || $10 != "" { exit 1 }
'; then
    echo "error: R3bo candidate is not the exact all-pass vector" >&2
    exit 1
fi
if ! paste <(report_rows "$before_report") <(report_rows "$candidate_report") \
    | awk -F'\t' '
        $1 != $11 || $2 != $12 || $3 != $13 ||
        $4 != $14 || $5 != $15 || $6 != $16 { exit 1 }
    '; then
    echo "error: R3bo before/candidate metadata join drifted" >&2
    exit 1
fi

transition_tmp=$(mktemp "$transition_receipt.XXXXXX")
cleanup_transition_tmp() {
    if [[ -n ${transition_tmp:-} && -e "$transition_tmp" ]]; then
        unlink "$transition_tmp"
    fi
}
trap cleanup_transition_tmp EXIT
{
    printf '# R3bo exhaustive focused globalThis admission transition.\n'
    printf '# before_oxide_profile_sha256=%s\n' "$(read_value parent_profile_sha256)"
    printf '# after_oxide_profile_sha256=%s\n' "$(read_value candidate_profile_sha256)"
    printf '# manifest_sha256=%s\n' "$(read_value activation_manifest_sha256)"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    paste <(report_rows "$before_report") <(report_rows "$candidate_report") \
        | awk -F'\t' 'BEGIN { OFS="\t" } {
            print $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$17,$18,$19,$20
        }'
} >"$transition_tmp"
transition_rows=$(awk -F'\t' '!/^#/ && $1 != "path" { n++ } END { print n + 0 }' \
    "$transition_tmp")
transition_keys_sha256=$(
    awk -F'\t' '!/^#/ && $1 != "path" { print $1 "\t" $2 }' "$transition_tmp" \
        | sha256_stream
)
transition_data_sha256=$(
    awk -F'\t' '!/^#/ && $1 != "path" { print }' "$transition_tmp" | sha256_stream
)
transition_receipt_sha256=$(sha256_file "$transition_tmp")
before_nonpass_sha256=$(report_nonpass_sha256 "$before_report")
candidate_nonpass_sha256=$(report_nonpass_sha256 "$candidate_report")
before_tsv_sha256=$(sha256_file "$before_report")
before_jsonl_sha256=$(sha256_file "$before_json_report")
candidate_tsv_sha256=$(sha256_file "$candidate_report")
candidate_jsonl_sha256=$(sha256_file "$candidate_json_report")
before_summary=$(report_summary "$before_report")
candidate_summary=$(report_summary "$candidate_report")
[[ "$transition_rows" == "$(read_value transition_rows)" \
    && "$transition_keys_sha256" == "$(read_value transition_keys_sha256)" ]] || {
    echo "error: R3bo transition key shape drifted" >&2
    exit 1
}

if "$bless"; then
    chmod 644 "$transition_tmp"
    mv -- "$transition_tmp" "$transition_receipt"
    transition_tmp=
    update_baseline \
        "transition_data_sha256=$transition_data_sha256" \
        "transition_receipt_sha256=$transition_receipt_sha256" \
        "before_runnable=$before_runnable" \
        "before_passes=$before_passes" \
        "before_failures=$before_failures" \
        "before_unsupported=$before_unsupported" \
        "before_skipped=$before_skipped" \
        "before_nonpass_sha256=$before_nonpass_sha256" \
        "before_tsv_sha256=$before_tsv_sha256" \
        "before_jsonl_sha256=$before_jsonl_sha256" \
        "before_summary=$before_summary" \
        "candidate_runnable=$candidate_runnable" \
        "candidate_passes=$candidate_passes" \
        "candidate_failures=$candidate_failures" \
        "candidate_unsupported=$candidate_unsupported" \
        "candidate_skipped=$candidate_skipped" \
        "candidate_nonpass_sha256=$candidate_nonpass_sha256" \
        "candidate_tsv_sha256=$candidate_tsv_sha256" \
        "candidate_jsonl_sha256=$candidate_jsonl_sha256" \
        "candidate_summary=$candidate_summary"
    printf 'globalThis baseline blessed: parent %s unsupported -> candidate %s/%s pass; QuickJS %s/%s\n' \
        "$before_unsupported" "$candidate_passes" "$(read_value activation_variants)" \
        "$(read_value quickjs_passes)" "$(read_value activation_variants)"
    exit 0
fi

[[ -f "$transition_receipt" ]] || {
    echo "error: missing R3bo transition receipt" >&2
    exit 1
}
cmp -s "$transition_tmp" "$transition_receipt" || {
    diff -u "$transition_receipt" "$transition_tmp" >&2 || true
    echo "error: R3bo transition receipt drifted" >&2
    exit 1
}
unlink "$transition_tmp"
transition_tmp=
for entry in \
    "transition_rows:$transition_rows" \
    "transition_keys_sha256:$transition_keys_sha256" \
    "transition_data_sha256:$transition_data_sha256" \
    "transition_receipt_sha256:$transition_receipt_sha256" \
    "before_runnable:$before_runnable" \
    "before_passes:$before_passes" \
    "before_failures:$before_failures" \
    "before_unsupported:$before_unsupported" \
    "before_skipped:$before_skipped" \
    "before_nonpass_sha256:$before_nonpass_sha256" \
    "before_tsv_sha256:$before_tsv_sha256" \
    "before_jsonl_sha256:$before_jsonl_sha256" \
    "before_summary:$before_summary" \
    "candidate_runnable:$candidate_runnable" \
    "candidate_passes:$candidate_passes" \
    "candidate_failures:$candidate_failures" \
    "candidate_unsupported:$candidate_unsupported" \
    "candidate_skipped:$candidate_skipped" \
    "candidate_nonpass_sha256:$candidate_nonpass_sha256" \
    "candidate_tsv_sha256:$candidate_tsv_sha256" \
    "candidate_jsonl_sha256:$candidate_jsonl_sha256" \
    "candidate_summary:$candidate_summary"
do
    name=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$name")" ]] || {
        printf 'error: R3bo receipt drifted: %s\n' "$name" >&2
        exit 1
    }
done
printf 'globalThis gate verified: parent %s unsupported -> candidate %s/%s pass; QuickJS %s/%s\n' \
    "$before_unsupported" "$candidate_passes" "$(read_value activation_variants)" \
    "$(read_value quickjs_passes)" "$(read_value activation_variants)"
