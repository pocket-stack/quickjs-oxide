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
expected_profile=08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc
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
expected_index_search_candidate_paths=152
expected_index_search_candidate=8e68d86281c54b4b2a6a35422a55b348969d43fa11622c142cc31507aaae371f
expected_index_search_candidate_variants=304
expected_index_search_candidate_keys=934945e7ae5feef7de11c400da0ea7cdb72350027e4b803e2747d6afe9033d00
expected_index_search_deferred_paths=1
expected_index_search_deferred=de7e9738d5d1934ea4d23809c52acc9c11598d51f7f8dc321cae940d054a0d46
expected_index_search_deferred_variants=2
expected_index_search_deferred_keys=0011f9e461db721dc942bb2025209c994a710862fd6bc9add662133e238934c3
expected_index_search_paths=151
expected_index_search_manifest=061efff451e31693b84f61bf8072651ef366c1feb5ac880b2a47bba24203aeab
expected_index_search_variants=302
expected_index_search_keys=a63a1a8f7103e49cbd70c614beaf7f68d09b1019b217fce1f6f38fed8c877f15
expected_find_candidate_paths=158
expected_find_candidate=88049528555f5f985395612fcd92e90f447f147d5ea63efb9449a840c259933f
expected_find_candidate_variants=300
expected_find_candidate_keys=622062fc24a78be0b21f77cd9e0ede4fecd5f93cac8858b0db9f75220dbdb990
expected_find_deferred_paths=2
expected_find_deferred=4faf20dabff85cc8ffdee8c8d0d8212d290c8f41b4ef38ea4fc7bf9c36e0f6cc
expected_find_deferred_variants=4
expected_find_deferred_keys=29de30037c833b16b08d51c5e1f9ed476d2b57c29c30d0924854b270d765c7d1
expected_find_paths=156
expected_find_manifest=86de1d6f7e44e6d148bef24f86e24256df53b97ab90f3ad4a4be543f22d0ed4b
expected_find_variants=296
expected_find_keys=1304d6a4cee8a78cef45653c1b8247aa0400e8fe4fbdb34abac53c5bcd1e623f
expected_every_some_candidate_paths=93
expected_every_some_candidate=dbbd4a7e6f601888070c0f56de9771942e4d2354d75a29ab70439df3517d61cd
expected_every_some_candidate_variants=185
expected_every_some_candidate_keys=213e8b79b6447d17e562139b268ab87d7394ee6edebc755f4c4bbb31b9fe3ec4
expected_every_some_deferred_paths=1
expected_every_some_deferred=6189caae9a943a1fa5d65308b4bba02c25bba4af5d9e7e791da8820bd851b99f
expected_every_some_deferred_variants=1
expected_every_some_deferred_keys=2b728d9962391b75d27de09d05010642a9919f826719497c55e40e3f03a3e2f2
expected_every_some_paths=92
expected_every_some_manifest=8ad580d2a9cb33a091e714f7f309fd6c814503bfcb251ccdfd3bbbf5f87bae88
expected_every_some_variants=184
expected_every_some_keys=9144eaf7e8b0c6664fd082d639aa35c176ee34d3d1947452fad6523dabe22604
expected_for_each_candidate_paths=45
expected_for_each_candidate=ee8af85d761e4da707fc72afc992e8c0e0b314782d0f879cff69845e66cc2bf6
expected_for_each_candidate_variants=89
expected_for_each_candidate_keys=67f42550bd10879a86d2401c4048e30a833a6ccda375b0d41ed44287b575c2a5
expected_for_each_deferred_paths=1
expected_for_each_deferred=26efea2e4065acf3a5bf1d8dab6ed0a78df866e1d956f9e08c44644635a5239f
expected_for_each_deferred_variants=1
expected_for_each_deferred_keys=e3ce2a05f163af4827c1fdad2c7535a2dfe7f46bbe27c3c0ed76a803650bf661
expected_for_each_paths=44
expected_for_each_manifest=dba18b09bd2a2bc35a9f716e9a371547757d6225d2433c524a45cd5b92ba7177
expected_for_each_variants=88
expected_for_each_keys=e3c038e152bb843d9dd55e9d16f89ca6227ac690a1e6d378c78d26757a211c4f
expected_reduce_candidate_paths=105
expected_reduce_candidate=f40c52a2edb4635d7ca1ec1a2b0abfa4c978c51a73ae567b8efffd8ab5d87ad5
expected_reduce_candidate_variants=209
expected_reduce_candidate_keys=6cc0b62d9fe01cdaacf629a3152ca09b975ada81b4169bad7ffb05714662fe72
expected_reduce_deferred_paths=1
expected_reduce_deferred=b99151319be2a66b2d78111bff0ea5e73a308313670a1b4e9488a3afefd6f909
expected_reduce_deferred_variants=1
expected_reduce_deferred_keys=97e3f4dbb189808dc1dd6cb9f8be100c74edbbb333e4c890c165cb7409fdf6cb
expected_reduce_paths=104
expected_reduce_manifest=79f2ce5172ba5afc48a87a3417ce99010762ba9de2cc3c49dd4db7696d6ba7b6
expected_reduce_variants=208
expected_reduce_keys=79522bed3692d0c21ac44370796b6c37861dca2fab511d38d8872605e78d9fff
expected_map_filter_candidate_paths=175
expected_map_filter_candidate=2a4d0d92c7a4b3aec6e559770bd3baa5780b2c3780f408333526619dfbfef9fc
expected_map_filter_candidate_variants=349
expected_map_filter_candidate_keys=9e51d82281ea14f0568b2116054927aca5187708584e68b8cf551426f7529743
expected_map_filter_deferred_paths=1
expected_map_filter_deferred=198ede24f4c8a6e1dbb4135a14906c9f8a513178a42f23545711651eeaf26e31
expected_map_filter_deferred_variants=1
expected_map_filter_deferred_keys=c7140d02e8e9d00feedd33ff35c98afa0a1bf365db3dd6ede640f1a8b34c6bd3
expected_map_filter_paths=174
expected_map_filter_manifest=57a0d825fa96ae56a44dd64be290d6368838d90fcd5cdd739c9735573b8d2a02
expected_map_filter_variants=348
expected_map_filter_keys=b92f4b302934a05ca68f39bde019ef71f2353a664f3e304f2092ccf1eb8cf78b
expected_excluded_paths=654
expected_exclusions=b2406a45aab98366342205bf4fb5149091b802500dc09b5a6afb8a1ef784c774
expected_exclusions_file=1c3d6f79c99f423c77c11256d65993143b4fced944f700f64b16975ffb730298
expected_paths=1707
expected_variants=3375
expected_quickjs_variants=3375
expected_features=24
expected_features_hash=1615b6491b5ce6759bb700f60052458442b3c0e1eaf275e157d094bb4ab411d4
expected_includes=11
expected_includes_hash=b1b60b5e1f7635615ff31eb139d1803608e5743c5f46ca53fadc3797e0abe012
expected_manifest=e6a3af181bf643b70558661802544681ac92356f06c4c27c9b1504b31379b42f
expected_keys=6bf48fc08165d42f32ff8ed7cf08ad94249b23daaf111cc3700df248c667b075
expected_test_typed_array_harness=4c0e237804f39a4aa670f72c05b4520730c03c2d2e9f2f41e6b380bd6749ec61
expected_sm_typed_array_harness=3798d277ac8f105b65ad26602b500b497af7f3361fd14a169c58a601c605bb2e
expected_sm_math_harness=79dea1172236685567e09da8c9e868e0f84686bf40cff728785223c5b43f5e7b

