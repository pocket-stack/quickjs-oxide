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
current_global_profile=compat/test262-oxide.conf
r3be_global_features=tests/test262-typed-array-r3be-global-features.txt
global_profile=
global_activation_baseline=tests/test262-typed-array-global-activation-baseline.txt
global_activation_manifest=tests/test262-typed-array-global-activation.txt
global_reason_only_manifest=tests/test262-typed-array-global-reason-only.txt
global_transition_receipt=tests/test262-typed-array-global-r3bd-r3be-transitions.tsv
report=target/test262-typed-array-core.tsv
json_report=target/test262-typed-array-core.jsonl
oracle_log=target/test262-typed-array-core-quickjs.log
candidate_oracle_log=target/test262-typed-array-core-candidate-quickjs.log
global_activation_report=target/test262-typed-array-global-activation.tsv
global_activation_json_report=target/test262-typed-array-global-activation.jsonl
global_reason_only_report=target/test262-typed-array-global-reason-only.tsv
global_reason_only_json_report=target/test262-typed-array-global-reason-only.jsonl
workers=${TEST262_WORKERS:-8}
check_only=false

expected_quickjs=2026-06-04
expected_test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
expected_patch=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expected_config=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expected_metadata=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expected_profile=dd106c074751866ce667352d3449cc0ec7d9b9072034a4f0a97050da7b7bad13
expected_schema=test262-canonical-classified-v2
expected_mode=both
expected_timeout_ms=30000
reason_detail_prefix='quickjs-oxide does not declare Test262 feature support: '
expected_direct_candidate_paths=2316
expected_direct_candidate=64dfc295efac5414db8743def6099f484bb69090676378087382a23d5b3565a4
expected_spillover_paths=86
expected_spillover=62f1568f813f2d4f892feab77d17fb85e6576bd9c89e645095830f0e85c71eae
expected_candidate_paths=2402
expected_candidate=3faf9a7c21d28381c13a6a56a0ee1198c4a2689b48b96b6d0ebab5b6ae4c88fa
expected_candidate_variants=4749
expected_candidate_keys=3ed6b7014bc4dbc2a0b000d9d51f075e902442567c48d315268a004d73c036c4
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
expected_slice_subarray_candidate_paths=178
expected_slice_subarray_candidate=b47079faf02e6e29ab9b1d1da45d35d79f30f1498fff96ea47c3d0fdf4057417
expected_slice_subarray_candidate_variants=356
expected_slice_subarray_candidate_keys=d149931f862e672317077644ffae6ccc6e319442a97dbb2a951bb1cdaeed8769
expected_slice_subarray_deferred_paths=5
expected_slice_subarray_deferred=9f1d0a737704df4c1503cecd69ec953faae2496fa6da4bff07d36b35b377c328
expected_slice_subarray_deferred_variants=10
expected_slice_subarray_deferred_keys=c991213141a15cd3e647dd9b1c40553c5dc0a709f5ebfbd10e30769683e7eb37
expected_slice_subarray_paths=173
expected_slice_subarray_manifest=a6f25c6d1af227a6f656284a2f3c833e4320caea80e7029fc376eb066e01584e
expected_slice_subarray_variants=346
expected_slice_subarray_keys=103222ebda62afb2a76d6b9efc6fefa0c086707509607f58a24b6a73a5f1cb1b
expected_with_to_reversed_candidate_paths=34
expected_with_to_reversed_candidate=e212ba0d3d9c819403d3d226f23a735ff2bb9b746618fff779e2654a39f5fddb
expected_with_to_reversed_candidate_variants=68
expected_with_to_reversed_candidate_keys=6d341ea9896a878f9beea36e477e96227642812a1cded595620a6de0f76e7723
expected_with_to_reversed_deferred_paths=0
expected_with_to_reversed_deferred=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
expected_with_to_reversed_deferred_variants=0
expected_with_to_reversed_deferred_keys=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
expected_with_to_reversed_paths=34
expected_with_to_reversed_manifest=e212ba0d3d9c819403d3d226f23a735ff2bb9b746618fff779e2654a39f5fddb
expected_with_to_reversed_variants=68
expected_with_to_reversed_keys=6d341ea9896a878f9beea36e477e96227642812a1cded595620a6de0f76e7723
expected_stringification_candidate_paths=88
expected_stringification_candidate=d968b61ff553acb2654f2904a9afff46660f43d6848ad7496ff28f18a81b8d4b
expected_stringification_candidate_variants=175
expected_stringification_candidate_keys=81131955a7d4ef4b2358965cd0691498bb78abfac7c48d0f60b8aafcdbbe81f1
expected_stringification_deferred_paths=5
expected_stringification_deferred=0254c5edb9969e43038d03dd42f9d43fd29c10c647673cd63cb4230bc8c53151
expected_stringification_deferred_variants=9
expected_stringification_deferred_keys=092d6f18a34c2dd23f7add4d9a73a5c1c14e63f99c6fd91f70c8a2c050edc44c
expected_stringification_paths=83
expected_stringification_manifest=ae64162fb7742828d9dc45d5f54e4666887c4ac95499bbfbe8622ae6fc875b89
expected_stringification_variants=166
expected_stringification_keys=0fe599bb568d384f84657000208d47df7b7ffa1d3133b6d2795abafa06bf00f6
expected_sort_candidate_paths=64
expected_sort_candidate=d06f1655781895a7f77a5ae378e25920e4cf62c87134a1cabaaa0418bfb8a0b8
expected_sort_candidate_variants=128
expected_sort_candidate_keys=53e35176074fdfdd0c414d30b9365995b0d420f43a2e45c420955cc0fc1d6de9
expected_sort_deferred_paths=6
expected_sort_deferred=0067268a56e709b6be94b51b1a7472b961a27f9a99e623a6cce6d04ed4cf1b96
expected_sort_deferred_variants=12
expected_sort_deferred_keys=f242add5304bef7ba11b82181cc1646b5a1ea970f06ee38d857d4c65f144ecfd
expected_sort_paths=58
expected_sort_manifest=1efa5ed5b57d0638963f183b0294e5dc90b711b754c63aa50b79cd34f3e0d3d4
expected_sort_variants=116
expected_sort_keys=b76f083344a23bdb330cdec16aa22f07175fb151f374858a77bbf3cc48e624c1
expected_entries_keys_candidate_paths=46
expected_entries_keys_candidate=45cfe102015cb7c25b3b2b064853c16c3e30d2f5c655bd3983a686689ca2540e
expected_entries_keys_candidate_variants=92
expected_entries_keys_candidate_keys=239f0a0f477d2d26f59b4247714e9dc2785bf5afac3adc8ea8a619067f299b4d
expected_entries_keys_deferred_paths=3
expected_entries_keys_deferred=bc0552a01cb1a8561461fe3bc6e82b3ed7a432599f16889a5c1e324552456a2d
expected_entries_keys_deferred_variants=6
expected_entries_keys_deferred_keys=4eb2eaecfec843385d2cb7562f278b17dae9d7cf20b61f8365f3fa734bc3b1c6
expected_entries_keys_paths=43
expected_entries_keys_manifest=029c249f88eb6a61f988495ea00e3455ca9878611e2c26e4c6b768faf0867d22
expected_entries_keys_variants=86
expected_entries_keys_keys=92e7b6ed05c315c3a6bc83e791e49a9880543247cf08bc5a329c6ffe0c2777ac
expected_of_candidate_paths=35
expected_of_candidate=6fdec16ab63ca0b1081a90f7a5f12fa6c87b6c73fdb209079d24bf793d2787b8
expected_of_candidate_variants=70
expected_of_candidate_keys=3bfcf9a16f2c28c819d121a819f7c52882e34fb3a3443ebb6c66db0bdbcc25a7
expected_of_deferred_paths=1
expected_of_deferred=2b66ebd26cc79b9df0d5e5771e665d164311633010ea66eb33a22e85d6d62a0e
expected_of_deferred_variants=2
expected_of_deferred_keys=07a640bcebe1fc380bde8bd0ab1a3b80779d4e45b085a744018a50858c016140
expected_of_paths=34
expected_of_manifest=01095b2e0348fb1328026684c7422975cf8396a08fa73719955c9350ee15f13f
expected_of_variants=68
expected_of_keys=8318904a86586b2bc771200348972ffd59c6f84b61219d84b262668517c363df
expected_from_candidate_paths=90
expected_from_candidate=87e7cfd69fbac9265f7e4a28ceaea8f21f053b7a587a95494becc7bbab61b20c
expected_from_candidate_variants=175
expected_from_candidate_keys=041fc07db938e2bf21fd1135fdbb3be648e2e5f3bdbf5688dfdf78784ed505a4
expected_from_deferred_paths=9
expected_from_deferred=7e466133fdeb876268cf10e629701daa332922d484d16ad76b58679aee3e47b6
expected_from_deferred_variants=17
expected_from_deferred_keys=df334b586f8ab8494ab8ec1d9a06d4492ae76b0fe0d73479637001f18ab3dd24
expected_from_paths=81
expected_from_manifest=a75d6ebea395327340d498c6f4d5e2b2c4224c039f6c1a58e42b19d070e94e41
expected_from_variants=158
expected_from_keys=5ea8a30f1578a6160441c068c91384ea635e179a90c6804af23730cfec7f6f34
expected_excluded_paths=148
expected_exclusions=0d425a326fc950257410849ada4c2435b410e84f4c9651f9393c39f6d5c3032a
expected_exclusions_file=4c79c3c86364a5c0aa6d2ea5bf3cba6da47261d0b4847fbfeaa5cd368749b783
expected_paths=2254
expected_variants=4463
expected_quickjs_variants=4463
expected_features=27
expected_features_hash=de5b9c5c6a66566a6b1481fc0b014a6ef00a95ebecc90c37da4508aa85a8d830
expected_includes=11
expected_includes_hash=b1b60b5e1f7635615ff31eb139d1803608e5743c5f46ca53fadc3797e0abe012
expected_manifest=91ac9a132c8099ecd15d3cfcfe160b21a1f7e9a083a5210a33406606270ad378
expected_keys=e8e3c0d8f19343bbf0160c5af3239caa98fb7e01d006ff6b53f0d946a500e7cc
expected_previous_global_profile=9b155f41c9c7541423c45b57da1bb805d6e7cf350ec7d6442d6700424afdbafc
expected_global_profile=99ad7997a6328ab24f87af9575f9e8ddda76db81092c008d5a84e06a84a0c5ee
expected_r3be_global_features=0208ceb83f737212fcd881dd43b95731d63196c3a1f7e3844d0c79ba1f9da0a8
expected_r3be_global_feature_count=80
expected_global_activation_paths=1865
expected_global_activation=44a9b901eb59f9dc41dde71e0595d2777f52814a864632e7e27bdd739654bdee
expected_global_activation_variants=3686
expected_global_activation_keys=68b01ca00423a3e62a090ee8cac24d54b5866276de306b0c846e74d3663218e5
expected_global_authenticated_paths=1824
expected_global_authenticated=b0c9f387fa32af126ce4fac0d84ffbb4e0b6876bd50a137c38ba9df2f6100fd4
expected_global_authenticated_variants=3606
expected_global_authenticated_keys=37a4623ced3162fe56e673ce24e9b532c1e171c00fc2deff6a75df5185fc2acb
expected_global_spillover_paths=41
expected_global_spillover=b85c1ab213028075ab4b9352eb8f939c1f39c345c7fee60847cdc5610a69412e
expected_global_spillover_variants=80
expected_global_spillover_keys=133d200dd4559dc869fbd7578ce6a948684dc80ae46fd5ac10491f80aad3d7ce
expected_global_activation_features=16
expected_global_activation_features_hash=47d6d7d8526717cf798fdf16b302dffcbdaf1d2f89af9875fabfff185e3498e2
expected_global_activation_includes=7
expected_global_activation_includes_hash=2ca4489c5ee986e70da369ed590d0d2c86963e84c6dcd7461edb4f9ec5d3a33b
expected_global_spillover_features=4
expected_global_spillover_features_hash=547bb058d5668040bec94843555efde0d00b924f827d738573449b4ad34ec28d
expected_global_spillover_includes=3
expected_global_spillover_includes_hash=d2f87c123dab82f2ef9f7c1824fb29732e188cc7110783c0b459d21350a5e593
expected_global_reason_only_paths=471
expected_global_reason_only=2c9273d1f8e3e793e519e6c5d09eb24ca7e65d798ae8827450931a403cdae2d9
expected_global_reason_only_variants=938
expected_global_reason_only_keys=6bb0f992e95e9f0c17d949c16a96675e61eeb297736cfdf0068551f6273b2999
expected_previous_typed_array_unsupported_paths=2336
expected_previous_typed_array_unsupported=2560741311d9fac8a5bf0b97a132a810a1e993270e4fcd65fa40155de1463b9b
expected_previous_typed_array_unsupported_variants=4624
expected_previous_typed_array_unsupported_keys=635b0f6190e77eb8e599eac245f58c17a88dcb3fd47bbde3bf9c0d3f186ab9db
# Provenance only: these identify the parent canonical artifacts from which
# the reason-only ledger was cut. The parent full A/B exact join is the
# transition proof; this scoped gate intentionally does not reopen full runs.
expected_previous_full_tsv=f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07
expected_previous_full_jsonl=8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390
expected_after_full_tsv=bdeb287ea6f74baefa0eb034773aa57f7c87f9ecaa6d2af20f27a6ea94b53693
expected_after_full_jsonl=916fbebcb964be779138ca6ad588d14b9cf3e55c0f22b4aaeb474739bdb74ece
expected_transition_physical_lines=4633
expected_transition_rows=4624
expected_transition=851ef0961a28532081f7b9dc281c305ea8839dd3b8ceed750d182da90b69eafd
expected_transition_data=26babcba92c23bb699f8fd3a2db7cce376fa868f5b3ca4081abc4148a90a4a57
expected_test_typed_array_harness=4c0e237804f39a4aa670f72c05b4520730c03c2d2e9f2f41e6b380bd6749ec61
expected_sm_typed_array_harness=3798d277ac8f105b65ad26602b500b497af7f3361fd14a169c58a601c605bb2e
expected_sm_math_harness=79dea1172236685567e09da8c9e868e0f84686bf40cff728785223c5b43f5e7b

