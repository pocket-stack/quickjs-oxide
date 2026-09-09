/*
 * QuickJS 2026-06-04 oracle for one whole-image FunctionBytecode/SAB graph.
 *
 * The native SharedArrayBuffer token is authenticated against JS_WriteObject2's
 * occurrence side table and then replaced with eight zero bytes before any wire
 * data is printed. No address, address-derived digest, or unredacted bytecode is
 * exposed by the deterministic transcript.
 */

#include "quickjs.h"

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_ARRAY = 9,
    BC_TAG_FUNCTION_BYTECODE = 12,
    BC_TAG_TYPED_ARRAY = 14,
    BC_TAG_SHARED_ARRAY_BUFFER = 16,
    BC_TAG_OBJECT_REFERENCE = 19,
    EXPECTED_WIRE_SIZE = 50,
    SAB_TOKEN_OFFSET = 38,
};

typedef struct SharedHeader {
    size_t references;
    uint8_t bytes[];
} SharedHeader;

typedef struct SharedCallbacks {
    size_t allocations;
    size_t duplicates;
    size_t frees;
    size_t releases;
} SharedCallbacks;

typedef struct CaseRuntime {
    JSRuntime *runtime;
    JSContext *context;
} CaseRuntime;

typedef struct TransportMessage {
    uint8_t *wire;
    size_t wire_size;
    uint8_t **side_table;
    size_t side_table_length;
} TransportMessage;

static const uint8_t expected_redacted_wire[EXPECTED_WIRE_SIZE] = {
    0x05, 0x00, 0x09, 0x04,
    0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
    0x0e, 0x02, 0x04, 0x00,
    0x10, 0x04, 0xff, 0xff, 0xff, 0xff, 0x0f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x13, 0x02, 0x13, 0x02,
};

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

static void *shared_alloc(void *opaque, size_t size)
{
    SharedCallbacks *callbacks = opaque;
    SharedHeader *header = malloc(sizeof(*header) + size);

    if (!header)
        return NULL;
    header->references = 1;
    callbacks->allocations++;
    return header->bytes;
}

static SharedHeader *shared_header(void *pointer)
{
    return (SharedHeader *)((uint8_t *)pointer - offsetof(SharedHeader, bytes));
}

static void shared_dup(void *opaque, void *pointer)
{
    SharedCallbacks *callbacks = opaque;
    SharedHeader *header = shared_header(pointer);

    header->references++;
    callbacks->duplicates++;
}

static void shared_free(void *opaque, void *pointer)
{
    SharedCallbacks *callbacks = opaque;
    SharedHeader *header = shared_header(pointer);

    callbacks->frees++;
    if (--header->references == 0) {
        callbacks->releases++;
        free(header);
    }
}

static int case_runtime_init(CaseRuntime *fixture, SharedCallbacks *callbacks)
{
    JSSharedArrayBufferFunctions functions;

    memset(fixture, 0, sizeof(*fixture));
    fixture->runtime = JS_NewRuntime();
    if (!fixture->runtime) {
        fputs("runtime allocation failed\n", stderr);
        return -1;
    }
    memset(&functions, 0, sizeof(functions));
    functions.sab_alloc = shared_alloc;
    functions.sab_free = shared_free;
    functions.sab_dup = shared_dup;
    functions.sab_opaque = callbacks;
    JS_SetSharedArrayBufferFunctions(fixture->runtime, &functions);
    fixture->context = JS_NewContext(fixture->runtime);
    if (!fixture->context) {
        fputs("context allocation failed\n", stderr);
        JS_FreeRuntime(fixture->runtime);
        fixture->runtime = NULL;
        return -1;
    }
    return 0;
}

static void case_runtime_free(CaseRuntime *fixture)
{
    if (fixture->context)
        JS_FreeContext(fixture->context);
    if (fixture->runtime)
        JS_FreeRuntime(fixture->runtime);
    fixture->context = NULL;
    fixture->runtime = NULL;
}

static JSValue eval_value(JSContext *ctx, const char *source,
                          const char *filename, int flags)
{
    return JS_Eval(ctx, source, strlen(source), filename, flags);
}

static int retain_message(SharedCallbacks *callbacks,
                          TransportMessage *message, const uint8_t *wire,
                          size_t wire_size, uint8_t *const *side_table,
                          size_t side_table_length)
{
    size_t index;

    memset(message, 0, sizeof(*message));
    message->wire = malloc(wire_size);
    if (!message->wire)
        return -1;
    if (side_table_length != 0) {
        message->side_table = malloc(sizeof(*message->side_table) *
                                     side_table_length);
        if (!message->side_table) {
            free(message->wire);
            memset(message, 0, sizeof(*message));
            return -1;
        }
    }
    memcpy(message->wire, wire, wire_size);
    if (side_table_length != 0)
        memcpy(message->side_table, side_table,
               sizeof(*message->side_table) * side_table_length);
    message->wire_size = wire_size;
    message->side_table_length = side_table_length;
    for (index = 0; index < side_table_length; index++)
        shared_dup(callbacks, message->side_table[index]);
    return 0;
}

