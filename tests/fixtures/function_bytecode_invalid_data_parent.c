#include "quickjs.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_FUNCTION_BYTECODE = 12,
    BC_TAG_TYPED_ARRAY = 14,
    BC_TAG_DATE = 17,
    BC_TAG_OBJECT_VALUE = 18,
};

#define RETURN_42_FUNCTION_RECORD                                            \
    0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00,     \
        0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, \
        0x28

static const uint8_t return_42_function_record[] = {
    RETURN_42_FUNCTION_RECORD,
};

static const uint8_t object_value_bytecode[] = {
    0x05, 0x00, 0x12, RETURN_42_FUNCTION_RECORD,
};

static const uint8_t date_bytecode[] = {
    0x05, 0x00, 0x11, RETURN_42_FUNCTION_RECORD,
};

static const uint8_t typed_array_bytecode[] = {
    0x05, 0x00, 0x0e, 0x02, 0x01, 0x00, RETURN_42_FUNCTION_RECORD,
};

typedef struct BytecodeCursor {
    const uint8_t *cursor;
    const uint8_t *end;
} BytecodeCursor;

typedef struct FunctionShape {
    uint16_t flags;
    uint8_t mode;
    uint32_t name_atom;
    uint32_t arg_count;
    uint32_t var_count;
    uint32_t defined_arg_count;
    uint32_t stack_size;
    uint32_t var_ref_count;
    uint32_t closure_var_count;
    uint32_t constant_pool_count;
    uint32_t bytecode_size;
    uint32_t local_count;
    size_t code_relative_offset;
} FunctionShape;

typedef struct InvalidParentCase {
    const char *name;
    const uint8_t *bytecode;
    size_t bytecode_size;
    uint8_t parent_tag;
    const uint8_t *parent_wire;
    size_t parent_wire_size;
    size_t function_offset;
    int function_decoded;
    unsigned object_slots_before_function;
    unsigned object_slots_after_function;
    const char *expected_error;
} InvalidParentCase;

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

static int parse_return_42_function(const uint8_t *record, size_t record_size,
                                    FunctionShape *shape) {
    static const uint8_t expected_code[] = {0xbb, 0x2a, 0xcb, 0x28};
    BytecodeCursor input = {record, record + record_size};
    uint32_t local_name;
    uint32_t local_scope_level;
    uint32_t local_scope_next;
    uint8_t local_flags;
    uint8_t tag;

    memset(shape, 0, sizeof(*shape));
    if (record_size != sizeof(return_42_function_record) ||
        memcmp(record, return_42_function_record, record_size) != 0 ||
        cursor_u8(&input, &tag) < 0 || tag != BC_TAG_FUNCTION_BYTECODE ||
        cursor_u16(&input, &shape->flags) < 0 ||
        shape->flags != UINT16_C(0x0200) ||
        cursor_u8(&input, &shape->mode) < 0 || shape->mode != 0 ||
        cursor_leb128(&input, &shape->name_atom) < 0 ||
        shape->name_atom != 168 ||
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
        shape->constant_pool_count != 0 ||
        shape->bytecode_size != sizeof(expected_code) ||
        shape->local_count != 1 ||
        cursor_leb128(&input, &local_name) < 0 || local_name != 0 ||
        cursor_leb128(&input, &local_scope_level) < 0 ||
        local_scope_level != 0 ||
        cursor_leb128(&input, &local_scope_next) < 0 ||
        local_scope_next != 0 || cursor_u8(&input, &local_flags) < 0 ||
        local_flags != 0)
        return -1;

    shape->code_relative_offset = (size_t)(input.cursor - record);
    if ((size_t)(input.end - input.cursor) != sizeof(expected_code) ||
        memcmp(input.cursor, expected_code, sizeof(expected_code)) != 0)
        return -1;
    input.cursor += sizeof(expected_code);
    return input.cursor == input.end ? 0 : -1;
}

static int validate_case_layout(const InvalidParentCase *test,
                                FunctionShape *shape) {
    if (test->bytecode_size !=
            test->function_offset + sizeof(return_42_function_record) ||
        test->function_offset != 2 + test->parent_wire_size ||
        test->bytecode[0] != BC_VERSION || test->bytecode[1] != 0 ||
        test->parent_wire_size == 0 ||
        test->parent_wire[0] != test->parent_tag ||
        memcmp(test->bytecode + 2, test->parent_wire,
               test->parent_wire_size) != 0 ||
        parse_return_42_function(test->bytecode + test->function_offset,
                                 sizeof(return_42_function_record), shape) < 0)
        return -1;
    return 0;
}

static void print_hex(const uint8_t *bytes, size_t size) {
    size_t index;

    for (index = 0; index < size; index++)
        printf("%02x", (unsigned)bytes[index]);
}

static int expect_read_error(JSContext *ctx, const InvalidParentCase *test) {
    JSValue value = JS_ReadObject(ctx, test->bytecode, test->bytecode_size,
                                  JS_READ_OBJ_BYTECODE |
                                      JS_READ_OBJ_REFERENCE);
    JSValue exception;
    const char *message;
    int matches;

    if (!JS_IsException(value)) {
        JS_FreeValue(ctx, value);
        fprintf(stderr, "%s unexpectedly deserialized\n", test->name);
        return -1;
    }
    exception = JS_GetException(ctx);
    message = JS_ToCString(ctx, exception);
    if (!message) {
        JS_FreeValue(ctx, exception);
        fprintf(stderr, "%s exception could not be converted to text\n",
                test->name);
        return -1;
    }
    matches = strcmp(message, test->expected_error) == 0;
    if (!matches)
        fprintf(stderr, "%s returned %s, expected %s\n", test->name,
                message, test->expected_error);
    if (matches)
        printf("error=%s\n", message);
    JS_FreeCString(ctx, message);
    JS_FreeValue(ctx, exception);
    return matches ? 0 : -1;
}

