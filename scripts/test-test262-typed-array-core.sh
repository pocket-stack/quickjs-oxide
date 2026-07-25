#!/usr/bin/env bash
# Reproduce the checksum-bound TypedArray shared-core Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-typed-array-core-baseline.txt
manifest=tests/test262-typed-array-core.txt
profile=tests/test262-typed-array-core.conf
exclusions=tests/test262-typed-array-core-exclusions.tsv
report=target/test262-typed-array-core.tsv
json_report=target/test262-typed-array-core.jsonl
oracle_log=target/test262-typed-array-core-quickjs.log
candidate_oracle_log=target/test262-typed-array-core-candidate-quickjs.log
workers=${TEST262_WORKERS:-8}
check_only=false

expected_quickjs=2026-06-04
expected_test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
expected_patch=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expected_config=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expected_metadata=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expected_profile=663ac07f1fe379125eec29aec0c7b8b8215c08f40b93e9c39056ff40c6331036
expected_schema=test262-canonical-classified-v2
expected_mode=both
expected_timeout_ms=30000
expected_direct_candidate_paths=2316
expected_direct_candidate=64dfc295efac5414db8743def6099f484bb69090676378087382a23d5b3565a4
expected_spillover_paths=45
expected_spillover=4551e669756e077624fdfc7e01f2abb916b624455b3f16f3afb8e67556d92557
expected_candidate_paths=2361
expected_candidate=81b1e9fa4104cf51f16a0e3cca8e9600ba1e3390c41f0b8ebb0b9618c12b533f
expected_candidate_variants=4669
expected_candidate_keys=fd98267b85136c844a3c83a238b4194a1c1447b22c370f1344bae51e49517320
expected_mutation_candidate_paths=254
expected_mutation_candidate=040d1a0cc4c9068b230fd681a544a1c3b0351616363c4fa0a70ebf94b7c5e429
expected_mutation_candidate_variants=508
expected_mutation_candidate_keys=abdaa1350701a1604e30850d1ee5de87ef7afb806b539d090d9fbd75326bc051
expected_mutation_deferred_paths=3
expected_mutation_deferred=3edd4f483e4a5ca8ba020a95a41f1bfc29035a457d0cd091c2294b80bce8673f
expected_mutation_paths=251
expected_mutation_manifest=d85c80e335b4ba886501d9b126d444a2516995b356d4375f741e2d14313d3375
expected_mutation_variants=502
expected_mutation_keys=33a298d9b5901e318ba5662e6fddc8c4ed0bdbbe1284805d0d283d6e4478cbf2
expected_excluded_paths=1375
expected_exclusions=389eedb4125a4dbe2e30a797f60adbafc12279cca04387b09ee9035d00794421
expected_exclusions_file=fe441699f63debd30e3c5e2ed66d2c9b21732280afc03807be8a2268dbe56c3a
expected_paths=986
expected_variants=1949
expected_quickjs_variants=1949
expected_features=21
expected_features_hash=114b22411f94406423103ca7429cdc2009162c9ff55b41a06b3532e73536a2d0
expected_includes=11
expected_includes_hash=b1b60b5e1f7635615ff31eb139d1803608e5743c5f46ca53fadc3797e0abe012
expected_manifest=8542757a466917d9841cdc25317b78abad5db64aceda07ab78c8f38ced08bd3f
expected_keys=1b983b9b5c97314449c54ec0da387f393964a758db02836e6bd2b9aa0af39f7b
expected_test_typed_array_harness=4c0e237804f39a4aa670f72c05b4520730c03c2d2e9f2f41e6b380bd6749ec61
expected_sm_typed_array_harness=3798d277ac8f105b65ad26602b500b497af7f3361fd14a169c58a601c605bb2e
expected_sm_math_harness=79dea1172236685567e09da8c9e868e0f84686bf40cff728785223c5b43f5e7b

usage() {
    cat <<'EOF'
usage: scripts/test-test262-typed-array-core.sh [--check]

With --check, rebuild and audit the frozen TypedArray candidate, mutation
promotion, manifest, and exclusion ledger and verify all 4,669 candidate
variants plus the 1,949 admitted variants against pinned QuickJS. With no
option, also run the checksum-bound quickjs-oxide gate; that mode requires a
measured all-green baseline file.
EOF
}

case ${1:-} in
    "")
        ;;
    --check)
        check_only=true
        ;;
    -h|--help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
if [[ $# -gt 1 ]]; then
    usage >&2
    exit 2
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    else
        shasum -a 256 | awk '{ print $1 }'
    fi
}

read_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found++ }
        END { if (found != 1) exit 1 }
    ' "$baseline"); then
        echo "error: TypedArray core baseline is missing exactly one $key entry: $baseline" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: TypedArray core baseline contains an empty $key entry: $baseline" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    if [[ "$actual" != "$expected" ]]; then
        echo "error: TypedArray core baseline $key drifted" >&2
        echo "expected: $expected" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

exclusion_paths() {
    awk -F'\t' 'NF && $1 !~ /^#/ { print $1 }' "$exclusions"
}

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

metadata_block() {
    local test_path=$1
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path"
}

metadata_list() {
    local test_path=$1 key=$2
    metadata_block "$test_path" | awk -v key="$key" '
        $0 ~ ("^" key ":[[:space:]]*\\[") {
            line=$0
            sub("^[^:]+:[[:space:]]*\\[", "", line)
            while (line !~ /\][[:space:]]*$/ && getline next_line) {
                line=line " " next_line
            }
            sub(/\][[:space:]]*$/, "", line)
            count=split(line, values, /,[[:space:]]*/)
            for (i=1; i <= count; i++) {
                gsub(/^[[:space:]]+|[[:space:]]+$/, "", values[i])
                if (values[i] != "") print values[i]
            }
            exit
        }
        $0 == key ":" { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]*/ {
            line=$0
            sub(/^[[:space:]]*-[[:space:]]*/, "", line)
            if (line != "") print line
            next
        }
        inside { exit }
    '
}

