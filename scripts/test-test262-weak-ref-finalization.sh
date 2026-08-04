#!/usr/bin/env bash
# Checksum-bound WeakRef and FinalizationRegistry Test262 admission gate.
#
# The candidate profile is generated in a temporary directory from the authenticated
# frozen global-admission parent and adds exactly WeakRef and FinalizationRegistry. The focused
# universe deliberately keeps its independent for-of and createRealm blockers
# visible instead of admitting either capability globally.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

base_profile=tests/test262-weak-ref-finalization-global-parent.conf
candidate_features=tests/test262-weak-ref-finalization-candidate-features.txt
universe=tests/test262-weak-ref-finalization-universe.txt
activation=tests/test262-weak-ref-finalization-activation.txt
for_of_blocker=tests/test262-weak-ref-finalization-for-of-blocker.txt
create_realm_blockers=tests/test262-weak-ref-finalization-create-realm-blockers.txt
report=target/test262-weak-ref-finalization.tsv
oracle_log=target/test262-weak-ref-finalization-quickjs.log
workers=${TEST262_WORKERS:-8}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
base_profile_sha=3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef
candidate_profile_sha=8be6c2a3892a62d89ed17df3f3d3b54e9e84fda8ef6be2bcdaa7d49044593990
candidate_features_sha=0a462001d5a51db3b103ccdadfad17076941c5a5f7f163d767bedec5fc471406
base_features_sha=a892ce31bef675386670419a9410e6086c24f1edd9f8e14f6c793d8bfb07503b
candidate_profile_features_sha=82f8c1c3f217e45d3e02b60776bad5ec8268b8270a608990906802c38c8ce139
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
report_sha=5ff2b92a694f71b63ab5b883e6c9416e2810c7230e26d36fcaec5f5815b20fe6

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs, scoped profile, and pinned QuickJS only\n'
}

check_only=false
case ${1-} in
    '') ;;
    --check) check_only=true ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid TEST262_WORKERS' >&2; exit 2; }

die() {
    echo "error: $*" >&2
    exit 1
}

sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

lines() {
    wc -l <"$1" | tr -d '[:space:]'
}

