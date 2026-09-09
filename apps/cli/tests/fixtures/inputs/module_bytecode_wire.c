/*
 * QuickJS 2026-06-04 oracle for the BC5 Module record.
 *
 * This fixture uses only the public QuickJS C API. It pins a stripped,
 * self-contained module through write, fresh-runtime read, byte-exact
 * reserialization, resolve, and evaluation. A second module exercises the
 * complete Module metadata topology but is deliberately never executed.
 */

#include "quickjs.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_NULL = 1,
    BC_TAG_UNDEFINED = 2,
    BC_TAG_BOOL_FALSE = 3,
    BC_TAG_BOOL_TRUE = 4,
    BC_TAG_INT32 = 5,
    BC_TAG_FLOAT64 = 6,
    BC_TAG_STRING = 7,
    BC_TAG_OBJECT = 8,
    BC_TAG_ARRAY = 9,
    BC_TAG_MODULE = 13,
    BC_TAG_FUNCTION_BYTECODE = 12,
    MAX_MODULE_ENTRIES = 16,
};

typedef struct BytecodeCursor {
    const uint8_t *base;
    const uint8_t *cursor;
    const uint8_t *end;
} BytecodeCursor;

typedef struct RequestShape {
    uint32_t module_name_atom;
    uint8_t attributes_tag;
    size_t attributes_offset;
    size_t attributes_size;
} RequestShape;

typedef struct ExportShape {
    uint8_t export_type;
    uint32_t index;
    uint32_t local_name_atom;
    uint32_t export_name_atom;
    int has_local_name;
} ExportShape;

typedef struct ImportShape {
    uint32_t variable_index;
    uint8_t is_star;
    uint32_t import_name_atom;
    uint32_t requested_module_index;
} ImportShape;

typedef struct ModuleShape {
    uint32_t atom_count;
    size_t module_offset;
    uint32_t module_name_atom;
    uint32_t request_count;
    RequestShape requests[MAX_MODULE_ENTRIES];
    uint32_t export_count;
    ExportShape exports[MAX_MODULE_ENTRIES];
    uint32_t star_export_count;
    uint32_t star_exports[MAX_MODULE_ENTRIES];
    uint32_t import_count;
    ImportShape imports[MAX_MODULE_ENTRIES];
    uint8_t has_tla;
    size_t function_offset;
    uint8_t function_tag;
} ModuleShape;

typedef struct FreshReceipt {
    int reserialized_identically;
    int resolve_status;
    int eval_result_is_object;
    double global_receipt;
} FreshReceipt;

typedef struct LoaderState {
    unsigned int checks;
    unsigned int loads;
    unsigned int attributed_loads;
    unsigned int dep_loads;
    unsigned int namespace_loads;
    unsigned int star_loads;
} LoaderState;

