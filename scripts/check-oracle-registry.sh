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
