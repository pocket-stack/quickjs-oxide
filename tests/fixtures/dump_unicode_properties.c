/*
 * Test-only generator oracle for checksum-pinned Unicode property data.
 *
 * This file is compiled only by scripts/generate-unicode-property-tables.sh.
 * Product builds consume the generated Rust arrays and never compile or link
 * QuickJS C code.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "libunicode.c"

#define UNICODE_LIMIT 0x110000U

typedef int (*PropertyBuilder)(CharRange *range, const char *name);

typedef struct {
    size_t count;
} SequenceDumpState;

static size_t name_group_count(const char *table)
{
    size_t count = 0;
    const char *entry;

    for (entry = table; *entry; entry += strlen(entry) + 1)
        count++;
    return count;
}

static void first_alias(char *buffer, size_t buffer_size, const char *entry)
{
    const char *comma = strchr(entry, ',');
    size_t length = comma ? (size_t)(comma - entry) : strlen(entry);

    if (length + 1 > buffer_size) {
        fprintf(stderr, "unicode property alias is too long\n");
        exit(1);
    }
    memcpy(buffer, entry, length);
    buffer[length] = '\0';
}

static void dump_one_range(const CharRange *range)
{
    int column = 0;
    int index;

    printf("    &[\n");
    for (index = 0; index < range->len; index += 2) {
        uint32_t start = range->points[index];
        uint32_t end = range->points[index + 1];
        uint32_t values[2];
        int value_index;

        if (start >= UNICODE_LIMIT)
            continue;
        if (end > UNICODE_LIMIT)
            end = UNICODE_LIMIT;
        if (start >= end)
            continue;
        values[0] = start;
        values[1] = end;
        for (value_index = 0; value_index < 2; value_index++) {
            if (column == 0)
                printf("        ");
            else
                printf(" ");
            printf("0x%06x,", values[value_index]);
            column++;
            if (column == 9) {
                printf("\n");
                column = 0;
            }
        }
    }
    if (column != 0)
        printf("\n");
    printf("    ],\n");
}

static int build_general_category(CharRange *range, const char *name)
{
    return unicode_general_category(range, name);
}

static int build_binary_property(CharRange *range, const char *name)
{
    return unicode_prop(range, name);
}

static void dump_property_ranges(const char *constant_name,
                                 const char *name_table,
                                 PropertyBuilder builder,
                                 size_t expected_groups,
                                 size_t expected_accepted)
{
    const char *entry;
    size_t groups = name_group_count(name_table);
    size_t accepted = 0;

    if (groups != expected_groups) {
        fprintf(stderr,
                "%s group count drifted: expected %zu, got %zu\n",
                constant_name, expected_groups, groups);
        exit(1);
    }

    printf("pub(super) const %s_RANGES: &[&[u32]] = &[\n", constant_name);
    for (entry = name_table; *entry; entry += strlen(entry) + 1) {
        char name[64];
        CharRange range;
        int result;

        first_alias(name, sizeof(name), entry);
        cr_init(&range, NULL, NULL);
        result = builder(&range, name);
        if (result == 0) {
            dump_one_range(&range);
            accepted++;
        } else if (result != -2) {
            fprintf(stderr, "could not build %s property %s\n",
                    constant_name, name);
            cr_free(&range);
            exit(1);
        }
        cr_free(&range);
    }
    printf("];\n\n");

    if (accepted != expected_accepted) {
        fprintf(stderr,
                "%s accepted count drifted: expected %zu, got %zu\n",
                constant_name, expected_accepted, accepted);
        exit(1);
    }
}

static void dump_script_ranges(const char *constant_name, int extensions)
{
    const char *entry;
    size_t groups = name_group_count(unicode_script_name_table);
    size_t accepted = 0;

    if (groups != UNICODE_SCRIPT_COUNT) {
        fprintf(stderr,
                "%s group count drifted: expected %d, got %zu\n",
                constant_name, UNICODE_SCRIPT_COUNT, groups);
        exit(1);
    }

    printf("pub(super) const %s_RANGES: &[&[u32]] = &[\n", constant_name);
    for (entry = unicode_script_name_table; *entry;
         entry += strlen(entry) + 1) {
        char name[64];
        CharRange range;
        int result;

        first_alias(name, sizeof(name), entry);
        cr_init(&range, NULL, NULL);
        result = unicode_script(&range, name, extensions);
        if (result != 0) {
            fprintf(stderr, "could not build %s script %s\n",
                    constant_name, name);
            cr_free(&range);
            exit(1);
        }
        dump_one_range(&range);
        accepted++;
        cr_free(&range);
    }
    printf("];\n\n");

    if (accepted != groups) {
        fprintf(stderr,
                "%s accepted count drifted: expected %zu, got %zu\n",
                constant_name, groups, accepted);
        exit(1);
    }
}

static void dump_aliases(const char *constant_name,
                         const char *name_table,
                         PropertyBuilder builder)
{
    const char *entry;
    unsigned int property_index = 0;

    printf("pub(super) const %s_ALIASES: &[(&str, u16)] = &[\n",
           constant_name);
    for (entry = name_table; *entry; entry += strlen(entry) + 1) {
        char name[64];
        CharRange range;
        const char *alias;
        int result;

        first_alias(name, sizeof(name), entry);
        cr_init(&range, NULL, NULL);
        result = builder(&range, name);
        cr_free(&range);
        if (result == -2)
            continue;
        if (result != 0) {
            fprintf(stderr, "could not validate %s property %s\n",
                    constant_name, name);
            exit(1);
        }

        alias = entry;
        for (;;) {
            const char *comma = strchr(alias, ',');
            size_t length = comma ? (size_t)(comma - alias) : strlen(alias);
            printf("    (\"%.*s\", %u),\n",
                   (int)length, alias, property_index);
            if (!comma)
                break;
            alias = comma + 1;
        }
        property_index++;
    }
    printf("];\n\n");
}

static void dump_script_aliases(void)
{
    const char *entry;
    unsigned int script_index = 0;

    printf("pub(super) const SCRIPT_ALIASES: &[(&str, u16)] = &[\n");
    for (entry = unicode_script_name_table; *entry;
         entry += strlen(entry) + 1) {
        const char *alias = entry;

        for (;;) {
            const char *comma = strchr(alias, ',');
            size_t length = comma ? (size_t)(comma - alias) : strlen(alias);
            printf("    (\"%.*s\", %u),\n",
                   (int)length, alias, script_index);
            if (!comma)
                break;
            alias = comma + 1;
        }
        script_index++;
    }
    printf("];\n\n");
}

static void dump_one_sequence(void *opaque, const uint32_t *sequence, int length)
{
    SequenceDumpState *state = opaque;
    int index;

    if (length <= 0) {
        fprintf(stderr, "unicode sequence property emitted an empty sequence\n");
        exit(1);
    }

    printf("        &[");
    for (index = 0; index < length; index++) {
        if (sequence[index] >= UNICODE_LIMIT) {
            fprintf(stderr,
                    "unicode sequence property emitted invalid code point %#x\n",
                    sequence[index]);
            exit(1);
        }
        if (index != 0)
            printf(" ");
        printf("0x%06x,", sequence[index]);
    }
    printf("],\n");
    state->count++;
}

static void dump_sequence_properties(void)
{
    static const size_t expected_sequence_counts[] = {
        1400, 12, 670, 259, 3, 1614, 3958,
    };
    const char *entry;
    size_t groups = name_group_count(unicode_sequence_prop_name_table);
    size_t accepted = 0;

    if (groups != UNICODE_SEQUENCE_PROP_COUNT) {
        fprintf(stderr,
                "SEQUENCE_PROPERTY group count drifted: expected %d, got %zu\n",
                UNICODE_SEQUENCE_PROP_COUNT, groups);
        exit(1);
    }
    if (groups != sizeof(expected_sequence_counts) /
                  sizeof(expected_sequence_counts[0])) {
        fprintf(stderr,
                "SEQUENCE_PROPERTY expected-count table drifted: "
                "expected %zu, got %zu\n",
                groups,
                sizeof(expected_sequence_counts) /
                    sizeof(expected_sequence_counts[0]));
        exit(1);
    }

    printf("pub(super) const SEQUENCE_PROPERTY_SEQUENCES: "
           "&[&[&[u32]]] = &[\n");
    for (entry = unicode_sequence_prop_name_table; *entry;
         entry += strlen(entry) + 1) {
        char name[64];
        CharRange range;
        SequenceDumpState state = { 0 };
        int result;

        first_alias(name, sizeof(name), entry);
        printf("    &[\n");
        cr_init(&range, NULL, NULL);
        result = unicode_sequence_prop(name, dump_one_sequence, &state, &range);
        cr_free(&range);
        if (result != 0) {
            fprintf(stderr, "could not build sequence property %s\n", name);
            exit(1);
        }
        if (state.count == 0) {
            fprintf(stderr, "sequence property %s is unexpectedly empty\n", name);
            exit(1);
        }
        if (state.count != expected_sequence_counts[accepted]) {
            fprintf(stderr,
                    "sequence property %s count drifted: expected %zu, got %zu\n",
                    name, expected_sequence_counts[accepted], state.count);
            exit(1);
        }
        printf("    ],\n");
        accepted++;
    }
    printf("];\n\n");

    if (accepted != groups) {
        fprintf(stderr,
                "SEQUENCE_PROPERTY accepted count drifted: expected %zu, got %zu\n",
                groups, accepted);
        exit(1);
    }
}

static void dump_sequence_aliases(void)
{
    const char *entry;
    unsigned int property_index = 0;

    printf("pub(super) const SEQUENCE_PROPERTY_ALIASES: "
           "&[(&str, u16)] = &[\n");
    for (entry = unicode_sequence_prop_name_table; *entry;
         entry += strlen(entry) + 1) {
        const char *alias = entry;

        for (;;) {
            const char *comma = strchr(alias, ',');
            size_t length = comma ? (size_t)(comma - alias) : strlen(alias);
            printf("    (\"%.*s\", %u),\n",
                   (int)length, alias, property_index);
            if (!comma)
                break;
            alias = comma + 1;
        }
        property_index++;
    }
    printf("];\n\n");
}

int main(void)
{
    printf("// @generated by scripts/generate-unicode-property-tables.sh.\n");
    printf("// Source: QuickJS 2026-06-04, Unicode 17.0.0.\n");
    printf("// libunicode-table.h SHA-256: "
           "cf782bc7a07549e976f606bd3cb8555858482b279574554dcb8d46412986006c.\n");
    printf("// Derived from QuickJS MIT source and Unicode UCD data; "
           "see NOTICE and LICENSES/.\n\n");

    dump_property_ranges("GENERAL_CATEGORY",
                         unicode_gc_name_table,
                         build_general_category,
                         UNICODE_GC_COUNT,
                         UNICODE_GC_COUNT);
    dump_script_ranges("SCRIPT", 0);
    dump_script_ranges("SCRIPT_EXTENSIONS", 1);
    dump_property_ranges("BINARY_PROPERTY",
                         unicode_prop_name_table,
                         build_binary_property,
                         58,
                         55);

    dump_aliases("GENERAL_CATEGORY",
                 unicode_gc_name_table,
                 build_general_category);
    dump_script_aliases();
    dump_aliases("BINARY_PROPERTY",
                 unicode_prop_name_table,
                 build_binary_property);
    dump_sequence_properties();
    dump_sequence_aliases();
    printf("// End of generated Unicode property tables.\n");
    return 0;
}
