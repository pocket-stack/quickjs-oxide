#!/usr/bin/env bash
# Ensure every nested oracle source is compiled by its aggregate target.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

die() {
    echo "error: $*" >&2
    exit 1
}

command -v rg >/dev/null 2>&1 || die 'rg is required'
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-oracle-registry.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

check_registry() {
    local key=$1
    local label=$2
    local entry=$3
    local source_dir=$4
    local path_prefix=$5
    local module_prefix=$6
    local actual=$tmp/$key.actual
    local declared=$tmp/$key.declared
    local duplicates
    local count

    [[ -f "$entry" && ! -L "$entry" ]] || die "missing $label aggregate entry: $entry"
    [[ -d "$source_dir" && ! -L "$source_dir" ]] \
        || die "missing $label oracle source directory: $source_dir"

    rg --files "$source_dir" -g '*.rs' | sort >"$actual"
    [[ -s "$actual" ]] || die "no $label oracle sources found under $source_dir"

    awk -v path_prefix="$path_prefix" -v module_prefix="$module_prefix" '
        function fail(message) {
            print "error: " message > "/dev/stderr"
            failed=1
        }
        BEGIN {
            path_pattern="^#\\[path = \"" path_prefix "/[^\"]+\\.rs\"\\]$"
            malformed_pattern="^#\\[path = \"" path_prefix "/"
            module_pattern="^mod " module_prefix
        }
        $0 ~ path_pattern {
            path=$0
            sub(/^#\[path = "/, "", path)
            sub(/"\]$/, "", path)
            stem=path
            sub(/^.*\//, "", stem)
            sub(/\.rs$/, "", stem)
            if ((getline declaration) <= 0 || declaration != "mod " stem ";") {
                fail("path " path " must be followed by mod " stem ";")
            }
            print "tests/" path
            next
        }
        $0 ~ malformed_pattern {
            fail("malformed oracle path declaration: " $0)
        }
        $0 ~ module_pattern {
            fail("oracle module lacks an adjacent path declaration: " $0)
        }
        END { exit failed }
    ' "$entry" | sort >"$declared"

    duplicates=$(uniq -d "$declared")
    [[ -z "$duplicates" ]] || die "duplicate $label oracle declarations: $duplicates"
    if ! cmp -s "$actual" "$declared"; then
        diff -u "$actual" "$declared" >&2 || true
        die "$label oracle registry does not match nested sources"
    fi

    count=$(wc -l <"$actual" | tr -d '[:space:]')
    printf '%s oracle registry covers %s modules.\n' "$label" "$count"
}

check_registry array Array tests/oracle_array_methods.rs \
    tests/oracle/array oracle/array oracle_array_
check_registry string String tests/oracle_string_methods.rs \
    tests/oracle/string oracle/string oracle_string_
check_registry object Object tests/oracle_object_semantics.rs \
    tests/oracle/object oracle/object oracle_object
check_registry regexp RegExp tests/oracle_regexp.rs \
    tests/oracle/regexp oracle/regexp oracle_regexp_
check_registry promise Promise tests/oracle_promise.rs \
    tests/oracle/promise oracle/promise oracle_promise_
check_registry collections Collections tests/oracle_collections.rs \
    tests/oracle/collections oracle/collections oracle_
check_registry number_kernels "Number kernels" tests/oracle_number_kernels.rs \
    tests/oracle/number_kernels oracle/number_kernels oracle_
check_registry updates Updates tests/oracle_updates.rs \
    tests/oracle/update oracle/update oracle_update_
check_registry function_declarations "Function declarations" \
    tests/oracle_function_declarations.rs tests/oracle/function_declarations \
    oracle/function_declarations oracle_
check_registry errors Errors tests/oracle_error_semantics.rs \
    tests/oracle/errors oracle/errors oracle_
check_registry parameters Parameters tests/oracle_parameters.rs \
    tests/oracle/parameters oracle/parameters oracle_
check_registry exponentiation Exponentiation tests/oracle_exponentiation.rs \
    tests/oracle/exponentiation oracle/exponentiation oracle_power_
check_registry async_methods "Async methods" tests/oracle_async_methods.rs \
    tests/oracle/async_methods oracle/async_methods oracle_async_
check_registry control_flow "Control flow" tests/oracle_control_flow.rs \
    tests/oracle/control_flow oracle/control_flow oracle_
check_registry typed_array TypedArray tests/oracle_typed_array_methods.rs \
    tests/oracle/typed_array oracle/typed_array oracle_typed_array_
check_registry program_declarations "Program declarations" \
    tests/oracle_program_declarations.rs tests/oracle/program_declarations \
    oracle/program_declarations oracle_program_
check_registry json JSON tests/oracle_json.rs \
    tests/oracle/json oracle/json oracle_json_
check_registry arguments Arguments tests/oracle_argument_semantics.rs \
    tests/oracle/arguments oracle/arguments oracle_argument
check_registry iterator Iterator tests/oracle_iterator_methods.rs \
    tests/oracle/iterator oracle/iterator oracle_iterator_
check_registry unicode_lexical "Unicode lexical" tests/oracle_unicode_lexical.rs \
    tests/oracle/unicode_lexical oracle/unicode_lexical oracle_unicode_
check_registry binary_data "Binary data" tests/oracle_binary_data.rs \
    tests/oracle/binary_data oracle/binary_data oracle_
check_registry member_access "Member access" tests/oracle_member_access.rs \
    tests/oracle/member_access oracle/member_access oracle_member_
