/*
 * QuickJS 2026-06-04 oracle for JSON modules produced by loader2.
 *
 * This is test-only C and links only against the pinned external oracle. The
 * product engine remains Rust-only. The implementation intentionally mirrors
 * QuickJS's create_json_module(): parse JSON in the loader, create a C module
 * with one `default` export, retain the parsed value as module-private state,
 * and initialize the export when the C module is evaluated.
 */

#include "quickjs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct OracleState {
    const char *label;
    unsigned int load_count;
    unsigned int init_count;
    unsigned int object_load_count;
} OracleState;

static OracleState *active_state;

static int has_suffix(const char *string, const char *suffix)
{
    size_t string_length = strlen(string);
    size_t suffix_length = strlen(suffix);

    return string_length >= suffix_length &&
           !memcmp(string + string_length - suffix_length, suffix,
                   suffix_length);
}

static const char *json_source_for_module(const char *module_name)
{
    if (has_suffix(module_name, "/primitive.json"))
        return "42\n";
    if (has_suffix(module_name, "/object.json"))
        return "{\"kind\":\"object\",\"count\":2}\n";
    if (has_suffix(module_name, "/array.json"))
        return "[1,2,3]\n";
    if (has_suffix(module_name, "/invalid.json")) {
        return "{\n"
               "  \"ok\": true,\n"
               "  \"broken\": ]\n"
               "}\n";
    }
    return NULL;
}

static int check_attributes(JSContext *ctx, void *opaque,
                            JSValueConst attributes)
{
    JSPropertyEnum *properties = NULL;
    uint32_t property_count = 0;
    uint32_t index;

    (void)opaque;
    if (JS_GetOwnPropertyNames(ctx, &properties, &property_count, attributes,
                               JS_GPN_ENUM_ONLY | JS_GPN_STRING_MASK) < 0)
        return -1;
    for (index = 0; index < property_count; index++) {
        const char *key = JS_AtomToCString(ctx, properties[index].atom);

        if (!key) {
            JS_FreePropertyEnum(ctx, properties, property_count);
            return -1;
        }
        if (strcmp(key, "type")) {
            JS_ThrowTypeError(ctx, "import attribute '%s' is not supported",
                              key);
            JS_FreeCString(ctx, key);
            JS_FreePropertyEnum(ctx, properties, property_count);
            return -1;
        }
        JS_FreeCString(ctx, key);
    }
    JS_FreePropertyEnum(ctx, properties, property_count);
    return 0;
}

static int json_module_init(JSContext *ctx, JSModuleDef *module)
{
    JSAtom name_atom;
    const char *module_name;
    JSValue namespace;
    JSValue before;
    JSValue private_value;
    JSValue after;
    unsigned int init_index;
    int identical;

    if (!active_state)
        return -1;
    init_index = active_state->init_count++;
    name_atom = JS_GetModuleName(ctx, module);
    module_name = JS_AtomToCString(ctx, name_atom);
    JS_FreeAtom(ctx, name_atom);
    if (!module_name)
        return -1;

    namespace = JS_GetModuleNamespace(ctx, module);
    if (JS_IsException(namespace)) {
        fprintf(stderr, "%s init namespace lookup failed\n",
                active_state->label);
        JS_FreeCString(ctx, module_name);
        return -1;
    }
    before = JS_GetPropertyStr(ctx, namespace, "default");
    if (JS_IsException(before) || !JS_IsUndefined(before)) {
        JS_FreeValue(ctx, before);
        JS_FreeValue(ctx, namespace);
        JS_FreeCString(ctx, module_name);
        fprintf(stderr, "%s init default was not undefined\n",
                active_state->label);
        JS_ThrowInternalError(ctx,
                              "JSON default export initialized before init");
        return -1;
    }
    JS_FreeValue(ctx, before);

    private_value = JS_GetModulePrivateValue(ctx, module);
    if (JS_IsException(private_value) ||
        JS_SetModuleExport(ctx, module, "default",
                           JS_DupValue(ctx, private_value)) < 0) {
        JS_FreeValue(ctx, private_value);
        JS_FreeValue(ctx, namespace);
        JS_FreeCString(ctx, module_name);
        fprintf(stderr, "%s init could not set default export\n",
                active_state->label);
        return -1;
    }
    after = JS_GetPropertyStr(ctx, namespace, "default");
    if (JS_IsException(after)) {
        JS_FreeValue(ctx, private_value);
        JS_FreeValue(ctx, namespace);
        JS_FreeCString(ctx, module_name);
        fprintf(stderr, "%s init could not read set default export\n",
                active_state->label);
        return -1;
    }
    identical = JS_StrictEq(ctx, after, private_value);
    printf("%s init[%u] name=%s before=undefined "
           "after-private-identical=%s\n",
           active_state->label, init_index, module_name,
           identical == 1 ? "true" : "false");

    JS_FreeValue(ctx, after);
    JS_FreeValue(ctx, private_value);
    JS_FreeValue(ctx, namespace);
    JS_FreeCString(ctx, module_name);
    return identical == 1 ? 0 : -1;
}

