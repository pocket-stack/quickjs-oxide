/*
 * QuickJS 2026-06-04 oracle for the standard file loader's JSON/JSON5
 * classification and extended-JSON grammar.
 *
 * This is test-only C and links only against the pinned external oracle. It
 * writes hermetic source files below a temporary working directory and calls
 * the exported quickjs-libc loader and attribute checker. The tracing wrapper
 * observes js_module_test_json() and filename suffixes, but deliberately does
 * not reproduce or influence the loader's classification decision.
 */

#define _POSIX_C_SOURCE 200809L
#define _XOPEN_SOURCE 700
#define _DARWIN_C_SOURCE

#include "quickjs.h"
#include "quickjs-libc.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

typedef struct FixtureFile {
    const char *path;
    const char *source;
} FixtureFile;

typedef struct OracleState {
    const char *label;
    unsigned int request_count;
    unsigned int type_test_none_count;
    unsigned int type_test_json_count;
    unsigned int type_test_json5_count;
    unsigned int json_suffix_count;
    unsigned int json5_suffix_count;
    int last_type_test;
    int last_json_suffix;
    int last_json5_suffix;
} OracleState;

typedef struct ExceptionExpectation {
    const char *name;
    const char *message;
    const char *file_name;
    const char *line_number;
    const char *column_number;
} ExceptionExpectation;

typedef struct Workspace {
    char original_directory[PATH_MAX];
    char temporary_directory[PATH_MAX];
} Workspace;

static const FixtureFile fixture_files[] = {
    {"classification/data/strict-suffix.json",
     "{\"kind\":\"suffix\",\"answer\":42}\n"},
    {"classification/data/strict-attribute.data",
     "{\"kind\":\"attribute\",\"answer\":42}\n"},
    {"classification/data/extended.data",
     "/* leading block comment */\n"
     "{\n"
     "  // line comment\n"
     "  bare: 'single quoted',\n"
     "  verticalEscape: 'a\\vb',\n"
     "  continued: 'left\\\nright',\n"
     "  trailing: [1, 2,],\n"
     "\f\v  plus: +.5,\n"
     "  leadingDot: .25,\n"
     "  hexadecimal: 0x2a,\n"
     "  octal: 0o52,\n"
     "  binary: 0b101010,\n"
     "  notANumber: NaN,\n"
     "  positiveInfinity: +Infinity,\n"
     "  negativeInfinity: -Infinity,\n"
     "}\n"},
    {"classification/data/extended-on-json.json",
     "{route: 'json5-on-json-suffix',}\n"},
    {"classification/data/script.json5",
     "export default 42;\n"
     "export const route = 'javascript';\n"
     "export const metaUrl = import.meta.url;\n"
     "export const metaMain = import.meta.main;\n"},
    {"strict-suffix-reject/data/strict-suffix-reject.json", "{bare: 1}\n"},
    {"strict-attribute-reject/data/strict-attribute-reject.data",
     "{bare: 1}\n"},
    {"unicode-bare-reject/data/unicode-bare.data", "{\xc3\xa9: 1}\n"},
    {"number-dot-reject/data/number-dot.data", "{\"value\": 1.}\n"},
    {"cr-continuation-reject/data/cr-continuation.data",
     "{'value':'left\\\rright'}\n"},
    {"crlf-continuation-reject/data/crlf-continuation.data",
     "{'value':'left\\\r\nright'}\n"},
};

static const char *const fixture_directories[] = {
    "classification",
    "classification/data",
    "strict-suffix-reject",
    "strict-suffix-reject/data",
    "strict-attribute-reject",
    "strict-attribute-reject/data",
    "unicode-bare-reject",
    "unicode-bare-reject/data",
    "number-dot-reject",
    "number-dot-reject/data",
    "cr-continuation-reject",
    "cr-continuation-reject/data",
    "crlf-continuation-reject",
    "crlf-continuation-reject/data",
};

static int observed_suffix(const char *string, const char *suffix)
{
    size_t string_length = strlen(string);
    size_t suffix_length = strlen(suffix);

    return string_length >= suffix_length &&
           !memcmp(string + string_length - suffix_length, suffix,
                   suffix_length);
}

