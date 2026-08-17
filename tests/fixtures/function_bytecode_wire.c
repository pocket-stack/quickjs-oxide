#include "quickjs.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static const uint8_t expected_bytecode[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
};

static const uint8_t compatible_scope_next_wrap[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x80, 0x80, 0x80, 0x80, 0x08, 0x00, 0x00, 0xbb, 0x2a,
    0xcb, 0x28,
};

static const uint8_t array_buffer_max_below_length[] = {
    0x05, 0x00, 0x0f, 0x01, 0x00,
};

static const uint8_t invalid_typed_array_kind[] = {
    0x05, 0x00, 0x0e, 0xff,
};

static const uint8_t malformed_uleb[] = {
    0x05, 0x80, 0x80, 0x80, 0x80, 0x80,
};

static const uint8_t invalid_metadata_atom[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xe6, 0x03, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
};

static const uint8_t hard_string_length[] = {
    0x05, 0x01, 0x80, 0x80, 0x80, 0x80, 0x08,
};

static const uint8_t bytecode_length_overflow[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x80, 0x80,
    0x80, 0x08, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a,
    0xcb, 0x28,
};

static const uint8_t uint16_array_unaligned_offset[] = {
    0x05, 0x00, 0x0e, 0x04, 0x00, 0x01, 0x0f, 0x00, 0x00,
};

static const uint8_t typed_array_view_beyond_backing[] = {
    0x05, 0x00, 0x0e, 0x02, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x00,
};

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

