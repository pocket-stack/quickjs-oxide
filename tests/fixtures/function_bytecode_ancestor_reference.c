#include "quickjs.h"

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_OBJECT = 8,
    BC_TAG_FUNCTION_BYTECODE = 12,
    BC_TAG_OBJECT_REFERENCE = 19,
    EXPECTED_BYTECODE_SIZE = 33,
    EXPECTED_PROPERTY_ATOM = 486,
    EXPECTED_FUNCTION_NAME_ATOM = 168,
};

typedef struct BytecodeCursor {
    const uint8_t *base;
    const uint8_t *cursor;
    const uint8_t *end;
} BytecodeCursor;

typedef struct AncestorReferenceShape {
    uint32_t atom_count;
    size_t root_offset;
    uint32_t root_reference_id;
    uint32_t root_property_count;
    size_t function_offset;
    uint16_t function_flags;
    uint8_t js_mode;
    uint32_t arg_count;
    uint32_t var_count;
    uint32_t defined_arg_count;
    uint32_t stack_size;
    uint32_t var_ref_count;
    uint32_t closure_var_count;
    uint32_t constant_pool_count;
    uint32_t bytecode_size;
    uint32_t local_count;
    size_t code_offset;
    size_t constant_pool_offset;
    size_t ancestor_reference_offset;
    uint32_t ancestor_reference_id;
    uint32_t object_definition_count;
} AncestorReferenceShape;

static void report_exception(JSContext *ctx, const char *operation) {
    JSValue exception = JS_GetException(ctx);
    const char *message = JS_ToCString(ctx, exception);

    if (message) {
        fprintf(stderr, "%s: %s\n", operation, message);
        JS_FreeCString(ctx, message);
    } else {
        fprintf(stderr, "%s\n", operation);
    }
    JS_FreeValue(ctx, exception);
}

static int cursor_take(BytecodeCursor *input, size_t count,
                       const uint8_t **bytes) {
    if ((size_t)(input->end - input->cursor) < count)
        return -1;
    if (bytes)
        *bytes = input->cursor;
    input->cursor += count;
    return 0;
}

static int cursor_u8(BytecodeCursor *input, uint8_t *value) {
    if (input->cursor == input->end)
        return -1;
    *value = *input->cursor++;
    return 0;
}

static int cursor_u16(BytecodeCursor *input, uint16_t *value) {
    if ((size_t)(input->end - input->cursor) < 2)
        return -1;
    *value = (uint16_t)input->cursor[0] |
             ((uint16_t)input->cursor[1] << 8);
    input->cursor += 2;
    return 0;
}

static int cursor_leb128(BytecodeCursor *input, uint32_t *value) {
    uint32_t result = 0;
    unsigned shift = 0;

    while (shift < 35) {
        uint8_t byte;

        if (cursor_u8(input, &byte) < 0)
            return -1;
        if (shift == 28 && (byte & 0xf0) != 0)
            return -1;
        result |= (uint32_t)(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0) {
            *value = result;
            return 0;
        }
        shift += 7;
    }
    return -1;
}

