/*
 * QuickJS 2026-06-04 oracle for parse-time module cache publication.
 *
 * This is test-only C and links only against the pinned external oracle. The
 * product engine remains Rust-only.
 */

#include "quickjs.h"

#include <stdio.h>
#include <string.h>

typedef enum ProbeMode {
    PROBE_SUCCESS,
    PROBE_FAILURE,
} ProbeMode;

typedef struct ProbeState {
    ProbeMode mode;
    unsigned int check_count;
    unsigned int load_count;
    JSValue nested_module;
} ProbeState;

static const char *mode_name(ProbeMode mode)
{
    return mode == PROBE_SUCCESS ? "success" : "failure";
}

static const char *boolean_name(int value)
{
    return value ? "true" : "false";
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

static int check_attributes(JSContext *ctx, void *opaque,
                            JSValueConst attributes)
{
    ProbeState *state = opaque;
    const char *label = mode_name(state->mode);
    unsigned int check_index = state->check_count++;
    const char *source = state->mode == PROBE_SUCCESS
                             ? "export const marker = 99;"
                             : "export const broken = ;";

    printf("%s check[%u] attrs=%s has-exception=%s\n", label, check_index,
           JS_IsObject(attributes) ? "object" : "other",
           boolean_name(JS_HasException(ctx)));
    if (check_index != 0) {
        JS_ThrowInternalError(ctx, "unexpected recursive attribute checker");
        return -1;
    }

    state->nested_module =
        JS_Eval(ctx, source, strlen(source), "same.js",
                JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    printf("%s reentry result=%s has-exception=%s\n", label,
           JS_IsException(state->nested_module) ? "exception" : "module",
           boolean_name(JS_HasException(ctx)));
    return JS_IsException(state->nested_module) ? -1 : 0;
}

static JSModuleDef *load_module(JSContext *ctx, const char *module_name,
                                void *opaque, JSValueConst attributes)
{
    ProbeState *state = opaque;

    (void)attributes;
    printf("%s load[%u] name=%s\n", mode_name(state->mode),
           state->load_count++, module_name);
    JS_ThrowReferenceError(ctx, "unexpected loader call for '%s'",
                           module_name);
    return NULL;
}

static int run_success_scenario(void)
{
    static const char source[] =
        "import { marker as cachedMarker } from './same.js' with { type: "
        "'probe' };"
        "export const marker = 41;"
        "globalThis.__cachePublicationResult = cachedMarker + 1;";
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;
    ProbeState state = {
        .mode = PROBE_SUCCESS,
        .nested_module = JS_UNDEFINED,
    };
    JSValue outer_module = JS_UNDEFINED;
    JSValue evaluation = JS_UNDEFINED;
    JSValue global = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    int32_t result_number = 0;
    int modules_are_distinct;
    int status = 0;

    if (!runtime)
        return 2;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return 2;
    }
    JS_SetModuleLoaderFunc2(runtime, NULL, load_module, check_attributes,
                            &state);

    outer_module = JS_Eval(
        context, source, sizeof(source) - 1, "same.js",
        JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    modules_are_distinct =
        !JS_IsException(outer_module) &&
        !JS_IsException(state.nested_module) &&
        JS_VALUE_GET_PTR(outer_module) != JS_VALUE_GET_PTR(state.nested_module);
    printf("success outer result=%s distinct=%s checks=%u loads=%u "
           "has-exception=%s\n",
           JS_IsException(outer_module) ? "exception" : "module",
           boolean_name(modules_are_distinct), state.check_count,
           state.load_count, boolean_name(JS_HasException(context)));
    if (JS_IsException(outer_module) || !modules_are_distinct ||
        state.check_count != 1 || state.load_count != 0 ||
        JS_HasException(context)) {
        status = 1;
        goto done;
    }

    evaluation = JS_EvalFunction(context, outer_module);
    outer_module = JS_UNDEFINED;
    if (JS_IsException(evaluation)) {
        status = 1;
        goto done;
    }
    global = JS_GetGlobalObject(context);
    result = JS_GetPropertyStr(context, global, "__cachePublicationResult");
    if (JS_IsException(result) ||
        JS_ToInt32(context, &result_number, result) < 0) {
        status = 1;
        goto done;
    }
    printf("success evaluate promise=%s result=%d has-exception=%s\n",
           promise_state_name(JS_PromiseState(context, evaluation)),
           result_number, boolean_name(JS_HasException(context)));
    if (JS_PromiseState(context, evaluation) != JS_PROMISE_FULFILLED ||
        result_number != 42 || JS_HasException(context)) {
        status = 1;
    }

done:
    JS_FreeValue(context, result);
    JS_FreeValue(context, global);
    JS_FreeValue(context, evaluation);
    JS_FreeValue(context, outer_module);
    JS_FreeValue(context, state.nested_module);
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return status;
}

static int run_failure_scenario(void)
{
    static const char source[] =
        "import './same.js' with { type: 'probe' };"
        "export const marker = 41;";
    static const char retry_source[] = "export const marker = 42;";
    JSRuntime *runtime = JS_NewRuntime();
    JSContext *context;
    ProbeState state = {
        .mode = PROBE_FAILURE,
        .nested_module = JS_UNDEFINED,
    };
    JSValue outer_module;
    JSValue exception;
    JSValue exception_name;
    JSValue retry_module;
    const char *exception_name_string;
    int status = 0;

    if (!runtime)
        return 2;
    context = JS_NewContext(runtime);
    if (!context) {
        JS_FreeRuntime(runtime);
        return 2;
    }
    JS_SetModuleLoaderFunc2(runtime, NULL, load_module, check_attributes,
                            &state);

    outer_module = JS_Eval(
        context, source, sizeof(source) - 1, "same.js",
        JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    printf("failure outer result=%s checks=%u loads=%u has-exception=%s\n",
           JS_IsException(outer_module) ? "exception" : "module",
           state.check_count, state.load_count,
           boolean_name(JS_HasException(context)));
    if (!JS_IsException(outer_module) || state.check_count != 1 ||
        state.load_count != 0 || !JS_HasException(context)) {
        JS_FreeValue(context, outer_module);
        status = 1;
        goto done;
    }

    exception = JS_GetException(context);
    exception_name = JS_GetPropertyStr(context, exception, "name");
    exception_name_string = JS_ToCString(context, exception_name);
    printf("failure exception=%s has-exception-after-get=%s\n",
           exception_name_string ? exception_name_string : "<non-string>",
           boolean_name(JS_HasException(context)));
    if (!exception_name_string || strcmp(exception_name_string, "SyntaxError") ||
        JS_HasException(context)) {
        status = 1;
    }
    if (exception_name_string)
        JS_FreeCString(context, exception_name_string);
    JS_FreeValue(context, exception_name);
    JS_FreeValue(context, exception);

    retry_module = JS_Eval(
        context, retry_source, sizeof(retry_source) - 1, "same.js",
        JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    printf("failure retry result=%s checks=%u loads=%u has-exception=%s\n",
           JS_IsException(retry_module) ? "exception" : "module",
           state.check_count, state.load_count,
           boolean_name(JS_HasException(context)));
    if (JS_IsException(retry_module) || state.check_count != 1 ||
        state.load_count != 0 || JS_HasException(context)) {
        status = 1;
    }
    JS_FreeValue(context, retry_module);

done:
    JS_FreeValue(context, state.nested_module);
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
    return status;
}

int main(void)
{
    int status = run_success_scenario();

    if (status)
        return status;
    return run_failure_scenario();
}