static void release_message(SharedCallbacks *callbacks,
                            TransportMessage *message)
{
    size_t index;

    for (index = 0; index < message->side_table_length; index++)
        shared_free(callbacks, message->side_table[index]);
    free(message->side_table);
    free(message->wire);
    memset(message, 0, sizeof(*message));
}

static uint64_t load_u64_le(const uint8_t *bytes)
{
    uint64_t value = 0;
    unsigned int index;

    for (index = 0; index < 8; index++)
        value |= (uint64_t)bytes[index] << (index * 8);
    return value;
}

static int redact_and_validate_wire(uint8_t *redacted, const uint8_t *wire,
                                    size_t wire_size,
                                    uint8_t *const *side_table,
                                    size_t side_table_length)
{
    const size_t suffix_offset = SAB_TOKEN_OFFSET + 8;
    uint64_t token;

    if (wire_size != sizeof(expected_redacted_wire) ||
        side_table_length != 1 || !side_table || !side_table[0])
        return -1;
    if (memcmp(wire, expected_redacted_wire, SAB_TOKEN_OFFSET) != 0 ||
        memcmp(wire + suffix_offset, expected_redacted_wire + suffix_offset,
               wire_size - suffix_offset) != 0)
        return -1;
    token = load_u64_le(wire + SAB_TOKEN_OFFSET);
    if (token != (uint64_t)(uintptr_t)side_table[0])
        return -1;

    memcpy(redacted, wire, wire_size);
    memset(redacted + SAB_TOKEN_OFFSET, 0, 8);
    return memcmp(redacted, expected_redacted_wire, wire_size) == 0 ? 0 : -1;
}

static void print_hex(const uint8_t *bytes, size_t length)
{
    size_t index;

    for (index = 0; index < length; index++)
        printf("%02x", (unsigned)bytes[index]);
}

static int check_view_bytes(JSContext *ctx, JSValueConst view)
{
    static const int32_t expected[] = {11, 22, 33, 44};
    size_t index;

    for (index = 0; index < sizeof(expected) / sizeof(expected[0]); index++) {
        JSValue value = JS_GetPropertyUint32(ctx, view, (uint32_t)index);
        int32_t actual;

        if (JS_IsException(value) || JS_ToInt32(ctx, &actual, value) < 0) {
            JS_FreeValue(ctx, value);
            return -1;
        }
        JS_FreeValue(ctx, value);
        if (actual != expected[index])
            return -1;
    }
    return 0;
}