static int write_fixture_file(const FixtureFile *fixture)
{
    const char *cursor = fixture->source;
    size_t remaining = strlen(fixture->source);
    int descriptor = open(fixture->path, O_WRONLY | O_CREAT | O_EXCL, 0600);

    if (descriptor < 0)
        return -1;
    while (remaining > 0) {
        ssize_t written = write(descriptor, cursor, remaining);

        if (written < 0) {
            if (errno == EINTR)
                continue;
            close(descriptor);
            return -1;
        }
        cursor += (size_t)written;
        remaining -= (size_t)written;
    }
    if (close(descriptor) < 0)
        return -1;
    return 0;
}

static int cleanup_workspace(Workspace *workspace)
{
    size_t index;
    int status = 0;

    if (!workspace->temporary_directory[0])
        return 0;
    if (chdir(workspace->temporary_directory) == 0) {
        for (index = 0;
             index < sizeof(fixture_files) / sizeof(fixture_files[0]);
             index++) {
            if (unlink(fixture_files[index].path) < 0 && errno != ENOENT)
                status = -1;
        }
        for (index =
                 sizeof(fixture_directories) / sizeof(fixture_directories[0]);
             index > 0; index--) {
            if (rmdir(fixture_directories[index - 1]) < 0 && errno != ENOENT)
                status = -1;
        }
    } else {
        status = -1;
    }
    if (workspace->original_directory[0] &&
        chdir(workspace->original_directory) < 0)
        return -1;
    if (rmdir(workspace->temporary_directory) < 0 && errno != ENOENT)
        status = -1;
    return status;
}

static int prepare_workspace(Workspace *workspace)
{
    char template[] = "/tmp/quickjs-oxide-module-json5.XXXXXX";
    char *temporary_directory;
    size_t index;

    memset(workspace, 0, sizeof(*workspace));
    if (!getcwd(workspace->original_directory,
                sizeof(workspace->original_directory)))
        return -1;
    temporary_directory = mkdtemp(template);
    if (!temporary_directory)
        return -1;
    if (strlen(temporary_directory) >= sizeof(workspace->temporary_directory)) {
        rmdir(temporary_directory);
        return -1;
    }
    strcpy(workspace->temporary_directory, temporary_directory);
    if (chdir(workspace->temporary_directory) < 0)
        return -1;
    for (index = 0;
         index < sizeof(fixture_directories) /
                     sizeof(fixture_directories[0]);
         index++) {
        if (mkdir(fixture_directories[index], 0700) < 0)
            return -1;
    }
    for (index = 0;
         index < sizeof(fixture_files) / sizeof(fixture_files[0]); index++) {
        if (write_fixture_file(&fixture_files[index]) < 0)
            return -1;
    }
    return 0;
}

/*
 * Observe only upstream's exported attribute probe and literal suffix facts,
 * then delegate the actual decision, parsing, C-module construction, and
 * JavaScript import.meta setup to quickjs-libc's js_module_loader().
 */
static JSModuleDef *tracing_module_loader(JSContext *ctx,
                                          const char *module_name,
                                          void *opaque,
                                          JSValueConst attributes)
{
    OracleState *state = opaque;
    unsigned int request_index;
    int type_test;
    int json_suffix;
    int json5_suffix;

    if (!state) {
        JS_ThrowInternalError(ctx, "JSON5 oracle state is missing");
        return NULL;
    }
    type_test = js_module_test_json(ctx, attributes);
    if (JS_HasException(ctx))
        return NULL;
    json_suffix = observed_suffix(module_name, ".json");
    json5_suffix = observed_suffix(module_name, ".json5");
    state->last_type_test = type_test;
    state->last_json_suffix = json_suffix;
    state->last_json5_suffix = json5_suffix;
    if (type_test == 1)
        state->type_test_json_count++;
    else if (type_test == 2)
        state->type_test_json5_count++;
    else
        state->type_test_none_count++;
    if (json_suffix)
        state->json_suffix_count++;
    if (json5_suffix)
        state->json5_suffix_count++;
    request_index = state->request_count++;
    printf("%s request[%u] name=%s observed-type-test=%d "
           "observed-json-suffix=%s observed-json5-suffix=%s\n",
           state->label, request_index, module_name, type_test,
           json_suffix ? "true" : "false",
           json5_suffix ? "true" : "false");
    return js_module_loader(ctx, module_name, NULL, attributes);
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
    JS_SetModuleLoaderFunc2(runtime, NULL, tracing_module_loader,
                            js_module_check_attributes, state);
    *runtime_out = runtime;
    *context_out = context;
    return 0;
}

