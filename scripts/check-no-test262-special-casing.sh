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

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-anticheat.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

# A Rust `#[path]` can make a production module compile source outside the
# roots scanned below. Permit exactly one such escape: the runtime module's
# unit tests, guarded by `cfg(test)` and rooted under the repository test tree.
# Keeping the declaration exact makes deleting the cfg guard or adding another
# unscanned production input fail closed.
external_module_paths=$(rg --with-filename --no-heading --color never \
    --glob '*.rs' -- '^#\[path[[:space:]]*=[[:space:]]*"\.\./' src || true)
[[ "$external_module_paths" == \
    'src/runtime/module.rs:#[path = "../../tests/unit/runtime_module/tests.rs"]' ]] \
    || die 'production sources contain an unauthenticated external module path'
rg --quiet --multiline --pcre2 -- \
    '^#\[cfg\(test\)\]\n#\[path = "\.\./\.\./tests/unit/runtime_module/tests\.rs"\]\nmod tests;$' \
    src/runtime/module.rs \
    || die 'external runtime unit tests are not protected by the exact cfg(test) boundary'
[[ -f tests/unit/runtime_module/tests.rs \
    && ! -L tests/unit/runtime_module/tests.rs ]] \
    || die 'external runtime unit-test module must be a regular repository file'

scan_roots=(src web/wasm/src Cargo.toml web/wasm/Cargo.toml)
while IFS= read -r build_script; do
    scan_roots+=("$build_script")
done < <(find . -path './target' -prune -o -name build.rs -print | LC_ALL=C sort)
scan_globs=(
    --glob '*.rs'
    --glob '*.toml'
    --glob '!**/*tests.rs'
    --glob '!src/bin/run_test262.rs'
    --glob '!src/bin/run_test262/**'
)

path_pattern='\b(?:test/)?(?:built-ins|language|intl402|annexB|staging|harness)/[A-Za-z0-9_./@+-]+\.js\b|[A-Za-z0-9_.@+-]+_FIXTURE\.js\b'
source_hash_pattern='\b(?:source|source_text|script|program|code)(?:_[a-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))[a-z0-9_]*|\b[^;\n]{0,100}\.[a-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))[a-z0-9_]*)\b|\b[a-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))[a-z0-9_]*\s*\([^;\n]{0,100}\b(?:source|source_text|script|program|code)\b'
source_literal_pattern='(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,80}(?:==|!=)\s*(?:r\#*)?"[^"\n]+"\#*|(?:r\#*)?"[^"\n]+"\#*\s*(?:==|!=)[^;\n]{0,80}(?i:\b(?:source|source_text|script|program|code)\b)'
source_probe_pattern='(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,160}\.(?:contains|starts_with|ends_with|find|rfind|match_indices|strip_prefix|strip_suffix)\(\s*(?:(?:b|br|r)\#*)?"[^"\n]+"'
source_byte_window_pattern='(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,160}(?:\.as_bytes\(\))?\.windows\([^;\n]{1,40}\)[^;\n]{0,160}(?:b|br)\#*"[^"\n]+"'
source_length_pattern='(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,120}\.len\(\)\s*(?:==|!=|<=|>=|<|>)\s*[1-9][0-9_]+|[1-9][0-9_]+\s*(?:==|!=|<=|>=|<|>)\s*(?i:\b(?:source|source_text|script|program|code)\b)[^;\n]{0,120}\.len\(\)'
source_alias_pattern='(?i:\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=\s*(?:&\s*)?(?:source|source_text|script|program|code)(?:\.(?:to_owned|to_string|clone|as_str|as_ref|trim|trim_start|trim_end)\(\))*\s*;)[\s\S]{0,400}?\b\1(?:\.[a-z0-9_]+\([^)]*\))*\.(?:contains|starts_with|ends_with|find|rfind|match_indices|strip_prefix|strip_suffix)\(\s*(?:(?:b|br|r)\#*)?"[^"\n]+"'
source_byte_alias_pattern='(?i:\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=\s*(?:&\s*)?(?:source|source_text|script|program|code)(?:\.(?:to_owned|to_string|clone|as_str|as_ref|trim|trim_start|trim_end)\(\))*\.(?:as_bytes|into_bytes)\(\)\s*;)[\s\S]{0,400}?\b\1[^;\n]{0,160}\.windows\([^;\n]{1,40}\)[^;\n]{0,160}(?:b|br)\#*"[^"\n]+"'
source_alias_identity_pattern='(?i:\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=\s*(?:&\s*)?(?:source|source_text|script|program|code)(?:\.(?:to_owned|to_string|clone|as_str|as_ref|trim|trim_start|trim_end)\(\))*\s*;)[\s\S]{0,400}?\b\1[^;\n]{0,80}(?:(?:==|!=)\s*(?:r\#*)?"[^"\n]+"\#*|\.len\(\)\s*(?:==|!=|<=|>=|<|>)\s*[1-9][0-9_]+)'
filename_probe_pattern='(?i:\b(?:filename|file_name|path)\b)[^;\n]{0,160}(?:\.(?:contains|starts_with|ends_with|find|rfind|match_indices|strip_prefix|strip_suffix)\(\s*(?:r\#*)?"[^"\n]+|(?:==|!=)\s*(?:r\#*)?"[^"\n]+"\#*)|(?:r\#*)?"[^"\n]+"\#*\s*(?:==|!=)[^;\n]{0,160}(?i:\b(?:filename|file_name|path)\b)|(?i:\bmatch\s+(?:&\s*)?(?:filename|file_name|path)\b)[^\{\n]{0,80}\{[\s\S]{0,240}?(?:r\#*)?"[^"\n]+"\#*\s*=>'
filename_alias_pattern='(?i:\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=\s*(?:&\s*)?(?:filename|file_name|path)(?:\.(?:to_owned|to_string|clone|as_str|as_ref|trim|trim_start|trim_end)\(\))*\s*;)[\s\S]{0,400}?\b\1[^;\n]{0,140}(?:(?:==|!=)\s*(?:r\#*)?"[^"\n]+"\#*|\.(?:contains|starts_with|ends_with|find|rfind|match_indices|strip_prefix|strip_suffix)\(\s*(?:r\#*)?"[^"\n]+)'
embedded_fixture_pattern='include_(?:str|bytes)!\s*\([^;\n]{0,180}(?:r\#*)?"[^"\n]*(?:test262|fixture|(?:^|/)test/)[^"\n]*"'
source_literal_allow_pattern="^src/lexer\\.rs:[0-9]+:[[:space:]]*let limit = if source == \"'abc'\" \\{ 2 \\} else \\{ 1 \\};$"

