#!/usr/bin/env bash
# Reproduce the focused default-parameters admission certificate.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-default-parameters-baseline.txt
live_profile=compat/test262-oxide.conf
workers=${TEST262_WORKERS:-8}
lock_dir=$root/target/test262-default-parameters-focused.lock
lock_held=0
live_policy=current

case ${1-} in
    '') ;;
    --frozen-profiles) live_policy=frozen ;;
    -h | --help)
        cat <<'EOF'
usage: scripts/test-test262-default-parameters.sh [--frozen-profiles]

Rebuild the exhaustive default-parameters tag inventory, certify its runnable
cohort in pinned QuickJS, and reproduce the exact 4,516-row parent/candidate
quickjs-oxide transition. TEST262_WORKERS controls parallelism (default: 8).

--frozen-profiles reproduces only the immutable certificate. A later global
admission gate may use it after independently authenticating its live profile.
EOF
        exit 0
        ;;
    *)
        echo 'error: this gate accepts only --frozen-profiles' >&2
        exit 2
        ;;
esac
[[ $# -le 1 ]] || {
    echo 'error: this gate accepts at most one argument' >&2
    exit 2
}
[[ "$workers" =~ ^[1-9][0-9]*$ ]] || {
    echo "error: TEST262_WORKERS must be a positive integer: $workers" >&2
    exit 2
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo 'error: sha256sum or shasum is required' >&2
        exit 2
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}

read_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$baseline"); then
        echo "error: baseline must contain exactly one $key entry" >&2
        exit 1
    fi
    [[ -n "$value" ]] || {
        echo "error: baseline contains an empty $key entry" >&2
        exit 1
    }
    printf '%s\n' "$key" >>"$consumed_keys"
    printf '%s\n' "$value"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    [[ "$actual" == "$expected" ]] || {
        printf 'error: default-parameters baseline %s drifted: %s != %s\n' \
            "$key" "$actual" "$expected" >&2
        exit 1
    }
}

profile_section() {
    local profile=$1 section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^[#;]/ { print }
    ' "$profile"
}

manifest_data() {
    awk 'NF && $1 !~ /^#/ {print}' "$1"
}

read_header() {
    local report=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$report"
}

execution_runnable() {
    awk '
        /^execution: runnable=/ {
            sub(/^execution: runnable=/, "")
            sub(/ .*/, "")
            print
            found=1
        }
        END { if (!found) exit 1 }
    ' "$1"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") {print}' "$1"
}

json_rows() {
    awk '/^\{"kind":"result",/' "$1"
}

json_triplets() {
    awk '
        /^\{"kind":"result",/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!match($0, /"variant":"[^"]*"/)) exit 3
            variant=substr($0, RSTART + 11, RLENGTH - 12)
            if (!match($0, /"outcome":"[^"]*"/)) exit 4
            outcome=substr($0, RSTART + 11, RLENGTH - 12)
            print path "\t" variant "\t" outcome
        }
    ' "$1"
}

json_summary() {
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
        "$(read_value quickjs)" "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" "$profile_hash" \
        "$(read_value schema)" "$(read_value mode)"
}

cleanup() {
    if [[ -n "${tmp_dir-}" && -d "$tmp_dir" ]]; then
        rm -rf -- "$tmp_dir"
    fi
    if [[ "$lock_held" == 1 ]]; then
        rmdir -- "$lock_dir" 2>/dev/null || true
        lock_held=0
    fi
}

cd -- "$root"
[[ -f "$baseline" ]] || { echo "error: missing $baseline" >&2; exit 1; }
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-default-parameters.XXXXXX")
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p -- "$(dirname -- "$lock_dir")"
if ! mkdir -- "$lock_dir" 2>/dev/null; then
    echo "error: another focused default-parameters gate holds $lock_dir" >&2
    exit 1
fi
lock_held=1
consumed_keys=$tmp_dir/consumed-baseline-keys.txt
: >"$consumed_keys"

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 \
    f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 \
    79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_records 53125
expect_value test262_metadata_sha256 \
    a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value parent_profile tests/test262-default-parameters-parent.conf
expect_value parent_features 92
expect_value parent_negative_paths 924
expect_value candidate_profile tests/test262-default-parameters-candidate.conf
expect_value candidate_features 93
expect_value candidate_negative_paths 1143
expect_value added_features 1
expect_value added_negative_paths 219
expect_value profile_execution_entries 1
expect_value universe_manifest tests/test262-default-parameters-universe.txt
expect_value universe_paths 2269
expect_value universe_variants 4516
expect_value universe_sloppy_variants 2266
expect_value universe_strict_variants 2250
expect_value tag_negative_paths 219
expect_value tag_negative_variants 435
expect_value tag_negative_phase parse
expect_value tag_negative_type SyntaxError
expect_value residual_negative_paths 48
expect_value activation_paths 1687
expect_value activation_variants 3352
expect_value positive_activation_paths 1516
expect_value positive_activation_variants 3013
expect_value negative_activation_paths 171
expect_value negative_activation_variants 339
expect_value residual_feature_paths 581
expect_value residual_feature_variants 1162
expect_value host_is_html_dda_paths 1
expect_value host_is_html_dda_variants 2
expect_value parent_runnable 0
expect_value parent_passes 0
expect_value parent_failures 0
expect_value parent_unsupported 4516
expect_value parent_skipped 0
expect_value parent_summary \
    'unsupported-feature=4514 unsupported-host-is-html-dda=2'
expect_value candidate_runnable 3352
expect_value candidate_passes 3352
expect_value candidate_failures 0
expect_value candidate_unsupported 1164
expect_value candidate_skipped 0
expect_value candidate_summary \
    'pass=3352 unsupported-feature=1162 unsupported-host-is-html-dda=2'
expect_value transition_rows 4516
expect_value transition_changed_rows 4514
expect_value transition_outcome_changed_rows 3352
expect_value transition_detail_only_rows 1162
expect_value transition_unchanged_rows 2
expect_value quickjs_paths 1687
expect_value quickjs_variants 3352
expect_value negative_worker_variants 435
expect_value negative_quickjs_paths 219
expect_value negative_quickjs_variants 435

parent_profile=$(read_value parent_profile)
candidate_profile=$(read_value candidate_profile)
universe_manifest=$(read_value universe_manifest)
for asset in "$live_profile" "$parent_profile" "$candidate_profile" \
    "$universe_manifest"; do
    [[ -f "$asset" ]] || { echo "error: missing gate input: $asset" >&2; exit 1; }
done

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ "$(basename -- "$source_dir")" != "quickjs-$(read_value quickjs)" \
    || "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" \
        != "$(read_value test262)" \
    || "$(sha256_file "$source_dir/tests/test262.patch")" \
        != "$(read_value test262_patch_sha256)" \
    || "$(sha256_file "$source_dir/test262.conf")" \
        != "$(read_value test262_config_sha256)" ]]; then
    echo 'error: prepared QuickJS/Test262 inputs drifted' >&2
    exit 1
