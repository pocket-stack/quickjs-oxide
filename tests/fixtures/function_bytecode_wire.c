#include "quickjs.h"

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

_Static_assert(sizeof(double) == sizeof(uint64_t),
               "QuickJS Float64 oracle requires 64-bit double");
_Static_assert(sizeof(void *) == sizeof(uint64_t),
               "QuickJS Float64 oracle requires a 64-bit build");

static const uint8_t expected_bytecode[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
};

static const char ordinary_leaf_source[] =
    "(function(a,b){ var acc=0.5; var step=b; while(a>0){ "
    "if(a===2) acc=(acc+step)/1; else acc=(acc+1)/1; "
    "a=a-1; } return acc===5.5 ? 42 : 0; })";

static const uint8_t ordinary_leaf_bytecode[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb,
    0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x02, 0x02,
    0x02, 0x02, 0x00, 0x00, 0x02, 0x2e, 0x04, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xbd,
    0x00, 0xc7, 0xd0, 0xc8, 0xcf, 0xb3, 0xa3, 0xe8,
    0x1a, 0xcf, 0xb5, 0xa9, 0xe8, 0x09, 0xc3, 0xc4,
    0x9b, 0xb4, 0x99, 0xc7, 0xea, 0x07, 0xc3, 0xb4,
    0x9b, 0xb4, 0x99, 0xc7, 0xcf, 0xb4, 0x9c, 0xd3,
    0xea, 0xe3, 0xc3, 0xbd, 0x01, 0xa9, 0xe8, 0x04,
    0xbb, 0x2a, 0x28, 0xb3, 0x28, 0x06, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xe0, 0x3f, 0x06, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x16, 0x40,
};

_Static_assert(sizeof(ordinary_leaf_bytecode) == 119,
               "ordinary leaf oracle must retain its pinned 119-byte wire");

static const uint8_t scalar_prefix[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00,
};

static const uint8_t scalar_local[] = {
    0x01, 0x00, 0x00, 0x00, 0x00,
};

#define SCALAR_MAX_CODE_SIZE 16
#define SCALAR_FLOAT64_POOL_SIZE 9
#define SCALAR_MAX_WIRE_SIZE \
    (sizeof(scalar_prefix) + 2 + sizeof(scalar_local) + \
     SCALAR_MAX_CODE_SIZE + SCALAR_FLOAT64_POOL_SIZE)
#define BIGINT_CONSTANT_MAX_PAYLOAD_SIZE 17
#define BIGINT_CONSTANT_MAX_WIRE_SIZE \
    (sizeof(scalar_prefix) + 2 + sizeof(scalar_local) + \
     SCALAR_MAX_CODE_SIZE + 1 + 5 + BIGINT_CONSTANT_MAX_PAYLOAD_SIZE)

typedef enum ScalarValueKind {
    SCALAR_VALUE_NUMBER,
    SCALAR_VALUE_UNDEFINED,
    SCALAR_VALUE_NULL,
    SCALAR_VALUE_BOOLEAN,
    SCALAR_VALUE_BIGINT,
    SCALAR_VALUE_EMPTY_STRING,
    SCALAR_VALUE_FLOAT64,
} ScalarValueKind;

typedef struct ScalarExpectation {
    ScalarValueKind kind;
    double number;
    int32_t integer;
    uint64_t bits;
} ScalarExpectation;

#define EXPECT_NUMBER(value) \
    { .kind = SCALAR_VALUE_NUMBER, .number = (value) }
#define EXPECT_VALUE(value_kind, value) \
    { .kind = (value_kind), .integer = (value) }
#define EXPECT_FLOAT64(value) \
    { .kind = SCALAR_VALUE_FLOAT64, .bits = UINT64_C(value) }

typedef struct ScalarCase {
    const char *label;
    const char *source;
    ScalarExpectation expected;
    size_t code_size;
    uint8_t code[SCALAR_MAX_CODE_SIZE];
} ScalarCase;

static const ScalarCase canonical_scalar_integers[] = {
    { "canonical-short-minus1", "-1;", EXPECT_NUMBER(-1), 3,
      { 0xb2, 0xcb, 0x28 } },
    { "canonical-short-0", "0;", EXPECT_NUMBER(0), 3,
      { 0xb3, 0xcb, 0x28 } },
    { "canonical-short-1", "1;", EXPECT_NUMBER(1), 3,
      { 0xb4, 0xcb, 0x28 } },
    { "canonical-short-2", "2;", EXPECT_NUMBER(2), 3,
      { 0xb5, 0xcb, 0x28 } },
    { "canonical-short-3", "3;", EXPECT_NUMBER(3), 3,
      { 0xb6, 0xcb, 0x28 } },
    { "canonical-short-4", "4;", EXPECT_NUMBER(4), 3,
      { 0xb7, 0xcb, 0x28 } },
    { "canonical-short-5", "5;", EXPECT_NUMBER(5), 3,
      { 0xb8, 0xcb, 0x28 } },
    { "canonical-short-6", "6;", EXPECT_NUMBER(6), 3,
      { 0xb9, 0xcb, 0x28 } },
    { "canonical-short-7", "7;", EXPECT_NUMBER(7), 3,
      { 0xba, 0xcb, 0x28 } },
    { "canonical-i8-min", "-128;", EXPECT_NUMBER(-128), 4,
      { 0xbb, 0x80, 0xcb, 0x28 } },
    { "canonical-i8-below-short", "-2;", EXPECT_NUMBER(-2), 4,
      { 0xbb, 0xfe, 0xcb, 0x28 } },
    { "canonical-i8-above-short", "8;", EXPECT_NUMBER(8), 4,
      { 0xbb, 0x08, 0xcb, 0x28 } },
    { "canonical-i8-max", "127;", EXPECT_NUMBER(127), 4,
      { 0xbb, 0x7f, 0xcb, 0x28 } },
    { "canonical-i16-min", "-32768;", EXPECT_NUMBER(-32768), 5,
      { 0xbc, 0x00, 0x80, 0xcb, 0x28 } },
    { "canonical-i16-below-i8", "-129;", EXPECT_NUMBER(-129), 5,
      { 0xbc, 0x7f, 0xff, 0xcb, 0x28 } },
    { "canonical-i16-above-i8", "128;", EXPECT_NUMBER(128), 5,
      { 0xbc, 0x80, 0x00, 0xcb, 0x28 } },
    { "canonical-i16-max", "32767;", EXPECT_NUMBER(32767), 5,
      { 0xbc, 0xff, 0x7f, 0xcb, 0x28 } },
    { "canonical-i32-lowest-emitted", "-2147483647;",
      EXPECT_NUMBER(-2147483647.0), 7,
      { 0x01, 0x01, 0x00, 0x00, 0x80, 0xcb, 0x28 } },
    { "canonical-i32-below-i16", "-32769;", EXPECT_NUMBER(-32769), 7,
      { 0x01, 0xff, 0x7f, 0xff, 0xff, 0xcb, 0x28 } },
    { "canonical-i32-above-i16", "32768;", EXPECT_NUMBER(32768), 7,
      { 0x01, 0x00, 0x80, 0x00, 0x00, 0xcb, 0x28 } },
    { "canonical-i32-max", "2147483647;", EXPECT_NUMBER(2147483647.0), 7,
      { 0x01, 0xff, 0xff, 0xff, 0x7f, 0xcb, 0x28 } },
};

static const ScalarCase compatible_scalar_integers[] = {
    { "compatible-i8-one", NULL, EXPECT_NUMBER(1), 4,
      { 0xbb, 0x01, 0xcb, 0x28 } },
    { "compatible-i16-one", NULL, EXPECT_NUMBER(1), 5,
      { 0xbc, 0x01, 0x00, 0xcb, 0x28 } },
    { "compatible-i32-one", NULL, EXPECT_NUMBER(1), 7,
      { 0x01, 0x01, 0x00, 0x00, 0x00, 0xcb, 0x28 } },
    { "compatible-i32-min", NULL, EXPECT_NUMBER(-2147483648.0), 7,
      { 0x01, 0x00, 0x00, 0x00, 0x80, 0xcb, 0x28 } },
};

static const ScalarCase canonical_scalar_values[] = {
    { "canonical-undefined", "void 0;",
      EXPECT_VALUE(SCALAR_VALUE_UNDEFINED, 0), 3,
      { 0x06, 0xcb, 0x28 } },
    { "canonical-null", "null;", EXPECT_VALUE(SCALAR_VALUE_NULL, 0), 3,
      { 0x07, 0xcb, 0x28 } },
    { "canonical-false", "false;", EXPECT_VALUE(SCALAR_VALUE_BOOLEAN, 0), 3,
      { 0x09, 0xcb, 0x28 } },
    { "canonical-true", "true;", EXPECT_VALUE(SCALAR_VALUE_BOOLEAN, 1), 3,
      { 0x0a, 0xcb, 0x28 } },
    { "canonical-empty-string", "\"\";",
      EXPECT_VALUE(SCALAR_VALUE_EMPTY_STRING, 0), 3,
      { 0xbf, 0xcb, 0x28 } },
    { "canonical-bigint-0", "0n;", EXPECT_VALUE(SCALAR_VALUE_BIGINT, 0), 7,
      { 0xb0, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28 } },
    { "canonical-bigint-minus1", "-1n;",
      EXPECT_VALUE(SCALAR_VALUE_BIGINT, -1), 7,
      { 0xb0, 0xff, 0xff, 0xff, 0xff, 0xcb, 0x28 } },
    { "canonical-bigint-max", "2147483647n;",
      EXPECT_VALUE(SCALAR_VALUE_BIGINT, INT32_MAX), 7,
      { 0xb0, 0xff, 0xff, 0xff, 0x7f, 0xcb, 0x28 } },
    { "canonical-bigint-lowest-emitted", "-2147483647n;",
      EXPECT_VALUE(SCALAR_VALUE_BIGINT, -2147483647), 7,
      { 0xb0, 0x01, 0x00, 0x00, 0x80, 0xcb, 0x28 } },
};

static const ScalarCase compatible_scalar_values[] = {
    { "compatible-bigint-i32-min", NULL,
      EXPECT_VALUE(SCALAR_VALUE_BIGINT, INT32_MIN), 7,
      { 0xb0, 0x00, 0x00, 0x00, 0x80, 0xcb, 0x28 } },
};

