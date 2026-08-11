#!/usr/bin/env bash
# Validate or run every authenticated QuickJS fixture through one data-driven gate.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
registry=$root/dev-support/quickjs-fixture-gates.tsv
oxide=
oxide_was_set=false
selection=all
case_id=
validate_only=false
selection_was_set=false

usage() {
    printf 'usage: %s [--all | --case ID] [--oxide PATH]\n' "${0##*/}"
    printf '       %s --validate\n' "${0##*/}"
    printf '  --all         run every registered fixture (default)\n'
    printf '  --case ID     run one registered fixture\n'
    printf '  --oxide PATH  also require byte-for-byte quickjs-oxide parity\n'
    printf '  --validate    validate registry schema and authenticated inputs only\n'
}

die() {
    echo "error: $*" >&2
    exit 1
}

usage_error() {
    usage >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --all | --check)
            $selection_was_set && usage_error
            selection=all
            selection_was_set=true
            shift
            ;;
        --case)
            $selection_was_set && usage_error
            [[ $# -ge 2 ]] || usage_error
            selection=case
            case_id=$2
            selection_was_set=true
            shift 2
            ;;
        --oxide)
            [[ $# -ge 2 ]] || usage_error
            oxide=$2
            oxide_was_set=true
            shift 2
            ;;
        --validate)
            $validate_only && usage_error
            validate_only=true
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) usage_error ;;
    esac
done

if $validate_only && { $oxide_was_set || [[ "$selection" == case ]]; }; then
    usage_error
fi
if ! $validate_only && ! $oxide_was_set; then
    oxide=${OXIDE_QJS:-}
fi
if [[ "$selection" == case && ! "$case_id" =~ ^r3[a-z0-9]+$ ]]; then
    usage_error
fi
[[ -f "$registry" && ! -L "$registry" ]] || die "fixture registry is missing: $registry"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "sha256sum or shasum is required"
    fi
}

verify_hash() {
    local path=$1
    local pinned=$2
    local actual
    [[ -f "$path" && ! -L "$path" ]] || die "oracle input is missing: $path"
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$pinned" ]]; then
        echo "error: oracle input hash drifted: $path" >&2
        echo "expected: $pinned" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-fixture-gates.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
rows=$tmp_dir/rows.tsv

expected_header=$'case_id\tfixture\texpected\tfixture_sha256\texpected_sha256\tmode\ttranscript\tcompletion\tlabel'
header=$(sed -n '1p' "$registry")
[[ "$header" == "$expected_header" ]] || die "unsupported fixture registry schema"

tail -n +2 "$registry" | while IFS= read -r line; do
    [[ -n "$line" ]] || die "fixture registry contains a blank row"
    [[ "$line" != *$'\r'* && "$line" != *$'\t' ]] \
        || die "fixture registry contains non-canonical whitespace"
    field_count=$(awk -F '\t' '{ print NF }' <<<"$line")
    [[ "$field_count" == 9 ]] || die "fixture registry row must have 9 columns"
    IFS=$'\t' read -r id fixture expected fixture_sha expected_sha mode transcript completion label <<<"$line"
    [[ "$id" =~ ^r3[a-z0-9]+$ ]] || die "invalid fixture case id: $id"
    [[ "$fixture" =~ ^tests/fixtures/[a-z0-9_]+\.js$ ]] \
        || die "invalid fixture path for $id: $fixture"
    [[ "$expected" =~ ^tests/fixtures/[a-z0-9_]+\.quickjs-2026-06-04\.txt$ ]] \
        || die "invalid expected path for $id: $expected"
    fixture_stem=${fixture%.js}
    [[ "${fixture##*/}" == "$id"_*.js ]] \
        || die "fixture basename does not match case id: $id"
    [[ "$expected" == "$fixture_stem.quickjs-2026-06-04.txt" ]] \
        || die "expected transcript does not match fixture: $id"
    [[ "$fixture_sha" =~ ^[0-9a-f]{64}$ ]] || die "invalid fixture hash for $id"
    [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || die "invalid expected hash for $id"
    [[ "$label" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || die "invalid label for $id"
    case $mode in
        value)
            [[ "$transcript" == "${id}Transcript" && "$completion" == - ]] \
                || die "invalid value-mode fields for $id"
            ;;
        promise)
            [[ "$transcript" == "${id}Transcript" ]] \
                || die "invalid transcript identifier for $id"
            [[ "$completion" == "${id}Done" ]] \
                || die "invalid completion identifier for $id"
            ;;
        direct)
            [[ "$transcript" == - && "$completion" == - ]] \
                || die "invalid direct-mode fields for $id"
            ;;
        *) die "invalid fixture mode for $id: $mode" ;;
    esac
    verify_hash "$root/$fixture" "$fixture_sha"
    verify_hash "$root/$expected" "$expected_sha"
    if [[ "$mode" == promise ]]; then
        last_line=$(tail -n 1 "$root/$fixture")
        [[ "$last_line" == "$completion;" ]] \
            || die "promise fixture does not end with $completion;: $id"
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$id" "$fixture" "$expected" "$fixture_sha" "$expected_sha" \
        "$mode" "$transcript" "$completion" "$label"