fi

parent_features=$tmp_dir/parent-features.txt
candidate_features=$tmp_dir/candidate-features.txt
parent_negatives=$tmp_dir/parent-negatives.txt
candidate_negatives=$tmp_dir/candidate-negatives.txt
parent_execution=$tmp_dir/parent-execution.txt
candidate_execution=$tmp_dir/candidate-execution.txt
added_features=$tmp_dir/added-features.txt
added_negatives=$tmp_dir/added-negatives.txt
profile_section "$parent_profile" features >"$parent_features"
profile_section "$candidate_profile" features >"$candidate_features"
profile_section "$parent_profile" audited-negative-tests >"$parent_negatives"
profile_section "$candidate_profile" audited-negative-tests >"$candidate_negatives"
profile_section "$parent_profile" execution >"$parent_execution"
profile_section "$candidate_profile" execution >"$candidate_execution"
for sorted in "$parent_features" "$candidate_features" \
    "$parent_negatives" "$candidate_negatives"; do
    sort -c "$sorted"
    [[ -z "$(uniq -d "$sorted")" ]] || {
        echo "error: duplicate profile entry in $sorted" >&2
        exit 1
    }
done
comm -13 "$parent_features" "$candidate_features" >"$added_features"
comm -23 "$parent_features" "$candidate_features" >"$tmp_dir/removed-features.txt"
comm -13 "$parent_negatives" "$candidate_negatives" >"$added_negatives"
comm -23 "$parent_negatives" "$candidate_negatives" >"$tmp_dir/removed-negatives.txt"