static int parse_ancestor_reference(const uint8_t *bytecode,
                                    size_t bytecode_size,
                                    AncestorReferenceShape *shape) {
    static const uint8_t expected_code[] = {0xbd, 0x00, 0xcb, 0x28};
    BytecodeCursor input = {bytecode, bytecode, bytecode + bytecode_size};
    const uint8_t *atom_bytes;
    const uint8_t *code;
    uint32_t string_header;
    uint32_t property_atom;
    uint32_t function_name_atom;
    uint32_t local_name_atom;
    uint32_t local_scope_level;
    uint32_t local_scope_next;
    uint8_t version;
    uint8_t tag;
    uint8_t local_flags;

    memset(shape, 0, sizeof(*shape));
    if (bytecode_size != EXPECTED_BYTECODE_SIZE ||
        cursor_u8(&input, &version) < 0 || version != BC_VERSION ||
        cursor_leb128(&input, &shape->atom_count) < 0 ||
        shape->atom_count != 1 ||
        cursor_leb128(&input, &string_header) < 0 || string_header != 2 ||
        cursor_take(&input, 1, &atom_bytes) < 0 || atom_bytes[0] != 'f')
        return -1;

    shape->root_offset = (size_t)(input.cursor - input.base);
    shape->root_reference_id = 0;
    shape->object_definition_count = 1;
    if (cursor_u8(&input, &tag) < 0 || tag != BC_TAG_OBJECT ||
        cursor_leb128(&input, &shape->root_property_count) < 0 ||
        shape->root_property_count != 1 ||
        cursor_leb128(&input, &property_atom) < 0 ||
        property_atom != EXPECTED_PROPERTY_ATOM)
        return -1;

    shape->function_offset = (size_t)(input.cursor - input.base);
    if (cursor_u8(&input, &tag) < 0 || tag != BC_TAG_FUNCTION_BYTECODE ||
        cursor_u16(&input, &shape->function_flags) < 0 ||
        shape->function_flags != UINT16_C(0x0200) ||
        cursor_u8(&input, &shape->js_mode) < 0 || shape->js_mode != 0 ||
        cursor_leb128(&input, &function_name_atom) < 0 ||
        function_name_atom != EXPECTED_FUNCTION_NAME_ATOM ||
        cursor_leb128(&input, &shape->arg_count) < 0 ||
        cursor_leb128(&input, &shape->var_count) < 0 ||
        cursor_leb128(&input, &shape->defined_arg_count) < 0 ||
        cursor_leb128(&input, &shape->stack_size) < 0 ||
        cursor_leb128(&input, &shape->var_ref_count) < 0 ||
        cursor_leb128(&input, &shape->closure_var_count) < 0 ||
        cursor_leb128(&input, &shape->constant_pool_count) < 0 ||
        cursor_leb128(&input, &shape->bytecode_size) < 0 ||
        cursor_leb128(&input, &shape->local_count) < 0 ||
        shape->arg_count != 0 || shape->var_count != 1 ||
        shape->defined_arg_count != 0 || shape->stack_size != 1 ||
        shape->var_ref_count != 0 || shape->closure_var_count != 0 ||
        shape->constant_pool_count != 1 ||
        shape->bytecode_size != sizeof(expected_code) ||
        shape->local_count != 1)
        return -1;

    if (cursor_leb128(&input, &local_name_atom) < 0 || local_name_atom != 0 ||
        cursor_leb128(&input, &local_scope_level) < 0 ||
        local_scope_level != 0 ||
        cursor_leb128(&input, &local_scope_next) < 0 ||
        local_scope_next != 0 || cursor_u8(&input, &local_flags) < 0 ||
        local_flags != 0)
        return -1;

    shape->code_offset = (size_t)(input.cursor - input.base);
    if (cursor_take(&input, shape->bytecode_size, &code) < 0 ||
        memcmp(code, expected_code, sizeof(expected_code)) != 0)
        return -1;

    shape->constant_pool_offset = (size_t)(input.cursor - input.base);
    shape->ancestor_reference_offset = shape->constant_pool_offset;
    if (cursor_u8(&input, &tag) < 0 || tag != BC_TAG_OBJECT_REFERENCE ||
        cursor_leb128(&input, &shape->ancestor_reference_id) < 0 ||
        shape->ancestor_reference_id != shape->root_reference_id ||
        input.cursor != input.end)
        return -1;
    return 0;
}

