/*
 * QuickJS 2026-06-04 oracle for SharedArrayBuffer BC5 transport records.
 *
 * The BC5 format embeds process-local backing addresses. This probe validates
 * them against JS_WriteObject2's side table, but replaces every address byte
 * with zero before printing the wire. No raw address or address-derived digest
 * is exposed in the deterministic transcript.
 */

#include "quickjs.h"

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

enum {
    BC_VERSION = 5,
    BC_TAG_NULL = 1,
    BC_TAG_UNDEFINED = 2,
    BC_TAG_BOOL_FALSE = 3,
    BC_TAG_BOOL_TRUE = 4,
    BC_TAG_INT32 = 5,
    BC_TAG_FLOAT64 = 6,
    BC_TAG_STRING = 7,
    BC_TAG_OBJECT = 8,
    BC_TAG_ARRAY = 9,
    BC_TAG_TYPED_ARRAY = 14,
    BC_TAG_SHARED_ARRAY_BUFFER = 16,
    BC_TAG_OBJECT_REFERENCE = 19,
};

typedef struct SharedHeader {
    size_t references;
    size_t capacity;
    uint8_t bytes[];
} SharedHeader;

typedef struct SharedCallbacks {
    size_t allocations;
    size_t duplicates;
    size_t frees;
    size_t releases;
} SharedCallbacks;

typedef struct WireCursor {
    uint8_t *base;
    uint8_t *cursor;
    uint8_t *end;
    uint8_t **side_table;
    size_t side_table_length;
    size_t sab_records;
} WireCursor;

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
    header->capacity = size;
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

