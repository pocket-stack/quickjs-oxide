/*
 * QuickJS 2026-06-04 oracle for dynamic import through a re-entrant loader.
 *
 * This is test-only C and links only against the pinned external oracle. The
 * product engine remains Rust-only.
 */

#include "quickjs.h"

#include <stdio.h>
#include <string.h>

typedef struct ProbeState {
    JSContext *expected_context;
    unsigned int normalize_count;
    unsigned int load_count;
    unsigned int loader_depth;
    int context_mismatch;
} ProbeState;

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
    printf("normalize[%u] depth=%u context=%s base=%s spec=%s\n",
           state->normalize_count++, state->loader_depth,
           same_context ? "same" : "different", base_name, specifier);
    if (normalized[0] == '.' && normalized[1] == '/')
        normalized += 2;
    return js_strdup(ctx, normalized);
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
    int same_context = ctx == state->expected_context;

    (void)attributes;
    if (!same_context)
        state->context_mismatch = 1;
    printf("load[%u] depth=%u context=%s name=%s\n", state->load_count++,
           state->loader_depth, same_context ? "same" : "different",
           module_name);
    if (!strcmp(module_name, "outer.js")) {
        return compile_loaded_module(
            ctx, state, module_name,
            "import { value } from './inner.js';"
            "globalThis.__dynamicReentryOrder.push('outer');"
            "export const answer = value + 1;");
    }
    if (!strcmp(module_name, "inner.js")) {
        return compile_loaded_module(
            ctx, state, module_name,
            "globalThis.__dynamicReentryOrder.push('inner');"
            "export const value = 41;");
    }
    JS_ThrowReferenceError(ctx, "unexpected oracle module '%s'", module_name);
    return NULL;
}

static const char *promise_state_name(JSPromiseStateEnum state)
{
    switch (state) {
    case JS_PROMISE_PENDING:
        return "pending";
    case JS_PROMISE_FULFILLED:
        return "fulfilled";
    case JS_PROMISE_REJECTED:
        return "rejected";
    }
    return "unknown";
}

static int print_final_result(JSContext *ctx)
{
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue order = JS_GetPropertyStr(ctx, global, "__dynamicReentryOrder");
    JSValue json = JS_JSONStringify(ctx, order, JS_UNDEFINED, JS_UNDEFINED);
    JSValue result = JS_GetPropertyStr(ctx, global, "__dynamicReentryResult");
    JSValue promise = JS_GetPropertyStr(ctx, global, "__dynamicReentryPromise");
    JSPromiseStateEnum state = JS_PromiseState(ctx, promise);
    JSValue promise_result = JS_PromiseResult(ctx, promise);
    const char *order_text = JS_ToCString(ctx, json);
    int32_t result_number;
    int32_t promise_result_number;
    int status = 0;

    if (!order_text || JS_ToInt32(ctx, &result_number, result) < 0 ||
        JS_ToInt32(ctx, &promise_result_number, promise_result) < 0) {
        status = -1;
    } else {
        printf("final order=%s result=%d promise=%s promise-result=%d\n",
               order_text, result_number, promise_state_name(state),
               promise_result_number);
        if (strcmp(order_text, "[\"inner\",\"outer\",\"entry\"]") ||
            result_number != 42 || state != JS_PROMISE_FULFILLED ||
            promise_result_number != 42) {
            status = -1;
        }
    }

    if (order_text)
        JS_FreeCString(ctx, order_text);
    JS_FreeValue(ctx, promise_result);
    JS_FreeValue(ctx, promise);
    JS_FreeValue(ctx, result);
    JS_FreeValue(ctx, json);
    JS_FreeValue(ctx, order);
    JS_FreeValue(ctx, global);
    return status;
}

int main(void)
{
    static const char source[] =
        "globalThis.__dynamicReentryOrder = [];"
        "globalThis.__dynamicReentryResult = 'pending';"
        "globalThis.__dynamicReentryPromise = "
        "import('./outer.js').then(function (namespace) {"
        "  globalThis.__dynamicReentryOrder.push('entry');"
        "  globalThis.__dynamicReentryResult = namespace.answer;"
        "  return namespace.answer;"
        "});";
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;
    JSContext *job_context = NULL;
    JSValue evaluation;
    ProbeState state = { 0 };
    unsigned int job_count = 0;
    int job_status;
    int status = 0;

    if (!runtime)
        return 2;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return 2;
    }
    state.expected_context = context;
    JS_SetModuleLoaderFunc2(runtime, normalize_module, load_module, NULL,
                            &state);

    evaluation = JS_Eval(context, source, sizeof(source) - 1,
                         "dynamic-reentry-entry.js", JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(evaluation)) {
        print_exception(context, "dynamic import scheduling");
        status = 1;
        goto done;
    }
    printf("scheduled promise=%s normalizes=%u loads=%u\n",
           promise_state_name(JS_PromiseState(context, evaluation)),
           state.normalize_count, state.load_count);
    if (JS_PromiseState(context, evaluation) != JS_PROMISE_PENDING ||
        state.normalize_count != 0 || state.load_count != 0) {
        fputs("dynamic import did not defer the host callbacks\n", stderr);
        status = 1;
        goto done;
    }
    JS_FreeValue(context, evaluation);
    evaluation = JS_UNDEFINED;

    while ((job_status = JS_ExecutePendingJob(runtime, &job_context)) > 0) {
        job_count++;
        if (job_context != context) {
            fputs("pending job used a different context\n", stderr);
            status = 1;
            goto done;
        }
        if (job_count > 16) {
            fputs("dynamic import jobs did not quiesce\n", stderr);
            status = 1;
            goto done;
        }
    }
    if (job_status < 0) {
        print_exception(job_context ? job_context : context,
                        "dynamic import job");
        status = 1;
        goto done;
    }
    if (state.context_mismatch || state.loader_depth != 0 ||
        state.normalize_count != 2 || state.load_count != 2) {
        fputs("dynamic loader callback contract mismatch\n", stderr);
        status = 1;
        goto done;
    }
    if (print_final_result(context) < 0) {
        fputs("dynamic import final result mismatch\n", stderr);
        status = 1;
    }

done:
    JS_FreeValue(context, evaluation);
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return status;
}