static int expect_read_exception(const char *label,
                                 const uint8_t *bytecode,
                                 size_t bytecode_size,
                                 const char *expected_class,
                                 const char *expected_message) {
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    JSValue class_value = JS_UNDEFINED;
    JSValue message_value = JS_UNDEFINED;
    const char *actual_class = NULL;
    const char *actual_message = NULL;
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", label);
        goto cleanup;
    }

    loaded = JS_ReadObject(context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (!JS_IsException(loaded)) {
        fprintf(stderr, "%s bytecode was unexpectedly accepted\n", label);
        goto cleanup;
    }
    exception = JS_GetException(context);
    if (!JS_IsError(context, exception)) {
        fprintf(stderr, "%s did not throw an Error object\n", label);
        goto cleanup;
    }

    class_value = JS_GetPropertyStr(context, exception, "name");
    message_value = JS_GetPropertyStr(context, exception, "message");
    if (JS_IsException(class_value) || JS_IsException(message_value)) {
        report_exception(context, "malformed-read exception inspection failed");
        goto cleanup;
    }
    actual_class = JS_ToCString(context, class_value);
    actual_message = JS_ToCString(context, message_value);
    if (!actual_class || !actual_message) {
        report_exception(context, "malformed-read exception conversion failed");
        goto cleanup;
    }
    if (strcmp(actual_class, expected_class) != 0 ||
        strcmp(actual_message, expected_message) != 0) {
        fprintf(stderr,
                "%s returned %s: %s, expected %s: %s\n",
                label, actual_class, actual_message,
                expected_class, expected_message);
        goto cleanup;
    }

    printf("%s-hex=", label);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("%s-class=%s\n", label, actual_class);
    printf("%s-message=%s\n", label, actual_message);
    status = 0;

cleanup:
    if (context) {
        if (actual_message)
            JS_FreeCString(context, actual_message);
        if (actual_class)
            JS_FreeCString(context, actual_class);
        JS_FreeValue(context, message_value);
        JS_FreeValue(context, class_value);
        JS_FreeValue(context, exception);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int expect_read_number(const char *label,
                              const uint8_t *bytecode,
                              size_t bytecode_size,
                              double expected) {
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    double actual = 0;
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", label);
        goto cleanup;
    }

    loaded = JS_ReadObject(context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "compatible bytecode read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(context, "compatible bytecode evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsNumber(result) || JS_ToFloat64(context, &actual, result) < 0) {
        fprintf(stderr, "%s did not evaluate to a number\n", label);
        goto cleanup;
    }
    if (actual != expected) {
        fprintf(stderr, "%s evaluated to %.17g, expected %.17g\n",
                label, actual, expected);
        goto cleanup;
    }

    printf("%s-hex=", label);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("%s-eval=%.17g\n", label, actual);
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

int main(void) {
    static const char source[] = "42;";
    JSRuntime *compile_runtime = NULL;
    JSContext *compile_context = NULL;
    JSRuntime *eval_runtime = NULL;
    JSContext *eval_context = NULL;
    JSValue compiled = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    uint8_t *bytecode = NULL;
    size_t bytecode_size = 0;
    double evaluated = 0;
    uint8_t wrong_version[sizeof(expected_bytecode)];
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
                       "return-42.js",
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
    if (bytecode_size != sizeof(expected_bytecode) ||
        memcmp(bytecode, expected_bytecode, sizeof(expected_bytecode)) != 0) {
        fputs("serialized bytecode did not match the pinned BC5 blob\n", stderr);
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
    if (!JS_IsNumber(result) || JS_ToFloat64(eval_context, &evaluated, result) < 0) {
        fputs("fresh-runtime evaluation did not return a number\n", stderr);
        goto cleanup;
    }
    if (evaluated != 42.0) {
        fprintf(stderr, "fresh-runtime evaluation returned %.17g, expected 42\n",
                evaluated);
        goto cleanup;
    }

    puts("quickjs=2026-06-04");
    puts("source-hex=34323b");
    printf("strip-flags=%d\n", JS_STRIP_DEBUG);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("fresh-eval=%.17g\n", evaluated);

    if (expect_read_number("scope-next-wrap", compatible_scope_next_wrap,
                           sizeof(compatible_scope_next_wrap), 42))
        goto cleanup;
    memcpy(wrong_version, expected_bytecode, sizeof(wrong_version));
    wrong_version[0] = 4;
    if (expect_read_exception("wrong-version", wrong_version,
                              sizeof(wrong_version),
                              "SyntaxError", "invalid version (4 expected=5)"))
        goto cleanup;
    if (expect_read_exception("truncated", expected_bytecode,
                              sizeof(expected_bytecode) - 1,
                              "SyntaxError", "read after the end of the buffer"))
        goto cleanup;
    if (expect_read_exception("malformed-uleb", malformed_uleb,
                              sizeof(malformed_uleb),
                              "SyntaxError", "read after the end of the buffer"))
        goto cleanup;
    if (expect_read_exception("invalid-metadata-atom",
                              invalid_metadata_atom,
                              sizeof(invalid_metadata_atom),
                              "SyntaxError", "invalid atom index (pos=8)"))
        goto cleanup;
    if (expect_read_exception("hard-string-length", hard_string_length,
                              sizeof(hard_string_length),
                              "InternalError", "string too long"))
        goto cleanup;
    if (expect_read_exception("bytecode-length-overflow",
                              bytecode_length_overflow,
                              sizeof(bytecode_length_overflow),
                              "InternalError", "out of memory"))
        goto cleanup;
    if (expect_read_exception("array-buffer-max-below-length",
                              array_buffer_max_below_length,
                              sizeof(array_buffer_max_below_length),
                              "TypeError", "invalid array buffer"))
        goto cleanup;
    if (expect_read_exception("invalid-typed-array-kind",
                              invalid_typed_array_kind,
                              sizeof(invalid_typed_array_kind),
                              "TypeError", "invalid typed array"))
        goto cleanup;
    if (expect_read_exception("uint16-array-unaligned-offset",
                              uint16_array_unaligned_offset,
                              sizeof(uint16_array_unaligned_offset),
                              "RangeError", "invalid offset"))
        goto cleanup;
    if (expect_read_exception("typed-array-view-beyond-backing",
                              typed_array_view_beyond_backing,
                              sizeof(typed_array_view_beyond_backing),
                              "RangeError", "invalid length"))
        goto cleanup;
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