static const ScalarCase canonical_scalar_float64[] = {
    { "canonical-float64-half", "0.5;",
      EXPECT_FLOAT64(0x3fe0000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "canonical-float64-i32-max-plus-one", "2147483648;",
      EXPECT_FLOAT64(0x41e0000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "canonical-float64-min-subnormal", "5e-324;",
      EXPECT_FLOAT64(0x0000000000000001), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "canonical-float64-max-finite", "1.7976931348623157e308;",
      EXPECT_FLOAT64(0x7fefffffffffffff), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "canonical-float64-positive-infinity", "1e309;",
      EXPECT_FLOAT64(0x7ff0000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
};

static const ScalarCase compatible_scalar_float64[] = {
    { "compatible-float64-wide-half", NULL,
      EXPECT_FLOAT64(0x3fe0000000000000), 7,
      { 0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-positive-zero", NULL,
      EXPECT_FLOAT64(0x0000000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-negative-zero", NULL,
      EXPECT_FLOAT64(0x8000000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-integral-42", NULL,
      EXPECT_FLOAT64(0x4045000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-positive-infinity", NULL,
      EXPECT_FLOAT64(0x7ff0000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-negative-infinity", NULL,
      EXPECT_FLOAT64(0xfff0000000000000), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-quiet-nan", NULL,
      EXPECT_FLOAT64(0x7ff8000000000042), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
    { "compatible-float64-signaling-nan", NULL,
      EXPECT_FLOAT64(0x7ff0000000000042), 4,
      { 0xbd, 0x00, 0xcb, 0x28 } },
};

#define STRING_SCALAR_MAX_WIRE_SIZE 256

typedef enum StringScalarKind {
    STRING_SCALAR_STRING,
    STRING_SCALAR_SYMBOL,
} StringScalarKind;

typedef struct ScalarWireEncoding {
    const uint8_t *atom_header;
    size_t atom_header_size;
    const uint8_t *code;
    size_t code_size;
    const uint8_t *pool;
    size_t pool_size;
    uint32_t pool_count;
} ScalarWireEncoding;

typedef struct StringScalarCase {
    const char *label;
    const char *source;
    const char *cohort;
    StringScalarKind expected_kind;
    int expected_tag;
    const uint16_t *expected_units;
    size_t expected_unit_count;
    ScalarWireEncoding input;
    /* A NULL atom header means the input is already the rewrite target. */
    ScalarWireEncoding canonical;
} StringScalarCase;

#define STRING_NO_POOL_ENCODING(header, bytecode) \
    { (header), sizeof(header), (bytecode), sizeof(bytecode), NULL, 0, 0 }
#define STRING_POOL_ENCODING(header, bytecode, constant) \
    { (header), sizeof(header), (bytecode), sizeof(bytecode), \
      (constant), sizeof(constant), 1 }
#define STRING_IDENTITY_REWRITE \
    { NULL, 0, NULL, 0, NULL, 0, 0 }

static const uint8_t string_header_none[] = { 0x00 };
static const uint8_t string_header_a[] = { 0x01, 0x02, 0x61 };
static const uint8_t string_header_length[] = {
    0x01, 0x0c, 0x6c, 0x65, 0x6e, 0x67, 0x74, 0x68,
};
static const uint8_t string_header_42[] = { 0x01, 0x04, 0x34, 0x32 };
static const uint8_t string_header_i31_plus_one[] = {
    0x01, 0x14, 0x32, 0x31, 0x34, 0x37, 0x34, 0x38,
    0x33, 0x36, 0x34, 0x38,
};
static const uint8_t string_header_leading_zero[] = {
    0x01, 0x04, 0x30, 0x31,
};
static const uint8_t string_header_nul[] = { 0x01, 0x02, 0x00 };
static const uint8_t string_header_a_nul_b[] = {
    0x01, 0x06, 0x61, 0x00, 0x62,
};
static const uint8_t string_header_latin1[] = { 0x01, 0x02, 0xe9 };
static const uint8_t string_header_wide_bmp[] = {
    0x01, 0x03, 0x00, 0x01,
};
static const uint8_t string_header_astral[] = {
    0x01, 0x05, 0x3d, 0xd8, 0x00, 0xde,
};
static const uint8_t string_header_lone_high[] = {
    0x01, 0x03, 0x00, 0xd8,
};
static const uint8_t string_header_lone_low[] = {
    0x01, 0x03, 0x00, 0xdc,
};
static const uint8_t string_header_symbol_description[] = {
    0x01, 0x24,
    0x53, 0x79, 0x6d, 0x62, 0x6f, 0x6c, 0x2e, 0x74, 0x6f,
    0x50, 0x72, 0x69, 0x6d, 0x69, 0x74, 0x69, 0x76, 0x65,
};
static const uint8_t string_header_wide_a[] = {
    0x01, 0x03, 0x61, 0x00,
};
static const uint8_t string_header_nonminimal_a[] = {
    0x81, 0x00, 0x82, 0x00, 0x61,
};

static const uint8_t string_code_push_empty[] = { 0xbf, 0xcb, 0x28 };
static const uint8_t string_code_push_atom_slot0[] = {
    0x04, 0xf3, 0x00, 0x00, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_atom_empty[] = {
    0x04, 0x2f, 0x00, 0x00, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_atom_length[] = {
    0x04, 0x32, 0x00, 0x00, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_atom_brand[] = {
    0x04, 0x7c, 0x00, 0x00, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_atom_tagged_42[] = {
    0x04, 0x2a, 0x00, 0x00, 0x80, 0xcb, 0x28,
};
static const uint8_t string_code_push_atom_private_brand[] = {
    0x04, 0xe5, 0x00, 0x00, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_atom_symbol[] = {
    0x04, 0xe6, 0x00, 0x00, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_const8[] = {
    0xbd, 0x00, 0xcb, 0x28,
};
static const uint8_t string_code_push_const[] = {
    0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28,
};

static const uint8_t string_pool_empty[] = { 0x07, 0x00 };
static const uint8_t string_pool_a[] = { 0x07, 0x02, 0x61 };
static const uint8_t string_pool_0[] = { 0x07, 0x02, 0x30 };
static const uint8_t string_pool_42[] = { 0x07, 0x04, 0x34, 0x32 };
static const uint8_t string_pool_i31_max[] = {
    0x07, 0x14, 0x32, 0x31, 0x34, 0x37, 0x34, 0x38,
    0x33, 0x36, 0x34, 0x37,
};
static const uint8_t string_pool_nul[] = { 0x07, 0x02, 0x00 };
static const uint8_t string_pool_latin1[] = { 0x07, 0x02, 0xe9 };
static const uint8_t string_pool_wide_bmp[] = {
    0x07, 0x03, 0x00, 0x01,
};
static const uint8_t string_pool_astral[] = {
    0x07, 0x05, 0x3d, 0xd8, 0x00, 0xde,
};
static const uint8_t string_pool_lone_high[] = {
    0x07, 0x03, 0x00, 0xd8,
};
static const uint8_t string_pool_wide_a[] = {
    0x07, 0x03, 0x61, 0x00,
};
static const uint8_t string_pool_nonminimal_a[] = {
    0x07, 0x82, 0x00, 0x61,
};

static const uint16_t string_units_empty[] = { 0 };
static const uint16_t string_units_a[] = { 0x0061 };
static const uint16_t string_units_length[] = {
    0x006c, 0x0065, 0x006e, 0x0067, 0x0074, 0x0068,
};
static const uint16_t string_units_0[] = { 0x0030 };
static const uint16_t string_units_42[] = { 0x0034, 0x0032 };
static const uint16_t string_units_i31_max[] = {
    0x0032, 0x0031, 0x0034, 0x0037, 0x0034,
    0x0038, 0x0033, 0x0036, 0x0034, 0x0037,
};
static const uint16_t string_units_i31_plus_one[] = {
    0x0032, 0x0031, 0x0034, 0x0037, 0x0034,
    0x0038, 0x0033, 0x0036, 0x0034, 0x0038,
};
static const uint16_t string_units_leading_zero[] = { 0x0030, 0x0031 };
static const uint16_t string_units_nul[] = { 0x0000 };
static const uint16_t string_units_a_nul_b[] = {
    0x0061, 0x0000, 0x0062,
};
static const uint16_t string_units_latin1[] = { 0x00e9 };
static const uint16_t string_units_wide_bmp[] = { 0x0100 };
static const uint16_t string_units_astral[] = { 0xd83d, 0xde00 };
static const uint16_t string_units_lone_high[] = { 0xd800 };
static const uint16_t string_units_lone_low[] = { 0xdc00 };
static const uint16_t string_units_brand[] = {
    0x003c, 0x0062, 0x0072, 0x0061, 0x006e, 0x0064, 0x003e,
};
static const uint16_t string_units_symbol_description[] = {
    0x0053, 0x0079, 0x006d, 0x0062, 0x006f, 0x006c,
    0x002e, 0x0074, 0x006f, 0x0050, 0x0072, 0x0069,
    0x006d, 0x0069, 0x0074, 0x0069, 0x0076, 0x0065,
};

#define STRING_CASE(label_value, source_value, cohort_value, units, count, encoding) \
    { (label_value), (source_value), (cohort_value), STRING_SCALAR_STRING, \
      JS_TAG_STRING, (units), (count), encoding, STRING_IDENTITY_REWRITE }

static const StringScalarCase canonical_string_scalars[] = {
    STRING_CASE("canonical-string-empty", "\"\";", "canonical",
                string_units_empty, 0,
                STRING_NO_POOL_ENCODING(string_header_none,
                                        string_code_push_empty)),
    STRING_CASE("canonical-string-predefined-length", "\"length\";",
                "canonical", string_units_length, 6,
                STRING_NO_POOL_ENCODING(string_header_none,
                                        string_code_push_atom_length)),
    STRING_CASE("canonical-string-dynamic-a", "\"a\";", "canonical",
                string_units_a, 1,
                STRING_NO_POOL_ENCODING(string_header_a,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-decimal-0", "\"0\";", "canonical",
                string_units_0, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8, string_pool_0)),
    STRING_CASE("canonical-string-decimal-42", "\"42\";", "canonical",
                string_units_42, 2,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8, string_pool_42)),
    STRING_CASE("canonical-string-decimal-i31-max", "\"2147483647\";",
                "canonical", string_units_i31_max, 10,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_i31_max)),
    STRING_CASE("canonical-string-dynamic-i31-plus-one", "\"2147483648\";",
                "canonical", string_units_i31_plus_one, 10,
                STRING_NO_POOL_ENCODING(string_header_i31_plus_one,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-dynamic-leading-zero", "\"01\";",
                "canonical", string_units_leading_zero, 2,
                STRING_NO_POOL_ENCODING(string_header_leading_zero,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-narrow-nul", "\"\\0\";", "canonical",
                string_units_nul, 1,
                STRING_NO_POOL_ENCODING(string_header_nul,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-narrow-a-nul-b", "\"a\\0b\";",
                "canonical", string_units_a_nul_b, 3,
                STRING_NO_POOL_ENCODING(string_header_a_nul_b,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-narrow-latin1", "\"\\u00e9\";",
                "canonical", string_units_latin1, 1,
                STRING_NO_POOL_ENCODING(string_header_latin1,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-wide-bmp", "\"\\u0100\";", "canonical",
                string_units_wide_bmp, 1,
                STRING_NO_POOL_ENCODING(string_header_wide_bmp,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-wide-astral", "\"\\u{1f600}\";",
                "canonical", string_units_astral, 2,
                STRING_NO_POOL_ENCODING(string_header_astral,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-wide-lone-high", "\"\\ud800\";",
                "canonical", string_units_lone_high, 1,
                STRING_NO_POOL_ENCODING(string_header_lone_high,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-wide-lone-low", "\"\\udc00\";",
                "canonical", string_units_lone_low, 1,
                STRING_NO_POOL_ENCODING(string_header_lone_low,
                                        string_code_push_atom_slot0)),
    STRING_CASE("canonical-string-ordinary-brand", "\"<brand>\";",
                "canonical", string_units_brand, 7,
                STRING_NO_POOL_ENCODING(string_header_none,
                                        string_code_push_atom_brand)),
    STRING_CASE("canonical-string-symbol-description",
                "\"Symbol.toPrimitive\";", "canonical",
                string_units_symbol_description, 18,
                STRING_NO_POOL_ENCODING(string_header_symbol_description,
                                        string_code_push_atom_slot0)),
};

static const StringScalarCase compatible_string_scalars[] = {
    STRING_CASE("compatible-string-atom-empty", NULL, "compatible-atom",
                string_units_empty, 0,
                STRING_NO_POOL_ENCODING(string_header_none,
                                        string_code_push_atom_empty)),
    STRING_CASE("compatible-string-atom-tagged-42", NULL,
                "compatible-atom", string_units_42, 2,
                STRING_NO_POOL_ENCODING(string_header_none,
                                        string_code_push_atom_tagged_42)),
    { "compatible-string-slot-predefined-length", NULL, "compatible-atom",
      STRING_SCALAR_STRING, JS_TAG_STRING, string_units_length, 6,
      STRING_NO_POOL_ENCODING(string_header_length,
                              string_code_push_atom_slot0),
      STRING_NO_POOL_ENCODING(string_header_none,
                              string_code_push_atom_length) },
    { "compatible-string-slot-tagged-42", NULL, "compatible-atom",
      STRING_SCALAR_STRING, JS_TAG_STRING, string_units_42, 2,
      STRING_NO_POOL_ENCODING(string_header_42,
                              string_code_push_atom_slot0),
      STRING_NO_POOL_ENCODING(string_header_none,
                              string_code_push_atom_tagged_42) },
    STRING_CASE("compatible-string-atom-wide-a", NULL, "compatible-atom",
                string_units_a, 1,
                STRING_NO_POOL_ENCODING(string_header_wide_a,
                                        string_code_push_atom_slot0)),
    { "compatible-string-atom-nonminimal-a", NULL, "compatible-atom",
      STRING_SCALAR_STRING, JS_TAG_STRING, string_units_a, 1,
      STRING_NO_POOL_ENCODING(string_header_nonminimal_a,
                              string_code_push_atom_slot0),
      STRING_NO_POOL_ENCODING(string_header_a,
                              string_code_push_atom_slot0) },
    STRING_CASE("compatible-string-cpool-empty", NULL, "compatible-cpool",
                string_units_empty, 0,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_empty)),
    STRING_CASE("compatible-string-cpool-a", NULL, "compatible-cpool",
                string_units_a, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8, string_pool_a)),
    STRING_CASE("compatible-string-cpool-42-wide-op", NULL,
                "compatible-cpool", string_units_42, 2,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const, string_pool_42)),
    STRING_CASE("compatible-string-cpool-nul", NULL, "compatible-cpool",
                string_units_nul, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_nul)),
    STRING_CASE("compatible-string-cpool-latin1", NULL, "compatible-cpool",
                string_units_latin1, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_latin1)),
    STRING_CASE("compatible-string-cpool-wide-bmp", NULL,
                "compatible-cpool", string_units_wide_bmp, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_wide_bmp)),
    STRING_CASE("compatible-string-cpool-astral", NULL, "compatible-cpool",
                string_units_astral, 2,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_astral)),
    STRING_CASE("compatible-string-cpool-lone-high", NULL,
                "compatible-cpool", string_units_lone_high, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_lone_high)),
    STRING_CASE("compatible-string-cpool-wide-a", NULL, "compatible-cpool",
                string_units_a, 1,
                STRING_POOL_ENCODING(string_header_none,
                                     string_code_push_const8,
                                     string_pool_wide_a)),
    { "compatible-string-cpool-nonminimal-a", NULL, "compatible-cpool",
      STRING_SCALAR_STRING, JS_TAG_STRING, string_units_a, 1,
      STRING_POOL_ENCODING(string_header_none, string_code_push_const8,
                           string_pool_nonminimal_a),
      STRING_POOL_ENCODING(string_header_none, string_code_push_const8,
                           string_pool_a) },
};

static const StringScalarCase outside_string_scalars[] = {
    { "outside-string-private-brand", NULL, "outside-symbol",
      STRING_SCALAR_SYMBOL, JS_TAG_SYMBOL, NULL, 0,
      STRING_NO_POOL_ENCODING(string_header_none,
                              string_code_push_atom_private_brand),
      STRING_IDENTITY_REWRITE },
    { "outside-string-well-known-symbol", NULL, "outside-symbol",
      STRING_SCALAR_SYMBOL, JS_TAG_SYMBOL, NULL, 0,
      STRING_NO_POOL_ENCODING(string_header_none,
                              string_code_push_atom_symbol),
      STRING_IDENTITY_REWRITE },
};

#undef STRING_CASE
#undef STRING_IDENTITY_REWRITE
#undef STRING_POOL_ENCODING
#undef STRING_NO_POOL_ENCODING

typedef enum BigIntConstantCohort {
    BIGINT_CONSTANT_THREE_INSTRUCTION,
    BIGINT_CONSTANT_UNARY_NEG,
    BIGINT_CONSTANT_DIRECT_UNARY_NEG,
    BIGINT_CONSTANT_UNARY_CHAIN,
} BigIntConstantCohort;

typedef struct BigIntConstantCase {
    const char *label;
    const char *source;
    const char *expected_decimal;
    int expected_tag;
    const uint8_t *code;
    size_t code_size;
    const uint8_t *payload;
    size_t payload_size;
    const uint8_t *canonical_payload;
    size_t canonical_payload_size;
    BigIntConstantCohort cohort;
} BigIntConstantCase;

static const uint8_t bigint_push_const8[] = {
    0xbd, 0x00, 0xcb, 0x28,
};

static const uint8_t bigint_push_const[] = {
    0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28,
};

static const uint8_t bigint_push_const8_neg[] = {
    0xbd, 0x00, 0x8a, 0xcb, 0x28,
};

static const uint8_t bigint_push_const_neg[] = {
    0x02, 0x00, 0x00, 0x00, 0x00, 0x8a, 0xcb, 0x28,
};

static const uint8_t bigint_push_bigint_i32_neg[] = {
    0xb0, 0x2a, 0x00, 0x00, 0x00, 0x8a, 0xcb, 0x28,
};

static const uint8_t bigint_push_const8_double_neg[] = {
    0xbd, 0x00, 0x8a, 0x8a, 0xcb, 0x28,
};

static const uint8_t bigint_i32_max_plus_one[] = {
    0x00, 0x00, 0x00, 0x80, 0x00,
};

static const uint8_t bigint_i32_max_plus_two[] = {
    0x01, 0x00, 0x00, 0x80, 0x00,
};

static const uint8_t bigint_u32_max[] = {
    0xff, 0xff, 0xff, 0xff, 0x00,
};

static const uint8_t bigint_i64_max[] = {
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
};

static const uint8_t bigint_i64_min[] = {
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,
};

static const uint8_t bigint_i64_max_plus_one[] = {
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00,
};

static const uint8_t bigint_two_to_128[] = {
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01,
};

static const uint8_t bigint_small_42[] = {
    0x2a,
};

static const uint8_t bigint_negative_i32_below_min[] = {
    0xff, 0xff, 0xff, 0x7f, 0xff,
};

static const uint8_t bigint_negative_i64_below_min[] = {
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff,
};

static const uint8_t bigint_one[] = {
    0x01,
};

static const uint8_t bigint_minus_one[] = {
    0xff,
};

static const uint8_t bigint_redundant_zero[] = {
    0x00,
};

static const uint8_t bigint_redundant_one[] = {
    0x01, 0x00,
};

static const uint8_t bigint_redundant_minus_one[] = {
    0xff, 0xff,
};

static const BigIntConstantCase bigint_constant_cases[] = {
    { "canonical-bigint-constant-i32-max-plus-one", "2147483648n;",
      "2147483648", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "canonical-bigint-constant-u32-max", "4294967295n;",
      "4294967295", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_u32_max, sizeof(bigint_u32_max),
      bigint_u32_max, sizeof(bigint_u32_max),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "canonical-bigint-constant-i64-max", "9223372036854775807n;",
      "9223372036854775807", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_i64_max, sizeof(bigint_i64_max),
      bigint_i64_max, sizeof(bigint_i64_max),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "canonical-bigint-constant-i64-max-plus-one", "9223372036854775808n;",
      "9223372036854775808", JS_TAG_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_i64_max_plus_one, sizeof(bigint_i64_max_plus_one),
      bigint_i64_max_plus_one, sizeof(bigint_i64_max_plus_one),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "canonical-bigint-constant-multilimb",
      "340282366920938463463374607431768211456n;",
      "340282366920938463463374607431768211456", JS_TAG_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_two_to_128, sizeof(bigint_two_to_128),
      bigint_two_to_128, sizeof(bigint_two_to_128),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-wide", NULL,
      "2147483648", JS_TAG_SHORT_BIG_INT,
      bigint_push_const, sizeof(bigint_push_const),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-zero", NULL,
      "0", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      NULL, 0, NULL, 0,
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-small", NULL,
      "42", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_small_42, sizeof(bigint_small_42),
      bigint_small_42, sizeof(bigint_small_42),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-negative", NULL,
      "-2147483649", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_negative_i32_below_min, sizeof(bigint_negative_i32_below_min),
      bigint_negative_i32_below_min, sizeof(bigint_negative_i32_below_min),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-negative-heap", NULL,
      "-9223372036854775809", JS_TAG_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_negative_i64_below_min, sizeof(bigint_negative_i64_below_min),
      bigint_negative_i64_below_min, sizeof(bigint_negative_i64_below_min),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-redundant-zero", NULL,
      "0", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_redundant_zero, sizeof(bigint_redundant_zero),
      NULL, 0,
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-redundant-positive", NULL,
      "1", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_redundant_one, sizeof(bigint_redundant_one),
      bigint_one, sizeof(bigint_one),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "compatible-bigint-constant-redundant-negative", NULL,
      "-1", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8, sizeof(bigint_push_const8),
      bigint_redundant_minus_one, sizeof(bigint_redundant_minus_one),
      bigint_minus_one, sizeof(bigint_minus_one),
      BIGINT_CONSTANT_THREE_INSTRUCTION },
    { "canonical-bigint-neg-i32-min", "-2147483648n;",
      "-2147483648", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      BIGINT_CONSTANT_UNARY_NEG },
    { "canonical-bigint-neg-i32-below-min", "-2147483649n;",
      "-2147483649", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_i32_max_plus_two, sizeof(bigint_i32_max_plus_two),
      bigint_i32_max_plus_two, sizeof(bigint_i32_max_plus_two),
      BIGINT_CONSTANT_UNARY_NEG },
    { "canonical-bigint-neg-i64-max", "-9223372036854775807n;",
      "-9223372036854775807", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_i64_max, sizeof(bigint_i64_max),
      bigint_i64_max, sizeof(bigint_i64_max),
      BIGINT_CONSTANT_UNARY_NEG },
    { "canonical-bigint-neg-i64-min", "-9223372036854775808n;",
      "-9223372036854775808", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_i64_max_plus_one, sizeof(bigint_i64_max_plus_one),
      bigint_i64_max_plus_one, sizeof(bigint_i64_max_plus_one),
      BIGINT_CONSTANT_UNARY_NEG },
    { "canonical-bigint-neg-multilimb",
      "-340282366920938463463374607431768211456n;",
      "-340282366920938463463374607431768211456", JS_TAG_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_two_to_128, sizeof(bigint_two_to_128),
      bigint_two_to_128, sizeof(bigint_two_to_128),
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-neg-short", NULL,
      "-42", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_small_42, sizeof(bigint_small_42),
      bigint_small_42, sizeof(bigint_small_42),
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-neg-wide", NULL,
      "-42", JS_TAG_SHORT_BIG_INT,
      bigint_push_const_neg, sizeof(bigint_push_const_neg),
      bigint_small_42, sizeof(bigint_small_42),
      bigint_small_42, sizeof(bigint_small_42),
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-neg-direct-i32", NULL,
      "-42", JS_TAG_SHORT_BIG_INT,
      bigint_push_bigint_i32_neg, sizeof(bigint_push_bigint_i32_neg),
      NULL, 0, NULL, 0,
      BIGINT_CONSTANT_DIRECT_UNARY_NEG },
    { "compatible-bigint-neg-negative-pool", NULL,
      "2147483649", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_negative_i32_below_min, sizeof(bigint_negative_i32_below_min),
      bigint_negative_i32_below_min, sizeof(bigint_negative_i32_below_min),
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-neg-i64-min-short-to-heap", NULL,
      "9223372036854775808", JS_TAG_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_i64_min, sizeof(bigint_i64_min),
      bigint_i64_min, sizeof(bigint_i64_min),
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-neg-zero-pool", NULL,
      "0", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      NULL, 0, NULL, 0,
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-neg-nonminimal-pool", NULL,
      "-1", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_neg, sizeof(bigint_push_const8_neg),
      bigint_redundant_one, sizeof(bigint_redundant_one),
      bigint_one, sizeof(bigint_one),
      BIGINT_CONSTANT_UNARY_NEG },
    { "compatible-bigint-double-neg-chain", NULL,
      "2147483648", JS_TAG_SHORT_BIG_INT,
      bigint_push_const8_double_neg, sizeof(bigint_push_const8_double_neg),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      bigint_i32_max_plus_one, sizeof(bigint_i32_max_plus_one),
      BIGINT_CONSTANT_UNARY_CHAIN },
};

typedef enum UnaryResultKind {
    UNARY_RESULT_INT,
    UNARY_RESULT_FLOAT64,
    UNARY_RESULT_BOOLEAN,
    UNARY_RESULT_BIGINT,
    UNARY_RESULT_STRING,
    UNARY_RESULT_EXCEPTION,
} UnaryResultKind;

typedef struct UnaryExpectation {
    UnaryResultKind kind;
    int tag;
    int64_t integer;
    uint64_t bits;
    const char *text;
    const char *exception_class;
} UnaryExpectation;

typedef struct UnaryCase {
    const char *label;
    const char *cohort;
    const char *ops;
    UnaryExpectation expected;
    ScalarWireEncoding input;
} UnaryCase;

static const uint8_t unary_pool_float64_42[] = {
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x45, 0x40,
};
static const uint8_t unary_pool_float64_41[] = {
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x44, 0x40,
};
static const uint8_t unary_pool_float64_43[] = {
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x45, 0x40,
};
static const uint8_t unary_pool_float64_negative_zero[] = {
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,
};
static const uint8_t unary_pool_float64_positive_nan[] = {
    0x06, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0x7f,
};
static const uint8_t unary_pool_float64_negative_nan[] = {
    0x06, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0xff,
};
static const uint8_t unary_pool_bigint_i64_max[] = {
    0x0a, 0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
};
static const uint8_t unary_pool_bigint_i64_min[] = {
    0x0a, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,
};

#define UNARY_NO_POOL(header, ...) \
    { (header), sizeof(header), (const uint8_t[]){ __VA_ARGS__ }, \
      sizeof((const uint8_t[]){ __VA_ARGS__ }), NULL, 0, 0 }
#define UNARY_POOL(header, pool_value, ...) \
    { (header), sizeof(header), (const uint8_t[]){ __VA_ARGS__ }, \
      sizeof((const uint8_t[]){ __VA_ARGS__ }), \
      (pool_value), sizeof(pool_value), 1 }
#define UNARY_INT_CASE(label_value, cohort_value, ops_value, value, encoding) \
    { (label_value), (cohort_value), (ops_value), \
      { UNARY_RESULT_INT, JS_TAG_INT, (value), 0, NULL, NULL }, encoding }
#define UNARY_FLOAT_CASE(label_value, cohort_value, ops_value, value, encoding) \
    { (label_value), (cohort_value), (ops_value), \
      { UNARY_RESULT_FLOAT64, JS_TAG_FLOAT64, 0, UINT64_C(value), NULL, NULL }, \
      encoding }
#define UNARY_BOOL_CASE(label_value, cohort_value, ops_value, value, encoding) \
    { (label_value), (cohort_value), (ops_value), \
      { UNARY_RESULT_BOOLEAN, JS_TAG_BOOL, (value), 0, NULL, NULL }, encoding }
#define UNARY_BIGINT_CASE(label_value, cohort_value, ops_value, tag_value, \
                          text_value, encoding) \
    { (label_value), (cohort_value), (ops_value), \
      { UNARY_RESULT_BIGINT, (tag_value), 0, 0, (text_value), NULL }, encoding }
#define UNARY_STRING_CASE(label_value, cohort_value, ops_value, text_value, \
                          encoding) \
    { (label_value), (cohort_value), (ops_value), \
      { UNARY_RESULT_STRING, JS_TAG_STRING, 0, 0, (text_value), NULL }, encoding }
#define UNARY_EXCEPTION_CASE(label_value, cohort_value, ops_value, class_value, \
                             message_value, encoding) \
    { (label_value), (cohort_value), (ops_value), \
      { UNARY_RESULT_EXCEPTION, 0, 0, 0, (message_value), (class_value) }, \
      encoding }

static const UnaryCase unary_cases[] = {
    UNARY_FLOAT_CASE(
        "unary-number-neg-int-zero", "number", "8a", 0x8000000000000000,
        UNARY_NO_POOL(string_header_none, 0xb3, 0x8a, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-neg-int-min-promotion", "number", "8a",
        0x41e0000000000000,
        UNARY_NO_POOL(string_header_none, 0x01, 0x00, 0x00, 0x00, 0x80,
                      0x8a, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-inc-int-max-promotion", "number", "8d",
        0x41e0000000000000,
        UNARY_NO_POOL(string_header_none, 0x01, 0xff, 0xff, 0xff, 0x7f,
                      0x8d, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-dec-int-min-promotion", "number", "8c",
        0xc1e0000000200000,
        UNARY_NO_POOL(string_header_none, 0x01, 0x00, 0x00, 0x00, 0x80,
                      0x8c, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-plus-integral-float", "number", "8b",
        0x4045000000000000,
        UNARY_POOL(string_header_none, unary_pool_float64_42,
                   0xbd, 0x00, 0x8b, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-plus-negative-zero", "number", "8b",
        0x8000000000000000,
        UNARY_POOL(string_header_none, unary_pool_float64_negative_zero,
                   0xbd, 0x00, 0x8b, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-neg-nan-payload-sign", "number", "8a",
        0xfff8000000000042,
        UNARY_POOL(string_header_none, unary_pool_float64_positive_nan,
                   0xbd, 0x00, 0x8a, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-plus-nan-payload-sign", "number", "8b",
        0xfff8000000000042,
        UNARY_POOL(string_header_none, unary_pool_float64_negative_nan,
                   0xbd, 0x00, 0x8b, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-inc-integral-float", "number", "8d",
        0x4045000000000000,
        UNARY_POOL(string_header_none, unary_pool_float64_41,
                   0xbd, 0x00, 0x8d, 0xcb, 0x28)),
    UNARY_FLOAT_CASE(
        "unary-number-dec-integral-float", "number", "8c",
        0x4045000000000000,
        UNARY_POOL(string_header_none, unary_pool_float64_43,
                   0xbd, 0x00, 0x8c, 0xcb, 0x28)),
    UNARY_INT_CASE(
        "unary-number-bitnot-float-nan-to-int32", "number", "93", -1,
        UNARY_POOL(string_header_none, unary_pool_float64_positive_nan,
                   0xbd, 0x00, 0x93, 0xcb, 0x28)),
    UNARY_INT_CASE(
        "unary-string-plus-decimal-to-int", "string-tonumeric", "8b", 42,
        UNARY_POOL(string_header_none, string_pool_42,
                   0xbd, 0x00, 0x8b, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-neg-short", "bigint", "8a", JS_TAG_SHORT_BIG_INT,
        "-42", UNARY_NO_POOL(string_header_none, 0xb0, 0x2a, 0x00, 0x00,
                              0x00, 0x8a, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-neg-i64-min-promotion", "bigint", "8a",
        JS_TAG_BIG_INT, "9223372036854775808",
        UNARY_POOL(string_header_none, unary_pool_bigint_i64_min,
                   0xbd, 0x00, 0x8a, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-inc-i64-max-promotion", "bigint", "8d",
        JS_TAG_BIG_INT, "9223372036854775808",
        UNARY_POOL(string_header_none, unary_pool_bigint_i64_max,
                   0xbd, 0x00, 0x8d, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-dec-short", "bigint", "8c",
        JS_TAG_SHORT_BIG_INT, "41",
        UNARY_NO_POOL(string_header_none, 0xb0, 0x2a, 0x00, 0x00, 0x00,
                      0x8c, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-dec-i64-min-pinned-unsigned-opcode-quirk",
        "bigint-pinned-quirk", "8c", JS_TAG_SHORT_BIG_INT,
        "-9223372032559808513",
        UNARY_POOL(string_header_none, unary_pool_bigint_i64_min,
                   0xbd, 0x00, 0x8c, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-bitnot-short", "bigint", "93",
        JS_TAG_SHORT_BIG_INT, "-43",
        UNARY_NO_POOL(string_header_none, 0xb0, 0x2a, 0x00, 0x00, 0x00,
                      0x93, 0xcb, 0x28)),
    UNARY_EXCEPTION_CASE(
        "unary-bigint-plus-type-error", "bigint", "8b", "TypeError",
        "bigint argument with unary +",
        UNARY_NO_POOL(string_header_none, 0xb0, 0x2a, 0x00, 0x00, 0x00,
                      0x8b, 0xcb, 0x28)),
    UNARY_BIGINT_CASE(
        "unary-bigint-mixed-inc-neg-bitnot", "mixed-chain", "8d,8a,93",
        JS_TAG_SHORT_BIG_INT, "41",
        UNARY_NO_POOL(string_header_none, 0xb0, 0x29, 0x00, 0x00, 0x00,
                      0x8d, 0x8a, 0x93, 0xcb, 0x28)),
    UNARY_INT_CASE(
        "unary-number-mixed-neg-lnot-plus", "mixed-chain", "8a,94,8b", 1,
        UNARY_NO_POOL(string_header_none, 0xb3, 0x8a, 0x94, 0x8b,
                      0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-undefined", "truthiness", "94", 1,
        UNARY_NO_POOL(string_header_none, 0x06, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-null", "truthiness", "94", 1,
        UNARY_NO_POOL(string_header_none, 0x07, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-false", "truthiness", "94", 1,
        UNARY_NO_POOL(string_header_none, 0x09, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-true", "truthiness", "94", 0,
        UNARY_NO_POOL(string_header_none, 0x0a, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-int-zero", "truthiness", "94", 1,
        UNARY_NO_POOL(string_header_none, 0xb3, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-int-nonzero", "truthiness", "94", 0,
        UNARY_NO_POOL(string_header_none, 0xbb, 0x2a, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-float-negative-zero", "truthiness", "94", 1,
        UNARY_POOL(string_header_none, unary_pool_float64_negative_zero,
                   0xbd, 0x00, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-float-nan", "truthiness", "94", 1,
        UNARY_POOL(string_header_none, unary_pool_float64_positive_nan,
                   0xbd, 0x00, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-bigint-zero", "truthiness", "94", 1,
        UNARY_NO_POOL(string_header_none, 0xb0, 0x00, 0x00, 0x00, 0x00,
                      0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-bigint-nonzero", "truthiness", "94", 0,
        UNARY_NO_POOL(string_header_none, 0xb0, 0x01, 0x00, 0x00, 0x00,
                      0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-empty-string", "truthiness", "94", 1,
        UNARY_NO_POOL(string_header_none, 0xbf, 0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-nonempty-string", "truthiness", "94", 0,
        UNARY_NO_POOL(string_header_a, 0x04, 0xf3, 0x00, 0x00, 0x00,
                      0x94, 0xcb, 0x28)),
    UNARY_BOOL_CASE(
        "unary-lnot-symbol", "outside-symbol-atom", "94", 0,
        UNARY_NO_POOL(string_header_none, 0x04, 0xe6, 0x00, 0x00, 0x00,
                      0x94, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-undefined", "typeof", "95", "undefined",
        UNARY_NO_POOL(string_header_none, 0x06, 0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-null", "typeof", "95", "object",
        UNARY_NO_POOL(string_header_none, 0x07, 0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-boolean", "typeof", "95", "boolean",
        UNARY_NO_POOL(string_header_none, 0x0a, 0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-int-number", "typeof", "95", "number",
        UNARY_NO_POOL(string_header_none, 0xbb, 0x2a, 0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-float-number", "typeof", "95", "number",
        UNARY_POOL(string_header_none, unary_pool_float64_42,
                   0xbd, 0x00, 0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-bigint", "typeof", "95", "bigint",
        UNARY_NO_POOL(string_header_none, 0xb0, 0x2a, 0x00, 0x00, 0x00,
                      0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-string", "typeof", "95", "string",
        UNARY_NO_POOL(string_header_a, 0x04, 0xf3, 0x00, 0x00, 0x00,
                      0x95, 0xcb, 0x28)),
    UNARY_STRING_CASE(
        "unary-typeof-symbol", "outside-symbol-atom", "95", "symbol",
        UNARY_NO_POOL(string_header_none, 0x04, 0xe6, 0x00, 0x00, 0x00,
                      0x95, 0xcb, 0x28)),
};

static const ScalarWireEncoding unary_typeof_number_atom_literal =
    UNARY_NO_POOL(string_header_none, 0x04, 0x4a, 0x00, 0x00, 0x00,
                  0xcb, 0x28);

#undef UNARY_EXCEPTION_CASE
#undef UNARY_STRING_CASE
#undef UNARY_BIGINT_CASE
#undef UNARY_BOOL_CASE
#undef UNARY_FLOAT_CASE
#undef UNARY_INT_CASE
#undef UNARY_POOL
#undef UNARY_NO_POOL

static const uint8_t compatible_scope_next_wrap[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x80, 0x80, 0x80, 0x80, 0x08, 0x00, 0x00, 0xbb, 0x2a,
    0xcb, 0x28,
};

static const uint8_t array_buffer_max_below_length[] = {
    0x05, 0x00, 0x0f, 0x01, 0x00,
};

static const uint8_t invalid_typed_array_kind[] = {
    0x05, 0x00, 0x0e, 0xff,
};

static const uint8_t malformed_uleb[] = {
    0x05, 0x80, 0x80, 0x80, 0x80, 0x80,
};

static const uint8_t invalid_metadata_atom[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xe6, 0x03, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
};

static const uint8_t hard_string_length[] = {
    0x05, 0x01, 0x80, 0x80, 0x80, 0x80, 0x08,
};

static const uint8_t bytecode_length_overflow[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x80, 0x80,
    0x80, 0x08, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a,
    0xcb, 0x28,
};

static const uint8_t uint16_array_unaligned_offset[] = {
    0x05, 0x00, 0x0e, 0x04, 0x00, 0x01, 0x0f, 0x00, 0x00,
};

static const uint8_t typed_array_view_beyond_backing[] = {
    0x05, 0x00, 0x0e, 0x02, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x00,
};

static void report_exception(JSContext *ctx, const char *operation) {
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

static int expect_exception_fields(JSContext *context,
                                   const char *label,
                                   JSValueConst exception,
                                   const char *expected_class,
                                   const char *expected_message) {
    JSValue class_value = JS_UNDEFINED;
    JSValue message_value = JS_UNDEFINED;
    const char *actual_class = NULL;
    const char *actual_message = NULL;
    int status = -1;

    if (!JS_IsError(context, exception)) {
        fprintf(stderr, "%s did not throw an Error object\n", label);
        goto cleanup;
    }
    class_value = JS_GetPropertyStr(context, exception, "name");
    message_value = JS_GetPropertyStr(context, exception, "message");
    if (JS_IsException(class_value) || JS_IsException(message_value)) {
        report_exception(context, "exception inspection failed");
        goto cleanup;
    }
    actual_class = JS_ToCString(context, class_value);
    actual_message = JS_ToCString(context, message_value);
    if (!actual_class || !actual_message) {
        report_exception(context, "exception conversion failed");
        goto cleanup;
    }
    if (strcmp(actual_class, expected_class) != 0 ||
        strcmp(actual_message, expected_message) != 0) {
        fprintf(stderr, "%s returned %s: %s, expected %s: %s\n",
                label, actual_class, actual_message,
                expected_class, expected_message);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (actual_message)
        JS_FreeCString(context, actual_message);
    if (actual_class)
        JS_FreeCString(context, actual_class);
    JS_FreeValue(context, message_value);
    JS_FreeValue(context, class_value);
    return status;
}

static int expect_ordinary_leaf(void) {
    enum {
        ROOT_CPOOL_COUNT_OFFSET = 14,
        ROOT_CODE_OFFSET = 21,
        CHILD_OFFSET = 25,
        CHILD_FLAGS_OFFSET = 26,
        CHILD_JS_MODE_OFFSET = 28,
        CHILD_ARG_COUNT_OFFSET = 30,
        CHILD_VAR_COUNT_OFFSET = 31,
        CHILD_DEFINED_ARG_COUNT_OFFSET = 32,
        CHILD_STACK_SIZE_OFFSET = 33,
        CHILD_VAR_REF_COUNT_OFFSET = 34,
        CHILD_CLOSURE_COUNT_OFFSET = 35,
        CHILD_CPOOL_COUNT_OFFSET = 36,
        CHILD_CODE_SIZE_OFFSET = 37,
        CHILD_LOCAL_COUNT_OFFSET = 38,
        CHILD_CODE_OFFSET = 55,
        CHILD_CODE_SIZE = 46,
        CHILD_POOL_OFFSET = 101,
        CHILD_INSTRUCTION_COUNT = 38,
    };
    static const uint8_t expected_root_code[] = {
        0xbe, 0x00, 0xcb, 0x28,
    };
    static const uint8_t expected_child_code[] = {
        0xbd, 0x00, 0xc7, 0xd0, 0xc8, 0xcf, 0xb3, 0xa3,
        0xe8, 0x1a, 0xcf, 0xb5, 0xa9, 0xe8, 0x09, 0xc3,
        0xc4, 0x9b, 0xb4, 0x99, 0xc7, 0xea, 0x07, 0xc3,
        0xb4, 0x9b, 0xb4, 0x99, 0xc7, 0xcf, 0xb4, 0x9c,
        0xd3, 0xea, 0xe3, 0xc3, 0xbd, 0x01, 0xa9, 0xe8,
        0x04, 0xbb, 0x2a, 0x28, 0xb3, 0x28,
    };
    static const uint8_t expected_child_pool[] = {
        0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe0, 0x3f,
        0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x16, 0x40,
    };
    static const uint8_t expected_instruction_pcs[] = {
        0, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 15,
        16, 17, 18, 19, 20, 21, 23, 24, 25, 26, 27, 28,
        29, 30, 31, 32, 33, 35, 36, 38, 39, 41, 43, 44, 45,
    };
    static const struct {
        uint8_t opcode_pc;
        uint8_t operand_pc;
        uint8_t opcode;
        int8_t displacement;
        uint8_t target_pc;
        uint8_t target_ir;
    } expected_branches[] = {
        { 8, 9, 0xe8, 26, 35, 30 },
        { 13, 14, 0xe8, 9, 23, 19 },
        { 21, 22, 0xea, 7, 29, 25 },
        { 33, 34, 0xea, -29, 5, 4 },
        { 39, 40, 0xe8, 4, 44, 36 },
    };
    _Static_assert(sizeof(expected_instruction_pcs) ==
                       CHILD_INSTRUCTION_COUNT,
                   "ordinary leaf instruction map must stay complete");
    JSRuntime *compile_runtime = NULL;
    JSContext *compile_context = NULL;
    JSRuntime *eval_runtime = NULL;
    JSContext *eval_context = NULL;
    JSRuntime *strict_runtime = NULL;
    JSContext *strict_context = NULL;
    JSValue compiled = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue function = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    JSValue length_value = JS_UNDEFINED;
    JSValue name_value = JS_UNDEFINED;
    JSValue prototype_value = JS_UNDEFINED;
    JSValue constructor_value = JS_UNDEFINED;
    JSValue instance = JS_UNDEFINED;
    JSValue instance_prototype = JS_UNDEFINED;
    JSValue caller_value = JS_UNDEFINED;
    JSValue arguments_value = JS_UNDEFINED;
    JSValue strict_loaded = JS_UNDEFINED;
    JSValue strict_function = JS_UNDEFINED;
    JSValue strict_property = JS_UNDEFINED;
    JSValue strict_exception = JS_UNDEFINED;
    JSValue arguments[2] = { JS_UNDEFINED, JS_UNDEFINED };
    uint8_t *bytecode = NULL;
    uint8_t *rewritten = NULL;
    uint8_t *function_wire = NULL;
    uint8_t *strict_rewritten = NULL;
    uint8_t strict_bytecode[sizeof(ordinary_leaf_bytecode)];
    size_t bytecode_size = 0;
    size_t rewritten_size = 0;
    size_t function_wire_size = 0;
    size_t strict_rewritten_size = 0;
    size_t name_length = 0;
    const char *name_string = NULL;
    JSAtom length_atom = JS_ATOM_NULL;
    JSAtom name_atom = JS_ATOM_NULL;
    JSAtom prototype_atom = JS_ATOM_NULL;
    JSAtom constructor_atom = JS_ATOM_NULL;
    uint16_t child_flags;
    int status = -1;

    compile_runtime = JS_NewRuntime();
    if (!compile_runtime) {
        fputs("ordinary leaf compile runtime allocation failed\n", stderr);
        goto cleanup;
    }
    JS_SetStripInfo(compile_runtime, JS_STRIP_DEBUG);
    compile_context = JS_NewContext(compile_runtime);
    if (!compile_context) {
        fputs("ordinary leaf compile context allocation failed\n", stderr);
        goto cleanup;
    }
    compiled = JS_Eval(compile_context, ordinary_leaf_source,
                       strlen(ordinary_leaf_source), "x",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "ordinary leaf compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    bytecode = JS_WriteObject(compile_context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(compile_context,
                         "ordinary leaf bytecode serialization failed");
        goto cleanup;
    }
    if (bytecode_size != sizeof(ordinary_leaf_bytecode) ||
        memcmp(bytecode, ordinary_leaf_bytecode,
               sizeof(ordinary_leaf_bytecode)) != 0) {
        fputs("ordinary leaf bytecode did not match its pinned wire\n", stderr);
        goto cleanup;
    }

    child_flags = (uint16_t)bytecode[CHILD_FLAGS_OFFSET] |
                  ((uint16_t)bytecode[CHILD_FLAGS_OFFSET + 1] << 8);
    if (bytecode[ROOT_CPOOL_COUNT_OFFSET] != 1 ||
        bytecode[CHILD_OFFSET] != 0x0c || child_flags != 0x0243 ||
        bytecode[CHILD_JS_MODE_OFFSET] != 0 ||
        bytecode[CHILD_ARG_COUNT_OFFSET] != 2 ||
        bytecode[CHILD_VAR_COUNT_OFFSET] != 2 ||
        bytecode[CHILD_DEFINED_ARG_COUNT_OFFSET] != 2 ||
        bytecode[CHILD_STACK_SIZE_OFFSET] != 2 ||
        bytecode[CHILD_VAR_REF_COUNT_OFFSET] != 0 ||
        bytecode[CHILD_CLOSURE_COUNT_OFFSET] != 0 ||
        bytecode[CHILD_CPOOL_COUNT_OFFSET] != 2 ||
        bytecode[CHILD_CODE_SIZE_OFFSET] != CHILD_CODE_SIZE ||
        bytecode[CHILD_LOCAL_COUNT_OFFSET] != 4 ||
        memcmp(bytecode + ROOT_CODE_OFFSET, expected_root_code,
               sizeof(expected_root_code)) != 0 ||
        memcmp(bytecode + CHILD_CODE_OFFSET, expected_child_code,
               sizeof(expected_child_code)) != 0 ||
        memcmp(bytecode + CHILD_POOL_OFFSET, expected_child_pool,
               sizeof(expected_child_pool)) != 0) {
        fputs("ordinary leaf metadata or branch targets drifted\n", stderr);
        goto cleanup;
    }
    for (size_t index = 0;
         index < sizeof(expected_branches) / sizeof(expected_branches[0]);
         index++) {
        int displacement = (int8_t)bytecode[
            CHILD_CODE_OFFSET + expected_branches[index].operand_pc];
        int target = expected_branches[index].operand_pc + displacement;
        size_t target_ir = CHILD_INSTRUCTION_COUNT;

        for (size_t instruction = 0;
             instruction < sizeof(expected_instruction_pcs);
             instruction++) {
            if (expected_instruction_pcs[instruction] == target) {
                target_ir = instruction;
                break;
            }
        }

        if (bytecode[CHILD_CODE_OFFSET +
                     expected_branches[index].opcode_pc] !=
                expected_branches[index].opcode ||
            displacement != expected_branches[index].displacement ||
            target != expected_branches[index].target_pc ||
            target_ir != expected_branches[index].target_ir) {
            fputs("ordinary leaf branch map drifted\n", stderr);
            goto cleanup;
        }
    }

    eval_runtime = JS_NewRuntime();
    if (!eval_runtime) {
        fputs("ordinary leaf evaluation runtime allocation failed\n", stderr);
        goto cleanup;
    }
    eval_context = JS_NewContext(eval_runtime);
    if (!eval_context) {
        fputs("ordinary leaf evaluation context allocation failed\n", stderr);
        goto cleanup;
    }
    loaded = JS_ReadObject(eval_context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(eval_context, "ordinary leaf bytecode read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(eval_context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten) {
        report_exception(eval_context, "ordinary leaf bytecode rewrite failed");
        goto cleanup;
    }
    if (rewritten_size != bytecode_size ||
        memcmp(rewritten, bytecode, bytecode_size) != 0) {
        fputs("ordinary leaf bytecode rewrite was not identical\n", stderr);
        goto cleanup;
    }

    function = JS_EvalFunction(eval_context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(function)) {
        report_exception(eval_context, "ordinary leaf root evaluation failed");
        function = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsFunction(eval_context, function)) {
        fputs("ordinary leaf root did not evaluate to a function\n", stderr);
        goto cleanup;
    }

    if (JS_IsConstructor(eval_context, function) != 1) {
        fputs("ordinary leaf function was not a constructor\n", stderr);
        goto cleanup;
    }
    length_atom = JS_NewAtom(eval_context, "length");
    name_atom = JS_NewAtom(eval_context, "name");
    prototype_atom = JS_NewAtom(eval_context, "prototype");
    constructor_atom = JS_NewAtom(eval_context, "constructor");
    if (length_atom == JS_ATOM_NULL || name_atom == JS_ATOM_NULL ||
        prototype_atom == JS_ATOM_NULL ||
        constructor_atom == JS_ATOM_NULL) {
        report_exception(eval_context,
                         "ordinary leaf property atom allocation failed");
        goto cleanup;
    }
    if (JS_GetOwnProperty(eval_context, NULL, function, length_atom) != 1 ||
        JS_GetOwnProperty(eval_context, NULL, function, name_atom) != 1 ||
        JS_GetOwnProperty(eval_context, NULL, function, prototype_atom) != 1) {
        fputs("ordinary leaf function own properties drifted\n", stderr);
        goto cleanup;
    }
    length_value = JS_GetPropertyStr(eval_context, function, "length");
    if (JS_IsException(length_value)) {
        report_exception(eval_context,
                         "ordinary leaf function length read failed");
        length_value = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_VALUE_GET_TAG(length_value) != JS_TAG_INT ||
        JS_VALUE_GET_INT(length_value) != 2) {
        fputs("ordinary leaf function length was not exact int 2\n", stderr);
        goto cleanup;
    }
    name_value = JS_GetPropertyStr(eval_context, function, "name");
    if (JS_IsException(name_value)) {
        report_exception(eval_context, "ordinary leaf function name read failed");
        name_value = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsString(name_value)) {
        fputs("ordinary leaf function name was not a string\n", stderr);
        goto cleanup;
    }
    name_string = JS_ToCStringLen(eval_context, &name_length, name_value);
    if (!name_string) {
        report_exception(eval_context,
                         "ordinary leaf function name conversion failed");
        goto cleanup;
    }
    if (name_length != 0 || name_string[0] != '\0') {
        fputs("ordinary leaf function name was not empty\n", stderr);
        goto cleanup;
    }
    prototype_value = JS_GetPropertyStr(eval_context, function, "prototype");
    if (JS_IsException(prototype_value)) {
        report_exception(eval_context,
                         "ordinary leaf function prototype read failed");
        prototype_value = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsObject(prototype_value) ||
        JS_GetOwnProperty(eval_context, NULL, prototype_value,
                          constructor_atom) != 1) {
        fputs("ordinary leaf function prototype shape drifted\n", stderr);
        goto cleanup;
    }
    constructor_value =
        JS_GetPropertyStr(eval_context, prototype_value, "constructor");
    if (JS_IsException(constructor_value)) {
        report_exception(eval_context,
                         "ordinary leaf prototype constructor read failed");
        constructor_value = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_StrictEq(eval_context, constructor_value, function) != 1) {
        fputs("ordinary leaf prototype constructor lost identity\n", stderr);
        goto cleanup;
    }

    arguments[0] = JS_NewInt32(eval_context, 3);
    arguments[1] = JS_NewInt32(eval_context, 3);
    result = JS_Call(eval_context, function, JS_UNDEFINED, 2, arguments);
    if (JS_IsException(result)) {
        report_exception(eval_context, "ordinary leaf equal call failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_VALUE_GET_TAG(result) != JS_TAG_INT ||
        JS_VALUE_GET_INT(result) != 42) {
        fputs("ordinary leaf equal call did not return exact int 42\n",
              stderr);
        goto cleanup;
    }
    JS_FreeValue(eval_context, result);
    result = JS_UNDEFINED;

    instance = JS_CallConstructor(eval_context, function, 2, arguments);
    if (JS_IsException(instance)) {
        report_exception(eval_context, "ordinary leaf constructor call failed");
        instance = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsObject(instance)) {
        fputs("ordinary leaf constructor did not return an object\n", stderr);
        goto cleanup;
    }
    instance_prototype = JS_GetPrototype(eval_context, instance);
    if (JS_IsException(instance_prototype)) {
        report_exception(eval_context,
                         "ordinary leaf instance prototype read failed");
        instance_prototype = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_StrictEq(eval_context, instance_prototype, prototype_value) != 1 ||
        JS_IsInstanceOf(eval_context, instance, function) != 1) {
        fputs("ordinary leaf constructed instance prototype drifted\n", stderr);
        goto cleanup;
    }

    JS_FreeValue(eval_context, arguments[1]);
    arguments[1] = JS_NewInt32(eval_context, 4);
    result = JS_Call(eval_context, function, JS_UNDEFINED, 2, arguments);
    if (JS_IsException(result)) {
        report_exception(eval_context, "ordinary leaf unequal call failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (JS_VALUE_GET_TAG(result) != JS_TAG_INT ||
        JS_VALUE_GET_INT(result) != 0) {
        fputs("ordinary leaf unequal call did not return exact int 0\n",
              stderr);
        goto cleanup;
    }

    caller_value = JS_GetPropertyStr(eval_context, function, "caller");
    if (JS_IsException(caller_value)) {
        report_exception(eval_context,
                         "ordinary leaf sloppy caller read failed");
        caller_value = JS_UNDEFINED;
        goto cleanup;
    }
    arguments_value = JS_GetPropertyStr(eval_context, function, "arguments");
    if (JS_IsException(arguments_value)) {
        report_exception(eval_context,
                         "ordinary leaf sloppy arguments read failed");
        arguments_value = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsUndefined(caller_value) ||
        !JS_IsUndefined(arguments_value)) {
        fputs("ordinary leaf sloppy restricted properties drifted\n", stderr);
        goto cleanup;
    }

    memcpy(strict_bytecode, bytecode, bytecode_size);
    strict_bytecode[CHILD_JS_MODE_OFFSET] = 1;
    strict_runtime = JS_NewRuntime();
    if (!strict_runtime) {
        fputs("ordinary leaf strict runtime allocation failed\n", stderr);
        goto cleanup;
    }
    strict_context = JS_NewContext(strict_runtime);
    if (!strict_context) {
        fputs("ordinary leaf strict context allocation failed\n", stderr);
        goto cleanup;
    }
    strict_loaded = JS_ReadObject(strict_context, strict_bytecode,
                                  bytecode_size, JS_READ_OBJ_BYTECODE);
    if (JS_IsException(strict_loaded)) {
        report_exception(strict_context,
                         "ordinary leaf strict bytecode read failed");
        strict_loaded = JS_UNDEFINED;
        goto cleanup;
    }
    strict_rewritten = JS_WriteObject(strict_context,
                                      &strict_rewritten_size,
                                      strict_loaded,
                                      JS_WRITE_OBJ_BYTECODE);
    if (!strict_rewritten) {
        report_exception(strict_context,
                         "ordinary leaf strict bytecode rewrite failed");
        goto cleanup;
    }
    if (strict_rewritten_size != bytecode_size ||
        memcmp(strict_rewritten, strict_bytecode, bytecode_size) != 0) {
        fputs("ordinary leaf strict bytecode rewrite was not identical\n",
              stderr);
        goto cleanup;
    }
    strict_function = JS_EvalFunction(strict_context, strict_loaded);
    strict_loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(strict_function)) {
        report_exception(strict_context,
                         "ordinary leaf strict root evaluation failed");
        strict_function = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsFunction(strict_context, strict_function)) {
        fputs("ordinary leaf strict root did not evaluate to a function\n",
              stderr);
        goto cleanup;
    }
    strict_property =
        JS_GetPropertyStr(strict_context, strict_function, "caller");
    if (!JS_IsException(strict_property)) {
        fputs("ordinary leaf strict caller did not throw\n", stderr);
        goto cleanup;
    }
    strict_property = JS_UNDEFINED;
    strict_exception = JS_GetException(strict_context);
    if (expect_exception_fields(strict_context,
                                "ordinary leaf strict caller read",
                                strict_exception, "TypeError",
                                "invalid property access"))
        goto cleanup;
    JS_FreeValue(strict_context, strict_exception);
    strict_exception = JS_UNDEFINED;

    strict_property =
        JS_GetPropertyStr(strict_context, strict_function, "arguments");
    if (!JS_IsException(strict_property)) {
        fputs("ordinary leaf strict arguments did not throw\n", stderr);
        goto cleanup;
    }
    strict_property = JS_UNDEFINED;
    strict_exception = JS_GetException(strict_context);
    if (expect_exception_fields(strict_context,
                                "ordinary leaf strict arguments read",
                                strict_exception, "TypeError",
                                "invalid property access"))
        goto cleanup;

    function_wire = JS_WriteObject(eval_context, &function_wire_size,
                                   function, JS_WRITE_OBJ_BYTECODE);
    if (function_wire) {
        fputs("ordinary leaf evaluated closure unexpectedly serialized\n",
              stderr);
        goto cleanup;
    }
    exception = JS_GetException(eval_context);
    if (expect_exception_fields(eval_context,
                                "ordinary leaf evaluated closure write",
                                exception, "TypeError",
                                "unsupported object class"))
        goto cleanup;

    fputs("ordinary-leaf-source-hex=", stdout);
    for (size_t index = 0; index < strlen(ordinary_leaf_source); index++)
        printf("%02x", (unsigned char)ordinary_leaf_source[index]);
    putchar('\n');
    printf("ordinary-leaf-bytecode-size=%zu\n", bytecode_size);
    fputs("ordinary-leaf-bytecode-hex=", stdout);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    puts("ordinary-leaf-rewrite=identity");
    printf("ordinary-leaf-root-cpool=%u\n",
           bytecode[ROOT_CPOOL_COUNT_OFFSET]);
    printf("ordinary-leaf-child-offset=%d\n", CHILD_OFFSET);
    printf("ordinary-leaf-child-flags=%04x\n", child_flags);
    printf("ordinary-leaf-child-js-mode=%02x\n",
           bytecode[CHILD_JS_MODE_OFFSET]);
    printf("ordinary-leaf-child-args=%u\n",
           bytecode[CHILD_ARG_COUNT_OFFSET]);
    printf("ordinary-leaf-child-vars=%u\n",
           bytecode[CHILD_VAR_COUNT_OFFSET]);
    printf("ordinary-leaf-child-defined-args=%u\n",
           bytecode[CHILD_DEFINED_ARG_COUNT_OFFSET]);
    printf("ordinary-leaf-child-stack=%u\n",
           bytecode[CHILD_STACK_SIZE_OFFSET]);
    printf("ordinary-leaf-child-var-refs=%u\n",
           bytecode[CHILD_VAR_REF_COUNT_OFFSET]);
    printf("ordinary-leaf-child-closures=%u\n",
           bytecode[CHILD_CLOSURE_COUNT_OFFSET]);
    printf("ordinary-leaf-child-cpool=%u\n",
           bytecode[CHILD_CPOOL_COUNT_OFFSET]);
    printf("ordinary-leaf-child-code-size=%u\n",
           bytecode[CHILD_CODE_SIZE_OFFSET]);
    printf("ordinary-leaf-child-local-count=%u\n",
           bytecode[CHILD_LOCAL_COUNT_OFFSET]);
    printf("ordinary-leaf-child-instruction-count=%zu\n",
           sizeof(expected_instruction_pcs));
    fputs("ordinary-leaf-child-code-hex=", stdout);
    for (size_t index = 0; index < CHILD_CODE_SIZE; index++)
        printf("%02x", bytecode[CHILD_CODE_OFFSET + index]);
    putchar('\n');
    fputs("ordinary-leaf-child-cpool-hex=", stdout);
    for (size_t index = 0; index < sizeof(expected_child_pool); index++)
        printf("%02x", bytecode[CHILD_POOL_OFFSET + index]);
    putchar('\n');
    for (size_t index = 0;
         index < sizeof(expected_branches) / sizeof(expected_branches[0]);
         index++) {
        printf("ordinary-leaf-branch-%zu=%u->%u/IR%u,operand%u,"
               "displacement%d\n",
               index, expected_branches[index].opcode_pc,
               expected_branches[index].target_pc,
               expected_branches[index].target_ir,
               expected_branches[index].operand_pc,
               expected_branches[index].displacement);
    }
    puts("ordinary-leaf-is-constructor=true");
    puts("ordinary-leaf-own-length=2");
    puts("ordinary-leaf-own-name=\"\"");
    puts("ordinary-leaf-prototype-constructor=identity");
    puts("ordinary-leaf-call-3-3-tag=int");
    puts("ordinary-leaf-call-3-3=42");
    puts("ordinary-leaf-new-3-3-prototype=identity");
    puts("ordinary-leaf-new-3-3-instanceof=true");
    puts("ordinary-leaf-call-3-4-tag=int");
    puts("ordinary-leaf-call-3-4=0");
    puts("ordinary-leaf-sloppy-caller=undefined");
    puts("ordinary-leaf-sloppy-arguments=undefined");
    puts("ordinary-leaf-strict-js-mode=01");
    puts("ordinary-leaf-strict-rewrite=identity");
    puts("ordinary-leaf-strict-caller-class=TypeError");
    puts("ordinary-leaf-strict-caller-message=invalid property access");
    puts("ordinary-leaf-strict-arguments-class=TypeError");
    puts("ordinary-leaf-strict-arguments-message=invalid property access");
    puts("ordinary-leaf-closure-write-class=TypeError");
    puts("ordinary-leaf-closure-write-message=unsupported object class");
    status = 0;

cleanup:
    if (strict_context) {
        if (strict_rewritten)
            js_free(strict_context, strict_rewritten);
        JS_FreeValue(strict_context, strict_exception);
        JS_FreeValue(strict_context, strict_property);
        JS_FreeValue(strict_context, strict_function);
        JS_FreeValue(strict_context, strict_loaded);
        JS_FreeContext(strict_context);
    }
    if (strict_runtime)
        JS_FreeRuntime(strict_runtime);
    if (eval_context) {
        if (function_wire)
            js_free(eval_context, function_wire);
        if (rewritten)
            js_free(eval_context, rewritten);
        if (name_string)
            JS_FreeCString(eval_context, name_string);
        if (constructor_atom != JS_ATOM_NULL)
            JS_FreeAtom(eval_context, constructor_atom);
        if (prototype_atom != JS_ATOM_NULL)
            JS_FreeAtom(eval_context, prototype_atom);
        if (name_atom != JS_ATOM_NULL)
            JS_FreeAtom(eval_context, name_atom);
        if (length_atom != JS_ATOM_NULL)
            JS_FreeAtom(eval_context, length_atom);
        JS_FreeValue(eval_context, exception);
        JS_FreeValue(eval_context, arguments_value);
        JS_FreeValue(eval_context, caller_value);
        JS_FreeValue(eval_context, instance_prototype);
        JS_FreeValue(eval_context, instance);
        JS_FreeValue(eval_context, constructor_value);
        JS_FreeValue(eval_context, prototype_value);
        JS_FreeValue(eval_context, name_value);
        JS_FreeValue(eval_context, length_value);
        JS_FreeValue(eval_context, result);
        JS_FreeValue(eval_context, arguments[1]);
        JS_FreeValue(eval_context, arguments[0]);
        JS_FreeValue(eval_context, function);
        JS_FreeValue(eval_context, loaded);
        JS_FreeContext(eval_context);
    }
    if (eval_runtime)
        JS_FreeRuntime(eval_runtime);
    if (compile_context) {
        if (bytecode)
            js_free(compile_context, bytecode);
        JS_FreeValue(compile_context, compiled);
        JS_FreeContext(compile_context);
    }
    if (compile_runtime)
        JS_FreeRuntime(compile_runtime);
    return status;
}

static int expect_read_exception(const char *label,
                                 const uint8_t *bytecode,
                                 size_t bytecode_size,
                                 const char *expected_class,
                                 const char *expected_message) {
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", label);
        goto cleanup;
    }

    loaded = JS_ReadObject(context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (!JS_IsException(loaded)) {
        fprintf(stderr, "%s bytecode was unexpectedly accepted\n", label);
        goto cleanup;
    }
    exception = JS_GetException(context);
    if (expect_exception_fields(context, label, exception,
                                expected_class, expected_message))
        goto cleanup;

    printf("%s-hex=", label);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("%s-class=%s\n", label, expected_class);
    printf("%s-message=%s\n", label, expected_message);
    status = 0;

cleanup:
    if (context) {
        JS_FreeValue(context, exception);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int expect_read_scalar(const char *label,
                              const uint8_t *bytecode,
                              size_t bytecode_size,
                              ScalarExpectation expected) {
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    uint8_t *rewritten_bytecode = NULL;
    size_t rewritten_bytecode_size = 0;
    double actual_number = 0;
    uint64_t actual_bits = 0;
    int actual_tag = -1;
    int actual_boolean = -1;
    int64_t actual_integer = 0;
    const char *actual_string = NULL;
    size_t actual_string_length = 0;
    int status = -1;

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", label);
        goto cleanup;
    }

    loaded = JS_ReadObject(context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "scalar bytecode read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    if (expected.kind == SCALAR_VALUE_FLOAT64) {
        rewritten_bytecode = JS_WriteObject(
            context, &rewritten_bytecode_size, loaded,
            JS_WRITE_OBJ_BYTECODE);
        if (!rewritten_bytecode) {
            report_exception(context, "scalar bytecode rewrite failed");
            goto cleanup;
        }
        if (rewritten_bytecode_size != bytecode_size ||
            memcmp(rewritten_bytecode, bytecode, bytecode_size) != 0) {
            fprintf(stderr, "%s did not preserve its bytecode wire\n", label);
            goto cleanup;
        }
    }
    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(context, "scalar bytecode evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }

    switch (expected.kind) {
    case SCALAR_VALUE_NUMBER:
        if (!JS_IsNumber(result) ||
            JS_ToFloat64(context, &actual_number, result) < 0) {
            fprintf(stderr, "%s did not evaluate to a number\n", label);
            goto cleanup;
        }
        if (actual_number != expected.number) {
            fprintf(stderr, "%s evaluated to %.17g, expected %.17g\n",
                    label, actual_number, expected.number);
            goto cleanup;
        }
        break;
    case SCALAR_VALUE_UNDEFINED:
        if (!JS_IsUndefined(result)) {
            fprintf(stderr, "%s did not evaluate to undefined\n", label);
            goto cleanup;
        }
        break;
    case SCALAR_VALUE_NULL:
        if (!JS_IsNull(result)) {
            fprintf(stderr, "%s did not evaluate to null\n", label);
            goto cleanup;
        }
        break;
    case SCALAR_VALUE_BOOLEAN:
        if (!JS_IsBool(result)) {
            fprintf(stderr, "%s did not evaluate to a boolean\n", label);
            goto cleanup;
        }
        actual_boolean = JS_ToBool(context, result);
        if (actual_boolean != expected.integer) {
            fprintf(stderr, "%s evaluated to %s, expected %s\n", label,
                    actual_boolean ? "true" : "false",
                    expected.integer ? "true" : "false");
            goto cleanup;
        }
        break;
    case SCALAR_VALUE_BIGINT:
        if (!JS_IsBigInt(context, result)) {
            fprintf(stderr, "%s did not evaluate to a BigInt\n", label);
            goto cleanup;
        }
        if (JS_ToBigInt64(context, &actual_integer, result) < 0) {
            report_exception(context, "BigInt conversion failed");
            goto cleanup;
        }
        if (actual_integer != expected.integer) {
            fprintf(stderr, "%s evaluated to %lldn, expected %dn\n", label,
                    (long long)actual_integer, expected.integer);
            goto cleanup;
        }
        break;
    case SCALAR_VALUE_EMPTY_STRING:
        if (!JS_IsString(result)) {
            fprintf(stderr, "%s did not evaluate to a string\n", label);
            goto cleanup;
        }
        actual_string = JS_ToCStringLen(context, &actual_string_length, result);
        if (!actual_string) {
            report_exception(context, "string conversion failed");
            goto cleanup;
        }
        if (actual_string_length != 0 || actual_string[0] != '\0') {
            fprintf(stderr, "%s did not evaluate to the empty string\n", label);
            goto cleanup;
        }
        break;
    case SCALAR_VALUE_FLOAT64:
        actual_tag = JS_VALUE_GET_TAG(result);
        if (actual_tag != JS_TAG_FLOAT64 ||
            JS_ToFloat64(context, &actual_number, result) < 0) {
            fprintf(stderr, "%s did not evaluate to JS_TAG_FLOAT64\n", label);
            goto cleanup;
        }
        memcpy(&actual_bits, &actual_number, sizeof(actual_bits));
        if (actual_bits != expected.bits) {
            fprintf(stderr,
                    "%s evaluated to Float64 bits %016" PRIx64
                    ", expected %016" PRIx64 "\n",
                    label, actual_bits, expected.bits);
            goto cleanup;
        }
        break;
    default:
        fprintf(stderr, "%s has an invalid expected scalar kind\n", label);
        goto cleanup;
    }

    printf("%s-hex=", label);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    switch (expected.kind) {
    case SCALAR_VALUE_NUMBER:
        printf("%s-eval=%.17g\n", label, actual_number);
        break;
    case SCALAR_VALUE_UNDEFINED:
        printf("%s-eval=undefined\n", label);
        break;
    case SCALAR_VALUE_NULL:
        printf("%s-eval=null\n", label);
        break;
    case SCALAR_VALUE_BOOLEAN:
        printf("%s-eval=%s\n", label,
               actual_boolean ? "true" : "false");
        break;
    case SCALAR_VALUE_BIGINT:
        printf("%s-eval=%lldn\n", label, (long long)actual_integer);
        break;
    case SCALAR_VALUE_EMPTY_STRING:
        printf("%s-eval=\"\"\n", label);
        break;
    case SCALAR_VALUE_FLOAT64:
        printf("%s-rewrite=identity\n", label);
        printf("%s-eval-tag=%d\n", label, actual_tag);
        printf("%s-eval-bits=%016" PRIx64 "\n", label, actual_bits);
        break;
    }
    status = 0;

cleanup:
    if (context) {
        if (rewritten_bytecode)
            js_free(context, rewritten_bytecode);
        if (actual_string)
            JS_FreeCString(context, actual_string);
        JS_FreeValue(context, result);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int build_scalar_wire(const ScalarCase *test,
                             uint8_t *output,
                             size_t output_capacity,
                             size_t *output_size) {
    size_t offset = 0;
    size_t expected_size;
    int has_float64_constant;

    if (test->code_size == 0 ||
        test->code_size > SCALAR_MAX_CODE_SIZE)
        return -1;
    has_float64_constant = test->expected.kind == SCALAR_VALUE_FLOAT64;
    expected_size = sizeof(scalar_prefix) + 2 +
                    sizeof(scalar_local) + test->code_size +
                    (has_float64_constant ? SCALAR_FLOAT64_POOL_SIZE : 0);
    if (expected_size > output_capacity)
        return -1;

    memcpy(output + offset, scalar_prefix, sizeof(scalar_prefix));
    offset += sizeof(scalar_prefix);
    output[offset++] = has_float64_constant ? 1 : 0;
    output[offset++] = (uint8_t)test->code_size;
    memcpy(output + offset, scalar_local, sizeof(scalar_local));
    offset += sizeof(scalar_local);
    memcpy(output + offset, test->code, test->code_size);
    offset += test->code_size;
    if (has_float64_constant) {
        output[offset++] = 0x06;
        for (unsigned shift = 0; shift < 64; shift += 8)
            output[offset++] = (uint8_t)(test->expected.bits >> shift);
    }
    *output_size = offset;
    return 0;
}

static int expect_compiled_scalar(JSContext *compile_context,
                                  const ScalarCase *test) {
    uint8_t expected_wire[SCALAR_MAX_WIRE_SIZE];
    size_t expected_wire_size = 0;
    JSValue compiled = JS_UNDEFINED;
    uint8_t *bytecode = NULL;
    size_t bytecode_size = 0;
    int status = -1;

    if (!test->source ||
        build_scalar_wire(test, expected_wire, sizeof(expected_wire),
                          &expected_wire_size)) {
        fprintf(stderr, "%s has an invalid oracle definition\n", test->label);
        goto cleanup;
    }

    compiled = JS_Eval(compile_context, test->source, strlen(test->source),
                       "scalar.js",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        fprintf(stderr, "%s ", test->label);
        report_exception(compile_context, "compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }

    bytecode = JS_WriteObject(compile_context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        fprintf(stderr, "%s ", test->label);
        report_exception(compile_context, "bytecode serialization failed");
        goto cleanup;
    }
    if (bytecode_size != expected_wire_size ||
        memcmp(bytecode, expected_wire, expected_wire_size) != 0) {
        fprintf(stderr,
                "%s compiler wire did not match its pinned BC5 vector\n",
                test->label);
        goto cleanup;
    }

    printf("%s-source-hex=", test->label);
    for (size_t index = 0; test->source[index] != '\0'; index++)
        printf("%02x", (unsigned char)test->source[index]);
    putchar('\n');
    if (expect_read_scalar(test->label, bytecode, bytecode_size,
                           test->expected))
        goto cleanup;
    status = 0;

cleanup:
    if (bytecode)
        js_free(compile_context, bytecode);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static int expect_compatible_scalar(const ScalarCase *test) {
    uint8_t wire[SCALAR_MAX_WIRE_SIZE];
    size_t wire_size = 0;

    if (test->source ||
        build_scalar_wire(test, wire, sizeof(wire), &wire_size)) {
        fprintf(stderr, "%s has an invalid oracle definition\n", test->label);
        return -1;
    }
    return expect_read_scalar(test->label, wire, wire_size, test->expected);
}

static int append_uleb_size(uint8_t *output,
                            size_t output_capacity,
                            size_t *offset,
                            size_t value) {
    do {
        uint8_t byte = (uint8_t)(value & 0x7f);

        value >>= 7;
        if (value != 0)
            byte |= 0x80;
        if (*offset >= output_capacity)
            return -1;
        output[(*offset)++] = byte;
    } while (value != 0);
    return 0;
}

static int build_scalar_encoding_wire(const ScalarWireEncoding *encoding,
                                      uint8_t *output,
                                      size_t output_capacity,
                                      size_t *output_size) {
    size_t offset = 0;

    if (!encoding->atom_header || encoding->atom_header_size == 0 ||
        !encoding->code || encoding->code_size == 0 ||
        encoding->code_size > SCALAR_MAX_CODE_SIZE ||
        encoding->pool_count > 1 ||
        (encoding->pool_count == 0 &&
         (encoding->pool || encoding->pool_size != 0)) ||
        (encoding->pool_count == 1 &&
         (!encoding->pool || encoding->pool_size == 0)))
        return -1;

    if (output_capacity < 1 ||
        encoding->atom_header_size > output_capacity - 1)
        return -1;
    output[offset++] = 0x05;
    memcpy(output + offset, encoding->atom_header,
           encoding->atom_header_size);
    offset += encoding->atom_header_size;

    if (sizeof(scalar_prefix) - 2 > output_capacity - offset)
        return -1;
    memcpy(output + offset, scalar_prefix + 2, sizeof(scalar_prefix) - 2);
    offset += sizeof(scalar_prefix) - 2;
    if (append_uleb_size(output, output_capacity, &offset,
                         encoding->pool_count) ||
        append_uleb_size(output, output_capacity, &offset,
                         encoding->code_size) ||
        sizeof(scalar_local) > output_capacity - offset)
        return -1;
    memcpy(output + offset, scalar_local, sizeof(scalar_local));
    offset += sizeof(scalar_local);

    if (encoding->code_size > output_capacity - offset)
        return -1;
    memcpy(output + offset, encoding->code, encoding->code_size);
    offset += encoding->code_size;
    if (encoding->pool_size > output_capacity - offset)
        return -1;
    if (encoding->pool_size != 0)
        memcpy(output + offset, encoding->pool, encoding->pool_size);
    offset += encoding->pool_size;
    *output_size = offset;
    return 0;
}

static int expect_string_scalar_case(JSContext *compile_context,
                                     const StringScalarCase *test) {
    uint8_t input_wire[STRING_SCALAR_MAX_WIRE_SIZE];
    uint8_t canonical_wire[STRING_SCALAR_MAX_WIRE_SIZE];
    size_t input_wire_size = 0;
    size_t canonical_wire_size = 0;
    const ScalarWireEncoding *canonical =
        test->canonical.atom_header ? &test->canonical : &test->input;
    JSValue compiled = JS_UNDEFINED;
    uint8_t *compiled_wire = NULL;
    size_t compiled_wire_size = 0;
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue length_value = JS_UNDEFINED;
    JSValue char_code_at = JS_UNDEFINED;
    uint8_t *rewritten_wire = NULL;
    size_t rewritten_wire_size = 0;
    uint32_t actual_length = 0;
    int actual_tag = -1;
    int rewrite_is_identity;
    int status = -1;

    if (!test->label || !test->cohort ||
        (test->expected_kind == STRING_SCALAR_STRING &&
         test->expected_tag != JS_TAG_STRING) ||
        (test->expected_kind == STRING_SCALAR_SYMBOL &&
         (test->expected_tag != JS_TAG_SYMBOL || test->expected_units ||
          test->expected_unit_count != 0)) ||
        build_scalar_encoding_wire(&test->input, input_wire,
                                   sizeof(input_wire), &input_wire_size) ||
        build_scalar_encoding_wire(canonical, canonical_wire,
                                   sizeof(canonical_wire),
                                   &canonical_wire_size)) {
        fprintf(stderr, "%s has an invalid String oracle definition\n",
                test->label ? test->label : "<unnamed>");
        goto cleanup;
    }

    if (test->source) {
        compiled = JS_Eval(compile_context, test->source,
                           strlen(test->source), "string-scalar.js",
                           JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
        if (JS_IsException(compiled)) {
            fprintf(stderr, "%s ", test->label);
            report_exception(compile_context, "compile failed");
            compiled = JS_UNDEFINED;
            goto cleanup;
        }
        compiled_wire = JS_WriteObject(compile_context, &compiled_wire_size,
                                       compiled, JS_WRITE_OBJ_BYTECODE);
        if (!compiled_wire) {
            fprintf(stderr, "%s ", test->label);
            report_exception(compile_context,
                             "bytecode serialization failed");
            goto cleanup;
        }
        if (compiled_wire_size != input_wire_size ||
            memcmp(compiled_wire, input_wire, input_wire_size) != 0) {
            fprintf(stderr,
                    "%s compiler wire did not match its pinned String BC5 vector\n",
                    test->label);
            goto cleanup;
        }
    }

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", test->label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", test->label);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, input_wire, input_wire_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "bytecode read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten_wire = JS_WriteObject(context, &rewritten_wire_size, loaded,
                                    JS_WRITE_OBJ_BYTECODE);
    if (!rewritten_wire) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "bytecode rewrite failed");
        goto cleanup;
    }
    if (rewritten_wire_size != canonical_wire_size ||
        memcmp(rewritten_wire, canonical_wire, canonical_wire_size) != 0) {
        fprintf(stderr,
                "%s rewrite did not match its canonical String BC5 vector\n",
                test->label);
        goto cleanup;
    }

    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "fresh-runtime evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    actual_tag = JS_VALUE_GET_TAG(result);
    if (actual_tag != test->expected_tag) {
        fprintf(stderr, "%s evaluated with tag %d, expected %d\n",
                test->label, actual_tag, test->expected_tag);
        goto cleanup;
    }

    if (test->expected_kind == STRING_SCALAR_SYMBOL) {
        if (!JS_IsSymbol(result)) {
            fprintf(stderr, "%s did not evaluate to a Symbol\n", test->label);
            goto cleanup;
        }
    } else {
        if (!JS_IsString(result)) {
            fprintf(stderr, "%s did not evaluate to a String\n", test->label);
            goto cleanup;
        }
        length_value = JS_GetPropertyStr(context, result, "length");
        if (JS_IsException(length_value) ||
            JS_ToUint32(context, &actual_length, length_value) < 0) {
            fprintf(stderr, "%s ", test->label);
            report_exception(context, "String length inspection failed");
            goto cleanup;
        }
        if (actual_length != test->expected_unit_count) {
            fprintf(stderr,
                    "%s evaluated to %u UTF-16 units, expected %zu\n",
                    test->label, actual_length,
                    test->expected_unit_count);
            goto cleanup;
        }
        char_code_at = JS_GetPropertyStr(context, result, "charCodeAt");
        if (JS_IsException(char_code_at)) {
            fprintf(stderr, "%s ", test->label);
            report_exception(context, "String charCodeAt lookup failed");
            goto cleanup;
        }
        for (uint32_t index = 0; index < actual_length; index++) {
            JSValue argument = JS_NewUint32(context, index);
            JSValue code = JS_Call(context, char_code_at, result,
                                   1, &argument);
            uint32_t actual_unit = 0;

            JS_FreeValue(context, argument);
            if (JS_IsException(code) ||
                JS_ToUint32(context, &actual_unit, code) < 0) {
                JS_FreeValue(context, code);
                fprintf(stderr, "%s ", test->label);
                report_exception(context,
                                 "String charCodeAt inspection failed");
                goto cleanup;
            }
            JS_FreeValue(context, code);
            if (actual_unit != test->expected_units[index]) {
                fprintf(stderr,
                        "%s UTF-16 unit %u was %04x, expected %04x\n",
                        test->label, index, actual_unit,
                        test->expected_units[index]);
                goto cleanup;
            }
        }
    }

    rewrite_is_identity = input_wire_size == canonical_wire_size &&
                          memcmp(input_wire, canonical_wire,
                                 input_wire_size) == 0;
    if (test->source) {
        printf("%s-source-hex=", test->label);
        for (size_t index = 0; test->source[index] != '\0'; index++)
            printf("%02x", (unsigned char)test->source[index]);
        putchar('\n');
    }
    printf("%s-hex=", test->label);
    for (size_t index = 0; index < input_wire_size; index++)
        printf("%02x", input_wire[index]);
    putchar('\n');
    printf("%s-cohort=%s\n", test->label, test->cohort);
    printf("%s-rewrite=%s\n", test->label,
           rewrite_is_identity ? "identity" : "canonical");
    if (!rewrite_is_identity) {
        printf("%s-rewrite-hex=", test->label);
        for (size_t index = 0; index < canonical_wire_size; index++)
            printf("%02x", canonical_wire[index]);
        putchar('\n');
    }
    printf("%s-eval-kind=%s\n", test->label,
           test->expected_kind == STRING_SCALAR_STRING ? "String" : "Symbol");
    printf("%s-eval-tag=%d\n", test->label, actual_tag);
    printf("%s-eval-u16=", test->label);
    if (test->expected_kind == STRING_SCALAR_SYMBOL) {
        puts("-");
    } else {
        for (size_t index = 0; index < test->expected_unit_count; index++) {
            if (index != 0)
                putchar(',');
            printf("%04x", test->expected_units[index]);
        }
        putchar('\n');
    }
    status = 0;

cleanup:
    if (context) {
        if (rewritten_wire)
            js_free(context, rewritten_wire);
        JS_FreeValue(context, char_code_at);
        JS_FreeValue(context, length_value);
        JS_FreeValue(context, result);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    if (compiled_wire)
        js_free(compile_context, compiled_wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static const StringScalarCase *find_string_scalar_case(const char *label) {
    for (size_t index = 0;
         index < sizeof(canonical_string_scalars) /
                     sizeof(canonical_string_scalars[0]);
         index++) {
        if (strcmp(canonical_string_scalars[index].label, label) == 0)
            return &canonical_string_scalars[index];
    }
    for (size_t index = 0;
         index < sizeof(compatible_string_scalars) /
                     sizeof(compatible_string_scalars[0]);
         index++) {
        if (strcmp(compatible_string_scalars[index].label, label) == 0)
            return &compatible_string_scalars[index];
    }
    return NULL;
}

static int read_scalar_encoding_function(JSContext *context,
                                         const ScalarWireEncoding *encoding,
                                         const char *label,
                                         JSValue *function) {
    uint8_t wire[STRING_SCALAR_MAX_WIRE_SIZE];
    size_t wire_size = 0;

    if (!encoding || !label ||
        build_scalar_encoding_wire(encoding, wire, sizeof(wire),
                                   &wire_size)) {
        fprintf(stderr, "%s has an invalid identity oracle encoding\n",
                label ? label : "<unnamed>");
        return -1;
    }
    *function = JS_ReadObject(context, wire, wire_size,
                              JS_READ_OBJ_BYTECODE);
    if (JS_IsException(*function)) {
        fprintf(stderr, "%s ", label);
        report_exception(context, "identity bytecode read failed");
        *function = JS_UNDEFINED;
        return -1;
    }
    return 0;
}

static int read_string_scalar_function(JSContext *context,
                                       const StringScalarCase *test,
                                       JSValue *function) {
    if (!test) {
        fprintf(stderr, "String identity matrix has an invalid oracle case\n");
        return -1;
    }
    return read_scalar_encoding_function(context, &test->input,
                                         test->label, function);
}

static int eval_string_function(JSContext *context,
                                JSValueConst function,
                                JSValue *result) {
    *result = JS_EvalFunction(context, JS_DupValue(context, function));
    if (JS_IsException(*result)) {
        report_exception(context, "identity bytecode evaluation failed");
        *result = JS_UNDEFINED;
        return -1;
    }
    if (!JS_IsString(*result)) {
        fprintf(stderr, "String identity matrix produced a non-String value\n");
        return -1;
    }
    return 0;
}

static int expect_string_scalar_identity_matrix(void) {
    const StringScalarCase *cpool =
        find_string_scalar_case("compatible-string-cpool-42-wide-op");
    const StringScalarCase *atom =
        find_string_scalar_case("canonical-string-dynamic-a");
    const StringScalarCase *empty_direct =
        find_string_scalar_case("canonical-string-empty");
    const StringScalarCase *empty_atom =
        find_string_scalar_case("compatible-string-atom-empty");
    const StringScalarCase *cpool_empty =
        find_string_scalar_case("compatible-string-cpool-empty");
    const StringScalarCase *tagged =
        find_string_scalar_case("compatible-string-atom-tagged-42");
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue cpool_function = JS_UNDEFINED;
    JSValue cpool_reload_function = JS_UNDEFINED;
    JSValue atom_function = JS_UNDEFINED;
    JSValue atom_reload_function = JS_UNDEFINED;
    JSValue empty_direct_function = JS_UNDEFINED;
    JSValue empty_atom_function = JS_UNDEFINED;
    JSValue cpool_empty_function = JS_UNDEFINED;
    JSValue tagged_function = JS_UNDEFINED;
    JSValue cpool_first = JS_UNDEFINED;
    JSValue cpool_repeat = JS_UNDEFINED;
    JSValue cpool_reload = JS_UNDEFINED;
    JSValue atom_first = JS_UNDEFINED;
    JSValue atom_reload = JS_UNDEFINED;
    JSValue empty_direct_result = JS_UNDEFINED;
    JSValue empty_atom_result = JS_UNDEFINED;
    JSValue cpool_empty_result = JS_UNDEFINED;
    JSValue tagged_first = JS_UNDEFINED;
    JSValue tagged_repeat = JS_UNDEFINED;
    int cpool_repeat_same;
    int cpool_reload_same;
    int atom_reload_same;
    int empty_forms_same;
    int cpool_empty_direct_same;
    int tagged_repeat_same;
    int tagged_cpool_same;
    int status = -1;

    if (!cpool || !atom || !empty_direct || !empty_atom || !cpool_empty ||
        !tagged) {
        fprintf(stderr, "String identity matrix lost a named oracle case\n");
        goto cleanup;
    }
    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "String identity runtime allocation failed\n");
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "String identity context allocation failed\n");
        goto cleanup;
    }

    if (read_string_scalar_function(context, cpool, &cpool_function) ||
        read_string_scalar_function(context, cpool,
                                    &cpool_reload_function) ||
        read_string_scalar_function(context, atom, &atom_function) ||
        read_string_scalar_function(context, atom, &atom_reload_function) ||
        read_string_scalar_function(context, empty_direct,
                                    &empty_direct_function) ||
        read_string_scalar_function(context, empty_atom,
                                    &empty_atom_function) ||
        read_string_scalar_function(context, cpool_empty,
                                    &cpool_empty_function) ||
        read_string_scalar_function(context, tagged, &tagged_function) ||
        eval_string_function(context, cpool_function, &cpool_first) ||
        eval_string_function(context, cpool_function, &cpool_repeat) ||
        eval_string_function(context, cpool_reload_function,
                             &cpool_reload) ||
        eval_string_function(context, atom_function, &atom_first) ||
        eval_string_function(context, atom_reload_function,
                             &atom_reload) ||
        eval_string_function(context, empty_direct_function,
                             &empty_direct_result) ||
        eval_string_function(context, empty_atom_function,
                             &empty_atom_result) ||
        eval_string_function(context, cpool_empty_function,
                             &cpool_empty_result) ||
        eval_string_function(context, tagged_function, &tagged_first) ||
        eval_string_function(context, tagged_function, &tagged_repeat))
        goto cleanup;

    cpool_repeat_same =
        JS_VALUE_GET_PTR(cpool_first) == JS_VALUE_GET_PTR(cpool_repeat);
    cpool_reload_same =
        JS_VALUE_GET_PTR(cpool_first) == JS_VALUE_GET_PTR(cpool_reload);
    atom_reload_same =
        JS_VALUE_GET_PTR(atom_first) == JS_VALUE_GET_PTR(atom_reload);
    empty_forms_same =
        JS_VALUE_GET_PTR(empty_direct_result) ==
        JS_VALUE_GET_PTR(empty_atom_result);
    cpool_empty_direct_same =
        JS_VALUE_GET_PTR(cpool_empty_result) ==
        JS_VALUE_GET_PTR(empty_direct_result);
    tagged_repeat_same =
        JS_VALUE_GET_PTR(tagged_first) == JS_VALUE_GET_PTR(tagged_repeat);
    tagged_cpool_same =
        JS_VALUE_GET_PTR(tagged_first) == JS_VALUE_GET_PTR(cpool_first);
    if (!cpool_repeat_same || cpool_reload_same || !atom_reload_same ||
        !empty_forms_same || cpool_empty_direct_same || tagged_repeat_same ||
        tagged_cpool_same) {
        fprintf(stderr,
                "String representation identity matrix did not match pinned QuickJS\n");
        goto cleanup;
    }

    puts("string-identity-cpool-repeat=same");
    puts("string-identity-cpool-reload=distinct");
    puts("string-identity-ordinary-atom-reload=same");
    puts("string-identity-empty-direct-atom=same");
    puts("string-identity-cpool-empty-direct=distinct");
    puts("string-identity-tagged-repeat=distinct");
    puts("string-identity-tagged-cpool-42=distinct");
    status = 0;

cleanup:
    if (context) {
        JS_FreeValue(context, tagged_repeat);
        JS_FreeValue(context, tagged_first);
        JS_FreeValue(context, empty_atom_result);
        JS_FreeValue(context, empty_direct_result);
        JS_FreeValue(context, cpool_empty_result);
        JS_FreeValue(context, atom_reload);
        JS_FreeValue(context, atom_first);
        JS_FreeValue(context, cpool_reload);
        JS_FreeValue(context, cpool_repeat);
        JS_FreeValue(context, cpool_first);
        JS_FreeValue(context, tagged_function);
        JS_FreeValue(context, empty_atom_function);
        JS_FreeValue(context, empty_direct_function);
        JS_FreeValue(context, cpool_empty_function);
        JS_FreeValue(context, atom_reload_function);
        JS_FreeValue(context, atom_function);
        JS_FreeValue(context, cpool_reload_function);
        JS_FreeValue(context, cpool_function);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int has_bigint_constant_code_shape(const BigIntConstantCase *test) {
    if (test->cohort == BIGINT_CONSTANT_THREE_INSTRUCTION) {
        return (test->code_size == sizeof(bigint_push_const8) &&
                memcmp(test->code, bigint_push_const8,
                       sizeof(bigint_push_const8)) == 0) ||
               (test->code_size == sizeof(bigint_push_const) &&
                memcmp(test->code, bigint_push_const,
                       sizeof(bigint_push_const)) == 0);
    }
    if (test->cohort == BIGINT_CONSTANT_UNARY_NEG) {
        return (test->code_size == sizeof(bigint_push_const8_neg) &&
                memcmp(test->code, bigint_push_const8_neg,
                       sizeof(bigint_push_const8_neg)) == 0) ||
               (test->code_size == sizeof(bigint_push_const_neg) &&
                memcmp(test->code, bigint_push_const_neg,
                       sizeof(bigint_push_const_neg)) == 0);
    }
    if (test->cohort == BIGINT_CONSTANT_DIRECT_UNARY_NEG) {
        return test->code_size == sizeof(bigint_push_bigint_i32_neg) &&
               memcmp(test->code, bigint_push_bigint_i32_neg,
                      sizeof(bigint_push_bigint_i32_neg)) == 0;
    }
    if (test->cohort == BIGINT_CONSTANT_UNARY_CHAIN) {
        return test->code_size == sizeof(bigint_push_const8_double_neg) &&
               memcmp(test->code, bigint_push_const8_double_neg,
                      sizeof(bigint_push_const8_double_neg)) == 0;
    }
    return 0;
}

static int build_bigint_constant_wire(const BigIntConstantCase *test,
                                      int canonical,
                                      uint8_t *output,
                                      size_t output_capacity,
                                      size_t *output_size) {
    const uint8_t *payload = canonical ? test->canonical_payload
                                       : test->payload;
    size_t payload_size = canonical ? test->canonical_payload_size
                                    : test->payload_size;
    int has_constant_pool =
        test->cohort != BIGINT_CONSTANT_DIRECT_UNARY_NEG;
    size_t offset = 0;

    if (!test->label || !test->expected_decimal ||
        !has_bigint_constant_code_shape(test) ||
        test->code_size > SCALAR_MAX_CODE_SIZE ||
        payload_size > BIGINT_CONSTANT_MAX_PAYLOAD_SIZE ||
        (payload_size != 0 && !payload) ||
        (!has_constant_pool && (payload_size != 0 || payload)))
        return -1;

    if (sizeof(scalar_prefix) + 2 + sizeof(scalar_local) +
            test->code_size + (has_constant_pool ? 1 : 0) >
        output_capacity)
        return -1;
    memcpy(output + offset, scalar_prefix, sizeof(scalar_prefix));
    offset += sizeof(scalar_prefix);
    output[offset++] = has_constant_pool ? 1 : 0;
    output[offset++] = (uint8_t)test->code_size;
    memcpy(output + offset, scalar_local, sizeof(scalar_local));
    offset += sizeof(scalar_local);
    memcpy(output + offset, test->code, test->code_size);
    offset += test->code_size;
    if (has_constant_pool) {
        output[offset++] = 0x0a;
        if (append_uleb_size(output, output_capacity, &offset,
                             payload_size) ||
            payload_size > output_capacity - offset)
            return -1;
        if (payload_size != 0)
            memcpy(output + offset, payload, payload_size);
        offset += payload_size;
    }
    *output_size = offset;
    return 0;
}

static int expect_bigint_constant_case(JSContext *compile_context,
                                       const BigIntConstantCase *test) {
    uint8_t input_wire[BIGINT_CONSTANT_MAX_WIRE_SIZE];
    uint8_t canonical_wire[BIGINT_CONSTANT_MAX_WIRE_SIZE];
    size_t input_wire_size = 0;
    size_t canonical_wire_size = 0;
    JSValue compiled = JS_UNDEFINED;
    uint8_t *compiled_wire = NULL;
    size_t compiled_wire_size = 0;
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    uint8_t *rewritten_wire = NULL;
    size_t rewritten_wire_size = 0;
    const char *actual_decimal = NULL;
    int actual_tag = -1;
    int rewrite_is_identity;
    int status = -1;

    if (build_bigint_constant_wire(test, 0, input_wire,
                                   sizeof(input_wire), &input_wire_size) ||
        build_bigint_constant_wire(test, 1, canonical_wire,
                                   sizeof(canonical_wire),
                                   &canonical_wire_size)) {
        fprintf(stderr, "%s has an invalid oracle definition\n", test->label);
        goto cleanup;
    }

    if (test->source) {
        compiled = JS_Eval(compile_context, test->source,
                           strlen(test->source), "bigint-constant.js",
                           JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
        if (JS_IsException(compiled)) {
            fprintf(stderr, "%s ", test->label);
            report_exception(compile_context, "compile failed");
            compiled = JS_UNDEFINED;
            goto cleanup;
        }
        compiled_wire = JS_WriteObject(compile_context, &compiled_wire_size,
                                       compiled, JS_WRITE_OBJ_BYTECODE);
        if (!compiled_wire) {
            fprintf(stderr, "%s ", test->label);
            report_exception(compile_context,
                             "bytecode serialization failed");
            goto cleanup;
        }
        if (compiled_wire_size != input_wire_size ||
            memcmp(compiled_wire, input_wire, input_wire_size) != 0) {
            fprintf(stderr,
                    "%s compiler wire did not match its pinned BC5 vector\n",
                    test->label);
            goto cleanup;
        }
        printf("%s-source-hex=", test->label);
        for (size_t index = 0; test->source[index] != '\0'; index++)
            printf("%02x", (unsigned char)test->source[index]);
        putchar('\n');
    }

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", test->label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", test->label);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, input_wire, input_wire_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "bytecode read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten_wire = JS_WriteObject(context, &rewritten_wire_size, loaded,
                                    JS_WRITE_OBJ_BYTECODE);
    if (!rewritten_wire) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "bytecode rewrite failed");
        goto cleanup;
    }
    if (rewritten_wire_size != canonical_wire_size ||
        memcmp(rewritten_wire, canonical_wire, canonical_wire_size) != 0) {
        fprintf(stderr,
                "%s rewrite did not match its canonical BC5 vector\n",
                test->label);
        goto cleanup;
    }

    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "fresh-runtime evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsBigInt(context, result)) {
        fprintf(stderr, "%s did not evaluate to a BigInt\n", test->label);
        goto cleanup;
    }
    actual_tag = JS_VALUE_GET_TAG(result);
    if (actual_tag != test->expected_tag) {
        fprintf(stderr, "%s evaluated with tag %d, expected %d\n",
                test->label, actual_tag, test->expected_tag);
        goto cleanup;
    }
    actual_decimal = JS_ToCString(context, result);
    if (!actual_decimal) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "BigInt string conversion failed");
        goto cleanup;
    }
    if (strcmp(actual_decimal, test->expected_decimal) != 0) {
        fprintf(stderr, "%s evaluated to %sn, expected %sn\n",
                test->label, actual_decimal, test->expected_decimal);
        goto cleanup;
    }

    rewrite_is_identity = input_wire_size == canonical_wire_size &&
                          memcmp(input_wire, canonical_wire,
                                 input_wire_size) == 0;
    printf("%s-hex=", test->label);
    for (size_t index = 0; index < input_wire_size; index++)
        printf("%02x", input_wire[index]);
    putchar('\n');
    printf("%s-cohort=", test->label);
    switch (test->cohort) {
    case BIGINT_CONSTANT_THREE_INSTRUCTION:
        puts("three-instruction");
        break;
    case BIGINT_CONSTANT_UNARY_NEG:
    case BIGINT_CONSTANT_DIRECT_UNARY_NEG:
        puts("unary-neg");
        break;
    case BIGINT_CONSTANT_UNARY_CHAIN:
        puts("unary-chain");
        break;
    }
    printf("%s-rewrite=%s\n", test->label,
           rewrite_is_identity ? "identity" : "canonical");
    if (!rewrite_is_identity) {
        printf("%s-rewrite-hex=", test->label);
        for (size_t index = 0; index < canonical_wire_size; index++)
            printf("%02x", canonical_wire[index]);
        putchar('\n');
    }
    printf("%s-eval-tag=%d\n", test->label, actual_tag);
    printf("%s-eval=%sn\n", test->label, actual_decimal);
    status = 0;

cleanup:
    if (context) {
        if (actual_decimal)
            JS_FreeCString(context, actual_decimal);
        if (rewritten_wire)
            js_free(context, rewritten_wire);
        JS_FreeValue(context, result);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    if (compiled_wire)
        js_free(compile_context, compiled_wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static const char *unary_result_kind_name(UnaryResultKind kind) {
    switch (kind) {
    case UNARY_RESULT_INT:
        return "Int";
    case UNARY_RESULT_FLOAT64:
        return "Float64";
    case UNARY_RESULT_BOOLEAN:
        return "Boolean";
    case UNARY_RESULT_BIGINT:
        return "BigInt";
    case UNARY_RESULT_STRING:
        return "String";
    case UNARY_RESULT_EXCEPTION:
        return "Exception";
    }
    return NULL;
}

static int expect_unary_case(const UnaryCase *test) {
    uint8_t wire[STRING_SCALAR_MAX_WIRE_SIZE];
    size_t wire_size = 0;
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    uint8_t *rewritten_wire = NULL;
    size_t rewritten_wire_size = 0;
    const char *actual_text = NULL;
    const char *kind_name;
    double actual_float = 0;
    uint64_t actual_bits = 0;
    int actual_integer = 0;
    int actual_boolean = -1;
    int actual_tag = -1;
    int status = -1;

    kind_name = test ? unary_result_kind_name(test->expected.kind) : NULL;
    if (!test || !test->label || !test->cohort || !test->ops || !kind_name ||
        (test->expected.kind == UNARY_RESULT_INT &&
         test->expected.tag != JS_TAG_INT) ||
        (test->expected.kind == UNARY_RESULT_FLOAT64 &&
         test->expected.tag != JS_TAG_FLOAT64) ||
        (test->expected.kind == UNARY_RESULT_BOOLEAN &&
         test->expected.tag != JS_TAG_BOOL) ||
        (test->expected.kind == UNARY_RESULT_BIGINT &&
         test->expected.tag != JS_TAG_SHORT_BIG_INT &&
         test->expected.tag != JS_TAG_BIG_INT) ||
        (test->expected.kind == UNARY_RESULT_STRING &&
         test->expected.tag != JS_TAG_STRING) ||
        ((test->expected.kind == UNARY_RESULT_BIGINT ||
          test->expected.kind == UNARY_RESULT_STRING) &&
         !test->expected.text) ||
        (test->expected.kind == UNARY_RESULT_EXCEPTION &&
         (!test->expected.exception_class || !test->expected.text)) ||
        build_scalar_encoding_wire(&test->input, wire, sizeof(wire),
                                   &wire_size)) {
        fprintf(stderr, "%s has an invalid unary oracle definition\n",
                test && test->label ? test->label : "<unnamed>");
        goto cleanup;
    }

    runtime = JS_NewRuntime();
    if (!runtime) {
        fprintf(stderr, "%s runtime allocation failed\n", test->label);
        goto cleanup;
    }
    context = JS_NewContext(runtime);
    if (!context) {
        fprintf(stderr, "%s context allocation failed\n", test->label);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, wire, wire_size, JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "unary bytecode read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten_wire = JS_WriteObject(context, &rewritten_wire_size, loaded,
                                    JS_WRITE_OBJ_BYTECODE);
    if (!rewritten_wire) {
        fprintf(stderr, "%s ", test->label);
        report_exception(context, "unary bytecode rewrite failed");
        goto cleanup;
    }
    if (rewritten_wire_size != wire_size ||
        memcmp(rewritten_wire, wire, wire_size) != 0) {
        fprintf(stderr, "%s did not preserve its unary BC5 wire\n",
                test->label);
        goto cleanup;
    }

    result = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (test->expected.kind == UNARY_RESULT_EXCEPTION) {
        if (!JS_IsException(result)) {
            fprintf(stderr, "%s did not throw during evaluation\n",
                    test->label);
            goto cleanup;
        }
        exception = JS_GetException(context);
        result = JS_UNDEFINED;
        if (expect_exception_fields(context, test->label, exception,
                                    test->expected.exception_class,
                                    test->expected.text))
            goto cleanup;
    } else {
        if (JS_IsException(result)) {
            fprintf(stderr, "%s ", test->label);
            report_exception(context, "unary bytecode evaluation failed");
            result = JS_UNDEFINED;
            goto cleanup;
        }
        actual_tag = JS_VALUE_GET_TAG(result);
        if (actual_tag != test->expected.tag) {
            fprintf(stderr, "%s evaluated with tag %d, expected %d\n",
                    test->label, actual_tag, test->expected.tag);
            goto cleanup;
        }

        switch (test->expected.kind) {
        case UNARY_RESULT_INT:
            actual_integer = JS_VALUE_GET_INT(result);
            if (actual_integer != test->expected.integer) {
                fprintf(stderr, "%s evaluated to %d, expected %" PRId64 "\n",
                        test->label, actual_integer,
                        test->expected.integer);
                goto cleanup;
            }
            break;
        case UNARY_RESULT_FLOAT64:
            if (JS_ToFloat64(context, &actual_float, result) < 0) {
                fprintf(stderr, "%s ", test->label);
                report_exception(context, "Float64 conversion failed");
                goto cleanup;
            }
            memcpy(&actual_bits, &actual_float, sizeof(actual_bits));
            if (actual_bits != test->expected.bits) {
                fprintf(stderr,
                        "%s evaluated to Float64 bits %016" PRIx64
                        ", expected %016" PRIx64 "\n",
                        test->label, actual_bits, test->expected.bits);
                goto cleanup;
            }
            break;
        case UNARY_RESULT_BOOLEAN:
            actual_boolean = JS_ToBool(context, result);
            if (actual_boolean != test->expected.integer) {
                fprintf(stderr, "%s evaluated to %s, expected %s\n",
                        test->label, actual_boolean ? "true" : "false",
                        test->expected.integer ? "true" : "false");
                goto cleanup;
            }
            break;
        case UNARY_RESULT_BIGINT:
            if (!JS_IsBigInt(context, result)) {
                fprintf(stderr, "%s did not evaluate to a BigInt\n",
                        test->label);
                goto cleanup;
            }
            actual_text = JS_ToCString(context, result);
            if (!actual_text) {
                fprintf(stderr, "%s ", test->label);
                report_exception(context, "BigInt string conversion failed");
                goto cleanup;
            }
            if (strcmp(actual_text, test->expected.text) != 0) {
                fprintf(stderr, "%s evaluated to %sn, expected %sn\n",
                        test->label, actual_text, test->expected.text);
                goto cleanup;
            }
            break;
        case UNARY_RESULT_STRING:
            if (!JS_IsString(result)) {
                fprintf(stderr, "%s did not evaluate to a String\n",
                        test->label);
                goto cleanup;
            }
            actual_text = JS_ToCString(context, result);
            if (!actual_text) {
                fprintf(stderr, "%s ", test->label);
                report_exception(context, "String conversion failed");
                goto cleanup;
            }
            if (strcmp(actual_text, test->expected.text) != 0) {
                fprintf(stderr, "%s evaluated to %s, expected %s\n",
                        test->label, actual_text, test->expected.text);
                goto cleanup;
            }
            break;
        case UNARY_RESULT_EXCEPTION:
            break;
        }
    }

    printf("%s-hex=", test->label);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    printf("%s-cohort=", test->label);
    if (strncmp(test->cohort, "outside-", strlen("outside-")) != 0)
        fputs("compatible-", stdout);
    puts(test->cohort);
    printf("%s-ops=%s\n", test->label, test->ops);
    printf("%s-rewrite=identity\n", test->label);
    printf("%s-eval-kind=%s\n", test->label, kind_name);
    if (test->expected.kind == UNARY_RESULT_EXCEPTION) {
        printf("%s-eval-class=%s\n", test->label,
               test->expected.exception_class);
        printf("%s-eval-message=%s\n", test->label,
               test->expected.text);
    } else {
        printf("%s-eval-tag=%d\n", test->label, actual_tag);
        switch (test->expected.kind) {
        case UNARY_RESULT_INT:
            printf("%s-eval=%d\n", test->label, actual_integer);
            break;
        case UNARY_RESULT_FLOAT64:
            printf("%s-eval-bits=%016" PRIx64 "\n",
                   test->label, actual_bits);
            break;
        case UNARY_RESULT_BOOLEAN:
            printf("%s-eval=%s\n", test->label,
                   actual_boolean ? "true" : "false");
            break;
        case UNARY_RESULT_BIGINT:
            printf("%s-eval=%sn\n", test->label, actual_text);
            break;
        case UNARY_RESULT_STRING:
            printf("%s-eval=%s\n", test->label, actual_text);
            break;
        case UNARY_RESULT_EXCEPTION:
            break;
        }
    }
    status = 0;

cleanup:
    if (context) {
        if (actual_text)
            JS_FreeCString(context, actual_text);
        if (rewritten_wire)
            js_free(context, rewritten_wire);
        JS_FreeValue(context, exception);
        JS_FreeValue(context, result);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static const UnaryCase *find_unary_case(const char *label) {
    for (size_t index = 0;
         index < sizeof(unary_cases) / sizeof(unary_cases[0]); index++) {
        if (strcmp(unary_cases[index].label, label) == 0)
            return &unary_cases[index];
    }
    return NULL;
}

static int expect_unary_typeof_identity_matrix(void) {
    const UnaryCase *number = find_unary_case("unary-typeof-int-number");
    JSRuntime *first_runtime = NULL;
    JSContext *first_context = NULL;
    JSRuntime *second_runtime = NULL;
    JSContext *second_context = NULL;
    JSValue function = JS_UNDEFINED;
    JSValue reload_function = JS_UNDEFINED;
    JSValue literal_function = JS_UNDEFINED;
    JSValue cross_runtime_function = JS_UNDEFINED;
    JSValue first = JS_UNDEFINED;
    JSValue repeat = JS_UNDEFINED;
    JSValue reload = JS_UNDEFINED;
    JSValue literal = JS_UNDEFINED;
    JSValue cross_runtime = JS_UNDEFINED;
    const char *first_text = NULL;
    const char *cross_runtime_text = NULL;
    int repeat_same;
    int reload_same;
    int literal_same;
    int cross_runtime_distinct;
    int status = -1;

    if (!number) {
        fprintf(stderr, "typeof identity matrix lost its number case\n");
        goto cleanup;
    }
    first_runtime = JS_NewRuntime();
    if (!first_runtime) {
        fprintf(stderr, "typeof identity runtime allocation failed\n");
        goto cleanup;
    }
    first_context = JS_NewContext(first_runtime);
    if (!first_context) {
        fprintf(stderr, "typeof identity context allocation failed\n");
        goto cleanup;
    }
    second_runtime = JS_NewRuntime();
    if (!second_runtime) {
        fprintf(stderr, "typeof cross-runtime allocation failed\n");
        goto cleanup;
    }
    second_context = JS_NewContext(second_runtime);
    if (!second_context) {
        fprintf(stderr, "typeof cross-runtime context allocation failed\n");
        goto cleanup;
    }

    if (read_scalar_encoding_function(first_context, &number->input,
                                      number->label, &function) ||
        read_scalar_encoding_function(first_context, &number->input,
                                      number->label, &reload_function) ||
        read_scalar_encoding_function(first_context,
                                      &unary_typeof_number_atom_literal,
                                      "unary-typeof-number-atom-literal",
                                      &literal_function) ||
        read_scalar_encoding_function(second_context, &number->input,
                                      number->label,
                                      &cross_runtime_function) ||
        eval_string_function(first_context, function, &first) ||
        eval_string_function(first_context, function, &repeat) ||
        eval_string_function(first_context, reload_function, &reload) ||
        eval_string_function(first_context, literal_function, &literal) ||
        eval_string_function(second_context, cross_runtime_function,
                             &cross_runtime))
        goto cleanup;

    first_text = JS_ToCString(first_context, first);
    cross_runtime_text = JS_ToCString(second_context, cross_runtime);
    if (!first_text || !cross_runtime_text) {
        fprintf(stderr, "typeof identity String conversion failed\n");
        goto cleanup;
    }
    if (strcmp(first_text, "number") != 0 ||
        strcmp(cross_runtime_text, "number") != 0) {
        fprintf(stderr, "typeof identity matrix did not produce number\n");
        goto cleanup;
    }

    repeat_same = JS_VALUE_GET_PTR(first) == JS_VALUE_GET_PTR(repeat);
    reload_same = JS_VALUE_GET_PTR(first) == JS_VALUE_GET_PTR(reload);
    literal_same = JS_VALUE_GET_PTR(first) == JS_VALUE_GET_PTR(literal);
    cross_runtime_distinct =
        JS_VALUE_GET_PTR(first) != JS_VALUE_GET_PTR(cross_runtime);
    if (!repeat_same || !reload_same || !literal_same ||
        !cross_runtime_distinct) {
        fprintf(stderr,
                "typeof representation identity matrix did not match pinned QuickJS\n");
        goto cleanup;
    }

    puts("unary-typeof-identity-repeat=same");
    puts("unary-typeof-identity-reload=same");
    puts("unary-typeof-identity-atom-literal=same");
    puts("unary-typeof-identity-cross-runtime=distinct");
    status = 0;

cleanup:
    if (second_context) {
        if (cross_runtime_text)
            JS_FreeCString(second_context, cross_runtime_text);
        JS_FreeValue(second_context, cross_runtime);
        JS_FreeValue(second_context, cross_runtime_function);
        JS_FreeContext(second_context);
    }
    if (second_runtime)
        JS_FreeRuntime(second_runtime);
    if (first_context) {
        if (first_text)
            JS_FreeCString(first_context, first_text);
        JS_FreeValue(first_context, literal);
        JS_FreeValue(first_context, reload);
        JS_FreeValue(first_context, repeat);
        JS_FreeValue(first_context, first);
        JS_FreeValue(first_context, literal_function);
        JS_FreeValue(first_context, reload_function);
        JS_FreeValue(first_context, function);
        JS_FreeContext(first_context);
    }
    if (first_runtime)
        JS_FreeRuntime(first_runtime);
    return status;
}

int main(void) {
    static const char source[] = "42;";
    JSRuntime *compile_runtime = NULL;
    JSContext *compile_context = NULL;
    JSRuntime *eval_runtime = NULL;
    JSContext *eval_context = NULL;
    JSValue compiled = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    uint8_t *bytecode = NULL;
    size_t bytecode_size = 0;
    double evaluated = 0;
    uint8_t wrong_version[sizeof(expected_bytecode)];
    int status = 1;

    compile_runtime = JS_NewRuntime();
    if (!compile_runtime) {
        fputs("compile runtime allocation failed\n", stderr);
        goto cleanup;
    }
    compile_context = JS_NewContext(compile_runtime);
    if (!compile_context) {
        fputs("compile context allocation failed\n", stderr);
        goto cleanup;
    }

    JS_SetStripInfo(compile_runtime, JS_STRIP_DEBUG);
    compiled = JS_Eval(compile_context, source, strlen(source),
                       "return-42.js",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "compile failed");
        goto cleanup;
    }

    bytecode = JS_WriteObject(compile_context, &bytecode_size, compiled,
                              JS_WRITE_OBJ_BYTECODE);
    if (!bytecode) {
        report_exception(compile_context, "bytecode serialization failed");
        goto cleanup;
    }
    if (bytecode_size != sizeof(expected_bytecode) ||
        memcmp(bytecode, expected_bytecode, sizeof(expected_bytecode)) != 0) {
        fputs("serialized bytecode did not match the pinned BC5 blob\n", stderr);
        goto cleanup;
    }

    eval_runtime = JS_NewRuntime();
    if (!eval_runtime) {
        fputs("evaluation runtime allocation failed\n", stderr);
        goto cleanup;
    }
    eval_context = JS_NewContext(eval_runtime);
    if (!eval_context) {
        fputs("evaluation context allocation failed\n", stderr);
        goto cleanup;
    }

    loaded = JS_ReadObject(eval_context, bytecode, bytecode_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(eval_context, "bytecode deserialization failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_EvalFunction(eval_context, loaded);
    loaded = JS_UNDEFINED; /* JS_EvalFunction consumes its argument. */
    if (JS_IsException(result)) {
        report_exception(eval_context, "fresh-runtime evaluation failed");
        result = JS_UNDEFINED;
        goto cleanup;
    }
    if (!JS_IsNumber(result) || JS_ToFloat64(eval_context, &evaluated, result) < 0) {
        fputs("fresh-runtime evaluation did not return a number\n", stderr);
        goto cleanup;
    }
    if (evaluated != 42.0) {
        fprintf(stderr, "fresh-runtime evaluation returned %.17g, expected 42\n",
                evaluated);
        goto cleanup;
    }

    puts("quickjs=2026-06-04");
    puts("source-hex=34323b");
    printf("strip-flags=%d\n", JS_STRIP_DEBUG);
    printf("bytecode-size=%zu\n", bytecode_size);
    fputs("bytecode-hex=", stdout);
    for (size_t index = 0; index < bytecode_size; index++)
        printf("%02x", bytecode[index]);
    putchar('\n');
    printf("fresh-eval=%.17g\n", evaluated);

    if (expect_ordinary_leaf())
        goto cleanup;

    printf("canonical-scalar-integer-count=%zu\n",
           sizeof(canonical_scalar_integers) /
           sizeof(canonical_scalar_integers[0]));
    for (size_t index = 0;
         index < sizeof(canonical_scalar_integers) /
                 sizeof(canonical_scalar_integers[0]);
         index++) {
        if (expect_compiled_scalar(
                compile_context, &canonical_scalar_integers[index]))
            goto cleanup;
    }
    printf("canonical-scalar-value-count=%zu\n",
           sizeof(canonical_scalar_values) /
           sizeof(canonical_scalar_values[0]));
    for (size_t index = 0;
         index < sizeof(canonical_scalar_values) /
                 sizeof(canonical_scalar_values[0]);
         index++) {
        if (expect_compiled_scalar(
                compile_context, &canonical_scalar_values[index]))
            goto cleanup;
    }
    printf("canonical-string-scalar-count=%zu\n",
           sizeof(canonical_string_scalars) /
           sizeof(canonical_string_scalars[0]));
    for (size_t index = 0;
         index < sizeof(canonical_string_scalars) /
                 sizeof(canonical_string_scalars[0]);
         index++) {
        if (expect_string_scalar_case(
                compile_context, &canonical_string_scalars[index]))
            goto cleanup;
    }
    printf("canonical-scalar-float64-count=%zu\n",
           sizeof(canonical_scalar_float64) /
           sizeof(canonical_scalar_float64[0]));
    for (size_t index = 0;
         index < sizeof(canonical_scalar_float64) /
                 sizeof(canonical_scalar_float64[0]);
         index++) {
        if (expect_compiled_scalar(
                compile_context, &canonical_scalar_float64[index]))
            goto cleanup;
    }
    printf("bigint-constant-case-count=%zu\n",
           sizeof(bigint_constant_cases) /
           sizeof(bigint_constant_cases[0]));
    for (size_t index = 0;
         index < sizeof(bigint_constant_cases) /
                 sizeof(bigint_constant_cases[0]);
         index++) {
        if (expect_bigint_constant_case(
                compile_context, &bigint_constant_cases[index]))
            goto cleanup;
    }
    printf("unary-case-count=%zu\n",
           sizeof(unary_cases) / sizeof(unary_cases[0]));
    for (size_t index = 0;
         index < sizeof(unary_cases) / sizeof(unary_cases[0]); index++) {
        if (expect_unary_case(&unary_cases[index]))
            goto cleanup;
    }
    if (expect_unary_typeof_identity_matrix())
        goto cleanup;
    printf("compatible-scalar-integer-count=%zu\n",
           sizeof(compatible_scalar_integers) /
           sizeof(compatible_scalar_integers[0]));
    for (size_t index = 0;
         index < sizeof(compatible_scalar_integers) /
                 sizeof(compatible_scalar_integers[0]);
         index++) {
        if (expect_compatible_scalar(
                &compatible_scalar_integers[index]))
            goto cleanup;
    }
    printf("compatible-scalar-value-count=%zu\n",
           sizeof(compatible_scalar_values) /
           sizeof(compatible_scalar_values[0]));
    for (size_t index = 0;
         index < sizeof(compatible_scalar_values) /
                 sizeof(compatible_scalar_values[0]);
         index++) {
        if (expect_compatible_scalar(&compatible_scalar_values[index]))
            goto cleanup;
    }
    printf("compatible-scalar-float64-count=%zu\n",
           sizeof(compatible_scalar_float64) /
           sizeof(compatible_scalar_float64[0]));
    for (size_t index = 0;
         index < sizeof(compatible_scalar_float64) /
                 sizeof(compatible_scalar_float64[0]);
         index++) {
        if (expect_compatible_scalar(&compatible_scalar_float64[index]))
            goto cleanup;
    }
    printf("compatible-string-scalar-count=%zu\n",
           sizeof(compatible_string_scalars) /
           sizeof(compatible_string_scalars[0]));
    for (size_t index = 0;
         index < sizeof(compatible_string_scalars) /
                 sizeof(compatible_string_scalars[0]);
         index++) {
        if (expect_string_scalar_case(
                compile_context, &compatible_string_scalars[index]))
            goto cleanup;
    }
    if (expect_string_scalar_identity_matrix())
        goto cleanup;
    printf("outside-string-scalar-count=%zu\n",
           sizeof(outside_string_scalars) /
           sizeof(outside_string_scalars[0]));
    for (size_t index = 0;
         index < sizeof(outside_string_scalars) /
                 sizeof(outside_string_scalars[0]);
         index++) {
        if (expect_string_scalar_case(
                compile_context, &outside_string_scalars[index]))
            goto cleanup;
    }

    if (expect_read_scalar(
            "scope-next-wrap", compatible_scope_next_wrap,
            sizeof(compatible_scope_next_wrap),
            (ScalarExpectation){ .kind = SCALAR_VALUE_NUMBER,
                                 .number = 42 }))
        goto cleanup;
    memcpy(wrong_version, expected_bytecode, sizeof(wrong_version));
    wrong_version[0] = 4;
    if (expect_read_exception("wrong-version", wrong_version,
                              sizeof(wrong_version),
                              "SyntaxError", "invalid version (4 expected=5)"))
        goto cleanup;
    if (expect_read_exception("truncated", expected_bytecode,
                              sizeof(expected_bytecode) - 1,
                              "SyntaxError", "read after the end of the buffer"))
        goto cleanup;
    if (expect_read_exception("malformed-uleb", malformed_uleb,
                              sizeof(malformed_uleb),
                              "SyntaxError", "read after the end of the buffer"))
        goto cleanup;
    if (expect_read_exception("invalid-metadata-atom",
                              invalid_metadata_atom,
                              sizeof(invalid_metadata_atom),
                              "SyntaxError", "invalid atom index (pos=8)"))
        goto cleanup;
    if (expect_read_exception("hard-string-length", hard_string_length,
                              sizeof(hard_string_length),
                              "InternalError", "string too long"))
        goto cleanup;
    if (expect_read_exception("bytecode-length-overflow",
                              bytecode_length_overflow,
                              sizeof(bytecode_length_overflow),
                              "InternalError", "out of memory"))
        goto cleanup;
    if (expect_read_exception("array-buffer-max-below-length",
                              array_buffer_max_below_length,
                              sizeof(array_buffer_max_below_length),
                              "TypeError", "invalid array buffer"))
        goto cleanup;
    if (expect_read_exception("invalid-typed-array-kind",
                              invalid_typed_array_kind,
                              sizeof(invalid_typed_array_kind),
                              "TypeError", "invalid typed array"))
        goto cleanup;
    if (expect_read_exception("uint16-array-unaligned-offset",
                              uint16_array_unaligned_offset,
                              sizeof(uint16_array_unaligned_offset),
                              "RangeError", "invalid offset"))
        goto cleanup;
    if (expect_read_exception("typed-array-view-beyond-backing",
                              typed_array_view_beyond_backing,
                              sizeof(typed_array_view_beyond_backing),
                              "RangeError", "invalid length"))
        goto cleanup;
    status = 0;

cleanup:
    if (eval_context) {
        JS_FreeValue(eval_context, result);
        JS_FreeValue(eval_context, loaded);
        JS_FreeContext(eval_context);
    }
    if (eval_runtime)
        JS_FreeRuntime(eval_runtime);
    if (compile_context) {
        if (bytecode)
            js_free(compile_context, bytecode);
        JS_FreeValue(compile_context, compiled);
        JS_FreeContext(compile_context);
    }
    if (compile_runtime)
        JS_FreeRuntime(compile_runtime);
    return status;
}