int main(void) {
    static const uint8_t bytecode[] = {
        0x05, 0x01, 0x02, 0x66, 0x08, 0x01, 0xe6, 0x03, 0x0c,
        0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01,
        0x00, 0x00, 0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00,
        0xbd, 0x00, 0xcb, 0x28, 0x13, 0x00,
    };
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue root = JS_UNDEFINED;
    JSValue function = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    AncestorReferenceShape parsed;
    size_t index;
    int same = 0;
    int status = 1;

    if (parse_ancestor_reference(bytecode, sizeof(bytecode), &parsed) < 0) {
        fputs("embedded bytecode lost the ancestor-reference layout\n", stderr);
        goto cleanup;
    }

    runtime = JS_NewRuntime();
    if (!runtime) {
        fputs("evaluation runtime allocation failed\n", stderr);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fputs("evaluation context allocation failed\n", stderr);
        goto cleanup;
    }

    root = JS_ReadObject(context, bytecode, sizeof(bytecode),
                         JS_READ_OBJ_BYTECODE | JS_READ_OBJ_REFERENCE);
    if (JS_IsException(root)) {
        report_exception(context, "bytecode deserialization failed");
        root = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsObject(root)) {
        fputs("bytecode root is not an object\n", stderr);
        goto cleanup;
    }
    function = JS_GetPropertyStr(context, root, "f");
    if (JS_IsException(function)) {
        report_exception(context, "loaded function access failed");
        function = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_EvalFunction(context, function);
    function = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(context, "fresh-runtime evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    same = JS_SameValue(context, result, root);
    if (same != 1) {
        fputs("fresh-runtime function did not return its root ancestor\n",
              stderr);
        goto cleanup;
    }

    puts("quickjs=2026-06-04");
    printf("read-flags=%d\n",
           JS_READ_OBJ_BYTECODE | JS_READ_OBJ_REFERENCE);
    printf("bytecode-version=%d\n", BC_VERSION);
    printf("bytecode-size=%zu\n", sizeof(bytecode));
    fputs("bytecode-hex=", stdout);
    for (index = 0; index < sizeof(bytecode); index++)
        printf("%02x", (unsigned)bytecode[index]);
    putchar('\n');
    printf("atom-count=%" PRIu32 "\n", parsed.atom_count);
    printf("root=offset:%zu,reference-id:%" PRIu32 ",properties:%" PRIu32
           "\n",
           parsed.root_offset, parsed.root_reference_id,
           parsed.root_property_count);
    puts("function-count=1");
    printf("function-0=offset:%zu,reference-id:none,cpool-offset:%zu,"
           "flags:%04x,mode:%u,args:%" PRIu32 ",vars:%" PRIu32
           ",defined-args:%" PRIu32 ",stack:%" PRIu32 ",var-refs:%"
           PRIu32 ",closures:%" PRIu32 ",cpool:%" PRIu32
           ",bytecode:%" PRIu32 ",locals:%" PRIu32 "\n",
           parsed.function_offset, parsed.constant_pool_offset,
           (unsigned)parsed.function_flags, (unsigned)parsed.js_mode,
           parsed.arg_count,
           parsed.var_count, parsed.defined_arg_count, parsed.stack_size,
           parsed.var_ref_count, parsed.closure_var_count,
           parsed.constant_pool_count, parsed.bytecode_size,
           parsed.local_count);
    printf("function-code=offset:%zu,hex=bd00cb28\n", parsed.code_offset);
    printf("cpool-root-reference=offset:%zu,target:%" PRIu32 "\n",
           parsed.ancestor_reference_offset, parsed.ancestor_reference_id);
    puts("ancestor-path=root.f.cpool[0]->root");
    printf("object-definition-count=%" PRIu32 "\n",
           parsed.object_definition_count);
    printf("fresh-function-root-same=%s\n",
           same == 1 ? "true" : "false");
    status = 0;

cleanup:
    if (context) {
        JS_FreeValue(context, result);
        JS_FreeValue(context, function);
        JS_FreeValue(context, root);
        JS_FreeContext(context);
    }
    if (runtime) {
        JS_RunGC(runtime);
        JS_FreeRuntime(runtime);
    }
    return status;
}
