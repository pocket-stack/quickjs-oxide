/*
 * QuickJS 2026-06-04 oracle for module callback exceptions and re-entry.
 *
 * This is test-only C and links only against the pinned external oracle. The
 * product engine remains Rust-only.
 */

#include "quickjs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef enum ProbeStage {
    PROBE_NORMALIZE_THROW,
    PROBE_ATTRIBUTE_THROW,
    PROBE_LOAD_THROW,
    PROBE_LOAD_RECOVER,
    PROBE_DYNAMIC_THROW,
    PROBE_REENTRY,
} ProbeStage;

typedef struct ProbeState {
    ProbeStage stage;
    JSValue normalize_reason;
    JSValue attribute_reason;
    JSValue load_reason;
    JSValue dynamic_reason;
    unsigned int normalize_count;
    unsigned int check_count;
    unsigned int load_count;
    unsigned int loader_depth;
    unsigned int maximum_loader_depth;
} ProbeState;

static const char *stage_name(ProbeStage stage)
{
    switch (stage) {
    case PROBE_NORMALIZE_THROW:
        return "normalize-throw";
    case PROBE_ATTRIBUTE_THROW:
        return "attribute-throw";
    case PROBE_LOAD_THROW:
        return "load-throw";
    case PROBE_LOAD_RECOVER:
        return "load-recover";
    case PROBE_DYNAMIC_THROW:
        return "dynamic-throw";
    case PROBE_REENTRY:
        return "reentry";
    }
    return "unknown";
}

static const char *value_kind(JSValueConst value)
{
    if (JS_IsObject(value))
        return "object";
    if (JS_IsSymbol(value))
        return "symbol";
    if (JS_IsNumber(value))
        return "number";
    if (JS_IsString(value))
        return "string";
    if (JS_IsNull(value))
        return "null";
    if (JS_IsUndefined(value))
        return "undefined";
    if (JS_IsBool(value))
        return "boolean";
    return "other";
}

static int print_expected_exception(JSContext *ctx, JSValue result,
                                    const char *site,
                                    JSValueConst expected)
{
    JSValue exception;
    JS_BOOL identical;

    if (!JS_IsException(result)) {
        JS_FreeValue(ctx, result);
        fprintf(stderr, "%s unexpectedly succeeded\n", site);
        return -1;
    }
    exception = JS_GetException(ctx);
    identical = JS_StrictEq(ctx, exception, expected);
    printf("%s exception-kind=%s identical=%s\n", site,
           value_kind(exception), identical ? "true" : "false");
    JS_FreeValue(ctx, exception);
    return identical ? 0 : -1;
}

static JSValue eval_global(JSContext *ctx, const char *source)
{
    return JS_Eval(ctx, source, strlen(source), "callback-probe.js",
                   JS_EVAL_TYPE_GLOBAL);
}

static int print_global_string(JSContext *ctx, const char *site,
                               const char *source)
{
    JSValue value = eval_global(ctx, source);
    const char *text;

    if (JS_IsException(value)) {
        JSValue exception = JS_GetException(ctx);
        JS_FreeValue(ctx, exception);
        fprintf(stderr, "%s global query failed\n", site);
        return -1;
    }
    text = JS_ToCString(ctx, value);
    if (!text) {
        JS_FreeValue(ctx, value);
        fprintf(stderr, "%s global query was not printable\n", site);
        return -1;
    }
    printf("%s=%s\n", site, text);
    JS_FreeCString(ctx, text);
    JS_FreeValue(ctx, value);
    return 0;
}

static char *normalize_module(JSContext *ctx, const char *base_name,
                              const char *specifier, void *opaque)
{
    ProbeState *state = opaque;
    const char *normalized = specifier;

    printf("normalize[%u] depth=%u stage=%s base=%s spec=%s\n",
           state->normalize_count++, state->loader_depth,
           stage_name(state->stage), base_name, specifier);
    if (state->stage == PROBE_NORMALIZE_THROW) {
        JS_Throw(ctx, JS_DupValue(ctx, state->normalize_reason));
        return NULL;
    }
    if (normalized[0] == '.' && normalized[1] == '/')
        normalized += 2;
    return js_strdup(ctx, normalized);
}

