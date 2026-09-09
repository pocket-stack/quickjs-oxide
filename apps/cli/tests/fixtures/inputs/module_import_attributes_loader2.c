/*
 * QuickJS 2026-06-04 oracle for static import-attribute host ordering.
 *
 * This is test-only C and links only against the pinned external oracle. The
 * product engine remains Rust-only.
 */

#include "quickjs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int print_attributes(JSContext *ctx, const char *site,
                            JSValueConst attributes)
{
    JSPropertyEnum *properties = NULL;
    uint32_t property_count = 0;
    JSValue prototype;
    uint32_t index;

    printf("%s", site);
    if (JS_IsUndefined(attributes)) {
        puts(" attrs=undefined");
        return 0;
    }

    prototype = JS_GetPrototype(ctx, attributes);
    if (JS_IsException(prototype))
        return -1;
    printf(" attrs=object proto=%s", JS_IsNull(prototype) ? "null" : "other");
    JS_FreeValue(ctx, prototype);

    if (JS_GetOwnPropertyNames(ctx, &properties, &property_count, attributes,
                               JS_GPN_STRING_MASK | JS_GPN_SET_ENUM) < 0)
        return -1;
    printf(" count=%u", property_count);
    for (index = 0; index < property_count; index++) {
        JSPropertyDescriptor descriptor;
        const char *key;
        const char *value;
        int present;

        key = JS_AtomToCString(ctx, properties[index].atom);
        if (!key) {
            JS_FreePropertyEnum(ctx, properties, property_count);
            return -1;
        }
        present = JS_GetOwnProperty(ctx, &descriptor, attributes,
                                    properties[index].atom);
        if (present <= 0) {
            JS_FreeCString(ctx, key);
            JS_FreePropertyEnum(ctx, properties, property_count);
            return -1;
        }
        value = JS_ToCString(ctx, descriptor.value);
        if (!value) {
            JS_FreeValue(ctx, descriptor.value);
            JS_FreeValue(ctx, descriptor.getter);
            JS_FreeValue(ctx, descriptor.setter);
            JS_FreeCString(ctx, key);
            JS_FreePropertyEnum(ctx, properties, property_count);
            return -1;
        }
        printf(" %s=%s:%s", key, value,
               (descriptor.flags & JS_PROP_C_W_E) == JS_PROP_C_W_E
                   ? "cwe"
                   : "other");
        JS_FreeCString(ctx, value);
        JS_FreeValue(ctx, descriptor.value);
        JS_FreeValue(ctx, descriptor.getter);
        JS_FreeValue(ctx, descriptor.setter);
        JS_FreeCString(ctx, key);
    }
    JS_FreePropertyEnum(ctx, properties, property_count);
    putchar('\n');
    return 0;
}

static int check_attributes(JSContext *ctx, void *opaque,
                            JSValueConst attributes)
{
    unsigned int *count = opaque;
    char site[48];

    snprintf(site, sizeof(site), "check[%u]", (*count)++);
    return print_attributes(ctx, site, attributes);
}

static const char *source_for_module(const char *module_name)
{
    if (!strcmp(module_name, "cached.js"))
        return "export const cached = 1;";
    if (!strcmp(module_name, "empty.js"))
        return "export const empty = 1;";
    if (!strcmp(module_name, "absent.js"))
        return "export const absent = 1;";
    return NULL;
}

static JSModuleDef *load_module(JSContext *ctx, const char *module_name,
                                void *opaque, JSValueConst attributes)
{
    static unsigned int load_count;
    const char *source;
    JSValue function;
    JSModuleDef *module;
    char site[96];

    (void)opaque;
    snprintf(site, sizeof(site), "load[%u] name=%s", load_count++, module_name);
    if (print_attributes(ctx, site, attributes) < 0)
        return NULL;

    source = source_for_module(module_name);
    if (!source) {
        JS_ThrowReferenceError(ctx, "missing oracle module '%s'", module_name);
        return NULL;
    }
    function = JS_Eval(ctx, source, strlen(source), module_name,
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(function))
        return NULL;
    module = JS_VALUE_GET_PTR(function);
    JS_FreeValue(ctx, function);
    return module;
}

static void print_exception(JSContext *ctx, const char *site)
{
    JSValue exception = JS_GetException(ctx);
    JSValue name = JS_GetPropertyStr(ctx, exception, "name");
    const char *name_string = JS_ToCString(ctx, name);

    printf("%s exception=%s\n", site, name_string ? name_string : "<non-string>");
    if (name_string)
        JS_FreeCString(ctx, name_string);
    JS_FreeValue(ctx, name);
    JS_FreeValue(ctx, exception);
}

int main(void)
{
    static const char bad_source[] =
        "import './cached.js' with { before: 'syntax' }; export {";
    static const char entry_source[] =
        "import './cached.js' with { zebra: 'z', alpha: 'a' };\n"
        "import './cached.js' with { second: '2' };\n"
        "import './empty.js' with {};\n"
        "import './absent.js';\n";
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;
    JSValue function;
    JSValue result;
    unsigned int check_count = 0;

    if (!runtime)
        return 2;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return 2;
    }
    JS_SetModuleLoaderFunc2(runtime, NULL, load_module, check_attributes,
                            &check_count);

    function = JS_Eval(context, bad_source, strlen(bad_source), "bad.js",
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (!JS_IsException(function)) {
        JS_FreeValue(context, function);
        fputs("bad source unexpectedly compiled\n", stderr);
        return 1;
    }
    print_exception(context, "bad");

    function = JS_Eval(context, entry_source, strlen(entry_source), "entry.js",
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(function)) {
        print_exception(context, "entry-compile");
        return 1;
    }
    result = JS_EvalFunction(context, function);
    if (JS_IsException(result)) {
        print_exception(context, "entry-evaluate");
        return 1;
    }
    JS_FreeValue(context, result);
    puts("entry result=ok");

    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return 0;
}