static void finish_scenario(JSRuntime *runtime, JSContext *context)
{
    JS_FreeContext(context);
    JS_FreeRuntime(runtime);
}

static int print_and_check_exception(JSContext *ctx, const char *site,
                                     const ExceptionExpectation *expected)
{
    static const char *const property_names[] = {
        "name", "message", "fileName", "lineNumber", "columnNumber",
    };
    const char *properties[sizeof(property_names) / sizeof(property_names[0])];
    const char *expected_values[] = {
        expected->name,
        expected->message,
        expected->file_name,
        expected->line_number,
        expected->column_number,
    };
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
        for (index = 0;
             index < sizeof(property_names) / sizeof(property_names[0]);
             index++) {
            if (expected_values[index] &&
                strcmp(properties[index], expected_values[index])) {
                fprintf(stderr,
                        "%s diagnostic mismatch for %s: expected '%s', got "
                        "'%s'\n",
                        site, property_names[index], expected_values[index],
                        properties[index]);
                result = -1;
            }
        }
    }
    for (index = 0; index < sizeof(properties) / sizeof(properties[0]);
         index++) {
        if (properties[index])
            JS_FreeCString(ctx, properties[index]);
    }
    JS_FreeValue(ctx, exception);
    return result;
}

static int print_and_check_global(JSContext *ctx, const char *site,
                                  const char *expected)
{
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue value = JS_GetPropertyStr(ctx, global, "__oracle");
    JSValue json;
    const char *json_string;
    int result;

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
    result = strcmp(json_string, expected) ? -1 : 0;
    if (result)
        fprintf(stderr, "%s value mismatch\n", site);
    JS_FreeCString(ctx, json_string);
    JS_FreeValue(ctx, json);
    return result;
}