done >"$rows"

[[ -s "$rows" ]] || die "fixture registry has no cases"
for column in 1 2 3; do
    duplicates=$(cut -f "$column" "$rows" | sort | uniq -d)
    [[ -z "$duplicates" ]] || die "duplicate fixture registry value: $duplicates"
done
sort -c -t $'\t' -k1,1 "$rows" \
    || die "fixture registry cases must be sorted by id"
case_count=$(wc -l <"$rows" | tr -d '[:space:]')

if $validate_only; then
    echo "QuickJS fixture registry passed: $case_count authenticated cases."
    exit 0
fi

if [[ "$selection" == case ]] && ! awk -F '\t' -v id="$case_id" '$1 == id { found=1 } END { exit !found }' "$rows"; then
    die "unknown fixture case: $case_id"
fi
selected_count=$case_count
if [[ "$selection" == case ]]; then
    selected_count=1
fi

oracle=$("$script_dir/build-quickjs-oracle.sh")
[[ -x "$oracle" ]] || die "pinned QuickJS oracle is not executable: $oracle"
if [[ -n "$oxide" && ! -x "$oxide" ]]; then
    echo "error: quickjs-oxide qjs is not executable: $oxide" >&2
    exit 2
fi

run_engine() {
    local label=$1
    local engine=$2
    local output=$3
    local errors=$4
    shift 4
    if ! "$engine" "$@" >"$output" 2>"$errors"; then
        echo "error: $label failed to execute" >&2
        sed -n '1,200p' "$errors" >&2
        exit 1
    fi
    if [[ -s "$errors" ]]; then
        echo "error: $label emitted unexpected stderr" >&2
        sed -n '1,200p' "$errors" >&2
        exit 1
    fi
}

compare_transcript() {
    local label=$1
    local expected=$2
    local actual=$3
    if ! cmp -s -- "$expected" "$actual"; then
        echo "error: $label transcript drifted" >&2
        diff -u -- "$expected" "$actual" >&2 || true
        exit 1
    fi
}

run_case() {
    local row=$1
    local id fixture expected fixture_sha expected_sha mode transcript completion label
    local fixture_path expected_path engine_fixture
    local quickjs_out quickjs_err oxide_out oxide_err
    local -a quickjs_args oxide_args
    IFS=$'\t' read -r id fixture expected fixture_sha expected_sha mode transcript completion label <<<"$row"
    fixture_path=$root/$fixture
    expected_path=$root/$expected
    engine_fixture=$fixture_path

    case $mode in
        value)
            engine_fixture=$tmp_dir/$id.js
            {
                cat "$fixture_path"
                printf '\nprint(%s.join("\\n"));\n' "$transcript"
            } >"$engine_fixture"
            quickjs_args=(--script "$engine_fixture")
            oxide_args=(--print-result "$fixture_path")
            ;;
        promise)
            engine_fixture=$tmp_dir/$id.js
            {
                sed '$d' "$fixture_path"
                printf '\n%s.then(\n' "$completion"
                printf '    function () { print(%s.join("\\n")); },\n' "$transcript"
                printf '    function (error) { throw error; }\n'
                printf ');\n'
            } >"$engine_fixture"
            quickjs_args=(--std --script "$engine_fixture")
            oxide_args=("$engine_fixture")
            ;;
        direct)
            quickjs_args=(--std --script "$fixture_path")
            oxide_args=("$fixture_path")
            ;;
    esac

    quickjs_out=$tmp_dir/$id.quickjs.out
    quickjs_err=$tmp_dir/$id.quickjs.err
    run_engine "pinned QuickJS 2026-06-04 ($id)" "$oracle" \
        "$quickjs_out" "$quickjs_err" "${quickjs_args[@]}"
    compare_transcript "pinned QuickJS 2026-06-04 ($id)" "$expected_path" "$quickjs_out"

    if [[ -n "$oxide" ]]; then
        oxide_out=$tmp_dir/$id.oxide.out
        oxide_err=$tmp_dir/$id.oxide.err
        run_engine "quickjs-oxide ($id)" "$oxide" \
            "$oxide_out" "$oxide_err" "${oxide_args[@]}"
        compare_transcript "quickjs-oxide ($id)" "$expected_path" "$oxide_out"
        if ! cmp -s -- "$quickjs_out" "$oxide_out"; then
            echo "error: quickjs-oxide differs from pinned QuickJS 2026-06-04 ($id)" >&2
            diff -u -- "$quickjs_out" "$oxide_out" >&2 || true
            exit 1
        fi
        echo "$id $label differential passed"
    else
        echo "$id $label oracle passed"
    fi
}

ran=0
while IFS= read -r row; do
    id=${row%%$'\t'*}
    if [[ "$selection" == all || "$id" == "$case_id" ]]; then
        run_case "$row"
        ran=$((ran + 1))
    fi
done <"$rows"

if [[ -n "$oxide" ]]; then
    echo "QuickJS fixture differential passed: $ran/$selected_count selected cases match."
else
    echo "QuickJS fixture oracle passed: $ran/$selected_count selected cases are stable."
fi