static JSModuleDef *load_module(JSContext *ctx, const char *module_name,
                                void *opaque, JSValueConst attributes)
{
    OracleState *state = opaque;
    const char *source;
    JSValue type_value;
    const char *type;
    JSValue parsed;
    JSModuleDef *module;
    unsigned int load_index;

    if (!state || state != active_state) {
        JS_ThrowInternalError(ctx, "JSON oracle state mismatch");
        return NULL;
    }
    type_value = JS_GetPropertyStr(ctx, attributes, "type");
    if (JS_IsException(type_value))
        return NULL;
    type = JS_ToCString(ctx, type_value);
    if (!type) {
        JS_FreeValue(ctx, type_value);
        return NULL;
    }
    if (strcmp(type, "json")) {
        JS_ThrowTypeError(ctx, "unsupported JSON module type '%s'", type);
        JS_FreeCString(ctx, type);
        JS_FreeValue(ctx, type_value);
        return NULL;
    }

    load_index = state->load_count++;
    if (has_suffix(module_name, "/object.json"))
        state->object_load_count++;
    printf("%s load[%u] name=%s type=%s\n", state->label, load_index,
           module_name, type);
    JS_FreeCString(ctx, type);
    JS_FreeValue(ctx, type_value);

    source = json_source_for_module(module_name);
    if (!source) {
        JS_ThrowReferenceError(ctx, "missing JSON oracle module '%s'",
                               module_name);
        return NULL;
    }
    parsed = JS_ParseJSON(ctx, source, strlen(source), module_name);
    if (JS_IsException(parsed))
        return NULL;
    module = JS_NewCModule(ctx, module_name, json_module_init);
    if (!module) {
        JS_FreeValue(ctx, parsed);
        return NULL;
    }
    if (JS_AddModuleExport(ctx, module, "default") < 0) {
        JS_FreeValue(ctx, parsed);
        return NULL;
    }
    JS_SetModulePrivateValue(ctx, module, parsed);
    printf("%s create name=%s module=c exports=default\n", state->label,
           module_name);
    return module;
}

static int start_scenario(OracleState *state, JSRuntime **runtime_out,
                          JSContext **context_out)
{
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;

    if (!runtime)
        return -1;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return -1;
    }
    active_state = state;
    JS_SetModuleLoaderFunc2(runtime, NULL, load_module, check_attributes,
                            state);
    *runtime_out = runtime;
    *context_out = context;
    return 0;
}

static void finish_scenario(JSRuntime *runtime, JSContext *context)
{
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    active_state = NULL;
}

static int print_exception(JSContext *ctx, const char *site)
{
    static const char *const property_names[] = {
        "name", "message", "fileName", "lineNumber", "columnNumber",
    };
    const char *properties[sizeof(property_names) / sizeof(property_names[0])];
    JSValue exception = JS_GetException(ctx);
    size_t index;
    int result = 0;

    memset(properties, 0, sizeof(properties));
    for (index = 0; index < sizeof(property_names) / sizeof(property_names[0]);
         index++) {
        JSValue value =
            JS_GetPropertyStr(ctx, exception, property_names[index]);

        properties[index] = JS_ToCString(ctx, value);
        JS_FreeValue(ctx, value);
        if (!properties[index])
            result = -1;
    }
    if (!result) {
        printf("%s error=%s message=%s file=%s line=%s column=%s\n", site,
               properties[0], properties[1], properties[2], properties[3],
               properties[4]);
    }
    for (index = 0; index < sizeof(properties) / sizeof(properties[0]);
         index++) {
        if (properties[index])
            JS_FreeCString(ctx, properties[index]);
    }
    JS_FreeValue(ctx, exception);
    return result;
}

static int print_global_result(JSContext *ctx, const char *site)
{
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue value = JS_GetPropertyStr(ctx, global, "__oracle");
    JSValue json;
    const char *json_string;

    JS_FreeValue(ctx, global);
    if (JS_IsException(value))
        return -1;
    json = JS_JSONStringify(ctx, value, JS_UNDEFINED, JS_UNDEFINED);
    JS_FreeValue(ctx, value);
    if (JS_IsException(json))
        return -1;
    json_string = JS_ToCString(ctx, json);
    if (!json_string) {
        JS_FreeValue(ctx, json);
        return -1;
    }
    printf("%s value=%s\n", site, json_string);
    JS_FreeCString(ctx, json_string);
    JS_FreeValue(ctx, json);
    return 0;
}