static int run_classification_scenario(void)
{
    static const char source[] =
        "import strictSuffix from './data/strict-suffix.json';\n"
        "import strictAttribute from './data/strict-attribute.data' with { "
        "type: 'json' };\n"
        "import extended from './data/extended.data' with { type: 'json5' "
        "};\n"
        "import * as extendedNamespace from './data/extended.data' with { "
        "type: 'json5' };\n"
        "import * as extendedAlias from '../classification/data/extended.data' "
        "with { type: 'json5' };\n"
        "import extendedOnJson from './data/extended-on-json.json' with { "
        "type: 'json5' };\n"
        "import scriptDefault, { route as scriptRoute, metaUrl as "
        "scriptMetaUrl, metaMain as scriptMetaMain } from "
        "'./data/script.json5';\n"
        "globalThis.__oracle = {\n"
        "  strictSuffix: strictSuffix.kind === 'suffix' && "
        "strictSuffix.answer === 42,\n"
        "  strictAttribute: strictAttribute.kind === 'attribute' && "
        "strictAttribute.answer === 42,\n"
        "  extended: {\n"
        "    singleQuote: extended.bare === 'single quoted',\n"
        "    verticalEscape: extended.verticalEscape.length === 3 && "
        "extended.verticalEscape.charCodeAt(1) === 11,\n"
        "    multiline: extended.continued === 'leftright',\n"
        "    trailingComma: extended.trailing.join(',') === '1,2',\n"
        "    signedLeadingDot: extended.plus === 0.5,\n"
        "    leadingDot: extended.leadingDot === 0.25,\n"
        "    hexadecimal: extended.hexadecimal === 42,\n"
        "    octal: extended.octal === 42,\n"
        "    binary: extended.binary === 42,\n"
        "    nan: Number.isNaN(extended.notANumber),\n"
        "    positiveInfinity: extended.positiveInfinity === Infinity,\n"
        "    negativeInfinity: extended.negativeInfinity === -Infinity\n"
        "  },\n"
        "  json5OnJsonSuffix: extendedOnJson.route,\n"
        "  json5ExtensionIsJavaScript: scriptDefault === 42 && "
        "scriptRoute === 'javascript' && scriptMetaMain === false && "
        "scriptMetaUrl.startsWith('file:///') && "
        "scriptMetaUrl.endsWith('/classification/data/script.json5'),\n"
        "  defaultExportIdentity: extendedNamespace.default === extended,\n"
        "  cacheNamespaceIdentity: extendedNamespace === extendedAlias,\n"
        "  namespaceKeys: Object.keys(extendedNamespace)\n"
        "};\n";
    static const char expected[] =
        "{\"strictSuffix\":true,\"strictAttribute\":true,\"extended\":{"
        "\"singleQuote\":true,\"verticalEscape\":true,\"multiline\":true,"
        "\"trailingComma\":true,\"signedLeadingDot\":true,"
        "\"leadingDot\":true,\"hexadecimal\":true,\"octal\":true,"
        "\"binary\":true,\"nan\":true,\"positiveInfinity\":true,"
        "\"negativeInfinity\":true},\"json5OnJsonSuffix\":"
        "\"json5-on-json-suffix\",\"json5ExtensionIsJavaScript\":true,"
        "\"defaultExportIdentity\":true,\"cacheNamespaceIdentity\":true,"
        "\"namespaceKeys\":[\"default\"]}";
    OracleState state = {"classification", 0, 0, 0, 0, 0, 0, 0, 0, 0};
    JSRuntime *runtime;
    JSContext *context;
    JSValue function;
    JSValue result;
    JSPromiseStateEnum promise_state;
    int scenario_status = 1;

    if (start_scenario(&state, &runtime, &context) < 0)
        return 1;
    function = JS_Eval(context, source, strlen(source),
                       "classification/entry.js",
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(function)) {
        static const ExceptionExpectation unexpected = {NULL, NULL, NULL,
                                                        NULL, NULL};
        print_and_check_exception(context, "classification compile",
                                  &unexpected);
        goto done;
    }
    printf("classification before-evaluate requests=%u type-test-0=%u "
           "type-test-json=%u type-test-json5=%u json-suffix=%u "
           "json5-suffix=%u\n",
           state.request_count, state.type_test_none_count,
           state.type_test_json_count, state.type_test_json5_count,
           state.json_suffix_count, state.json5_suffix_count);
    if (state.request_count != 5 || state.type_test_none_count != 2 ||
        state.type_test_json_count != 1 ||
        state.type_test_json5_count != 2 || state.json_suffix_count != 2 ||
        state.json5_suffix_count != 1)
        goto done;
    result = JS_EvalFunction(context, function);
    if (JS_IsException(result)) {
        static const ExceptionExpectation unexpected = {NULL, NULL, NULL,
                                                        NULL, NULL};
        print_and_check_exception(context, "classification evaluate",
                                  &unexpected);
        goto done;
    }
    promise_state = JS_PromiseState(context, result);
    JS_FreeValue(context, result);
    printf("classification after-evaluate state=%s requests=%u\n",
           promise_state == JS_PROMISE_FULFILLED ? "fulfilled" : "other",
           state.request_count);
    if (promise_state != JS_PROMISE_FULFILLED ||
        print_and_check_global(context, "classification", expected) < 0)
        goto done;
    printf("classification cases=5 status=pass\n");
    scenario_status = 0;
done:
    finish_scenario(runtime, context);
    return scenario_status;
}