source_body() {
    local test_path=$1
    awk '
        /^\/\*---$/ { in_metadata=1; next }
        in_metadata && /^---\*\/$/ { in_metadata=0; next }
        !in_metadata { print }
    ' "$suite/$test_path"
}

concrete_typed_array_tokens() {
    local source_file=$1 constructor
    for constructor in \
        Uint8ClampedArray Int8Array Uint8Array Int16Array Uint16Array \
        Int32Array Uint32Array BigInt64Array BigUint64Array Float16Array \
        Float32Array Float64Array
    do
        if grep -Eq \
            "(^|[^[:alnum:]_$])${constructor}([^[:alnum:]_$]|$)" \
            "$source_file"; then
            printf '%s\n' "$constructor"
        fi
    done
}

spillover_paths() {
    cat <<'EOF'
test/built-ins/Array/prototype/concat/Array.prototype.concat_large-typed-array.js
test/built-ins/Array/prototype/concat/Array.prototype.concat_small-typed-array.js
test/built-ins/Object/seal/seal-bigint64array.js
test/built-ins/Object/seal/seal-biguint64array.js
test/built-ins/Object/seal/seal-float32array.js
test/built-ins/Object/seal/seal-float64array.js
test/built-ins/Object/seal/seal-int16array.js
test/built-ins/Object/seal/seal-int32array.js
test/built-ins/Object/seal/seal-int8array.js
test/built-ins/Object/seal/seal-uint16array.js
test/built-ins/Object/seal/seal-uint32array.js
test/built-ins/Object/seal/seal-uint8array.js
test/built-ins/Object/seal/seal-uint8clampedarray.js
test/language/statements/class/subclass/builtins.js
test/staging/sm/Array/fill.js
test/staging/sm/Array/from_errors.js
test/staging/sm/ArrayBuffer/CloneArrayBuffer.js
test/staging/sm/Math/acosh-approx.js
test/staging/sm/Math/acosh-exact.js
test/staging/sm/Math/asinh-approx.js
test/staging/sm/Math/atanh-approx.js
test/staging/sm/Math/atanh-exact.js
test/staging/sm/Math/cbrt-approx.js
test/staging/sm/Math/cosh-approx.js
test/staging/sm/Math/expm1-approx.js
test/staging/sm/Math/fround.js
test/staging/sm/Math/log10-approx.js
test/staging/sm/Math/log1p-approx.js
test/staging/sm/Math/log1p-exact.js
test/staging/sm/Math/log2-approx.js
test/staging/sm/Math/sinh-approx.js
test/staging/sm/Math/tanh-approx.js
test/staging/sm/Math/trunc.js
test/staging/sm/Proxy/revoked-get-function-realm-typeerror.js
test/staging/sm/Reflect/get.js
test/staging/sm/Reflect/isExtensible.js
test/staging/sm/Reflect/preventExtensions.js
test/staging/sm/Symbol/species.js
test/staging/sm/Symbol/toStringTag.js
test/staging/sm/Symbol/typed-arrays.js
test/staging/sm/extensions/element-setting-ToNumber-detaches.js
test/staging/sm/extensions/reviver-mutates-holder-array-nonnative.js
test/staging/sm/extensions/reviver-mutates-holder-object-nonnative.js
test/staging/sm/object/values-entries-typedarray.js
test/staging/sm/regress/regress-571014.js
EOF
}