if [[ "$(sha256_file "$parent_profile")" \
        != "$(read_value parent_profile_sha256)" \
    || "$(sha256_file "$candidate_profile")" \
        != "$(read_value candidate_profile_sha256)" \
    || "$(wc -l <"$parent_features" | tr -d '[:space:]')" \
        != "$(read_value parent_features)" \
    || "$(sha256_file "$parent_features")" \
        != "$(read_value parent_features_sha256)" \
    || "$(wc -l <"$candidate_features" | tr -d '[:space:]')" \
        != "$(read_value candidate_features)" \
    || "$(sha256_file "$candidate_features")" \
        != "$(read_value candidate_features_sha256)" \
    || "$(wc -l <"$added_features" | tr -d '[:space:]')" \
        != "$(read_value added_features)" \
    || "$(sha256_file "$added_features")" \
        != "$(read_value added_features_sha256)" \
    || "$(cat "$added_features")" != default-parameters \
    || -s "$tmp_dir/removed-features.txt" \
    || "$(wc -l <"$parent_negatives" | tr -d '[:space:]')" \
        != "$(read_value parent_negative_paths)" \
    || "$(sha256_file "$parent_negatives")" \
        != "$(read_value parent_negative_sha256)" \
    || "$(wc -l <"$candidate_negatives" | tr -d '[:space:]')" \
        != "$(read_value candidate_negative_paths)" \
    || "$(sha256_file "$candidate_negatives")" \
        != "$(read_value candidate_negative_sha256)" \
    || "$(wc -l <"$added_negatives" | tr -d '[:space:]')" \
        != "$(read_value added_negative_paths)" \
    || "$(sha256_file "$added_negatives")" \
        != "$(read_value added_negative_sha256)" \
    || -s "$tmp_dir/removed-negatives.txt" ]]; then
    echo 'error: parent/candidate capability transition drifted' >&2
    exit 1
fi
diff -u "$parent_execution" "$candidate_execution"
if [[ "$(wc -l <"$candidate_execution" | tr -d '[:space:]')" \
        != "$(read_value profile_execution_entries)" \
    || "$(sha256_file "$candidate_execution")" \
        != "$(read_value profile_execution_sha256)" \
    || "$(cat "$candidate_execution")" != async=true ]]; then
    echo 'error: profile execution policy drifted' >&2
    exit 1
fi
if [[ "$live_policy" == current ]] \
    && ! cmp -s "$live_profile" "$parent_profile" \
    && ! cmp -s "$live_profile" "$candidate_profile"; then
    echo 'error: live profile is neither the frozen parent nor candidate' >&2
    exit 1
fi

runner=$tmp_dir/run-test262
cargo build --locked --release --quiet --bin run-test262
cp -- target/release/run-test262 "$runner"
chmod 755 "$runner"
[[ -x "$runner" ]] || {
    echo 'error: failed to stage a stable focused run-test262 binary' >&2
    exit 1
}

metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
"$runner" --suite "$suite" --validate-metadata "$metadata_records"
if [[ "$(sha256_file "$metadata_records")" \
        != "$(read_value test262_metadata_sha256)" ]]; then
    echo 'error: exhaustive Test262 metadata fingerprint drifted' >&2
    exit 1
fi
tr '\0' '\t' <"$metadata_records" >"$metadata_tsv"
if [[ "$(wc -l <"$metadata_tsv" | tr -d '[:space:]')" \
        != "$(read_value test262_metadata_records)" ]] \
    || ! awk -F'\t' 'NF != 6 || $1 == "" {exit 1}' "$metadata_tsv" \
    || ! cut -f1 "$metadata_tsv" | sort -c \
    || [[ -n "$(cut -f1 "$metadata_tsv" | uniq -d)" ]]; then
    echo 'error: exhaustive Test262 metadata record structure drifted' >&2
    exit 1
fi

