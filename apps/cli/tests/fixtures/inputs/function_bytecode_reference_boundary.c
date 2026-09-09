#include "quickjs.h"

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_UNDEFINED = 2,
    BC_TAG_STRING = 7,
    BC_TAG_OBJECT = 8,
    BC_TAG_TEMPLATE_OBJECT = 11,
    BC_TAG_FUNCTION_BYTECODE = 12,
    BC_TAG_OBJECT_REFERENCE = 19,
    EXPECTED_BYTECODE_SIZE = 75,
};

typedef struct ReferenceWireShape {
    uint32_t atom_count;
    size_t root_offset;
    uint32_t root_reference_id;
    size_t outer_function_offset;
    size_t outer_constant_pool_offset;
    uint32_t outer_constant_pool_count;
    size_t nested_function_offset;
    size_t nested_constant_pool_offset;
    uint32_t nested_constant_pool_count;
    size_t template_offset;
    uint32_t template_reference_id;
    size_t raw_template_offset;
    uint32_t raw_template_reference_id;
    size_t trailing_reference_offset;
    uint32_t trailing_reference_id;
    uint32_t object_definition_count;
} ReferenceWireShape;

static int parse_reference_wire(const uint8_t *bytecode,
                                size_t bytecode_size,
                                ReferenceWireShape *shape) {
    static const uint8_t atom_prefix[] = {
        BC_VERSION, 1, 16, 't', 'e', 'm', 'p', 'l', 'a', 't', 'e',
    };

    memset(shape, 0, sizeof(*shape));
    if (bytecode_size != EXPECTED_BYTECODE_SIZE ||
        memcmp(bytecode, atom_prefix, sizeof(atom_prefix)) != 0 ||
        bytecode[11] != BC_TAG_OBJECT || bytecode[12] != 2 ||
        bytecode[13] != 0x36 ||
        bytecode[14] != BC_TAG_FUNCTION_BYTECODE ||
        bytecode[26] != 2 || bytecode[27] != 7 ||
        bytecode[40] != BC_TAG_FUNCTION_BYTECODE ||
        bytecode[51] != 0 || bytecode[52] != 2 ||
        bytecode[60] != BC_TAG_TEMPLATE_OBJECT || bytecode[61] != 1 ||
        bytecode[62] != BC_TAG_STRING || bytecode[63] != 2 ||
        bytecode[64] != 'x' ||
        bytecode[65] != BC_TAG_TEMPLATE_OBJECT || bytecode[66] != 1 ||
        bytecode[67] != BC_TAG_STRING || bytecode[68] != 2 ||
        bytecode[69] != 'x' || bytecode[70] != BC_TAG_UNDEFINED ||
        bytecode[71] != 0xe6 || bytecode[72] != 0x03 ||
        bytecode[73] != BC_TAG_OBJECT_REFERENCE || bytecode[74] != 1)
        return -1;

    shape->atom_count = 1;
    shape->root_offset = 11;
    shape->root_reference_id = 0;
    shape->outer_function_offset = 14;
    shape->outer_constant_pool_offset = 40;
    shape->outer_constant_pool_count = 2;
    shape->nested_function_offset = 40;
    shape->nested_constant_pool_offset = 60;
    shape->nested_constant_pool_count = 0;
    shape->template_offset = 60;
    shape->template_reference_id = 1;
    shape->raw_template_offset = 65;
    shape->raw_template_reference_id = 2;
    shape->trailing_reference_offset = 73;
    shape->trailing_reference_id = 1;
    shape->object_definition_count = 3;
    return 0;
}

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
    static const char source[] = "((strings) => strings)`x`;";
    JSRuntime *compile_runtime = NULL;
    JSContext *compile_context = NULL;
    JSRuntime *eval_runtime = NULL;
    JSContext *eval_context = NULL;
    JSValue compiled = JS_UNDEFINED;
    JSValue warm_template = JS_UNDEFINED;
    JSValue root = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue loaded_function = JS_UNDEFINED;
    JSValue loaded_template = JS_UNDEFINED;
    JSValue evaluated_template = JS_UNDEFINED;
    JSValue first = JS_UNDEFINED;
    ReferenceWireShape parsed;
    uint8_t *bytecode = NULL;
    size_t bytecode_size = 0;
    const char *first_text = NULL;
    size_t index;
    int same = 0;
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
                       "reference-boundary.js",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    warm_template = JS_EvalFunction(
        compile_context, JS_DupValue(compile_context, compiled));
    if (JS_IsException(warm_template)) {
        report_exception(compile_context, "template evaluation failed");
        warm_template = JS_UNDEFINED;
        goto cleanup;
    }

    root = JS_NewObject(compile_context);
    if (JS_IsException(root) ||
        JS_SetPropertyStr(compile_context, root, "function",
                          JS_DupValue(compile_context, compiled)) < 0 ||
        JS_SetPropertyStr(compile_context, root, "template",
                          JS_DupValue(compile_context, warm_template)) < 0) {
        report_exception(compile_context, "root construction failed");
        goto cleanup;
    }
    bytecode = JS_WriteObject(compile_context, &bytecode_size, root,
                              JS_WRITE_OBJ_BYTECODE |
                                  JS_WRITE_OBJ_REFERENCE);
    if (!bytecode) {
        report_exception(compile_context, "bytecode serialization failed");
        goto cleanup;
    }
    if (parse_reference_wire(bytecode, bytecode_size, &parsed) < 0) {
        fputs("serialized bytecode lost the nested reference-boundary layout\n",
              stderr);
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
                           JS_READ_OBJ_BYTECODE | JS_READ_OBJ_REFERENCE);
    if (JS_IsException(loaded)) {
        report_exception(eval_context, "bytecode deserialization failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    loaded_function = JS_GetPropertyStr(eval_context, loaded, "function");
    loaded_template = JS_GetPropertyStr(eval_context, loaded, "template");
    if (JS_IsException(loaded_function) || JS_IsException(loaded_template)) {
        report_exception(eval_context, "loaded property access failed");
        goto cleanup;
    }
    evaluated_template = JS_EvalFunction(eval_context, loaded_function);
    loaded_function = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(evaluated_template)) {
        report_exception(eval_context, "fresh-runtime evaluation failed");
        evaluated_template = JS_UNDEFINED;
        goto cleanup;
    }
    same = JS_SameValue(eval_context, evaluated_template, loaded_template);
    first = JS_GetPropertyUint32(eval_context, evaluated_template, 0);
    if (JS_IsException(first)) {
        report_exception(eval_context, "template element access failed");
        first = JS_UNDEFINED;
        goto cleanup;
    }
    first_text = JS_ToCString(eval_context, first);
    if (!first_text) {
        report_exception(eval_context, "template element conversion failed");
        goto cleanup;
    }
    if (!same || strcmp(first_text, "x") != 0) {
        fprintf(stderr, "fresh-runtime mismatch: same=%d, first=%s\n", same,
                first_text);
        goto cleanup;
    }

    puts("quickjs=2026-06-04");
    fputs("source-hex=", stdout);
    for (index = 0; index < strlen(source); index++)
        printf("%02x", (unsigned char)source[index]);
    putchar('\n');
    printf("strip-flags=%d\n", JS_STRIP_DEBUG);
    printf("write-flags=%d\n",
           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_REFERENCE);
    printf("read-flags=%d\n",
           JS_READ_OBJ_BYTECODE | JS_READ_OBJ_REFERENCE);
    printf("bytecode-version=%d\n", BC_VERSION);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    for (index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("atom-count=%" PRIu32 "\n", parsed.atom_count);
    printf("root=offset:%zu,reference-id:%" PRIu32 ",properties:2\n",
           parsed.root_offset, parsed.root_reference_id);
    printf("function-count=2\n");
    printf("function-0=offset:%zu,reference-id:none,cpool-offset:%zu,cpool:%"
           PRIu32 "\n",
           parsed.outer_function_offset, parsed.outer_constant_pool_offset,
           parsed.outer_constant_pool_count);
    printf("function-1=offset:%zu,reference-id:none,cpool-offset:%zu,cpool:%"
           PRIu32 "\n",
           parsed.nested_function_offset, parsed.nested_constant_pool_offset,
           parsed.nested_constant_pool_count);
    puts("nested-function-path=root.function.cpool[0]");
    printf("cpool-template=offset:%zu,reference-id:%" PRIu32 "\n",
           parsed.template_offset, parsed.template_reference_id);
    printf("raw-template=offset:%zu,reference-id:%" PRIu32 "\n",
           parsed.raw_template_offset, parsed.raw_template_reference_id);
    printf("root-template-reference=offset:%zu,target:%" PRIu32 "\n",
           parsed.trailing_reference_offset, parsed.trailing_reference_id);
    printf("object-definition-count=%" PRIu32 "\n",
           parsed.object_definition_count);
    printf("fresh-function-template-same=%s\n", same ? "true" : "false");
    printf("fresh-template-first=%s\n", first_text);
    status = 0;

cleanup:
    if (eval_context) {
        if (first_text)
            JS_FreeCString(eval_context, first_text);
        JS_FreeValue(eval_context, first);
        JS_FreeValue(eval_context, evaluated_template);
        JS_FreeValue(eval_context, loaded_template);
        JS_FreeValue(eval_context, loaded_function);
        JS_FreeValue(eval_context, loaded);
        JS_FreeContext(eval_context);
    }
    if (eval_runtime)
        JS_FreeRuntime(eval_runtime);
    if (compile_context) {
        if (bytecode)
            js_free(compile_context, bytecode);
        JS_FreeValue(compile_context, root);
        JS_FreeValue(compile_context, warm_template);
        JS_FreeValue(compile_context, compiled);
        JS_FreeContext(compile_context);
    }
    if (compile_runtime)
        JS_FreeRuntime(compile_runtime);
    return status;
}
