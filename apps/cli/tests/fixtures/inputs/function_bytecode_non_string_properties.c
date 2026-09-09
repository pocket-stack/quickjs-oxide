#include "quickjs.h"

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef struct FreshShape {
    char keys[16];
    int32_t symbol_count;
    uint32_t private_count;
    int32_t keep;
} FreshShape;

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

static void print_hex(const uint8_t *bytes, size_t size) {
    size_t index;

    for (index = 0; index < size; index++)
        printf("%02x", bytes[index]);
}

static int take_single_enumerable_value(JSContext *ctx, JSValueConst root,
                                        int name_mask, const char *kind,
                                        JSValue *value) {
    JSPropertyEnum *names = NULL;
    JSPropertyDescriptor descriptor = {
        0, JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED,
    };
    uint32_t count = 0;
    int found;
    int status = -1;

    if (JS_GetOwnPropertyNames(ctx, &names, &count, root,
                               name_mask | JS_GPN_ENUM_ONLY |
                                   JS_GPN_SET_ENUM) < 0) {
        report_exception(ctx, kind);
        goto cleanup;
    }
    if (count != 1 || !names[0].is_enumerable) {
        fprintf(stderr, "%s: expected one enumerable property, got %u\n",
                kind, count);
        goto cleanup;
    }
    found = JS_GetOwnProperty(ctx, &descriptor, root, names[0].atom);
    if (found < 0) {
        report_exception(ctx, kind);
        goto cleanup;
    }
    if (found != 1 || descriptor.flags != JS_PROP_C_W_E ||
        !JS_IsObject(descriptor.value)) {
        fprintf(stderr, "%s: expected one C_W_E object value\n", kind);
        goto cleanup;
    }
    *value = descriptor.value;
    descriptor.value = JS_UNDEFINED;
    status = 0;

cleanup:
    JS_FreeValue(ctx, descriptor.setter);
    JS_FreeValue(ctx, descriptor.getter);
    JS_FreeValue(ctx, descriptor.value);
    JS_FreePropertyEnum(ctx, names, count);
    return status;
}