skipped_features=$tmp_dir/skipped-features.txt
awk '
    $0 == "[features]" {inside=1; next}
    /^\[/ {inside=0}
    inside && NF && $1 !~ /^#/ && /=skip$/ {sub(/=skip$/, ""); print}
' "$source_dir/test262.conf" | sort -u >"$skipped_features"

universe_paths_file=$tmp_dir/universe.paths
universe_keys=$tmp_dir/universe.keys
tag_negatives=$tmp_dir/tag-negative.paths
tag_negative_keys=$tmp_dir/tag-negative.keys
positive_paths=$tmp_dir/positive-activation.paths
positive_keys=$tmp_dir/positive-activation.keys
negative_paths=$tmp_dir/negative-activation.paths
negative_keys=$tmp_dir/negative-activation.keys
residual_paths=$tmp_dir/residual-feature.paths
residual_keys=$tmp_dir/residual-feature.keys
residual_negative_paths=$tmp_dir/residual-negative.paths
host_paths=$tmp_dir/host-is-html-dda.paths
host_keys=$tmp_dir/host-is-html-dda.keys
for output in "$universe_paths_file" "$universe_keys" "$tag_negatives" \
    "$tag_negative_keys" \
    "$positive_paths" "$positive_keys" "$negative_paths" "$negative_keys" \
    "$residual_paths" "$residual_keys" "$residual_negative_paths" \
    "$host_paths" "$host_keys"; do
    : >"$output"
done

if ! awk -F'\t' \
    -v universe="$universe_paths_file" -v universe_keys="$universe_keys" \
    -v negatives="$tag_negatives" -v negative_keys_all="$tag_negative_keys" \
    -v positive="$positive_paths" -v positive_keys="$positive_keys" \
    -v negative="$negative_paths" -v negative_keys="$negative_keys" \
    -v residual="$residual_paths" -v residual_keys="$residual_keys" \
    -v residual_negative="$residual_negative_paths" \
    -v host="$host_paths" -v host_keys="$host_keys" '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    function emit_keys(path, flags, keys_file) {
        if (has(flags, "onlyStrict")) {
            print path "\tstrict" >keys_file
        } else if (has(flags, "noStrict")) {
            print path "\tsloppy" >keys_file
        } else {
            print path "\tsloppy" >keys_file
            print path "\tstrict" >keys_file
        }
    }
    function emit(path, flags, paths_file, keys_file) {
        print path >paths_file
        emit_keys(path, flags, keys_file)
    }
    FILENAME == ARGV[1] {supported[$1]=1; next}
    FILENAME == ARGV[2] {skipped[$1]=1; next}
    !has($4, "default-parameters") {next}
    {
        if (has($3, "module") || has($3, "raw") ||
            (has($3, "onlyStrict") && has($3, "noStrict"))) bad=1
        is_negative=($5 != "")
        if (is_negative) {
            if ($5 != "parse" || $6 != "SyntaxError") bad=1
            print $1 >negatives
            emit_keys($1, $3, negative_keys_all)
        } else if ($6 != "") {
            bad=1
        }
        missing=0
        n=split($4, feature, ",")
        for (i=1; i<=n; i++) {
            if (feature[i] in skipped) bad=1
            if (feature[i] != "default-parameters" &&
                feature[i] != "IsHTMLDDA" &&
                !(feature[i] in supported)) missing++
        }
        emit($1, $3, universe, universe_keys)
        if (has($4, "IsHTMLDDA")) {
            if (missing != 0 || is_negative) bad=1
            emit($1, $3, host, host_keys)
        } else if (missing != 0) {
            emit($1, $3, residual, residual_keys)
            if (is_negative) print $1 >residual_negative
        } else if (is_negative) {
            emit($1, $3, negative, negative_keys)
        } else {
            emit($1, $3, positive, positive_keys)
        }
    }
    END {if (bad) exit 1}
' "$parent_features" "$skipped_features" "$metadata_tsv"; then
    echo 'error: default-parameters metadata contract drifted' >&2
    exit 1
fi

for sorted in "$universe_paths_file" "$universe_keys" "$tag_negatives" \
    "$tag_negative_keys" \
    "$positive_paths" "$positive_keys" "$negative_paths" "$negative_keys" \
    "$residual_paths" "$residual_keys" "$residual_negative_paths" \
    "$host_paths" "$host_keys"; do
    sort -c "$sorted"
    [[ -z "$(uniq -d "$sorted")" ]] || {
        echo "error: duplicate derived inventory entry in $sorted" >&2
        exit 1
    }
done

manifest_paths=$tmp_dir/manifest.paths
manifest_data "$universe_manifest" >"$manifest_paths"
sort -c "$manifest_paths"
diff -u "$manifest_paths" "$universe_paths_file"
diff -u "$tag_negatives" "$added_negatives"
cat "$positive_paths" "$negative_paths" >"$tmp_dir/activation.unsorted"
sort -u "$tmp_dir/activation.unsorted" >"$tmp_dir/activation.paths"
cat "$positive_keys" "$negative_keys" >"$tmp_dir/activation-keys.unsorted"
sort -u "$tmp_dir/activation-keys.unsorted" >"$tmp_dir/activation.keys"
cat "$positive_paths" "$negative_paths" "$residual_paths" "$host_paths" \
    >"$tmp_dir/partition-paths.unsorted"
sort "$tmp_dir/partition-paths.unsorted" >"$tmp_dir/partition.paths"
[[ -z "$(uniq -d "$tmp_dir/partition.paths")" ]] || {
    echo 'error: default-parameters path partitions overlap' >&2
    exit 1
}
diff -u "$universe_paths_file" "$tmp_dir/partition.paths"
cat "$positive_keys" "$negative_keys" "$residual_keys" "$host_keys" \
    >"$tmp_dir/partition-keys.unsorted"