static void report_exception(JSContext *ctx, const char *operation)
{
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

static void print_hex(const uint8_t *bytes, size_t size)
{
    size_t index;

    for (index = 0; index < size; index++)
        printf("%02x", bytes[index]);
}

static int cursor_take(BytecodeCursor *input, size_t count)
{
    if ((size_t)(input->end - input->cursor) < count)
        return -1;
    input->cursor += count;
    return 0;
}

static int cursor_u8(BytecodeCursor *input, uint8_t *value)
{
    if (input->cursor == input->end)
        return -1;
    *value = *input->cursor++;
    return 0;
}

static int cursor_leb128(BytecodeCursor *input, uint32_t *value)
{
    uint32_t result = 0;
    unsigned int shift = 0;

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

static int cursor_string(BytecodeCursor *input)
{
    uint32_t header;
    size_t byte_size;

    if (cursor_leb128(input, &header) < 0)
        return -1;
    byte_size = (size_t)(header >> 1);
    if ((header & 1) != 0) {
        if (byte_size > SIZE_MAX / 2)
            return -1;
        byte_size *= 2;
    }
    return cursor_take(input, byte_size);
}

static int skip_attribute_value(BytecodeCursor *input, unsigned int depth)
{
    uint8_t tag;
    uint32_t count;
    uint32_t index;
    uint32_t unused;

    if (depth > 16 || cursor_u8(input, &tag) < 0)
        return -1;
    switch (tag) {
    case BC_TAG_NULL:
    case BC_TAG_UNDEFINED:
    case BC_TAG_BOOL_FALSE:
    case BC_TAG_BOOL_TRUE:
        return 0;
    case BC_TAG_INT32:
        return cursor_leb128(input, &unused);
    case BC_TAG_FLOAT64:
        return cursor_take(input, 8);
    case BC_TAG_STRING:
        return cursor_string(input);
    case BC_TAG_OBJECT:
        if (cursor_leb128(input, &count) < 0)
            return -1;
        for (index = 0; index < count; index++) {
            if (cursor_leb128(input, &unused) < 0 ||
                skip_attribute_value(input, depth + 1) < 0)
                return -1;
        }
        return 0;
    case BC_TAG_ARRAY:
        if (cursor_leb128(input, &count) < 0)
            return -1;
        for (index = 0; index < count; index++) {
            if (skip_attribute_value(input, depth + 1) < 0)
                return -1;
        }
        return 0;
    default:
        return -1;
    }
}

static int parse_module(const uint8_t *bytecode, size_t bytecode_size,
                        ModuleShape *shape)
{
    BytecodeCursor input = {
        .base = bytecode,
        .cursor = bytecode,
        .end = bytecode + bytecode_size,
    };
    uint8_t version;
    uint8_t tag;
    uint32_t index;

    memset(shape, 0, sizeof(*shape));
    if (cursor_u8(&input, &version) < 0 || version != BC_VERSION ||
        cursor_leb128(&input, &shape->atom_count) < 0)
        return -1;
    for (index = 0; index < shape->atom_count; index++) {
        if (cursor_string(&input) < 0)
            return -1;
    }

    shape->module_offset = (size_t)(input.cursor - input.base);
    if (cursor_u8(&input, &tag) < 0 || tag != BC_TAG_MODULE ||
        cursor_leb128(&input, &shape->module_name_atom) < 0 ||
        cursor_leb128(&input, &shape->request_count) < 0 ||
        shape->request_count > MAX_MODULE_ENTRIES)
        return -1;
    for (index = 0; index < shape->request_count; index++) {
        RequestShape *request = &shape->requests[index];
        const uint8_t *attribute_start;

        if (cursor_leb128(&input, &request->module_name_atom) < 0 ||
            input.cursor == input.end)
            return -1;
        request->attributes_offset = (size_t)(input.cursor - input.base);
        request->attributes_tag = *input.cursor;
        attribute_start = input.cursor;
        if (skip_attribute_value(&input, 0) < 0)
            return -1;
        request->attributes_size = (size_t)(input.cursor - attribute_start);
    }

    if (cursor_leb128(&input, &shape->export_count) < 0 ||
        shape->export_count > MAX_MODULE_ENTRIES)
        return -1;
    for (index = 0; index < shape->export_count; index++) {
        ExportShape *entry = &shape->exports[index];

        if (cursor_u8(&input, &entry->export_type) < 0 ||
            cursor_leb128(&input, &entry->index) < 0)
            return -1;
        if (entry->export_type != 0) {
            entry->has_local_name = 1;
            if (cursor_leb128(&input, &entry->local_name_atom) < 0)
                return -1;
        }
        if (cursor_leb128(&input, &entry->export_name_atom) < 0)
            return -1;
    }

    if (cursor_leb128(&input, &shape->star_export_count) < 0 ||
        shape->star_export_count > MAX_MODULE_ENTRIES)
        return -1;
    for (index = 0; index < shape->star_export_count; index++) {
        if (cursor_leb128(&input, &shape->star_exports[index]) < 0)
            return -1;
    }

    if (cursor_leb128(&input, &shape->import_count) < 0 ||
        shape->import_count > MAX_MODULE_ENTRIES)
        return -1;
    for (index = 0; index < shape->import_count; index++) {
        ImportShape *entry = &shape->imports[index];

        if (cursor_leb128(&input, &entry->variable_index) < 0 ||
            cursor_u8(&input, &entry->is_star) < 0 ||
            cursor_leb128(&input, &entry->import_name_atom) < 0 ||
            cursor_leb128(&input, &entry->requested_module_index) < 0)
            return -1;
    }

    if (cursor_u8(&input, &shape->has_tla) < 0)
        return -1;
    shape->function_offset = (size_t)(input.cursor - input.base);
    if (cursor_u8(&input, &shape->function_tag) < 0 ||
        shape->function_tag != BC_TAG_FUNCTION_BYTECODE)
        return -1;
    return 0;
}

static int validate_self_shape(const ModuleShape *shape)
{
    return shape->request_count == 0 && shape->export_count == 1 &&
           shape->exports[0].export_type == 0 &&
           shape->star_export_count == 0 && shape->import_count == 0 &&
           shape->has_tla == 0 &&
           shape->function_tag == BC_TAG_FUNCTION_BYTECODE
               ? 0
               : -1;
}

static int validate_rich_shape(const ModuleShape *shape)
{
    uint32_t index;
    unsigned int attribute_objects = 0;
    unsigned int local_exports = 0;
    unsigned int indirect_exports = 0;
    unsigned int star_imports = 0;

    for (index = 0; index < shape->request_count; index++) {
        if (shape->requests[index].attributes_tag == BC_TAG_OBJECT)
            attribute_objects++;
    }
    for (index = 0; index < shape->export_count; index++) {
        if (shape->exports[index].export_type == 0)
            local_exports++;
        else
            indirect_exports++;
    }
    for (index = 0; index < shape->import_count; index++) {
        if (shape->imports[index].is_star != 0)
            star_imports++;
    }

    return shape->request_count == 5 && shape->export_count == 3 &&
           shape->star_export_count == 1 && shape->import_count == 3 &&
           shape->has_tla == 1 && attribute_objects == 1 &&
           local_exports == 1 && indirect_exports == 2 &&
           star_imports == 1 &&
           shape->function_tag == BC_TAG_FUNCTION_BYTECODE
               ? 0
               : -1;
}

static int fresh_roundtrip(const uint8_t *bytecode, size_t bytecode_size,
                           int evaluate, FreshReceipt *receipt)
{
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue global = JS_UNDEFINED;
    JSValue receipt_value = JS_UNDEFINED;
    uint8_t *reserialized = NULL;
    size_t reserialized_size = 0;
    int status = -1;

    memset(receipt, 0, sizeof(*receipt));
    receipt->resolve_status = -1;
    runtime = JS_NewRuntime();
    if (!runtime) {
        fputs("fresh runtime allocation failed\n", stderr);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fputs("fresh context allocation failed\n", stderr);
        goto cleanup;
    }

    loaded = JS_ReadObject(context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "fresh module read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    reserialized = JS_WriteObject(context, &reserialized_size, loaded,
                                  JS_WRITE_OBJ_BYTECODE);
    if (!reserialized) {
        report_exception(context, "fresh module reserialization failed");
        goto cleanup;
    }
    receipt->reserialized_identically =
        reserialized_size == bytecode_size &&
        memcmp(reserialized, bytecode, bytecode_size) == 0;
    if (!receipt->reserialized_identically) {
        fputs("fresh module reserialization changed bytes\n", stderr);
        goto cleanup;
    }

    if (!evaluate) {
        status = 0;
        goto cleanup;
    }

    receipt->resolve_status = JS_ResolveModule(context, loaded);
    if (receipt->resolve_status < 0) {
        report_exception(context, "fresh module resolve failed");
        goto cleanup;
    }
    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes the module value. */
    if (JS_IsException(result)) {
        report_exception(context, "fresh module evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    receipt->eval_result_is_object = JS_IsObject(result);
    if (!receipt->eval_result_is_object ||
        JS_PromiseState(context, result) != JS_PROMISE_FULFILLED) {
        fputs("fresh module evaluation did not return a fulfilled promise\n",
              stderr);
        goto cleanup;
    }

    global = JS_GetGlobalObject(context);
    if (JS_IsException(global)) {
        report_exception(context, "fresh global lookup failed");
        global = JS_UNDEFINED;
        goto cleanup;
    }
    receipt_value =
        JS_GetPropertyStr(context, global, "__moduleBytecodeReceipt");
    if (JS_IsException(receipt_value)) {
        report_exception(context, "fresh receipt lookup failed");
        receipt_value = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsNumber(receipt_value) ||
        JS_ToFloat64(context, &receipt->global_receipt, receipt_value) < 0 ||
        receipt->global_receipt != 42.0) {
        fputs("fresh module global receipt was not 42\n", stderr);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (context) {
        if (reserialized)
            js_free(context, reserialized);
        JS_FreeValue(context, receipt_value);
        JS_FreeValue(context, global);
        JS_FreeValue(context, result);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int module_check_attributes(JSContext *ctx, void *opaque,
                                   JSValueConst attributes)
{
    LoaderState *state = opaque;

    (void)ctx;
    if (!JS_IsObject(attributes))
        return -1;
    state->checks++;
    return 0;
}

static int has_basename(const char *name, const char *basename)
{
    const char *slash = strrchr(name, '/');

    return strcmp(slash ? slash + 1 : name, basename) == 0;
}

static JSModuleDef *compile_dependency(JSContext *ctx, const char *name,
                                       const char *source)
{
    JSValue compiled =
        JS_Eval(ctx, source, strlen(source), name,
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    JSModuleDef *module;

    if (JS_IsException(compiled))
        return NULL;
    module = JS_VALUE_GET_PTR(compiled);
    JS_FreeValue(ctx, compiled);
    return module;
}

static JSModuleDef *module_loader(JSContext *ctx, const char *name,
                                  void *opaque, JSValueConst attributes)
{
    LoaderState *state = opaque;

    state->loads++;
    if (!JS_IsUndefined(attributes))
        state->attributed_loads++;
    if (has_basename(name, "dep.js")) {
        state->dep_loads++;
        return compile_dependency(
            ctx, name, "export default 10; export const named = 20;");
    }
    if (has_basename(name, "namespace.js")) {
        state->namespace_loads++;
        return compile_dependency(ctx, name, "export const ns = 30;");
    }
    if (has_basename(name, "star.js")) {
        state->star_loads++;
        return compile_dependency(ctx, name, "export const star = 40;");
    }
    JS_ThrowReferenceError(ctx, "unexpected metadata module '%s'", name);
    return NULL;
}

static void print_shape(const ModuleShape *shape)
{
    uint32_t index;

    printf("atom-count=%u\n", shape->atom_count);
    printf("module-offset=%zu\n", shape->module_offset);
    printf("module-name-atom=%u\n", shape->module_name_atom);
    printf("request-count=%u\n", shape->request_count);
    for (index = 0; index < shape->request_count; index++) {
        const RequestShape *entry = &shape->requests[index];

        printf("request-%u=name-atom:%u,attributes-offset:%zu,"
               "attributes-size:%zu,attributes-tag:%u\n",
               index, entry->module_name_atom, entry->attributes_offset,
               entry->attributes_size, entry->attributes_tag);
    }
    printf("export-count=%u\n", shape->export_count);
    for (index = 0; index < shape->export_count; index++) {
        const ExportShape *entry = &shape->exports[index];

        printf("export-%u=type:%u,index:%u,local-name:%s", index,
               entry->export_type, entry->index,
               entry->has_local_name ? "present:" : "absent");
        if (entry->has_local_name)
            printf("%u", entry->local_name_atom);
        printf(",export-name:%u\n", entry->export_name_atom);
    }
    printf("star-export-count=%u\n", shape->star_export_count);
    for (index = 0; index < shape->star_export_count; index++)
        printf("star-export-%u=request-index:%u\n", index,
               shape->star_exports[index]);
    printf("import-count=%u\n", shape->import_count);
    for (index = 0; index < shape->import_count; index++) {
        const ImportShape *entry = &shape->imports[index];

        printf("import-%u=variable-index:%u,is-star:%u,import-name:%u,"
               "request-index:%u\n",
               index, entry->variable_index, entry->is_star,
               entry->import_name_atom, entry->requested_module_index);
    }
    printf("has-tla=%u\n", shape->has_tla);
    printf("function-offset=%zu\n", shape->function_offset);
    printf("function-tag=%u\n", shape->function_tag);
}

static int run_self_contained(void)
{
    static const char source[] =
        "export const answer = 42; "
        "globalThis.__moduleBytecodeReceipt = answer;";
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue compiled = JS_UNDEFINED;
    uint8_t *bytecode = NULL;
    uint8_t *bswap = NULL;
    size_t bytecode_size = 0;
    size_t bswap_size = 0;
    ModuleShape shape;
    FreshReceipt receipt;
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fputs("self-contained runtime allocation failed\n", stderr);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fputs("self-contained context allocation failed\n", stderr);
        goto cleanup;
    }
    JS_SetStripInfo(runtime, JS_STRIP_DEBUG);
    if (JS_GetStripInfo(runtime) != JS_STRIP_DEBUG) {
        fputs("self-contained strip flags were not retained\n", stderr);
        goto cleanup;
    }
    compiled =
        JS_Eval(context, source, sizeof(source) - 1, "self-contained.mjs",
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(context, "self-contained compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }

    bytecode = JS_WriteObject(context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(context, "self-contained serialization failed");
        goto cleanup;
    }
    bswap = JS_WriteObject(context, &bswap_size, compiled,
                           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    if (!bswap) {
        report_exception(context, "self-contained BSWAP serialization failed");
        goto cleanup;
    }
    if (bytecode_size != bswap_size ||
        memcmp(bytecode, bswap, bytecode_size) != 0) {
        fputs("self-contained BSWAP changed bytes\n", stderr);
        goto cleanup;
    }
    if (parse_module(bytecode, bytecode_size, &shape) < 0 ||
        validate_self_shape(&shape) < 0) {
        fputs("self-contained BC5 Module shape was invalid\n", stderr);
        goto cleanup;
    }
    if (fresh_roundtrip(bytecode, bytecode_size, 1, &receipt) < 0)
        goto cleanup;

    puts("case=self-contained");
    fputs("source-hex=", stdout);
    print_hex((const uint8_t *)source, sizeof(source) - 1);
    putchar('\n');
    printf("strip-flags=%d\n", JS_STRIP_DEBUG);
    printf("write-flags=%d,%d\n", JS_WRITE_OBJ_BYTECODE,
           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    print_hex(bytecode, bytecode_size);
    putchar('\n');
    puts("bswap-identical=true");
    print_shape(&shape);
    printf("fresh-reserialize-identical=%s\n",
           receipt.reserialized_identically ? "true" : "false");
    printf("fresh-resolve=%d\n", receipt.resolve_status);
    printf("fresh-eval-result=%s\n",
           receipt.eval_result_is_object ? "object" : "non-object");
    printf("fresh-global-receipt=%.17g\n", receipt.global_receipt);
    status = 0;

cleanup:
    if (context) {
        if (bswap)
            js_free(context, bswap);
        if (bytecode)
            js_free(context, bytecode);
        JS_FreeValue(context, compiled);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int run_metadata_rich(void)
{
    static const char source[] =
        "import defaultValue, { named as importedName } from './dep.js' "
        "with { type: 'oracle', mode: 'rich' };\n"
        "import * as namespaceValue from './namespace.js';\n"
        "export const localValue = 1;\n"
        "export { named as indirectValue } from './dep.js';\n"
        "export * from './star.js';\n"
        "export * as namespaceExport from './namespace.js';\n"
        "void defaultValue; void importedName; void namespaceValue;\n"
        "await 0;";
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue compiled = JS_UNDEFINED;
    uint8_t *bytecode = NULL;
    uint8_t *bswap = NULL;
    size_t bytecode_size = 0;
    size_t bswap_size = 0;
    ModuleShape shape;
    FreshReceipt receipt;
    LoaderState loader = {0};
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fputs("metadata-rich runtime allocation failed\n", stderr);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fputs("metadata-rich context allocation failed\n", stderr);
        goto cleanup;
    }
    JS_SetStripInfo(runtime, JS_STRIP_DEBUG);
    JS_SetModuleLoaderFunc2(runtime, NULL, module_loader,
                            module_check_attributes, &loader);
    compiled =
        JS_Eval(context, source, sizeof(source) - 1, "metadata-rich.mjs",
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(context, "metadata-rich compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    if (loader.loads != 3 || loader.dep_loads != 1 ||
        loader.namespace_loads != 1 || loader.star_loads != 1 ||
        loader.attributed_loads != 1 || loader.checks != 1) {
        fprintf(stderr,
                "metadata-rich loader topology drifted: checks=%u loads=%u "
                "attributed=%u dep=%u namespace=%u star=%u\n",
                loader.checks, loader.loads, loader.attributed_loads,
                loader.dep_loads, loader.namespace_loads, loader.star_loads);
        goto cleanup;
    }

    bytecode = JS_WriteObject(context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(context, "metadata-rich serialization failed");
        goto cleanup;
    }
    bswap = JS_WriteObject(context, &bswap_size, compiled,
                           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    if (!bswap) {
        report_exception(context, "metadata-rich BSWAP serialization failed");
        goto cleanup;
    }
    if (bytecode_size != bswap_size ||
        memcmp(bytecode, bswap, bytecode_size) != 0) {
        fputs("metadata-rich BSWAP changed bytes\n", stderr);
        goto cleanup;
    }
    if (parse_module(bytecode, bytecode_size, &shape) < 0 ||
        validate_rich_shape(&shape) < 0) {
        fputs("metadata-rich BC5 Module shape was invalid\n", stderr);
        goto cleanup;
    }
    if (fresh_roundtrip(bytecode, bytecode_size, 0, &receipt) < 0)
        goto cleanup;

    puts("case=metadata-rich");
    fputs("source-hex=", stdout);
    print_hex((const uint8_t *)source, sizeof(source) - 1);
    putchar('\n');
    printf("strip-flags=%d\n", JS_STRIP_DEBUG);
    printf("write-flags=%d,%d\n", JS_WRITE_OBJ_BYTECODE,
           JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_BSWAP);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    print_hex(bytecode, bytecode_size);
    putchar('\n');
    puts("bswap-identical=true");
    printf("loader-checks=%u\n", loader.checks);
    printf("loader-loads=%u\n", loader.loads);
    printf("loader-attributed-loads=%u\n", loader.attributed_loads);
    print_shape(&shape);
    printf("fresh-reserialize-identical=%s\n",
           receipt.reserialized_identically ? "true" : "false");
    puts("metadata-executed=false");
    status = 0;

cleanup:
    if (context) {
        if (bswap)
            js_free(context, bswap);
        if (bytecode)
            js_free(context, bytecode);
        JS_FreeValue(context, compiled);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

int main(void)
{
    puts("quickjs=2026-06-04");
    if (run_self_contained() < 0 || run_metadata_rich() < 0)
        return 1;
    return 0;
}
