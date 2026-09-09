/*
 * QuickJS 2026-06-04 oracle for parse-time request-prefix resolution.
 *
 * This is test-only C and links only against the pinned external oracle. The
 * product engine remains Rust-only.
 */

#include "quickjs.h"

#include <stdio.h>
#include <string.h>

typedef struct ProbeState {
    JSContext *expected_context;
    JSRuntime *runtime;
    JSValue probe_module;
    unsigned int check_count;
    unsigned int normalize_count;
    unsigned int load_count;
    unsigned int outer_load_count;
    unsigned int before_load_count;
    unsigned int after_load_count;
    int checker_active;
    int context_mismatch;
} ProbeState;

static const char *boolean_name(int value)
{
    return value ? "true" : "false";
}

static void print_exception(JSContext *ctx, const char *site)
{
    JSValue exception = JS_GetException(ctx);
    const char *text = JS_ToCString(ctx, exception);

    fprintf(stderr, "%s failed", site);
    if (text)
        fprintf(stderr, ": %s", text);
    fputc('\n', stderr);
    if (text)
        JS_FreeCString(ctx, text);
    JS_FreeValue(ctx, exception);
}

static char *normalize_module(JSContext *ctx, const char *base_name,
                              const char *specifier, void *opaque)
{
    ProbeState *state = opaque;
    const char *normalized = specifier;
    int same_context = ctx == state->expected_context;

    if (!same_context)
        state->context_mismatch = 1;
    if (normalized[0] == '.' && normalized[1] == '/')
        normalized += 2;
    printf("normalize[%u] during-check=%s context=%s base=%s spec=%s "
           "result=%s\n",
           state->normalize_count++, boolean_name(state->checker_active),
           same_context ? "same" : "different", base_name, specifier,
           normalized);
    return js_strdup(ctx, normalized);
}

static JSModuleDef *compile_loaded_module(JSContext *ctx,
                                          const char *module_name,
                                          const char *source)
{
    JSValue function =
        JS_Eval(ctx, source, strlen(source), module_name,
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    JSModuleDef *module;

    if (JS_IsException(function))
        return NULL;
    module = JS_VALUE_GET_PTR(function);
    JS_FreeValue(ctx, function);
    return module;
}

static JSModuleDef *load_module(JSContext *ctx, const char *module_name,
                                void *opaque, JSValueConst attributes)
{
    ProbeState *state = opaque;
    int same_context = ctx == state->expected_context;

    (void)attributes;
    if (!same_context)
        state->context_mismatch = 1;
    printf("load[%u] during-check=%s context=%s name=%s\n",
           state->load_count++, boolean_name(state->checker_active),
           same_context ? "same" : "different", module_name);
    if (!strcmp(module_name, "outer.js")) {
        state->outer_load_count++;
    } else if (!strcmp(module_name, "before.js")) {
        state->before_load_count++;
        return compile_loaded_module(ctx, module_name,
                                     "export const before = 1;");
    } else if (!strcmp(module_name, "after.js")) {
        state->after_load_count++;
    }
    JS_ThrowReferenceError(ctx, "unexpected oracle module '%s'", module_name);
    return NULL;
}

static int check_attributes(JSContext *ctx, void *opaque,
                            JSValueConst attributes)
{
    static const char probe_source[] =
        "import './outer.js'; export const probe = 1;";
    ProbeState *state = opaque;
    unsigned int check_index = state->check_count++;
    int same_context = ctx == state->expected_context;
    int resolve_status;

    if (!same_context)
        state->context_mismatch = 1;
    printf("check[%u] context=%s attrs=%s\n", check_index,
           same_context ? "same" : "different",
           JS_IsObject(attributes) ? "object" : "other");
    if (check_index != 0) {
        JS_ThrowInternalError(ctx, "unexpected recursive attribute checker");
        return -1;
    }

    state->checker_active = 1;
    state->probe_module =
        JS_Eval(ctx, probe_source, sizeof(probe_source) - 1, "probe.js",
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(state->probe_module)) {
        state->checker_active = 0;
        return -1;
    }
    resolve_status = JS_ResolveModule(ctx, state->probe_module);
    JS_RunGC(state->runtime);
    printf("checker probe=module resolve=%d gc=ok\n", resolve_status);
    state->checker_active = 0;
    return resolve_status;
}

int main(void)
{
    static const char outer_source[] =
        "import './before.js' with { type: 'probe' };"
        "import './after.js';"
        "export const answer = 42;";
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;
    ProbeState state = {
        .probe_module = JS_UNDEFINED,
    };
    JSValue outer_module = JS_UNDEFINED;
    int modules_are_distinct;
    int outer_resolve;
    int probe_resolve;
    int status = 0;

    if (!runtime)
        return 2;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return 2;
    }
    state.expected_context = context;
    state.runtime = runtime;
    JS_SetModuleLoaderFunc2(runtime, normalize_module, load_module,
                            check_attributes, &state);

    outer_module =
        JS_Eval(context, outer_source, sizeof(outer_source) - 1, "outer.js",
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    modules_are_distinct =
        !JS_IsException(outer_module) &&
        !JS_IsException(state.probe_module) &&
        JS_VALUE_GET_PTR(outer_module) != JS_VALUE_GET_PTR(state.probe_module);
    printf("outer compile=%s distinct=%s has-exception=%s\n",
           JS_IsException(outer_module) ? "exception" : "module",
           boolean_name(modules_are_distinct),
           boolean_name(JS_HasException(context)));
    if (JS_IsException(outer_module)) {
        print_exception(context, "outer compilation");
        status = 1;
        goto done;
    }

    outer_resolve = JS_ResolveModule(context, outer_module);
    probe_resolve = JS_ResolveModule(context, state.probe_module);
    JS_RunGC(runtime);
    printf("post-compile resolve outer=%d probe=%d gc=ok\n", outer_resolve,
           probe_resolve);
    printf("summary checks=%u normalizes=%u loads=%u outer-loads=%u "
           "before-loads=%u after-loads=%u\n",
           state.check_count, state.normalize_count, state.load_count,
           state.outer_load_count, state.before_load_count,
           state.after_load_count);

    if (!modules_are_distinct || state.context_mismatch ||
        JS_HasException(context) || outer_resolve != 0 ||
        probe_resolve != 0 || state.check_count != 1 ||
        state.normalize_count != 2 || state.load_count != 1 ||
        state.outer_load_count != 0 || state.before_load_count != 1 ||
        state.after_load_count != 0) {
        status = 1;
    }

done:
    JS_FreeValue(context, outer_module);
    JS_FreeValue(context, state.probe_module);
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return status;
}