static int run_oracle(void)
{
    static const char function_source[] = "42;";
    static const char graph_source[] =
        "(()=>{const s=new SharedArrayBuffer(4);"
        "const v=new Uint8Array(s);v.set([11,22,33,44]);"
        "return [undefined,v,s,s]})()";
    SharedCallbacks callbacks = {0};
    CaseRuntime writer = {0};
    CaseRuntime reader = {0};
    TransportMessage message = {0};
    JSValue compiled = JS_UNDEFINED;
    JSValue root = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue function = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue view = JS_UNDEFINED;
    JSValue first = JS_UNDEFINED;
    JSValue second = JS_UNDEFINED;
    JSValue view_buffer = JS_UNDEFINED;
    uint8_t *wire = NULL;
    uint8_t *redacted = NULL;
    uint8_t **side_table = NULL;
    size_t wire_size = 0;
    size_t side_table_length = 0;
    int32_t evaluated = 0;
    int status = -1;

    if (case_runtime_init(&writer, &callbacks) < 0)
        return -1;
    JS_SetStripInfo(writer.runtime, JS_STRIP_DEBUG);
    compiled = eval_value(writer.context, function_source,
                          "function-bytecode-sab-reference.js",
                          JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(writer.context, "function compilation failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    root = eval_value(writer.context, graph_source,
                      "function-bytecode-sab-reference-graph.js",
                      JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(root)) {
        report_exception(writer.context, "whole-image graph setup failed");
        root = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_SetPropertyUint32(writer.context, root, 0, compiled) < 0) {
        compiled = JS_UNDEFINED; /* JS_SetPropertyUint32 consumes the value. */
        report_exception(writer.context, "compiled function installation failed");
        goto cleanup;
    }
    compiled = JS_UNDEFINED;

    wire = JS_WriteObject2(
        writer.context, &wire_size, root,
        JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_SAB | JS_WRITE_OBJ_REFERENCE,
        &side_table, &side_table_length);
    if (!wire) {
        report_exception(writer.context, "whole-image serialization failed");
        goto cleanup;
    }
    redacted = malloc(wire_size);
    if (!redacted) {
        fputs("redacted wire allocation failed\n", stderr);
        goto cleanup;
    }
    if (redact_and_validate_wire(redacted, wire, wire_size, side_table,
                                 side_table_length) < 0) {
        fputs("whole-image wire/side-table contract mismatch\n", stderr);
        goto cleanup;
    }
    if (retain_message(&callbacks, &message, wire, wire_size, side_table,
                       side_table_length) < 0) {
        fputs("whole-image message retention failed\n", stderr);
        goto cleanup;
    }
    js_free(writer.context, side_table);
    side_table = NULL;
    js_free(writer.context, wire);
    wire = NULL;
    JS_FreeValue(writer.context, root);
    root = JS_UNDEFINED;
    case_runtime_free(&writer);
    if (callbacks.allocations != 1 || callbacks.duplicates != 1 ||
        callbacks.frees != 1 || callbacks.releases != 0) {
        fputs("whole-image message did not outlive writer runtime\n", stderr);
        goto cleanup;
    }

    if (case_runtime_init(&reader, &callbacks) < 0)
        goto cleanup;
    loaded = JS_ReadObject(
        reader.context, message.wire, message.wire_size,
        JS_READ_OBJ_BYTECODE | JS_READ_OBJ_SAB | JS_READ_OBJ_REFERENCE);
    if (JS_IsException(loaded)) {
        report_exception(reader.context, "whole-image deserialization failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    if (callbacks.duplicates != 2) {
        fputs("fresh whole-image read clone count mismatch\n", stderr);
        goto cleanup;
    }
    release_message(&callbacks, &message);

    function = JS_GetPropertyUint32(reader.context, loaded, 0);
    view = JS_GetPropertyUint32(reader.context, loaded, 1);
    first = JS_GetPropertyUint32(reader.context, loaded, 2);
    second = JS_GetPropertyUint32(reader.context, loaded, 3);
    if (JS_IsException(function) || JS_IsException(view) ||
        JS_IsException(first) || JS_IsException(second)) {
        report_exception(reader.context, "whole-image property read failed");
        goto cleanup;
    }
    view_buffer = JS_GetTypedArrayBuffer(reader.context, view, NULL, NULL, NULL);
    if (JS_IsException(view_buffer)) {
        report_exception(reader.context, "whole-image TypedArray backing failed");
        view_buffer = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_StrictEq(reader.context, first, second) ||
        !JS_StrictEq(reader.context, view_buffer, first) ||
        check_view_bytes(reader.context, view) < 0) {
        fputs("whole-image alias/value contract mismatch\n", stderr);
        goto cleanup;
    }

    result = JS_EvalFunction(reader.context, function);
    function = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(reader.context, "fresh-runtime function evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_ToInt32(reader.context, &evaluated, result) < 0 || evaluated != 42) {
        fputs("fresh-runtime function did not evaluate to 42\n", stderr);
        goto cleanup;
    }

    printf("write-flags=%d\n", JS_WRITE_OBJ_BYTECODE | JS_WRITE_OBJ_SAB |
                                 JS_WRITE_OBJ_REFERENCE);
    printf("read-flags=%d\n", JS_READ_OBJ_BYTECODE | JS_READ_OBJ_SAB |
                                JS_READ_OBJ_REFERENCE);
    printf("wire-size=%zu\n", wire_size);
    fputs("wire-redacted-hex=", stdout);
    print_hex(redacted, wire_size);
    putchar('\n');
    puts("root=offset:2,reference-id:0,array-length:4");
    puts("function=offset:4,reference-id:none,bytecode-size:4,result:42");
    puts("typed-array=offset:27,reference-id:1,kind:uint8,length:4,byte-offset:0");
    puts("shared-array-buffer=offset:31,reference-id:2,byte-length:4,max-byte-length:none,token-offset:38");
    puts("object-references=offset:46,target:2;offset:48,target:2");
    printf("sab-records=%zu\n", side_table_length);
    puts("side-order=typed-array-backing");
    puts("fresh-runtime=true");
    puts("message-retention=dup-each-occurrence-before-writer-release");
    puts("message-release=before-decoded-release");
    puts("view-backing-identity=true");
    puts("duplicate-identity=true");
    puts("bytes=11,22,33,44");
    status = 0;

cleanup:
    release_message(&callbacks, &message);
    if (reader.context) {
        JS_FreeValue(reader.context, view_buffer);
        JS_FreeValue(reader.context, second);
        JS_FreeValue(reader.context, first);
        JS_FreeValue(reader.context, view);
        JS_FreeValue(reader.context, result);
        JS_FreeValue(reader.context, function);
        JS_FreeValue(reader.context, loaded);
    }
    case_runtime_free(&reader);
    if (writer.context) {
        if (side_table)
            js_free(writer.context, side_table);
        if (wire)
            js_free(writer.context, wire);
        JS_FreeValue(writer.context, root);
        JS_FreeValue(writer.context, compiled);
    }
    case_runtime_free(&writer);
    free(redacted);
    if (status == 0 &&
        (callbacks.allocations != 1 || callbacks.duplicates != 2 ||
         callbacks.frees != 3 || callbacks.releases != 1)) {
        fputs("whole-image callback ownership contract mismatch\n", stderr);
        return -1;
    }
    if (status == 0)
        puts("callbacks=alloc:1,dup:2,free:3,release:1");
    return status;
}

int main(void)
{
    puts("quickjs=2026-06-04");
    printf("bytecode-version=%d\n", BC_VERSION);
    puts("pointer-output=redacted-zero-token");
    return run_oracle() < 0 ? 1 : 0;
}