is_direct_core_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/ArrayBuffer/isView/*|\
        test/built-ins/ArrayBuffer/prototype/*)
            return 0
            ;;
        test/built-ins/TypedArrayConstructors/BigInt64Array/*|\
        test/built-ins/TypedArrayConstructors/BigUint64Array/*|\
        test/built-ins/TypedArrayConstructors/Float32Array/*|\
        test/built-ins/TypedArrayConstructors/Float64Array/*|\
        test/built-ins/TypedArrayConstructors/Int16Array/*|\
        test/built-ins/TypedArrayConstructors/Int32Array/*|\
        test/built-ins/TypedArrayConstructors/Int8Array/*|\
        test/built-ins/TypedArrayConstructors/Uint16Array/*|\
        test/built-ins/TypedArrayConstructors/Uint32Array/*|\
        test/built-ins/TypedArrayConstructors/Uint8Array/*|\
        test/built-ins/TypedArrayConstructors/Uint8ClampedArray/*|\
        test/built-ins/TypedArrayConstructors/ctors/*|\
        test/built-ins/TypedArrayConstructors/ctors-bigint/*|\
        test/built-ins/TypedArrayConstructors/internals/*|\
        test/built-ins/TypedArrayConstructors/prototype/Symbol.iterator.js|\
        test/built-ins/TypedArrayConstructors/prototype/bigint-Symbol.iterator.js|\
        test/built-ins/TypedArrayConstructors/prototype/Symbol.toStringTag/*|\
        test/built-ins/TypedArrayConstructors/prototype/buffer/*|\
        test/built-ins/TypedArrayConstructors/prototype/byteLength/*|\
        test/built-ins/TypedArrayConstructors/prototype/byteOffset/*|\
        test/built-ins/TypedArrayConstructors/prototype/length/*|\
        test/built-ins/TypedArrayConstructors/prototype/values/*)
            return 0
            ;;
        test/built-ins/TypedArray/Symbol.species/*|\
        test/built-ins/TypedArray/invoked.js|\
        test/built-ins/TypedArray/length.js|\
        test/built-ins/TypedArray/name.js|\
        test/built-ins/TypedArray/prototype.js|\
        test/built-ins/TypedArray/out-of-bounds-behaves-like-detached.js|\
        test/built-ins/TypedArray/out-of-bounds-get-and-set.js|\
        test/built-ins/TypedArray/out-of-bounds-has.js|\
        test/built-ins/TypedArray/resizable-buffer-length-tracking-1.js|\
        test/built-ins/TypedArray/resizable-buffer-length-tracking-2.js|\
        test/built-ins/TypedArray/prototype/Symbol.iterator.js|\
        test/built-ins/TypedArray/prototype/constructor.js|\
        test/built-ins/TypedArray/prototype/resizable-and-fixed-have-same-prototype.js|\
        test/built-ins/TypedArray/prototype/Symbol.iterator/*|\
        test/built-ins/TypedArray/prototype/Symbol.toStringTag/*|\
        test/built-ins/TypedArray/prototype/buffer/*|\
        test/built-ins/TypedArray/prototype/byteLength/*|\
        test/built-ins/TypedArray/prototype/byteOffset/*|\
        test/built-ins/TypedArray/prototype/length/*|\
        test/built-ins/TypedArray/prototype/values/*)
            return 0
            ;;
        test/staging/sm/TypedArray/Tconstructor-fromTypedArray-byteLength.js|\
        test/staging/sm/TypedArray/bug1526838.js|\
        test/staging/sm/TypedArray/constructor-*.js|\
        test/staging/sm/TypedArray/constructor_bad-args.js|\
        test/staging/sm/TypedArray/element-setting-converts-using-ToNumber.js|\
        test/staging/sm/TypedArray/getter-name.js|\
        test/staging/sm/TypedArray/has-property-op.js|\
        test/staging/sm/TypedArray/iterator-next-with-detached.js|\
        test/staging/sm/TypedArray/iterator.js|\
        test/staging/sm/TypedArray/object-defineproperty.js|\
        test/staging/sm/TypedArray/seal-and-freeze.js|\
        test/staging/sm/TypedArray/set-with-receiver.js|\
        test/staging/sm/TypedArray/test-integrity-level-detached.js|\
        test/staging/sm/TypedArray/test-integrity-level.js|\
        test/staging/sm/TypedArray/toStringTag-cross-compartment.js|\
        test/staging/sm/TypedArray/uint8clamped-constructor.js|\
        test/staging/sm/TypedArray/values.js|\
        test/staging/sm/TypedArray/write-out-of-bounds-tonumber.js)
            return 0
            ;;
    esac
    return 1
}

prototype_method_reason() {
    local method=$1
    case "$method" in
        entries|keys)
            printf 'method:iterator-entries-keys\n'
            ;;
        copyWithin|fill|reverse|set)
            printf 'method:mutation-copy-set\n'
            ;;
        at|every|some|find|findIndex|findLast|findLastIndex|includes|indexOf|lastIndexOf)
            printf 'method:search-predicate\n'
            ;;
        filter|map|slice|subarray|toReversed|toSorted|with)
            printf 'method:species-copy-transform\n'
            ;;
        forEach|reduce|reduceRight)
            printf 'method:callback-reduce\n'
            ;;
        join|toLocaleString|toString)
            printf 'method:stringification\n'
            ;;
        sort)
            printf 'method:sort\n'
            ;;
        *)
            return 1
            ;;
    esac
}

followup_reason() {
    local test_path=$1 relative method file
    case "$test_path" in
        test/built-ins/TypedArray/from/*|test/built-ins/TypedArrayConstructors/from/*)
            printf 'static:from\n'
            return
            ;;
        test/built-ins/TypedArray/of/*|test/built-ins/TypedArrayConstructors/of/*)
            printf 'static:of\n'
            return
            ;;
        test/built-ins/TypedArray/prototype/*)
            relative=${test_path#test/built-ins/TypedArray/prototype/}
            method=${relative%%/*}
            method=${method%.js}
            prototype_method_reason "$method"
            return
            ;;
        test/built-ins/TypedArrayConstructors/prototype/*)
            relative=${test_path#test/built-ins/TypedArrayConstructors/prototype/}
            method=${relative%%/*}
            method=${method%.js}
            prototype_method_reason "$method"
            return
            ;;
        test/staging/sm/TypedArray/*)
            file=${test_path##*/}
            file=${file%.js}
            case "$file" in
                from_*)
                    printf 'static:from\n'
                    ;;
                of)
                    printf 'static:of\n'
                    ;;
                detached-array-buffer-checks|prototype-constructor-identity)
                    printf 'method:full-prototype-contract\n'
                    ;;
                entries|keys)
                    printf 'method:iterator-entries-keys\n'
                    ;;
                at|every-*|find*|includes|indexOf*|lastIndexOf*)
                    printf 'method:search-predicate\n'
                    ;;
                fill*|reverse|set|set-*|set_*)
                    printf 'method:mutation-copy-set\n'
                    ;;
                filter*|map*|slice*|subarray*|toReversed*|toSorted*|with*)
                    printf 'method:species-copy-transform\n'
                    ;;
                forEach|reduce*)
                    printf 'method:callback-reduce\n'
                    ;;
                join|toLocaleString*|toString)
                    printf 'method:stringification\n'
                    ;;
                sort*|sorting_buffer_access)
                    printf 'method:sort\n'
                    ;;
                *)
                    return 1
                    ;;
            esac
            return
            ;;
    esac
    return 1
}

