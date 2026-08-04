/*
 * Test-only fingerprint oracle for checksum-pinned Unicode normalization.
 *
 * This file is compiled only by scripts/check-unicode-normalize-fingerprint.sh.
 * Product builds consume the generated Rust tables and never compile or link
 * QuickJS C code.
 */

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

#include "libunicode.c"

static uint64_t hash_u32(uint64_t hash, uint32_t value)
{
    int index;

    for (index = 0; index < 4; index++) {
        hash ^= (value >> (index * 8)) & 0xff;
        hash *= UINT64_C(1099511628211);
    }
    return hash;
}

int main(void)
{
    uint64_t decomposition_hash = UINT64_C(14695981039346656037);
    uint64_t class_hash = UINT64_C(14695981039346656037);
    uint64_t composition_hash = UINT64_C(14695981039346656037);
    uint32_t canonical_count = 0;
    uint32_t compatibility_count = 0;
    uint32_t nonzero_class_count = 0;
    uint32_t mapping[UNICODE_DECOMP_LEN_MAX];
    uint32_t code_point;
    uint32_t index;

    for (code_point = 0; code_point <= 0x10ffff; code_point++) {
        uint32_t combining_class = unicode_get_cc(code_point);
        uint32_t compatibility;

        nonzero_class_count += combining_class != 0;
        class_hash = hash_u32(class_hash, code_point);
        class_hash = hash_u32(class_hash, combining_class);

        for (compatibility = 0; compatibility < 2; compatibility++) {
            int length = unicode_decomp_char(mapping, code_point, compatibility);
            int mapping_index;

            if (compatibility)
                compatibility_count += length != 0;
            else
                canonical_count += length != 0;
            decomposition_hash = hash_u32(decomposition_hash, code_point);
            decomposition_hash = hash_u32(decomposition_hash, compatibility);
            decomposition_hash = hash_u32(decomposition_hash, length);
            for (mapping_index = 0; mapping_index < length; mapping_index++)
                decomposition_hash = hash_u32(decomposition_hash,
                                              mapping[mapping_index]);
        }
    }

    for (index = 0; index < countof(unicode_comp_table); index++) {
        uint32_t decomposition_index = unicode_comp_table[index];
        uint32_t table_index = decomposition_index >> 6;
        uint32_t run_offset = decomposition_index & 0x3f;
        uint32_t entry = unicode_decomp_table1[table_index];
        uint32_t run_start = entry >> 14;
        uint32_t run_length = (entry >> 7) & 0x7f;
        uint32_t run_type = (entry >> 1) & 0x3f;
        uint32_t composed = run_start + run_offset;
        int pair_length = unicode_decomp_entry(mapping, composed, table_index,
                                               run_start, run_length, run_type);
        uint32_t actual = unicode_compose_pair(mapping[0], mapping[1]);

        composition_hash = hash_u32(composition_hash, index);
        composition_hash = hash_u32(composition_hash, pair_length);
        composition_hash = hash_u32(composition_hash, mapping[0]);
        composition_hash = hash_u32(composition_hash, mapping[1]);
        composition_hash = hash_u32(composition_hash, actual);
    }

    printf("canonical_count=%" PRIu32 "\n", canonical_count);
    printf("compatibility_count=%" PRIu32 "\n", compatibility_count);
    printf("nonzero_cc_count=%" PRIu32 "\n", nonzero_class_count);
    printf("decomp_hash=%" PRIu64 "\n", decomposition_hash);
    printf("cc_hash=%" PRIu64 "\n", class_hash);
    printf("compose_hash=%" PRIu64 "\n", composition_hash);
    return 0;
}