sort "$tmp_dir/partition-keys.unsorted" >"$tmp_dir/partition.keys"
[[ -z "$(uniq -d "$tmp_dir/partition.keys")" ]] || {
    echo 'error: default-parameters key partitions overlap' >&2
    exit 1
}
diff -u "$universe_keys" "$tmp_dir/partition.keys"

check_inventory() {
    local file=$1 count_key=$2 hash_key=$3
    [[ "$(wc -l <"$file" | tr -d '[:space:]')" == "$(read_value "$count_key")" \
        && "$(sha256_file "$file")" == "$(read_value "$hash_key")" ]] || {
        echo "error: default-parameters inventory drifted: $count_key" >&2
        exit 1
    }
}

if [[ "$(sha256_file "$universe_manifest")" \
        != "$(read_value universe_manifest_sha256)" ]]; then
    echo 'error: default-parameters manifest file drifted' >&2
    exit 1
fi
check_inventory "$universe_paths_file" universe_paths universe_paths_sha256
check_inventory "$universe_keys" universe_variants universe_keys_sha256
check_inventory "$tag_negatives" tag_negative_paths tag_negative_sha256
check_inventory "$tag_negative_keys" tag_negative_variants \
    tag_negative_keys_sha256
check_inventory "$residual_negative_paths" residual_negative_paths \
    residual_negative_sha256
check_inventory "$tmp_dir/activation.paths" activation_paths activation_paths_sha256
check_inventory "$tmp_dir/activation.keys" activation_variants activation_keys_sha256
check_inventory "$positive_paths" positive_activation_paths \
    positive_activation_paths_sha256
check_inventory "$positive_keys" positive_activation_variants \
    positive_activation_keys_sha256
check_inventory "$negative_paths" negative_activation_paths \
    negative_activation_paths_sha256
check_inventory "$negative_keys" negative_activation_variants \
    negative_activation_keys_sha256
check_inventory "$residual_paths" residual_feature_paths \
    residual_feature_paths_sha256
check_inventory "$residual_keys" residual_feature_variants \
    residual_feature_keys_sha256
check_inventory "$host_paths" host_is_html_dda_paths \
    host_is_html_dda_paths_sha256
check_inventory "$host_keys" host_is_html_dda_variants \
    host_is_html_dda_keys_sha256
if [[ "$(awk -F'\t' '$2 == "sloppy" {n++} END {print n+0}' "$universe_keys")" \
        != "$(read_value universe_sloppy_variants)" \
    || "$(awk -F'\t' '$2 == "strict" {n++} END {print n+0}' "$universe_keys")" \
        != "$(read_value universe_strict_variants)" ]]; then
    echo 'error: default-parameters strict/sloppy variant inventory drifted' >&2
    exit 1
fi
printf 'Default parameter assets pass: 2,269 paths / 4,516 variants in four exact partitions\n'

quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$tmp_dir/activation.paths"
[[ "${#quickjs_files[@]}" == "$(read_value quickjs_paths)" ]] || {
    echo 'error: QuickJS activation path count drifted' >&2
    exit 1
}
oracle_log=$tmp_dir/quickjs.log
if ! (
    cd -- "$source_dir"
    ./run-test262 -m -c test262.conf -a -T "$workers" \
        -f "${quickjs_files[@]}"
) >"$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    echo 'error: pinned QuickJS failed the default-parameters activation cohort' >&2
    exit 1
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || ! grep -Fq \
        "Average memory statistics for $(read_value quickjs_variants) tests:" \
        "$oracle_log" \
    || [[ "$(sha256_file "$oracle_log")" \
        != "$(read_value quickjs_log_sha256)" ]]; then
    tail -n 100 "$oracle_log" >&2
    echo 'error: pinned QuickJS activation receipt drifted' >&2
    exit 1
fi
printf 'Pinned QuickJS passes all %s activation variants\n' \
    "$(read_value quickjs_variants)"

negative_worker_receipt=$tmp_dir/negative-worker.tsv
: >"$negative_worker_receipt"
while IFS=$'\t' read -r test_path variant; do
    result=$("$runner" --worker-one --suite "$suite" \
        --test "$test_path" --variant "$variant")
    printf '%s\t%s\t%s\n' "$test_path" "$variant" "$result" \
        >>"$negative_worker_receipt"
done <"$tag_negative_keys"
if [[ "$(wc -l <"$negative_worker_receipt" | tr -d '[:space:]')" \
        != "$(read_value negative_worker_variants)" \
    || "$(sha256_file "$negative_worker_receipt")" \
        != "$(read_value negative_worker_receipt_sha256)" ]] \
    || ! awk -F'\t' '
        NF != 6 || $3 != "pass" || $4 != "parse" ||
            $5 != "SyntaxError" || $6 == "" {exit 1}
    ' "$negative_worker_receipt"; then
    echo 'error: raw Oxide negative-provenance receipt drifted' >&2
    exit 1