mutation_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/built-ins/TypedArray/prototype/set/BigInt/array-arg-set-values-in-order.js|\
        test/built-ins/TypedArray/prototype/set/array-arg-set-values-in-order.js)
            if ! grep -Fq 'sample.join()' "$source_file"; then
                echo "error: TypedArray mutation join dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'dependency:join\n'
            ;;
        test/staging/sm/TypedArray/set.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file"; then
                echo "error: TypedArray mutation WeakMap harness dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

direct_core_dependency_reason() {
    local test_path=$1 includes_file=$2
    case "$test_path" in
        test/built-ins/TypedArrayConstructors/internals/HasProperty/BigInt/inherited-property.js|\
        test/built-ins/TypedArrayConstructors/internals/HasProperty/inherited-property.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/BigInt/integer-indexes-and-string-and-symbol-keys-.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/BigInt/integer-indexes-and-string-keys.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/BigInt/integer-indexes.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/integer-indexes-and-string-and-symbol-keys-.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/integer-indexes-and-string-keys.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/integer-indexes.js)
            printf 'method:subarray\n'
            ;;
        test/staging/sm/TypedArray/*)
            if grep -Fxq sm/non262-TypedArray-shell.js "$includes_file"; then
                printf 'external:SharedArrayBuffer\n'
            else
                return 1
            fi
            ;;
        *)
            return 1
            ;;
    esac
}

spillover_dependency_reason() {
    local test_path=$1
    case "$test_path" in
        test/staging/sm/Math/atanh-approx.js)
            printf 'external:Math\n'
            ;;
        test/staging/sm/Proxy/revoked-get-function-realm-typeerror.js|\
        test/staging/sm/Symbol/toStringTag.js)
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

append_variant_keys() {
    local test_path=$1 flags_file=$2 output=$3
    local flag_count
    flag_count=$(wc -l <"$flags_file" | tr -d '[:space:]')
    case "$flag_count:$(tr '\n' ',' <"$flags_file")" in
        0:)
            printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path" >>"$output"
            ;;
        1:noStrict,)
            printf '%s\tsloppy\n' "$test_path" >>"$output"
            ;;
        1:onlyStrict,)
            printf '%s\tstrict\n' "$test_path" >>"$output"
            ;;
        *)
            echo "error: TypedArray candidate gained unsupported variant flags: $test_path" >&2
            sed 's/^/  /' "$flags_file" >&2
            exit 1
            ;;
    esac
}

verify_quickjs_oracle() {
    local label=$1 inventory=$2 expected_count=$3 log=$4
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done <"$inventory"

    if ! (
        cd -- "$source_dir"
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}"
    ) >"$log" 2>&1; then
        tail -n 100 "$log" >&2
        echo "error: pinned QuickJS could not execute the $label" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$log" \
        || ! grep -Fq "Average memory statistics for $expected_count tests:" "$log"; then
        tail -n 100 "$log" >&2
        echo "error: pinned QuickJS no longer passes all $label variants" >&2
        exit 1
    fi
}

verify_oxide_constructor_surface() {
    local probe output
    probe='(function () {
      var rows = [
        [Uint8ClampedArray, 1, false], [Int8Array, 1, false],
        [Uint8Array, 1, false], [Int16Array, 2, false],
        [Uint16Array, 2, false], [Int32Array, 4, false],
        [Uint32Array, 4, false], [BigInt64Array, 8, true],
        [BigUint64Array, 8, true], [Float16Array, 2, false],
        [Float32Array, 4, false], [Float64Array, 8, false]
      ];
      var TypedArray = Object.getPrototypeOf(Uint8Array);
      if (rows.length !== 12) throw new Error("constructor inventory");
      for (var i = 0; i < rows.length; i++) {
        var C = rows[i][0], size = rows[i][1], isBigInt = rows[i][2];
        if (typeof C !== "function" || C.BYTES_PER_ELEMENT !== size ||
            C.prototype.BYTES_PER_ELEMENT !== size ||
            Object.getPrototypeOf(C) !== TypedArray ||
            Object.getPrototypeOf(C.prototype) !== TypedArray.prototype) {
          throw new Error("constructor shape: " + C.name);
        }
        var view = new C(2);
        view[0] = isBigInt ? 1n : 1.5;
        if (view.length !== 2 || view.byteLength !== 2 * size ||
            view.buffer.byteLength !== 2 * size || !ArrayBuffer.isView(view)) {
          throw new Error("constructor storage: " + C.name);
        }
      }
      return 42;
    })()'
    if ! output=$(cargo run --locked --release --quiet --bin qjs -- \
        --print-result -e "$probe"); then
        echo "error: quickjs-oxide failed the twelve-constructor probe" >&2
        exit 1
    fi
    if [[ "$output" != "42" ]]; then
        echo "error: quickjs-oxide twelve-constructor probe returned: $output" >&2
        exit 1
    fi
}

cd -- "$root"

for required in "$manifest" "$profile" "$exclusions"; do
    if [[ ! -f "$required" ]]; then
        echo "error: TypedArray core gate input is missing: $required" >&2
        exit 1
    fi
done
if [[ "$check_only" == false && ! -f "$baseline" ]]; then
    echo "error: measured TypedArray core baseline is intentionally absent: $baseline" >&2
    echo "error: run --check now; add the baseline only after an all-green Oxide run" >&2
    exit 1
fi
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer, found: $workers" >&2
    exit 2
fi

if [[ "$check_only" == false ]]; then
    expect_value quickjs "$expected_quickjs"
    expect_value test262 "$expected_test262"
    expect_value test262_patch_sha256 "$expected_patch"
    expect_value test262_config_sha256 "$expected_config"
    expect_value test262_metadata_sha256 "$expected_metadata"
    expect_value oxide_profile_sha256 "$expected_profile"
    expect_value schema "$expected_schema"
    expect_value mode "$expected_mode"
    expect_value timeout_ms "$expected_timeout_ms"
    expect_value direct_candidate_paths "$expected_direct_candidate_paths"
    expect_value direct_candidate_sha256 "$expected_direct_candidate"
    expect_value spillover_paths "$expected_spillover_paths"
    expect_value spillover_sha256 "$expected_spillover"
    expect_value candidate_paths "$expected_candidate_paths"
    expect_value candidate_sha256 "$expected_candidate"
    expect_value candidate_variants "$expected_candidate_variants"
    expect_value candidate_keys_sha256 "$expected_candidate_keys"
    expect_value mutation_candidate_paths "$expected_mutation_candidate_paths"
    expect_value mutation_candidate_sha256 "$expected_mutation_candidate"
    expect_value mutation_candidate_variants "$expected_mutation_candidate_variants"
    expect_value mutation_candidate_keys_sha256 "$expected_mutation_candidate_keys"
    expect_value mutation_deferred_paths "$expected_mutation_deferred_paths"
    expect_value mutation_deferred_sha256 "$expected_mutation_deferred"
    expect_value mutation_paths "$expected_mutation_paths"
    expect_value mutation_manifest_sha256 "$expected_mutation_manifest"
    expect_value mutation_variants "$expected_mutation_variants"
    expect_value mutation_keys_sha256 "$expected_mutation_keys"
    expect_value excluded_paths "$expected_excluded_paths"
    expect_value exclusions_sha256 "$expected_exclusions"
    expect_value exclusions_file_sha256 "$expected_exclusions_file"
    expect_value paths "$expected_paths"
    expect_value variants "$expected_variants"
    expect_value quickjs_variants "$expected_quickjs_variants"
    expect_value features "$expected_features"
    expect_value features_sha256 "$expected_features_hash"
    expect_value includes "$expected_includes"
    expect_value includes_sha256 "$expected_includes_hash"
    expect_value manifest_sha256 "$expected_manifest"
    expect_value manifest_file_sha256 "$expected_manifest"
    expect_value keys_sha256 "$expected_keys"
    expect_value runnable "$expected_variants"

    pending_keys=$(awk -F= '$2 == "PENDING" { print $1 }' "$baseline")
    if [[ -n "$pending_keys" ]]; then
        echo "error: TypedArray core baseline still contains PENDING measured values" >&2
        printf '%s\n' "$pending_keys" | sed 's/^/  /' >&2
        exit 1
    fi
fi

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ "$(basename -- "$source_dir")" != "quickjs-$expected_quickjs" \
    || "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" != "$expected_test262" \
    || "$(sha256_file "$source_dir/tests/test262.patch")" != "$expected_patch" \
    || "$(sha256_file "$source_dir/test262.conf")" != "$expected_config" ]]; then
    echo "error: prepared QuickJS/Test262 inputs drifted from the TypedArray core gate" >&2
    exit 1
fi
if [[ "$(sha256_file "$suite/harness/testTypedArray.js")" \
        != "$expected_test_typed_array_harness" \
    || "$(sha256_file "$suite/harness/sm/non262-TypedArray-shell.js")" \
        != "$expected_sm_typed_array_harness" \
    || "$(sha256_file "$suite/harness/sm/non262-Math-shell.js")" \
        != "$expected_sm_math_harness" ]]; then
    echo "error: a TypedArray-dependent pinned harness drifted" >&2
    exit 1
fi
if ! grep -Fq 'floatArrayConstructors.push(Float16Array);' \
    "$suite/harness/testTypedArray.js"; then
    echo "error: pinned testTypedArray.js no longer dynamically covers Float16Array" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" \
    || "$(sha256_file "$manifest")" != "$expected_manifest" \
    || "$(sha256_file "$exclusions")" != "$expected_exclusions_file" ]]; then
    echo "error: committed TypedArray core gate assets drifted" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-typed-array-core.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
direct_base=$tmp_dir/direct-base.txt
array_buffer_inventory=$tmp_dir/array-buffer.txt
array_buffer_interop=$tmp_dir/array-buffer-interop.txt
direct_candidate=$tmp_dir/direct-candidate.txt
spillover_inventory=$tmp_dir/spillover.txt
candidate_inventory=$tmp_dir/candidate.txt
manifest_inventory=$tmp_dir/manifest.txt
excluded_inventory=$tmp_dir/excluded.txt
combined_inventory=$tmp_dir/combined.txt
derived_manifest=$tmp_dir/derived-manifest.txt
derived_exclusions=$tmp_dir/derived-exclusions.tsv
derived_exclusion_rows=$tmp_dir/derived-exclusion-rows.tsv
mutation_candidate=$tmp_dir/mutation-candidate.txt
mutation_candidate_keys=$tmp_dir/mutation-candidate-keys.txt
mutation_deferred=$tmp_dir/mutation-deferred.txt
mutation_manifest=$tmp_dir/mutation-manifest.txt
mutation_keys=$tmp_dir/mutation-keys.txt
candidate_features=$tmp_dir/candidate-features.txt
candidate_includes=$tmp_dir/candidate-includes.txt
candidate_flags=$tmp_dir/candidate-flags.txt
source_file=$tmp_dir/source-body.js
typed_array_tokens=$tmp_dir/typed-array-tokens.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
candidate_keys=$tmp_dir/candidate-keys.txt
variant_keys=$tmp_dir/variant-keys.txt

manifest_paths >"$manifest_inventory"
exclusion_paths >"$excluded_inventory"
spillover_paths >"$spillover_inventory"
LC_ALL=C sort -c "$manifest_inventory"
LC_ALL=C sort -c "$excluded_inventory"
LC_ALL=C sort -c "$spillover_inventory"

(
    cd -- "$suite"
    find \
        test/built-ins/TypedArrayConstructors \
        test/built-ins/TypedArray \
        test/built-ins/ArrayBuffer/isView \
        test/staging/sm/TypedArray \
        test/annexB/built-ins/TypedArrayConstructors \
        -type f -name '*.js' ! -name '*_FIXTURE.js' -print
) | LC_ALL=C sort >"$direct_base"
(
    cd -- "$suite"
    find test/built-ins/ArrayBuffer \
        -type f -name '*.js' ! -name '*_FIXTURE.js' -print
) | LC_ALL=C sort >"$array_buffer_inventory"
: >"$array_buffer_interop"
while IFS= read -r test_path; do
    [[ "$test_path" == test/built-ins/ArrayBuffer/isView/* ]] && continue
    source_body "$test_path" >"$source_file"
    concrete_typed_array_tokens "$source_file" >"$typed_array_tokens"
    if [[ -s "$typed_array_tokens" ]]; then
        printf '%s\n' "$test_path" >>"$array_buffer_interop"
    fi
done <"$array_buffer_inventory"
LC_ALL=C sort -u "$direct_base" "$array_buffer_interop" >"$direct_candidate"
LC_ALL=C sort -u "$direct_candidate" "$spillover_inventory" >"$candidate_inventory"

direct_candidate_count="$(wc -l <"$direct_candidate" | tr -d '[:space:]')"
spillover_count="$(wc -l <"$spillover_inventory" | tr -d '[:space:]')"
candidate_count="$(wc -l <"$candidate_inventory" | tr -d '[:space:]')"
if [[ "$direct_candidate_count" != "$expected_direct_candidate_paths" ||
    "$(sha256_file "$direct_candidate")" != "$expected_direct_candidate" ||
    "$spillover_count" != "$expected_spillover_paths" ||
    "$(sha256_file "$spillover_inventory")" != "$expected_spillover" ||
    "$candidate_count" != "$expected_candidate_paths" ||
    "$(sha256_file "$candidate_inventory")" != "$expected_candidate" ]]; then
    echo "error: TypedArray candidate inventory drifted" >&2
    exit 1
fi
if [[ -n "$(LC_ALL=C comm -12 "$direct_candidate" "$spillover_inventory")" ]]; then
    echo "error: TypedArray latent spillover overlaps the direct candidate" >&2
    exit 1
fi

if ! awk -F'\t' '
    NR == 1 {
        if ($1 != "# path" || $2 != "reason" || NF != 2) exit 1
        next
    }
    {
        if (NF != 2 || $1 == "") exit 1
        counts[$2]++
    }
    END {
        if (NR != 1376 ||
            counts["dependency:join"] != 2 ||
            counts["external:cross-realm"] != 54 ||
            counts["external:SharedArrayBuffer"] != 71 ||
            counts["external:WeakMap"] != 3 ||
            counts["external:Math"] != 1 ||
            counts["external:IsHTMLDDA"] != 1 ||
            counts["static:from"] != 88 ||
            counts["static:of"] != 34 ||
            counts["method:iterator-entries-keys"] != 42 ||
            counts["method:mutation-copy-set"] != 0 ||
            counts["method:search-predicate"] != 402 ||
            counts["method:species-copy-transform"] != 388 ||
            counts["method:callback-reduce"] != 148 ||
            counts["method:sort"] != 47 ||
            counts["method:stringification"] != 84 ||
            counts["method:subarray"] != 8 ||
            counts["method:full-prototype-contract"] != 2) {
            exit 1
        }
    }
' "$exclusions"; then
    echo "error: TypedArray exclusion ledger reason inventory drifted" >&2
    exit 1
fi
if [[ "$(wc -l <"$manifest_inventory" | tr -d '[:space:]')" != "$expected_paths" \
    || "$(LC_ALL=C sort -u "$manifest_inventory" | wc -l | tr -d '[:space:]')" \
        != "$expected_paths" \
    || "$(sha256_file "$manifest_inventory")" != "$expected_manifest" \
    || "$(wc -l <"$excluded_inventory" | tr -d '[:space:]')" \
        != "$expected_excluded_paths" \
    || "$(LC_ALL=C sort -u "$excluded_inventory" | wc -l | tr -d '[:space:]')" \
        != "$expected_excluded_paths" \
    || "$(sha256_file "$excluded_inventory")" != "$expected_exclusions" ]]; then
    echo "error: TypedArray manifest or exclusion path inventory drifted" >&2
    exit 1
fi
if [[ -n "$(LC_ALL=C comm -12 "$manifest_inventory" "$excluded_inventory")" ]]; then
    echo "error: TypedArray manifest overlaps its exclusion ledger" >&2
    exit 1
fi
LC_ALL=C sort -u "$manifest_inventory" "$excluded_inventory" >"$combined_inventory"
diff -u "$candidate_inventory" "$combined_inventory"

: >"$derived_exclusion_rows"
: >"$derived_manifest"
: >"$mutation_candidate"
: >"$mutation_deferred"
: >"$mutation_manifest"
: >"$candidate_keys"
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: missing TypedArray candidate path: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    if [[ -z "$metadata" \
        || "$(grep -c '^/\*---$' "$suite/$test_path" || true)" != "1" \
        || "$(grep -c '^---\*/$' "$suite/$test_path" || true)" != "1" ]]; then
        echo "error: TypedArray candidate lost a unique metadata block: $test_path" >&2
        exit 1
    fi
    if grep -q '^negative:' <<<"$metadata"; then
        echo "error: TypedArray all-green candidate gained a negative test: $test_path" >&2
        exit 1
    fi
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$candidate_keys"
done <"$candidate_inventory"
LC_ALL=C sort -o "$candidate_keys" "$candidate_keys"
if [[ "$(wc -l <"$candidate_keys" | tr -d '[:space:]')" \
        != "$expected_candidate_variants" \
    || "$(sha256_file "$candidate_keys")" != "$expected_candidate_keys" ]]; then
    echo "error: TypedArray candidate path/variant key stream drifted" >&2
    exit 1