section() {
    local file=$1 wanted=$2
    awk -v wanted="[$wanted]" '
        $0 == wanted { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$file"
}

header() {
    local file=$1 wanted=$2
    awk -F= -v wanted="# $wanted" '
        $1 == wanted { sub(/^[^=]*=/, ""); print; found++ }
        END { if (found != 1) exit 1 }
    ' "$file"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant")' "$1"
}

check_file() {
    local file=$1 expected_lines=$2 expected_sha=$3
    [[ -f "$file" ]] || die "missing gate input: $file"
    [[ "$(lines "$file")" == "$expected_lines" \
        && "$(sha "$file")" == "$expected_sha" ]] \
        || die "authenticated input drifted: $file"
}

variant_keys() {
    local paths=$1
    awk -F'\t' '
        function has(list, value) {
            return index("," list ",", "," value ",") != 0
        }
        NR == FNR { wanted[$0]=1; next }
        $1 in wanted {
            if (has($3, "module") || has($3, "noStrict") || has($3, "raw")) {
                print $1 "\tsloppy"
            } else if (has($3, "onlyStrict")) {
                print $1 "\tstrict"
            } else {
                print $1 "\tsloppy"
                print $1 "\tstrict"
            }
        }
    ' "$paths" "$metadata_tsv" | sort
}

check_keys() {
    local paths=$1 expected_lines=$2 expected_sha=$3 output=$4
    variant_keys "$paths" >"$output"
    [[ "$(lines "$output")" == "$expected_lines" \
        && "$(sha "$output")" == "$expected_sha" ]] \
        || die "variant-key inventory drifted: $paths"
}

cd -- "$root"

check_file "$base_profile" 1269 "$base_profile_sha"
check_file "$candidate_features" 2 "$candidate_features_sha"
check_file "$universe" 82 "$universe_sha"
check_file "$activation" 79 "$activation_sha"
check_file "$for_of_blocker" 1 "$for_of_sha"
check_file "$create_realm_blockers" 2 "$create_realm_sha"
for sorted_input in "$candidate_features" "$universe" "$activation" \
        "$for_of_blocker" "$create_realm_blockers"; do
    sort -c "$sorted_input"
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-weak-ref-finalization.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
candidate_profile=$tmp/candidate.conf

# Preserve every byte of the parent profile except for two sorted feature rows.
awk '
    $0 == "Map" { print "FinalizationRegistry" }
    $0 == "WeakSet" { print "WeakRef" }
    { print }
' "$base_profile" >"$candidate_profile"
[[ "$(lines "$candidate_profile")" == 1271 \
    && "$(sha "$candidate_profile")" == "$candidate_profile_sha" ]] \
    || die 'scoped candidate profile drifted'

base_features=$tmp/base.features
candidate_profile_features=$tmp/candidate.features
section "$base_profile" features | sort >"$base_features"
section "$candidate_profile" features | sort >"$candidate_profile_features"
[[ "$(lines "$base_features")" == 99 \
    && "$(sha "$base_features")" == "$base_features_sha" \
    && "$(lines "$candidate_profile_features")" == 101 \
    && "$(sha "$candidate_profile_features")" \
        == "$candidate_profile_features_sha" ]] \
    || die 'candidate profile feature inventory drifted'
diff -u "$candidate_features" \
    <(comm -13 "$base_features" "$candidate_profile_features")
[[ -z "$(comm -23 "$base_features" "$candidate_profile_features")" ]] \
    || die 'candidate removed a parent feature'
for name in audited-negative-tests execution; do
    section "$base_profile" "$name" >"$tmp/base.$name"
    section "$candidate_profile" "$name" >"$tmp/candidate.$name"
    diff -u "$tmp/base.$name" "$tmp/candidate.$name"
done
[[ "$(lines "$tmp/base.audited-negative-tests")" == 1157 \
    && "$(sha "$tmp/base.audited-negative-tests")" \
        == "$audited_negative_tests_sha" \
    && "$(lines "$tmp/base.execution")" == 1 \
    && "$(sha "$tmp/base.execution")" == "$execution_sha" ]] \
    || die 'non-feature profile sections drifted'

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
[[ "$(basename -- "$source_dir")" == "quickjs-$quickjs" \
    && "$(git -C "$suite" rev-parse 'HEAD^{commit}')" == "$test262" \
    && "$(sha "$source_dir/tests/test262.patch")" == "$patch_sha" \
    && "$(sha "$source_dir/test262.conf")" == "$config_sha" ]] \
    || die 'prepared QuickJS/Test262 inputs drifted'

metadata_bin=$tmp/metadata.bin
metadata_tsv=$tmp/metadata.tsv
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(sha "$metadata_bin")" == "$metadata_sha" ]] \
    || die 'pinned Test262 metadata drifted'
tr '\0' '\t' <"$metadata_bin" >"$metadata_tsv"
[[ "$(lines "$metadata_tsv")" == 53125 ]] \
    || die 'pinned Test262 metadata record count drifted'

derived_universe=$tmp/universe.paths
awk -F'\t' '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    has($4, "WeakRef") || has($4, "FinalizationRegistry") { print $1 }
' "$metadata_tsv" | sort -u >"$derived_universe"
[[ "$(lines "$derived_universe")" == 82 \
    && "$(sha "$derived_universe")" == "$universe_sha" ]] \
    || die 'WeakRef/FinalizationRegistry metadata universe drifted'
diff -u "$universe" "$derived_universe"

# All paths in this cohort are ordinary sloppy+strict tests. Two carry the
# generated flag, but none is negative, module, async, or GC-host dependent.
awk -F'\t' '
    NR == FNR { wanted[$0]=1; next }
    $1 in wanted {
        if ($5 != "" || $6 != "" ||
            ($3 != "" && $3 != "generated")) print $1
    }
' "$derived_universe" "$metadata_tsv" >"$tmp/invalid-metadata"
[[ ! -s "$tmp/invalid-metadata" ]] \
    || die 'weak-reference universe gained negative or unsupported flags'
generated_count=$(awk -F'\t' '
    NR == FNR { wanted[$0]=1; next }
    $1 in wanted && $3 == "generated" { count++ }
    END { print count + 0 }
' "$derived_universe" "$metadata_tsv")
[[ "$generated_count" == 2 ]] \
    || die 'weak-reference generated-path count drifted'

derived_for_of=$tmp/for-of.paths
awk -F'\t' '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    NR == FNR { wanted[$0]=1; next }
    $1 in wanted && has($4, "for-of") { print $1 }
' "$derived_universe" "$metadata_tsv" | sort -u >"$derived_for_of"
diff -u "$for_of_blocker" "$derived_for_of"

derived_create_realm=$tmp/create-realm.paths
: >"$derived_create_realm"
while IFS= read -r test_file; do
    test_source=$suite/$test_file
    [[ -f "$test_source" ]] || die "pinned Test262 path is missing: $test_file"
    if grep -Fq '$262.createRealm' "$test_source"; then
        printf '%s\n' "$test_file" >>"$derived_create_realm"
    fi
    if awk '
        {
            gsub(/\$262\.createRealm/, "")
            if ($0 ~ /\$262\./ || $0 ~ /\$DONE/) found=1
        }
        END { exit !found }
    ' "$test_source"; then
        die "unexpected non-createRealm or async host dependency: $test_file"
    fi
done <"$derived_universe"
sort -u -o "$derived_create_realm" "$derived_create_realm"
diff -u "$create_realm_blockers" "$derived_create_realm"

{ cat "$derived_for_of"; cat "$derived_create_realm"; } \
    | sort -u >"$tmp/blockers.paths"
comm -23 "$derived_universe" "$tmp/blockers.paths" >"$tmp/activation.paths"
diff -u "$activation" "$tmp/activation.paths"
[[ -z "$(comm -12 "$derived_for_of" "$derived_create_realm")" ]] \
    || die 'for-of and createRealm blocker sets overlap'

# The two createRealm paths also have the cross-realm feature tag. Host
# classification has precedence, so the candidate intentionally leaves both
# cross-realm and for-of undeclared.
awk -F'\t' '
    NR == FNR { supported[$0]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    has($4, "WeakRef") || has($4, "FinalizationRegistry") {
        missing=0
        count=split($4, features, ",")
        for (i=1; i<=count; i++) {
            if (features[i] != "" && !(features[i] in supported)) {
                missing=1
            }
        }
        if (missing) print $1
    }
' "$candidate_profile_features" "$metadata_tsv" \
    | sort -u >"$tmp/missing-feature.paths"
diff -u "$tmp/blockers.paths" "$tmp/missing-feature.paths"

check_keys "$derived_universe" 164 "$universe_keys_sha" "$tmp/universe.keys"
check_keys "$activation" 158 "$activation_keys_sha" "$tmp/activation.keys"
check_keys "$for_of_blocker" 2 "$for_of_keys_sha" "$tmp/for-of.keys"
check_keys "$create_realm_blockers" 4 "$create_realm_keys_sha" \
    "$tmp/create-realm.keys"
{ cat "$tmp/activation.keys"; cat "$tmp/for-of.keys"; \
    cat "$tmp/create-realm.keys"; } | sort >"$tmp/partition.keys"
diff -u "$tmp/universe.keys" "$tmp/partition.keys"

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] \
    || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r test_file; do
    files+=("test262/$test_file")
done <"$derived_universe"
if ! (cd "$source_dir"; \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS failed the WeakRef/FinalizationRegistry universe'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
        "$oracle_log" \
    || ! grep -Fq 'Average memory statistics for 164 tests:' "$oracle_log"; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS oracle receipt drifted'
fi

if "$check_only"; then
    echo 'WeakRef/FinalizationRegistry inputs verified: QuickJS passes 164 variants; candidate is 158 + 2 for-of + 4 createRealm.'
    exit 0
fi

{
    awk '{print $0 "\tactivation"}' "$activation"
    awk '{print $0 "\tfor-of"}' "$for_of_blocker"
    awk '{print $0 "\tcreate-realm"}' "$create_realm_blockers"
} >"$tmp/classes"

# The coordinator intentionally accepts only profile paths enumerated in its
# Rust source. Keep this gate independent of that global registry: authenticate
# the scoped profile above, then invoke the coordinator's process-isolated
# worker entrypoint for exactly the 158 runnable variants. The six blockers are
# emitted as selection rows without execution.
command -v perl >/dev/null 2>&1 \
    || die 'perl is required to enforce focused worker timeouts'
awk -F'\t' -v OFS='\034' '
    NR == FNR { class[$1]=$2; next }
    $1 in class {
        print $1, "sloppy", $3, $4, class[$1]
        print $1, "strict", $3, $4, class[$1]
    }
' "$tmp/classes" "$metadata_tsv" | sort >"$tmp/planned.rows"
[[ "$(lines "$tmp/planned.rows")" == 164 ]] \
    || die 'focused worker plan cardinality drifted'

: >"$tmp/result.rows"
: >"$tmp/worker.failures"
while IFS=$'\034' read -r test_file variant flags features class; do
    case $class in
        activation)
            worker_stderr=$tmp/worker.stderr
            if worker_output=$(perl -e '
                    my $seconds=shift @ARGV;
                    alarm $seconds;
                    exec @ARGV;
                    die "exec: $!";
                ' 30 "$runner" \
                    --worker-one \
                    --suite "$suite" \
                    --test "$test_file" \
                    --variant "$variant" 2>"$worker_stderr"); then
                if [[ "$(printf '%s\n' "$worker_output" \
                        | awk -F'\t' 'NF == 4 { print "valid" }')" \
                        != valid ]]; then
                    worker_output=$'runner-error\thost\t\tworker returned a malformed result'
                fi
            else
                worker_detail=$(tr '\t\r\n' '   ' <"$worker_stderr")
                worker_output="runner-error"$'\t'"host"$'\t\t'"$worker_detail"
            fi
            if [[ "$worker_output" != $'pass\tnormal\t\t' ]]; then
                printf '%s\t%s\t%s\n' "$test_file" "$variant" "$worker_output" \
                    >>"$tmp/worker.failures"
            fi
            ;;
        for-of)
            worker_output=$'unsupported-feature\tselection\tEngineCapability\tquickjs-oxide does not declare Test262 feature support: for-of'
            ;;
        create-realm)
            worker_output=$'unsupported-host-create-realm\tselection\tHostCapability\tmissing execution capabilities: create-realm'
            ;;
        *) die "unknown focused worker class: $class" ;;
    esac
    printf '%s\t%s\t%s\t%s\tnormal\t\t%s\n' \
        "$test_file" "$variant" "$flags" "$features" "$worker_output" \
        >>"$tmp/result.rows"
