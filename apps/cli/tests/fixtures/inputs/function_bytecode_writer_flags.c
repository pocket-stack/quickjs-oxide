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
    uint32_t filename_atom;
    const uint8_t *pc2line;
    uint32_t pc2line_size;
    const uint8_t *source;
    uint32_t source_size;
} FunctionShape;

typedef struct BytecodeCursor {
    const uint8_t *base;
    const uint8_t *cursor;
    const uint8_t *end;
    uint32_t atom_count;
    size_t function_count;
    FunctionShape functions[EXPECTED_FUNCTION_COUNT];
} BytecodeCursor;

typedef struct StripCase {
    const char *name;
    int strip_flags;
    int expect_debug;
    int expect_source;
} StripCase;

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

static int cursor_atom(BytecodeCursor *input, uint32_t *atom) {
    return cursor_leb128(input, atom);
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
        cursor_u8(input, &shape->js_mode) < 0 ||
        cursor_atom(input, &unused) < 0 ||
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
        if (cursor_atom(input, &unused) < 0 ||
            cursor_leb128(input, &unused) < 0 ||
            cursor_leb128(input, &unused) < 0 ||
            cursor_u8(input, &unused_u8) < 0)
            return -1;
    }
    for (index = 0; index < shape->closure_var_count; index++) {
        if (cursor_atom(input, &unused) < 0 ||
            cursor_leb128(input, &unused) < 0 ||
            cursor_u16(input, &unused_u16) < 0)
            return -1;
    }
    if (cursor_take(input, shape->bytecode_size) < 0)
        return -1;

    if ((shape->flags & (UINT16_C(1) << 10)) != 0) {
        if (cursor_atom(input, &shape->filename_atom) < 0 ||
            cursor_leb128(input, &shape->pc2line_size) < 0)
            return -1;
        shape->pc2line = input->cursor;
        if (cursor_take(input, shape->pc2line_size) < 0 ||
            cursor_leb128(input, &shape->source_size) < 0)
            return -1;
        shape->source = input->cursor;
        if (cursor_take(input, shape->source_size) < 0)
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

static int validate_shape(const BytecodeCursor *parsed,
                          const StripCase *test) {
    size_t index;

    if (parsed->function_count != EXPECTED_FUNCTION_COUNT ||
        parsed->functions[0].constant_pool_count != 1 ||
        parsed->functions[1].constant_pool_count != 1 ||
        parsed->functions[2].constant_pool_count != 0 ||
        parsed->functions[2].closure_var_count != 1)
        return -1;

    for (index = 0; index < parsed->function_count; index++) {
        const FunctionShape *shape = &parsed->functions[index];
        int has_debug = (shape->flags & (UINT16_C(1) << 10)) != 0;

        if (has_debug != test->expect_debug)
            return -1;
        if (test->expect_debug) {
            if (shape->pc2line_size == 0)
                return -1;
            if (test->expect_source) {
                if ((index == 0 && shape->source_size != 0) ||
                    (index != 0 && shape->source_size == 0))
                    return -1;
            } else if (shape->source_size != 0) {
                return -1;
            }
        } else if (shape->filename_atom != 0 || shape->pc2line_size != 0 ||
                   shape->source_size != 0) {
            return -1;
        }
    }
    return 0;
}

static void print_hex(const uint8_t *bytes, size_t size) {
    size_t index;

    for (index = 0; index < size; index++)
        printf("%02x", bytes[index]);
}

static int evaluate_fresh(const uint8_t *bytecode, size_t bytecode_size,
                          const char *operation, double *evaluated) {
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s: runtime allocation failed\n", operation);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s: context allocation failed\n", operation);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, operation);
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(context, operation);
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsNumber(result) ||
        JS_ToFloat64(context, evaluated, result) < 0 || *evaluated != 42.0) {
        fprintf(stderr, "%s: expected the number 42\n", operation);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (context) {
        JS_FreeValue(context, result);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int run_case(const StripCase *test, const char *source) {
    JSRuntime *compile_runtime = NULL;
    JSContext *compile_context = NULL;
    JSValue compiled = JS_UNDEFINED;
    BytecodeCursor parsed;
    uint8_t *bytecode = NULL;
    uint8_t *bswap_bytecode = NULL;
    size_t bytecode_size = 0;
    size_t bswap_bytecode_size = 0;
    double bytecode_result = 0;
    double bswap_result = 0;
    size_t index;
    int status = -1;

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

    JS_SetStripInfo(compile_runtime, test->strip_flags);
    if (JS_GetStripInfo(compile_runtime) != test->strip_flags) {
        fprintf(stderr, "%s: strip flags were not retained\n", test->name);
        goto cleanup;
    }
    compiled = JS_Eval(compile_context, source, strlen(source),
                       "writer-flags.js",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "compile failed");
        goto cleanup;
    }

    bytecode = JS_WriteObject(compile_context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(compile_context, "BYTECODE serialization failed");
        goto cleanup;
    }
    bswap_bytecode = JS_WriteObject(
        compile_context, &bswap_bytecode_size, compiled,
        JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    if (!bswap_bytecode) {
        report_exception(compile_context,
                         "BYTECODE|BSWAP serialization failed");
        goto cleanup;
    }
    if (bytecode_size != bswap_bytecode_size ||
        memcmp(bytecode, bswap_bytecode, bytecode_size) != 0) {
        fprintf(stderr, "%s: BSWAP changed the serialized bytes\n", test->name);
        goto cleanup;
    }
    if (parse_bytecode(bytecode, bytecode_size, &parsed) < 0) {
        fprintf(stderr, "%s: serialized function grammar was invalid\n",
                test->name);
        goto cleanup;
    }
    if (validate_shape(&parsed, test) < 0) {
        fprintf(stderr, "%s: serialized function shape was invalid: functions=%zu\n",
                test->name, parsed.function_count);
        for (index = 0; index < parsed.function_count; index++) {
            const FunctionShape *shape = &parsed.functions[index];

            fprintf(stderr,
                    "function-%zu: flags=%04x closures=%u cpool=%u pc2line=%u "
                    "source=%u\n",
                    index, shape->flags, shape->closure_var_count,
                    shape->constant_pool_count, shape->pc2line_size,
                    shape->source_size);
        }
        goto cleanup;
    }
    if (evaluate_fresh(bytecode, bytecode_size, "BYTECODE fresh evaluation",
                       &bytecode_result) < 0 ||
        evaluate_fresh(bswap_bytecode, bswap_bytecode_size,
                       "BYTECODE|BSWAP fresh evaluation", &bswap_result) < 0)
        goto cleanup;

    printf("case=%s\n", test->name);
    printf("strip-flags=%d\n", test->strip_flags);
    printf("write-flags=%d,%d\n", JS_WRITE_OBJ_BYTECODE,
           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    print_hex(bytecode, bytecode_size);
    putchar('\n');
    puts("bswap-identical=true");
    printf("atom-count=%u\n", parsed.atom_count);
    printf("function-count=%zu\n", parsed.function_count);
    for (index = 0; index < parsed.function_count; index++) {
        const FunctionShape *shape = &parsed.functions[index];

        printf("function-%zu=offset:%zu,cpool-offset:%zu,flags:%04x,mode:%u,"
               "args:%u,vars:%u,defined-args:%u,stack:%u,var-refs:%u,"
               "closures:%u,cpool:%u,bytecode:%u,locals:%u,debug:%u,"
               "filename-atom:%u,pc2line:%u,source:%u\n",
               index, shape->record_offset, shape->constant_pool_offset,
               shape->flags, shape->js_mode, shape->arg_count,
               shape->var_count, shape->defined_arg_count, shape->stack_size,
               shape->var_ref_count, shape->closure_var_count,
               shape->constant_pool_count, shape->bytecode_size,
               shape->local_count,
               (shape->flags & (UINT16_C(1) << 10)) != 0,
               shape->filename_atom, shape->pc2line_size,
               shape->source_size);
        printf("function-%zu-pc2line-hex=", index);
        print_hex(shape->pc2line, shape->pc2line_size);
        putchar('\n');
        printf("function-%zu-source-hex=", index);
        print_hex(shape->source, shape->source_size);
        putchar('\n');
    }
    puts("nested-function-path=0.cpool[0].cpool[0]");
    printf("fresh-eval-bytecode=%.17g\n", bytecode_result);
    printf("fresh-eval-bswap=%.17g\n", bswap_result);
    status = 0;

cleanup:
    if (compile_context) {
        if (bswap_bytecode)
            js_free(compile_context, bswap_bytecode);
        if (bytecode)
            js_free(compile_context, bytecode);
        JS_FreeValue(compile_context, compiled);
        JS_FreeContext(compile_context);
    }
    if (compile_runtime)
        JS_FreeRuntime(compile_runtime);
    return status;
}

int main(void) {
    static const char source[] =
        "(function outer(seed) {\n"
        "  return function answer() {\n"
        "    return seed + 2;\n"
        "  };\n"
        "})(40)();";
    static const StripCase cases[] = {
        {"keep-source", 0, 1, 1},
        {"strip-source", JS_STRIP_SOURCE, 1, 0},
        {"strip-debug", JS_STRIP_DEBUG, 0, 0},
    };
    size_t index;

    puts("quickjs=2026-06-04");
    fputs("source-hex=", stdout);
    print_hex((const uint8_t *)source, strlen(source));
    putchar('\n');
    for (index = 0; index < sizeof(cases) / sizeof(cases[0]); index++) {
        if (run_case(&cases[index], source) < 0)
            return 1;
    }
    return 0;
}
