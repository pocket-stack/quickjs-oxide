/*
 * QuickJS 2026-06-04 oracle for the parse-time failed-resolution latch.
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
    unsigned int before_load_count;
    unsigned int after_load_count;
    int checker_active;
    int swallowed_failure;
    int context_mismatch;
} ProbeState;

static const char *boolean_name(int value)
{
    return value ? "true" : "false";
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
    if (!strcmp(module_name, "before.js")) {
        state->before_load_count++;
        JS_ThrowReferenceError(ctx, "intentional parse-time prefix load failure");
        return NULL;
    }
    if (!strcmp(module_name, "after.js"))
        state->after_load_count++;
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
    JSValue exception;
    const char *exception_text;
    int same_context = ctx == state->expected_context;

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
    if (!JS_IsException(state->probe_module)) {
        JS_ThrowInternalError(ctx, "nested prefix failure unexpectedly compiled");
        state->checker_active = 0;
        return -1;
    }
    exception = JS_GetException(ctx);
    exception_text = JS_ToCString(ctx, exception);
    JS_RunGC(state->runtime);
    printf("checker compile=exception exception=%s has-exception-after-get=%s "
           "gc=ok\n",
           exception_text ? exception_text : "<non-string>",
           boolean_name(JS_HasException(ctx)));
    if (exception_text)
        JS_FreeCString(ctx, exception_text);
    JS_FreeValue(ctx, exception);
    state->swallowed_failure = 1;
    state->checker_active = 0;
    return 0;
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
    int outer_resolve;
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
    printf("outer compile=%s swallowed=%s has-exception=%s\n",
           JS_IsException(outer_module) ? "exception" : "module",
           boolean_name(state.swallowed_failure),
           boolean_name(JS_HasException(context)));
    if (JS_IsException(outer_module)) {
        status = 1;
        goto done;
    }

    outer_resolve = JS_ResolveModule(context, outer_module);
    JS_RunGC(runtime);
    printf("post-compile resolve outer=%d gc=ok\n", outer_resolve);
    printf("summary checks=%u normalizes=%u loads=%u before-loads=%u "
           "after-loads=%u\n",
           state.check_count, state.normalize_count, state.load_count,
           state.before_load_count, state.after_load_count);

    if (!state.swallowed_failure || state.context_mismatch ||
        JS_HasException(context) || outer_resolve != 0 || state.check_count != 1 ||
        state.normalize_count != 2 || state.load_count != 1 ||
        state.before_load_count != 1 || state.after_load_count != 0) {
        status = 1;
    }

done:
    JS_FreeValue(context, outer_module);
    JS_FreeValue(context, state.probe_module);
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return status;
}