usage() {
    cat <<'EOF'
usage: scripts/test-test262-typed-array-core.sh [--check]

With --check, rebuild and audit the frozen TypedArray candidate, mutation,
index/search, callback-find, every/some, forEach, reduce/reduceRight, and
map/filter, slice/subarray, with/toReversed, join/toLocaleString/toString
stringification, sort/toSorted, entries/keys iterator-contract, and static
`of`/`from` promotions, manifest, and exclusion ledger.
Verify all 4,749 candidate variants plus the 4,463 admitted variants against
pinned QuickJS, and audit the 3,686-row global `TypedArray` activation
partition plus its 938-row reason-only ledger.
With no option, also run the checksum-bound quickjs-oxide scoped and global
activation gates; that mode requires both measured all-green baseline files.
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

read_global_activation_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found++ }
        END { if (found != 1) exit 1 }
    ' "$global_activation_baseline"); then
        echo "error: TypedArray global activation baseline is missing exactly one $key entry: $global_activation_baseline" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: TypedArray global activation baseline contains an empty $key entry: $global_activation_baseline" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

expect_global_activation_value() {
    local key=$1 expected=$2 actual
    actual=$(read_global_activation_value "$key")
    if [[ "$actual" != "$expected" ]]; then
        echo "error: TypedArray global activation baseline $key drifted" >&2
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

read_report_header() {
    local report_file=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report_file"
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

profile_section_from_file() {
    local profile_file=$1 section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile_file"
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

legacy_spillover_paths() {
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

global_activation_spillover_paths() {
    cat <<'EOF'
test/built-ins/ArrayIteratorPrototype/next/Float32Array.js
test/built-ins/ArrayIteratorPrototype/next/Float64Array.js
test/built-ins/ArrayIteratorPrototype/next/Int16Array.js
test/built-ins/ArrayIteratorPrototype/next/Int32Array.js
test/built-ins/ArrayIteratorPrototype/next/Int8Array.js
test/built-ins/ArrayIteratorPrototype/next/Uint16Array.js
test/built-ins/ArrayIteratorPrototype/next/Uint32Array.js
test/built-ins/ArrayIteratorPrototype/next/Uint8Array.js
test/built-ins/ArrayIteratorPrototype/next/Uint8ClampedArray.js
test/built-ins/ArrayIteratorPrototype/next/detach-typedarray-in-progress.js
test/harness/testTypedArray-conversions-call-error.js
test/harness/testTypedArray-conversions.js
test/harness/testTypedArray.js
test/language/expressions/class/subclass-builtins/subclass-ArrayBuffer.js
test/language/expressions/class/subclass-builtins/subclass-BigInt64Array.js
test/language/expressions/class/subclass-builtins/subclass-BigUint64Array.js
test/language/statements/class/subclass-builtins/subclass-ArrayBuffer.js
test/language/statements/class/subclass-builtins/subclass-BigInt64Array.js
test/language/statements/class/subclass-builtins/subclass-BigUint64Array.js
test/language/statements/class/subclass/builtin-objects/TypedArray/regular-subclassing.js
test/language/statements/class/subclass/builtin-objects/TypedArray/super-must-be-called.js
test/language/statements/for-of/float32array-mutate.js
test/language/statements/for-of/float32array.js
test/language/statements/for-of/float64array-mutate.js
test/language/statements/for-of/float64array.js
test/language/statements/for-of/int16array-mutate.js
test/language/statements/for-of/int16array.js
test/language/statements/for-of/int32array-mutate.js
test/language/statements/for-of/int32array.js
test/language/statements/for-of/int8array-mutate.js
test/language/statements/for-of/int8array.js
test/language/statements/for-of/uint16array-mutate.js
test/language/statements/for-of/uint16array.js
test/language/statements/for-of/uint32array-mutate.js
test/language/statements/for-of/uint32array.js
test/language/statements/for-of/uint8array-mutate.js
test/language/statements/for-of/uint8array.js
test/language/statements/for-of/uint8clampedarray-mutate.js
test/language/statements/for-of/uint8clampedarray.js
test/language/statements/with/set-mutable-binding-binding-deleted-with-typed-array-in-proto-chain-strict-mode.js
test/language/statements/with/set-mutable-binding-binding-deleted-with-typed-array-in-proto-chain.js
EOF
}

spillover_paths() {
    {
        legacy_spillover_paths
        global_activation_spillover_paths
    } | LC_ALL=C sort
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

slice_subarray_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/slice/*|\
        test/built-ins/TypedArray/prototype/subarray/*|\
        test/built-ins/TypedArrayConstructors/prototype/slice/*|\
        test/built-ins/TypedArrayConstructors/prototype/subarray/*|\
        test/built-ins/TypedArrayConstructors/internals/HasProperty/BigInt/inherited-property.js|\
        test/built-ins/TypedArrayConstructors/internals/HasProperty/inherited-property.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/BigInt/integer-indexes-and-string-and-symbol-keys-.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/BigInt/integer-indexes-and-string-keys.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/BigInt/integer-indexes.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/integer-indexes-and-string-and-symbol-keys-.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/integer-indexes-and-string-keys.js|\
        test/built-ins/TypedArrayConstructors/internals/OwnPropertyKeys/integer-indexes.js|\
        test/staging/sm/TypedArray/slice-bitwise-same.js|\
        test/staging/sm/TypedArray/slice-conversion.js|\
        test/staging/sm/TypedArray/slice-detached.js|\
        test/staging/sm/TypedArray/slice-memcpy.js|\
        test/staging/sm/TypedArray/slice-species.js|\
        test/staging/sm/TypedArray/slice.js|\
        test/staging/sm/TypedArray/subarray-species.js|\
        test/staging/sm/TypedArray/subarray.js)
            return 0
            ;;
    esac
    return 1
}

slice_subarray_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/slice-bitwise-same.js|\
        test/staging/sm/TypedArray/slice-memcpy.js|\
        test/staging/sm/TypedArray/slice.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray slice realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        test/staging/sm/TypedArray/slice-species.js|\
        test/staging/sm/TypedArray/subarray.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray slice/subarray WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

with_to_reversed_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/toReversed/*|\
        test/built-ins/TypedArray/prototype/with/*|\
        test/staging/sm/TypedArray/toReversed-detached.js|\
        test/staging/sm/TypedArray/with-detached.js|\
        test/staging/sm/TypedArray/with.js)
            return 0
            ;;
    esac
    return 1
}

stringification_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/join/*|\
        test/built-ins/TypedArray/prototype/toLocaleString/*|\
        test/built-ins/TypedArray/prototype/toString.js|\
        test/built-ins/TypedArray/prototype/toString/*|\
        test/built-ins/TypedArray/prototype/set/BigInt/array-arg-set-values-in-order.js|\
        test/built-ins/TypedArray/prototype/set/array-arg-set-values-in-order.js|\
        test/built-ins/TypedArrayConstructors/prototype/join/*|\
        test/built-ins/TypedArrayConstructors/prototype/toLocaleString/*|\
        test/built-ins/TypedArrayConstructors/prototype/toString/*|\
        test/staging/sm/TypedArray/join.js|\
        test/staging/sm/TypedArray/toLocaleString-detached.js|\
        test/staging/sm/TypedArray/toLocaleString-nointl.js|\
        test/staging/sm/TypedArray/toLocaleString.js|\
        test/staging/sm/TypedArray/toString.js)
            return 0
            ;;
    esac
    return 1
}

stringification_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/join.js|\
        test/staging/sm/TypedArray/toLocaleString.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray stringification realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        test/staging/sm/TypedArray/toLocaleString-detached.js|\
        test/staging/sm/TypedArray/toLocaleString-nointl.js|\
        test/staging/sm/TypedArray/toString.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray stringification WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

sort_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/sort/*|\
        test/built-ins/TypedArray/prototype/toSorted/*|\
        test/built-ins/TypedArrayConstructors/prototype/sort/*|\
        test/staging/sm/TypedArray/sort*.js|\
        test/staging/sm/TypedArray/sorting_buffer_access.js|\
        test/staging/sm/TypedArray/toSorted-detached.js)
            return 0
            ;;
    esac
    return 1
}

sort_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/sort-negative-nan.js|\
        test/staging/sm/TypedArray/sort_byteoffset.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray sort realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        test/staging/sm/TypedArray/sort_errors.js|\
        test/staging/sm/TypedArray/sort_globals.js)
            if ! grep -Fq '$262.createRealm' "$source_file"; then
                echo "error: TypedArray sort realm dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        test/staging/sm/TypedArray/sort_large_countingsort.js|\
        test/staging/sm/TypedArray/sorting_buffer_access.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray sort WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

entries_keys_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/prototype/entries/*|\
        test/built-ins/TypedArray/prototype/keys/*|\
        test/built-ins/TypedArrayConstructors/prototype/entries/*|\
        test/built-ins/TypedArrayConstructors/prototype/keys/*|\
        test/staging/sm/TypedArray/detached-array-buffer-checks.js|\
        test/staging/sm/TypedArray/entries.js|\
        test/staging/sm/TypedArray/keys.js|\
        test/staging/sm/TypedArray/prototype-constructor-identity.js)
            return 0
            ;;
    esac
    return 1
}

entries_keys_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/entries.js|\
        test/staging/sm/TypedArray/keys.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray entries/keys realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        test/staging/sm/TypedArray/prototype-constructor-identity.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js" \
                || ! grep -Fq 'if (ctor === Uint8Array)' "$source_file" \
                || ! grep -Fq 'assert.sameValue(props.length, 6);' \
                    "$source_file"; then
                echo "error: TypedArray prototype identity WeakMap or Uint8 codec dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:WeakMap\n'
            ;;
        *)
            return 1
            ;;
    esac
}

of_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/built-ins/TypedArray/of/*|\
        test/built-ins/TypedArrayConstructors/of/*|\
        test/staging/sm/TypedArray/of.js)
            return 0
            ;;
    esac
    return 1
}

of_dependency_reason() {
    local test_path=$1 includes_file=$2 source_file=$3
    case "$test_path" in
        test/staging/sm/TypedArray/of.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq '$262.createRealm' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js"; then
                echo "error: TypedArray static of realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        *)
            return 1
            ;;
    esac
}

from_candidate_path() {
    local test_path=$1
    case "$test_path" in
        test/annexB/built-ins/TypedArrayConstructors/from/*|\
        test/built-ins/TypedArray/from/*|\
        test/built-ins/TypedArrayConstructors/from/*|\
        test/staging/sm/TypedArray/from_*.js)
            return 0
            ;;
    esac
    return 1
}

from_dependency_reason() {
    local test_path=$1 includes_file=$2 features_file=$3 source_file=$4
    case "$test_path" in
        test/annexB/built-ins/TypedArrayConstructors/from/iterator-method-emulates-undefined.js)
            if ! grep -Fxq IsHTMLDDA "$features_file" \
                || ! grep -Fq '$262.IsHTMLDDA' "$source_file"; then
                echo "error: TypedArray static from IsHTMLDDA dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:IsHTMLDDA\n'
            ;;
        test/staging/sm/TypedArray/from_realms.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq 'anyTypedArrayConstructors' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js" \
                || [[ "$(grep -Fc '$262.createRealm' "$source_file" || true)" != "2" ]]; then
                echo "error: TypedArray static from realm or WeakMap dependency drifted: $test_path" >&2
                return 2
            fi
            printf 'external:cross-realm\n'
            ;;
        test/staging/sm/TypedArray/from_basics.js|\
        test/staging/sm/TypedArray/from_constructor.js|\
        test/staging/sm/TypedArray/from_errors.js|\
        test/staging/sm/TypedArray/from_iterable.js|\
        test/staging/sm/TypedArray/from_mapping.js|\
        test/staging/sm/TypedArray/from_surfaces.js|\
        test/staging/sm/TypedArray/from_this.js)
            if ! grep -Fxq sm/non262-TypedArray-shell.js "$includes_file" \
                || ! grep -Fq 'anyTypedArrayConstructors' "$source_file" \
                || ! grep -Fq 'const sharedConstructors = new WeakMap();' \
                    "$suite/harness/sm/non262-TypedArray-shell.js" \
                || grep -Fq '$262.createRealm' "$source_file"; then
                echo "error: TypedArray static from WeakMap dependency drifted: $test_path" >&2
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
    local flag_count flag_list
    if grep -Evq '^(generated|noStrict|onlyStrict)$' "$flags_file" \
        || [[ -n "$(LC_ALL=C sort "$flags_file" | uniq -d)" ]]; then
        echo "error: TypedArray candidate gained unsupported variant flags: $test_path" >&2
        sed 's/^/  /' "$flags_file" >&2
        exit 1
    fi
    flag_count=$(grep -Evxc 'generated' "$flags_file" || true)
    flag_list=$(grep -Ev 'generated' "$flags_file" | tr '\n' ',' || true)
    case "$flag_count:$flag_list" in
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

append_reason_only_variant_keys() {
    local test_path=$1 flags_file=$2 output=$3
    local mode_count mode_list
    if grep -Evq \
        '^(generated|CanBlockIsTrue|CanBlockIsFalse|noStrict|onlyStrict)$' \
        "$flags_file" \
        || [[ -n "$(LC_ALL=C sort "$flags_file" | uniq -d)" ]] \
        || [[ "$(grep -Ec '^(CanBlockIsTrue|CanBlockIsFalse)$' \
            "$flags_file" || true)" -gt 1 ]]; then
        echo "error: TypedArray reason-only ledger gained unsupported or conflicting flags: $test_path" >&2
        sed 's/^/  /' "$flags_file" >&2
        exit 1
    fi
    mode_count=$(grep -Ec '^(noStrict|onlyStrict)$' "$flags_file" || true)
    mode_list=$(grep -E '^(noStrict|onlyStrict)$' "$flags_file" \
        | tr '\n' ',' || true)
    case "$mode_count:$mode_list" in
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
            echo "error: TypedArray reason-only ledger gained conflicting mode flags: $test_path" >&2
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

for required in \
    "$manifest" \
    "$profile" \
    "$exclusions" \
    "$current_global_profile" \
    "$r3be_global_features" \
    "$global_activation_manifest" \
    "$global_reason_only_manifest" \
    "$global_transition_receipt"
do
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
if [[ "$check_only" == false && ! -f "$global_activation_baseline" ]]; then
    echo "error: measured TypedArray global activation baseline is intentionally absent: $global_activation_baseline" >&2
    echo "error: run --check now; add the baseline only after an all-green Oxide run" >&2
    exit 1
fi
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer, found: $workers" >&2
    exit 2
fi

# The R3be activation receipt is historical evidence. Rebuild its exact parent
# by retaining the checked-in 80-feature inventory, so every later global
# admission is excluded without coupling this gate to a growing removal list.
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-typed-array-core.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
global_profile=$tmp_dir/test262-oxide-r3be.conf
awk '
    FNR == NR {
        if (NF && $0 !~ /^#/) retained[$0] = 1
        next
    }
    $0 == "[features]" {
        in_features = 1
        print
        next
    }
    /^\[/ {
        in_features = 0
        print
        next
    }
    in_features && NF && $0 !~ /^#/ {
        if ($0 in retained) print
        next
    }
    { print }
' "$r3be_global_features" "$current_global_profile" >"$global_profile"
r3be_global_feature_count=$(awk 'NF && $0 !~ /^#/ { count++ } END { print count + 0 }' \
    "$r3be_global_features")
if [[ "$(sha256_file "$r3be_global_features")" \
        != "$expected_r3be_global_features" \
    || "$r3be_global_feature_count" != "$expected_r3be_global_feature_count" \
    || "$(sha256_file "$global_profile")" != "$expected_global_profile" ]]; then
    echo "error: committed R3be inventory or derived global profile drifted" >&2
    exit 1
fi
awk 'NF && $0 !~ /^#/ { print }' "$r3be_global_features" | LC_ALL=C sort -c
diff -u \
    <(awk 'NF && $0 !~ /^#/ { print }' "$r3be_global_features") \
    <(profile_section_from_file "$global_profile" features)

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
    expect_value slice_subarray_candidate_paths "$expected_slice_subarray_candidate_paths"
    expect_value slice_subarray_candidate_sha256 "$expected_slice_subarray_candidate"
    expect_value slice_subarray_candidate_variants "$expected_slice_subarray_candidate_variants"
    expect_value slice_subarray_candidate_keys_sha256 "$expected_slice_subarray_candidate_keys"
    expect_value slice_subarray_deferred_paths "$expected_slice_subarray_deferred_paths"
    expect_value slice_subarray_deferred_sha256 "$expected_slice_subarray_deferred"
    expect_value slice_subarray_deferred_variants "$expected_slice_subarray_deferred_variants"
    expect_value slice_subarray_deferred_keys_sha256 "$expected_slice_subarray_deferred_keys"
    expect_value slice_subarray_paths "$expected_slice_subarray_paths"
    expect_value slice_subarray_manifest_sha256 "$expected_slice_subarray_manifest"
    expect_value slice_subarray_variants "$expected_slice_subarray_variants"
    expect_value slice_subarray_keys_sha256 "$expected_slice_subarray_keys"
    expect_value with_to_reversed_candidate_paths "$expected_with_to_reversed_candidate_paths"
    expect_value with_to_reversed_candidate_sha256 "$expected_with_to_reversed_candidate"
    expect_value with_to_reversed_candidate_variants "$expected_with_to_reversed_candidate_variants"
    expect_value with_to_reversed_candidate_keys_sha256 "$expected_with_to_reversed_candidate_keys"
    expect_value with_to_reversed_deferred_paths "$expected_with_to_reversed_deferred_paths"
    expect_value with_to_reversed_deferred_sha256 "$expected_with_to_reversed_deferred"
    expect_value with_to_reversed_deferred_variants "$expected_with_to_reversed_deferred_variants"
    expect_value with_to_reversed_deferred_keys_sha256 "$expected_with_to_reversed_deferred_keys"
    expect_value with_to_reversed_paths "$expected_with_to_reversed_paths"
    expect_value with_to_reversed_manifest_sha256 "$expected_with_to_reversed_manifest"
    expect_value with_to_reversed_variants "$expected_with_to_reversed_variants"
    expect_value with_to_reversed_keys_sha256 "$expected_with_to_reversed_keys"
    expect_value stringification_candidate_paths "$expected_stringification_candidate_paths"
    expect_value stringification_candidate_sha256 "$expected_stringification_candidate"
    expect_value stringification_candidate_variants "$expected_stringification_candidate_variants"
    expect_value stringification_candidate_keys_sha256 "$expected_stringification_candidate_keys"
    expect_value stringification_deferred_paths "$expected_stringification_deferred_paths"
    expect_value stringification_deferred_sha256 "$expected_stringification_deferred"
    expect_value stringification_deferred_variants "$expected_stringification_deferred_variants"
    expect_value stringification_deferred_keys_sha256 "$expected_stringification_deferred_keys"
    expect_value stringification_paths "$expected_stringification_paths"
    expect_value stringification_manifest_sha256 "$expected_stringification_manifest"
    expect_value stringification_variants "$expected_stringification_variants"
    expect_value stringification_keys_sha256 "$expected_stringification_keys"
    expect_value sort_candidate_paths "$expected_sort_candidate_paths"
    expect_value sort_candidate_sha256 "$expected_sort_candidate"
    expect_value sort_candidate_variants "$expected_sort_candidate_variants"
    expect_value sort_candidate_keys_sha256 "$expected_sort_candidate_keys"
    expect_value sort_deferred_paths "$expected_sort_deferred_paths"
    expect_value sort_deferred_sha256 "$expected_sort_deferred"
    expect_value sort_deferred_variants "$expected_sort_deferred_variants"
    expect_value sort_deferred_keys_sha256 "$expected_sort_deferred_keys"
    expect_value sort_paths "$expected_sort_paths"
    expect_value sort_manifest_sha256 "$expected_sort_manifest"
    expect_value sort_variants "$expected_sort_variants"
    expect_value sort_keys_sha256 "$expected_sort_keys"
    expect_value entries_keys_candidate_paths "$expected_entries_keys_candidate_paths"
    expect_value entries_keys_candidate_sha256 "$expected_entries_keys_candidate"
    expect_value entries_keys_candidate_variants "$expected_entries_keys_candidate_variants"
    expect_value entries_keys_candidate_keys_sha256 "$expected_entries_keys_candidate_keys"
    expect_value entries_keys_deferred_paths "$expected_entries_keys_deferred_paths"
    expect_value entries_keys_deferred_sha256 "$expected_entries_keys_deferred"
    expect_value entries_keys_deferred_variants "$expected_entries_keys_deferred_variants"
    expect_value entries_keys_deferred_keys_sha256 "$expected_entries_keys_deferred_keys"
    expect_value entries_keys_paths "$expected_entries_keys_paths"
    expect_value entries_keys_manifest_sha256 "$expected_entries_keys_manifest"
    expect_value entries_keys_variants "$expected_entries_keys_variants"
    expect_value entries_keys_keys_sha256 "$expected_entries_keys_keys"
    expect_value of_candidate_paths "$expected_of_candidate_paths"
    expect_value of_candidate_sha256 "$expected_of_candidate"
    expect_value of_candidate_variants "$expected_of_candidate_variants"
    expect_value of_candidate_keys_sha256 "$expected_of_candidate_keys"
    expect_value of_deferred_paths "$expected_of_deferred_paths"
    expect_value of_deferred_sha256 "$expected_of_deferred"
    expect_value of_deferred_variants "$expected_of_deferred_variants"
    expect_value of_deferred_keys_sha256 "$expected_of_deferred_keys"
    expect_value of_paths "$expected_of_paths"
    expect_value of_manifest_sha256 "$expected_of_manifest"
    expect_value of_variants "$expected_of_variants"
    expect_value of_keys_sha256 "$expected_of_keys"
    expect_value from_candidate_paths "$expected_from_candidate_paths"
    expect_value from_candidate_sha256 "$expected_from_candidate"
    expect_value from_candidate_variants "$expected_from_candidate_variants"
    expect_value from_candidate_keys_sha256 "$expected_from_candidate_keys"
    expect_value from_deferred_paths "$expected_from_deferred_paths"
    expect_value from_deferred_sha256 "$expected_from_deferred"
    expect_value from_deferred_variants "$expected_from_deferred_variants"
    expect_value from_deferred_keys_sha256 "$expected_from_deferred_keys"
    expect_value from_paths "$expected_from_paths"
    expect_value from_manifest_sha256 "$expected_from_manifest"
    expect_value from_variants "$expected_from_variants"
    expect_value from_keys_sha256 "$expected_from_keys"
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

    expect_global_activation_value quickjs "$expected_quickjs"
    expect_global_activation_value test262 "$expected_test262"
    expect_global_activation_value test262_patch_sha256 "$expected_patch"
    expect_global_activation_value test262_config_sha256 "$expected_config"
    expect_global_activation_value test262_metadata_sha256 "$expected_metadata"
    expect_global_activation_value previous_oxide_profile_sha256 \
        "$expected_previous_global_profile"
    expect_global_activation_value oxide_profile_sha256 "$expected_global_profile"
    expect_global_activation_value schema "$expected_schema"
    expect_global_activation_value mode "$expected_mode"
    expect_global_activation_value timeout_ms "$expected_timeout_ms"
    expect_global_activation_value external_excluded_paths "$expected_excluded_paths"
    expect_global_activation_value external_exclusions_sha256 "$expected_exclusions"
    expect_global_activation_value activation_paths "$expected_global_activation_paths"
    expect_global_activation_value activation_sha256 "$expected_global_activation"
    expect_global_activation_value activation_variants \
        "$expected_global_activation_variants"
    expect_global_activation_value activation_keys_sha256 \
        "$expected_global_activation_keys"
    expect_global_activation_value authenticated_paths \
        "$expected_global_authenticated_paths"
    expect_global_activation_value authenticated_sha256 \
        "$expected_global_authenticated"
    expect_global_activation_value authenticated_variants \
        "$expected_global_authenticated_variants"
    expect_global_activation_value authenticated_keys_sha256 \
        "$expected_global_authenticated_keys"
    expect_global_activation_value spillover_paths "$expected_global_spillover_paths"
    expect_global_activation_value spillover_sha256 "$expected_global_spillover"
    expect_global_activation_value spillover_variants \
        "$expected_global_spillover_variants"
    expect_global_activation_value spillover_keys_sha256 \
        "$expected_global_spillover_keys"
    expect_global_activation_value activation_features \
        "$expected_global_activation_features"
    expect_global_activation_value activation_features_sha256 \
        "$expected_global_activation_features_hash"
    expect_global_activation_value activation_includes \
        "$expected_global_activation_includes"
    expect_global_activation_value activation_includes_sha256 \
        "$expected_global_activation_includes_hash"
    expect_global_activation_value reason_only_paths \
        "$expected_global_reason_only_paths"
    expect_global_activation_value reason_only_sha256 \
        "$expected_global_reason_only"
    expect_global_activation_value reason_only_variants \
        "$expected_global_reason_only_variants"
    expect_global_activation_value reason_only_keys_sha256 \
        "$expected_global_reason_only_keys"
    expect_global_activation_value previous_typed_array_unsupported_paths \
        "$expected_previous_typed_array_unsupported_paths"
    expect_global_activation_value previous_typed_array_unsupported_sha256 \
        "$expected_previous_typed_array_unsupported"
    expect_global_activation_value previous_typed_array_unsupported_variants \
        "$expected_previous_typed_array_unsupported_variants"
    expect_global_activation_value previous_typed_array_unsupported_keys_sha256 \
        "$expected_previous_typed_array_unsupported_keys"
    expect_global_activation_value previous_full_tsv_sha256 \
        "$expected_previous_full_tsv"
    expect_global_activation_value previous_full_jsonl_sha256 \
        "$expected_previous_full_jsonl"
    expect_global_activation_value transition_rows "$expected_transition_rows"
    expect_global_activation_value transition_sha256 "$expected_transition"
    expect_global_activation_value transition_data_sha256 \
        "$expected_transition_data"
    expect_global_activation_value runnable "$expected_global_activation_variants"
    expect_global_activation_value reason_only_runnable 0
    expect_global_activation_value reason_only_unsupported \
        "$expected_global_reason_only_variants"
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
    || "$(sha256_file "$exclusions")" != "$expected_exclusions_file" \
    || "$(sha256_file "$r3be_global_features")" \
        != "$expected_r3be_global_features" \
    || "$(sha256_file "$global_profile")" != "$expected_global_profile" \
    || "$(sha256_file "$global_activation_manifest")" \
        != "$expected_global_activation" \
    || "$(sha256_file "$global_reason_only_manifest")" \
        != "$expected_global_reason_only" \
    || "$(awk '$0 != "TypedArray" { print }' "$global_profile" | sha256_stream)" \
        != "$expected_previous_global_profile" ]]; then
    echo "error: committed TypedArray core gate assets drifted" >&2
    exit 1
fi
expected_transition_header=$(printf '%s\n' \
    '# quickjs-oxide R3bd-to-R3be TypedArray global-admission transitions v1' \
    "# before_tsv_sha256=$expected_previous_full_tsv" \
    "# before_jsonl_sha256=$expected_previous_full_jsonl" \
    "# before_oxide_profile_sha256=$expected_previous_global_profile" \
    "# after_tsv_sha256=$expected_after_full_tsv" \
    "# after_jsonl_sha256=$expected_after_full_jsonl" \
    "# after_oxide_profile_sha256=$expected_global_profile" \
    "# schema=$expected_schema" \
    $'path\tvariant\tbefore_outcome\tafter_outcome\tbefore_detail\tafter_detail')
if [[ "$(sha256_file "$global_transition_receipt")" \
        != "$expected_transition" \
    || "$(wc -l <"$global_transition_receipt" | tr -d '[:space:]')" \
        != "$expected_transition_physical_lines" \
    || "$(sed -n '1,9p' "$global_transition_receipt")" \
        != "$expected_transition_header" ]]; then
    echo "error: TypedArray R3bd-to-R3be transition receipt header or file drifted" >&2
    exit 1
fi
if [[ "$(profile_section_from_file "$global_profile" features \
        | grep -Fxc TypedArray)" != "1" \
    || -n "$(profile_section_from_file "$global_profile" features \
        | LC_ALL=C sort | uniq -d)" ]]; then
    echo "error: global profile TypedArray admission or feature ordering drifted" >&2
    exit 1
fi

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
slice_subarray_candidate=$tmp_dir/slice-subarray-candidate.txt
slice_subarray_candidate_keys=$tmp_dir/slice-subarray-candidate-keys.txt
slice_subarray_deferred=$tmp_dir/slice-subarray-deferred.txt
slice_subarray_deferred_keys=$tmp_dir/slice-subarray-deferred-keys.txt
slice_subarray_manifest=$tmp_dir/slice-subarray-manifest.txt
slice_subarray_keys=$tmp_dir/slice-subarray-keys.txt
with_to_reversed_candidate=$tmp_dir/with-to-reversed-candidate.txt
with_to_reversed_candidate_keys=$tmp_dir/with-to-reversed-candidate-keys.txt
with_to_reversed_deferred=$tmp_dir/with-to-reversed-deferred.txt
with_to_reversed_deferred_keys=$tmp_dir/with-to-reversed-deferred-keys.txt
with_to_reversed_manifest=$tmp_dir/with-to-reversed-manifest.txt
with_to_reversed_keys=$tmp_dir/with-to-reversed-keys.txt
stringification_candidate=$tmp_dir/stringification-candidate.txt
stringification_candidate_keys=$tmp_dir/stringification-candidate-keys.txt
stringification_deferred=$tmp_dir/stringification-deferred.txt
stringification_deferred_keys=$tmp_dir/stringification-deferred-keys.txt
stringification_manifest=$tmp_dir/stringification-manifest.txt
stringification_keys=$tmp_dir/stringification-keys.txt
sort_candidate=$tmp_dir/sort-candidate.txt
sort_candidate_keys=$tmp_dir/sort-candidate-keys.txt
sort_deferred=$tmp_dir/sort-deferred.txt
sort_deferred_keys=$tmp_dir/sort-deferred-keys.txt
sort_manifest=$tmp_dir/sort-manifest.txt
sort_keys=$tmp_dir/sort-keys.txt
entries_keys_candidate=$tmp_dir/entries-keys-candidate.txt
entries_keys_candidate_keys=$tmp_dir/entries-keys-candidate-keys.txt
entries_keys_deferred=$tmp_dir/entries-keys-deferred.txt
entries_keys_deferred_keys=$tmp_dir/entries-keys-deferred-keys.txt
entries_keys_manifest=$tmp_dir/entries-keys-manifest.txt
entries_keys_keys=$tmp_dir/entries-keys-keys.txt
of_candidate=$tmp_dir/of-candidate.txt
of_candidate_keys=$tmp_dir/of-candidate-keys.txt
of_deferred=$tmp_dir/of-deferred.txt
of_deferred_keys=$tmp_dir/of-deferred-keys.txt
of_manifest=$tmp_dir/of-manifest.txt
of_keys=$tmp_dir/of-keys.txt
from_candidate=$tmp_dir/from-candidate.txt
from_candidate_keys=$tmp_dir/from-candidate-keys.txt
from_deferred=$tmp_dir/from-deferred.txt
from_deferred_keys=$tmp_dir/from-deferred-keys.txt
from_manifest=$tmp_dir/from-manifest.txt
from_keys=$tmp_dir/from-keys.txt
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
global_features=$tmp_dir/global-features.txt
global_activation_inventory=$tmp_dir/global-activation.txt
derived_global_activation=$tmp_dir/derived-global-activation.txt
global_activation_keys=$tmp_dir/global-activation-keys.txt
global_authenticated_inventory=$tmp_dir/global-authenticated.txt
global_authenticated_keys=$tmp_dir/global-authenticated-keys.txt
global_spillover_inventory=$tmp_dir/global-spillover.txt
global_spillover_keys=$tmp_dir/global-spillover-keys.txt
global_reason_only_inventory=$tmp_dir/global-reason-only.txt
global_reason_only_keys=$tmp_dir/global-reason-only-keys.txt
global_reason_only_flag_inventory=$tmp_dir/global-reason-only-flags.txt
global_reason_only_missing_features=$tmp_dir/global-reason-only-missing-features.txt
global_reason_only_missing_details=$tmp_dir/global-reason-only-missing-details.tsv
global_reason_only_expected_details=$tmp_dir/global-reason-only-expected-details.tsv
global_reason_only_actual_details=$tmp_dir/global-reason-only-actual-details.tsv
global_transition_data=$tmp_dir/global-transition-data.tsv
global_transition_activation_inventory=$tmp_dir/global-transition-activation.txt
global_transition_activation_keys=$tmp_dir/global-transition-activation-keys.txt
global_transition_reason_inventory=$tmp_dir/global-transition-reason.txt
global_transition_reason_keys=$tmp_dir/global-transition-reason-keys.txt
global_transition_all_inventory=$tmp_dir/global-transition-all.txt
global_transition_all_keys=$tmp_dir/global-transition-all-keys.txt
global_transition_reason_after_details=$tmp_dir/global-transition-reason-after-details.tsv
global_transition_after_expected=$tmp_dir/global-transition-after-expected.tsv
global_transition_after_actual=$tmp_dir/global-transition-after-actual.tsv
previous_typed_array_unsupported_inventory=$tmp_dir/previous-typed-array-unsupported.txt
previous_typed_array_unsupported_keys=$tmp_dir/previous-typed-array-unsupported-keys.txt
global_activation_feature_occurrences=$tmp_dir/global-activation-features.raw
global_activation_include_occurrences=$tmp_dir/global-activation-includes.raw
global_activation_feature_inventory=$tmp_dir/global-activation-features.txt
global_activation_include_inventory=$tmp_dir/global-activation-includes.txt
global_spillover_feature_occurrences=$tmp_dir/global-spillover-features.raw
global_spillover_include_occurrences=$tmp_dir/global-spillover-includes.raw
global_spillover_feature_inventory=$tmp_dir/global-spillover-features.txt
global_spillover_include_inventory=$tmp_dir/global-spillover-includes.txt
global_spillover_flag_inventory=$tmp_dir/global-spillover-flags.txt

awk -F'\t' '
    BEGIN { OFS="\t" }
    !/^#/ && !($1 == "path" && $2 == "variant") {
        print $1, $2, $3, $4, $5, $6
    }
' "$global_transition_receipt" >"$global_transition_data"
if [[ "$(wc -l <"$global_transition_data" | tr -d '[:space:]')" \
        != "$expected_transition_rows" \
    || "$(sha256_file "$global_transition_data")" \
        != "$expected_transition_data" ]]; then
    echo "error: TypedArray transition receipt data rows drifted" >&2
    exit 1
fi
if ! LC_ALL=C sort -c "$global_transition_data" \
    || ! awk -F'\t' \
        -v prefix="$reason_detail_prefix" \
        -v expected_rows="$expected_transition_rows" \
        -v expected_activation="$expected_global_activation_variants" \
        -v expected_reason="$expected_global_reason_only_variants" '
    function fail() {
        invalid=1
        exit 1
    }
    {
        if (NF != 6 || $1 == "" ||
            ($2 != "sloppy" && $2 != "strict") ||
            $3 != "unsupported-feature") fail()
        key=$1 SUBSEP $2
        if (seen_key[key]++) fail()
        if ($4 == "pass") {
            activation++
            if ($5 != prefix "TypedArray" || $6 != "") fail()
            next
        }
        if ($4 != "unsupported-feature" ||
            index($5, prefix) != 1 ||
            index($6, prefix) != 1) fail()
        reason++
        before=substr($5, length(prefix) + 1)
        token_count=split(before, tokens, /, /)
        if (token_count < 2) fail()
        typed_array_count=0
        remaining=""
        delete seen_token
        for (i=1; i <= token_count; i++) {
            token=tokens[i]
            if (token == "" ||
                token !~ /^[^,[:space:]]+$/ ||
                seen_token[token]++) fail()
            if (token == "TypedArray") {
                typed_array_count++
            } else {
                if (remaining != "") remaining=remaining ", "
                remaining=remaining token
            }
        }
        if (typed_array_count != 1 ||
            remaining == "" ||
            $6 != prefix remaining) fail()
    }
    END {
        if (invalid ||
            NR != expected_rows ||
            activation != expected_activation ||
            reason != expected_reason) exit 1
    }
' "$global_transition_data"; then
    echo "error: TypedArray transition receipt ordering, keys, or semantics drifted" >&2
    exit 1
fi
awk -F'\t' '$4 == "pass" { print $1 }' \
    "$global_transition_data" \
    | LC_ALL=C sort -u >"$global_transition_activation_inventory"
awk -F'\t' '$4 == "pass" { print $1 "\t" $2 }' \
    "$global_transition_data" \
    | LC_ALL=C sort >"$global_transition_activation_keys"
awk -F'\t' '$4 == "unsupported-feature" { print $1 }' \
    "$global_transition_data" \
    | LC_ALL=C sort -u >"$global_transition_reason_inventory"
awk -F'\t' '$4 == "unsupported-feature" { print $1 "\t" $2 }' \
    "$global_transition_data" \
    | LC_ALL=C sort >"$global_transition_reason_keys"
awk -F'\t' '{ print $1 }' "$global_transition_data" \
    | LC_ALL=C sort -u >"$global_transition_all_inventory"
awk -F'\t' '{ print $1 "\t" $2 }' "$global_transition_data" \
    | LC_ALL=C sort >"$global_transition_all_keys"
awk -F'\t' '$4 == "unsupported-feature" {
    print $1 "\t" $2 "\t" $6
}' "$global_transition_data" \
    | LC_ALL=C sort >"$global_transition_reason_after_details"
awk -F'\t' '{ print $1 "\t" $2 "\t" $4 "\t" $6 }' \
    "$global_transition_data" \
    | LC_ALL=C sort >"$global_transition_after_expected"
if [[ "$(wc -l <"$global_transition_activation_inventory" \
        | tr -d '[:space:]')" != "$expected_global_activation_paths" \
    || "$(sha256_file "$global_transition_activation_inventory")" \
        != "$expected_global_activation" \
    || "$(wc -l <"$global_transition_activation_keys" \
        | tr -d '[:space:]')" != "$expected_global_activation_variants" \
    || "$(sha256_file "$global_transition_activation_keys")" \
        != "$expected_global_activation_keys" \
    || "$(wc -l <"$global_transition_reason_inventory" \
        | tr -d '[:space:]')" != "$expected_global_reason_only_paths" \
    || "$(sha256_file "$global_transition_reason_inventory")" \
        != "$expected_global_reason_only" \
    || "$(wc -l <"$global_transition_reason_keys" \
        | tr -d '[:space:]')" != "$expected_global_reason_only_variants" \
    || "$(sha256_file "$global_transition_reason_keys")" \
        != "$expected_global_reason_only_keys" \
    || "$(wc -l <"$global_transition_all_inventory" \
        | tr -d '[:space:]')" \
        != "$expected_previous_typed_array_unsupported_paths" \
    || "$(sha256_file "$global_transition_all_inventory")" \
        != "$expected_previous_typed_array_unsupported" \
    || "$(wc -l <"$global_transition_all_keys" | tr -d '[:space:]')" \
        != "$expected_previous_typed_array_unsupported_variants" \
    || "$(sha256_file "$global_transition_all_keys")" \
        != "$expected_previous_typed_array_unsupported_keys" ]]; then
    echo "error: TypedArray transition receipt partition inventory drifted" >&2
    exit 1
fi

manifest_paths >"$manifest_inventory"
exclusion_paths >"$excluded_inventory"
spillover_paths >"$spillover_inventory"
awk 'NF && $1 !~ /^#/ { print }' \
    "$global_activation_manifest" >"$global_activation_inventory"
awk 'NF && $1 !~ /^#/ { print }' \
    "$global_reason_only_manifest" >"$global_reason_only_inventory"
global_activation_spillover_paths >"$global_spillover_inventory"
profile_section_from_file "$global_profile" features >"$global_features"
LC_ALL=C sort -c "$manifest_inventory"
LC_ALL=C sort -c "$excluded_inventory"
LC_ALL=C sort -c "$spillover_inventory"
LC_ALL=C sort -c "$global_activation_inventory"
LC_ALL=C sort -c "$global_reason_only_inventory"
LC_ALL=C sort -c "$global_spillover_inventory"
LC_ALL=C sort -c "$global_features"

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
        if (NR != 149 ||
            counts["dependency:join"] != 0 ||
            counts["external:cross-realm"] != 54 ||
            counts["external:SharedArrayBuffer"] != 71 ||
            counts["external:WeakMap"] != 21 ||
            counts["external:Math"] != 1 ||
            counts["external:IsHTMLDDA"] != 1 ||
            counts["static:from"] != 0 ||
            counts["static:of"] != 0 ||
            counts["method:iterator-entries-keys"] != 0 ||
            counts["method:mutation-copy-set"] != 0 ||
            counts["method:search-predicate"] != 0 ||
            counts["method:species-copy-transform"] != 0 ||
            counts["method:callback-reduce"] != 0 ||
            counts["method:sort"] != 0 ||
            counts["method:stringification"] != 0 ||
            counts["method:subarray"] != 0 ||
            counts["method:full-prototype-contract"] != 0) {
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
: >"$slice_subarray_candidate"
: >"$slice_subarray_deferred"
: >"$slice_subarray_manifest"
: >"$with_to_reversed_candidate"
: >"$with_to_reversed_deferred"
: >"$with_to_reversed_manifest"
: >"$stringification_candidate"
: >"$stringification_deferred"
: >"$stringification_manifest"
: >"$sort_candidate"
: >"$sort_deferred"
: >"$sort_manifest"
: >"$entries_keys_candidate"
: >"$entries_keys_deferred"
: >"$entries_keys_manifest"
: >"$of_candidate"
: >"$of_deferred"
: >"$of_manifest"
: >"$from_candidate"
: >"$from_deferred"
: >"$from_manifest"
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
    if slice_subarray_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$slice_subarray_candidate"
        if reason=$(slice_subarray_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$slice_subarray_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
    fi
    if with_to_reversed_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$with_to_reversed_candidate"
    fi
    if stringification_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$stringification_candidate"
        if reason=$(stringification_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$stringification_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
        case "$test_path" in
            test/built-ins/TypedArray/prototype/set/BigInt/array-arg-set-values-in-order.js|\
            test/built-ins/TypedArray/prototype/set/array-arg-set-values-in-order.js)
                ;;
            *)
                printf '%s\n' "$test_path" >>"$derived_manifest"
                printf '%s\n' "$test_path" >>"$stringification_manifest"
                continue
                ;;
        esac
    fi
    if sort_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$sort_candidate"
        if reason=$(sort_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$sort_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$sort_manifest"
        continue
    fi
    if entries_keys_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$entries_keys_candidate"
        if reason=$(entries_keys_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$entries_keys_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$entries_keys_manifest"
        continue
    fi
    if of_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$of_candidate"
        if reason=$(of_dependency_reason \
            "$test_path" "$candidate_includes" "$source_file"); then
            printf '%s\n' "$test_path" >>"$of_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$of_manifest"
        continue
    fi
    if from_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$from_candidate"
        if reason=$(from_dependency_reason \
            "$test_path" "$candidate_includes" "$candidate_features" "$source_file"); then
            printf '%s\n' "$test_path" >>"$from_deferred"
            printf '%s\t%s\n' "$test_path" "$reason" >>"$derived_exclusion_rows"
            continue
        else
            dependency_status=$?
            if [[ "$dependency_status" != "1" ]]; then
                exit 1
            fi
        fi
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$from_manifest"
        continue
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
        if [[ "$reason" == "method:subarray" ]] \
            && slice_subarray_candidate_path "$test_path"; then
            printf '%s\n' "$test_path" >>"$derived_manifest"
            printf '%s\n' "$test_path" >>"$slice_subarray_manifest"
            continue
        fi
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
            if [[ "$reason" == "dependency:join" ]] \
                && stringification_candidate_path "$test_path"; then
                printf '%s\n' "$test_path" >>"$derived_manifest"
                printf '%s\n' "$test_path" >>"$stringification_manifest"
                continue
            fi
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
    elif [[ "$reason" == "method:species-copy-transform" ]] \
        && slice_subarray_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$slice_subarray_manifest"
        continue
    elif [[ "$reason" == "method:species-copy-transform" ]] \
        && with_to_reversed_candidate_path "$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_manifest"
        printf '%s\n' "$test_path" >>"$with_to_reversed_manifest"
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

LC_ALL=C sort -o "$slice_subarray_candidate" "$slice_subarray_candidate"
LC_ALL=C sort -o "$slice_subarray_deferred" "$slice_subarray_deferred"
LC_ALL=C sort -o "$slice_subarray_manifest" "$slice_subarray_manifest"
diff -u \
    "$slice_subarray_candidate" \
    <(LC_ALL=C sort -u "$slice_subarray_manifest" "$slice_subarray_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$slice_subarray_manifest" "$slice_subarray_deferred")" ]]; then
    echo "error: TypedArray slice/subarray manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$slice_subarray_candidate_keys"
: >"$slice_subarray_deferred_keys"
: >"$slice_subarray_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$slice_subarray_candidate_keys"
done <"$slice_subarray_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$slice_subarray_deferred_keys"
done <"$slice_subarray_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$slice_subarray_keys"
done <"$slice_subarray_manifest"
LC_ALL=C sort -o "$slice_subarray_candidate_keys" \
    "$slice_subarray_candidate_keys"
LC_ALL=C sort -o "$slice_subarray_deferred_keys" \
    "$slice_subarray_deferred_keys"
LC_ALL=C sort -o "$slice_subarray_keys" "$slice_subarray_keys"
if [[ "$(wc -l <"$slice_subarray_candidate" | tr -d '[:space:]')" \
        != "$expected_slice_subarray_candidate_paths" \
    || "$(sha256_file "$slice_subarray_candidate")" \
        != "$expected_slice_subarray_candidate" \
    || "$(wc -l <"$slice_subarray_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_slice_subarray_candidate_variants" \
    || "$(sha256_file "$slice_subarray_candidate_keys")" \
        != "$expected_slice_subarray_candidate_keys" \
    || "$(wc -l <"$slice_subarray_deferred" | tr -d '[:space:]')" \
        != "$expected_slice_subarray_deferred_paths" \
    || "$(sha256_file "$slice_subarray_deferred")" \
        != "$expected_slice_subarray_deferred" \
    || "$(wc -l <"$slice_subarray_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_slice_subarray_deferred_variants" \
    || "$(sha256_file "$slice_subarray_deferred_keys")" \
        != "$expected_slice_subarray_deferred_keys" \
    || "$(wc -l <"$slice_subarray_manifest" | tr -d '[:space:]')" \
        != "$expected_slice_subarray_paths" \
    || "$(sha256_file "$slice_subarray_manifest")" \
        != "$expected_slice_subarray_manifest" \
    || "$(wc -l <"$slice_subarray_keys" | tr -d '[:space:]')" \
        != "$expected_slice_subarray_variants" \
    || "$(sha256_file "$slice_subarray_keys")" \
        != "$expected_slice_subarray_keys" ]]; then
    echo "error: TypedArray slice/subarray promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$with_to_reversed_candidate" "$with_to_reversed_candidate"
LC_ALL=C sort -o "$with_to_reversed_deferred" "$with_to_reversed_deferred"
LC_ALL=C sort -o "$with_to_reversed_manifest" "$with_to_reversed_manifest"
diff -u \
    "$with_to_reversed_candidate" \
    <(LC_ALL=C sort -u \
        "$with_to_reversed_manifest" "$with_to_reversed_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$with_to_reversed_manifest" "$with_to_reversed_deferred")" ]]; then
    echo "error: TypedArray with/toReversed manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$with_to_reversed_candidate_keys"
: >"$with_to_reversed_deferred_keys"
: >"$with_to_reversed_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$with_to_reversed_candidate_keys"
done <"$with_to_reversed_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$with_to_reversed_deferred_keys"
done <"$with_to_reversed_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$with_to_reversed_keys"
done <"$with_to_reversed_manifest"
LC_ALL=C sort -o "$with_to_reversed_candidate_keys" \
    "$with_to_reversed_candidate_keys"
LC_ALL=C sort -o "$with_to_reversed_deferred_keys" \
    "$with_to_reversed_deferred_keys"
LC_ALL=C sort -o "$with_to_reversed_keys" "$with_to_reversed_keys"
if [[ "$(wc -l <"$with_to_reversed_candidate" | tr -d '[:space:]')" \
        != "$expected_with_to_reversed_candidate_paths" \
    || "$(sha256_file "$with_to_reversed_candidate")" \
        != "$expected_with_to_reversed_candidate" \
    || "$(wc -l <"$with_to_reversed_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_with_to_reversed_candidate_variants" \
    || "$(sha256_file "$with_to_reversed_candidate_keys")" \
        != "$expected_with_to_reversed_candidate_keys" \
    || "$(wc -l <"$with_to_reversed_deferred" | tr -d '[:space:]')" \
        != "$expected_with_to_reversed_deferred_paths" \
    || "$(sha256_file "$with_to_reversed_deferred")" \
        != "$expected_with_to_reversed_deferred" \
    || "$(wc -l <"$with_to_reversed_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_with_to_reversed_deferred_variants" \
    || "$(sha256_file "$with_to_reversed_deferred_keys")" \
        != "$expected_with_to_reversed_deferred_keys" \
    || "$(wc -l <"$with_to_reversed_manifest" | tr -d '[:space:]')" \
        != "$expected_with_to_reversed_paths" \
    || "$(sha256_file "$with_to_reversed_manifest")" \
        != "$expected_with_to_reversed_manifest" \
    || "$(wc -l <"$with_to_reversed_keys" | tr -d '[:space:]')" \
        != "$expected_with_to_reversed_variants" \
    || "$(sha256_file "$with_to_reversed_keys")" \
        != "$expected_with_to_reversed_keys" ]]; then
    echo "error: TypedArray with/toReversed promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$stringification_candidate" "$stringification_candidate"
LC_ALL=C sort -o "$stringification_deferred" "$stringification_deferred"
LC_ALL=C sort -o "$stringification_manifest" "$stringification_manifest"
diff -u \
    "$stringification_candidate" \
    <(LC_ALL=C sort -u \
        "$stringification_manifest" "$stringification_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$stringification_manifest" "$stringification_deferred")" ]]; then
    echo "error: TypedArray stringification manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$stringification_candidate_keys"
: >"$stringification_deferred_keys"
: >"$stringification_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$stringification_candidate_keys"
done <"$stringification_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$stringification_deferred_keys"
done <"$stringification_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$stringification_keys"
done <"$stringification_manifest"
LC_ALL=C sort -o "$stringification_candidate_keys" \
    "$stringification_candidate_keys"
LC_ALL=C sort -o "$stringification_deferred_keys" \
    "$stringification_deferred_keys"
LC_ALL=C sort -o "$stringification_keys" "$stringification_keys"
if [[ "$(wc -l <"$stringification_candidate" | tr -d '[:space:]')" \
        != "$expected_stringification_candidate_paths" \
    || "$(sha256_file "$stringification_candidate")" \
        != "$expected_stringification_candidate" \
    || "$(wc -l <"$stringification_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_stringification_candidate_variants" \
    || "$(sha256_file "$stringification_candidate_keys")" \
        != "$expected_stringification_candidate_keys" \
    || "$(wc -l <"$stringification_deferred" | tr -d '[:space:]')" \
        != "$expected_stringification_deferred_paths" \
    || "$(sha256_file "$stringification_deferred")" \
        != "$expected_stringification_deferred" \
    || "$(wc -l <"$stringification_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_stringification_deferred_variants" \
    || "$(sha256_file "$stringification_deferred_keys")" \
        != "$expected_stringification_deferred_keys" \
    || "$(wc -l <"$stringification_manifest" | tr -d '[:space:]')" \
        != "$expected_stringification_paths" \
    || "$(sha256_file "$stringification_manifest")" \
        != "$expected_stringification_manifest" \
    || "$(wc -l <"$stringification_keys" | tr -d '[:space:]')" \
        != "$expected_stringification_variants" \
    || "$(sha256_file "$stringification_keys")" \
        != "$expected_stringification_keys" ]]; then
    echo "error: TypedArray stringification promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$sort_candidate" "$sort_candidate"
LC_ALL=C sort -o "$sort_deferred" "$sort_deferred"
LC_ALL=C sort -o "$sort_manifest" "$sort_manifest"
diff -u \
    "$sort_candidate" \
    <(LC_ALL=C sort -u "$sort_manifest" "$sort_deferred")
if [[ -n "$(LC_ALL=C comm -12 "$sort_manifest" "$sort_deferred")" ]]; then
    echo "error: TypedArray sort/toSorted manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$sort_candidate_keys"
: >"$sort_deferred_keys"
: >"$sort_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$sort_candidate_keys"
done <"$sort_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$sort_deferred_keys"
done <"$sort_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$sort_keys"
done <"$sort_manifest"
LC_ALL=C sort -o "$sort_candidate_keys" "$sort_candidate_keys"
LC_ALL=C sort -o "$sort_deferred_keys" "$sort_deferred_keys"
LC_ALL=C sort -o "$sort_keys" "$sort_keys"
if [[ "$(wc -l <"$sort_candidate" | tr -d '[:space:]')" \
        != "$expected_sort_candidate_paths" \
    || "$(sha256_file "$sort_candidate")" != "$expected_sort_candidate" \
    || "$(wc -l <"$sort_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_sort_candidate_variants" \
    || "$(sha256_file "$sort_candidate_keys")" \
        != "$expected_sort_candidate_keys" \
    || "$(wc -l <"$sort_deferred" | tr -d '[:space:]')" \
        != "$expected_sort_deferred_paths" \
    || "$(sha256_file "$sort_deferred")" != "$expected_sort_deferred" \
    || "$(wc -l <"$sort_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_sort_deferred_variants" \
    || "$(sha256_file "$sort_deferred_keys")" \
        != "$expected_sort_deferred_keys" \
    || "$(wc -l <"$sort_manifest" | tr -d '[:space:]')" \
        != "$expected_sort_paths" \
    || "$(sha256_file "$sort_manifest")" != "$expected_sort_manifest" \
    || "$(wc -l <"$sort_keys" | tr -d '[:space:]')" \
        != "$expected_sort_variants" \
    || "$(sha256_file "$sort_keys")" != "$expected_sort_keys" ]]; then
    echo "error: TypedArray sort/toSorted promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$entries_keys_candidate" "$entries_keys_candidate"
LC_ALL=C sort -o "$entries_keys_deferred" "$entries_keys_deferred"
LC_ALL=C sort -o "$entries_keys_manifest" "$entries_keys_manifest"
diff -u \
    "$entries_keys_candidate" \
    <(LC_ALL=C sort -u "$entries_keys_manifest" "$entries_keys_deferred")
if [[ -n "$(LC_ALL=C comm -12 \
    "$entries_keys_manifest" "$entries_keys_deferred")" ]]; then
    echo "error: TypedArray entries/keys manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$entries_keys_candidate_keys"
: >"$entries_keys_deferred_keys"
: >"$entries_keys_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$entries_keys_candidate_keys"
done <"$entries_keys_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$entries_keys_deferred_keys"
done <"$entries_keys_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$entries_keys_keys"
done <"$entries_keys_manifest"
LC_ALL=C sort -o "$entries_keys_candidate_keys" "$entries_keys_candidate_keys"
LC_ALL=C sort -o "$entries_keys_deferred_keys" "$entries_keys_deferred_keys"
LC_ALL=C sort -o "$entries_keys_keys" "$entries_keys_keys"
if [[ "$(wc -l <"$entries_keys_candidate" | tr -d '[:space:]')" \
        != "$expected_entries_keys_candidate_paths" \
    || "$(sha256_file "$entries_keys_candidate")" \
        != "$expected_entries_keys_candidate" \
    || "$(wc -l <"$entries_keys_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_entries_keys_candidate_variants" \
    || "$(sha256_file "$entries_keys_candidate_keys")" \
        != "$expected_entries_keys_candidate_keys" \
    || "$(wc -l <"$entries_keys_deferred" | tr -d '[:space:]')" \
        != "$expected_entries_keys_deferred_paths" \
    || "$(sha256_file "$entries_keys_deferred")" \
        != "$expected_entries_keys_deferred" \
    || "$(wc -l <"$entries_keys_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_entries_keys_deferred_variants" \
    || "$(sha256_file "$entries_keys_deferred_keys")" \
        != "$expected_entries_keys_deferred_keys" \
    || "$(wc -l <"$entries_keys_manifest" | tr -d '[:space:]')" \
        != "$expected_entries_keys_paths" \
    || "$(sha256_file "$entries_keys_manifest")" \
        != "$expected_entries_keys_manifest" \
    || "$(wc -l <"$entries_keys_keys" | tr -d '[:space:]')" \
        != "$expected_entries_keys_variants" \
    || "$(sha256_file "$entries_keys_keys")" \
        != "$expected_entries_keys_keys" ]]; then
    echo "error: TypedArray entries/keys promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$of_candidate" "$of_candidate"
LC_ALL=C sort -o "$of_deferred" "$of_deferred"
LC_ALL=C sort -o "$of_manifest" "$of_manifest"
diff -u \
    "$of_candidate" \
    <(LC_ALL=C sort -u "$of_manifest" "$of_deferred")
if [[ -n "$(LC_ALL=C comm -12 "$of_manifest" "$of_deferred")" ]]; then
    echo "error: TypedArray static of manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$of_candidate_keys"
: >"$of_deferred_keys"
: >"$of_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$of_candidate_keys"
done <"$of_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$of_deferred_keys"
done <"$of_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$of_keys"
done <"$of_manifest"
LC_ALL=C sort -o "$of_candidate_keys" "$of_candidate_keys"
LC_ALL=C sort -o "$of_deferred_keys" "$of_deferred_keys"
LC_ALL=C sort -o "$of_keys" "$of_keys"
if [[ "$(wc -l <"$of_candidate" | tr -d '[:space:]')" \
        != "$expected_of_candidate_paths" \
    || "$(sha256_file "$of_candidate")" != "$expected_of_candidate" \
    || "$(wc -l <"$of_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_of_candidate_variants" \
    || "$(sha256_file "$of_candidate_keys")" \
        != "$expected_of_candidate_keys" \
    || "$(wc -l <"$of_deferred" | tr -d '[:space:]')" \
        != "$expected_of_deferred_paths" \
    || "$(sha256_file "$of_deferred")" != "$expected_of_deferred" \
    || "$(wc -l <"$of_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_of_deferred_variants" \
    || "$(sha256_file "$of_deferred_keys")" \
        != "$expected_of_deferred_keys" \
    || "$(wc -l <"$of_manifest" | tr -d '[:space:]')" \
        != "$expected_of_paths" \
    || "$(sha256_file "$of_manifest")" != "$expected_of_manifest" \
    || "$(wc -l <"$of_keys" | tr -d '[:space:]')" \
        != "$expected_of_variants" \
    || "$(sha256_file "$of_keys")" != "$expected_of_keys" ]]; then
    echo "error: TypedArray static of promotion inventory drifted" >&2
    exit 1
fi

LC_ALL=C sort -o "$from_candidate" "$from_candidate"
LC_ALL=C sort -o "$from_deferred" "$from_deferred"
LC_ALL=C sort -o "$from_manifest" "$from_manifest"
diff -u \
    "$from_candidate" \
    <(LC_ALL=C sort -u "$from_manifest" "$from_deferred")
if [[ -n "$(LC_ALL=C comm -12 "$from_manifest" "$from_deferred")" ]]; then
    echo "error: TypedArray static from manifest overlaps its deferred ledger" >&2
    exit 1
fi

: >"$from_candidate_keys"
: >"$from_deferred_keys"
: >"$from_keys"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$from_candidate_keys"
done <"$from_candidate"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$from_deferred_keys"
done <"$from_deferred"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys "$test_path" "$candidate_flags" "$from_keys"
done <"$from_manifest"
LC_ALL=C sort -o "$from_candidate_keys" "$from_candidate_keys"
LC_ALL=C sort -o "$from_deferred_keys" "$from_deferred_keys"
LC_ALL=C sort -o "$from_keys" "$from_keys"
if [[ "$(wc -l <"$from_candidate" | tr -d '[:space:]')" \
        != "$expected_from_candidate_paths" \
    || "$(sha256_file "$from_candidate")" != "$expected_from_candidate" \
    || "$(wc -l <"$from_candidate_keys" | tr -d '[:space:]')" \
        != "$expected_from_candidate_variants" \
    || "$(sha256_file "$from_candidate_keys")" \
        != "$expected_from_candidate_keys" \
    || "$(wc -l <"$from_deferred" | tr -d '[:space:]')" \
        != "$expected_from_deferred_paths" \
    || "$(sha256_file "$from_deferred")" != "$expected_from_deferred" \
    || "$(wc -l <"$from_deferred_keys" | tr -d '[:space:]')" \
        != "$expected_from_deferred_variants" \
    || "$(sha256_file "$from_deferred_keys")" \
        != "$expected_from_deferred_keys" \
    || "$(wc -l <"$from_manifest" | tr -d '[:space:]')" \
        != "$expected_from_paths" \
    || "$(sha256_file "$from_manifest")" != "$expected_from_manifest" \
    || "$(wc -l <"$from_keys" | tr -d '[:space:]')" \
        != "$expected_from_variants" \
    || "$(sha256_file "$from_keys")" != "$expected_from_keys" ]]; then
    echo "error: TypedArray static from promotion inventory drifted" >&2
    exit 1
fi

while IFS= read -r test_path; do
    metadata_list "$test_path" features >"$candidate_features"
    metadata_list "$test_path" includes >"$candidate_includes"
    metadata_list "$test_path" flags >"$candidate_flags"
    source_body "$test_path" >"$source_file"
    concrete_typed_array_tokens "$source_file" >"$typed_array_tokens"
    if [[ -z "$(metadata_block "$test_path")" \
        || -n "$(metadata_list "$test_path" features \
            | grep -E '^(SharedArrayBuffer|Atomics|immutable-arraybuffer|cross-realm)$' \
            || true)" \
        || "$(grep -c '^negative:' <<<"$(metadata_block "$test_path")" || true)" \
            != "0" ]]; then
        echo "error: latent TypedArray core spillover gained an external dependency: $test_path" >&2
        exit 1
    fi
    if global_activation_spillover_paths | grep -Fxq "$test_path"; then
        if grep -Evq '^(generated|noStrict)$' "$candidate_flags" \
            || [[ -n "$(LC_ALL=C sort "$candidate_flags" | uniq -d)" ]] \
            || [[ "$(grep -Fxc TypedArray "$candidate_features")" != "1" ]]; then
            echo "error: TypedArray global activation spillover flags drifted: $test_path" >&2
            exit 1
        fi
    elif [[ -s "$candidate_flags" ]]; then
        echo "error: latent TypedArray core spillover gained execution flags: $test_path" >&2
        exit 1
    elif [[ ! -s "$typed_array_tokens" ]] \
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

: >"$derived_global_activation"
while IFS= read -r test_path; do
    metadata_list "$test_path" features >"$candidate_features"
    if grep -Fxq TypedArray "$candidate_features" \
        && [[ -z "$(LC_ALL=C comm -23 \
            <(LC_ALL=C sort -u "$candidate_features") \
            "$global_features")" ]]; then
        printf '%s\n' "$test_path" >>"$derived_global_activation"
    fi
done <"$manifest_inventory"
LC_ALL=C sort -o "$derived_global_activation" "$derived_global_activation"
diff -u "$global_activation_inventory" "$derived_global_activation"

LC_ALL=C comm -23 \
    "$global_activation_inventory" \
    "$global_spillover_inventory" >"$global_authenticated_inventory"
if [[ -n "$(LC_ALL=C comm -12 \
        "$global_authenticated_inventory" \
        "$global_spillover_inventory")" ]]; then
    echo "error: TypedArray global activation partitions overlap" >&2
    exit 1
fi
diff -u \
    "$global_activation_inventory" \
    <(LC_ALL=C sort -u \
        "$global_authenticated_inventory" \
        "$global_spillover_inventory")
diff -u \
    "$global_spillover_inventory" \
    <(LC_ALL=C comm -12 \
        "$global_activation_inventory" \
        "$global_spillover_inventory")
if [[ -n "$(LC_ALL=C comm -12 \
        "$global_activation_inventory" \
        "$global_reason_only_inventory")" ]]; then
    echo "error: TypedArray activation overlaps its reason-only ledger" >&2
    exit 1
fi

: >"$global_activation_keys"
: >"$global_authenticated_keys"
: >"$global_spillover_keys"
: >"$global_reason_only_keys"
: >"$global_reason_only_flag_inventory"
: >"$global_reason_only_missing_details"
: >"$global_activation_feature_occurrences"
: >"$global_activation_include_occurrences"
: >"$global_spillover_feature_occurrences"
: >"$global_spillover_include_occurrences"
: >"$global_spillover_flag_inventory"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$global_activation_keys"
    metadata_list "$test_path" features \
        >>"$global_activation_feature_occurrences"
    metadata_list "$test_path" includes \
        >>"$global_activation_include_occurrences"
done <"$global_activation_inventory"
while IFS= read -r test_path; do
    metadata_list "$test_path" flags >"$candidate_flags"
    append_variant_keys \
        "$test_path" "$candidate_flags" "$global_authenticated_keys"
done <"$global_authenticated_inventory"
while IFS= read -r test_path; do
    metadata_list "$test_path" features >"$candidate_features"
    metadata_list "$test_path" includes >"$candidate_includes"
    metadata_list "$test_path" flags >"$candidate_flags"
    source_body "$test_path" >"$source_file"
    concrete_typed_array_tokens "$source_file" >"$typed_array_tokens"
    if [[ -z "$(metadata_block "$test_path")" \
        || ! -s "$candidate_features" \
        || "$(grep -Fxc TypedArray "$candidate_features")" != "1" \
        || -n "$(LC_ALL=C comm -23 \
            <(LC_ALL=C sort -u "$candidate_features") \
            "$global_features")" \
        || -n "$(grep -Ev \
            '^(compareArray\.js|detachArrayBuffer\.js|testTypedArray\.js)$' \
            "$candidate_includes" || true)" \
        || -n "$(grep -Ev '^(generated|noStrict)$' \
            "$candidate_flags" || true)" \
        || -n "$(LC_ALL=C sort "$candidate_flags" | uniq -d)" \
        || "$(grep -c '^negative:' <<<"$(metadata_block "$test_path")" || true)" \
            != "0" \
        || -n "$(grep -F '$262.' "$source_file" || true)" ]]; then
        echo "error: TypedArray global activation spillover dependency drifted: $test_path" >&2
        exit 1
    fi
    if [[ -s "$candidate_flags" ]]; then
        sed "s#^#$test_path\\t#" "$candidate_flags" \
            >>"$global_spillover_flag_inventory"
    else
        printf '%s\tdefault\n' "$test_path" \
            >>"$global_spillover_flag_inventory"
    fi
    append_variant_keys \
        "$test_path" "$candidate_flags" "$global_spillover_keys"
    cat "$candidate_features" >>"$global_spillover_feature_occurrences"
    cat "$candidate_includes" >>"$global_spillover_include_occurrences"
done <"$global_spillover_inventory"
while IFS= read -r test_path; do
    metadata_list "$test_path" features >"$candidate_features"
    metadata_list "$test_path" flags >"$candidate_flags"
    LC_ALL=C sort -u -o "$candidate_features" "$candidate_features"
    LC_ALL=C comm -23 \
        "$candidate_features" \
        "$global_features" >"$global_reason_only_missing_features"
    if [[ "$(grep -Fxc TypedArray "$candidate_features")" != "1" \
        || ! -s "$global_reason_only_missing_features" \
        || -n "$(grep -Fx TypedArray \
            "$global_reason_only_missing_features" || true)" ]]; then
        echo "error: TypedArray reason-only dependency drifted: $test_path" >&2
        exit 1
    fi
    reason_flag_signature=$(LC_ALL=C sort "$candidate_flags" | paste -sd, -)
    if [[ -z "$reason_flag_signature" ]]; then
        reason_flag_signature=default
    fi
    printf '%s\t%s\n' \
        "$test_path" \
        "$reason_flag_signature" >>"$global_reason_only_flag_inventory"
    reason_missing_detail=$(awk '
        BEGIN { separator="" }
        {
            printf "%s%s", separator, $0
            separator=", "
        }
        END { print "" }
    ' "$global_reason_only_missing_features")
    printf '%s\t%s%s\n' \
        "$test_path" \
        "$reason_detail_prefix" \
        "$reason_missing_detail" >>"$global_reason_only_missing_details"
    append_reason_only_variant_keys \
        "$test_path" "$candidate_flags" "$global_reason_only_keys"
done <"$global_reason_only_inventory"
LC_ALL=C sort -o "$global_activation_keys" "$global_activation_keys"
LC_ALL=C sort -o "$global_authenticated_keys" "$global_authenticated_keys"
LC_ALL=C sort -o "$global_spillover_keys" "$global_spillover_keys"
LC_ALL=C sort -o "$global_reason_only_keys" "$global_reason_only_keys"
LC_ALL=C sort -o \
    "$global_reason_only_flag_inventory" \
    "$global_reason_only_flag_inventory"
LC_ALL=C sort -o \
    "$global_reason_only_missing_details" \
    "$global_reason_only_missing_details"
awk -F'\t' '
    BEGIN { OFS="\t" }
    NR == FNR {
        if (NF != 2 || $1 == "" || ($1 in details)) exit 1
        details[$1]=$2
        next
    }
    {
        if (NF != 2 || !($1 in details)) exit 1
        print $1, $2, details[$1]
        used[$1]=1
    }
    END {
        for (path in details) {
            if (!(path in used)) exit 1
        }
    }
' "$global_reason_only_missing_details" \
    "$global_reason_only_keys" >"$global_reason_only_expected_details"
diff -u \
    "$global_transition_reason_after_details" \
    "$global_reason_only_expected_details"
LC_ALL=C sort -u \
    "$global_activation_inventory" \
    "$global_reason_only_inventory" \
    >"$previous_typed_array_unsupported_inventory"
LC_ALL=C sort -u \
    "$global_activation_keys" \
    "$global_reason_only_keys" \
    >"$previous_typed_array_unsupported_keys"
diff -u \
    "$global_transition_activation_inventory" \
    "$global_activation_inventory"
diff -u \
    "$global_transition_activation_keys" \
    "$global_activation_keys"
diff -u \
    "$global_transition_reason_inventory" \
    "$global_reason_only_inventory"
diff -u \
    "$global_transition_reason_keys" \
    "$global_reason_only_keys"
diff -u \
    "$global_transition_all_inventory" \
    "$previous_typed_array_unsupported_inventory"
diff -u \
    "$global_transition_all_keys" \
    "$previous_typed_array_unsupported_keys"
LC_ALL=C sort -u \
    "$global_activation_feature_occurrences" \
    >"$global_activation_feature_inventory"
LC_ALL=C sort -u \
    "$global_activation_include_occurrences" \
    >"$global_activation_include_inventory"
LC_ALL=C sort -u \
    "$global_spillover_feature_occurrences" \
    >"$global_spillover_feature_inventory"
LC_ALL=C sort -u \
    "$global_spillover_include_occurrences" \
    >"$global_spillover_include_inventory"
LC_ALL=C sort -o \
    "$global_spillover_flag_inventory" \
    "$global_spillover_flag_inventory"

if [[ "$(wc -l <"$global_activation_inventory" | tr -d '[:space:]')" \
        != "$expected_global_activation_paths" \
    || "$(sha256_file "$global_activation_inventory")" \
        != "$expected_global_activation" \
    || "$(wc -l <"$global_activation_keys" | tr -d '[:space:]')" \
        != "$expected_global_activation_variants" \
    || "$(sha256_file "$global_activation_keys")" \
        != "$expected_global_activation_keys" \
    || "$(wc -l <"$global_authenticated_inventory" | tr -d '[:space:]')" \
        != "$expected_global_authenticated_paths" \
    || "$(sha256_file "$global_authenticated_inventory")" \
        != "$expected_global_authenticated" \
    || "$(wc -l <"$global_authenticated_keys" | tr -d '[:space:]')" \
        != "$expected_global_authenticated_variants" \
    || "$(sha256_file "$global_authenticated_keys")" \
        != "$expected_global_authenticated_keys" \
    || "$(wc -l <"$global_spillover_inventory" | tr -d '[:space:]')" \
        != "$expected_global_spillover_paths" \
    || "$(sha256_file "$global_spillover_inventory")" \
        != "$expected_global_spillover" \
    || "$(wc -l <"$global_spillover_keys" | tr -d '[:space:]')" \
        != "$expected_global_spillover_variants" \
    || "$(sha256_file "$global_spillover_keys")" \
        != "$expected_global_spillover_keys" \
    || "$(wc -l <"$global_activation_feature_inventory" \
        | tr -d '[:space:]')" != "$expected_global_activation_features" \
    || "$(sha256_file "$global_activation_feature_inventory")" \
        != "$expected_global_activation_features_hash" \
    || "$(wc -l <"$global_activation_include_inventory" \
        | tr -d '[:space:]')" != "$expected_global_activation_includes" \
    || "$(sha256_file "$global_activation_include_inventory")" \
        != "$expected_global_activation_includes_hash" \
    || "$(wc -l <"$global_spillover_feature_inventory" \
        | tr -d '[:space:]')" != "$expected_global_spillover_features" \
    || "$(sha256_file "$global_spillover_feature_inventory")" \
        != "$expected_global_spillover_features_hash" \
    || "$(wc -l <"$global_spillover_include_inventory" \
        | tr -d '[:space:]')" != "$expected_global_spillover_includes" \
    || "$(sha256_file "$global_spillover_include_inventory")" \
        != "$expected_global_spillover_includes_hash" \
    || "$(wc -l <"$global_reason_only_inventory" | tr -d '[:space:]')" \
        != "$expected_global_reason_only_paths" \
    || "$(sha256_file "$global_reason_only_inventory")" \
        != "$expected_global_reason_only" \
    || "$(wc -l <"$global_reason_only_keys" | tr -d '[:space:]')" \
        != "$expected_global_reason_only_variants" \
    || "$(sha256_file "$global_reason_only_keys")" \
        != "$expected_global_reason_only_keys" \
    || "$(wc -l <"$global_reason_only_flag_inventory" \
        | tr -d '[:space:]')" != "$expected_global_reason_only_paths" \
    || "$(wc -l <"$global_reason_only_missing_details" \
        | tr -d '[:space:]')" != "$expected_global_reason_only_paths" \
    || "$(wc -l <"$global_reason_only_expected_details" \
        | tr -d '[:space:]')" != "$expected_global_reason_only_variants" \
    || "$(wc -l <"$previous_typed_array_unsupported_inventory" \
        | tr -d '[:space:]')" \
        != "$expected_previous_typed_array_unsupported_paths" \
    || "$(sha256_file "$previous_typed_array_unsupported_inventory")" \
        != "$expected_previous_typed_array_unsupported" \
    || "$(wc -l <"$previous_typed_array_unsupported_keys" \
        | tr -d '[:space:]')" \
        != "$expected_previous_typed_array_unsupported_variants" \
    || "$(sha256_file "$previous_typed_array_unsupported_keys")" \
        != "$expected_previous_typed_array_unsupported_keys" ]]; then
    echo "error: TypedArray global activation inventory drifted" >&2
    exit 1
fi
if ! awk -F'\t' -v expected="$expected_global_reason_only_paths" '
    {
        if (NF != 2 || $1 == "" || seen[$1]++) exit 1
        counts[$2]++
    }
    END {
        for (flag in counts) {
            if (flag != "default" &&
                flag != "generated" &&
                flag != "CanBlockIsTrue" &&
                flag != "CanBlockIsFalse" &&
                flag != "onlyStrict" &&
                flag != "noStrict") exit 1
        }
        if (NR != expected ||
            counts["default"] != 440 ||
            counts["generated"] != 20 ||
            counts["CanBlockIsTrue"] != 7 ||
            counts["CanBlockIsFalse"] != 0 ||
            counts["onlyStrict"] != 2 ||
            counts["noStrict"] != 2) exit 1
    }
' "$global_reason_only_flag_inventory"; then
    echo "error: TypedArray reason-only path flag inventory drifted" >&2
    exit 1
fi
if ! awk -F'\t' '
    { counts[$2]++ }
    END {
        if (NR != 41 ||
            counts["default"] != 33 ||
            counts["generated"] != 6 ||
            counts["noStrict"] != 2) exit 1
    }
' "$global_spillover_flag_inventory"; then
    echo "error: TypedArray global activation spillover flag inventory drifted" >&2
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
    printf 'TypedArray core Test262 assets pass: %s candidate paths/%s variants, %s core paths/%s variants (including %s callback-find paths/%s variants, %s every/some paths/%s variants, %s forEach paths/%s variants, %s reduce/reduceRight paths/%s variants, %s map/filter paths/%s variants, %s slice/subarray paths/%s variants, %s with/toReversed paths/%s variants, %s stringification paths/%s variants, %s sort/toSorted paths/%s variants, %s entries/keys paths/%s variants, %s static of paths/%s variants, and %s static from paths/%s variants; %s every/some, %s forEach, %s reduce/reduceRight, %s map/filter, %s slice/subarray, %s with/toReversed, %s stringification, %s sort/toSorted, %s entries/keys, %s static of, and %s static from staging paths deferred), %s exclusions; pinned QuickJS passes candidate and admitted vectors\n' \
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
        "$expected_slice_subarray_paths" \
        "$expected_slice_subarray_variants" \
        "$expected_with_to_reversed_paths" \
        "$expected_with_to_reversed_variants" \
        "$expected_stringification_paths" \
        "$expected_stringification_variants" \
        "$expected_sort_paths" \
        "$expected_sort_variants" \
        "$expected_entries_keys_paths" \
        "$expected_entries_keys_variants" \
        "$expected_of_paths" \
        "$expected_of_variants" \
        "$expected_from_paths" \
        "$expected_from_variants" \
        "$expected_every_some_deferred_paths" \
        "$expected_for_each_deferred_paths" \
        "$expected_reduce_deferred_paths" \
        "$expected_map_filter_deferred_paths" \
        "$expected_slice_subarray_deferred_paths" \
        "$expected_with_to_reversed_deferred_paths" \
        "$expected_stringification_deferred_paths" \
        "$expected_sort_deferred_paths" \
        "$expected_entries_keys_deferred_paths" \
        "$expected_of_deferred_paths" \
        "$expected_from_deferred_paths" \
        "$expected_excluded_paths"
    printf 'TypedArray global activation assets pass: %s paths/%s variants (%s authenticated paths/%s variants + %s spillover paths/%s variants), %s reason-only paths/%s variants, the %s-row R3bd-to-R3be receipt, and %s external exclusions remain frozen\n' \
        "$expected_global_activation_paths" \
        "$expected_global_activation_variants" \
        "$expected_global_authenticated_paths" \
        "$expected_global_authenticated_variants" \
        "$expected_global_spillover_paths" \
        "$expected_global_spillover_variants" \
        "$expected_global_reason_only_paths" \
        "$expected_global_reason_only_variants" \
        "$expected_transition_rows" \
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

activation_expected_passes=$(read_global_activation_value passes)
activation_expected_failures=$(read_global_activation_value failures)
activation_expected_unsupported=$(read_global_activation_value unsupported)
activation_expected_skipped=$(read_global_activation_value skipped)
activation_expected_nonpass=$(read_global_activation_value nonpass_sha256)
activation_expected_tsv=$(read_global_activation_value tsv_sha256)
activation_expected_jsonl=$(read_global_activation_value jsonl_sha256)
activation_expected_summary=$(read_global_activation_value summary)
if [[ "$activation_expected_passes" \
        != "$expected_global_activation_variants" \
    || "$activation_expected_failures" != "0" \
    || "$activation_expected_unsupported" != "0" \
    || "$activation_expected_skipped" != "0" \
    || "$activation_expected_nonpass" \
        != "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" \
    || "$activation_expected_summary" \
        != "pass=$expected_global_activation_variants" ]]; then
    echo "error: measured TypedArray global activation baseline is not an all-green gate" >&2
    exit 1
fi

rm -f -- "$global_activation_report" "$global_activation_json_report"
global_activation_run_output=$(cargo run --locked --release --quiet \
    --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$global_profile" \
    --manifest "$global_activation_manifest" \
    --report "$global_activation_report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms" \
    --allow-failures)
printf '%s\n' "$global_activation_run_output"

activation_actual_variants=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { count++ }
    END { print count + 0 }' "$global_activation_report")
activation_execution_line=$(printf '%s\n' "$global_activation_run_output" \
    | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
activation_actual_runnable=${activation_execution_line#*runnable=}
activation_actual_runnable=${activation_actual_runnable%% *}
if [[ "$(read_report_header "$global_activation_report" quickjs)" \
        != "$expected_quickjs" \
    || "$(read_report_header "$global_activation_report" test262)" \
        != "$expected_test262" \
    || "$(read_report_header \
        "$global_activation_report" test262_patch_sha256)" != "$expected_patch" \
    || "$(read_report_header \
        "$global_activation_report" test262_config_sha256)" != "$expected_config" \
    || "$(read_report_header \
        "$global_activation_report" test262_metadata_sha256)" \
        != "$expected_metadata" \
    || "$(read_report_header \
        "$global_activation_report" oxide_profile_sha256)" \
        != "$expected_global_profile" \
    || "$(read_report_header "$global_activation_report" profile)" \
        != "$expected_schema" \
    || "$(read_report_header "$global_activation_report" mode)" \
        != "$expected_mode" \
    || "$activation_actual_variants" \
        != "$expected_global_activation_variants" \
    || "$activation_actual_runnable" \
        != "$expected_global_activation_variants" ]]; then
    echo "error: TypedArray global activation report metadata drifted" >&2
    exit 1
fi

diff -u \
    "$global_activation_inventory" \
    <(awk -F'\t' \
        '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$global_activation_report" | LC_ALL=C sort -u)
diff -u \
    "$global_activation_feature_inventory" \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i <= count; i++) {
                if (features[i] != "") print features[i]
            }
        }
    ' "$global_activation_report" | LC_ALL=C sort -u)

activation_actual_keys=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2
    }' "$global_activation_report" | LC_ALL=C sort | sha256_stream)
activation_actual_passes=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" {
        count++
    }
    END { print count + 0 }' "$global_activation_report")
activation_actual_unsupported=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") &&
        $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }' "$global_activation_report")
activation_actual_skipped=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") &&
        $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }' "$global_activation_report")
activation_actual_failures=$((
    activation_actual_variants
    - activation_actual_passes
    - activation_actual_unsupported
    - activation_actual_skipped
))
activation_actual_nonpass=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' "$global_activation_report" | sha256_stream)
activation_actual_summary=$(tail -n 1 "$global_activation_report" \
    | sed 's/^# summary //')
activation_runner_summary=$(printf '%s\n' "$global_activation_run_output" \
    | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
activation_expected_runner_summary="Test262: total=$expected_global_activation_variants pass=$activation_expected_passes fail=$activation_expected_failures unsupported=$activation_expected_unsupported skipped=$activation_expected_skipped"

if [[ "$activation_runner_summary" \
        != "$activation_expected_runner_summary" \
    || "$activation_actual_passes" != "$activation_expected_passes" \
    || "$activation_actual_failures" != "$activation_expected_failures" \
    || "$activation_actual_unsupported" != "$activation_expected_unsupported" \
    || "$activation_actual_skipped" != "$activation_expected_skipped" \
    || "$activation_actual_keys" != "$expected_global_activation_keys" \
    || "$activation_actual_nonpass" != "$activation_expected_nonpass" \
    || "$activation_actual_summary" != "$activation_expected_summary" \
    || "$(sha256_file "$global_activation_report")" \
        != "$activation_expected_tsv" \
    || "$(sha256_file "$global_activation_json_report")" \
        != "$activation_expected_jsonl" ]]; then
    echo "error: TypedArray global activation classified vector drifted" >&2
    printf 'path\tvariant\toutcome\tactual_phase\tactual_type\tdetail\n' >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            if (++shown == 80) exit
        }
    ' "$global_activation_report" >&2
    exit 1
fi

reason_expected_passes=$(read_global_activation_value reason_only_passes)
reason_expected_failures=$(read_global_activation_value reason_only_failures)
reason_expected_unsupported=$(read_global_activation_value reason_only_unsupported)
reason_expected_skipped=$(read_global_activation_value reason_only_skipped)
reason_expected_nonpass=$(read_global_activation_value \
    reason_only_nonpass_sha256)
reason_expected_tsv=$(read_global_activation_value reason_only_tsv_sha256)
reason_expected_jsonl=$(read_global_activation_value \
    reason_only_jsonl_sha256)
reason_expected_summary=$(read_global_activation_value reason_only_summary)
if [[ "$reason_expected_passes" != "0" \
    || "$reason_expected_failures" != "0" \
    || "$reason_expected_unsupported" \
        != "$expected_global_reason_only_variants" \
    || "$reason_expected_skipped" != "0" \
    || "$reason_expected_summary" \
        != "unsupported-feature=$expected_global_reason_only_variants" ]]; then
    echo "error: measured TypedArray reason-only baseline drifted" >&2
    exit 1
fi

rm -f -- "$global_reason_only_report" "$global_reason_only_json_report"
reason_run_output=$(cargo run --locked --release --quiet \
    --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$global_profile" \
    --manifest "$global_reason_only_manifest" \
    --report "$global_reason_only_report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms" \
    --allow-failures)
printf '%s\n' "$reason_run_output"

reason_actual_variants=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { count++ }
    END { print count + 0 }' "$global_reason_only_report")
reason_execution_line=$(printf '%s\n' "$reason_run_output" \
    | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
reason_actual_runnable=${reason_execution_line#*runnable=}
reason_actual_runnable=${reason_actual_runnable%% *}
if [[ "$(read_report_header "$global_reason_only_report" quickjs)" \
        != "$expected_quickjs" \
    || "$(read_report_header "$global_reason_only_report" test262)" \
        != "$expected_test262" \
    || "$(read_report_header \
        "$global_reason_only_report" test262_patch_sha256)" != "$expected_patch" \
    || "$(read_report_header \
        "$global_reason_only_report" test262_config_sha256)" != "$expected_config" \
    || "$(read_report_header \
        "$global_reason_only_report" test262_metadata_sha256)" \
        != "$expected_metadata" \
    || "$(read_report_header \
        "$global_reason_only_report" oxide_profile_sha256)" \
        != "$expected_global_profile" \
    || "$(read_report_header "$global_reason_only_report" profile)" \
        != "$expected_schema" \
    || "$(read_report_header "$global_reason_only_report" mode)" \
        != "$expected_mode" \
    || "$reason_actual_variants" != "$expected_global_reason_only_variants" \
    || "$reason_actual_runnable" != "0" ]]; then
    echo "error: TypedArray reason-only report metadata drifted" >&2
    exit 1
fi

diff -u \
    "$global_reason_only_inventory" \
    <(awk -F'\t' \
        '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$global_reason_only_report" | LC_ALL=C sort -u)
awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2 "\t" $10
    }
' "$global_reason_only_report" \
    | LC_ALL=C sort >"$global_reason_only_actual_details"
diff -u \
    "$global_reason_only_expected_details" \
    "$global_reason_only_actual_details"
if ! awk -F'\t' -v prefix="$reason_detail_prefix" '
    !/^#/ && !($1 == "path" && $2 == "variant") {
        if ($7 != "unsupported-feature" ||
            $8 != "selection" ||
            $9 != "EngineCapability") exit 1
        typed_array_feature=0
        count=split($4, features, ",")
        for (i=1; i <= count; i++) {
            if (features[i] == "TypedArray") typed_array_feature++
        }
        if (typed_array_feature != 1) exit 1
        if (index($10, prefix) != 1) exit 1
        detail=substr($10, length(prefix) + 1)
        count=split(detail, missing, /, /)
        if (count < 1) exit 1
        for (i=1; i <= count; i++) {
            if (missing[i] == "TypedArray") exit 1
        }
    }
' "$global_reason_only_report"; then
    echo "error: TypedArray reason-only outcome or missing-feature reason drifted" >&2
    exit 1
fi

reason_actual_keys=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") {
        print $1 "\t" $2
    }' "$global_reason_only_report" | LC_ALL=C sort | sha256_stream)
reason_actual_passes=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" {
        count++
    }
    END { print count + 0 }' "$global_reason_only_report")
reason_actual_unsupported=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") &&
        $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }' "$global_reason_only_report")
reason_actual_skipped=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") &&
        $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }' "$global_reason_only_report")
reason_actual_failures=$((
    reason_actual_variants
    - reason_actual_passes
    - reason_actual_unsupported
    - reason_actual_skipped
))
reason_actual_nonpass=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' "$global_reason_only_report" | sha256_stream)
reason_actual_summary=$(tail -n 1 "$global_reason_only_report" \
    | sed 's/^# summary //')
reason_runner_summary=$(printf '%s\n' "$reason_run_output" \
    | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
reason_expected_runner_summary="Test262: total=$expected_global_reason_only_variants pass=0 fail=$expected_global_reason_only_variants unsupported=$expected_global_reason_only_variants skipped=0"
if [[ "$reason_runner_summary" != "$reason_expected_runner_summary" \
    || "$reason_actual_passes" != "$reason_expected_passes" \
    || "$reason_actual_failures" != "$reason_expected_failures" \
    || "$reason_actual_unsupported" != "$reason_expected_unsupported" \
    || "$reason_actual_skipped" != "$reason_expected_skipped" \
    || "$reason_actual_keys" != "$expected_global_reason_only_keys" \
    || "$reason_actual_nonpass" != "$reason_expected_nonpass" \
    || "$reason_actual_summary" != "$reason_expected_summary" \
    || "$(sha256_file "$global_reason_only_report")" \
        != "$reason_expected_tsv" \
    || "$(sha256_file "$global_reason_only_json_report")" \
        != "$reason_expected_jsonl" ]]; then
    echo "error: TypedArray reason-only classified vector drifted" >&2
    exit 1
fi

{
    awk -F'\t' '
        BEGIN { OFS="\t" }
        !/^#/ && !($1 == "path" && $2 == "variant") {
            print $1, $2, $7, $10
        }
    ' "$global_activation_report"
    awk -F'\t' '
        BEGIN { OFS="\t" }
        !/^#/ && !($1 == "path" && $2 == "variant") {
            print $1, $2, $7, $10
        }
    ' "$global_reason_only_report"
} | LC_ALL=C sort >"$global_transition_after_actual"
if ! LC_ALL=C sort -c "$global_transition_after_actual" \
    || ! awk -F'\t' -v expected="$expected_transition_rows" '
        {
            if (NF != 4 || $1 == "" ||
                ($2 != "sloppy" && $2 != "strict") ||
                seen[$1 SUBSEP $2]++) exit 1
        }
        END { if (NR != expected) exit 1 }
    ' "$global_transition_after_actual"; then
    echo "error: live TypedArray transition report join is incomplete or duplicated" >&2
    exit 1
fi
if ! diff -u \
    "$global_transition_after_expected" \
    "$global_transition_after_actual"; then
    echo "error: live TypedArray transition outcomes or details drifted from the receipt" >&2
    exit 1
fi

printf 'TypedArray global activation gate passes: %s/%s variants across %s paths (%s authenticated + %s spillover); %s old unsupported rows transition to pass and %s retain unsupported-feature with reason-only changes\n' \
    "$activation_expected_passes" \
    "$expected_global_activation_variants" \
    "$expected_global_activation_paths" \
    "$expected_global_authenticated_paths" \
    "$expected_global_spillover_paths" \
    "$expected_global_activation_variants" \
    "$expected_global_reason_only_variants"