source_probe_allowlist=$tmp/source-probe-allowlist.txt
printf '%s\n' \
    "src/lexer.rs:        if ch == '\\r' && self.source[self.offset..].starts_with(\"\\r\\n\") {" \
    "src/lexer.rs:            '/' if source.starts_with(\"//\") => {" \
    "src/lexer.rs:            '/' if source.starts_with(\"/*\") => {" \
    "src/lexer.rs:                let Some(end) = source[2..].find(\"*/\") else {" \
    "src/lexer.rs:        if source.starts_with(\"//\") {" \
    "src/lexer.rs:        } else if source.starts_with(\"/*\") {" \
    "src/lexer.rs:            let Some(end) = source[2..].find(\"*/\") else {" \
    "src/lexer.rs:    let Some(after) = source.strip_prefix(\"of\") else {" \
    "src/lexer.rs:    if !source.starts_with(b\"#!\") {" \
    "src/lexer.rs:        if source.starts_with(b\"//\") {" \
    "src/lexer.rs:        } else if source.starts_with(b\"/*\") {" \
    "src/lexer.rs:    if let Some(rest) = source.strip_prefix(b\"import\")" \
    "src/lexer.rs:    } else if let Some(rest) = source.strip_prefix(b\"export\")" \
    "src/lexer.rs:    } else if let Some(rest) = source.strip_prefix(b\".\") {" \
    "src/lexer.rs:    } else if let Some(rest) = source.strip_prefix(b\"(\") {" \
    > "$source_probe_allowlist"

filename_probe_allowlist=$tmp/filename-probe-allowlist.txt
printf '%s\n' \
    "src/main.rs:        if filename.ends_with(\".json\") || import_type_is(attributes, \"json\") {" \
    "src/main.rs:    filename.ends_with(\".mjs\") || quickjs_detect_module_bytes(source)" \
    > "$filename_probe_allowlist"

