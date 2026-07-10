#!/usr/bin/env bash
# Prove that the product is a Rust implementation, not a wrapper around a
# native QuickJS engine. Test-only oracle code is deliberately outside the
# product file set: differential tests may execute an independently installed
# upstream qjs, but that engine must never enter a product build or runtime.

set -euo pipefail

if ! command -v rg >/dev/null 2>&1; then
    echo "error: check-rust-only.sh requires ripgrep (rg)" >&2
    exit 2
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "error: check-rust-only.sh requires cargo" >&2
    exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=${RUST_ONLY_ROOT:-"$script_dir/.."}
root=$(CDPATH= cd -- "$root" && pwd)

if [[ ! -f "$root/Cargo.toml" ]]; then
    echo "error: no Cargo.toml found at rust-only root: $root" >&2
    exit 2
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-rust-only.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

cd -- "$root"

# Keep the exclusions intentionally narrow. External qjs and native fixtures
# are permitted only below an explicitly named test/tests directory. Generated
# target trees, VCS metadata, and prose documentation are not product inputs.
rg --files --hidden \
    -g '!.git/**' -g '!**/.git/**' \
    -g '!target/**' -g '!**/target/**' \
    >"$tmp_dir/repository-files"
rg -v '(^|/)(tests?|docs?)(/|$)' "$tmp_dir/repository-files" \
    >"$tmp_dir/product-files"

violations=0

report_matches() {
    local heading=$1
    local matches=$2

    violations=$((violations + 1))
    printf '\n[%s]\n%s\n' "$heading" "$matches" >&2
}

check_matches() {
    local heading=$1
    local matches
    local status
    shift

    if matches=$(rg "$@"); then
        report_matches "$heading" "$matches"
        return
    else
        status=$?
    fi

    if (( status != 1 )); then
        echo "error: ripgrep failed while checking: $heading" >&2
        exit 2
    fi
}

# Product inputs may not contain a native implementation, prebuilt native
# engine, or hidden WebAssembly engine. Headers are included here because this
# check is about current product build inputs. A future generated C ABI facade
# can be allowed narrowly without weakening the external-engine checks below.
check_matches "native source or binary in a product path" -i \
    '\.(c|cc|cpp|cxx|h|hh|hpp|hxx|s|asm|a|o|obj|so|dylib|dll|lib|wasm)(\.in)?$' \
    "$tmp_dir/product-files"

# Archives named for QuickJS are native engine inputs even when their contents
# have not been unpacked into the repository yet.
check_matches "archived QuickJS engine in a product path" -i \
    '(^|/)[^/]*(quick[-_]?js|qjs)[^/]*\.(tar(\.(gz|xz|bz2|zst))?|tgz|txz|zip)$' \
    "$tmp_dir/product-files"

# Direct native tool dependencies in a manifest and native compiler/generator
# calls in build.rs are forbidden. This catches renamed Cargo dependencies via
# `package = ...` as well as the ordinary dependency spelling.
check_matches "native compiler/generator configured by Cargo" -n -i \
    '(^[[:space:]]*(cc|cmake|bindgen)(\.[A-Za-z0-9_-]+)?[[:space:]]*=|package[[:space:]]*=[[:space:]]*[\x22\x27](cc|cmake|bindgen)[\x22\x27]|\.(cc|cmake|bindgen)[[:space:]]*\]|(cc|cmake|bindgen)::[A-Za-z_][A-Za-z0-9_]*|Command::new[[:space:]]*\([[:space:]]*(r#*)?\x22(cc|c\+\+|gcc|clang|cmake|bindgen)\x22)' \
    . \
    -g 'Cargo.toml' -g 'build.rs' \
    -g '!target/**' -g '!**/target/**' \
    -g '!tests/**' -g '!**/tests/**' \
    -g '!test/**' -g '!**/test/**'

# Reject actionable link/load/embed instructions for an external QuickJS
# library. References to upstream algorithms in ordinary comments are fine.
check_matches "external QuickJS native link or embed instruction" -n -i \
    '(cargo:{1,2}rustc-link-(lib|search)[^\x22\x27]*quick[-_]?js|quick[-_]?js[^\x22\x27]*cargo:{1,2}rustc-link-(lib|search)|#[[:space:]]*\[[[:space:]]*link[[:space:]]*\([^]]*(quick[-_]?js|qjs)|(^|[[:space:]\x22\x27])-l[[:space:]]*(lib)?quick[-_]?js|/DEFAULTLIB:[^[:space:]\x22\x27]*quick[-_]?js|libquick[-_]?js\.(a|so|dylib|dll|lib)|include_(bytes|str)![[:space:]]*\([^)]*(quick[-_]?js|qjs)[^)]*\.(a|so|dylib|dll|lib|wasm))' \
    . \
    -g '*.rs' -g 'Cargo.toml' -g 'config' -g 'config.toml' \
    -g '!target/**' -g '!**/target/**' \
    -g '!tests/**' -g '!**/tests/**' \
    -g '!test/**' -g '!**/test/**'

# An extern block containing QuickJS API declarations imports a foreign engine.
# Do not ban `extern \"C\"` functions in general: the finished Rust rewrite must
# eventually export its own C ABI, and those exports are valid product code.
check_matches "foreign QuickJS C API import" -n -U \
    '(?s)(unsafe[[:space:]]+)?extern[[:space:]]+"C"[[:space:]]*\{[^}]*\b(JS|js)_[A-Za-z0-9_]+[[:space:]]*\(' \
    . \
    -g '*.rs' \
    -g '!target/**' -g '!**/target/**' \
    -g '!tests/**' -g '!**/tests/**' \
    -g '!test/**' -g '!**/test/**'

# Product code must not delegate parsing or execution to a qjs subprocess.
# Test oracle calls are excluded by the same explicit test-path boundary.
check_matches "external qjs execution in product code" -n -i \
    '(QJS_ORACLE|Command::new[[:space:]]*\([[:space:]]*(r#*)?\x22([^\x22]*/)?(qjs|quickjs)(\.exe)?\x22)' \
    . \
    -g '*.rs' \
    -g '!target/**' -g '!**/target/**' \
    -g '!tests/**' -g '!**/tests/**' \
    -g '!test/**' -g '!**/test/**'

# Inspect Cargo's resolved graph rather than trusting manifest spelling: Cargo
# aliases can hide the real package name. Depth zero entries are workspace
# products; every positive-depth entry is a dependency (including dev/build and
# target-specific dependencies because all features and targets are selected).
if ! CARGO_TERM_COLOR=never cargo tree \
    --manifest-path "$root/Cargo.toml" \
    --locked --workspace --all-features --target all \
    --prefix depth --format '{p}' \
    >"$tmp_dir/cargo-tree"; then
    echo "error: cargo tree could not resolve the locked dependency graph" >&2
    exit 2
fi

check_matches "QuickJS wrapper in the resolved Cargo dependency graph" -n -i \
    '^[1-9][0-9]*[^[:space:]]*(quick[-_]?js|rquickjs|qjs[-_]?sys)[^[:space:]]*[[:space:]]+v' \
    "$tmp_dir/cargo-tree"

if (( violations != 0 )); then
    printf '\nrust-only gate failed: %d violation categor%s found\n' \
        "$violations" "$([[ $violations -eq 1 ]] && printf 'y' || printf 'ies')" >&2
    exit 1
fi

echo "rust-only gate passed: product paths and resolved Cargo dependencies contain no external QuickJS engine"