static int check_attributes(JSContext *ctx, void *opaque,
                            JSValueConst attributes)
{
    ProbeState *state = opaque;

    printf("check[%u] stage=%s attrs=%s\n", state->check_count++,
           stage_name(state->stage),
           JS_IsObject(attributes) ? "object" : "other");
    if (state->stage == PROBE_ATTRIBUTE_THROW) {
        JS_Throw(ctx, JS_DupValue(ctx, state->attribute_reason));
        return -1;
    }
    return 0;
}

static JSModuleDef *compile_loaded_module(JSContext *ctx, ProbeState *state,
                                          const char *module_name,
                                          const char *source)
{
    JSValue function;
    JSModuleDef *module;

    state->loader_depth++;
    function = JS_Eval(ctx, source, strlen(source), module_name,
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    state->loader_depth--;
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

    if (state->loader_depth > state->maximum_loader_depth)
        state->maximum_loader_depth = state->loader_depth;
    printf("load[%u] depth=%u stage=%s name=%s\n", state->load_count++,
           state->loader_depth, stage_name(state->stage), module_name);

    switch (state->stage) {
    case PROBE_LOAD_THROW:
        JS_Throw(ctx, JS_DupValue(ctx, state->load_reason));
        return NULL;
    case PROBE_LOAD_RECOVER:
        if (!strcmp(module_name, "load.js")) {
            return compile_loaded_module(
                ctx, state, module_name,
                "globalThis.loadRecovered = 42; export const value = 42;");
        }
        break;
    case PROBE_DYNAMIC_THROW:
        JS_Throw(ctx, JS_DupValue(ctx, state->dynamic_reason));
        return NULL;
    case PROBE_REENTRY:
        if (!strcmp(module_name, "outer.js")) {
            return compile_loaded_module(
                ctx, state, module_name,
                "import './inner.js'; globalThis.reentryOrder.push('outer'); "
                "export const outer = 1;");
        }
        if (!strcmp(module_name, "inner.js")) {
            return compile_loaded_module(
                ctx, state, module_name,
                "globalThis.reentryOrder.push('inner'); export const inner = 1;");
        }
        break;
    case PROBE_NORMALIZE_THROW:
    case PROBE_ATTRIBUTE_THROW:
        break;
    }
    JS_ThrowReferenceError(ctx, "unexpected probe module '%s'", module_name);
    return NULL;
}

static int run_recovery_module(JSContext *ctx)
{
    static const char source[] = "import './load.js';";
    JSValue result = JS_Eval(ctx, source, sizeof(source) - 1,
                             "load-retry-entry.js", JS_EVAL_TYPE_MODULE);

    if (JS_IsException(result)) {
        JSValue exception = JS_GetException(ctx);
        JS_FreeValue(ctx, exception);
        fputs("load retry unexpectedly failed\n", stderr);
        return -1;
    }
    JS_FreeValue(ctx, result);
    return print_global_string(ctx, "load-retry",
                               "globalThis.loadRecovered === 42");
}

static int run_dynamic_probe(JSRuntime *runtime, JSContext *ctx,
                             ProbeState *state)
{
    static const char source[] =
        "globalThis.dynamicEvents = ['before'];"
        "import('./dynamic.js').catch(reason => "
        "  dynamicEvents.push('catch:' + (reason === dynamicReason)));"
        "dynamicEvents.push('after');"
        "JSON.stringify(dynamicEvents);";
    JSContext *job_context;
    JSValue value;
    int status;

    state->stage = PROBE_DYNAMIC_THROW;
    value = eval_global(ctx, source);
    if (JS_IsException(value)) {
        JSValue exception = JS_GetException(ctx);
        JS_FreeValue(ctx, exception);
        fputs("dynamic import scheduling unexpectedly failed\n", stderr);
        return -1;
    }
    {
        const char *text = JS_ToCString(ctx, value);
        if (!text) {
            JS_FreeValue(ctx, value);
            return -1;
        }
        printf("dynamic-scheduled=%s\n", text);
        JS_FreeCString(ctx, text);
    }
    JS_FreeValue(ctx, value);

    while ((status = JS_ExecutePendingJob(runtime, &job_context)) > 0) {
    }
    if (status < 0) {
        fputs("dynamic import job unexpectedly escaped as an exception\n", stderr);
        return -1;
    }
    return print_global_string(ctx, "dynamic-final",
                               "JSON.stringify(dynamicEvents)");
}

static int run_reentry_probe(JSContext *ctx, ProbeState *state)
{
    static const char source[] =
        "import './outer.js'; globalThis.reentryOrder.push('entry');";
    JSValue result;

    state->stage = PROBE_REENTRY;
    result = eval_global(ctx, "globalThis.reentryOrder = [];");
    if (JS_IsException(result))
        return -1;
    JS_FreeValue(ctx, result);

    result = JS_Eval(ctx, source, sizeof(source) - 1, "reentry-entry.js",
                     JS_EVAL_TYPE_MODULE);
    if (JS_IsException(result)) {
        JSValue exception = JS_GetException(ctx);
        JS_FreeValue(ctx, exception);
        fputs("reentrant loader unexpectedly failed\n", stderr);
        return -1;
    }
    JS_FreeValue(ctx, result);
    printf("reentry result=ok max-loader-depth=%u\n",
           state->maximum_loader_depth);
    return print_global_string(ctx, "reentry-order",
                               "JSON.stringify(reentryOrder)");
}

int main(void)
{
    static const char normalize_source[] = "import './normalize.js';";
    static const char attribute_source[] =
        "import './attribute.js' with { type: 'json' };";
    static const char load_source[] = "import './load.js';";
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;
    JSValue result;
    JSValue global;
    ProbeState state = { 0 };
    int status = 0;

    if (!runtime)
        return 2;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return 2;
    }

    state.normalize_reason = JS_NewObject(context);
    state.attribute_reason = JS_NewInt32(context, 42);
    state.load_reason = eval_global(context, "Symbol('load-reason')");
    state.dynamic_reason = JS_NewObject(context);
    if (JS_IsException(state.normalize_reason) ||
        JS_IsException(state.load_reason) ||
        JS_IsException(state.dynamic_reason)) {
        status = 2;
        goto done;
    }
    global = JS_GetGlobalObject(context);
    if (JS_IsException(global) ||
        JS_SetPropertyStr(context, global, "dynamicReason",
                          JS_DupValue(context, state.dynamic_reason)) < 0) {
        JS_FreeValue(context, global);
        status = 2;
        goto done;
    }
    JS_FreeValue(context, global);
    JS_SetModuleLoaderFunc2(runtime, normalize_module, load_module,
                            check_attributes, &state);

    state.stage = PROBE_NORMALIZE_THROW;
    result = JS_Eval(context, normalize_source, sizeof(normalize_source) - 1,
                     "normalize-entry.js",
                     JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (print_expected_exception(context, result, "normalize",
                                 state.normalize_reason) < 0) {
        status = 1;
        goto done;
    }

    state.stage = PROBE_ATTRIBUTE_THROW;
    result = JS_Eval(context, attribute_source, sizeof(attribute_source) - 1,
                     "attribute-entry.js",
                     JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (print_expected_exception(context, result, "attribute",
                                 state.attribute_reason) < 0) {
        status = 1;
        goto done;
    }

    state.stage = PROBE_LOAD_THROW;
    result = JS_Eval(context, load_source, sizeof(load_source) - 1,
                     "load-entry.js",
                     JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (print_expected_exception(context, result, "load", state.load_reason) < 0) {
        status = 1;
        goto done;
    }

    state.stage = PROBE_LOAD_RECOVER;
    if (run_recovery_module(context) < 0 ||
        run_dynamic_probe(runtime, context, &state) < 0 ||
        run_reentry_probe(context, &state) < 0) {
        status = 1;
        goto done;
    }
    printf("summary normalizes=%u checks=%u loads=%u\n",
           state.normalize_count, state.check_count, state.load_count);

done:
    JS_FreeValue(context, state.normalize_reason);
    JS_FreeValue(context, state.attribute_reason);
    JS_FreeValue(context, state.load_reason);
    JS_FreeValue(context, state.dynamic_reason);
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return status;
}