static int validate_source_shape(JSContext *ctx, JSValueConst root) {
    JSPropertyEnum *strings = NULL;
    JSValue symbol_cycle = JS_UNDEFINED;
    JSValue private_cycle = JS_UNDEFINED;
    JSValue self = JS_UNDEFINED;
    const char *name = NULL;
    uint32_t string_count = 0;
    int status = -1;

    if (JS_GetOwnPropertyNames(ctx, &strings, &string_count, root,
                               JS_GPN_STRING_MASK | JS_GPN_ENUM_ONLY |
                                   JS_GPN_SET_ENUM) < 0) {
        report_exception(ctx, "source string properties");
        goto cleanup;
    }
    if (string_count != 1 || !strings[0].is_enumerable) {
        fprintf(stderr, "source string properties: expected one, got %u\n",
                string_count);
        goto cleanup;
    }
    name = JS_AtomToCString(ctx, strings[0].atom);
    if (!name || strcmp(name, "keep") != 0) {
        fputs("source string property is not keep\n", stderr);
        goto cleanup;
    }
    JS_FreeCString(ctx, name);
    name = NULL;

    if (take_single_enumerable_value(ctx, root, JS_GPN_SYMBOL_MASK,
                                     "source symbol property",
                                     &symbol_cycle) < 0 ||
        take_single_enumerable_value(ctx, root, JS_GPN_PRIVATE_MASK,
                                     "source private property",
                                     &private_cycle) < 0)
        goto cleanup;
    if (JS_SameValue(ctx, symbol_cycle, private_cycle) != 1) {
        fputs("source non-string values do not share identity\n", stderr);
        goto cleanup;
    }
    self = JS_GetPropertyStr(ctx, symbol_cycle, "self");
    if (JS_IsException(self)) {
        report_exception(ctx, "source cycle self property");
        self = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_SameValue(ctx, self, symbol_cycle) != 1) {
        fputs("source non-string value is not circular\n", stderr);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (name)
        JS_FreeCString(ctx, name);
    JS_FreeValue(ctx, self);
    JS_FreeValue(ctx, private_cycle);
    JS_FreeValue(ctx, symbol_cycle);
    JS_FreePropertyEnum(ctx, strings, string_count);
    return status;
}

static int validate_fresh(const uint8_t *bytes, size_t size,
                          const char *operation, FreshShape *shape) {
    static const char checker_source[] =
        "root => [Object.keys(root).join(','), "
        "Object.getOwnPropertySymbols(root).length, root.keep]";
    JSRuntime *runtime = NULL;
    JSContext *ctx = NULL;
    JSPropertyEnum *private_names = NULL;
    JSValue root = JS_UNDEFINED;
    JSValue checker = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue keys = JS_UNDEFINED;
    JSValue symbols = JS_UNDEFINED;
    JSValue keep = JS_UNDEFINED;
    const char *keys_text = NULL;
    uint32_t private_count = 0;
    int status = -1;

    memset(shape, 0, sizeof(*shape));
    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s: runtime allocation failed\n", operation);
        goto cleanup;
    }
    ctx = JS_NewContext(runtime);
    if (!ctx) {
        fprintf(stderr, "%s: context allocation failed\n", operation);
        goto cleanup;
    }
    root = JS_ReadObject(ctx, bytes, size, JS_READ_OBJ_BYTECODE);
    if (JS_IsException(root)) {
        report_exception(ctx, operation);
        root = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_GetOwnPropertyNames(ctx, &private_names, &private_count, root,
                               JS_GPN_PRIVATE_MASK | JS_GPN_ENUM_ONLY) < 0) {
        report_exception(ctx, operation);
        goto cleanup;
    }
    checker = JS_Eval(ctx, checker_source, strlen(checker_source),
                      "non-string-property-check.js", JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(checker)) {
        report_exception(ctx, operation);
        checker = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_Call(ctx, checker, JS_UNDEFINED, 1, &root);
    if (JS_IsException(result)) {
        report_exception(ctx, operation);
        result = JS_UNDEFINED;
        goto cleanup;
    }
    keys = JS_GetPropertyUint32(ctx, result, 0);
    symbols = JS_GetPropertyUint32(ctx, result, 1);
    keep = JS_GetPropertyUint32(ctx, result, 2);
    if (JS_IsException(keys) || JS_IsException(symbols) ||
        JS_IsException(keep)) {
        report_exception(ctx, operation);
        goto cleanup;
    }
    keys_text = JS_ToCString(ctx, keys);
    if (!keys_text || strlen(keys_text) >= sizeof(shape->keys) ||
        JS_ToInt32(ctx, &shape->symbol_count, symbols) < 0 ||
        JS_ToInt32(ctx, &shape->keep, keep) < 0) {
        report_exception(ctx, operation);
        goto cleanup;
    }
    strcpy(shape->keys, keys_text);
    shape->private_count = private_count;
    if (strcmp(shape->keys, "keep") != 0 || shape->symbol_count != 0 ||
        shape->private_count != 0 || shape->keep != 42) {
        fprintf(stderr,
                "%s: keys=%s symbols=%" PRId32 " private=%u keep=%" PRId32
                "\n",
                operation, shape->keys, shape->symbol_count,
                shape->private_count, shape->keep);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (ctx) {
        if (keys_text)
            JS_FreeCString(ctx, keys_text);
        JS_FreeValue(ctx, keep);
        JS_FreeValue(ctx, symbols);
        JS_FreeValue(ctx, keys);
        JS_FreeValue(ctx, result);
        JS_FreeValue(ctx, checker);
        JS_FreeValue(ctx, root);
        JS_FreePropertyEnum(ctx, private_names, private_count);
        JS_FreeContext(ctx);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

int main(void) {
    static const char source[] =
        "(() => {\n"
        "  const cycle = {};\n"
        "  cycle.self = cycle;\n"
        "  const symbol = Symbol('hidden');\n"
        "  class Root {\n"
        "    #hidden = cycle;\n"
        "    constructor() {\n"
        "      this.keep = 42;\n"
        "      Object.defineProperty(this, symbol, {\n"
        "        value: cycle, enumerable: true, configurable: true, writable: true\n"
        "      });\n"
        "    }\n"
        "  }\n"
        "  return new Root();\n"
        "})()";
    static const uint8_t expected[] = {
        0x05, 0x01, 0x08, 0x6b, 0x65, 0x65, 0x70,
        0x08, 0x01, 0xe6, 0x03, 0x05, 0x54,
    };
    JSRuntime *runtime = NULL;
    JSContext *ctx = NULL;
    JSValue root = JS_UNDEFINED;
    uint8_t *bytecode = NULL;
    uint8_t *bswap = NULL;
    size_t bytecode_size = 0;
    size_t bswap_size = 0;
    FreshShape bytecode_shape;
    FreshShape bswap_shape;
    int status = 1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fputs("source runtime allocation failed\n", stderr);
        goto cleanup;
    }
    ctx = JS_NewContext(runtime);
    if (!ctx) {
        fputs("source context allocation failed\n", stderr);
        goto cleanup;
    }
    root = JS_Eval(ctx, source, strlen(source), "non-string-properties.js",
                   JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(root)) {
        report_exception(ctx, "source evaluation failed");
        root = JS_UNDEFINED;
        goto cleanup;
    }
    if (validate_source_shape(ctx, root) < 0)
        goto cleanup;

    bytecode = JS_WriteObject(ctx, &bytecode_size, root,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(ctx, "BYTECODE serialization failed");
        goto cleanup;
    }
    bswap = JS_WriteObject(ctx, &bswap_size, root,
                           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    if (!bswap) {
        report_exception(ctx, "BYTECODE|BSWAP serialization failed");
        goto cleanup;
    }
    if (bytecode_size != sizeof(expected) ||
        memcmp(bytecode, expected, sizeof(expected)) != 0 ||
        bswap_size != bytecode_size ||
        memcmp(bytecode, bswap, bytecode_size) != 0) {
        fputs("serialized bytes lost the filtered-property vector\n", stderr);
        goto cleanup;
    }
    if (validate_fresh(bytecode, bytecode_size, "BYTECODE fresh read",
                       &bytecode_shape) < 0 ||
        validate_fresh(bswap, bswap_size, "BYTECODE|BSWAP fresh read",
                       &bswap_shape) < 0)
        goto cleanup;

    puts("quickjs=2026-06-04");
    puts("source-enumerable-properties=string:1,symbol:1,private:1");
    puts("source-non-string-values=shared-circular-object");
    printf("write-flags=%d,%d\n", JS_WRITE_OBJ_BYTECODE,
           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    puts("reference-flag-enabled=false");
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    print_hex(bytecode, bytecode_size);
    putchar('\n');
    puts("bswap-identical=true");
    printf("fresh-bytecode=keys:%s,symbols:%" PRId32 ",private:%u,keep:%" PRId32
           "\n",
           bytecode_shape.keys, bytecode_shape.symbol_count,
           bytecode_shape.private_count, bytecode_shape.keep);
    printf("fresh-bswap=keys:%s,symbols:%" PRId32 ",private:%u,keep:%" PRId32
           "\n",
           bswap_shape.keys, bswap_shape.symbol_count,
           bswap_shape.private_count, bswap_shape.keep);
    status = 0;

cleanup:
    if (ctx) {
        if (bswap)
            js_free(ctx, bswap);
        if (bytecode)
            js_free(ctx, bytecode);
        JS_FreeValue(ctx, root);
        JS_FreeContext(ctx);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}
