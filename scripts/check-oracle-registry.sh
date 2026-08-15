#!/usr/bin/env bash
# Ensure the shared harness covers every wrapper and every nested oracle source.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

die() {
    echo "error: $*" >&2
    exit 1
}

compiled=false
case ${1:-} in
    '') ;;
    --compiled) compiled=true ;;
    *) die "usage: ${0##*/} [--compiled]" ;;
esac

command -v rg >/dev/null 2>&1 || die 'rg is required'
command -v node >/dev/null 2>&1 || die 'node is required'
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-oracle-registry.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

check_harness() {
    local entry=tests/oracle.rs
    local actual=$tmp/harness.actual
    local declared=$tmp/harness.declared
    local duplicates
    local default_test_count
    local host_cfg_actual=$tmp/harness.host-cfg.actual
    local host_test_count
    local host_wrapper
    local expected_count
    local local_count
    local wrapper_count
    local test_count

    [[ -f "$entry" && ! -L "$entry" ]] || die "missing shared oracle harness: $entry"

    rg --files tests -g 'oracle_*.rs' | awk -F/ 'NF == 2' | sort >"$actual"
    [[ -s "$actual" ]] || die 'no top-level oracle wrappers found under tests'
    while IFS= read -r wrapper; do
        [[ -f "$wrapper" && ! -L "$wrapper" ]] \
            || die "oracle wrapper must be a regular file: $wrapper"
    done <"$actual"

    awk '
        function fail(message) {
            print "error: " message > "/dev/stderr"
            failed=1
        }
        BEGIN {
            path_pattern="^#\\[path = \"oracle_[^\"]+\\.rs\"\\]$"
            malformed_pattern="^#\\[path = \"oracle_"
            module_pattern="^mod (oracle_|test262_)"
            feature_gate="#[cfg(feature = \"test262-host\")]"
        }
        $0 ~ path_pattern {
            path=$0
            sub(/^#\[path = "/, "", path)
            sub(/"\]$/, "", path)
            stem=path
            sub(/\.rs$/, "", stem)
            module=stem
            if (stem == "oracle_create_realm") module="test262_create_realm"
            if (stem == "oracle_host_gc") module="test262_host_gc"
            if (stem == "oracle_is_html_dda") module="test262_is_html_dda"
            if (module ~ /^test262_/ && previous != feature_gate) {
                fail("host oracle path " path " must immediately follow " feature_gate)
            }
            if (module !~ /^test262_/ && previous == feature_gate) {
                fail("non-host oracle path " path " must not follow " feature_gate)
            }
            if ((getline declaration) <= 0 || declaration != "mod " module ";") {
                fail("path " path " must be followed by mod " module ";")
            }
            print "tests/" path
            previous=declaration
            next
        }
        $0 ~ malformed_pattern {
            fail("malformed harness path declaration: " $0)
        }
        $0 == feature_gate { gate_count++ }
        $0 ~ module_pattern {
            fail("harness module lacks an adjacent path declaration: " $0)
        }
        { previous=$0 }
        END {
            if (gate_count != 3) {
                fail("shared harness must contain exactly 3 host feature gates")
            }
            exit failed
        }
    ' "$entry" >"$declared"

    duplicates=$(sort "$declared" | uniq -d)
    [[ -z "$duplicates" ]] || die "duplicate oracle harness declarations: $duplicates"
    if ! cmp -s "$actual" "$declared"; then
        diff -u "$actual" "$declared" >&2 || true
        die 'shared oracle harness does not match the sorted wrapper inventory'
    fi

    wrapper_count=$(wc -l <"$actual" | tr -d '[:space:]')
    [[ "$wrapper_count" == 50 ]] \
        || die "oracle wrapper count drifted: expected 50, found $wrapper_count"

    if rg -l --fixed-strings 'cfg(feature = "test262-host")' \
        tests/oracle_*.rs tests/oracle -g '*.rs' \
        | sort >"$host_cfg_actual"; then
        sed -n '1,40p' "$host_cfg_actual" >&2
        die 'test262-host gating must live only on shared harness module declarations'
    fi

    test_count=$(
        {
            rg -o --no-filename '^[[:space:]]*#\[test\]' tests/oracle_*.rs
            rg -o --no-filename '^[[:space:]]*#\[test\]' tests/oracle -g '*.rs'
        } | wc -l | tr -d '[:space:]'
    )
    [[ "$test_count" == 908 ]] \
        || die "oracle test count drifted: expected 908, found $test_count"
    host_test_count=0
    while IFS=$'\t' read -r host_wrapper expected_count; do
        local_count=$(
            rg -o --no-filename '^[[:space:]]*#\[test\]' "$host_wrapper" \
                | wc -l | tr -d '[:space:]'
        )
        [[ "$local_count" == "$expected_count" ]] \
            || die "$host_wrapper test count drifted: expected $expected_count, found $local_count"
        host_test_count=$((host_test_count + local_count))
    done <<'EOF'
tests/oracle_create_realm.rs	3
tests/oracle_host_gc.rs	1
tests/oracle_is_html_dda.rs	1
EOF
    [[ "$host_test_count" == 5 ]] \
        || die "host oracle test count drifted: expected 5, found $host_test_count"
    default_test_count=$((test_count - host_test_count))
    [[ "$default_test_count" == 903 ]] \
        || die "default oracle test count drifted: expected 903, found $default_test_count"

    printf 'Oracle harness covers %s wrappers / %s default + %s host tests.\n' \
        "$wrapper_count" "$default_test_count" "$host_test_count"
}

count_listed_tests() {
    awk '/: test$/ { count++ } END { print count + 0 }'
}

check_compiled_harness() {
    local default_count
    local host_count
    local host_filter_count

    default_count=$(
        cargo test --locked --test oracle -- --list \
            | count_listed_tests
    )
    [[ "$default_count" == 903 ]] \
        || die "compiled default oracle list drifted: expected 903, found $default_count"

    host_count=$(
        cargo test --locked --features test262-host --test oracle -- --list \
            | count_listed_tests
    )
    [[ "$host_count" == 908 ]] \
        || die "compiled host oracle list drifted: expected 908, found $host_count"

    host_filter_count=$(
        cargo test --locked --features test262-host \
            --test oracle test262_ -- --list \
            | count_listed_tests
    )
    [[ "$host_filter_count" == 5 ]] \
        || die "compiled host filter drifted: expected 5, found $host_filter_count"

    printf 'Compiled oracle harness lists %s default / %s host / %s filtered host tests.\n' \
        "$default_count" "$host_count" "$host_filter_count"
}

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

check_harness
node "$script_dir/check-oracle-helper-duplication.mjs"
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
check_registry templates "Template semantics" tests/oracle_template_semantics.rs \
    tests/oracle/templates oracle/templates oracle_
check_registry global "Global semantics" tests/oracle_global_semantics.rs \
    tests/oracle/global oracle/global oracle_global_
check_registry proxy_reflect "Proxy and Reflect" tests/oracle_proxy_reflect.rs \
    tests/oracle/proxy_reflect oracle/proxy_reflect oracle_
check_registry class_initialization "Class initialization" \
    tests/oracle_class_initialization.rs tests/oracle/class_initialization \
    oracle/class_initialization oracle_class_
check_registry operators "Expression operators" tests/oracle_operator_semantics.rs \
    tests/oracle/operators oracle/operators oracle_
check_registry function_semantics "Function semantics" \
    tests/oracle_function_semantics.rs tests/oracle/function_semantics \
    oracle/function_semantics oracle_function
check_registry primitive_intrinsics "Primitive intrinsics" \
    tests/oracle_primitive_intrinsics.rs tests/oracle/primitive_intrinsics \
    oracle/primitive_intrinsics oracle_
check_registry number "Number semantics" tests/oracle_number_semantics.rs \
    tests/oracle/number oracle/number oracle_number_
check_registry eval "Eval semantics" tests/oracle_eval_semantics.rs \
    tests/oracle/eval oracle/eval oracle_eval_
check_registry async_functions "Async functions" tests/oracle_async_functions.rs \
    tests/oracle/async_functions oracle/async_functions oracle_async_

if $compiled; then
    check_compiled_harness
fi