fi
awk -F'\t' '{print $1 "\t" $2}' "$negative_worker_receipt" \
    >"$tmp_dir/negative-worker.keys"
diff -u "$tag_negative_keys" "$tmp_dir/negative-worker.keys"
printf 'Oxide passes all %s forced negative variants\n' \
    "$(read_value negative_worker_variants)"

negative_quickjs_files=()
while IFS= read -r test_path; do
    negative_quickjs_files+=("test262/$test_path")
done <"$tag_negatives"
[[ "${#negative_quickjs_files[@]}" == "$(read_value negative_quickjs_paths)" ]] || {
    echo 'error: QuickJS negative path count drifted' >&2
    exit 1
}
negative_oracle_log=$tmp_dir/negative-quickjs.log
if ! (
    cd -- "$source_dir"
    ./run-test262 -m -c test262.conf -a -T "$workers" \
        -f "${negative_quickjs_files[@]}"
) >"$negative_oracle_log" 2>&1; then
    tail -n 100 "$negative_oracle_log" >&2
    echo 'error: pinned QuickJS failed the forced negative cohort' >&2
    exit 1
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
        "$negative_oracle_log" \
    || ! grep -Fq \
        "Average memory statistics for $(read_value negative_quickjs_variants) tests:" \
        "$negative_oracle_log" \
    || [[ "$(sha256_file "$negative_oracle_log")" \
        != "$(read_value negative_quickjs_log_sha256)" ]]; then
    tail -n 100 "$negative_oracle_log" >&2
    echo 'error: pinned QuickJS negative-provenance receipt drifted' >&2
    exit 1
fi
printf 'Pinned QuickJS passes all %s forced negative variants\n' \
    "$(read_value negative_quickjs_variants)"