filter_exact_allowlist() {
    local allowlist=$1 occurrence normalized
    while IFS= read -r occurrence; do
        [[ -z $occurrence ]] && continue
        if [[ $occurrence =~ ^([^:]+):[0-9]+:(.*)$ ]]; then
            normalized=${BASH_REMATCH[1]}:${BASH_REMATCH[2]}
        else
            printf '%s\n' "$occurrence"
            continue
        fi
        rg --quiet --fixed-strings --line-regexp -- "$normalized" "$allowlist" \
            || printf '%s\n' "$occurrence"
    done
}

scan_regex() {
    local label=$1 pattern=$2 allow_pattern=${3:-} allowlist=${4:-} output status
    set +e
    output=$(rg --line-number --no-heading --color never --multiline --pcre2 \
        "${scan_globs[@]}" -- "$pattern" "${scan_roots[@]}" 2>&1)
    status=$?
    set -e
    case $status in
        0)
            if [[ -n $allow_pattern ]]; then
                output=$(printf '%s\n' "$output" | rg --invert-match --pcre2 -- "$allow_pattern" || true)
            fi
            if [[ -n $allowlist ]]; then
                output=$(filter_exact_allowlist "$allowlist" <<< "$output")
            fi
            [[ -z $output ]] && return
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
scan_regex "an exact authored-source comparison" "$source_literal_pattern" "$source_literal_allow_pattern"
scan_regex "an authored-source substring probe" "$source_probe_pattern" "" "$source_probe_allowlist"
scan_regex "an authored-source byte-window probe" "$source_byte_window_pattern"
scan_regex "an authored-source length identity" "$source_length_pattern"
scan_regex "an aliased authored-source substring probe" "$source_alias_pattern"
scan_regex "an aliased authored-source byte-window probe" "$source_byte_alias_pattern"
scan_regex "an aliased authored-source identity check" "$source_alias_identity_pattern"
scan_regex "a filename-specific branch" "$filename_probe_pattern" "" "$filename_probe_allowlist"
scan_regex "an aliased JavaScript filename-specific branch" "$filename_alias_pattern"
scan_regex "an embedded Test262 or fixture source" "$embedded_fixture_pattern"

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
! rg --quiet --multiline --pcre2 -- "$path_pattern|$source_hash_pattern|$source_literal_pattern|$source_probe_pattern|$source_byte_window_pattern|$source_length_pattern|$source_alias_pattern|$source_byte_alias_pattern|$source_alias_identity_pattern|$filename_probe_pattern|$filename_alias_pattern|$embedded_fixture_pattern" "$tmp/allowed.rs" \
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

printf 'if source.contains("short") {}\n' > "$tmp/source-short-probe.rs"
rg --quiet --pcre2 -- "$source_probe_pattern" "$tmp/source-short-probe.rs" \
    || die "short source-special-casing canary escaped the anti-cheat pattern"

printf 'if source.find("short").is_some() {}\n' > "$tmp/source-find.rs"
rg --quiet --pcre2 -- "$source_probe_pattern" "$tmp/source-find.rs" \
    || die "source-find canary escaped the anti-cheat pattern"

printf 'if source.as_bytes().windows(5).any(|window| window == b"short") {}\n' > "$tmp/source-window.rs"
rg --quiet --pcre2 -- "$source_byte_window_pattern" "$tmp/source-window.rs" \
    || die "source byte-window canary escaped the anti-cheat pattern"

printf 'if source.trim().contains("this exact fixture body") {}\n' > "$tmp/source-chain.rs"
rg --quiet --pcre2 -- "$source_probe_pattern" "$tmp/source-chain.rs" \
    || die "chained source-special-casing canary escaped the anti-cheat pattern"

printf 'if "this exact fixture body" == source {}\n' > "$tmp/source-reverse.rs"
rg --quiet --pcre2 -- "$source_literal_pattern" "$tmp/source-reverse.rs" \
    || die "reversed source-special-casing canary escaped the anti-cheat pattern"

printf 'if source.content_hash() == 0xdeadbeef {}\n' > "$tmp/source-hash.rs"
rg --quiet --pcre2 -- "$source_hash_pattern" "$tmp/source-hash.rs" \
    || die "source-hash canary escaped the anti-cheat pattern"

printf 'if source == "x" {}\n' > "$tmp/source-short.rs"
rg --quiet --pcre2 -- "$source_literal_pattern" "$tmp/source-short.rs" \
    || die "short source-equality canary escaped the anti-cheat pattern"

