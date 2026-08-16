#include "quickjs.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

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