run_report() {
    local label=$1 profile=$2
    local report=$tmp_dir/$label.tsv json=$tmp_dir/$label.jsonl
    local output_file=$tmp_dir/$label.output rows=$tmp_dir/$label.rows
    local json_data=$tmp_dir/$label-json.rows
    if ! "$runner" \
        --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$universe_manifest" \
        --report "$report" --mode "$(read_value mode)" \
        --workers "$workers" --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures >"$output_file"; then
        cat "$output_file" >&2
        echo "error: $label focused Test262 run failed" >&2
        exit 1
    fi
    cat "$output_file"
    report_rows "$report" >"$rows"
    json_rows "$json" >"$json_data"
    json_triplets "$json" >"$tmp_dir/$label-json.triplets"
    awk -F'\t' 'NF != 10 {exit 1}' "$rows" || {
        echo "error: $label TSV row schema drifted" >&2
        exit 1
    }
    awk -v expected="$(read_value universe_variants)" '
        NR == 1 && /^\{"kind":"metadata",/ {metadata++; next}
        /^\{"kind":"result",/ {results++; next}
        /^\{"kind":"summary",/ {summary++; summary_line=NR; next}
        {exit 1}
        END {
            if (metadata != 1 || results != expected || summary != 1 ||
                summary_line != NR) exit 1
        }
    ' "$json" || { echo "error: $label JSONL structure drifted" >&2; exit 1; }

    local profile_hash expected_summary
    profile_hash=$(read_value "${label}_profile_sha256")
    expected_summary=$(read_value "${label}_summary")
    if [[ "$(read_header "$report" quickjs)" != "$(read_value quickjs)" \
        || "$(read_header "$report" test262)" != "$(read_value test262)" \
        || "$(read_header "$report" test262_patch_sha256)" \
            != "$(read_value test262_patch_sha256)" \
        || "$(read_header "$report" test262_config_sha256)" \
            != "$(read_value test262_config_sha256)" \
        || "$(read_header "$report" test262_metadata_sha256)" \
            != "$(read_value test262_metadata_sha256)" \
        || "$(read_header "$report" oxide_profile_sha256)" != "$profile_hash" \
        || "$(read_header "$report" profile)" != "$(read_value schema)" \
        || "$(read_header "$report" mode)" != "$(read_value mode)" \
        || "$(head -n 1 "$json")" != "$(expected_json_metadata "$profile_hash")" \
        || "$(execution_runnable "$output_file")" \
            != "$(read_value "${label}_runnable")" \
        || "$(tail -n 1 "$report")" != "# summary $expected_summary" \
        || "$(json_summary "$json")" != "$expected_summary" \
        || "$(wc -l <"$rows" | tr -d '[:space:]')" \
            != "$(read_value universe_variants)" \
        || "$(wc -l <"$json_data" | tr -d '[:space:]')" \
            != "$(read_value universe_variants)" \
        || "$(sha256_file "$rows")" \
            != "$(read_value "${label}_tsv_data_sha256")" \
        || "$(sha256_file "$json_data")" \
            != "$(read_value "${label}_jsonl_data_sha256")" \
        || "$(awk -F'\t' '$7 != "pass" {print}' "$rows" | sha256_stream)" \
            != "$(read_value "${label}_nonpass_sha256")" \
        || "$(sha256_file "$report")" != "$(read_value "${label}_tsv_sha256")" \
        || "$(sha256_file "$json")" != "$(read_value "${label}_jsonl_sha256")" ]]; then
        echo "error: $label report receipt drifted" >&2
        exit 1
    fi

    local passes unsupported skipped failures total
    total=$(wc -l <"$rows" | tr -d '[:space:]')
    passes=$(awk -F'\t' '$7 == "pass" {n++} END {print n+0}' "$rows")
    unsupported=$(awk -F'\t' '$7 ~ /^unsupported-/ {n++} END {print n+0}' "$rows")
    skipped=$(awk -F'\t' '$7 ~ /^skipped-/ {n++} END {print n+0}' "$rows")
    failures=$((total - passes - unsupported - skipped))
    if [[ "$passes" != "$(read_value "${label}_passes")" \
        || "$failures" != "$(read_value "${label}_failures")" \
        || "$unsupported" != "$(read_value "${label}_unsupported")" \
        || "$skipped" != "$(read_value "${label}_skipped")" ]]; then
        echo "error: $label outcome totals drifted" >&2
        exit 1
    fi
    awk -F'\t' '{print $1 "\t" $2}' "$rows" | sort >"$tmp_dir/$label.keys"
    awk -F'\t' '{print $1 "\t" $2}' "$tmp_dir/$label-json.triplets" \
        | sort >"$tmp_dir/$label-json.keys"
    awk -F'\t' '{print $1 "\t" $2 "\t" $7}' "$rows" | sort \
        >"$tmp_dir/$label-tsv.triplets"
    sort "$tmp_dir/$label-json.triplets" >"$tmp_dir/$label-json-triplets.sorted"
    diff -u "$universe_keys" "$tmp_dir/$label.keys"
    diff -u "$universe_keys" "$tmp_dir/$label-json.keys"
    diff -u "$tmp_dir/$label-tsv.triplets" \
        "$tmp_dir/$label-json-triplets.sorted"
}

run_report parent "$parent_profile"
run_report candidate "$candidate_profile"

positive_rows=$tmp_dir/positive-activation.rows
negative_rows=$tmp_dir/negative-activation.rows
residual_rows=$tmp_dir/residual-feature.rows
host_rows=$tmp_dir/host-is-html-dda.rows
for rows in "$positive_rows" "$negative_rows" "$residual_rows" "$host_rows"; do
    : >"$rows"
done
if ! awk -F'\t' -v positive="$positive_rows" -v negative="$negative_rows" \
    -v residual="$residual_rows" -v host="$host_rows" '
    $7 == "pass" && $5 == "normal" && $6 == "" &&
        $8 == "normal" && $9 == "" && $10 == "" {
        print >positive; next
    }
    $7 == "pass" && $5 == "parse" && $6 == "SyntaxError" &&
        $8 == "parse" && $9 == "SyntaxError" && $10 != "" {
        print >negative; next
    }
    $7 == "unsupported-feature" && $8 == "selection" &&
        $9 == "EngineCapability" &&
        $10 ~ /^quickjs-oxide does not declare Test262 feature support: / {
        print >residual; next
    }
    $7 == "unsupported-host-is-html-dda" && $8 == "selection" &&
        $9 == "HostCapability" &&
        $10 == "missing execution capabilities: is-html-dda" {
        print >host; next
    }
    {exit 1}
' "$tmp_dir/candidate.rows"; then
    echo 'error: candidate rows do not match the four exhaustive partitions' >&2
    exit 1
fi

verify_partition() {
    local prefix=$1 rows=$2 expected_keys=$3
    local keys=$tmp_dir/$prefix-candidate.keys
    awk -F'\t' '{print $1 "\t" $2}' "$rows" | sort >"$keys"
    diff -u "$expected_keys" "$keys"
    if [[ "$(wc -l <"$rows" | tr -d '[:space:]')" \
            != "$(read_value "${prefix}_variants")" \
        || "$(sha256_file "$rows")" \
            != "$(read_value "${prefix}_candidate_rows_sha256")" ]]; then
        echo "error: candidate $prefix partition receipt drifted" >&2
        exit 1
    fi
}
verify_partition positive_activation "$positive_rows" "$positive_keys"
verify_partition negative_activation "$negative_rows" "$negative_keys"
verify_partition residual_feature "$residual_rows" "$residual_keys"
verify_partition host_is_html_dda "$host_rows" "$host_keys"

transition=$tmp_dir/transition.data
if ! awk -F'\t' -v OFS='\t' \
    -v expected="$(read_value transition_rows)" \
    -v expected_changed="$(read_value transition_changed_rows)" \
    -v expected_outcome="$(read_value transition_outcome_changed_rows)" \
    -v expected_detail="$(read_value transition_detail_only_rows)" \
    -v expected_unchanged="$(read_value transition_unchanged_rows)" '
    NR == FNR {
        key=$1 SUBSEP $2
        if (key in old) exit 2
        for (i=1; i<=10; i++) before[key,i]=$i
        old[key]=1
        old_count++
        next
    }
    {
        key=$1 SUBSEP $2
        if (!(key in old) || key in seen) exit 3
        for (i=1; i<=6; i++) if ($i != before[key,i]) exit 4
        if ($7 == "unsupported-host-is-html-dda") {
            if (before[key,7] != $7 || before[key,8] != $8 ||
                before[key,9] != $9 || before[key,10] != $10 ||
                $8 != "selection" || $9 != "HostCapability" ||
                $10 != "missing execution capabilities: is-html-dda") exit 5
        } else {
            if (before[key,7] != "unsupported-feature" ||
                before[key,8] != "selection" ||
                before[key,9] != "EngineCapability" ||
                index(before[key,10], "default-parameters") == 0) exit 6
            if ($7 == "pass") {
                if (!(($5 == "normal" && $8 == "normal" && $9 == "" && $10 == "") ||
                      ($5 == "parse" && $6 == "SyntaxError" &&
                       $8 == "parse" && $9 == "SyntaxError" && $10 != ""))) exit 7
            } else if ($7 == "unsupported-feature") {
                if ($8 != "selection" || $9 != "EngineCapability" ||
                    index($10, "default-parameters") != 0 ||
                    $10 !~ /^quickjs-oxide does not declare Test262 feature support: /) exit 8
            } else exit 9
        }
        for (i=1; i<=6; i++) printf "%s%s", $i, OFS
        for (i=7; i<=10; i++) printf "%s%s", before[key,i], OFS
        for (i=7; i<=9; i++) printf "%s%s", $i, OFS
        print $10
        before_tuple=before[key,7] SUBSEP before[key,8] SUBSEP \
            before[key,9] SUBSEP before[key,10]
        after_tuple=$7 SUBSEP $8 SUBSEP $9 SUBSEP $10
        if (before_tuple == after_tuple) unchanged++
        else if (before[key,7] != $7) {changed++; outcome++}
        else {changed++; detail++}
        seen[key]=1
        seen_count++
    }
    END {
        if (old_count != expected || seen_count != expected ||
            changed != expected_changed || outcome != expected_outcome ||
            detail != expected_detail || unchanged != expected_unchanged) exit 10
        for (key in old) if (!(key in seen)) exit 11
    }
' "$tmp_dir/parent.rows" "$tmp_dir/candidate.rows" >"$transition"; then
    echo 'error: keyed parent/candidate transition drifted' >&2
    exit 1
fi
if [[ "$(wc -l <"$transition" | tr -d '[:space:]')" \
        != "$(read_value transition_rows)" \
    || "$(sha256_file "$transition")" \
        != "$(read_value transition_data_sha256)" ]]; then
    echo 'error: exact transition receipt drifted' >&2
    exit 1
fi

awk -F= 'NF && $1 !~ /^#/ {print $1}' "$baseline" | sort \
    >"$tmp_dir/all-baseline-keys.txt"
sort -u "$consumed_keys" >"$tmp_dir/consumed-baseline-keys-sorted.txt"
if [[ -n "$(awk -F= '
    NF && $1 !~ /^#/ {
        if (index($0, "=") <= 1 || $1 ~ /[[:space:]]/ || seen[$1]++) print $1
    }
' "$baseline")" ]]; then
    echo 'error: duplicate or malformed baseline keys' >&2
    exit 1
fi
diff -u "$tmp_dir/all-baseline-keys.txt" \
    "$tmp_dir/consumed-baseline-keys-sorted.txt"

printf 'Default parameters focused gate passes: QuickJS and Oxide pass all 3,352 activation variants; 4,516/4,516 rows classified\n'