printf 'if source.len() == 417 {}\n' > "$tmp/source-length.rs"
rg --quiet --pcre2 -- "$source_length_pattern" "$tmp/source-length.rs" \
    || die "source-length canary escaped the anti-cheat pattern"

printf 'let probe = source; if probe.contains("this exact fixture body") {}\n' > "$tmp/source-alias.rs"
rg --quiet --multiline --pcre2 -- "$source_alias_pattern" "$tmp/source-alias.rs" \
    || die "source-alias canary escaped the anti-cheat pattern"

printf 'let text = source.to_owned(); if text.contains("new import(") {}\n' > "$tmp/source-owned-alias.rs"
rg --quiet --multiline --pcre2 -- "$source_alias_pattern" "$tmp/source-owned-alias.rs" \
    || die "owned source-alias canary escaped the anti-cheat pattern"

printf 'let bytes = source.as_bytes(); if bytes.windows(11).any(|window| window == b"new import(") {}\n' > "$tmp/source-byte-alias.rs"
rg --quiet --multiline --pcre2 -- "$source_byte_alias_pattern" "$tmp/source-byte-alias.rs" \
    || die "source byte-alias canary escaped the anti-cheat pattern"

printf 'let probe = source; if probe == "x" {}\n' > "$tmp/source-alias-identity.rs"
rg --quiet --multiline --pcre2 -- "$source_alias_identity_pattern" "$tmp/source-alias-identity.rs" \
    || die "source-alias identity canary escaped the anti-cheat pattern"

printf 'if filename == "fixture.js" {}\n' > "$tmp/filename.rs"
rg --quiet --multiline --pcre2 -- "$filename_probe_pattern" "$tmp/filename.rs" \
    || die "filename-equality canary escaped the anti-cheat pattern"

printf 'if filename.contains("case-stem") {}\n' > "$tmp/filename-short.rs"
rg --quiet --multiline --pcre2 -- "$filename_probe_pattern" "$tmp/filename-short.rs" \
    || die "short filename canary escaped the anti-cheat pattern"

printf 'match filename { "fixture.js" => true, _ => false }\n' > "$tmp/filename-match.rs"
rg --quiet --multiline --pcre2 -- "$filename_probe_pattern" "$tmp/filename-match.rs" \
    || die "filename-match canary escaped the anti-cheat pattern"

printf 'let fixture = filename; if fixture.ends_with("fixture.js") {}\n' > "$tmp/filename-alias.rs"
rg --quiet --multiline --pcre2 -- "$filename_alias_pattern" "$tmp/filename-alias.rs" \
    || die "filename-alias canary escaped the anti-cheat pattern"

printf 'let name = filename.to_owned(); if name.contains("no-new-call-expression") {}\n' > "$tmp/filename-owned-alias.rs"
rg --quiet --multiline --pcre2 -- "$filename_alias_pattern" "$tmp/filename-owned-alias.rs" \
    || die "owned filename-alias canary escaped the anti-cheat pattern"

allowed_source_occurrence="src/lexer.rs:1:        if source.starts_with(\"//\") {"
[[ -z $(filter_exact_allowlist "$source_probe_allowlist" <<< "$allowed_source_occurrence") ]] \
    || die "exact source-probe allowlist rejected its authenticated expression"
blocked_source_occurrence='src/lexer.rs:999:if source.find("import").is_some() { syntax_error(); }'
[[ $(filter_exact_allowlist "$source_probe_allowlist" <<< "$blocked_source_occurrence") == \
    "$blocked_source_occurrence" ]] \
    || die "source-probe allowlist admitted a different expression with an allowed literal"
blocked_filename_occurrence='src/main.rs:999:if filename.ends_with(".mjs") { syntax_error(); }'
[[ $(filter_exact_allowlist "$filename_probe_allowlist" <<< "$blocked_filename_occurrence") == \
    "$blocked_filename_occurrence" ]] \
    || die "filename-probe allowlist admitted a different expression with an allowed suffix"

printf 'const CASE: &str = include_str!("tests/test262/fixture.js");\n' > "$tmp/include.rs"
rg --quiet --multiline --pcre2 -- "$embedded_fixture_pattern" "$tmp/include.rs" \
    || die "embedded-fixture canary escaped the anti-cheat pattern"

echo "Production engine Test262 anti-cheat gate passed."