static int run_rejection_scenario(const char *label, const char *source,
                                  const char *entry_name,
                                  int expected_type_test,
                                  int expected_json_suffix,
                                  int expected_json5_suffix,
                                  const ExceptionExpectation *expected)
{
    OracleState state = {label, 0, 0, 0, 0, 0, 0, 0, 0, 0};
    JSRuntime *runtime;
    JSContext *context;
    JSValue function;
    int scenario_status = 1;

    if (start_scenario(&state, &runtime, &context) < 0)
        return 1;
    function = JS_Eval(context, source, strlen(source), entry_name,
                       JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    if (!JS_IsException(function)) {
        JS_FreeValue(context, function);
        fprintf(stderr, "%s unexpectedly compiled\n", label);
        goto done;
    }
    if (print_and_check_exception(context, label, expected) < 0)
        goto done;
    if (state.request_count != 1 ||
        state.last_type_test != expected_type_test ||
        state.last_json_suffix != expected_json_suffix ||
        state.last_json5_suffix != expected_json5_suffix)
        goto done;
    printf("%s case=pass\n", label);
    scenario_status = 0;
done:
    finish_scenario(runtime, context);
    return scenario_status;
}

static int run_oracle(void)
{
    static const ExceptionExpectation strict_suffix_error = {
        "SyntaxError",
        "expecting property name",
        "strict-suffix-reject/data/strict-suffix-reject.json",
        "1",
        "2",
    };
    static const ExceptionExpectation strict_attribute_error = {
        "SyntaxError",
        "expecting property name",
        "strict-attribute-reject/data/strict-attribute-reject.data",
        "1",
        "2",
    };
    static const ExceptionExpectation unicode_bare_error = {
        "SyntaxError",
        "unexpected character",
        "unicode-bare-reject/data/unicode-bare.data",
        "1",
        "2",
    };
    static const ExceptionExpectation number_dot_error = {
        "SyntaxError",
        "Unterminated fractional number",
        "number-dot-reject/data/number-dot.data",
        "1",
        "13",
    };
    static const ExceptionExpectation cr_continuation_error = {
        "SyntaxError",
        "Bad escaped character",
        "cr-continuation-reject/data/cr-continuation.data",
        "1",
        "16",
    };
    static const ExceptionExpectation crlf_continuation_error = {
        "SyntaxError",
        "Bad escaped character",
        "crlf-continuation-reject/data/crlf-continuation.data",
        "1",
        "16",
    };
    unsigned int passed = 0;

    if (run_classification_scenario())
        return 1;
    passed += 5;
    if (run_rejection_scenario(
            "strict-suffix-reject",
            "import value from './data/strict-suffix-reject.json';\n",
            "strict-suffix-reject/entry.js", 0, 1, 0,
            &strict_suffix_error))
        return 1;
    passed++;
    if (run_rejection_scenario(
            "strict-attribute-reject",
            "import value from './data/strict-attribute-reject.data' with { "
            "type: 'json' };\n",
            "strict-attribute-reject/entry.js", 1, 0, 0,
            &strict_attribute_error))
        return 1;
    passed++;
    if (run_rejection_scenario(
            "unicode-bare-reject",
            "import value from './data/unicode-bare.data' with { type: "
            "'json5' };\n",
            "unicode-bare-reject/entry.js", 2, 0, 0,
            &unicode_bare_error))
        return 1;
    passed++;
    if (run_rejection_scenario(
            "number-dot-reject",
            "import value from './data/number-dot.data' with { type: "
            "'json5' };\n",
            "number-dot-reject/entry.js", 2, 0, 0, &number_dot_error))
        return 1;
    passed++;
    if (run_rejection_scenario(
            "cr-continuation-reject",
            "import value from './data/cr-continuation.data' with { type: "
            "'json5' };\n",
            "cr-continuation-reject/entry.js", 2, 0, 0,
            &cr_continuation_error))
        return 1;
    passed++;
    if (run_rejection_scenario(
            "crlf-continuation-reject",
            "import value from './data/crlf-continuation.data' with { type: "
            "'json5' };\n",
            "crlf-continuation-reject/entry.js", 2, 0, 0,
            &crlf_continuation_error))
        return 1;
    passed++;
    printf("json5 loader oracle cases=%u passed=%u\n", passed, passed);
    return passed == 11 ? 0 : 1;
}

int main(void)
{
    Workspace workspace;
    int oracle_status;

    if (prepare_workspace(&workspace) < 0) {
        fputs("could not prepare JSON5 oracle workspace\n", stderr);
        (void)cleanup_workspace(&workspace);
        return 1;
    }
    oracle_status = run_oracle();
    if (cleanup_workspace(&workspace) < 0) {
        fputs("could not clean JSON5 oracle workspace\n", stderr);
        return 1;
    }
    return oracle_status;
}