static int expect_truncated_function_error(JSContext *ctx,
                                           const InvalidParentCase *test) {
    static const char expected[] =
        "SyntaxError: read after the end of the buffer";
    JSValue value = JS_ReadObject(ctx, test->bytecode,
                                  test->bytecode_size - 1,
                                  JS_READ_OBJ_BYTECODE |
                                      JS_READ_OBJ_REFERENCE);
    JSValue exception;
    const char *message;
    int matches;

    if (!JS_IsException(value)) {
        JS_FreeValue(ctx, value);
        fprintf(stderr, "%s truncated function unexpectedly deserialized\n",
                test->name);
        return -1;
    }
    exception = JS_GetException(ctx);
    message = JS_ToCString(ctx, exception);
    if (!message) {
        JS_FreeValue(ctx, exception);
        fprintf(stderr, "%s truncated exception was not text\n", test->name);
        return -1;
    }
    matches = strcmp(message, expected) == 0;
    if (!matches)
        fprintf(stderr, "%s truncated function returned %s, expected %s\n",
                test->name, message, expected);
    if (matches)
        printf("truncated-function-error=%s\n", message);
    JS_FreeCString(ctx, message);
    JS_FreeValue(ctx, exception);
    return matches ? 0 : -1;
}

int main(void) {
    static const uint8_t object_value_wire[] = {0x12};
    static const uint8_t date_wire[] = {0x11};
    static const uint8_t typed_array_wire[] = {0x0e, 0x02, 0x01, 0x00};
    static const InvalidParentCase cases[] = {
        {
            "object-value",
            object_value_bytecode,
            sizeof(object_value_bytecode),
            BC_TAG_OBJECT_VALUE,
            object_value_wire,
            sizeof(object_value_wire),
            3,
            1,
            0,
            0,
            "TypeError: cannot convert to object",
        },
        {
            "date",
            date_bytecode,
            sizeof(date_bytecode),
            BC_TAG_DATE,
            date_wire,
            sizeof(date_wire),
            3,
            1,
            0,
            0,
            "TypeError: Number tag expected for date",
        },
        {
            "typed-array",
            typed_array_bytecode,
            sizeof(typed_array_bytecode),
            BC_TAG_TYPED_ARRAY,
            typed_array_wire,
            sizeof(typed_array_wire),
            6,
            1,
            1,
            1,
            "TypeError: ArrayBuffer object expected",
        },
    };
    FunctionShape shapes[sizeof(cases) / sizeof(cases[0])];
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    size_t index;
    int status = 1;

    for (index = 0; index < sizeof(cases) / sizeof(cases[0]); index++) {
        if (validate_case_layout(&cases[index], &shapes[index]) < 0) {
            fprintf(stderr, "%s lost its authenticated input layout\n",
                    cases[index].name);
            goto cleanup;
        }
    }

    runtime = JS_NewRuntime();
    if (!runtime) {
        fputs("runtime allocation failed\n", stderr);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fputs("context allocation failed\n", stderr);
        goto cleanup;
    }

    puts("quickjs=2026-06-04");
    printf("read-flags=%d\n",
           JS_READ_OBJ_BYTECODE | JS_READ_OBJ_REFERENCE);
    printf("bytecode-version=%d\n", BC_VERSION);
    puts("atom-count=0");
    printf("function-record-size=%zu\n", sizeof(return_42_function_record));
    fputs("function-record-hex=", stdout);
    print_hex(return_42_function_record,
              sizeof(return_42_function_record));
    putchar('\n');
    puts("function-record-reference-id=none");

    for (index = 0; index < sizeof(cases) / sizeof(cases[0]); index++) {
        const InvalidParentCase *test = &cases[index];
        const FunctionShape *shape = &shapes[index];

        printf("case=%s\n", test->name);
        printf("bytecode-size=%zu\n", test->bytecode_size);
        fputs("bytecode-hex=", stdout);
        print_hex(test->bytecode, test->bytecode_size);
        putchar('\n');
        printf("parent=offset:2,tag:%02x,wire:",
               (unsigned)test->parent_tag);
        print_hex(test->parent_wire, test->parent_wire_size);
        putchar('\n');
        printf("function=offset:%zu,decoded:%s,reference-id:none,"
               "code-offset:%zu,flags:%04x,mode:%u,name-atom:%u,"
               "args:%u,vars:%u,defined-args:%u,stack:%u,var-refs:%u,"
               "closures:%u,cpool:%u,bytecode:%u,locals:%u\n",
               test->function_offset,
               test->function_decoded ? "true" : "false",
               test->function_offset + shape->code_relative_offset,
               (unsigned)shape->flags, (unsigned)shape->mode,
               (unsigned)shape->name_atom, (unsigned)shape->arg_count,
               (unsigned)shape->var_count,
               (unsigned)shape->defined_arg_count,
               (unsigned)shape->stack_size,
               (unsigned)shape->var_ref_count,
               (unsigned)shape->closure_var_count,
               (unsigned)shape->constant_pool_count,
               (unsigned)shape->bytecode_size,
               (unsigned)shape->local_count);
        printf("object-reference-slots=before-function:%u,"
               "after-function:%u\n",
               test->object_slots_before_function,
               test->object_slots_after_function);
        if (expect_read_error(context, test) < 0)
            goto cleanup;
        if (expect_truncated_function_error(context, test) < 0)
            goto cleanup;
    }
    status = 0;

cleanup:
    if (context)
        JS_FreeContext(context);
    if (runtime) {
        JS_RunGC(runtime);
        JS_FreeRuntime(runtime);
    }
    return status;
}