usage() {
    cat <<'EOF'
usage: scripts/test-test262-typed-array-core.sh [--check]

With --check, rebuild and audit the frozen TypedArray candidate, mutation,
index/search, callback-find, every/some, forEach, reduce/reduceRight, and
map/filter promotions, manifest, and exclusion ledger. Verify all 4,669
candidate variants plus the 3,375 admitted variants against pinned QuickJS.
With no option, also run the checksum-bound quickjs-oxide gate; that mode
requires a measured all-green baseline file.
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

index_search_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/at/*|\
        test/built-ins/TypedArray/prototype/includes/*|\
        test/built-ins/TypedArray/prototype/indexOf/*|\
        test/built-ins/TypedArray/prototype/lastIndexOf/*|\
        test/built-ins/TypedArrayConstructors/prototype/indexOf/*|\
        test/built-ins/TypedArrayConstructors/prototype/lastIndexOf/*|\
        test/staging/sm/TypedArray/indexOf-and-lastIndexOf.js|\
        test/staging/sm/TypedArray/indexOf-never-returns-negative-zero.js|\
        test/staging/sm/TypedArray/lastIndexOf-never-returns-negative-zero.js)
            return 0
            ;;
    esac
    return 1
}

index_search_dependency_reason() {
    local test_path=$1 includes_file=$2
    case "$test_path" in
        test/staging/sm/TypedArray/indexOf-and-lastIndexOf.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file"; then
                echo "error: TypedArray index/search WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

find_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/find/*|\
        test/built-ins/TypedArray/prototype/findIndex/*|\
        test/built-ins/TypedArray/prototype/findLast/*|\
        test/built-ins/TypedArray/prototype/findLastIndex/*|\
        test/built-ins/TypedArrayConstructors/prototype/find/*|\
        test/built-ins/TypedArrayConstructors/prototype/findIndex/*|\
        test/staging/sm/TypedArray/find-and-findIndex.js|\
        test/staging/sm/TypedArray/findLast-and-findLastIndex.js)
            return 0
            ;;
    esac
    return 1
}

find_dependency_reason() {
    local test_path=$1 includes_file=$2
    case "$test_path" in
        test/staging/sm/TypedArray/find-and-findIndex.js|\
        test/staging/sm/TypedArray/findLast-and-findLastIndex.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file"; then
                echo "error: TypedArray callback-find WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

every_some_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/every/*|\
        test/built-ins/TypedArray/prototype/some/*|\
        test/built-ins/TypedArrayConstructors/prototype/every/*|\
        test/built-ins/TypedArrayConstructors/prototype/some/*|\
        test/staging/sm/TypedArray/every-and-some.js)
            return 0
            ;;
    esac
    return 1
}

every_some_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/every-and-some.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray every/some realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        *)
            return 1
            ;;
    esac
}

for_each_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/forEach/*|\
        test/built-ins/TypedArrayConstructors/prototype/forEach/*|\
        test/staging/sm/TypedArray/forEach.js)
            return 0
            ;;
    esac
    return 1
}

for_each_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/forEach.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray forEach realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        *)
            return 1
            ;;
    esac
}

reduce_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/reduce/*|\
        test/built-ins/TypedArray/prototype/reduceRight/*|\
        test/built-ins/TypedArrayConstructors/prototype/reduce/*|\
        test/built-ins/TypedArrayConstructors/prototype/reduceRight/*|\
        test/staging/sm/TypedArray/reduce-and-reduceRight.js)
            return 0
            ;;
    esac
    return 1
}

reduce_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/reduce-and-reduceRight.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray reduce/reduceRight realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        *)
            return 1
            ;;
    esac
}

map_filter_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/filter/*|\
        test/built-ins/TypedArray/prototype/map/*|\
        test/built-ins/TypedArrayConstructors/prototype/filter/*|\
        test/built-ins/TypedArrayConstructors/prototype/map/*|\
        test/staging/sm/TypedArray/filter-species.js|\
        test/staging/sm/TypedArray/map-and-filter.js|\
        test/staging/sm/TypedArray/map-species.js)
            return 0
            ;;
    esac
    return 1
}

map_filter_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/map-and-filter.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray map/filter realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
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
    expect_value index_search_candidate_paths "$expected_index_search_candidate_paths"
    expect_value index_search_candidate_sha256 "$expected_index_search_candidate"
    expect_value index_search_candidate_variants "$expected_index_search_candidate_variants"
    expect_value index_search_candidate_keys_sha256 "$expected_index_search_candidate_keys"
    expect_value index_search_deferred_paths "$expected_index_search_deferred_paths"
    expect_value index_search_deferred_sha256 "$expected_index_search_deferred"
    expect_value index_search_deferred_variants "$expected_index_search_deferred_variants"
    expect_value index_search_deferred_keys_sha256 "$expected_index_search_deferred_keys"
    expect_value index_search_paths "$expected_index_search_paths"
    expect_value index_search_manifest_sha256 "$expected_index_search_manifest"
    expect_value index_search_variants "$expected_index_search_variants"
    expect_value index_search_keys_sha256 "$expected_index_search_keys"
    expect_value find_candidate_paths "$expected_find_candidate_paths"
    expect_value find_candidate_sha256 "$expected_find_candidate"
    expect_value find_candidate_variants "$expected_find_candidate_variants"
    expect_value find_candidate_keys_sha256 "$expected_find_candidate_keys"
    expect_value find_deferred_paths "$expected_find_deferred_paths"
    expect_value find_deferred_sha256 "$expected_find_deferred"
    expect_value find_deferred_variants "$expected_find_deferred_variants"
    expect_value find_deferred_keys_sha256 "$expected_find_deferred_keys"
    expect_value find_paths "$expected_find_paths"
    expect_value find_manifest_sha256 "$expected_find_manifest"
    expect_value find_variants "$expected_find_variants"
    expect_value find_keys_sha256 "$expected_find_keys"
    expect_value every_some_candidate_paths "$expected_every_some_candidate_paths"
    expect_value every_some_candidate_sha256 "$expected_every_some_candidate"
    expect_value every_some_candidate_variants "$expected_every_some_candidate_variants"
    expect_value every_some_candidate_keys_sha256 "$expected_every_some_candidate_keys"
    expect_value every_some_deferred_paths "$expected_every_some_deferred_paths"
    expect_value every_some_deferred_sha256 "$expected_every_some_deferred"
    expect_value every_some_deferred_variants "$expected_every_some_deferred_variants"
    expect_value every_some_deferred_keys_sha256 "$expected_every_some_deferred_keys"
    expect_value every_some_paths "$expected_every_some_paths"
    expect_value every_some_manifest_sha256 "$expected_every_some_manifest"
    expect_value every_some_variants "$expected_every_some_variants"
    expect_value every_some_keys_sha256 "$expected_every_some_keys"
    expect_value for_each_candidate_paths "$expected_for_each_candidate_paths"
    expect_value for_each_candidate_sha256 "$expected_for_each_candidate"
    expect_value for_each_candidate_variants "$expected_for_each_candidate_variants"
    expect_value for_each_candidate_keys_sha256 "$expected_for_each_candidate_keys"
    expect_value for_each_deferred_paths "$expected_for_each_deferred_paths"
    expect_value for_each_deferred_sha256 "$expected_for_each_deferred"
    expect_value for_each_deferred_variants "$expected_for_each_deferred_variants"
    expect_value for_each_deferred_keys_sha256 "$expected_for_each_deferred_keys"
    expect_value for_each_paths "$expected_for_each_paths"
    expect_value for_each_manifest_sha256 "$expected_for_each_manifest"
    expect_value for_each_variants "$expected_for_each_variants"
    expect_value for_each_keys_sha256 "$expected_for_each_keys"
    expect_value reduce_candidate_paths "$expected_reduce_candidate_paths"
    expect_value reduce_candidate_sha256 "$expected_reduce_candidate"
    expect_value reduce_candidate_variants "$expected_reduce_candidate_variants"
    expect_value reduce_candidate_keys_sha256 "$expected_reduce_candidate_keys"
    expect_value reduce_deferred_paths "$expected_reduce_deferred_paths"
    expect_value reduce_deferred_sha256 "$expected_reduce_deferred"
    expect_value reduce_deferred_variants "$expected_reduce_deferred_variants"
    expect_value reduce_deferred_keys_sha256 "$expected_reduce_deferred_keys"
    expect_value reduce_paths "$expected_reduce_paths"
    expect_value reduce_manifest_sha256 "$expected_reduce_manifest"
    expect_value reduce_variants "$expected_reduce_variants"
    expect_value reduce_keys_sha256 "$expected_reduce_keys"
    expect_value map_filter_candidate_paths "$expected_map_filter_candidate_paths"
    expect_value map_filter_candidate_sha256 "$expected_map_filter_candidate"
    expect_value map_filter_candidate_variants "$expected_map_filter_candidate_variants"
    expect_value map_filter_candidate_keys_sha256 "$expected_map_filter_candidate_keys"
    expect_value map_filter_deferred_paths "$expected_map_filter_deferred_paths"
    expect_value map_filter_deferred_sha256 "$expected_map_filter_deferred"
    expect_value map_filter_deferred_variants "$expected_map_filter_deferred_variants"
    expect_value map_filter_deferred_keys_sha256 "$expected_map_filter_deferred_keys"
    expect_value map_filter_paths "$expected_map_filter_paths"
    expect_value map_filter_manifest_sha256 "$expected_map_filter_manifest"
    expect_value map_filter_variants "$expected_map_filter_variants"
    expect_value map_filter_keys_sha256 "$expected_map_filter_keys"
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
index_search_candidate=$tmp_dir/index-search-candidate.txt
index_search_candidate_keys=$tmp_dir/index-search-candidate-keys.txt
index_search_deferred=$tmp_dir/index-search-deferred.txt
index_search_deferred_keys=$tmp_dir/index-search-deferred-keys.txt
index_search_manifest=$tmp_dir/index-search-manifest.txt
index_search_keys=$tmp_dir/index-search-keys.txt
find_candidate=$tmp_dir/find-candidate.txt
find_candidate_keys=$tmp_dir/find-candidate-keys.txt
find_deferred=$tmp_dir/find-deferred.txt
find_deferred_keys=$tmp_dir/find-deferred-keys.txt
find_manifest=$tmp_dir/find-manifest.txt
find_keys=$tmp_dir/find-keys.txt
every_some_candidate=$tmp_dir/every-some-candidate.txt
every_some_candidate_keys=$tmp_dir/every-some-candidate-keys.txt
every_some_deferred=$tmp_dir/every-some-deferred.txt
every_some_deferred_keys=$tmp_dir/every-some-deferred-keys.txt
every_some_manifest=$tmp_dir/every-some-manifest.txt
every_some_keys=$tmp_dir/every-some-keys.txt
for_each_candidate=$tmp_dir/for-each-candidate.txt
for_each_candidate_keys=$tmp_dir/for-each-candidate-keys.txt
for_each_deferred=$tmp_dir/for-each-deferred.txt
for_each_deferred_keys=$tmp_dir/for-each-deferred-keys.txt
for_each_manifest=$tmp_dir/for-each-manifest.txt
for_each_keys=$tmp_dir/for-each-keys.txt
reduce_candidate=$tmp_dir/reduce-candidate.txt
reduce_candidate_keys=$tmp_dir/reduce-candidate-keys.txt
reduce_deferred=$tmp_dir/reduce-deferred.txt
reduce_deferred_keys=$tmp_dir/reduce-deferred-keys.txt
reduce_manifest=$tmp_dir/reduce-manifest.txt
reduce_keys=$tmp_dir/reduce-keys.txt
map_filter_candidate=$tmp_dir/map-filter-candidate.txt
map_filter_candidate_keys=$tmp_dir/map-filter-candidate-keys.txt
map_filter_deferred=$tmp_dir/map-filter-deferred.txt
map_filter_deferred_keys=$tmp_dir/map-filter-deferred-keys.txt
map_filter_manifest=$tmp_dir/map-filter-manifest.txt
map_filter_keys=$tmp_dir/map-filter-keys.txt
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
        if (NR != 655 ||
            counts["dependency:join"] != 2 ||
            counts["external:cross-realm"] != 54 ||
            counts["external:SharedArrayBuffer"] != 71 ||
            counts["external:WeakMap"] != 6 ||
            counts["external:Math"] != 1 ||
            counts["external:IsHTMLDDA"] != 1 ||
            counts["static:from"] != 88 ||
            counts["static:of"] != 34 ||
            counts["method:iterator-entries-keys"] != 42 ||
            counts["method:mutation-copy-set"] != 0 ||
            counts["method:search-predicate"] != 0 ||
            counts["method:species-copy-transform"] != 214 ||
            counts["method:callback-reduce"] != 0 ||
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
: >"$index_search_candidate"
: >"$index_search_deferred"
: >"$index_search_manifest"
: >"$find_candidate"
: >"$find_deferred"
: >"$find_manifest"
: >"$every_some_candidate"
: >"$every_some_deferred"
: >"$every_some_manifest"
: >"$for_each_candidate"
: >"$for_each_deferred"
: >"$for_each_manifest"
: >"$reduce_candidate"
: >"$reduce_deferred"
: >"$reduce_manifest"
: >"$map_filter_candidate"
: >"$map_filter_deferred"
: >"$map_filter_manifest"
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
    if every_some_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$every_some_candidate"
        if reason=$(every_some_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$every_some_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
    fi
    if for_each_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$for_each_candidate"
        if reason=$(for_each_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$for_each_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
    fi
    if reduce_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$reduce_candidate"
        if reason=$(reduce_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$reduce_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
    fi
    if map_filter_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$map_filter_candidate"
        if reason=$(map_filter_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$map_filter_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
    fi
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
    elif [[ "$reason" == "method:search-predicate" ]] \
        && index_search_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$index_search_candidate"
        if reason=$(index_search_dependency_reason \
            "$test_path" "$candidate_includes"); then
            printf '%s\n' "$test_path" >>"$index_search_deferred"
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
            printf '%s\n' "$test_path" >>"$derived_manifest"
            printf '%s\n' "$test_path" >>"$index_search_manifest"
            continue
        fi
    elif [[ "$reason" == "method:search-predicate" ]] \
        && find_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$find_candidate"
        if reason=$(find_dependency_reason \
            "$test_path" "$candidate_includes"); then
            printf '%s\n' "$test_path" >>"$find_deferred"
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
            printf '%s\n' "$test_path" >>"$derived_manifest"
            printf '%s\n' "$test_path" >>"$find_manifest"
            continue
        fi
    elif [[ "$reason" == "method:search-predicate" ]] \
        && every_some_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$every_some_manifest"
        continue
    elif [[ "$reason" == "method:callback-reduce" ]] \
        && for_each_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$for_each_manifest"
        continue
    elif [[ "$reason" == "method:callback-reduce" ]] \
        && reduce_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$reduce_manifest"
        continue
    elif [[ "$reason" == "method:species-copy-transform" ]] \
        && map_filter_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$map_filter_manifest"
        continue
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

LC_ALL=C sort -o "$index_search_candidate" "$index_search_candidate"
LC_ALL=C sort -o "$index_search_deferred" "$index_search_deferred"
LC_ALL=C sort -o "$index_search_manifest" "$index_search_manifest"
diff -u \
    "$index_search_candidate" \
    <(LC_ALL=C sort -u "$index_search_manifest" "$index_search_deferred")
if [[ -n "$(LC_ALL=C comm -12 "$index_search_manifest" "$index_search_deferred")" ]]; then
    echo "error: TypedArray index/search manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$index_search_candidate_keys"
: >"$index_search_deferred_keys"
: >"$index_search_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$index_search_candidate_keys"
done <"$index_search_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$index_search_deferred_keys"
done <"$index_search_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$index_search_keys"
done <"$index_search_manifest"
LC_ALL=C sort -o "$index_search_candidate_keys" "$index_search_candidate_keys"
LC_ALL=C sort -o "$index_search_deferred_keys" "$index_search_deferred_keys"
LC_ALL=C sort -o "$index_search_keys" "$index_search_keys"
if [[ "$(wc -l <"$index_search_candidate" | tr -d '[:space:]')" \
        != "$expected_index_search_candidate_paths" \
    || "$(sha256_file "$index_search_candidate")" \
        != "$expected_index_search_candidate" \
    || "$(wc -l <"$index_search_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_index_search_candidate_variants" \
    || "$(sha256_file "$index_search_candidate_keys")" \
        != "$expected_index_search_candidate_keys" \
    || "$(wc -l <"$index_search_deferred" | tr -d '[:space:]')" \
        != "$expected_index_search_deferred_paths" \
    || "$(sha256_file "$index_search_deferred")" \
        != "$expected_index_search_deferred" \
    || "$(wc -l <"$index_search_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_index_search_deferred_variants" \
    || "$(sha256_file "$index_search_deferred_keys")" \
        != "$expected_index_search_deferred_keys" \
    || "$(wc -l <"$index_search_manifest" | tr -d '[:space:]')" \
        != "$expected_index_search_paths" \
    || "$(sha256_file "$index_search_manifest")" \
        != "$expected_index_search_manifest" \
    || "$(wc -l <"$index_search_keys" | tr -d '[:space:]')" \
        != "$expected_index_search_variants" \
    || "$(sha256_file "$index_search_keys")" != "$expected_index_search_keys" ]]; then
    echo "error: TypedArray index/search promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$find_candidate" "$find_candidate"
LC_ALL=C sort -o "$find_deferred" "$find_deferred"
LC_ALL=C sort -o "$find_manifest" "$find_manifest"
diff -u \
    "$find_candidate" \
    <(LC_ALL=C sort -u "$find_manifest" "$find_deferred")
if [[ -n "$(LC_ALL=C comm -12 "$find_manifest" "$find_deferred")" ]]; then
    echo "error: TypedArray callback-find manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$find_candidate_keys"
: >"$find_deferred_keys"
: >"$find_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$find_candidate_keys"
done <"$find_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$find_deferred_keys"
done <"$find_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$find_keys"
done <"$find_manifest"
LC_ALL=C sort -o "$find_candidate_keys" "$find_candidate_keys"
LC_ALL=C sort -o "$find_deferred_keys" "$find_deferred_keys"
LC_ALL=C sort -o "$find_keys" "$find_keys"
if [[ "$(wc -l <"$find_candidate" | tr -d '[:space:]')" \
        != "$expected_find_candidate_paths" \
    || "$(sha256_file "$find_candidate")" != "$expected_find_candidate" \
    || "$(wc -l <"$find_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_find_candidate_variants" \
    || "$(sha256_file "$find_candidate_keys")" \
        != "$expected_find_candidate_keys" \
    || "$(wc -l <"$find_deferred" | tr -d '[:space:]')" \
        != "$expected_find_deferred_paths" \
    || "$(sha256_file "$find_deferred")" != "$expected_find_deferred" \
    || "$(wc -l <"$find_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_find_deferred_variants" \
    || "$(sha256_file "$find_deferred_keys")" \
        != "$expected_find_deferred_keys" \
    || "$(wc -l <"$find_manifest" | tr -d '[:space:]')" \
        != "$expected_find_paths" \
    || "$(sha256_file "$find_manifest")" != "$expected_find_manifest" \
    || "$(wc -l <"$find_keys" | tr -d '[:space:]')" \
        != "$expected_find_variants" \
    || "$(sha256_file "$find_keys")" != "$expected_find_keys" ]]; then
    echo "error: TypedArray callback-find promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$every_some_candidate" "$every_some_candidate"
LC_ALL=C sort -o "$every_some_deferred" "$every_some_deferred"
LC_ALL=C sort -o "$every_some_manifest" "$every_some_manifest"
diff -u \
    "$every_some_candidate" \
    <(LC_ALL=C sort -u "$every_some_manifest" "$every_some_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$every_some_manifest" "$every_some_deferred")" ]]; then
    echo "error: TypedArray every/some manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$every_some_candidate_keys"
: >"$every_some_deferred_keys"
: >"$every_some_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$every_some_candidate_keys"
done <"$every_some_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$every_some_deferred_keys"
done <"$every_some_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$every_some_keys"
done <"$every_some_manifest"
LC_ALL=C sort -o "$every_some_candidate_keys" "$every_some_candidate_keys"
LC_ALL=C sort -o "$every_some_deferred_keys" "$every_some_deferred_keys"
LC_ALL=C sort -o "$every_some_keys" "$every_some_keys"
if [[ "$(wc -l <"$every_some_candidate" | tr -d '[:space:]')" \
        != "$expected_every_some_candidate_paths" \
    || "$(sha256_file "$every_some_candidate")" \
        != "$expected_every_some_candidate" \
    || "$(wc -l <"$every_some_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_every_some_candidate_variants" \
    || "$(sha256_file "$every_some_candidate_keys")" \
        != "$expected_every_some_candidate_keys" \
    || "$(wc -l <"$every_some_deferred" | tr -d '[:space:]')" \
        != "$expected_every_some_deferred_paths" \
    || "$(sha256_file "$every_some_deferred")" \
        != "$expected_every_some_deferred" \
    || "$(wc -l <"$every_some_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_every_some_deferred_variants" \
    || "$(sha256_file "$every_some_deferred_keys")" \
        != "$expected_every_some_deferred_keys" \
    || "$(wc -l <"$every_some_manifest" | tr -d '[:space:]')" \
        != "$expected_every_some_paths" \
    || "$(sha256_file "$every_some_manifest")" \
        != "$expected_every_some_manifest" \
    || "$(wc -l <"$every_some_keys" | tr -d '[:space:]')" \
        != "$expected_every_some_variants" \
    || "$(sha256_file "$every_some_keys")" \
        != "$expected_every_some_keys" ]]; then
    echo "error: TypedArray every/some promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$for_each_candidate" "$for_each_candidate"
LC_ALL=C sort -o "$for_each_deferred" "$for_each_deferred"
LC_ALL=C sort -o "$for_each_manifest" "$for_each_manifest"
diff -u \
    "$for_each_candidate" \
    <(LC_ALL=C sort -u "$for_each_manifest" "$for_each_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$for_each_manifest" "$for_each_deferred")" ]]; then
    echo "error: TypedArray forEach manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$for_each_candidate_keys"
: >"$for_each_deferred_keys"
: >"$for_each_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$for_each_candidate_keys"
done <"$for_each_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$for_each_deferred_keys"
done <"$for_each_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$for_each_keys"
done <"$for_each_manifest"
LC_ALL=C sort -o "$for_each_candidate_keys" "$for_each_candidate_keys"
LC_ALL=C sort -o "$for_each_deferred_keys" "$for_each_deferred_keys"
LC_ALL=C sort -o "$for_each_keys" "$for_each_keys"
if [[ "$(wc -l <"$for_each_candidate" | tr -d '[:space:]')" \
        != "$expected_for_each_candidate_paths" \
    || "$(sha256_file "$for_each_candidate")" \
        != "$expected_for_each_candidate" \
    || "$(wc -l <"$for_each_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_for_each_candidate_variants" \
    || "$(sha256_file "$for_each_candidate_keys")" \
        != "$expected_for_each_candidate_keys" \
    || "$(wc -l <"$for_each_deferred" | tr -d '[:space:]')" \
        != "$expected_for_each_deferred_paths" \
    || "$(sha256_file "$for_each_deferred")" \
        != "$expected_for_each_deferred" \
    || "$(wc -l <"$for_each_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_for_each_deferred_variants" \
    || "$(sha256_file "$for_each_deferred_keys")" \
        != "$expected_for_each_deferred_keys" \
    || "$(wc -l <"$for_each_manifest" | tr -d '[:space:]')" \
        != "$expected_for_each_paths" \
    || "$(sha256_file "$for_each_manifest")" \
        != "$expected_for_each_manifest" \
    || "$(wc -l <"$for_each_keys" | tr -d '[:space:]')" \
        != "$expected_for_each_variants" \
    || "$(sha256_file "$for_each_keys")" \
        != "$expected_for_each_keys" ]]; then
    echo "error: TypedArray forEach promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$reduce_candidate" "$reduce_candidate"
LC_ALL=C sort -o "$reduce_deferred" "$reduce_deferred"
LC_ALL=C sort -o "$reduce_manifest" "$reduce_manifest"
diff -u \
    "$reduce_candidate" \
    <(LC_ALL=C sort -u "$reduce_manifest" "$reduce_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$reduce_manifest" "$reduce_deferred")" ]]; then
    echo "error: TypedArray reduce/reduceRight manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$reduce_candidate_keys"
: >"$reduce_deferred_keys"
: >"$reduce_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$reduce_candidate_keys"
done <"$reduce_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$reduce_deferred_keys"
done <"$reduce_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$reduce_keys"
done <"$reduce_manifest"
LC_ALL=C sort -o "$reduce_candidate_keys" "$reduce_candidate_keys"
LC_ALL=C sort -o "$reduce_deferred_keys" "$reduce_deferred_keys"
LC_ALL=C sort -o "$reduce_keys" "$reduce_keys"
if [[ "$(wc -l <"$reduce_candidate" | tr -d '[:space:]')" \
        != "$expected_reduce_candidate_paths" \
    || "$(sha256_file "$reduce_candidate")" \
        != "$expected_reduce_candidate" \
    || "$(wc -l <"$reduce_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_reduce_candidate_variants" \
    || "$(sha256_file "$reduce_candidate_keys")" \
        != "$expected_reduce_candidate_keys" \
    || "$(wc -l <"$reduce_deferred" | tr -d '[:space:]')" \
        != "$expected_reduce_deferred_paths" \
    || "$(sha256_file "$reduce_deferred")" \
        != "$expected_reduce_deferred" \
    || "$(wc -l <"$reduce_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_reduce_deferred_variants" \
    || "$(sha256_file "$reduce_deferred_keys")" \
        != "$expected_reduce_deferred_keys" \
    || "$(wc -l <"$reduce_manifest" | tr -d '[:space:]')" \
        != "$expected_reduce_paths" \
    || "$(sha256_file "$reduce_manifest")" \
        != "$expected_reduce_manifest" \
    || "$(wc -l <"$reduce_keys" | tr -d '[:space:]')" \
        != "$expected_reduce_variants" \
    || "$(sha256_file "$reduce_keys")" \
        != "$expected_reduce_keys" ]]; then
    echo "error: TypedArray reduce/reduceRight promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$map_filter_candidate" "$map_filter_candidate"
LC_ALL=C sort -o "$map_filter_deferred" "$map_filter_deferred"
LC_ALL=C sort -o "$map_filter_manifest" "$map_filter_manifest"
diff -u \
    "$map_filter_candidate" \
    <(LC_ALL=C sort -u "$map_filter_manifest" "$map_filter_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$map_filter_manifest" "$map_filter_deferred")" ]]; then
    echo "error: TypedArray map/filter manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$map_filter_candidate_keys"
: >"$map_filter_deferred_keys"
: >"$map_filter_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$map_filter_candidate_keys"
done <"$map_filter_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$map_filter_deferred_keys"
done <"$map_filter_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$map_filter_keys"
done <"$map_filter_manifest"
LC_ALL=C sort -o "$map_filter_candidate_keys" "$map_filter_candidate_keys"
LC_ALL=C sort -o "$map_filter_deferred_keys" "$map_filter_deferred_keys"
LC_ALL=C sort -o "$map_filter_keys" "$map_filter_keys"
if [[ "$(wc -l <"$map_filter_candidate" | tr -d '[:space:]')" \
        != "$expected_map_filter_candidate_paths" \
    || "$(sha256_file "$map_filter_candidate")" \
        != "$expected_map_filter_candidate" \
    || "$(wc -l <"$map_filter_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_map_filter_candidate_variants" \
    || "$(sha256_file "$map_filter_candidate_keys")" \
        != "$expected_map_filter_candidate_keys" \
    || "$(wc -l <"$map_filter_deferred" | tr -d '[:space:]')" \
        != "$expected_map_filter_deferred_paths" \
    || "$(sha256_file "$map_filter_deferred")" \
        != "$expected_map_filter_deferred" \
    || "$(wc -l <"$map_filter_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_map_filter_deferred_variants" \
    || "$(sha256_file "$map_filter_deferred_keys")" \
        != "$expected_map_filter_deferred_keys" \
    || "$(wc -l <"$map_filter_manifest" | tr -d '[:space:]')" \
        != "$expected_map_filter_paths" \
    || "$(sha256_file "$map_filter_manifest")" \
        != "$expected_map_filter_manifest" \
    || "$(wc -l <"$map_filter_keys" | tr -d '[:space:]')" \
        != "$expected_map_filter_variants" \
    || "$(sha256_file "$map_filter_keys")" \
        != "$expected_map_filter_keys" ]]; then
    echo "error: TypedArray map/filter promotion inventory drifted" >&2
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
    printf 'TypedArray core Test262 assets pass: %s candidate paths/%s variants, %s core paths/%s variants (including %s callback-find paths/%s variants, %s every/some paths/%s variants, %s forEach paths/%s variants, %s reduce/reduceRight paths/%s variants, and %s map/filter paths/%s variants; %s every/some, %s forEach, %s reduce/reduceRight, and %s map/filter staging paths deferred), %s exclusions; pinned QuickJS passes candidate and admitted vectors\n' \
        "$expected_candidate_paths" \
        "$expected_candidate_variants" \
        "$expected_paths" \
        "$expected_variants" \
        "$expected_find_paths" \
        "$expected_find_variants" \
        "$expected_every_some_paths" \
        "$expected_every_some_variants" \
        "$expected_for_each_paths" \
        "$expected_for_each_variants" \
        "$expected_reduce_paths" \
        "$expected_reduce_variants" \
        "$expected_map_filter_paths" \
        "$expected_map_filter_variants" \
        "$expected_every_some_deferred_paths" \
        "$expected_for_each_deferred_paths" \
        "$expected_reduce_deferred_paths" \
        "$expected_map_filter_deferred_paths" \
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