done <"$tmp/planned.rows"
sort -o "$tmp/result.rows" "$tmp/result.rows"

summary=$(awk -F'\t' '
    { outcomes[$7]++ }
    END { for (outcome in outcomes) print outcome "=" outcomes[outcome] }
' "$tmp/result.rows" | sort | paste -sd ' ' -)

rm -f -- "$report"
{
    echo '# quickjs-oxide focused Test262 outcome vector v1'
    echo "# quickjs=$quickjs"
    echo "# test262=$test262"
    echo "# test262_patch_sha256=$patch_sha"
    echo "# test262_config_sha256=$config_sha"
    echo "# test262_metadata_sha256=$metadata_sha"
    echo "# oxide_profile_sha256=$candidate_profile_sha"
    echo '# profile=test262-weak-ref-finalization-focused-v1'
    echo '# mode=both'
    echo '# execution=direct-worker-one'
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\toutcome\tactual_phase\tactual_type\tdetail\n'
    cat "$tmp/result.rows"
    echo "# summary $summary"
} >"$report"

[[ -f "$report" \
    && "$(sha "$report")" == "$report_sha" \
    && "$(header "$report" quickjs)" == "$quickjs" \
    && "$(header "$report" test262)" == "$test262" \
    && "$(header "$report" test262_patch_sha256)" == "$patch_sha" \
    && "$(header "$report" test262_config_sha256)" == "$config_sha" \
    && "$(header "$report" test262_metadata_sha256)" == "$metadata_sha" \
    && "$(header "$report" oxide_profile_sha256)" \
        == "$candidate_profile_sha" \
    && "$(header "$report" profile)" \
        == test262-weak-ref-finalization-focused-v1 \
    && "$(header "$report" mode)" == both \
    && "$(header "$report" execution)" == direct-worker-one ]] \
    || die 'focused candidate report metadata drifted'

report_rows "$report" >"$tmp/report.rows"
[[ "$(lines "$tmp/report.rows")" == 164 \
    && "$(awk -F'\t' '{print $1 "\t" $2}' "$tmp/report.rows" \
        | sort | sha /dev/stdin)" == "$universe_keys_sha" ]] \
    || die 'focused candidate report key inventory drifted'

counts=$(awk -F'\t' '
    NR == FNR { class[$1]=$2; next }
    {
        wanted=class[$1]
        if (wanted == "activation") {
            activation++
        } else if (wanted == "for-of") {
            if ($7 != "unsupported-feature" || $8 != "selection" ||
                $9 != "EngineCapability" ||
                $10 != "quickjs-oxide does not declare Test262 feature support: for-of") {
                exit 3
            }
            for_of++
        } else if (wanted == "create-realm") {
            if ($7 != "unsupported-host-create-realm" || $8 != "selection" ||
                $9 != "HostCapability" ||
                $10 != "missing execution capabilities: create-realm") {
                exit 4
            }
            create_realm++
        } else {
            exit 5
        }
    }
    END {
        printf "activation=%d for-of=%d create-realm=%d", \
            activation, for_of, create_realm
    }
' "$tmp/classes" "$tmp/report.rows") \
    || die 'focused candidate outcome semantics drifted'
[[ "$counts" == 'activation=158 for-of=2 create-realm=4' ]] \
    || die "focused candidate partition drifted: $counts"

if [[ -s "$tmp/worker.failures" \
    || "$summary" \
        != 'pass=158 unsupported-feature=2 unsupported-host-create-realm=4' ]]; then
    if [[ -s "$tmp/worker.failures" ]]; then
        echo 'Focused WeakRef/FinalizationRegistry worker failures:' >&2
        cat "$tmp/worker.failures" >&2
    fi
    die "focused candidate summary drifted: $summary"
fi

echo 'WeakRef/FinalizationRegistry gate passes: QuickJS 164/164; Oxide 158/158 with 2 for-of + 4 createRealm blockers preserved.'