static int retain_message(SharedCallbacks *callbacks, TransportMessage *message,
                          const uint8_t *wire, size_t wire_size,
                          uint8_t *const *side_table,
                          size_t side_table_length)
{
    size_t index;

    memset(message, 0, sizeof(*message));
    message->wire = malloc(wire_size);
    message->side_table = malloc(sizeof(*message->side_table) *
                                 side_table_length);
    if (!message->wire || (side_table_length != 0 && !message->side_table)) {
        free(message->side_table);
        free(message->wire);
        memset(message, 0, sizeof(*message));
        return -1;
    }
    memcpy(message->wire, wire, wire_size);
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

static JSValue eval_value(JSContext *ctx, const char *source,
                          const char *filename)
{
    return JS_Eval(ctx, source, strlen(source), filename,
                   JS_EVAL_TYPE_GLOBAL);
}

static int cursor_take(WireCursor *input, size_t count, uint8_t **start)
{
    if ((size_t)(input->end - input->cursor) < count)
        return -1;
    if (start)
        *start = input->cursor;
    input->cursor += count;
    return 0;
}

static int cursor_u8(WireCursor *input, uint8_t *value)
{
    if (input->cursor == input->end)
        return -1;
    *value = *input->cursor++;
    return 0;
}

static int cursor_leb128(WireCursor *input, uint32_t *value)
{
    uint32_t result = 0;
    unsigned int shift = 0;

    while (shift < 35) {
        uint8_t byte;

        if (cursor_u8(input, &byte) < 0)
            return -1;
        if (shift == 28 && (byte & 0xf0) != 0)
            return -1;
        result |= (uint32_t)(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0) {
            *value = result;
            return 0;
        }
        shift += 7;
    }
    return -1;
}

static int cursor_string(WireCursor *input)
{
    uint32_t header;
    size_t size;

    if (cursor_leb128(input, &header) < 0)
        return -1;
    size = header >> 1;
    if ((header & 1) != 0) {
        if (size > SIZE_MAX / 2)
            return -1;
        size *= 2;
    }
    return cursor_take(input, size, NULL);
}

static uint64_t load_u64_le(const uint8_t *bytes)
{
    uint64_t value = 0;
    unsigned int index;

    for (index = 0; index < 8; index++)
        value |= (uint64_t)bytes[index] << (index * 8);
    return value;
}

static int normalize_value(WireCursor *input, unsigned int depth)
{
    uint8_t tag;
    uint32_t count;
    uint32_t index;

    if (depth > 32 || cursor_u8(input, &tag) < 0)
        return -1;
    switch (tag) {
    case BC_TAG_NULL:
    case BC_TAG_UNDEFINED:
    case BC_TAG_BOOL_FALSE:
    case BC_TAG_BOOL_TRUE:
        return 0;
    case BC_TAG_INT32:
    case BC_TAG_OBJECT_REFERENCE:
        return cursor_leb128(input, &count);
    case BC_TAG_FLOAT64:
        return cursor_take(input, 8, NULL);
    case BC_TAG_STRING:
        return cursor_string(input);
    case BC_TAG_OBJECT:
        if (cursor_leb128(input, &count) < 0)
            return -1;
        for (index = 0; index < count; index++) {
            uint32_t atom;

            if (cursor_leb128(input, &atom) < 0 ||
                normalize_value(input, depth + 1) < 0)
                return -1;
        }
        return 0;
    case BC_TAG_ARRAY:
        if (cursor_leb128(input, &count) < 0)
            return -1;
        for (index = 0; index < count; index++) {
            if (normalize_value(input, depth + 1) < 0)
                return -1;
        }
        return 0;
    case BC_TAG_TYPED_ARRAY:
        if (cursor_u8(input, &tag) < 0 ||
            cursor_leb128(input, &count) < 0 ||
            cursor_leb128(input, &count) < 0)
            return -1;
        return normalize_value(input, depth + 1);
    case BC_TAG_SHARED_ARRAY_BUFFER: {
        uint8_t *token;
        uint64_t encoded;

        if (cursor_leb128(input, &count) < 0 ||
            cursor_leb128(input, &count) < 0 ||
            cursor_take(input, 8, &token) < 0 ||
            input->sab_records >= input->side_table_length)
            return -1;
        encoded = load_u64_le(token);
        if (encoded !=
            (uint64_t)(uintptr_t)input->side_table[input->sab_records])
            return -1;
        memset(token, 0, 8);
        input->sab_records++;
        return 0;
    }
    default:
        return -1;
    }
}

static int normalize_wire(uint8_t *bytes, size_t size, uint8_t **side_table,
                          size_t side_table_length)
{
    WireCursor input = {
        .base = bytes,
        .cursor = bytes,
        .end = bytes + size,
        .side_table = side_table,
        .side_table_length = side_table_length,
        .sab_records = 0,
    };
    uint8_t version;
    uint32_t atom_count;
    uint32_t index;

    if (cursor_u8(&input, &version) < 0 || version != BC_VERSION ||
        cursor_leb128(&input, &atom_count) < 0)
        return -1;
    for (index = 0; index < atom_count; index++) {
        if (cursor_string(&input) < 0)
            return -1;
    }
    if (normalize_value(&input, 0) < 0 || input.cursor != input.end ||
        input.sab_records != side_table_length)
        return -1;
    return 0;
}

static void print_hex(const uint8_t *bytes, size_t size)
{
    size_t index;

    for (index = 0; index < size; index++)
        printf("%02x", bytes[index]);
}

static int value_to_i32(JSContext *ctx, JSValueConst value, int32_t *result)
{
    return JS_ToInt32(ctx, result, value);
}

static int check_view_bytes(JSContext *ctx, JSValueConst view,
                            const int32_t *expected, size_t length)
{
    size_t index;

    for (index = 0; index < length; index++) {
        JSValue value = JS_GetPropertyUint32(ctx, view, (uint32_t)index);
        int32_t actual;

        if (JS_IsException(value) || value_to_i32(ctx, value, &actual) < 0) {
            JS_FreeValue(ctx, value);
            return -1;
        }
        JS_FreeValue(ctx, value);
        if (actual != expected[index])
            return -1;
    }
    return 0;
}

static int run_references_on(void)
{
    static const char source[] =
        "(()=>{const s=new SharedArrayBuffer(4);"
        "const v=new Uint8Array(s);v.set([11,22,33,44]);"
        "return [v,s,s]})()";
    static const int32_t expected_bytes[] = {11, 22, 33, 44};
    SharedCallbacks callbacks = {0};
    CaseRuntime writer = {0};
    CaseRuntime reader = {0};
    TransportMessage message = {0};
    JSValue root = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue view = JS_UNDEFINED;
    JSValue first = JS_UNDEFINED;
    JSValue second = JS_UNDEFINED;
    JSValue view_buffer = JS_UNDEFINED;
    uint8_t *wire = NULL;
    uint8_t *redacted = NULL;
    uint8_t **side_table = NULL;
    size_t wire_size = 0;
    size_t side_table_length = 0;
    int status = -1;

    if (case_runtime_init(&writer, &callbacks) < 0)
        return -1;
    root = eval_value(writer.context, source, "sab-refs-on.js");
    if (JS_IsException(root)) {
        report_exception(writer.context, "refs-on setup failed");
        root = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject2(writer.context, &wire_size, root,
                           JS_WRITE_OBJ_SAB | JS_WRITE_OBJ_REFERENCE,
                           &side_table, &side_table_length);
    if (!wire) {
        report_exception(writer.context, "refs-on write failed");
        goto cleanup;
    }
    redacted = malloc(wire_size);
    if (!redacted) {
        fputs("refs-on redacted wire allocation failed\n", stderr);
        goto cleanup;
    }
    memcpy(redacted, wire, wire_size);
    if (side_table_length != 1 || normalize_wire(redacted, wire_size, side_table,
                                                 side_table_length) < 0) {
        fputs("refs-on wire/side-table contract mismatch\n", stderr);
        goto cleanup;
    }
    if (retain_message(&callbacks, &message, wire, wire_size, side_table,
                       side_table_length) < 0) {
        fputs("refs-on message retention failed\n", stderr);
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
        fputs("refs-on message did not outlive writer runtime\n", stderr);
        goto cleanup;
    }
    if (case_runtime_init(&reader, &callbacks) < 0)
        goto cleanup;
    loaded = JS_ReadObject(reader.context, message.wire, message.wire_size,
                           JS_READ_OBJ_SAB | JS_READ_OBJ_REFERENCE);
    if (JS_IsException(loaded)) {
        report_exception(reader.context, "refs-on read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    if (callbacks.duplicates != 2) {
        fputs("refs-on fresh read clone count mismatch\n", stderr);
        goto cleanup;
    }
    release_message(&callbacks, &message);
    view = JS_GetPropertyUint32(reader.context, loaded, 0);
    first = JS_GetPropertyUint32(reader.context, loaded, 1);
    second = JS_GetPropertyUint32(reader.context, loaded, 2);
    if (JS_IsException(view) || JS_IsException(first) ||
        JS_IsException(second)) {
        report_exception(reader.context, "refs-on property read failed");
        goto cleanup;
    }
    view_buffer = JS_GetTypedArrayBuffer(reader.context, view, NULL, NULL, NULL);
    if (JS_IsException(view_buffer)) {
        report_exception(reader.context, "refs-on typed-array backing failed");
        view_buffer = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_StrictEq(reader.context, first, second) ||
        !JS_StrictEq(reader.context, view_buffer, first) ||
        check_view_bytes(reader.context, view, expected_bytes,
                         sizeof(expected_bytes) / sizeof(expected_bytes[0])) < 0) {
        fputs("refs-on alias/value contract mismatch\n", stderr);
        goto cleanup;
    }

    printf("refs-on-wire-size=%zu\n", wire_size);
    fputs("refs-on-wire-redacted-hex=", stdout);
    print_hex(redacted, wire_size);
    putchar('\n');
    printf("refs-on-sab-records=%zu\n", side_table_length);
    puts("refs-on-side-order=typed-array-backing");
    puts("refs-on-fresh-runtime=true");
    puts("refs-on-message-retention=dup-each-occurrence-before-writer-release");
    puts("refs-on-message-release=before-decoded-release");
    puts("refs-on-view-backing-identity=true");
    puts("refs-on-duplicate-identity=true");
    puts("refs-on-bytes=11,22,33,44");
    status = 0;

cleanup:
    release_message(&callbacks, &message);
    if (reader.context) {
        JS_FreeValue(reader.context, view_buffer);
        JS_FreeValue(reader.context, second);
        JS_FreeValue(reader.context, first);
        JS_FreeValue(reader.context, view);
        JS_FreeValue(reader.context, loaded);
    }
    case_runtime_free(&reader);
    if (writer.context) {
        if (side_table)
            js_free(writer.context, side_table);
        if (wire)
            js_free(writer.context, wire);
        JS_FreeValue(writer.context, root);
    }
    case_runtime_free(&writer);
    free(redacted);
    if (status == 0 &&
        (callbacks.allocations != 1 || callbacks.duplicates != 2 ||
         callbacks.frees != 3 || callbacks.releases != 1)) {
        fputs("refs-on callback ownership contract mismatch\n", stderr);
        return -1;
    }
    if (status == 0)
        puts("refs-on-callbacks=alloc:1,dup:2,free:3,release:1");
    return status;
}

static int run_references_off(void)
{
    static const char source[] =
        "(()=>{const s=new SharedArrayBuffer(2);"
        "new Uint8Array(s).set([7,9]);return [s,s]})()";
    SharedCallbacks callbacks = {0};
    CaseRuntime writer = {0};
    CaseRuntime reader = {0};
    TransportMessage message = {0};
    JSValue root = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue first = JS_UNDEFINED;
    JSValue second = JS_UNDEFINED;
    uint8_t *wire = NULL;
    uint8_t *redacted = NULL;
    uint8_t **side_table = NULL;
    uint8_t *first_bytes;
    uint8_t *second_bytes;
    size_t first_size;
    size_t second_size;
    size_t wire_size = 0;
    size_t side_table_length = 0;
    int status = -1;

    if (case_runtime_init(&writer, &callbacks) < 0)
        return -1;
    root = eval_value(writer.context, source, "sab-refs-off.js");
    if (JS_IsException(root)) {
        report_exception(writer.context, "refs-off setup failed");
        root = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject2(writer.context, &wire_size, root, JS_WRITE_OBJ_SAB,
                           &side_table, &side_table_length);
    if (!wire) {
        report_exception(writer.context, "refs-off write failed");
        goto cleanup;
    }
    redacted = malloc(wire_size);
    if (!redacted) {
        fputs("refs-off redacted wire allocation failed\n", stderr);
        goto cleanup;
    }
    memcpy(redacted, wire, wire_size);
    if (side_table_length != 2 || side_table[0] != side_table[1] ||
        normalize_wire(redacted, wire_size, side_table, side_table_length) < 0) {
        fputs("refs-off wire/side-table contract mismatch\n", stderr);
        goto cleanup;
    }
    if (retain_message(&callbacks, &message, wire, wire_size, side_table,
                       side_table_length) < 0) {
        fputs("refs-off message retention failed\n", stderr);
        goto cleanup;
    }
    js_free(writer.context, side_table);
    side_table = NULL;
    js_free(writer.context, wire);
    wire = NULL;
    JS_FreeValue(writer.context, root);
    root = JS_UNDEFINED;
    case_runtime_free(&writer);
    if (callbacks.allocations != 1 || callbacks.duplicates != 2 ||
        callbacks.frees != 1 || callbacks.releases != 0) {
        fputs("refs-off message did not outlive writer runtime\n", stderr);
        goto cleanup;
    }
    if (case_runtime_init(&reader, &callbacks) < 0)
        goto cleanup;
    loaded = JS_ReadObject(reader.context, message.wire, message.wire_size,
                           JS_READ_OBJ_SAB);
    if (JS_IsException(loaded)) {
        report_exception(reader.context, "refs-off read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    if (callbacks.duplicates != 4) {
        fputs("refs-off fresh read clone count mismatch\n", stderr);
        goto cleanup;
    }
    release_message(&callbacks, &message);
    first = JS_GetPropertyUint32(reader.context, loaded, 0);
    second = JS_GetPropertyUint32(reader.context, loaded, 1);
    if (JS_IsException(first) || JS_IsException(second)) {
        report_exception(reader.context, "refs-off property read failed");
        goto cleanup;
    }
    first_bytes = JS_GetArrayBuffer(reader.context, &first_size, first);
    second_bytes = JS_GetArrayBuffer(reader.context, &second_size, second);
    if (!first_bytes || !second_bytes || first_size != 2 || second_size != 2 ||
        first_bytes != second_bytes || JS_StrictEq(reader.context, first, second) ||
        first_bytes[0] != 7 || first_bytes[1] != 9) {
        fputs("refs-off occurrence/alias contract mismatch\n", stderr);
        goto cleanup;
    }

    printf("refs-off-wire-size=%zu\n", wire_size);
    fputs("refs-off-wire-redacted-hex=", stdout);
    print_hex(redacted, wire_size);
    putchar('\n');
    printf("refs-off-sab-records=%zu\n", side_table_length);
    puts("refs-off-side-order=backing,backing");
    puts("refs-off-fresh-runtime=true");
    puts("refs-off-message-retention=dup-each-occurrence-before-writer-release");
    puts("refs-off-message-release=before-decoded-release");
    puts("refs-off-wrapper-identity=false");
    puts("refs-off-backing-identity=true");
    puts("refs-off-bytes=7,9");
    status = 0;

cleanup:
    release_message(&callbacks, &message);
    if (reader.context) {
        JS_FreeValue(reader.context, second);
        JS_FreeValue(reader.context, first);
        JS_FreeValue(reader.context, loaded);
    }
    case_runtime_free(&reader);
    if (writer.context) {
        if (side_table)
            js_free(writer.context, side_table);
        if (wire)
            js_free(writer.context, wire);
        JS_FreeValue(writer.context, root);
    }
    case_runtime_free(&writer);
    free(redacted);
    if (status == 0 &&
        (callbacks.allocations != 1 || callbacks.duplicates != 4 ||
         callbacks.frees != 5 || callbacks.releases != 1)) {
        fputs("refs-off callback ownership contract mismatch\n", stderr);
        return -1;
    }
    if (status == 0)
        puts("refs-off-callbacks=alloc:1,dup:4,free:5,release:1");
    return status;
}

static int expected_internal_error(JSContext *ctx)
{
    static const char expected_name[] = "InternalError";
    static const char expected_message[] =
        "resizable ArrayBuffers not supported for externally managed buffers";
    JSValue exception = JS_GetException(ctx);
    JSValue name_value = JS_GetPropertyStr(ctx, exception, "name");
    JSValue message_value = JS_GetPropertyStr(ctx, exception, "message");
    const char *name = NULL;
    const char *message = NULL;
    int matches = 0;

    if (!JS_IsException(name_value) && !JS_IsException(message_value)) {
        name = JS_ToCString(ctx, name_value);
        message = JS_ToCString(ctx, message_value);
        if (name && message && strcmp(name, expected_name) == 0 &&
            strcmp(message, expected_message) == 0)
            matches = 1;
    }
    if (message)
        JS_FreeCString(ctx, message);
    if (name)
        JS_FreeCString(ctx, name);
    JS_FreeValue(ctx, message_value);
    JS_FreeValue(ctx, name_value);
    JS_FreeValue(ctx, exception);
    return matches;
}

static int run_growable_asymmetry(void)
{
    static const char source[] =
        "new SharedArrayBuffer(2,{maxByteLength:8})";
    SharedCallbacks callbacks = {0};
    CaseRuntime writer = {0};
    CaseRuntime reader = {0};
    TransportMessage message = {0};
    JSValue root = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    uint8_t *wire = NULL;
    uint8_t *redacted = NULL;
    uint8_t **side_table = NULL;
    size_t wire_size = 0;
    size_t side_table_length = 0;
    int status = -1;

    if (case_runtime_init(&writer, &callbacks) < 0)
        return -1;
    root = eval_value(writer.context, source, "sab-growable.js");
    if (JS_IsException(root)) {
        report_exception(writer.context, "growable setup failed");
        root = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject2(writer.context, &wire_size, root, JS_WRITE_OBJ_SAB,
                           &side_table, &side_table_length);
    if (!wire) {
        report_exception(writer.context, "growable write failed");
        goto cleanup;
    }
    redacted = malloc(wire_size);
    if (!redacted) {
        fputs("growable redacted wire allocation failed\n", stderr);
        goto cleanup;
    }
    memcpy(redacted, wire, wire_size);
    if (side_table_length != 1 || normalize_wire(redacted, wire_size, side_table,
                                                 side_table_length) < 0) {
        fputs("growable wire/side-table contract mismatch\n", stderr);
        goto cleanup;
    }
    if (retain_message(&callbacks, &message, wire, wire_size, side_table,
                       side_table_length) < 0) {
        fputs("growable message retention failed\n", stderr);
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
        fputs("growable message did not outlive writer runtime\n", stderr);
        goto cleanup;
    }
    if (case_runtime_init(&reader, &callbacks) < 0)
        goto cleanup;
    loaded = JS_ReadObject(reader.context, message.wire, message.wire_size,
                           JS_READ_OBJ_SAB);
    if (!JS_IsException(loaded)) {
        fputs("growable read unexpectedly succeeded\n", stderr);
        goto cleanup;
    }
    loaded = JS_UNDEFINED;
    if (!expected_internal_error(reader.context)) {
        fputs("growable read diagnostic mismatch\n", stderr);
        goto cleanup;
    }
    if (callbacks.duplicates != 1) {
        fputs("growable failed read unexpectedly cloned backing\n", stderr);
        goto cleanup;
    }
    release_message(&callbacks, &message);

    printf("growable-wire-size=%zu\n", wire_size);
    fputs("growable-wire-redacted-hex=", stdout);
    print_hex(redacted, wire_size);
    putchar('\n');
    printf("growable-sab-records=%zu\n", side_table_length);
    puts("growable-fresh-runtime=true");
    puts("growable-message-retention=dup-each-occurrence-before-writer-release");
    puts("growable-read-dup-delta=0");
    puts("growable-message-release=after-failed-read");
    puts("growable-write=ok:length:2,maxByteLength:8");
    puts("growable-read=throw:InternalError:resizable ArrayBuffers not supported for externally managed buffers");
    status = 0;

cleanup:
    release_message(&callbacks, &message);
    if (reader.context)
        JS_FreeValue(reader.context, loaded);
    case_runtime_free(&reader);
    if (writer.context) {
        if (side_table)
            js_free(writer.context, side_table);
        if (wire)
            js_free(writer.context, wire);
        JS_FreeValue(writer.context, root);
    }
    case_runtime_free(&writer);
    free(redacted);
    if (status == 0 &&
        (callbacks.allocations != 1 || callbacks.duplicates != 1 ||
         callbacks.frees != 2 || callbacks.releases != 1)) {
        fputs("growable callback ownership contract mismatch\n", stderr);
        return -1;
    }
    if (status == 0)
        puts("growable-callbacks=alloc:1,dup:1,free:2,release:1");
    return status;
}

int main(void)
{
    puts("quickjs=2026-06-04");
    puts("bytecode-version=5");
    puts("pointer-output=redacted-zero-token");
    if (run_references_on() < 0 || run_references_off() < 0 ||
        run_growable_asymmetry() < 0)
        return 1;
    return 0;
}