fi

while IFS= read -r test_path; do
    metadata_list "$test_path" features >"$candidate_features"
    metadata_list "$test_path" includes >"$candidate_includes"
    source_body "$test_path" >"$source_file"
    reason=
    if grep -Fxq cross-realm "$candidate_features" \
        || grep -Fq '$262.createRealm' "$source_file"; then
        reason=external:cross-realm
    elif grep -Fxq SharedArrayBuffer "$candidate_features"; then
        reason=external:SharedArrayBuffer
    elif [[ "$test_path" == test/annexB/built-ins/TypedArrayConstructors/* ]]; then
        if ! grep -Fxq IsHTMLDDA "$candidate_features"; then
            echo "error: Annex B TypedArray exclusion lost IsHTMLDDA: $test_path" >&2
            exit 1
        fi
        reason=external:IsHTMLDDA
    elif is_direct_core_path "$test_path" \
        && reason=$(direct_core_dependency_reason \
            "$test_path" "$candidate_includes"); then
        :
    elif is_direct_core_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        continue
    elif ! reason=$(followup_reason "$test_path"); then
        echo "error: unclassified TypedArray follow-up path: $test_path" >&2
        exit 1
    elif [[ "$reason" == "method:mutation-copy-set" ]]; then
        printf '%s\n' "$test_path" >>"$mutation_candidate"
        if reason=$(mutation_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$mutation_deferred"
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
            printf '%s\n' "$test_path" >>"$derived_manifest"
            printf '%s\n' "$test_path" >>"$mutation_manifest"
            continue
        fi
    fi
    printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
done <"$direct_candidate"

LC_ALL=C sort -o "$mutation_candidate" "$mutation_candidate"
LC_ALL=C sort -o "$mutation_deferred" "$mutation_deferred"
LC_ALL=C sort -o "$mutation_manifest" "$mutation_manifest"
diff -u \
    "$mutation_candidate" \
    <(LC_ALL=C sort -u "$mutation_manifest" "$mutation_deferred")
if [[ -n "$(LC_ALL=C comm -12 "$mutation_manifest" "$mutation_deferred")" ]]; then
    echo "error: TypedArray mutation manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$mutation_candidate_keys"
: >"$mutation_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$mutation_candidate_keys"
done <"$mutation_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$mutation_keys"
done <"$mutation_manifest"
LC_ALL=C sort -o "$mutation_candidate_keys" "$mutation_candidate_keys"
LC_ALL=C sort -o "$mutation_keys" "$mutation_keys"
if [[ "$(wc -l <"$mutation_candidate" | tr -d '[:space:]')" \
        != "$expected_mutation_candidate_paths" \
    || "$(sha256_file "$mutation_candidate")" != "$expected_mutation_candidate" \
    || "$(wc -l <"$mutation_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_mutation_candidate_variants" \
    || "$(sha256_file "$mutation_candidate_keys")" \
        != "$expected_mutation_candidate_keys" \
    || "$(wc -l <"$mutation_deferred" | tr -d '[:space:]')" \
        != "$expected_mutation_deferred_paths" \
    || "$(sha256_file "$mutation_deferred")" != "$expected_mutation_deferred" \
    || "$(wc -l <"$mutation_manifest" | tr -d '[:space:]')" \
        != "$expected_mutation_paths" \
    || "$(sha256_file "$mutation_manifest")" != "$expected_mutation_manifest" \
    || "$(wc -l <"$mutation_keys" | tr -d '[:space:]')" \
        != "$expected_mutation_variants" \
    || "$(sha256_file "$mutation_keys")" != "$expected_mutation_keys" ]]; then
    echo "error: TypedArray mutation promotion inventory drifted" >&2
    exit 1
fi

while IFS= read -r test_path; do
    metadata_list "$test_path" features >"$candidate_features"
    metadata_list "$test_path" includes >"$candidate_includes"
    source_body "$test_path" >"$source_file"
    concrete_typed_array_tokens "$source_file" >"$typed_array_tokens"
    if [[ -z "$(metadata_block "$test_path")" \
        || -n "$(metadata_list "$test_path" flags)" \
        || -n "$(metadata_list "$test_path" features \
            | grep -E '^(SharedArrayBuffer|Atomics|immutable-arraybuffer|cross-realm)$' \
            || true)" \
        || "$(grep -c '^negative:' <<<"$(metadata_block "$test_path")" || true)" \
            != "0" ]]; then
        echo "error: latent TypedArray core spillover gained an external dependency: $test_path" >&2
        exit 1
    fi
    if [[ ! -s "$typed_array_tokens" ]] \
        && ! grep -Fxq sm/non262-Math-shell.js "$candidate_includes"; then
        echo "error: latent TypedArray spillover lost its source or harness dependency: $test_path" >&2
        exit 1
    fi
    if reason=$(spillover_dependency_reason "$test_path"); then
        printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
    else
        printf '%s\n' "$test_path" >>"$derived_manifest"
    fi
done <"$spillover_inventory"
LC_ALL=C sort -o "$derived_manifest" "$derived_manifest"
LC_ALL=C sort -o "$derived_exclusion_rows" "$derived_exclusion_rows"
printf '# path\treason\n' >"$derived_exclusions"
cat "$derived_exclusion_rows" >>"$derived_exclusions"
diff -u "$manifest_inventory" "$derived_manifest"
diff -u "$exclusions" "$derived_exclusions"

: >"$feature_occurrences"
: >"$include_occurrences"
: >"$variant_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" features >>"$feature_occurrences"
    metadata_list "$test_path" includes >>"$include_occurrences"
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$variant_keys"
done <"$manifest_inventory"
LC_ALL=C sort -u "$feature_occurrences" >"$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" >"$include_inventory"
LC_ALL=C sort -o "$variant_keys" "$variant_keys"
if [[ "$(wc -l <"$feature_inventory" | tr -d '[:space:]')" != "$expected_features" \
    || "$(sha256_file "$feature_inventory")" != "$expected_features_hash" \
    || "$(wc -l <"$include_inventory" | tr -d '[:space:]')" != "$expected_includes" \
    || "$(sha256_file "$include_inventory")" != "$expected_includes_hash" \
    || "$(wc -l <"$variant_keys" | tr -d '[:space:]')" != "$expected_variants" \
    || "$(sha256_file "$variant_keys")" != "$expected_keys" ]]; then
    echo "error: TypedArray manifest metadata or variant inventory drifted" >&2
    exit 1
fi
diff -u <(profile_section features | LC_ALL=C sort) "$feature_inventory"
if [[ -n "$(profile_section audited-negative-tests)" \
    || -n "$(profile_section execution)" ]]; then
    echo "error: TypedArray core profile must contain no negatives or execution opt-ins" >&2
    exit 1
fi

verify_quickjs_oracle \
    "TypedArray expanded candidate" \
    "$candidate_inventory" \
    "$expected_candidate_variants" \
    "$candidate_oracle_log"
verify_quickjs_oracle \
    "TypedArray core cohort" \
    "$manifest_inventory" \
    "$expected_quickjs_variants" \
    "$oracle_log"

if [[ "$check_only" == true ]]; then
    printf 'TypedArray core Test262 assets pass: %s candidate paths/%s variants, %s core paths/%s variants, %s exclusions; pinned QuickJS passes both vectors\n' \
        "$expected_candidate_paths" \
        "$expected_candidate_variants" \
        "$expected_paths" \
        "$expected_variants" \
        "$expected_excluded_paths"
    exit 0
fi

expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)
if [[ "$expected_passes" != "$expected_variants" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_nonpass" \
        != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" \
    || "$expected_summary" != "pass=$expected_variants" ]]; then
    echo "error: measured TypedArray core baseline is not an all-green gate" >&2
    exit 1
fi

verify_oxide_constructor_surface
rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" \
    --manifest "$manifest" \
    --report "$report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms" \
    --allow-failures)
printf '%s\n' "$run_output"

actual_variants=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { count++ } END { print count + 0 }' \
    "$report")
execution_line=$(printf '%s\n' "$run_output" \
    | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
actual_runnable=${execution_line#*runnable=}
actual_runnable=${actual_runnable%% *}
if [[ "$(read_header quickjs)" != "$expected_quickjs" \
    || "$(read_header test262)" != "$expected_test262" \
    || "$(read_header test262_patch_sha256)" != "$expected_patch" \
    || "$(read_header test262_config_sha256)" != "$expected_config" \
    || "$(read_header test262_metadata_sha256)" != "$expected_metadata" \
    || "$(read_header oxide_profile_sha256)" != "$expected_profile" \
    || "$(read_header profile)" != "$expected_schema" \
    || "$(read_header mode)" != "$expected_mode" \
    || "$actual_variants" != "$expected_variants" \
    || "$actual_runnable" != "$expected_variants" ]]; then
    echo "error: TypedArray core report metadata drifted" >&2
    exit 1
fi

diff -u \
    "$manifest_inventory" \
    <(awk -F'\t' \
        '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$report" | LC_ALL=C sort -u)
diff -u \
    "$feature_inventory" \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i <= count; i++) {
                if (features[i] != "") print features[i]
            }
        }
    ' "$report" | LC_ALL=C sort -u)

actual_keys=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' \
    "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ }
    END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" \
    | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
expected_runner_summary="Test262: total=$expected_variants pass=$expected_passes fail=$expected_failures unsupported=$expected_unsupported skipped=$expected_skipped"

if [[ "$runner_summary" != "$expected_runner_summary" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: TypedArray core classified vector drifted" >&2
    printf 'path\tvariant\toutcome\tactual_phase\tactual_type\tdetail\n' >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            if (++shown == 80) exit
        }
    ' "$report" >&2
    exit 1
fi

printf 'TypedArray core Test262 gate passes: %s/%s variants across %s paths; pinned QuickJS passes %s/%s\n' \
    "$expected_passes" \
    "$expected_variants" \
    "$expected_paths" \
    "$expected_quickjs_variants" \
    "$expected_quickjs_variants"
