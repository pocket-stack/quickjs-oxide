#include "quickjs.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_FUNCTION_BYTECODE = 12,
    EXPECTED_FUNCTION_COUNT = 3,
};

typedef struct FunctionShape {
    size_t record_offset;
    size_t constant_pool_offset;
    uint16_t flags;
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
} FunctionShape;

typedef struct BytecodeCursor {
    const uint8_t *base;
    const uint8_t *cursor;
    const uint8_t *end;
    uint32_t atom_count;
    size_t function_count;
    FunctionShape functions[EXPECTED_FUNCTION_COUNT];
} BytecodeCursor;

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

static int cursor_take(BytecodeCursor *input, size_t count) {
    if ((size_t)(input->end - input->cursor) < count)
        return -1;
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

static int cursor_atom(BytecodeCursor *input) {
    uint32_t unused;

    return cursor_leb128(input, &unused);
}

static int parse_function(BytecodeCursor *input);

static int parse_value(BytecodeCursor *input) {
    uint8_t tag;

    if (cursor_u8(input, &tag) < 0 || tag != BC_TAG_FUNCTION_BYTECODE)
        return -1;
    return parse_function(input);
}

static int parse_function(BytecodeCursor *input) {
    FunctionShape *shape;
    uint32_t index;
    uint32_t unused;
    uint8_t unused_u8;
    uint16_t unused_u16;

    if (input->function_count == EXPECTED_FUNCTION_COUNT)
        return -1;
    shape = &input->functions[input->function_count++];
    shape->record_offset = (size_t)(input->cursor - input->base) - 1;

    if (cursor_u16(input, &shape->flags) < 0 ||
        cursor_u8(input, &shape->js_mode) < 0 || cursor_atom(input) < 0 ||
        cursor_leb128(input, &shape->arg_count) < 0 ||
        cursor_leb128(input, &shape->var_count) < 0 ||
        cursor_leb128(input, &shape->defined_arg_count) < 0 ||
        cursor_leb128(input, &shape->stack_size) < 0 ||
        cursor_leb128(input, &shape->var_ref_count) < 0 ||
        cursor_leb128(input, &shape->closure_var_count) < 0 ||
        cursor_leb128(input, &shape->constant_pool_count) < 0 ||
        cursor_leb128(input, &shape->bytecode_size) < 0 ||
        cursor_leb128(input, &shape->local_count) < 0)
        return -1;

    for (index = 0; index < shape->local_count; index++) {
        if (cursor_atom(input) < 0 || cursor_leb128(input, &unused) < 0 ||
            cursor_leb128(input, &unused) < 0 ||
            cursor_u8(input, &unused_u8) < 0)
            return -1;
    }
    for (index = 0; index < shape->closure_var_count; index++) {
        if (cursor_atom(input) < 0 || cursor_leb128(input, &unused) < 0 ||
            cursor_u16(input, &unused_u16) < 0)
            return -1;
    }
    if (cursor_take(input, shape->bytecode_size) < 0)
        return -1;

    if ((shape->flags & (UINT16_C(1) << 10)) != 0) {
        uint32_t debug_size;

        if (cursor_atom(input) < 0 || cursor_leb128(input, &debug_size) < 0 ||
            cursor_take(input, debug_size) < 0 ||
            cursor_leb128(input, &debug_size) < 0 ||
            cursor_take(input, debug_size) < 0)
            return -1;
    }

    shape->constant_pool_offset = (size_t)(input->cursor - input->base);
    for (index = 0; index < shape->constant_pool_count; index++) {
        if (parse_value(input) < 0)
            return -1;
    }
    return 0;
}

static int parse_bytecode(const uint8_t *bytecode, size_t bytecode_size,
                          BytecodeCursor *parsed) {
    uint8_t version;
    uint32_t index;

    memset(parsed, 0, sizeof(*parsed));
    parsed->base = bytecode;
    parsed->cursor = bytecode;
    parsed->end = bytecode + bytecode_size;
    if (cursor_u8(parsed, &version) < 0 || version != BC_VERSION ||
        cursor_leb128(parsed, &parsed->atom_count) < 0)
        return -1;
    for (index = 0; index < parsed->atom_count; index++) {
        uint32_t string_header;
        size_t byte_size;

        if (cursor_leb128(parsed, &string_header) < 0)
            return -1;
        byte_size = (size_t)(string_header >> 1);
        if ((string_header & 1) != 0)
            byte_size *= 2;
        if (cursor_take(parsed, byte_size) < 0)
            return -1;
    }
    if (parse_value(parsed) < 0 || parsed->cursor != parsed->end)
        return -1;
    return 0;
}

static int validate_nested_shape(const BytecodeCursor *parsed) {
    const FunctionShape *root = &parsed->functions[0];
    const FunctionShape *outer = &parsed->functions[1];
    const FunctionShape *inner = &parsed->functions[2];

    return parsed->function_count == EXPECTED_FUNCTION_COUNT &&
           root->closure_var_count == 0 && root->constant_pool_count == 1 &&
           outer->closure_var_count == 0 && outer->constant_pool_count == 1 &&
           inner->closure_var_count == 1 && inner->constant_pool_count == 0;
}

int main(void) {
    static const char source[] =
        "(function outer(seed) { let captured = seed; return function "
        "inner(delta) { captured += delta; return captured; }; })(40)(2);";
    JSRuntime *compile_runtime = NULL;
    JSContext *compile_context = NULL;
    JSRuntime *eval_runtime = NULL;
    JSContext *eval_context = NULL;
    JSValue compiled = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    BytecodeCursor parsed;
    uint8_t *bytecode = NULL;
    size_t bytecode_size = 0;
    double evaluated = 0;
    size_t index;
    int status = 1;

    compile_runtime = JS_NewRuntime();
    if (!compile_runtime) {
        fputs("compile runtime allocation failed\n", stderr);
        goto cleanup;
    }
    compile_context = JS_NewContext(compile_runtime);
    if (!compile_context) {
        fputs("compile context allocation failed\n", stderr);
        goto cleanup;
    }

    JS_SetStripInfo(compile_runtime, JS_STRIP_DEBUG);
    compiled = JS_Eval(compile_context, source, strlen(source),
                       "nested-closure.js",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "compile failed");
        goto cleanup;
    }

    bytecode = JS_WriteObject(compile_context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(compile_context, "bytecode serialization failed");
        goto cleanup;
    }
    if (parse_bytecode(bytecode, bytecode_size, &parsed) < 0) {
        fputs("serialized bytecode did not match the BC5 function grammar\n",
              stderr);
        goto cleanup;
    }
    if (!validate_nested_shape(&parsed)) {
        fputs("serialized bytecode lost the nested closure topology\n", stderr);
        goto cleanup;
    }

    eval_runtime = JS_NewRuntime();
    if (!eval_runtime) {
        fputs("evaluation runtime allocation failed\n", stderr);
        goto cleanup;
    }
    eval_context = JS_NewContext(eval_runtime);
    if (!eval_context) {
        fputs("evaluation context allocation failed\n", stderr);
        goto cleanup;
    }

    loaded = JS_ReadObject(eval_context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(eval_context, "bytecode deserialization failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_EvalFunction(eval_context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(eval_context, "fresh-runtime evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsNumber(result) ||
        JS_ToFloat64(eval_context, &evaluated, result) < 0) {
        fputs("fresh-runtime evaluation did not return a number\n", stderr);
        goto cleanup;
    }
    if (evaluated != 42.0) {
        fprintf(stderr,
                "fresh-runtime evaluation returned %.17g, expected 42\n",
                evaluated);
        goto cleanup;
    }

    puts("quickjs=2026-06-04");
    fputs("source-hex=", stdout);
    for (index = 0; index < strlen(source); index++)
        printf("%02x", (unsigned char)source[index]);
    putchar('\n');
    printf("strip-flags=%d\n", JS_STRIP_DEBUG);
    printf("bytecode-version=%d\n", BC_VERSION);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    for (index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("atom-count=%u\n", parsed.atom_count);
    printf("function-count=%zu\n", parsed.function_count);
    for (index = 0; index < parsed.function_count; index++) {
        const FunctionShape *shape = &parsed.functions[index];

        printf("function-%zu=offset:%zu,cpool-offset:%zu,flags:%04x,mode:%u,"
               "args:%u,vars:%u,defined-args:%u,stack:%u,var-refs:%u,"
               "closures:%u,cpool:%u,bytecode:%u,locals:%u\n",
               index, shape->record_offset, shape->constant_pool_offset,
               shape->flags, shape->js_mode, shape->arg_count,
               shape->var_count, shape->defined_arg_count, shape->stack_size,
               shape->var_ref_count, shape->closure_var_count,
               shape->constant_pool_count, shape->bytecode_size,
               shape->local_count);
    }
    puts("nested-function-path=0.cpool[0].cpool[0]");
    printf("inner-captured-closure-count=%u\n",
           parsed.functions[2].closure_var_count);
    printf("fresh-eval=%.17g\n", evaluated);
    status = 0;

cleanup:
    if (eval_context) {
        JS_FreeValue(eval_context, result);
        JS_FreeValue(eval_context, loaded);
        JS_FreeContext(eval_context);
    }
    if (eval_runtime)
        JS_FreeRuntime(eval_runtime);
    if (compile_context) {
        if (bytecode)
            js_free(compile_context, bytecode);
        JS_FreeValue(compile_context, compiled);
        JS_FreeContext(compile_context);
    }
    if (compile_runtime)
        JS_FreeRuntime(compile_runtime);
    return status;
}