static int run_main_scenario(void)
{
    static const char source[] =
        "import primitiveValue from './data/primitive.json' with { type: "
        "'json' };\n"
        "import * as primitiveNamespace from './data/primitive.json' with { "
        "type: 'json' };\n"
        "import objectValue from './data/object.json' with { type: 'json' "
        "};\n"
        "import * as objectNamespace from './data/object.json' with { type: "
        "'json' };\n"
        "import * as objectAlias from '../suite/data/object.json' with { "
        "type: 'json' };\n"
        "import arrayValue from './data/array.json' with { type: 'json' };\n"
        "import * as arrayNamespace from './data/array.json' with { type: "
        "'json' };\n"
        "const objectExtensibleBefore = Object.isExtensible(objectValue);\n"
        "objectValue.extra = 7;\n"
        "globalThis.__oracle = {\n"
        "  primitive: primitiveValue,\n"
        "  primitiveType: typeof primitiveValue,\n"
        "  primitiveNamespaceKeys: Object.keys(primitiveNamespace),\n"
        "  primitiveDefaultIdentity: primitiveNamespace.default === "
        "primitiveValue,\n"
        "  object: objectValue,\n"
        "  objectExtensibleBefore,\n"
        "  objectNamespaceKeys: Object.keys(objectNamespace),\n"
        "  objectNamespaceExtensible: Object.isExtensible(objectNamespace),\n"
        "  objectNamespaceNullPrototype: Object.getPrototypeOf(objectNamespace) "
        "=== null,\n"
        "  objectDefaultIdentity: objectNamespace.default === objectValue,\n"
        "  objectAliasIdentity: objectAlias === objectNamespace,\n"
        "  array: arrayValue,\n"
        "  arrayIsArray: Array.isArray(arrayValue),\n"
        "  arrayExtensible: Object.isExtensible(arrayValue),\n"
        "  arrayNamespaceKeys: Object.keys(arrayNamespace),\n"
        "  arrayDefaultIdentity: arrayNamespace.default === arrayValue\n"
        "};\n";
    OracleState state = {"main", 0, 0, 0};
    JSRuntime *runtime;
    JSContext *context;
    JSValue function;
    JSValue result;
    JSPromiseStateEnum promise_state;
    int status = 1;

    if (start_scenario(&state, &runtime, &context) < 0)
        return 1;
    function = JS_Eval(context, source, strlen(source), "suite/entry.js",
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(function)) {
        print_exception(context, "main compile");
        goto done;
    }
    printf("main before-evaluate loads=%u object-loads=%u init-callbacks=%u\n",
           state.load_count, state.object_load_count, state.init_count);
    result = JS_EvalFunction(context, function);
    if (JS_IsException(result)) {
        print_exception(context, "main evaluate");
        goto done;
    }
    promise_state = JS_PromiseState(context, result);
    printf("main after-evaluate state=%s loads=%u object-loads=%u "
           "init-callbacks=%u\n",
           promise_state == JS_PROMISE_FULFILLED ? "fulfilled" : "other",
           state.load_count, state.object_load_count, state.init_count);
    JS_FreeValue(context, result);
    if (promise_state != JS_PROMISE_FULFILLED ||
        print_global_result(context, "main") < 0)
        goto done;
    if (state.load_count != 3 || state.object_load_count != 1 ||
        state.init_count != 3)
        goto done;
    status = 0;
done:
    finish_scenario(runtime, context);
    return status;
}

static int run_named_import_scenario(void)
{
    static const char source[] =
        "import { named } from './data/object.json' with { type: 'json' };\n"
        "globalThis.__oracle = named;\n";
    OracleState state = {"named", 0, 0, 0};
    JSRuntime *runtime;
    JSContext *context;
    JSValue function;
    JSValue result;
    int status = 1;

    if (start_scenario(&state, &runtime, &context) < 0)
        return 1;
    function = JS_Eval(context, source, strlen(source), "named/entry.js",
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(function)) {
        print_exception(context, "named compile");
        goto done;
    }
    printf("named before-evaluate loads=%u init-callbacks=%u\n",
           state.load_count, state.init_count);
    result = JS_EvalFunction(context, function);
    if (!JS_IsException(result)) {
        JS_FreeValue(context, result);
        fputs("named import unexpectedly evaluated\n", stderr);
        goto done;
    }
    if (print_exception(context, "named resolution") < 0)
        goto done;
    printf("named after-error loads=%u init-callbacks=%u\n", state.load_count,
           state.init_count);
    if (state.load_count != 1 || state.init_count != 0)
        goto done;
    status = 0;
done:
    finish_scenario(runtime, context);
    return status;
}

static int run_invalid_json_scenario(void)
{
    static const char source[] =
        "import invalid from './data/invalid.json' with { type: 'json' };\n"
        "globalThis.__oracle = invalid;\n";
    OracleState state = {"invalid", 0, 0, 0};
    JSRuntime *runtime;
    JSContext *context;
    JSValue function;
    int status = 1;

    if (start_scenario(&state, &runtime, &context) < 0)
        return 1;
    function = JS_Eval(context, source, strlen(source), "invalid/entry.js",
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (!JS_IsException(function)) {
        JS_FreeValue(context, function);
        fputs("invalid JSON unexpectedly resolved\n", stderr);
        goto done;
    }
    if (print_exception(context, "invalid resolution") < 0)
        goto done;
    printf("invalid after-error loads=%u init-callbacks=%u\n",
           state.load_count, state.init_count);
    if (state.load_count != 1 || state.init_count != 0)
        goto done;
    status = 0;
done:
    finish_scenario(runtime, context);
    return status;
}

int main(void)
{
    if (run_main_scenario() || run_named_import_scenario() ||
        run_invalid_json_scenario())
        return 1;
    return 0;
}
