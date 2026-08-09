#!/usr/bin/env bash
# Reject fixture-specific behavior in production engine sources.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
cd "$root"

die() {
    echo "error: $*" >&2
    exit 1
}

command -v rg >/dev/null 2>&1 || die "ripgrep is required"

scan_roots=(src web/wasm/src)
scan_globs=(
    --glob '*.rs'
    --glob '!**/tests.rs'
    --glob '!src/bin/run_test262.rs'
    --glob '!src/bin/run_test262/**'
)

path_pattern='\b(?:test/)?(?:built-ins|language|intl402|annexB|staging|harness)/[A-Za-z0-9_./@+-]+\.js\b|[A-Za-z0-9_.@+-]+_FIXTURE\.js\b'
source_hash_pattern='\b(?:source|source_text|script|program|code)(?:_[a-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))[a-z0-9_]*|\b[^;\n]{0,100}\.[a-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))[a-z0-9_]*)\b|\b[a-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))[a-z0-9_]*\s*\([^;\n]{0,100}\b(?:source|source_text|script|program|code)\b'
source_literal_pattern='(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,80}(?:==|!=)\s*(?:r\#*)?"[^"\n]{16,}"\#*|(?:r\#*)?"[^"\n]{16,}"\#*\s*(?:==|!=)[^;\n]{0,80}(?i:\b(?:source|source_text|script|program|code)\b)'
source_probe_pattern='(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,120}\.(?:contains|starts_with|ends_with)\(\s*(?:r\#*)?"[^"\n]{16,}"'
filename_probe_pattern='(?i:\b(?:filename|file_name|path)\b)[^;\n]{0,120}\.(?:contains|starts_with|ends_with)\(\s*(?:r\#*)?"[^"\n]+\.js'

scan_regex() {
    local label=$1 pattern=$2 output status
    set +e
    output=$(rg --line-number --no-heading --color never --pcre2 \
        "${scan_globs[@]}" -- "$pattern" "${scan_roots[@]}" 2>&1)
    status=$?
    set -e
    case $status in
        0)
            printf '%s\n' "$output" >&2
            die "production sources contain $label"
            ;;
        1) ;;
        *)
            printf '%s\n' "$output" >&2
            die "could not scan production sources for $label"
            ;;
    esac
}

scan_regex "a Test262 path or fixture name" "$path_pattern"
scan_regex "source-derived hash dispatch" "$source_hash_pattern"
scan_regex "an exact authored-source comparison" "$source_literal_pattern"
scan_regex "an authored-source substring probe" "$source_probe_pattern"
scan_regex "a JavaScript filename-specific branch" "$filename_probe_pattern"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-anticheat.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

# Production code has one authenticated data-provenance digest: the pinned
# QuickJS Unicode table source. Reject every other SHA-1/SHA-256-shaped literal
# instead of coupling this gate to the runner's current profile history.
hash_pattern='(?i:\b(?=[0-9a-f]{40}\b)(?=[0-9a-f]*[a-f])[0-9a-f]{40}\b|\b(?=[0-9a-f]{64}\b)(?=[0-9a-f]*[a-f])[0-9a-f]{64}\b)'
unicode_source_sha=cf782bc7a07549e976f606bd3cb8555858482b279574554dcb8d46412986006c
set +e
hash_output=$(rg --line-number --no-heading --color never --pcre2 \
    "${scan_globs[@]}" -- "$hash_pattern" "${scan_roots[@]}" 2>&1)
hash_status=$?
set -e
case $hash_status in
    0)
        unexpected_hashes=$tmp/unexpected-hashes.txt
        : > "$unexpected_hashes"
        while IFS= read -r occurrence; do
            case $occurrence in
                src/unicode_*"$unicode_source_sha"*) ;;
                *) printf '%s\n' "$occurrence" >> "$unexpected_hashes" ;;
            esac
        done <<< "$hash_output"
        if [[ -s "$unexpected_hashes" ]]; then
            cat "$unexpected_hashes" >&2
            die "production sources contain an unauthenticated test-shaped hash"
        fi
        ;;
    1) ;;
    *)
        printf '%s\n' "$hash_output" >&2
        die "could not scan production sources for test-shaped hashes"
        ;;
esac

# Keep the patterns honest. Legitimate host vocabulary must stay allowed,
# while each prohibited coupling class must have a positive canary.
printf 'const HOST_NAME: &str = "$262";\n' > "$tmp/allowed.rs"
! rg --quiet --pcre2 -- "$path_pattern|$source_hash_pattern|$source_literal_pattern|$source_probe_pattern|$filename_probe_pattern" "$tmp/allowed.rs" \
    || die 'anti-cheat patterns reject the legitimate $262 host name'

printf '// language/statements/fixture-special-case.js\n' > "$tmp/path.rs"
rg --quiet --pcre2 -- "$path_pattern" "$tmp/path.rs" \
    || die "Test262 path canary escaped the anti-cheat pattern"

printf 'const FIXTURE_DIGEST: &str = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";\n' > "$tmp/hash.rs"
rg --quiet --pcre2 -- "$hash_pattern" "$tmp/hash.rs" \
    || die "test-shaped hash canary escaped the anti-cheat pattern"

printf 'if source.contains("this exact fixture body") {}\n' > "$tmp/source.rs"
rg --quiet --pcre2 -- "$source_probe_pattern" "$tmp/source.rs" \
    || die "source-special-casing canary escaped the anti-cheat pattern"

printf 'if source.trim().contains("this exact fixture body") {}\n' > "$tmp/source-chain.rs"
rg --quiet --pcre2 -- "$source_probe_pattern" "$tmp/source-chain.rs" \
    || die "chained source-special-casing canary escaped the anti-cheat pattern"

printf 'if "this exact fixture body" == source {}\n' > "$tmp/source-reverse.rs"
rg --quiet --pcre2 -- "$source_literal_pattern" "$tmp/source-reverse.rs" \
    || die "reversed source-special-casing canary escaped the anti-cheat pattern"

printf 'if source.content_hash() == 0xdeadbeef {}\n' > "$tmp/source-hash.rs"
rg --quiet --pcre2 -- "$source_hash_pattern" "$tmp/source-hash.rs" \
    || die "source-hash canary escaped the anti-cheat pattern"

echo "Production engine Test262 anti-cheat gate passed."
