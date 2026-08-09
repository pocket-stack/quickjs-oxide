#!/usr/bin/env bash
# Ensure every nested Array oracle source is compiled by the aggregate target.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

die() {
    echo "error: $*" >&2
    exit 1
}

entry=tests/oracle_array_methods.rs
source_dir=tests/oracle/array
command -v rg >/dev/null 2>&1 || die 'rg is required'
[[ -f "$entry" && ! -L "$entry" ]] || die "missing aggregate entry: $entry"
[[ -d "$source_dir" && ! -L "$source_dir" ]] || die "missing oracle source directory: $source_dir"

actual=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-array-actual.XXXXXX")
declared=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-array-declared.XXXXXX")
trap 'rm -f -- "$actual" "$declared"' EXIT HUP INT TERM

rg --files "$source_dir" -g '*.rs' | sort >"$actual"
[[ -s "$actual" ]] || die "no Array oracle sources found under $source_dir"

awk '
    function fail(message) {
        print "error: " message > "/dev/stderr"
        failed=1
    }
    /^#\[path = "oracle\/array\/[^\"]+\.rs"\]$/ {
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
    /^#\[path = "oracle\/array\// {
        fail("malformed Array oracle path declaration: " $0)
    }
    /^mod oracle_array_/ {
        fail("Array oracle module lacks an adjacent path declaration: " $0)
    }
    END { exit failed }
' "$entry" | sort >"$declared"

duplicates=$(uniq -d "$declared")
[[ -z "$duplicates" ]] || die "duplicate Array oracle declarations: $duplicates"
if ! cmp -s "$actual" "$declared"; then
    diff -u "$actual" "$declared" >&2 || true
    die 'Array oracle registry does not match nested sources'
fi

count=$(wc -l <"$actual" | tr -d '[:space:]')
printf 'Array oracle registry covers %s modules.\n' "$count"
