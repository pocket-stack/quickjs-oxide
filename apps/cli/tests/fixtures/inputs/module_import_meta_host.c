#include "quickjs-libc.h"
#include "quickjs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static JSModuleDef *module_loader(JSContext *ctx, const char *module_name,
                                  void *opaque) {
    const char *source =
        "export const url = import.meta.url;"
        "export const main = import.meta.main;";
    JSValue module = JS_Eval(ctx, source, strlen(source), module_name,
                             JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(module))
        return NULL;
    if (js_module_set_import_meta(ctx, module, 0, 0) < 0) {
        JS_FreeValue(ctx, module);
        return NULL;
    }
    JSModuleDef *def = JS_VALUE_GET_PTR(module);
    JS_FreeValue(ctx, module);
    return def;
}

static void fail(JSContext *ctx, const char *message) {
    JSValue exception = JS_GetException(ctx);
    const char *text = JS_ToCString(ctx, exception);
    fprintf(stderr, "%s", message);
    if (text)
        fprintf(stderr, ": %s", text);
    fputc('\n', stderr);
    JS_FreeCString(ctx, text);
    JS_FreeValue(ctx, exception);
    exit(1);
}

int main(void) {
    JSRuntime *rt = JS_NewRuntime();
    if (!rt)
        return 1;
    JSContext *ctx = JS_NewContext(rt);
    if (!ctx) {
        JS_FreeRuntime(rt);
        return 1;
    }
    JS_SetModuleLoaderFunc(rt, NULL, module_loader, NULL);

    const char *source =
        "import { url, main } from './dependency.js';"
        "globalThis.result = ["
        "  import.meta === import.meta,"
        "  Object.getPrototypeOf(import.meta) === null,"
        "  import.meta.url, import.meta.main, url, main,"
        "  Object.getOwnPropertyDescriptor(import.meta, 'url')"
        "];";
    JSValue module = JS_Eval(ctx, source, strlen(source), "entry.js",
                             JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(module))
        fail(ctx, "compile failed");
    if (js_module_set_import_meta(ctx, module, 0, 1) < 0)
        fail(ctx, "entry import.meta initialization failed");
    JSValue promise = JS_EvalFunction(ctx, module);
    if (JS_IsException(promise))
        fail(ctx, "evaluation failed");
    JS_FreeValue(ctx, promise);

    JSValue global = JS_GetGlobalObject(ctx);
    JSValue result = JS_GetPropertyStr(ctx, global, "result");
    JSValue json = JS_JSONStringify(ctx, result, JS_UNDEFINED, JS_UNDEFINED);
    const char *text = JS_ToCString(ctx, json);
    if (!text)
        fail(ctx, "stringify failed");
    puts(text);
    JS_FreeCString(ctx, text);
    JS_FreeValue(ctx, json);
    JS_FreeValue(ctx, result);
    JS_FreeValue(ctx, global);
    JS_FreeContext(ctx);
    JS_FreeRuntime(rt);
    return 0;
}
