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

enum {
    ORD_ARGS, ORD_VARS, ORD_DEFINED_ARGS, ORD_STACK, ORD_VAR_REFS, ORD_CLOSURES,
    ORD_CPOOL, ORD_CODE, ORD_LOCALS, ORD_METADATA_FIELD_COUNT,
};
typedef struct OrdinaryFunctionMetadata {
    uint16_t flags;
    uint8_t js_mode;
    uint32_t fields[ORD_METADATA_FIELD_COUNT];
    size_t code_offset;
} OrdinaryFunctionMetadata;

typedef struct OrdinaryExpansionCase {
    const char *label, *source;
    uint16_t wire_size;
    uint64_t wire_fnv1a64;
    OrdinaryFunctionMetadata child;
} OrdinaryExpansionCase;

static const OrdinaryExpansionCase ordinary_expansion_cases[] = {
    { "implicit", "(function implicit(){})",
      50, UINT64_C(0xca96655ed845f1b4),
      { 0x0243, 0, { 0, 0, 0, 0, 0, 0, 0, 1, 0 }, 49 } },
    { "primitives",
      "(function primitives(f){'use strict';"
      "return f(void 0)+f(null)+f(false)+f(true)+f('')+f(7n)+f(0.5);})",
      97, UINT64_C(0xac53929d2069cf29),
      { 0x0243, 1, { 1, 0, 1, 3, 0, 0, 1, 33, 1 }, 55 } },
    { "predicates",
      "(function predicates(a,k){'use strict';"
      "if(k===0)return a===void 0;if(k===1)return a===null;"
      "if(k===2)return typeof a==='undefined';"
      "if(k===3)return typeof a==='function';return a==null;})",
      95, UINT64_C(0x5cde810e0e789921),
      { 0x0243, 1, { 2, 0, 2, 2, 0, 0, 0, 36, 2 }, 59 } },
    { "calls", "(function calls(f,a,b,c,d){'use strict';"
      "return f()+f(a)+f(a,b)+f(a,b,c)+f(a,b,c,d);})",
      95, UINT64_C(0x851636a627a5ff92),
      { 0x0243, 1, { 5, 0, 5, 6, 0, 0, 0, 29, 5 }, 66 } },
    { "unary-binary", "(function unary_binary(f,a,b){'use strict';"
      "f(-a);f(+a);f(++a);f(--a);f(~a);f(!a);f(typeof a);"
      "f(a*b);f(a%b);f(a**b);f(a<<b);f(a>>b);f(a>>>b);"
      "f(a<b);f(a<=b);f(a>=b);f(a==b);f(a!=b);f(a!==b);"
      "f(a&b);f(a^b);f(a|b);return 42;})",
      196, UINT64_C(0xa25f0c9a9b3e6e1d),
      { 0x0243, 1, { 3, 0, 3, 3, 0, 0, 0, 130, 3 }, 66 } },
    { "branches-updates",
      "(function branches_updates(f,a,b){'use strict';"
      "f(a++);f(b--);return (a&&b)||(a??b);})",
      99, UINT64_C(0xce13ccf04c7cb95e),
      { 0x0243, 1, { 3, 0, 3, 3, 0, 0, 0, 30, 3 }, 69 } },
    { "wide-if-true",
      "(function wide_if_true(f,a){'use strict';return a||("
      "f()+f()+f()+f()+f()+f()+f()+f()+f()+f()+"
      "f()+f()+f()+f()+f()+f()+f()+f()+f()+f()+"
      "f()+f()+f()+f()+f()+f()+f()+f()+f()+f()+"
      "f()+f()+f()+f()+f()+f()+f()+f()+f()+f()+"
      "f()+f()+f()+f()+f()+f()+f()+f()+f()+f());})",
      220, UINT64_C(0x6cc8033fc7dc4a7c),
      { 0x0243, 1, { 2, 0, 2, 2, 0, 0, 0, 158, 2 }, 62 } },
};
static const OrdinaryExpansionCase ordinary_invocation_cases[] = {
    { "invocation-constructor",
      "(function constructor(F,a,b){'use strict';return new F(a,b);})",
      60, UINT64_C(0xaf44ae48a662bc59),
      { 0x0243, 1, { 3, 0, 3, 4, 0, 0, 0, 8, 3 }, 52 } },
    { "invocation-method",
      "(function method(receiver,a,b){'use strict';"
      "return receiver.m(a,b)+0;})",
      75, UINT64_C(0xd5d8a3b7afa95547),
      { 0x0243, 1, { 3, 0, 3, 4, 0, 0, 0, 14, 3 }, 61 } },
    { "invocation-arrays",
      "(function arrays(a,b,c){'use strict';return [[],[a,b,c]];})",
      72, UINT64_C(0xfe60d3e92788d870),
      { 0x0243, 1, { 3, 0, 3, 4, 0, 0, 0, 13, 3 }, 59 } },
    { "invocation-tail-call",
      "(function(f,a,b){'use strict';return f(a,b);})",
      57, UINT64_C(0x8ff9d2c10c7e2228),
      { 0x0243, 1, { 3, 0, 3, 3, 0, 0, 0, 6, 3 }, 51 } },
    { "invocation-tail-method",
      "(function(receiver,a,b){'use strict';return receiver.m(a,b);})",
      64, UINT64_C(0xfb6d6ed6f8e894bd),
      { 0x0243, 1, { 3, 0, 3, 4, 0, 0, 0, 11, 3 }, 53 } },
};
static const OrdinaryExpansionCase ordinary_apply_cases[] = {
    { "invocation-apply-call",
      "(function(f,t,a){'use strict';return f(...a);})",
      65, UINT64_C(0x2b4c9b53812d0f4e),
      { 0x0243, 1, { 3, 0, 3, 4, 0, 0, 0, 14, 3 }, 51 } },
    { "invocation-apply-construct",
      "(function(f,t,a){'use strict';return new f(...a);})",
      65, UINT64_C(0x135c326513baafe6),
      { 0x0243, 1, { 3, 0, 3, 5, 0, 0, 0, 14, 3 }, 51 } },
};
static const uint8_t ordinary_throw_bytecode[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb,
    0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00,
    0x01, 0x00, 0x00, 0xcf, 0x30,
};
static const uint8_t ordinary_throw_error_natural_bytecode[] = {
    0x05, 0x01, 0x02, 0x78, 0x0c, 0x00, 0x02, 0x00,
    0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
    0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbe,
    0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00,
    0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x0d,
    0x01, 0x00, 0x00, 0x00, 0xb0, 0x5e, 0x00, 0x00,
    0xb3, 0xc7, 0xb4, 0x11, 0x31, 0xf3, 0x00, 0x00,
    0x00, 0x00,
};
static const uint8_t ordinary_throw_error_bytecode[] = {
    0x05, 0x01, 0x02, 0x78, 0x0c, 0x00, 0x02, 0x00,
    0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
    0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbe,
    0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06,
    0x00, 0x31, 0xf3, 0x00, 0x00, 0x00, 0x00,
};
static const uint8_t ordinary_expansion_atom_free_raws[] = {
    6, 7, 9, 10, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32, 41, 105, 138, 139, 140, 141,
    142, 143, 147, 148, 149, 152, 154, 157, 158, 159, 160, 161,
    162, 164, 167, 168, 170, 171, 172, 173, 174, 176, 191, 233,
    240, 241, 242, 243,
};
static const uint8_t ordinary_expansion_call_raws[] = {
    34, 236, 237, 238, 239,
};
static const uint8_t ordinary_invocation_raws[] = { 33, 35, 36, 37, 38, 39 };
static const uint8_t ordinary_invocation_natural_admission_raws[] = {
    33, 35, 38, 39,
};
static const uint8_t ordinary_invocation_manual_admission_raws[] = { 36, 37 };

static const uint8_t ordinary_manual_constructor_wire[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb,
    0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x02, 0x00,
    0x02, 0x04, 0x00, 0x00, 0x00, 0x08, 0x02, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xcf,
    0xd0, 0xb4, 0xb5, 0x21, 0x02, 0x00, 0x28,
};
static const uint8_t ordinary_manual_method_wire[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb,
    0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x02, 0x00,
    0x02, 0x03, 0x00, 0x00, 0x00, 0x08, 0x02, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xcf,
    0xd0, 0xbb, 0x2a, 0x24, 0x01, 0x00, 0x28,
};
static const uint8_t ordinary_manual_array_from_zero_wire[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x26, 0x00, 0x00,
    0x28,
};
static const uint8_t ordinary_manual_array_from_multi_wire[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x03, 0x00, 0x00, 0x00, 0x07,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xb4, 0xb5, 0xb6,
    0x26, 0x03, 0x00, 0x28,
};
static const uint8_t ordinary_manual_apply_base_wire[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb,
    0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x03, 0x00,
    0x03, 0x01, 0x00, 0x00, 0x00, 0x02, 0x03, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0xcf, 0x28,
};
static const uint8_t ordinary_manual_tail_method_base_wire[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb,
    0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x04, 0x00,
    0x04, 0x01, 0x00, 0x00, 0x00, 0x02, 0x04, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xcf,
    0x28,
};

typedef struct OrdinaryApplyWireCase {
    const char *label;
    uint16_t magic;
    uint64_t wire_fnv1a64;
} OrdinaryApplyWireCase;

static const OrdinaryApplyWireCase ordinary_apply_wire_cases[] = {
    { "magic-0", 0, UINT64_C(0xa714563baa016e71) },
    { "magic-1", 1, UINT64_C(0x9e6acb3ba5195496) },
    { "magic-2", 2, UINT64_C(0x95c1a03ba031dddb) },
    { "magic-65535", UINT16_MAX, UINT64_C(0xb3e08339fdccd583) },
};
typedef struct OrdinaryStackCase {
    const char *name;
    uint8_t raw, input_count, output_count;
    uint8_t inputs[5];
    uint8_t outputs[6];
    uint8_t lowering_count, lowering[4];
} OrdinaryStackCase;
static const OrdinaryStackCase ordinary_stack_cases[] = {
    { "nip", 15, 2, 1, { 1, 2 }, { 2 }, 1, { 15 } },
    { "nip1", 16, 3, 2, { 1, 2, 3 }, { 2, 3 }, 2, { 24, 15 } },
    { "dup1", 18, 2, 3, { 1, 2 }, { 1, 1, 2 }, 1, { 18 } },
    { "dup2", 19, 2, 4, { 1, 2 }, { 1, 2, 1, 2 }, 3, { 18, 17, 24 } },
    { "dup3", 20, 3, 6, { 1, 2, 3 }, { 1, 2, 3, 1, 2, 3 }, 1, { 20 } },
    { "insert2", 21, 2, 3, { 1, 2 }, { 2, 1, 2 }, 1, { 21 } },
    { "insert3", 22, 3, 4, { 1, 2, 3 }, { 3, 1, 2, 3 }, 1, { 22 } },
    { "insert4", 23, 4, 5, { 1, 2, 3, 4 }, { 4, 1, 2, 3, 4 }, 1, { 23 } },
    { "perm3", 24, 3, 3, { 1, 2, 3 }, { 2, 1, 3 }, 1, { 24 } },
    { "perm4", 25, 4, 4, { 1, 2, 3, 4 }, { 3, 1, 2, 4 }, 1, { 25 } },
    { "perm5", 26, 5, 5, { 1, 2, 3, 4, 5 }, { 4, 1, 2, 3, 5 }, 1, { 26 } },
    { "swap", 27, 2, 2, { 1, 2 }, { 2, 1 }, 1, { 27 } },
    { "swap2", 28, 4, 4, { 1, 2, 3, 4 }, { 3, 4, 1, 2 }, 2, { 31, 31 } },
    { "rot3l", 29, 3, 3, { 1, 2, 3 }, { 2, 3, 1 }, 2, { 24, 27 } },
    { "rot3r", 30, 3, 3, { 1, 2, 3 }, { 3, 1, 2 }, 2, { 27, 24 } },
    { "rot4l", 31, 4, 4, { 1, 2, 3, 4 }, { 2, 3, 4, 1 }, 1, { 31 } },
    { "rot5l", 32, 5, 5, { 1, 2, 3, 4, 5 }, { 2, 3, 4, 5, 1 }, 4, { 25, 25, 26, 31 } },
};

_Static_assert(sizeof(ordinary_expansion_atom_free_raws) == 57, "57 atom-free rows");
_Static_assert(sizeof(ordinary_expansion_call_raws) == 5, "five plain-call rows");
_Static_assert(sizeof(ordinary_invocation_cases) /
                   sizeof(ordinary_invocation_cases[0]) == 5,
               "five compiler-natural invocation cases");
_Static_assert(sizeof(ordinary_apply_cases) /
                   sizeof(ordinary_apply_cases[0]) == 2,
               "two compiler-natural apply cases");
_Static_assert(sizeof(ordinary_throw_bytecode) == 45,
               "ordinary throw oracle must retain its pinned 45-byte wire");
_Static_assert(sizeof(ordinary_throw_error_natural_bytecode) == 58,
               "natural throw_error oracle must retain its pinned 58-byte wire");
_Static_assert(sizeof(ordinary_throw_error_bytecode) == 47,
               "manual throw_error oracle must retain its pinned 47-byte wire");
_Static_assert(sizeof(ordinary_invocation_raws) == 6,
               "six admitted invocation rows");
_Static_assert(sizeof(ordinary_manual_constructor_wire) == 55,
               "manual constructor wire must remain 55 bytes");
_Static_assert(sizeof(ordinary_manual_method_wire) == 55,
               "manual method wire must remain 55 bytes");
_Static_assert(sizeof(ordinary_manual_array_from_zero_wire) == 25,
               "manual empty array_from wire must remain 25 bytes");
_Static_assert(sizeof(ordinary_manual_array_from_multi_wire) == 28,
               "manual multi array_from wire must remain 28 bytes");
_Static_assert(sizeof(ordinary_manual_apply_base_wire) == 53,
               "manual apply base wire must remain 53 bytes");
_Static_assert(sizeof(ordinary_manual_tail_method_base_wire) == 57,
               "manual tail-method base wire must remain 57 bytes");
_Static_assert(sizeof(ordinary_apply_wire_cases) /
                   sizeof(ordinary_apply_wire_cases[0]) == 4,
               "four manual apply magic rows");
_Static_assert(sizeof(ordinary_stack_cases) / sizeof(ordinary_stack_cases[0]) == 17,
               "17 rare stack rows");
static const uint8_t scalar_prefix[] = {
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00,
};

static const uint8_t scalar_local[] = {
    0x01, 0x00, 0x00, 0x00, 0x00,
};

#define SCALAR_MAX_CODE_SIZE 24
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
    {
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

static size_t ordinary_opcode_size(uint8_t raw) {
    switch (raw) {
    case 33: case 34: case 35: case 36: case 37: case 38: case 39: case 88:
    case 94:
        return 3;
    case 49:
        return 6;
    case 62: case 105: case 176:
        return 5;
    case 187: case 189: case 232: case 233:
        return 2;
    default:
        return raw < 244 ? 1 : 0;
    }
}

static int ordinary_collect_opcodes(const uint8_t *code,
                                    size_t code_size,
                                    uint8_t present[256]) {
    size_t offset = 0;
    while (offset < code_size) {
        size_t size = ordinary_opcode_size(code[offset]);
        if (size == 0 || size > code_size - offset)
            return -1;
        present[code[offset]] = 1;
        offset += size;
    }
    return 0;
}

static int ordinary_terminal_opcode(const uint8_t *code,
                                    size_t code_size,
                                    uint8_t *terminal_raw) {
    size_t offset = 0;
    if (code_size == 0)
        return -1;
    while (offset < code_size) {
        size_t size = ordinary_opcode_size(code[offset]);
        if (size == 0 || size > code_size - offset)
            return -1;
        *terminal_raw = code[offset];
        offset += size;
    }
    return 0;
}

static void ordinary_print_raw_set(const char *label,
                                   const uint8_t present[256]) {
    int first = 1;
    printf("%s=", label);
    for (unsigned raw = 0; raw < 256; raw++) {
        if (present[raw]) {
            printf("%s%u", first ? "" : ",", raw);
            first = 0;
        }
    }
    putchar('\n');
}

static int ordinary_build_raw_set(const char *label, const uint8_t *raws,
                                  size_t count, uint8_t present[256]) {
    for (size_t index = 0; index < count; index++) {
        if (present[raws[index]]) {
            fprintf(stderr, "%s contains duplicate raw %u\n",
                    label, raws[index]);
            return -1;
        }
        present[raws[index]] = 1;
    }
    return 0;
}

static uint64_t ordinary_fnv1a64(const uint8_t *bytes, size_t length) {
    uint64_t hash = UINT64_C(14695981039346656037);
    for (size_t index = 0; index < length; index++)
        hash = (hash ^ bytes[index]) * UINT64_C(1099511628211);
    return hash;
}

typedef struct OrdinaryWireCursor {
    const uint8_t *wire;
    size_t size;
    size_t offset;
    int failed;
} OrdinaryWireCursor;

static void ordinary_wire_skip(OrdinaryWireCursor *cursor, size_t count) {
    if (cursor->failed || count > cursor->size - cursor->offset)
        cursor->failed = 1;
    else
        cursor->offset += count;
}

static uint8_t ordinary_wire_u8(OrdinaryWireCursor *cursor) {
    if (cursor->failed || cursor->offset == cursor->size) {
        cursor->failed = 1;
        return 0;
    }
    return cursor->wire[cursor->offset++];
}

static uint32_t ordinary_wire_uleb(OrdinaryWireCursor *cursor) {
    uint32_t result = 0;
    for (unsigned shift = 0; shift <= 28; shift += 7) {
        uint8_t byte = ordinary_wire_u8(cursor);
        if (cursor->failed || (shift == 28 && (byte & 0xf0) != 0)) {
            cursor->failed = 1;
            return 0;
        }
        result |= (uint32_t)(byte & 0x7f) << shift;
        if ((byte & 0x80) == 0)
            return result;
    }
    cursor->failed = 1;
    return 0;
}

static void ordinary_wire_function(OrdinaryWireCursor *cursor,
                                   OrdinaryFunctionMetadata *metadata) {
    if (ordinary_wire_u8(cursor) != 12)
        cursor->failed = 1;
    metadata->flags = ordinary_wire_u8(cursor);
    metadata->flags |= (uint16_t)ordinary_wire_u8(cursor) << 8;
    metadata->js_mode = ordinary_wire_u8(cursor);
    (void)ordinary_wire_uleb(cursor); /* func_name */
    for (size_t index = 0; index < ORD_METADATA_FIELD_COUNT; index++)
        metadata->fields[index] = ordinary_wire_uleb(cursor);
    for (uint32_t index = 0; index < metadata->fields[ORD_LOCALS]; index++) {
        (void)ordinary_wire_uleb(cursor); /* name */
        (void)ordinary_wire_uleb(cursor); /* scope_next */
        (void)ordinary_wire_uleb(cursor); /* var_ref_idx */
        (void)ordinary_wire_u8(cursor);   /* flags */
    }
    if (metadata->fields[ORD_CLOSURES] != 0 ||
        (metadata->flags & 0x0400) != 0)
        cursor->failed = 1; /* These stripped, detached functions have neither. */
    metadata->code_offset = cursor->offset;
    ordinary_wire_skip(cursor, metadata->fields[ORD_CODE]);
}

static int ordinary_wire_child_metadata(
    const uint8_t *wire, size_t wire_size,
    OrdinaryFunctionMetadata *child) {
    OrdinaryWireCursor cursor = { wire, wire_size, 0, 0 };
    OrdinaryFunctionMetadata root = { 0 };
    uint8_t version = ordinary_wire_u8(&cursor);
    uint32_t atom_count = ordinary_wire_uleb(&cursor);
    for (uint32_t index = 0; index < atom_count; index++) {
        uint32_t header = ordinary_wire_uleb(&cursor);
        ordinary_wire_skip(&cursor,
                           (size_t)(header >> 1) * (1 + (header & 1)));
    }
    ordinary_wire_function(&cursor, &root);
    ordinary_wire_function(&cursor, child);
    return cursor.failed || version != 5 || root.fields[ORD_CPOOL] != 1 ?
               -1 : 0;
}

static int ordinary_metadata_equal(const OrdinaryFunctionMetadata *left,
                                   const OrdinaryFunctionMetadata *right) {
    return left->flags == right->flags && left->js_mode == right->js_mode &&
           memcmp(left->fields, right->fields, sizeof(left->fields)) == 0 &&
           left->code_offset == right->code_offset;
}

static int build_ordinary_stack_wire(const OrdinaryStackCase *test,
                                     const uint8_t *operations,
                                     size_t operation_count,
                                     size_t output_index,
                                     uint8_t wire[SCALAR_MAX_WIRE_SIZE],
                                     size_t *wire_size) {
    size_t drop_count = test->output_count - output_index - 1;
    ScalarCase scalar = { 0 };
    size_t offset = 0;
    scalar.code_size = (size_t)test->input_count * 2 + operation_count +
                       drop_count + 2;
    if (output_index >= test->output_count ||
        scalar.code_size > SCALAR_MAX_CODE_SIZE)
        return -1;
    for (size_t index = 0; index < test->input_count; index++) {
        scalar.code[offset++] = 187;
        scalar.code[offset++] = test->inputs[index];
    }
    for (size_t index = 0; index < operation_count; index++)
        scalar.code[offset++] = operations[index];
    for (size_t index = 0; index < drop_count; index++)
        scalar.code[offset++] = 14;
    scalar.code[offset++] = 203;
    scalar.code[offset++] = 40;
    if (offset != scalar.code_size ||
        build_scalar_wire(&scalar, wire, SCALAR_MAX_WIRE_SIZE, wire_size))
        return -1;
    wire[11] = test->input_count > test->output_count ?
                   test->input_count : test->output_count;
    return 0;
}

static int expect_ordinary_stack_case(const OrdinaryStackCase *test) {
    uint8_t wire[SCALAR_MAX_WIRE_SIZE];
    size_t wire_size;
    char label[80];
    for (size_t index = 0; index < test->output_count; index++) {
        snprintf(label, sizeof(label), "ordinary-stack-raw-%u-native-%zu",
                 test->raw, index);
        if (build_ordinary_stack_wire(test, &test->raw, 1, index,
                                      wire, &wire_size) ||
            expect_read_scalar(label, wire, wire_size,
                               (ScalarExpectation)
                                   EXPECT_NUMBER(test->outputs[index])))
            return -1;
        if (test->lowering_count > 1) {
            snprintf(label, sizeof(label),
                     "ordinary-stack-raw-%u-lowering-%zu",
                     test->raw, index);
            if (build_ordinary_stack_wire(test, test->lowering,
                                          test->lowering_count, index,
                                          wire, &wire_size) ||
                expect_read_scalar(label, wire, wire_size,
                                   (ScalarExpectation)
                                       EXPECT_NUMBER(test->outputs[index])))
                return -1;
        }
    }
    printf("ordinary-stack-raw-%u-contract=%s,stack:%u->%u,peak:%u,"
           "evidence:authenticated-manual-wire,lowering:",
           test->raw, test->name, test->input_count, test->output_count,
           test->input_count > test->output_count ?
               test->input_count : test->output_count);
    for (size_t index = 0; index < test->lowering_count; index++)
        printf("%s%u", index == 0 ? "" : ",", test->lowering[index]);
    puts(",rewrite:identity,fresh-eval:all-slots");
    return 0;
}

static int ordinary_primitive_index, ordinary_call_index;
static int ordinary_plain_receiver_count, ordinary_sink_mode;
static int ordinary_bigint_tag, ordinary_float_tag, ordinary_float_norm_tag;
static uint64_t ordinary_float_bits;
static char ordinary_sink_sequence[160];
static size_t ordinary_sink_sequence_length;

static JSValue ordinary_callback_error(JSContext *context,
                                       const char *message) {
    return JS_ThrowInternalError(context, "%s", message);
}

static JSValue ordinary_sink(JSContext *context,
                             JSValueConst this_value,
                             int argc,
                             JSValueConst *argv) {
    static const int expected_tags[] = {
        JS_TAG_UNDEFINED, JS_TAG_NULL, JS_TAG_BOOL, JS_TAG_BOOL,
        JS_TAG_STRING, JS_TAG_SHORT_BIG_INT, JS_TAG_FLOAT64,
    };
    static const int expected_args[] = { 11, 22, 33, 44 };
    if (!JS_IsUndefined(this_value))
        return ordinary_callback_error(context, "plain receiver drifted");
    ordinary_plain_receiver_count++;
    if (ordinary_sink_mode == 2) {
        char token[24];
        int token_length;
        if (argc != 1)
            return ordinary_callback_error(context, "generic argc drifted");
        if (JS_VALUE_GET_TAG(argv[0]) == JS_TAG_INT) {
            token_length = snprintf(token, sizeof(token), "%d",
                                    JS_VALUE_GET_INT(argv[0]));
        } else if (JS_VALUE_GET_TAG(argv[0]) == JS_TAG_BOOL) {
            token_length = snprintf(token, sizeof(token), "%s",
                                    JS_VALUE_GET_BOOL(argv[0]) ?
                                        "true" : "false");
        } else if (JS_VALUE_GET_TAG(argv[0]) == JS_TAG_STRING) {
            size_t length;
            const char *text = JS_ToCStringLen(context, &length, argv[0]);
            if (!text)
                return JS_EXCEPTION;
            token_length = snprintf(token, sizeof(token), "%.*s",
                                    (int)length, text);
            JS_FreeCString(context, text);
        } else {
            return ordinary_callback_error(context,
                                           "generic result sequence drifted");
        }
        if (token_length < 0 || (size_t)token_length >= sizeof(token) ||
            ordinary_sink_sequence_length + (ordinary_sink_sequence_length != 0) +
                    (size_t)token_length >= sizeof(ordinary_sink_sequence))
            return ordinary_callback_error(context, "generic result overflow");
        ordinary_sink_sequence_length += snprintf(
            ordinary_sink_sequence + ordinary_sink_sequence_length,
            sizeof(ordinary_sink_sequence) - ordinary_sink_sequence_length,
            "%s%s", ordinary_sink_sequence_length == 0 ? "" : ",", token);
        return JS_NewInt32(context, 0);
    }
    if (ordinary_sink_mode == 1) {
        if (ordinary_call_index >= 5 || argc != ordinary_call_index)
            return ordinary_callback_error(context, "plain call argc drifted");
        for (int index = 0; index < argc; index++) {
            if (index >= 4 || JS_VALUE_GET_TAG(argv[index]) != JS_TAG_INT ||
                JS_VALUE_GET_INT(argv[index]) != expected_args[index])
                return ordinary_callback_error(context,
                                               "plain call argument drifted");
        }
        ordinary_call_index++;
        return JS_NewInt32(context, argc);
    }
    if (argc != 1 || ordinary_primitive_index >= 7 ||
        JS_VALUE_GET_NORM_TAG(argv[0]) !=
            expected_tags[ordinary_primitive_index])
        return ordinary_callback_error(context, "primitive shape drifted");
    if (ordinary_primitive_index == 2 && JS_VALUE_GET_BOOL(argv[0]) != 0)
        return ordinary_callback_error(context, "false payload drifted");
    if (ordinary_primitive_index == 3 && JS_VALUE_GET_BOOL(argv[0]) != 1)
        return ordinary_callback_error(context, "true payload drifted");
    if (ordinary_primitive_index == 4) {
        size_t length;
        const char *text = JS_ToCStringLen(context, &length, argv[0]);
        if (!text)
            return JS_EXCEPTION;
        JS_FreeCString(context, text);
        if (length != 0)
            return ordinary_callback_error(context,
                                           "empty String payload drifted");
    } else if (ordinary_primitive_index == 5) {
        ordinary_bigint_tag = JS_VALUE_GET_TAG(argv[0]);
        if (ordinary_bigint_tag != JS_TAG_SHORT_BIG_INT ||
            JS_VALUE_GET_SHORT_BIG_INT(argv[0]) != 7)
            return ordinary_callback_error(context, "BigInt payload drifted");
    } else if (ordinary_primitive_index == 6) {
        double number = JS_VALUE_GET_FLOAT64(argv[0]);
        ordinary_float_tag = JS_VALUE_GET_TAG(argv[0]);
        ordinary_float_norm_tag = JS_VALUE_GET_NORM_TAG(argv[0]);
        memcpy(&ordinary_float_bits, &number, sizeof(ordinary_float_bits));
        if (ordinary_float_tag != JS_TAG_FLOAT64 ||
            ordinary_float_bits != UINT64_C(0x3fe0000000000000))
            return ordinary_callback_error(context, "Float64 bits drifted");
    }
    ordinary_primitive_index++;
    return JS_NewInt32(context, 1);
}

static JSValue ordinary_html_dda_call(JSContext *context,
                                      JSValueConst this_value,
                                      int argc,
                                      JSValueConst *argv) {
    return JS_UNDEFINED;
}

static int ordinary_boolean_result(JSContext *context,
                                   JSValueConst function,
                                   JSValueConst value,
                                   int selector) {
    JSValue arguments[2] = {
        JS_DupValue(context, value), JS_NewInt32(context, selector),
    };
    JSValue result = JS_Call(context, function, JS_UNDEFINED, 2, arguments);
    int boolean = -1;
    JS_FreeValue(context, arguments[1]);
    JS_FreeValue(context, arguments[0]);
    if (JS_IsException(result))
        report_exception(context, "ordinary predicate call failed");
    else if (JS_VALUE_GET_TAG(result) != JS_TAG_BOOL)
        fputs("ordinary predicate result was not exact JS_TAG_BOOL\n",
              stderr);
    else
        boolean = JS_VALUE_GET_BOOL(result);
    JS_FreeValue(context, result);
    return boolean;
}

static int ordinary_compile_load_case(JSContext *compile_context,
                                      JSContext *eval_context,
                                      const OrdinaryExpansionCase *test,
                                      JSValue *function,
                                      uint8_t union_raws[256],
                                      uint8_t target_raw) {
    JSValue compiled = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    uint8_t *wire = NULL;
    uint8_t *rewritten = NULL;
    size_t wire_size = 0;
    size_t rewritten_size = 0;
    uint8_t case_raws[256] = { 0 };
    uint8_t terminal_raw = 0;
    OrdinaryFunctionMetadata child = { 0 };
    char raw_label[96];
    int status = -1;
    compiled = JS_Eval(compile_context, test->source, strlen(test->source),
                       test->label,
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "ordinary compile-only failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject(compile_context, &wire_size, compiled,
                          JS_WRITE_OBJ_BYTECODE);
    if (!wire) {
        report_exception(compile_context, "ordinary write failed");
        goto cleanup;
    }
    if (wire_size != test->wire_size ||
        ordinary_fnv1a64(wire, wire_size) != test->wire_fnv1a64 ||
        ordinary_wire_child_metadata(wire, wire_size, &child) ||
        !ordinary_metadata_equal(&child, &test->child) ||
        ordinary_collect_opcodes(wire + child.code_offset,
                                 child.fields[ORD_CODE], case_raws) ||
        (target_raw != 0 && !case_raws[target_raw]) ||
        (target_raw == 39 &&
         (case_raws[33] || case_raws[35] || case_raws[36] ||
          case_raws[37] || !case_raws[38])) ||
        (target_raw != 0 && target_raw != 39 &&
         case_raws[33] + case_raws[35] + case_raws[36] +
                 case_raws[37] + case_raws[38] + case_raws[39] != 1) ||
        ((target_raw == 36 || target_raw == 37) && !case_raws[62]) ||
        ((target_raw == 35 || target_raw == 37) &&
         (case_raws[40] ||
          ordinary_terminal_opcode(wire + child.code_offset,
                                   child.fields[ORD_CODE],
                                   &terminal_raw) ||
          terminal_raw != target_raw))) {
        fprintf(stderr, "%s ordinary BC5 wire/metadata/opcodes drifted\n",
                test->label);
        goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++)
        union_raws[raw] |= case_raws[raw];
    loaded = JS_ReadObject(eval_context, wire, wire_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(eval_context, "ordinary fresh-runtime read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(eval_context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten || rewritten_size != wire_size ||
        memcmp(rewritten, wire, wire_size) != 0) {
        if (!rewritten)
            report_exception(eval_context, "ordinary rewrite failed");
        else
            fprintf(stderr, "%s ordinary rewrite drifted\n", test->label);
        goto cleanup;
    }
    *function = JS_EvalFunction(eval_context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(*function) ||
        !JS_IsFunction(eval_context, *function)) {
        report_exception(eval_context, "ordinary root evaluation failed");
        *function = JS_UNDEFINED;
        goto cleanup;
    }
    printf("ordinary-expansion-%s-wire-size=%zu\n", test->label, wire_size);
    printf("ordinary-expansion-%s-wire-fnv1a64=%016" PRIx64 "\n",
           test->label, ordinary_fnv1a64(wire, wire_size));
    printf("ordinary-expansion-%s-wire-hex=", test->label);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    printf("ordinary-expansion-%s-child-metadata=flags:%04x,js_mode:%u,"
           "args:%" PRIu32 ",vars:%" PRIu32 ",defined_args:%" PRIu32
           ",stack:%" PRIu32 ",var_refs:%" PRIu32 ",closures:%" PRIu32
           ",cpool:%" PRIu32 ",code:%" PRIu32 ",locals:%" PRIu32
           ",code_offset:%zu\n",
           test->label, child.flags, child.js_mode, child.fields[ORD_ARGS],
           child.fields[ORD_VARS], child.fields[ORD_DEFINED_ARGS],
           child.fields[ORD_STACK], child.fields[ORD_VAR_REFS],
           child.fields[ORD_CLOSURES], child.fields[ORD_CPOOL],
           child.fields[ORD_CODE], child.fields[ORD_LOCALS], child.code_offset);
    snprintf(raw_label, sizeof(raw_label),
             "ordinary-expansion-%s-child-raw", test->label);
    ordinary_print_raw_set(raw_label, case_raws);
    printf("ordinary-expansion-%s-rewrite=identity\n", test->label);
    printf("ordinary-expansion-%s-fresh-root=Function\n", test->label);
    status = 0;

cleanup:
    if (rewritten)
        js_free(eval_context, rewritten);
    JS_FreeValue(eval_context, loaded);
    if (wire)
        js_free(compile_context, wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

typedef enum OrdinaryCallResult {
    ORDINARY_CALL_UNDEFINED, ORDINARY_CALL_INT, ORDINARY_CALL_BOOL,
} OrdinaryCallResult;

static int ordinary_expect_call(JSContext *context,
                                JSValueConst function,
                                int argc,
                                JSValueConst *arguments,
                                OrdinaryCallResult kind,
                                int expected,
                                const char *label) {
    JSValue result = JS_Call(context, function, JS_UNDEFINED,
                             argc, arguments);
    int matches = kind == ORDINARY_CALL_UNDEFINED ?
                      JS_IsUndefined(result) :
                  kind == ORDINARY_CALL_INT ?
                      JS_VALUE_GET_TAG(result) == JS_TAG_INT &&
                          JS_VALUE_GET_INT(result) == expected :
                      JS_VALUE_GET_TAG(result) == JS_TAG_BOOL &&
                          JS_VALUE_GET_BOOL(result) == expected;
    if (JS_IsException(result))
        report_exception(context, label);
    else if (!matches)
        fprintf(stderr, "%s result drifted\n", label);
    JS_FreeValue(context, result);
    return matches ? 0 : -1;
}

static int ordinary_expect_i32_call(JSContext *context,
                                    JSValueConst function,
                                    JSValueConst sink,
                                    const int *values,
                                    size_t value_count,
                                    int expected,
                                    const char *label) {
    JSValue arguments[5] = { JS_UNDEFINED };
    int status;
    if (value_count > 4)
        return -1;
    arguments[0] = sink;
    for (size_t index = 0; index < value_count; index++)
        arguments[index + 1] = JS_NewInt32(context, values[index]);
    status = ordinary_expect_call(context, function,
                                  (int)value_count + 1, arguments,
                                  ORDINARY_CALL_INT, expected, label);
    for (size_t index = 0; index < value_count; index++)
        JS_FreeValue(context, arguments[index + 1]);
    return status;
}

static int ordinary_load_manual_invocation(
    JSContext *context, const char *label, const uint8_t *wire,
    size_t wire_size, uint64_t expected_hash,
    const OrdinaryFunctionMetadata *expected_metadata,
    uint8_t target_raw, JSValue *function) {
    JSValue loaded = JS_UNDEFINED;
    uint8_t *rewritten = NULL;
    size_t rewritten_size = 0;
    OrdinaryFunctionMetadata child = { 0 };
    uint8_t raws[256] = { 0 };
    uint8_t terminal_raw = 0;
    char raw_label[96];
    int status = -1;

    if (ordinary_fnv1a64(wire, wire_size) != expected_hash ||
        ordinary_wire_child_metadata(wire, wire_size, &child) ||
        !ordinary_metadata_equal(&child, expected_metadata) ||
        ordinary_collect_opcodes(wire + child.code_offset,
                                 child.fields[ORD_CODE], raws) ||
        !raws[target_raw] ||
        raws[33] + raws[35] + raws[36] + raws[37] + raws[38] +
                raws[39] != 1 ||
        ((target_raw == 36 || target_raw == 37) && raws[62]) ||
        ((target_raw == 35 || target_raw == 37) &&
         (raws[40] ||
          ordinary_terminal_opcode(wire + child.code_offset,
                                   child.fields[ORD_CODE],
                                   &terminal_raw) ||
          terminal_raw != target_raw))) {
        fprintf(stderr, "%s manual invocation wire drifted\n", label);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, wire, wire_size, JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "manual invocation read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten || rewritten_size != wire_size ||
        memcmp(rewritten, wire, wire_size) != 0) {
        if (!rewritten)
            report_exception(context, "manual invocation rewrite failed");
        else
            fprintf(stderr, "%s manual invocation rewrite drifted\n", label);
        goto cleanup;
    }
    *function = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(*function) ||
        !JS_IsFunction(context, *function)) {
        report_exception(context, "manual invocation root evaluation failed");
        *function = JS_UNDEFINED;
        goto cleanup;
    }
    printf("ordinary-invocation-manual-%s-wire-size=%zu\n", label,
           wire_size);
    printf("ordinary-invocation-manual-%s-wire-fnv1a64=%016" PRIx64 "\n",
           label, ordinary_fnv1a64(wire, wire_size));
    printf("ordinary-invocation-manual-%s-wire-hex=", label);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    printf("ordinary-invocation-manual-%s-child-metadata="
           "flags:%04x,js_mode:%u,args:%" PRIu32 ",vars:%" PRIu32
           ",defined_args:%" PRIu32 ",stack:%" PRIu32
           ",var_refs:%" PRIu32 ",closures:%" PRIu32
           ",cpool:%" PRIu32 ",code:%" PRIu32 ",locals:%" PRIu32
           ",code_offset:%zu\n",
           label, child.flags, child.js_mode, child.fields[ORD_ARGS],
           child.fields[ORD_VARS], child.fields[ORD_DEFINED_ARGS],
           child.fields[ORD_STACK], child.fields[ORD_VAR_REFS],
           child.fields[ORD_CLOSURES], child.fields[ORD_CPOOL],
           child.fields[ORD_CODE], child.fields[ORD_LOCALS],
           child.code_offset);
    snprintf(raw_label, sizeof(raw_label),
             "ordinary-invocation-manual-%s-child-raw", label);
    ordinary_print_raw_set(raw_label, raws);
    printf("ordinary-invocation-manual-%s-rewrite=identity\n", label);
    printf("ordinary-invocation-manual-%s-fresh-root=Function\n", label);
    status = 0;

cleanup:
    if (rewritten)
        js_free(context, rewritten);
    JS_FreeValue(context, loaded);
    return status;
}

static int ordinary_eval_manual_array_from(
    JSContext *context, const char *label, const uint8_t *wire,
    size_t wire_size, uint64_t expected_hash, uint8_t expected_stack,
    uint8_t expected_code_size, JSValue *array) {
    JSValue loaded = JS_UNDEFINED;
    uint8_t *rewritten = NULL;
    size_t rewritten_size = 0;
    uint8_t raws[256] = { 0 };
    char raw_label[104];
    int status = -1;

    if (wire_size != (size_t)21 + expected_code_size ||
        ordinary_fnv1a64(wire, wire_size) != expected_hash ||
        wire[11] != expected_stack || wire[14] != 0 ||
        wire[15] != expected_code_size ||
        ordinary_collect_opcodes(wire + 21, expected_code_size, raws) ||
        !raws[38] || raws[33] || raws[36] || raws[35] || raws[37] ||
        raws[39]) {
        fprintf(stderr, "%s manual array_from wire drifted\n", label);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, wire, wire_size, JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "manual array_from read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten || rewritten_size != wire_size ||
        memcmp(rewritten, wire, wire_size) != 0) {
        if (!rewritten)
            report_exception(context, "manual array_from rewrite failed");
        else
            fprintf(stderr, "%s manual array_from rewrite drifted\n", label);
        goto cleanup;
    }
    *array = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(*array) || JS_IsArray(context, *array) != 1) {
        report_exception(context, "manual array_from evaluation failed");
        *array = JS_UNDEFINED;
        goto cleanup;
    }
    printf("ordinary-invocation-manual-array-from-%s-wire-size=%zu\n",
           label, wire_size);
    printf("ordinary-invocation-manual-array-from-%s-wire-fnv1a64="
           "%016" PRIx64 "\n", label, ordinary_fnv1a64(wire, wire_size));
    printf("ordinary-invocation-manual-array-from-%s-wire-hex=", label);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    printf("ordinary-invocation-manual-array-from-%s-root-metadata="
           "flags:0200,js_mode:0,args:0,vars:1,defined_args:0,stack:%u,"
           "var_refs:0,closures:0,cpool:0,code:%u,locals:1,"
           "code_offset:21\n", label, expected_stack, expected_code_size);
    snprintf(raw_label, sizeof(raw_label),
             "ordinary-invocation-manual-array-from-%s-root-raw", label);
    ordinary_print_raw_set(raw_label, raws);
    printf("ordinary-invocation-manual-array-from-%s-rewrite=identity\n",
           label);
    printf("ordinary-invocation-manual-array-from-%s-fresh-eval=Array\n",
           label);
    status = 0;

cleanup:
    if (rewritten)
        js_free(context, rewritten);
    JS_FreeValue(context, loaded);
    return status;
}

#define ORDINARY_APPLY_CODE_OFFSET 51
#define ORDINARY_APPLY_CODE_SIZE 7
#define ORDINARY_APPLY_WIRE_SIZE \
    (ORDINARY_APPLY_CODE_OFFSET + ORDINARY_APPLY_CODE_SIZE)

static int ordinary_expect_manual_apply_base(JSContext *compile_context) {
    static const char source[] =
        "(function(a,b,c){'use strict';return a})";
    JSValue compiled = JS_UNDEFINED;
    uint8_t *wire = NULL;
    size_t wire_size = 0;
    int status = -1;

    compiled = JS_Eval(compile_context, source, strlen(source),
                       "ordinary-apply-base",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "manual apply base compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject(compile_context, &wire_size, compiled,
                          JS_WRITE_OBJ_BYTECODE);
    if (!wire) {
        report_exception(compile_context, "manual apply base write failed");
        goto cleanup;
    }
    if (wire_size != sizeof(ordinary_manual_apply_base_wire) ||
        memcmp(wire, ordinary_manual_apply_base_wire, wire_size) != 0 ||
        ordinary_fnv1a64(wire, wire_size) !=
            UINT64_C(0xa891be862c468350)) {
        fputs("manual apply base compiler wire drifted\n", stderr);
        goto cleanup;
    }
    printf("ordinary-invocation-manual-apply-base-wire-size=%zu\n",
           wire_size);
    printf("ordinary-invocation-manual-apply-base-wire-fnv1a64="
           "%016" PRIx64 "\n", ordinary_fnv1a64(wire, wire_size));
    fputs("ordinary-invocation-manual-apply-base-wire-hex=", stdout);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    puts("ordinary-invocation-manual-apply-base-provenance="
         "compiler-natural-strict-anonymous-three-argument");
    puts("ordinary-invocation-manual-apply-base-transform="
         "replace-code-at-51,get_arg0,get_arg1,get_arg2,raw39-u16,return");
    status = 0;

cleanup:
    if (wire)
        js_free(compile_context, wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static int ordinary_build_manual_apply_wire(
    uint16_t magic, uint8_t wire[ORDINARY_APPLY_WIRE_SIZE]) {
    static const uint8_t code[] = {
        0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28,
    };

    if (ordinary_fnv1a64(ordinary_manual_apply_base_wire,
                         sizeof(ordinary_manual_apply_base_wire)) !=
        UINT64_C(0xa891be862c468350))
        return -1;
    memcpy(wire, ordinary_manual_apply_base_wire,
           ORDINARY_APPLY_CODE_OFFSET);
    wire[33] = 3; /* max_stack_size */
    wire[37] = ORDINARY_APPLY_CODE_SIZE;
    memcpy(wire + ORDINARY_APPLY_CODE_OFFSET, code, sizeof(code));
    wire[ORDINARY_APPLY_CODE_OFFSET + 4] = (uint8_t)magic;
    wire[ORDINARY_APPLY_CODE_OFFSET + 5] = (uint8_t)(magic >> 8);
    return 0;
}

static int ordinary_load_manual_apply(
    JSContext *context, const OrdinaryApplyWireCase *test,
    JSValue *function) {
    static const OrdinaryFunctionMetadata expected_metadata = {
        0x0243, 1, { 3, 0, 3, 3, 0, 0, 0, 7, 3 }, 51,
    };
    uint8_t wire[ORDINARY_APPLY_WIRE_SIZE];
    OrdinaryFunctionMetadata child = { 0 };
    uint8_t raws[256] = { 0 };
    JSValue loaded = JS_UNDEFINED;
    uint8_t *rewritten = NULL;
    size_t rewritten_size = 0;
    char raw_label[112];
    size_t raw_count = 0;
    int status = -1;

    if (!test || ordinary_build_manual_apply_wire(test->magic, wire) ||
        ordinary_fnv1a64(wire, sizeof(wire)) != test->wire_fnv1a64 ||
        ordinary_wire_child_metadata(wire, sizeof(wire), &child) ||
        !ordinary_metadata_equal(&child, &expected_metadata) ||
        ordinary_collect_opcodes(wire + child.code_offset,
                                 child.fields[ORD_CODE], raws)) {
        fputs("manual apply wire construction drifted\n", stderr);
        goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++)
        raw_count += raws[raw] != 0;
    if (raw_count != 5 || !raws[39] || !raws[40] || !raws[207] ||
        !raws[208] || !raws[209] || raws[33] || raws[35] || raws[36] ||
        raws[37] || raws[38]) {
        fputs("manual apply opcode set drifted\n", stderr);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, wire, sizeof(wire),
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "manual apply read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten || rewritten_size != sizeof(wire) ||
        memcmp(rewritten, wire, sizeof(wire)) != 0) {
        if (!rewritten)
            report_exception(context, "manual apply rewrite failed");
        else
            fputs("manual apply rewrite drifted\n", stderr);
        goto cleanup;
    }
    *function = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(*function) ||
        !JS_IsFunction(context, *function)) {
        report_exception(context, "manual apply root evaluation failed");
        *function = JS_UNDEFINED;
        goto cleanup;
    }

    printf("ordinary-invocation-manual-apply-%s-wire-size=%zu\n",
           test->label, sizeof(wire));
    printf("ordinary-invocation-manual-apply-%s-wire-fnv1a64="
           "%016" PRIx64 "\n", test->label,
           ordinary_fnv1a64(wire, sizeof(wire)));
    printf("ordinary-invocation-manual-apply-%s-wire-hex=", test->label);
    for (size_t index = 0; index < sizeof(wire); index++)
        printf("%02x", wire[index]);
    putchar('\n');
    printf("ordinary-invocation-manual-apply-%s-child-metadata="
           "flags:%04x,js_mode:%u,args:%" PRIu32 ",vars:%" PRIu32
           ",defined_args:%" PRIu32 ",stack:%" PRIu32
           ",var_refs:%" PRIu32 ",closures:%" PRIu32
           ",cpool:%" PRIu32 ",code:%" PRIu32 ",locals:%" PRIu32
           ",code_offset:%zu\n",
           test->label, child.flags, child.js_mode, child.fields[ORD_ARGS],
           child.fields[ORD_VARS], child.fields[ORD_DEFINED_ARGS],
           child.fields[ORD_STACK], child.fields[ORD_VAR_REFS],
           child.fields[ORD_CLOSURES], child.fields[ORD_CPOOL],
           child.fields[ORD_CODE], child.fields[ORD_LOCALS],
           child.code_offset);
    snprintf(raw_label, sizeof(raw_label),
             "ordinary-invocation-manual-apply-%s-child-raw",
             test->label);
    ordinary_print_raw_set(raw_label, raws);
    printf("ordinary-invocation-manual-apply-%s-rewrite=identity\n",
           test->label);
    printf("ordinary-invocation-manual-apply-%s-fresh-root=Function\n",
           test->label);
    status = 0;

cleanup:
    if (rewritten)
        js_free(context, rewritten);
    JS_FreeValue(context, loaded);
    return status;
}

#define ORDINARY_TAIL_METHOD_CODE_OFFSET 55
#define ORDINARY_TAIL_METHOD_CODE_SIZE 7
#define ORDINARY_TAIL_METHOD_WIRE_SIZE \
    (ORDINARY_TAIL_METHOD_CODE_OFFSET + ORDINARY_TAIL_METHOD_CODE_SIZE)
_Static_assert(ORDINARY_TAIL_METHOD_WIRE_SIZE == 62,
               "manual tail-method wire must remain 62 bytes");

static int ordinary_expect_manual_tail_method_base(
    JSContext *compile_context) {
    static const char source[] =
        "(function(receiver,f,a,b){'use strict';return receiver})";
    JSValue compiled = JS_UNDEFINED;
    uint8_t *wire = NULL;
    size_t wire_size = 0;
    int status = -1;

    compiled = JS_Eval(compile_context, source, strlen(source),
                       "ordinary-tail-method-base",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context,
                         "manual tail-method base compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject(compile_context, &wire_size, compiled,
                          JS_WRITE_OBJ_BYTECODE);
    if (!wire) {
        report_exception(compile_context,
                         "manual tail-method base write failed");
        goto cleanup;
    }
    if (wire_size != sizeof(ordinary_manual_tail_method_base_wire) ||
        memcmp(wire, ordinary_manual_tail_method_base_wire,
               wire_size) != 0 ||
        ordinary_fnv1a64(wire, wire_size) !=
            UINT64_C(0x31f9978a081891f6)) {
        fputs("manual tail-method base compiler wire drifted\n", stderr);
        goto cleanup;
    }
    printf("ordinary-invocation-manual-tail-method-base-wire-size=%zu\n",
           wire_size);
    printf("ordinary-invocation-manual-tail-method-base-wire-fnv1a64="
           "%016" PRIx64 "\n", ordinary_fnv1a64(wire, wire_size));
    fputs("ordinary-invocation-manual-tail-method-base-wire-hex=", stdout);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    puts("ordinary-invocation-manual-tail-method-base-provenance="
         "compiler-natural-strict-anonymous-four-argument");
    puts("ordinary-invocation-manual-tail-method-base-transform="
         "replace-code-at-55,get_arg0,get_arg1,get_arg2,get_arg3,"
         "raw37-u16-argc2");
    status = 0;

cleanup:
    if (wire)
        js_free(compile_context, wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static int ordinary_build_manual_tail_method_wire(
    uint8_t wire[ORDINARY_TAIL_METHOD_WIRE_SIZE]) {
    static const uint8_t code[] = {
        0xcf, 0xd0, 0xd1, 0xd2, 0x25, 0x02, 0x00,
    };

    if (ordinary_fnv1a64(ordinary_manual_tail_method_base_wire,
                         sizeof(ordinary_manual_tail_method_base_wire)) !=
        UINT64_C(0x31f9978a081891f6))
        return -1;
    memcpy(wire, ordinary_manual_tail_method_base_wire,
           ORDINARY_TAIL_METHOD_CODE_OFFSET);
    wire[33] = 4; /* max_stack_size */
    wire[37] = ORDINARY_TAIL_METHOD_CODE_SIZE;
    memcpy(wire + ORDINARY_TAIL_METHOD_CODE_OFFSET, code, sizeof(code));
    return ordinary_fnv1a64(wire, ORDINARY_TAIL_METHOD_WIRE_SIZE) ==
                   UINT64_C(0xe87d54c0a2a140ca) ?
               0 : -1;
}

static int ordinary_expect_constructor_result(
    JSContext *context, JSValueConst result, JSValueConst new_target,
    int expected_kind, int expected_order) {
    JSValue prototype = JS_UNDEFINED;
    JSValue expected_prototype = JS_UNDEFINED;
    JSValue kind = JS_UNDEFINED;
    JSValue order = JS_UNDEFINED;
    JSValue value = JS_UNDEFINED;
    int matches = 0;

    if (!JS_IsObject(result))
        goto cleanup;
    prototype = JS_GetPrototype(context, result);
    expected_prototype = JS_GetPropertyStr(context, new_target, "prototype");
    kind = JS_GetPropertyStr(context, result, "newTargetKind");
    order = JS_GetPropertyStr(context, result, "argOrder");
    value = JS_GetPropertyStr(context, result, "result");
    if (JS_IsException(prototype) || JS_IsException(expected_prototype) ||
        JS_IsException(kind) || JS_IsException(order) ||
        JS_IsException(value)) {
        report_exception(context, "constructor observation failed");
        goto cleanup;
    }
    matches = JS_VALUE_GET_PTR(prototype) ==
                  JS_VALUE_GET_PTR(expected_prototype) &&
              JS_VALUE_GET_TAG(kind) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(kind) == expected_kind &&
              JS_VALUE_GET_TAG(order) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(order) == expected_order &&
              JS_VALUE_GET_TAG(value) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(value) == 42;
cleanup:
    JS_FreeValue(context, value);
    JS_FreeValue(context, order);
    JS_FreeValue(context, kind);
    JS_FreeValue(context, expected_prototype);
    JS_FreeValue(context, prototype);
    return matches ? 0 : -1;
}

static JSValue ordinary_method_receiver = JS_UNDEFINED;
static int ordinary_method_call_count;

static JSValue ordinary_method_sink(JSContext *context,
                                    JSValueConst this_value,
                                    int argc, JSValueConst *argv) {
    static const int expected_args[] = { 20, 22 };
    JSValue base = JS_UNDEFINED;
    int sum = 0;

    if (ordinary_method_call_count != 0 ||
        !JS_IsObject(this_value) ||
        JS_VALUE_GET_PTR(this_value) !=
            JS_VALUE_GET_PTR(ordinary_method_receiver) ||
        argc != 2)
        return ordinary_callback_error(context,
                                       "method receiver or argc drifted");
    for (int index = 0; index < argc; index++) {
        if (JS_VALUE_GET_TAG(argv[index]) != JS_TAG_INT ||
            JS_VALUE_GET_INT(argv[index]) !=
                expected_args[index])
            return ordinary_callback_error(context,
                                           "method argument order drifted");
        sum += JS_VALUE_GET_INT(argv[index]);
    }
    base = JS_GetPropertyStr(context, this_value, "base");
    if (JS_IsException(base))
        return base;
    if (JS_VALUE_GET_TAG(base) != JS_TAG_INT ||
        JS_VALUE_GET_INT(base) != 7) {
        JS_FreeValue(context, base);
        return ordinary_callback_error(context, "method base drifted");
    }
    sum += JS_VALUE_GET_INT(base);
    JS_FreeValue(context, base);
    ordinary_method_call_count++;
    return JS_NewInt32(context, sum);
}

typedef struct OrdinaryArrayBundle {
    JSValue empty;
    JSValue multi;
} OrdinaryArrayBundle;

static int ordinary_expect_array_bundle(JSContext *context,
                                        JSValueConst value,
                                        OrdinaryArrayBundle *bundle) {
    JSValue outer_length = JS_UNDEFINED;
    JSValue empty_length = JS_UNDEFINED;
    JSValue multi_length = JS_UNDEFINED;
    JSValue elements[3] = { JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED };
    int matches = 0;

    bundle->empty = JS_UNDEFINED;
    bundle->multi = JS_UNDEFINED;
    if (JS_IsArray(context, value) != 1)
        goto cleanup;
    outer_length = JS_GetPropertyStr(context, value, "length");
    bundle->empty = JS_GetPropertyUint32(context, value, 0);
    bundle->multi = JS_GetPropertyUint32(context, value, 1);
    if (JS_IsException(outer_length) || JS_IsException(bundle->empty) ||
        JS_IsException(bundle->multi) ||
        JS_IsArray(context, bundle->empty) != 1 ||
        JS_IsArray(context, bundle->multi) != 1)
        goto cleanup;
    empty_length = JS_GetPropertyStr(context, bundle->empty, "length");
    multi_length = JS_GetPropertyStr(context, bundle->multi, "length");
    for (uint32_t index = 0; index < 3; index++)
        elements[index] = JS_GetPropertyUint32(context, bundle->multi,
                                               index);
    matches = JS_VALUE_GET_TAG(outer_length) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(outer_length) == 2 &&
              JS_VALUE_GET_TAG(empty_length) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(empty_length) == 0 &&
              JS_VALUE_GET_TAG(multi_length) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(multi_length) == 3;
    for (int index = 0; matches && index < 3; index++)
        matches = JS_VALUE_GET_TAG(elements[index]) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(elements[index]) == index + 1;
cleanup:
    for (size_t index = 0; index < 3; index++)
        JS_FreeValue(context, elements[index]);
    JS_FreeValue(context, multi_length);
    JS_FreeValue(context, empty_length);
    JS_FreeValue(context, outer_length);
    return matches ? 0 : -1;
}

static int ordinary_expect_flat_array(JSContext *context,
                                      JSValueConst value,
                                      const int *expected,
                                      size_t expected_count) {
    JSValue length = JS_UNDEFINED;
    JSValue element = JS_UNDEFINED;
    int matches = JS_IsArray(context, value) == 1;

    if (!matches)
        return -1;
    length = JS_GetPropertyStr(context, value, "length");
    matches = JS_VALUE_GET_TAG(length) == JS_TAG_INT &&
              JS_VALUE_GET_INT(length) == (int)expected_count;
    for (size_t index = 0; matches && index < expected_count; index++) {
        element = JS_GetPropertyUint32(context, value, (uint32_t)index);
        matches = JS_VALUE_GET_TAG(element) == JS_TAG_INT &&
                  JS_VALUE_GET_INT(element) == expected[index];
        JS_FreeValue(context, element);
        element = JS_UNDEFINED;
    }
    JS_FreeValue(context, length);
    return matches ? 0 : -1;
}

static int expect_ordinary_invocation_cohort(JSContext *compile_context) {
    enum {
        CONSTRUCTOR_CASE, METHOD_CASE, ARRAYS_CASE,
        TAIL_CALL_CASE, TAIL_METHOD_CASE,
    };
    enum { APPLY_CALL_CASE, APPLY_CONSTRUCT_CASE };
    static const uint8_t case_targets[] = { 33, 36, 38, 35, 37 };
    static const OrdinaryFunctionMetadata manual_constructor_metadata = {
        0x0243, 1, { 2, 0, 2, 4, 0, 0, 0, 8, 2 }, 47,
    };
    static const OrdinaryFunctionMetadata manual_method_metadata = {
        0x0243, 1, { 2, 0, 2, 3, 0, 0, 0, 8, 2 }, 47,
    };
    static const OrdinaryFunctionMetadata manual_tail_method_metadata = {
        0x0243, 1, { 4, 0, 4, 4, 0, 0, 0, 7, 4 }, 55,
    };
    static const char constructor_observer_source[] =
        "(function(){"
        "function Target(a,b){"
        "this.newTargetKind=new.target===Target?1:"
        "new.target===NewTarget?2:0;"
        "this.argOrder=a*10+b;this.result=42;}"
        "function NewTarget(){}return [Target,NewTarget];})()";
    static const char strict_method_source[] =
        "(function(value){'use strict';"
        "var receiver=globalThis.__stage3aMethodReceiver;"
        "receiver.seenThis=this===receiver;"
        "receiver.seenArgc=arguments.length;"
        "receiver.seenValue=value;"
        "receiver.seenOrder=arguments.length*100+value;"
        "return this.base+value;})";
    static const char apply_semantic_source[] =
        "(function(){"
        "function ok(v,m){if(!v)throw Error(m);}"
        "function sameLog(log,want){"
        "return log.length===want.length&&"
        "log.every((v,i)=>v===want[i]);}"
        "var expected;"
        "function Call(a,b){'use strict';return {"
        "receiver:this===expected,argc:arguments.length,"
        "order:arguments.length?a*10+b:0,"
        "noNewTarget:new.target===void 0};}"
        "function Target(a,b){this.rawNewTarget=new.target===expected;"
        "this.argc=arguments.length;this.order=a*10+b;}"
        "function checkCall(r,argc,order){ok(r.receiver&&r.argc===argc&&"
        "r.order===order&&r.noNewTarget,'call observation');}"
        "function checkCtor(r,proto,argc,order){"
        "ok(Object.getPrototypeOf(r)===proto&&r.rawNewTarget&&"
        "r.argc===argc&&r.order===order,'construct observation');}"
        "var dense=[4,2],raw={};var r,e;"
        "expected=void 0;"
        "checkCall(__stage3bNaturalApply0(Call,0,dense),2,42);"
        "expected=Target;"
        "checkCtor(__stage3bNaturalApply1(Target,0,dense),"
        "Target.prototype,2,42);"
        "expected=raw;"
        "[__stage3bApply0,__stage3bApply1].forEach(function(apply){"
        "[null,void 0].forEach(function(args){"
        "checkCall(apply(Call,raw,args),0,0);});});"
        "expected=17;checkCall(__stage3bApply0(Call,17,dense),2,42);"
        "checkCall(__stage3bApply2(Call,17,dense),2,42);"
        "try{__stage3bApply2(Call,raw,null);}catch(x){e=x;}"
        "ok(e instanceof TypeError&&e.message==='not a object',"
        "'magic 2 nullish');e=void 0;"
        "expected=raw;checkCall(__stage3bApplyMax(Call,raw,void 0),0,0);"
        "expected=17;r=__stage3bApply1(Target,17,dense);"
        "checkCtor(r,Object.prototype,2,42);"
        "var ordinaryProto={},ordinary={prototype:ordinaryProto};"
        "expected=ordinary;r=__stage3bApply1(Target,ordinary,dense);"
        "checkCtor(r,ordinaryProto,2,42);"
        "function NewTarget(){}expected=NewTarget;"
        "r=__stage3bApply1(Target,NewTarget,dense);"
        "checkCtor(r,NewTarget.prototype,2,42);"
        "r=__stage3bApplyMax(Target,NewTarget,dense);"
        "checkCtor(r,NewTarget.prototype,2,42);"
        "var log=[];var poison={get length(){"
        "log.push('poison-length');throw Error('poison length');}};"
        "try{__stage3bApply1(7,raw,poison);}catch(x){e=x;}"
        "ok(e instanceof TypeError&&e.message==='not a function'&&"
        "log.length===0,'callability order');e=void 0;"
        "var list={get length(){log.push('length');return 2;},"
        "get 0(){log.push('0');return 2;},"
        "get 1(){log.push('1');return 1;}};"
        "var Arrow=(a,b)=>a+b;"
        "try{__stage3bApply1(Arrow,ordinary,list);}catch(x){e=x;}"
        "ok(e instanceof TypeError&&sameLog(log,['length','0','1']),"
        "'constructor order');e=void 0;log.length=0;"
        "var proxy=new Proxy(class ConstructOnly{}, {"
        "construct:function(target,args,newTarget){log.push('construct');"
        "return {rawNewTarget:newTarget===expected,argc:args.length,"
        "order:args[0]*10+args[1]};}});"
        "expected=ordinary;r=__stage3bApply1(proxy,ordinary,list);"
        "checkCtor(r,Object.prototype,2,21);"
        "ok(sameLog(log,['length','0','1','construct']),'proxy order');"
        "return 42;})()";
    static const char tail_semantic_source[] =
        "(function(){"
        "function ok(v,m){if(!v)throw Error(m);}"
        "function caught(thunk){try{thunk();}catch(e){return e;}"
        "throw Error('missing throw');}"
        "function lines(e){return String(e.stack).split('\\n')."
        "filter(function(line){return line.length!==0;});}"
        "function checkTrace(e,name){var trace=lines(e);"
        "ok(trace.length>=2&&trace[0].indexOf('at '+name)>=0&&"
        "trace[1].indexOf('at <anonymous>')>=0,'backtrace '+name);}"
        "var plainCalls=0,methodCalls=0,manualCalls=0;"
        "function Plain(a,b){'use strict';plainCalls++;"
        "ok(this===void 0&&arguments.length===2&&a===4&&b===2,"
        "'plain receiver/args');return 42;}"
        "var naturalReceiver={m:function(a,b){'use strict';methodCalls++;"
        "ok(this===naturalReceiver&&arguments.length===2&&a===4&&b===2,"
        "'natural method receiver/args');return 42;}};"
        "var manualReceiver={};"
        "function Manual(a,b){'use strict';manualCalls++;"
        "ok(this===manualReceiver&&arguments.length===2&&a===4&&b===2,"
        "'manual method receiver/args');return 42;}"
        "ok(__stage3cTailCall(Plain,4,2)===42&&plainCalls===1,"
        "'plain success');"
        "ok(__stage3cTailMethodNatural(naturalReceiver,4,2)===42&&"
        "methodCalls===1,'natural method success');"
        "ok(__stage3cTailMethodManual(manualReceiver,Manual,4,2)===42&&"
        "manualCalls===1,'manual method success');"
        "var e=caught(function(){__stage3cTailCall(7,4,2);});"
        "ok(e instanceof TypeError&&e.message==='not a function'&&"
        "lines(e)[0].indexOf('at <anonymous>')>=0,'plain noncallable');"
        "e=caught(function(){__stage3cTailMethodNatural({m:7},4,2);});"
        "ok(e instanceof TypeError&&e.message==='not a function'&&"
        "lines(e)[0].indexOf('at <anonymous>')>=0,'natural noncallable');"
        "e=caught(function(){"
        "__stage3cTailMethodManual(manualReceiver,7,4,2);});"
        "ok(e instanceof TypeError&&e.message==='not a function'&&"
        "lines(e)[0].indexOf('at <anonymous>')>=0,'manual noncallable');"
        "var sentinel={kind:'callee'},getterSentinel={kind:'getter'};"
        "function ThrowValue(){throw sentinel;}"
        "ok(caught(function(){__stage3cTailCall(ThrowValue,4,2);})"
        "===sentinel,'plain throw identity');"
        "naturalReceiver.m=ThrowValue;"
        "ok(caught(function(){"
        "__stage3cTailMethodNatural(naturalReceiver,4,2);})===sentinel,"
        "'natural method throw identity');"
        "ok(caught(function(){__stage3cTailMethodManual("
        "manualReceiver,ThrowValue,4,2);})===sentinel,"
        "'manual method throw identity');"
        "var getterReceiver={get m(){throw getterSentinel;}};"
        "ok(caught(function(){__stage3cTailMethodNatural("
        "getterReceiver,4,2);})===getterSentinel,'getter throw identity');"
        "function PlainBoom(){throw Error('tail-plain-boom');}"
        "e=caught(function(){__stage3cTailCall(PlainBoom,4,2);});"
        "ok(e.message==='tail-plain-boom','plain error');"
        "checkTrace(e,'PlainBoom');"
        "function MethodBoom(){throw Error('tail-method-boom');}"
        "naturalReceiver.m=MethodBoom;"
        "e=caught(function(){"
        "__stage3cTailMethodNatural(naturalReceiver,4,2);});"
        "ok(e.message==='tail-method-boom','natural method error');"
        "checkTrace(e,'MethodBoom');"
        "e=caught(function(){__stage3cTailMethodManual("
        "manualReceiver,MethodBoom,4,2);});"
        "ok(e.message==='tail-method-boom','manual method error');"
        "checkTrace(e,'MethodBoom');"
        "function RecurPlain(){return __stage3cTailCall("
        "RecurPlain,0,0);}"
        "e=caught(function(){RecurPlain();});"
        "ok(e instanceof InternalError&&e.message==='stack overflow',"
        "'plain recursion');"
        "var recurReceiver={};recurReceiver.m=function RecurMethod(){"
        "return __stage3cTailMethodNatural(recurReceiver,0,0);};"
        "e=caught(function(){recurReceiver.m();});"
        "ok(e instanceof InternalError&&e.message==='stack overflow',"
        "'method recursion');"
        "return 42;})()";
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue functions[5] = {
        JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED,
    };
    JSValue apply_functions[2] = { JS_UNDEFINED, JS_UNDEFINED };
    JSValue manual_apply_functions[4] = {
        JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED, JS_UNDEFINED,
    };
    JSValue manual_constructor = JS_UNDEFINED;
    JSValue manual_method = JS_UNDEFINED;
    JSValue manual_tail_method = JS_UNDEFINED;
    JSValue constructors = JS_UNDEFINED;
    JSValue target = JS_UNDEFINED;
    JSValue new_target = JS_UNDEFINED;
    JSValue natural_result = JS_UNDEFINED;
    JSValue manual_result = JS_UNDEFINED;
    JSValue receiver = JS_UNDEFINED;
    JSValue method = JS_UNDEFINED;
    JSValue strict_method = JS_UNDEFINED;
    JSValue global = JS_UNDEFINED;
    JSValue method_seen_this = JS_UNDEFINED;
    JSValue method_seen_argc = JS_UNDEFINED;
    JSValue method_seen_value = JS_UNDEFINED;
    JSValue method_seen_order = JS_UNDEFINED;
    JSValue manual_empty_array = JS_UNDEFINED;
    JSValue manual_multi_array = JS_UNDEFINED;
    JSValue apply_semantic_result = JS_UNDEFINED;
    JSValue tail_semantic_result = JS_UNDEFINED;
    JSValue array_results[2] = { JS_UNDEFINED, JS_UNDEFINED };
    OrdinaryArrayBundle bundles[2] = {
        { JS_UNDEFINED, JS_UNDEFINED },
        { JS_UNDEFINED, JS_UNDEFINED },
    };
    uint8_t compiler_raws[256] = { 0 };
    uint8_t invocation_raws[256] = { 0 };
    uint8_t natural_admission_raws[256] = { 0 };
    uint8_t manual_admission_raws[256] = { 0 };
    uint8_t deferred_raws[256] = { 0 };
    uint8_t atom_free_raws[256] = { 0 };
    uint8_t plain_call_raws[256] = { 0 };
    size_t compiler_raw_count = 0;
    size_t compiler_target_count = 0;
    uint8_t manual_tail_method_wire[ORDINARY_TAIL_METHOD_WIRE_SIZE];
    int status = -1;

    if (ordinary_build_raw_set("ordinary invocation raws",
                               ordinary_invocation_raws,
                               sizeof(ordinary_invocation_raws),
                               invocation_raws) ||
        ordinary_build_raw_set("ordinary invocation natural admission raws",
                               ordinary_invocation_natural_admission_raws,
                               sizeof(ordinary_invocation_natural_admission_raws),
                               natural_admission_raws) ||
        ordinary_build_raw_set("ordinary invocation manual admission raws",
                               ordinary_invocation_manual_admission_raws,
                               sizeof(ordinary_invocation_manual_admission_raws),
                               manual_admission_raws) ||
        ordinary_build_raw_set("ordinary atom-free raws",
                               ordinary_expansion_atom_free_raws,
                               sizeof(ordinary_expansion_atom_free_raws),
                               atom_free_raws) ||
        ordinary_build_raw_set("ordinary plain-call raws",
                               ordinary_expansion_call_raws,
                               sizeof(ordinary_expansion_call_raws),
                               plain_call_raws))
        goto cleanup;
    for (unsigned raw = 0; raw < 256; raw++) {
        if ((invocation_raws[raw] !=
             (natural_admission_raws[raw] | manual_admission_raws[raw])) ||
            (natural_admission_raws[raw] && manual_admission_raws[raw]) ||
            (invocation_raws[raw] && deferred_raws[raw]) ||
            ((invocation_raws[raw] || deferred_raws[raw]) &&
             (atom_free_raws[raw] || plain_call_raws[raw]))) {
            fputs("ordinary invocation raw cohorts overlap or drifted\n",
                  stderr);
            goto cleanup;
        }
    }
    runtime = JS_NewRuntime();
    context = runtime ? JS_NewContext(runtime) : NULL;
    if (!context) {
        fputs("ordinary invocation fresh runtime allocation failed\n", stderr);
        goto cleanup;
    }
    for (size_t index = 0;
         index < sizeof(ordinary_invocation_cases) /
                     sizeof(ordinary_invocation_cases[0]); index++) {
        if (ordinary_compile_load_case(compile_context, context,
                                       &ordinary_invocation_cases[index],
                                       &functions[index], compiler_raws,
                                       case_targets[index]))
            goto cleanup;
    }
    for (size_t index = 0;
         index < sizeof(ordinary_apply_cases) /
                     sizeof(ordinary_apply_cases[0]); index++) {
        if (ordinary_compile_load_case(compile_context, context,
                                       &ordinary_apply_cases[index],
                                       &apply_functions[index],
                                       compiler_raws, 39))
            goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++) {
        compiler_raw_count += compiler_raws[raw] != 0;
        compiler_target_count += compiler_raws[raw] && invocation_raws[raw];
        if (compiler_raws[raw] && deferred_raws[raw]) {
            fputs("ordinary invocation natural cases emitted deferred raw\n",
                  stderr);
            goto cleanup;
        }
    }
    if (compiler_raw_count != 19 || compiler_target_count != 6 ||
        !compiler_raws[62]) {
        fputs("ordinary invocation compiler discovery drifted\n", stderr);
        goto cleanup;
    }
    puts("ordinary-invocation-evidence="
         "compiler-natural-write-read-write-fresh-runtime");
    puts("ordinary-invocation-compiler-natural-case-count=7");
    puts("ordinary-invocation-compiler-natural-raw-count=6");
    ordinary_print_raw_set("ordinary-invocation-compiler-natural-raw",
                           invocation_raws);
    puts("ordinary-invocation-compiler-union-count=19");
    ordinary_print_raw_set("ordinary-invocation-compiler-union-raw",
                           compiler_raws);
    puts("ordinary-invocation-natural-full-admission-count=4");
    ordinary_print_raw_set("ordinary-invocation-natural-full-admission-raw",
                           natural_admission_raws);
    puts("ordinary-invocation-natural-full-admission-status="
         "upstream-evidence-for-rust-admission");
    puts("ordinary-invocation-method-natural-property-producer="
         "raw36,raw37:raw62-get_field2-blocked");
    puts("ordinary-invocation-method-public-provenance="
         "raw36,raw37:authenticated-manual-wire-property-free-"
         "synthetic-stack");
    puts("ordinary-invocation-public-admission-count=6");
    ordinary_print_raw_set("ordinary-invocation-public-admission-raw",
                           invocation_raws);
    puts("ordinary-invocation-deferred-status=none");
    puts("ordinary-invocation-deferred-count=0");
    ordinary_print_raw_set("ordinary-invocation-deferred-raw",
                           deferred_raws);
    puts("ordinary-invocation-deferred-detail=none");
    puts("ordinary-invocation-apply-operand-policy="
         "rust-admitted:0,1;rust-unadmitted:2,65535;"
         "upstream-mechanically-executable:u16");

    if (ordinary_expect_manual_apply_base(compile_context) ||
        ordinary_expect_manual_tail_method_base(compile_context) ||
        ordinary_build_manual_tail_method_wire(manual_tail_method_wire) ||
        ordinary_load_manual_invocation(
            context, "constructor", ordinary_manual_constructor_wire,
            sizeof(ordinary_manual_constructor_wire),
            UINT64_C(0xc30527b59bccdaa3),
            &manual_constructor_metadata, 33, &manual_constructor) ||
        ordinary_load_manual_invocation(
            context, "method", ordinary_manual_method_wire,
            sizeof(ordinary_manual_method_wire),
            UINT64_C(0xd751b6fb94500c22), &manual_method_metadata,
            36, &manual_method) ||
        ordinary_load_manual_invocation(
            context, "tail-method", manual_tail_method_wire,
            sizeof(manual_tail_method_wire),
            UINT64_C(0xe87d54c0a2a140ca),
            &manual_tail_method_metadata, 37, &manual_tail_method) ||
        ordinary_eval_manual_array_from(
            context, "zero", ordinary_manual_array_from_zero_wire,
            sizeof(ordinary_manual_array_from_zero_wire),
            UINT64_C(0x98bf03482102917a), 1, 4,
            &manual_empty_array) ||
        ordinary_eval_manual_array_from(
            context, "multi", ordinary_manual_array_from_multi_wire,
            sizeof(ordinary_manual_array_from_multi_wire),
            UINT64_C(0xf0741b4abade33ab), 3, 7,
            &manual_multi_array))
        goto cleanup;
    for (size_t index = 0;
         index < sizeof(ordinary_apply_wire_cases) /
                     sizeof(ordinary_apply_wire_cases[0]); index++) {
        if (ordinary_load_manual_apply(
                context, &ordinary_apply_wire_cases[index],
                &manual_apply_functions[index]))
            goto cleanup;
    }
    {
        static const int expected_multi[] = { 1, 2, 3 };
        if (ordinary_expect_flat_array(context, manual_empty_array,
                                       NULL, 0) ||
            ordinary_expect_flat_array(context, manual_multi_array,
                                       expected_multi, 3) ||
            JS_VALUE_GET_PTR(manual_empty_array) ==
                JS_VALUE_GET_PTR(manual_multi_array)) {
            fputs("manual array_from values or identity drifted\n", stderr);
            goto cleanup;
        }
    }
    puts("ordinary-invocation-manual-array-from-values="
         "zero:[],multi:[1,2,3],identity:distinct");

    constructors = JS_Eval(context, constructor_observer_source,
                           strlen(constructor_observer_source),
                           "constructor-observer.js", JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(constructors) ||
        JS_IsArray(context, constructors) != 1) {
        report_exception(context, "constructor observer setup failed");
        constructors = JS_UNDEFINED;
        goto cleanup;
    }
    target = JS_GetPropertyUint32(context, constructors, 0);
    new_target = JS_GetPropertyUint32(context, constructors, 1);
    if (JS_IsException(target) || JS_IsException(new_target) ||
        !JS_IsConstructor(context, target) ||
        !JS_IsConstructor(context, new_target) ||
        JS_VALUE_GET_PTR(target) == JS_VALUE_GET_PTR(new_target)) {
        report_exception(context, "constructor observer functions failed");
        goto cleanup;
    }
    {
        JSValue arguments[3] = {
            target, JS_NewInt32(context, 3), JS_NewInt32(context, 4),
        };
        natural_result = JS_Call(context, functions[CONSTRUCTOR_CASE],
                                 JS_UNDEFINED, 3, arguments);
        JS_FreeValue(context, arguments[2]);
        JS_FreeValue(context, arguments[1]);
    }
    if (JS_IsException(natural_result) ||
        ordinary_expect_constructor_result(context, natural_result, target,
                                           1, 34)) {
        report_exception(context, "natural constructor execution failed");
        goto cleanup;
    }
    {
        JSValueConst arguments[2] = { target, new_target };
        manual_result = JS_Call(context, manual_constructor, JS_UNDEFINED,
                                2, arguments);
    }
    if (JS_IsException(manual_result) ||
        ordinary_expect_constructor_result(context, manual_result,
                                           new_target, 2, 12)) {
        report_exception(context, "manual constructor execution failed");
        goto cleanup;
    }
    puts("ordinary-invocation-constructor-natural="
         "new-target:same,args:3,4,order:34,result:42");
    puts("ordinary-invocation-constructor-manual="
         "new-target:distinct,args:1,2,order:12,result:42,"
         "prototype:new-target");

    receiver = JS_NewObject(context);
    method = JS_NewCFunction(context, ordinary_method_sink,
                             "ordinaryMethodSink", 2);
    global = JS_GetGlobalObject(context);
    if (JS_IsException(receiver) || JS_IsException(method) ||
        JS_IsException(global) ||
        JS_SetPropertyStr(context, receiver, "base",
                          JS_NewInt32(context, 7)) < 0 ||
        JS_SetPropertyStr(context, receiver, "m",
                          JS_DupValue(context, method)) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3aMethodReceiver",
                          JS_DupValue(context, receiver)) < 0) {
        report_exception(context, "method observer setup failed");
        goto cleanup;
    }
    strict_method = JS_Eval(context, strict_method_source,
                            strlen(strict_method_source),
                            "strict-method-observer.js",
                            JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(strict_method) ||
        !JS_IsFunction(context, strict_method)) {
        report_exception(context, "strict method observer compile failed");
        strict_method = JS_UNDEFINED;
        goto cleanup;
    }
    ordinary_method_receiver = receiver;
    ordinary_method_call_count = 0;
    {
        JSValue arguments[3] = {
            receiver, JS_NewInt32(context, 20), JS_NewInt32(context, 22),
        };
        if (ordinary_expect_call(context, functions[METHOD_CASE], 3,
                                 arguments, ORDINARY_CALL_INT, 49,
                                 "natural method execution")) {
            JS_FreeValue(context, arguments[2]);
            JS_FreeValue(context, arguments[1]);
            goto cleanup;
        }
        JS_FreeValue(context, arguments[2]);
        JS_FreeValue(context, arguments[1]);
    }
    {
        JSValueConst arguments[2] = { receiver, strict_method };
        if (ordinary_expect_call(context, manual_method, 2, arguments,
                                 ORDINARY_CALL_INT, 49,
                                 "manual method execution"))
            goto cleanup;
    }
    method_seen_this = JS_GetPropertyStr(context, receiver, "seenThis");
    method_seen_argc = JS_GetPropertyStr(context, receiver, "seenArgc");
    method_seen_value = JS_GetPropertyStr(context, receiver, "seenValue");
    method_seen_order = JS_GetPropertyStr(context, receiver, "seenOrder");
    if (ordinary_method_call_count != 1 ||
        JS_VALUE_GET_TAG(method_seen_this) != JS_TAG_BOOL ||
        JS_VALUE_GET_BOOL(method_seen_this) != 1 ||
        JS_VALUE_GET_TAG(method_seen_argc) != JS_TAG_INT ||
        JS_VALUE_GET_INT(method_seen_argc) != 1 ||
        JS_VALUE_GET_TAG(method_seen_value) != JS_TAG_INT ||
        JS_VALUE_GET_INT(method_seen_value) != 42 ||
        JS_VALUE_GET_TAG(method_seen_order) != JS_TAG_INT ||
        JS_VALUE_GET_INT(method_seen_order) != 142) {
        fputs("method observation sequence drifted\n", stderr);
        goto cleanup;
    }
    puts("ordinary-invocation-method-natural="
         "strict-receiver:identity,argc:2,args:20,22,result:49");
    puts("ordinary-invocation-method-manual="
         "strict-receiver:identity,argc:1,args:42,result:49,"
         "producer:property-free-synthetic-stack");
    puts("ordinary-invocation-method-order=natural,manual");

    for (size_t call = 0; call < 2; call++) {
        JSValue arguments[3] = {
            JS_NewInt32(context, 1), JS_NewInt32(context, 2),
            JS_NewInt32(context, 3),
        };
        array_results[call] = JS_Call(context, functions[ARRAYS_CASE],
                                      JS_UNDEFINED, 3, arguments);
        for (size_t index = 0; index < 3; index++)
            JS_FreeValue(context, arguments[index]);
        if (JS_IsException(array_results[call]) ||
            ordinary_expect_array_bundle(context, array_results[call],
                                         &bundles[call])) {
            report_exception(context, "array_from execution failed");
            goto cleanup;
        }
    }
    if (JS_VALUE_GET_PTR(array_results[0]) ==
            JS_VALUE_GET_PTR(array_results[1]) ||
        JS_VALUE_GET_PTR(bundles[0].empty) ==
            JS_VALUE_GET_PTR(bundles[1].empty) ||
        JS_VALUE_GET_PTR(bundles[0].multi) ==
            JS_VALUE_GET_PTR(bundles[1].multi) ||
        JS_VALUE_GET_PTR(bundles[0].empty) ==
            JS_VALUE_GET_PTR(bundles[0].multi)) {
        fputs("array_from fresh identity drifted\n", stderr);
        goto cleanup;
    }
    puts("ordinary-invocation-array-from-empty=length:0");
    puts("ordinary-invocation-array-from-multi=length:3,values:1,2,3");
    puts("ordinary-invocation-array-from-fresh-identity="
         "outer:distinct,empty:distinct,multi:distinct");

    if (JS_SetPropertyStr(context, global, "__stage3cTailCall",
                          JS_DupValue(context,
                                      functions[TAIL_CALL_CASE])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3cTailMethodNatural",
                          JS_DupValue(context,
                                      functions[TAIL_METHOD_CASE])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3cTailMethodManual",
                          JS_DupValue(context, manual_tail_method)) < 0) {
        report_exception(context, "tail invocation oracle publication failed");
        goto cleanup;
    }
    tail_semantic_result = JS_Eval(
        context, tail_semantic_source, strlen(tail_semantic_source),
        "tail-semantic-oracle.js", JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(tail_semantic_result) ||
        JS_VALUE_GET_TAG(tail_semantic_result) != JS_TAG_INT ||
        JS_VALUE_GET_INT(tail_semantic_result) != 42) {
        report_exception(context, "tail invocation semantic oracle failed");
        goto cleanup;
    }
    puts("ordinary-invocation-tail-success-terminal="
         "raw35:terminal-no-return,raw37:terminal-no-return;fresh-eval:42");
    puts("ordinary-invocation-tail-call="
         "strict-receiver:undefined,argc:2,args:4,2,result:42");
    puts("ordinary-invocation-tail-method="
         "natural,manual;strict-receiver:identity,argc:2,args:4,2,"
         "result:42");
    puts("ordinary-invocation-tail-noncallable="
         "raw35,raw37-natural,raw37-manual:TypeError-not-a-function,"
         "tail-frame:first");
    puts("ordinary-invocation-tail-throw-catch="
         "raw35,raw37-natural,raw37-manual:callee-object-identity;"
         "raw37-natural:getter-object-identity");
    puts("ordinary-invocation-tail-backtrace="
         "raw35,raw37-natural,raw37-manual:callee-then-tail-frame");
    puts("ordinary-invocation-tail-recursion="
         "raw35,raw37:InternalError-stack-overflow,not-PTC");
    puts("ordinary-invocation-tail-oracle=passed");

    if (JS_SetPropertyStr(context, global, "__stage3bNaturalApply0",
                          JS_DupValue(context,
                                      apply_functions[APPLY_CALL_CASE])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3bNaturalApply1",
                          JS_DupValue(context,
                                      apply_functions[APPLY_CONSTRUCT_CASE])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3bApply0",
                          JS_DupValue(context,
                                      manual_apply_functions[0])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3bApply1",
                          JS_DupValue(context,
                                      manual_apply_functions[1])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3bApply2",
                          JS_DupValue(context,
                                      manual_apply_functions[2])) < 0 ||
        JS_SetPropertyStr(context, global, "__stage3bApplyMax",
                          JS_DupValue(context,
                                      manual_apply_functions[3])) < 0) {
        report_exception(context, "apply oracle publication failed");
        goto cleanup;
    }
    apply_semantic_result = JS_Eval(
        context, apply_semantic_source, strlen(apply_semantic_source),
        "apply-semantic-oracle.js", JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(apply_semantic_result) ||
        JS_VALUE_GET_TAG(apply_semantic_result) != JS_TAG_INT ||
        JS_VALUE_GET_INT(apply_semantic_result) != 42) {
        report_exception(context, "apply semantic oracle failed");
        goto cleanup;
    }
    puts("ordinary-invocation-apply-natural-call="
         "magic:0,receiver:undefined,argc:2,order:42,new.target:undefined");
    puts("ordinary-invocation-apply-natural-construct="
         "magic:1,new-target:same,argc:2,order:42,prototype:target");
    puts("ordinary-invocation-apply-nullish-shortcut="
         "magic:0,1;array:null,undefined;receiver:raw-object;"
         "argc:0;new.target:undefined");
    puts("ordinary-invocation-apply-dense-call="
         "magic:0,receiver:raw-int-17,argc:2,order:42");
    puts("ordinary-invocation-apply-noncanonical-even="
         "magic:2,upstream:call,receiver:raw-int-17,argc:2,order:42,"
         "rust:unadmitted");
    puts("ordinary-invocation-apply-noncanonical-nullish="
         "magic:2,null:TypeError-not-a-object;"
         "magic:65535,undefined:ordinary-call");
    puts("ordinary-invocation-apply-construct-raw-new-target="
         "magic:1;primitive:accepted-default-prototype;"
         "ordinary-object:accepted-own-prototype;"
         "callable:accepted-own-prototype;prevalidation:none");
    puts("ordinary-invocation-apply-noncanonical-odd="
         "magic:65535,upstream:construct,callable-new-target,"
         "argc:2,order:42,rust:unadmitted");
    puts("ordinary-invocation-apply-error-order-callability="
         "not-function-before-poison-length,log:empty");
    puts("ordinary-invocation-apply-error-order-constructor="
         "build-list:length,0,1;then:not-constructor-TypeError");
    puts("ordinary-invocation-apply-construct-only-proxy="
         "build-list:length,0,1;trap:construct;raw-new-target:ordinary;"
         "argc:2,order:21");
    puts("ordinary-invocation-apply-oracle=passed");
    puts("ordinary-invocation-oracle=passed");
    status = 0;

cleanup:
    ordinary_method_receiver = JS_UNDEFINED;
    if (context) {
        JS_FreeValue(context, tail_semantic_result);
        JS_FreeValue(context, apply_semantic_result);
        JS_FreeValue(context, manual_multi_array);
        JS_FreeValue(context, manual_empty_array);
        for (size_t index = 0; index < 2; index++) {
            JS_FreeValue(context, bundles[index].multi);
            JS_FreeValue(context, bundles[index].empty);
            JS_FreeValue(context, array_results[index]);
        }
        JS_FreeValue(context, method_seen_order);
        JS_FreeValue(context, method_seen_value);
        JS_FreeValue(context, method_seen_argc);
        JS_FreeValue(context, method_seen_this);
        JS_FreeValue(context, global);
        JS_FreeValue(context, strict_method);
        JS_FreeValue(context, method);
        JS_FreeValue(context, receiver);
        JS_FreeValue(context, manual_result);
        JS_FreeValue(context, natural_result);
        JS_FreeValue(context, new_target);
        JS_FreeValue(context, target);
        JS_FreeValue(context, constructors);
        JS_FreeValue(context, manual_method);
        JS_FreeValue(context, manual_tail_method);
        JS_FreeValue(context, manual_constructor);
        for (size_t index = 0; index < 4; index++)
            JS_FreeValue(context, manual_apply_functions[index]);
        for (size_t index = 0; index < 2; index++)
            JS_FreeValue(context, apply_functions[index]);
        for (size_t index = 0; index < 5; index++)
            JS_FreeValue(context, functions[index]);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int ordinary_expect_uncaught_throw_identity(
    JSContext *context, JSValueConst function, JSValueConst value,
    const char *label, int expect_new_stack) {
    JSAtom stack_atom = JS_ATOM_NULL;
    JSValue argument = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    int stack_before = 0;
    int stack_after = 0;
    int status = -1;

    if (expect_new_stack) {
        stack_atom = JS_NewAtom(context, "stack");
        if (stack_atom == JS_ATOM_NULL)
            goto cleanup;
        stack_before = JS_GetOwnProperty(context, NULL, value, stack_atom);
        if (stack_before != 0) {
            fprintf(stderr, "%s Error already had an own stack\n", label);
            goto cleanup;
        }
    }
    argument = JS_DupValue(context, value);
    result = JS_Call(context, function, JS_UNDEFINED, 1, &argument);
    JS_FreeValue(context, argument);
    argument = JS_UNDEFINED;
    if (!JS_IsException(result) || !JS_HasException(context)) {
        fprintf(stderr, "%s did not publish a pending exception\n", label);
        goto cleanup;
    }
    if (expect_new_stack) {
        stack_after = JS_GetOwnProperty(context, NULL, value, stack_atom);
        if (stack_after != 1 || !JS_HasException(context)) {
            fprintf(stderr, "%s Error backtrace timing drifted\n", label);
            goto cleanup;
        }
    }
    exception = JS_GetException(context);
    if (JS_HasException(context) ||
        JS_StrictEq(context, exception, value) != 1) {
        fprintf(stderr, "%s exception identity or pending clear drifted\n",
                label);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (JS_HasException(context)) {
        JSValue pending = JS_GetException(context);
        JS_FreeValue(context, pending);
    }
    JS_FreeValue(context, exception);
    JS_FreeValue(context, result);
    JS_FreeValue(context, argument);
    JS_FreeAtom(context, stack_atom);
    return status;
}

static int expect_ordinary_throw_completion(JSContext *compile_context) {
    static const char source[] =
        "(function(a){'use strict';throw a;})";
    static const char semantic_source[] =
        "(function(){"
        "function ok(v,m){if(!v)throw Error(m);}"
        "function own(o,p){return Object.prototype.hasOwnProperty.call(o,p);}"
        "var original={kind:'original'},caught;"
        "try{__stage3dThrow(73);}catch(e){caught=e;}"
        "ok(caught===73,'caller catch int identity');caught=void 0;"
        "try{__stage3dThrow(original);ok(false,'throw returned');}"
        "catch(e){caught=e;}"
        "ok(caught===original,'caller catch identity');"
        "var error=new Error('stage3d-backtrace');delete error.stack;"
        "ok(!own(error,'stack'),'Error started with stack');caught=void 0;"
        "try{__stage3dThrow(error);}catch(e){caught=e;}"
        "ok(caught===error&&own(error,'stack')&&"
        "String(error.stack).indexOf('at <anonymous>')>=0,"
        "'Error backtrace before catch');"
        "var close={kind:'close'},log=[];"
        "var iterable={};iterable[Symbol.iterator]=function(){return {"
        "next:function(){return {value:1,done:false};},"
        "return:function(){log.push('return');throw close;}};};"
        "try{for(var item of iterable){log.push('body');"
        "__stage3dThrow(original);log.push('after-throw');}}"
        "catch(e){log.push(e===original?'catch-original':'catch-other');}"
        "ok(log.join(',')==='body,return,catch-original',"
        "'iterator close ordering or replacement');"
        "return 42;})()";
    static const OrdinaryFunctionMetadata expected_metadata = {
        0x0243, 1, { 1, 0, 1, 1, 0, 0, 0, 2, 1 }, 43,
    };
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue compiled = JS_UNDEFINED;
    JSValue loaded = JS_UNDEFINED;
    JSValue function = JS_UNDEFINED;
    JSValue integer = JS_UNDEFINED;
    JSValue object = JS_UNDEFINED;
    JSValue error = JS_UNDEFINED;
    JSValue global = JS_UNDEFINED;
    JSValue semantic_result = JS_UNDEFINED;
    uint8_t *wire = NULL;
    uint8_t *rewritten = NULL;
    size_t wire_size = 0;
    size_t rewritten_size = 0;
    OrdinaryFunctionMetadata child = { 0 };
    uint8_t raws[256] = { 0 };
    uint8_t terminal_raw = 0;
    size_t raw_count = 0;
    int status = -1;

    compiled = JS_Eval(compile_context, source, strlen(source),
                       "ordinary-throw",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context, "ordinary throw compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    wire = JS_WriteObject(compile_context, &wire_size, compiled,
                          JS_WRITE_OBJ_BYTECODE);
    if (!wire) {
        report_exception(compile_context, "ordinary throw write failed");
        goto cleanup;
    }
    if (wire_size != sizeof(ordinary_throw_bytecode) ||
        memcmp(wire, ordinary_throw_bytecode, wire_size) != 0 ||
        ordinary_fnv1a64(wire, wire_size) !=
            UINT64_C(0x73cf217e06c5fee2) ||
        ordinary_wire_child_metadata(wire, wire_size, &child) ||
        !ordinary_metadata_equal(&child, &expected_metadata) ||
        ordinary_collect_opcodes(wire + child.code_offset,
                                 child.fields[ORD_CODE], raws) ||
        ordinary_terminal_opcode(wire + child.code_offset,
                                 child.fields[ORD_CODE], &terminal_raw)) {
        fputs("ordinary throw BC5 wire/metadata/opcodes drifted\n", stderr);
        goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++)
        raw_count += raws[raw] != 0;
    if (raw_count != 2 || !raws[48] || !raws[207] || raws[40] ||
        raws[41] || terminal_raw != 48) {
        fputs("ordinary throw opcode set or terminal drifted\n", stderr);
        goto cleanup;
    }

    runtime = JS_NewRuntime();
    context = runtime ? JS_NewContext(runtime) : NULL;
    if (!context) {
        fputs("ordinary throw fresh runtime allocation failed\n", stderr);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, wire, wire_size,
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "ordinary throw read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten || rewritten_size != wire_size ||
        memcmp(rewritten, wire, wire_size) != 0) {
        if (!rewritten)
            report_exception(context, "ordinary throw rewrite failed");
        else
            fputs("ordinary throw rewrite drifted\n", stderr);
        goto cleanup;
    }
    function = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(function) || !JS_IsFunction(context, function)) {
        report_exception(context, "ordinary throw root evaluation failed");
        function = JS_UNDEFINED;
        goto cleanup;
    }

    integer = JS_NewInt32(context, 73);
    object = JS_NewObject(context);
    error = JS_NewError(context);
    if (JS_IsException(object) || JS_IsException(error) ||
        ordinary_expect_uncaught_throw_identity(
            context, function, integer, "ordinary throw int", 0) ||
        ordinary_expect_uncaught_throw_identity(
            context, function, object, "ordinary throw object", 0) ||
        ordinary_expect_uncaught_throw_identity(
            context, function, error, "ordinary throw Error", 1))
        goto cleanup;

    global = JS_GetGlobalObject(context);
    if (JS_IsException(global) ||
        JS_SetPropertyStr(context, global, "__stage3dThrow",
                          JS_DupValue(context, function)) < 0) {
        report_exception(context, "ordinary throw publication failed");
        goto cleanup;
    }
    semantic_result = JS_Eval(context, semantic_source,
                              strlen(semantic_source),
                              "ordinary-throw-semantic.js",
                              JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(semantic_result) || JS_HasException(context) ||
        JS_VALUE_GET_TAG(semantic_result) != JS_TAG_INT ||
        JS_VALUE_GET_INT(semantic_result) != 42) {
        report_exception(context, "ordinary throw semantic oracle failed");
        semantic_result = JS_UNDEFINED;
        goto cleanup;
    }

    puts("ordinary-throw-evidence="
         "compiler-natural-write-read-write-fresh-runtime");
    puts("ordinary-throw-source-hex="
         "2866756e6374696f6e2861297b2775736520737472696374273b7468726f7720"
         "613b7d29");
    puts("ordinary-throw-compile-mode=global-compile-only,strip-debug");
    printf("ordinary-throw-wire-size=%zu\n", wire_size);
    printf("ordinary-throw-wire-fnv1a64=%016" PRIx64 "\n",
           ordinary_fnv1a64(wire, wire_size));
    puts("ordinary-throw-wire-sha256="
         "b7998b9678635e7e0a4eb2e465b683d168395adc7f156f733c25521907e3c8a8");
    fputs("ordinary-throw-wire-hex=", stdout);
    for (size_t index = 0; index < wire_size; index++)
        printf("%02x", wire[index]);
    putchar('\n');
    printf("ordinary-throw-child-metadata="
           "flags:%04x,js_mode:%u,args:%" PRIu32 ",vars:%" PRIu32
           ",defined_args:%" PRIu32 ",stack:%" PRIu32
           ",var_refs:%" PRIu32 ",closures:%" PRIu32
           ",cpool:%" PRIu32 ",code:%" PRIu32 ",locals:%" PRIu32
           ",code_offset:%zu\n",
           child.flags, child.js_mode, child.fields[ORD_ARGS],
           child.fields[ORD_VARS], child.fields[ORD_DEFINED_ARGS],
           child.fields[ORD_STACK], child.fields[ORD_VAR_REFS],
           child.fields[ORD_CLOSURES], child.fields[ORD_CPOOL],
           child.fields[ORD_CODE], child.fields[ORD_LOCALS],
           child.code_offset);
    puts("ordinary-throw-child-code-hex=cf30");
    puts("ordinary-throw-child-code-raw=207,48");
    ordinary_print_raw_set("ordinary-throw-child-raw", raws);
    puts("ordinary-throw-terminal=raw48,stack:1->0,no-return");
    puts("ordinary-throw-rewrite=identity");
    puts("ordinary-throw-fresh-root=Function");
    puts("ordinary-throw-uncaught-c-api="
         "int,object,Error:JS_EXCEPTION,GetException-original-identity");
    puts("ordinary-throw-pending="
         "GetException-clears;caller-catch-clears");
    puts("ordinary-throw-error-backtrace="
         "missing-own-stack-before,own-stack-before-catch");
    puts("ordinary-throw-caller-catch="
         "int,object,Error:original-identity;terminal-no-return");
    puts("ordinary-throw-iterator-close="
         "body,return,catch-original;close-throw-does-not-replace-original");
    puts("ordinary-throw-admitted-count=1");
    puts("ordinary-throw-admitted-raw=48");
    puts("ordinary-throw-deferred-count=1");
    puts("ordinary-throw-deferred-raw=177");
    puts("ordinary-throw-deferred-detail=raw177:nop-specialized-blocked");
    puts("ordinary-throw-oracle=passed");
    status = 0;

cleanup:
    if (rewritten && context)
        js_free(context, rewritten);
    if (context) {
        if (JS_HasException(context)) {
            JSValue pending = JS_GetException(context);
            JS_FreeValue(context, pending);
        }
        JS_FreeValue(context, semantic_result);
        JS_FreeValue(context, global);
        JS_FreeValue(context, error);
        JS_FreeValue(context, object);
        JS_FreeValue(context, integer);
        JS_FreeValue(context, function);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    if (wire)
        js_free(compile_context, wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static int ordinary_expect_throw_error_wire(
    const uint8_t *wire, size_t wire_size, const char *label,
    const char *expected_class, const char *expected_message) {
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue function = JS_UNDEFINED;
    JSValue result = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    uint8_t *rewritten = NULL;
    size_t rewritten_size = 0;
    int status = -1;

    runtime = JS_NewRuntime();
    context = runtime ? JS_NewContext(runtime) : NULL;
    if (!context) {
        fprintf(stderr, "%s fresh runtime allocation failed\n", label);
        goto cleanup;
    }
    loaded = JS_ReadObject(context, wire, wire_size, JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(context, "throw_error wire read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten || rewritten_size != wire_size ||
        memcmp(rewritten, wire, wire_size) != 0) {
        if (!rewritten)
            report_exception(context, "throw_error wire rewrite failed");
        else
            fprintf(stderr, "%s rewrite drifted\n", label);
        goto cleanup;
    }
    function = JS_EvalFunction(context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(function) || !JS_IsFunction(context, function)) {
        report_exception(context, "throw_error root evaluation failed");
        function = JS_UNDEFINED;
        goto cleanup;
    }
    result = JS_Call(context, function, JS_UNDEFINED, 0, NULL);
    if (!JS_IsException(result) || !JS_HasException(context)) {
        fprintf(stderr, "%s returned instead of publishing an exception\n",
                label);
        goto cleanup;
    }
    exception = JS_GetException(context);
    if (JS_HasException(context) ||
        expect_exception_fields(context, label, exception,
                                expected_class, expected_message))
        goto cleanup;
    status = 0;

cleanup:
    if (context && JS_HasException(context)) {
        JSValue pending = JS_GetException(context);
        JS_FreeValue(context, pending);
    }
    if (rewritten && context)
        js_free(context, rewritten);
    if (context) {
        JS_FreeValue(context, exception);
        JS_FreeValue(context, result);
        JS_FreeValue(context, function);
        JS_FreeValue(context, loaded);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    return status;
}

static int ordinary_expect_throw_error_call(
    JSContext *caller_context, JSValueConst function,
    JSValueConst defining_type_error, JSValueConst caller_type_error,
    const char *expected_message, const char *label) {
    JSValue result = JS_UNDEFINED;
    JSValue exception = JS_UNDEFINED;
    JSValue stack = JS_UNDEFINED;
    JSAtom stack_atom = JS_ATOM_NULL;
    const char *stack_text = NULL;
    int defining_instance;
    int caller_instance;
    int status = -1;

    result = JS_Call(caller_context, function, JS_UNDEFINED, 0, NULL);
    if (!JS_IsException(result) || !JS_HasException(caller_context)) {
        fprintf(stderr, "%s returned instead of publishing an exception\n",
                label);
        goto cleanup;
    }
    exception = JS_GetException(caller_context);
    if (JS_HasException(caller_context) ||
        expect_exception_fields(caller_context, label, exception,
                                "TypeError", expected_message))
        goto cleanup;
    defining_instance = JS_IsInstanceOf(caller_context, exception,
                                        defining_type_error);
    caller_instance = JS_IsInstanceOf(caller_context, exception,
                                      caller_type_error);
    if (defining_instance != 1 || caller_instance != 0) {
        fprintf(stderr, "%s Error realm drifted\n", label);
        goto cleanup;
    }
    stack_atom = JS_NewAtom(caller_context, "stack");
    if (stack_atom == JS_ATOM_NULL ||
        JS_GetOwnProperty(caller_context, NULL, exception, stack_atom) != 1) {
        fprintf(stderr, "%s did not attach an own backtrace\n", label);
        goto cleanup;
    }
    stack = JS_GetProperty(caller_context, exception, stack_atom);
    if (JS_IsException(stack)) {
        report_exception(caller_context, "throw_error stack read failed");
        stack = JS_UNDEFINED;
        goto cleanup;
    }
    stack_text = JS_ToCString(caller_context, stack);
    if (!stack_text || !strstr(stack_text, "at <anonymous>")) {
        fprintf(stderr, "%s backtrace lost its anonymous frame\n", label);
        goto cleanup;
    }
    status = 0;

cleanup:
    if (caller_context && JS_HasException(caller_context)) {
        JSValue pending = JS_GetException(caller_context);
        JS_FreeValue(caller_context, pending);
    }
    if (stack_text)
        JS_FreeCString(caller_context, stack_text);
    JS_FreeAtom(caller_context, stack_atom);
    JS_FreeValue(caller_context, stack);
    JS_FreeValue(caller_context, exception);
    JS_FreeValue(caller_context, result);
    return status;
}

static int expect_ordinary_throw_error_completion(JSContext *compile_context) {
    static const char natural_source[] =
        "(function(){'use strict';const x=0;x=1;})";
    static const char semantic_source[] =
        "(function(){"
        "function ok(v,m){if(!v)throw Error(m);}"
        "var caught,reached=false;"
        "try{__stage3eThrowError();reached=true;}catch(e){caught=e;}"
        "ok(!reached,'throw_error returned');"
        "ok(caught instanceof __stage3eDefiningTypeError,"
        "'defining realm TypeError');"
        "ok(!(caught instanceof TypeError),'caller realm TypeError');"
        "ok(caught.name==='TypeError'&&"
        "caught.message===\"'\\u00e9' is read-only\","
        "'Unicode read-only message');"
        "ok(Object.prototype.hasOwnProperty.call(caught,'stack')&&"
        "String(caught.stack).indexOf('at <anonymous>')>=0,"
        "'throw_error backtrace');"
        "return 42;})()";
    static const OrdinaryFunctionMetadata expected_natural_metadata = {
        0x0243, 1, { 0, 1, 0, 2, 0, 0, 0, 13, 1 }, 45,
    };
    static const OrdinaryFunctionMetadata expected_manual_metadata = {
        0x0243, 1, { 0, 0, 0, 0, 0, 0, 0, 6, 0 }, 41,
    };
    static const char expected_x_message[] = "'x' is read-only";
    static const char expected_unicode_message[] =
        "'\xc3\xa9' is read-only";
    JSValue compiled = JS_UNDEFINED;
    JSRuntime *runtime = NULL;
    JSContext *defining_context = NULL;
    JSContext *caller_context = NULL;
    JSValue loaded = JS_UNDEFINED;
    JSValue function = JS_UNDEFINED;
    JSValue unicode_loaded = JS_UNDEFINED;
    JSValue unicode_function = JS_UNDEFINED;
    JSValue defining_global = JS_UNDEFINED;
    JSValue caller_global = JS_UNDEFINED;
    JSValue defining_type_error = JS_UNDEFINED;
    JSValue caller_type_error = JS_UNDEFINED;
    JSValue semantic_result = JS_UNDEFINED;
    uint8_t *natural_wire = NULL;
    uint8_t *rewritten = NULL;
    uint8_t *unicode_rewritten = NULL;
    size_t natural_wire_size = 0;
    size_t rewritten_size = 0;
    size_t unicode_rewritten_size = 0;
    OrdinaryFunctionMetadata natural_child = { 0 };
    OrdinaryFunctionMetadata manual_child = { 0 };
    uint8_t natural_raws[256] = { 0 };
    uint8_t manual_raws[256] = { 0 };
    uint8_t natural_terminal = 0;
    uint8_t manual_terminal = 0;
    uint8_t unicode_wire[sizeof(ordinary_throw_error_bytecode)];
    uint8_t subtype_one_wire[sizeof(ordinary_throw_error_bytecode)];
    uint8_t subtype_max_wire[sizeof(ordinary_throw_error_bytecode)];
    size_t natural_raw_count = 0;
    size_t manual_raw_count = 0;
    int status = -1;

    compiled = JS_Eval(compile_context, natural_source,
                       strlen(natural_source), "ordinary-throw-error",
                       JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(compiled)) {
        report_exception(compile_context,
                         "natural throw_error compile failed");
        compiled = JS_UNDEFINED;
        goto cleanup;
    }
    natural_wire = JS_WriteObject(compile_context, &natural_wire_size,
                                  compiled, JS_WRITE_OBJ_BYTECODE);
    if (!natural_wire) {
        report_exception(compile_context, "natural throw_error write failed");
        goto cleanup;
    }
    if (natural_wire_size != sizeof(ordinary_throw_error_natural_bytecode) ||
        memcmp(natural_wire, ordinary_throw_error_natural_bytecode,
               natural_wire_size) != 0 ||
        ordinary_fnv1a64(natural_wire, natural_wire_size) !=
            UINT64_C(0x026914eda60a481f) ||
        ordinary_wire_child_metadata(natural_wire, natural_wire_size,
                                     &natural_child) ||
        !ordinary_metadata_equal(&natural_child,
                                 &expected_natural_metadata) ||
        natural_wire[44] != 0xb0 ||
        ordinary_collect_opcodes(natural_wire + natural_child.code_offset,
                                 natural_child.fields[ORD_CODE],
                                 natural_raws) ||
        ordinary_terminal_opcode(natural_wire + natural_child.code_offset,
                                 natural_child.fields[ORD_CODE],
                                 &natural_terminal)) {
        fputs("natural throw_error wire/metadata/opcodes drifted\n", stderr);
        goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++)
        natural_raw_count += natural_raws[raw] != 0;
    if (natural_raw_count != 6 || !natural_raws[17] ||
        !natural_raws[49] || !natural_raws[94] ||
        !natural_raws[179] || !natural_raws[180] ||
        !natural_raws[199] || natural_terminal != 49) {
        fputs("natural throw_error opcode set or terminal drifted\n", stderr);
        goto cleanup;
    }
    if (ordinary_expect_throw_error_wire(
            natural_wire, natural_wire_size, "natural throw_error",
            "TypeError", expected_x_message))
        goto cleanup;

    if (ordinary_fnv1a64(ordinary_throw_error_bytecode,
                         sizeof(ordinary_throw_error_bytecode)) !=
            UINT64_C(0xb4c1126c283093af) ||
        ordinary_wire_child_metadata(ordinary_throw_error_bytecode,
                                     sizeof(ordinary_throw_error_bytecode),
                                     &manual_child) ||
        !ordinary_metadata_equal(&manual_child,
                                 &expected_manual_metadata) ||
        ordinary_collect_opcodes(
            ordinary_throw_error_bytecode + manual_child.code_offset,
            manual_child.fields[ORD_CODE], manual_raws) ||
        ordinary_terminal_opcode(
            ordinary_throw_error_bytecode + manual_child.code_offset,
            manual_child.fields[ORD_CODE], &manual_terminal)) {
        fputs("manual throw_error wire/metadata/opcodes drifted\n", stderr);
        goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++)
        manual_raw_count += manual_raws[raw] != 0;
    if (manual_raw_count != 1 || !manual_raws[49] ||
        manual_terminal != 49) {
        fputs("manual throw_error opcode set or terminal drifted\n", stderr);
        goto cleanup;
    }

    runtime = JS_NewRuntime();
    defining_context = runtime ? JS_NewContext(runtime) : NULL;
    caller_context = runtime ? JS_NewContext(runtime) : NULL;
    if (!defining_context || !caller_context) {
        fputs("throw_error realm allocation failed\n", stderr);
        goto cleanup;
    }
    loaded = JS_ReadObject(defining_context, ordinary_throw_error_bytecode,
                           sizeof(ordinary_throw_error_bytecode),
                           JS_READ_OBJ_BYTECODE);
    if (JS_IsException(loaded)) {
        report_exception(defining_context, "manual throw_error read failed");
        loaded = JS_UNDEFINED;
        goto cleanup;
    }
    rewritten = JS_WriteObject(defining_context, &rewritten_size, loaded,
                               JS_WRITE_OBJ_BYTECODE);
    if (!rewritten ||
        rewritten_size != sizeof(ordinary_throw_error_bytecode) ||
        memcmp(rewritten, ordinary_throw_error_bytecode,
               rewritten_size) != 0) {
        if (!rewritten)
            report_exception(defining_context,
                             "manual throw_error rewrite failed");
        else
            fputs("manual throw_error rewrite drifted\n", stderr);
        goto cleanup;
    }
    function = JS_EvalFunction(defining_context, loaded);
    loaded = JS_UNDEFINED;
    if (JS_IsException(function) ||
        !JS_IsFunction(defining_context, function)) {
        report_exception(defining_context,
                         "manual throw_error root evaluation failed");
        function = JS_UNDEFINED;
        goto cleanup;
    }

    defining_global = JS_GetGlobalObject(defining_context);
    caller_global = JS_GetGlobalObject(caller_context);
    defining_type_error = JS_GetPropertyStr(defining_context,
                                            defining_global, "TypeError");
    caller_type_error = JS_GetPropertyStr(caller_context,
                                          caller_global, "TypeError");
    if (JS_IsException(defining_global) || JS_IsException(caller_global) ||
        JS_IsException(defining_type_error) ||
        JS_IsException(caller_type_error) ||
        !JS_IsFunction(defining_context, defining_type_error) ||
        !JS_IsFunction(caller_context, caller_type_error)) {
        report_exception(caller_context,
                         "throw_error realm constructor setup failed");
        goto cleanup;
    }
    if (ordinary_expect_throw_error_call(
            caller_context, function, defining_type_error,
            caller_type_error, expected_x_message,
            "manual throw_error subtype 0"))
        goto cleanup;

    memcpy(unicode_wire, ordinary_throw_error_bytecode,
           sizeof(unicode_wire));
    unicode_wire[3] = 0xe9;
    if (ordinary_fnv1a64(unicode_wire, sizeof(unicode_wire)) !=
        UINT64_C(0xb733634a7dff678e)) {
        fputs("Unicode throw_error wire drifted\n", stderr);
        goto cleanup;
    }
    unicode_loaded = JS_ReadObject(defining_context, unicode_wire,
                                   sizeof(unicode_wire),
                                   JS_READ_OBJ_BYTECODE);
    if (JS_IsException(unicode_loaded)) {
        report_exception(defining_context, "Unicode throw_error read failed");
        unicode_loaded = JS_UNDEFINED;
        goto cleanup;
    }
    unicode_rewritten = JS_WriteObject(defining_context,
                                       &unicode_rewritten_size,
                                       unicode_loaded,
                                       JS_WRITE_OBJ_BYTECODE);
    if (!unicode_rewritten || unicode_rewritten_size != sizeof(unicode_wire) ||
        memcmp(unicode_rewritten, unicode_wire, sizeof(unicode_wire)) != 0) {
        if (!unicode_rewritten)
            report_exception(defining_context,
                             "Unicode throw_error rewrite failed");
        else
            fputs("Unicode throw_error rewrite drifted\n", stderr);
        goto cleanup;
    }
    unicode_function = JS_EvalFunction(defining_context, unicode_loaded);
    unicode_loaded = JS_UNDEFINED;
    if (JS_IsException(unicode_function) ||
        !JS_IsFunction(defining_context, unicode_function)) {
        report_exception(defining_context,
                         "Unicode throw_error root evaluation failed");
        unicode_function = JS_UNDEFINED;
        goto cleanup;
    }
    if (ordinary_expect_throw_error_call(
            caller_context, unicode_function, defining_type_error,
            caller_type_error, expected_unicode_message,
            "Unicode throw_error subtype 0"))
        goto cleanup;

    if (JS_SetPropertyStr(caller_context, caller_global,
                          "__stage3eThrowError",
                          JS_DupValue(caller_context,
                                      unicode_function)) < 0 ||
        JS_SetPropertyStr(caller_context, caller_global,
                          "__stage3eDefiningTypeError",
                          JS_DupValue(caller_context,
                                      defining_type_error)) < 0) {
        report_exception(caller_context,
                         "throw_error caller publication failed");
        goto cleanup;
    }
    semantic_result = JS_Eval(caller_context, semantic_source,
                              strlen(semantic_source),
                              "ordinary-throw-error-semantic.js",
                              JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(semantic_result) ||
        JS_HasException(caller_context) ||
        JS_VALUE_GET_TAG(semantic_result) != JS_TAG_INT ||
        JS_VALUE_GET_INT(semantic_result) != 42) {
        report_exception(caller_context,
                         "throw_error caller catch oracle failed");
        semantic_result = JS_UNDEFINED;
        goto cleanup;
    }

    memcpy(subtype_one_wire, ordinary_throw_error_bytecode,
           sizeof(subtype_one_wire));
    memcpy(subtype_max_wire, ordinary_throw_error_bytecode,
           sizeof(subtype_max_wire));
    subtype_one_wire[sizeof(subtype_one_wire) - 1] = 1;
    subtype_max_wire[sizeof(subtype_max_wire) - 1] = UINT8_MAX;
    if (ordinary_expect_throw_error_wire(
            subtype_one_wire, sizeof(subtype_one_wire),
            "throw_error subtype 1", "SyntaxError",
            "redeclaration of 'x'") ||
        ordinary_expect_throw_error_wire(
            subtype_max_wire, sizeof(subtype_max_wire),
            "throw_error subtype 255", "InternalError",
            "invalid throw var type 255"))
        goto cleanup;

    puts("ordinary-throw-error-evidence="
         "compiler-natural-plus-mechanically-derived-property-free-wire");
    puts("ordinary-throw-error-natural-source-hex="
         "2866756e6374696f6e28297b2775736520737472696374273b636f6e737420"
         "783d303b783d313b7d29");
    puts("ordinary-throw-error-natural-compile-mode="
         "global-compile-only,strip-debug");
    printf("ordinary-throw-error-natural-wire-size=%zu\n",
           natural_wire_size);
    printf("ordinary-throw-error-natural-wire-fnv1a64=%016" PRIx64 "\n",
           ordinary_fnv1a64(natural_wire, natural_wire_size));
    puts("ordinary-throw-error-natural-wire-sha256="
         "a07b3f39a5e3929af4899a07686e91324e4ee9c54b729f518813eaa4a1875199");
    fputs("ordinary-throw-error-natural-wire-hex=", stdout);
    for (size_t index = 0; index < natural_wire_size; index++)
        printf("%02x", natural_wire[index]);
    putchar('\n');
    printf("ordinary-throw-error-natural-child-metadata="
           "flags:%04x,js_mode:%u,args:%" PRIu32 ",vars:%" PRIu32
           ",defined_args:%" PRIu32 ",stack:%" PRIu32
           ",var_refs:%" PRIu32 ",closures:%" PRIu32
           ",cpool:%" PRIu32 ",code:%" PRIu32 ",locals:%" PRIu32
           ",code_offset:%zu\n",
           natural_child.flags, natural_child.js_mode,
           natural_child.fields[ORD_ARGS], natural_child.fields[ORD_VARS],
           natural_child.fields[ORD_DEFINED_ARGS],
           natural_child.fields[ORD_STACK],
           natural_child.fields[ORD_VAR_REFS],
           natural_child.fields[ORD_CLOSURES],
           natural_child.fields[ORD_CPOOL],
           natural_child.fields[ORD_CODE],
           natural_child.fields[ORD_LOCALS], natural_child.code_offset);
    puts("ordinary-throw-error-natural-local-flags=b0");
    puts("ordinary-throw-error-natural-child-code-hex="
         "5e0000b3c7b41131f300000000");
    ordinary_print_raw_set("ordinary-throw-error-natural-child-raw",
                           natural_raws);
    puts("ordinary-throw-error-natural-terminal=raw49/subtype0,stack:0->0,no-return");
    puts("ordinary-throw-error-natural-rewrite=identity,fresh-runtime-exec:TypeError");
    puts("ordinary-throw-error-natural-provenance="
         "strict-const-assignment;source-only-for-Rust-admission");
    puts("ordinary-throw-error-natural-ordinary-cohort-exclusion="
         "lexical-vars:1,locals:1,local-flags:b0,raw94:set_loc_uninitialized");

    printf("ordinary-throw-error-wire-size=%zu\n",
           sizeof(ordinary_throw_error_bytecode));
    printf("ordinary-throw-error-wire-fnv1a64=%016" PRIx64 "\n",
           ordinary_fnv1a64(ordinary_throw_error_bytecode,
                            sizeof(ordinary_throw_error_bytecode)));
    puts("ordinary-throw-error-wire-sha256="
         "d05cabd4c18598b024f66eab8fd723c412fc5a469325b26fca5042507dea3ee8");
    fputs("ordinary-throw-error-wire-hex=", stdout);
    for (size_t index = 0;
         index < sizeof(ordinary_throw_error_bytecode); index++)
        printf("%02x", ordinary_throw_error_bytecode[index]);
    putchar('\n');
    printf("ordinary-throw-error-child-metadata="
           "flags:%04x,js_mode:%u,args:%" PRIu32 ",vars:%" PRIu32
           ",defined_args:%" PRIu32 ",stack:%" PRIu32
           ",var_refs:%" PRIu32 ",closures:%" PRIu32
           ",cpool:%" PRIu32 ",code:%" PRIu32 ",locals:%" PRIu32
           ",code_offset:%zu\n",
           manual_child.flags, manual_child.js_mode,
           manual_child.fields[ORD_ARGS], manual_child.fields[ORD_VARS],
           manual_child.fields[ORD_DEFINED_ARGS],
           manual_child.fields[ORD_STACK],
           manual_child.fields[ORD_VAR_REFS],
           manual_child.fields[ORD_CLOSURES],
           manual_child.fields[ORD_CPOOL], manual_child.fields[ORD_CODE],
           manual_child.fields[ORD_LOCALS], manual_child.code_offset);
    puts("ordinary-throw-error-child-code-hex=31f300000000");
    ordinary_print_raw_set("ordinary-throw-error-child-raw", manual_raws);
    puts("ordinary-throw-error-terminal=raw49/subtype0,stack:0->0,no-return");
    puts("ordinary-throw-error-derivation="
         "natural58:vars1->0,stack2->0,code13->6,locals1->0;"
         "remove-local-record:000000b0;"
         "remove-code-prefix:5e0000b3c7b411;retain-atom-slot:x-and-raw49");
    puts("ordinary-throw-error-property-free="
         "args:0,vars:0,var_refs:0,closures:0,cpool:0,locals:0,stack:0");
    puts("ordinary-throw-error-rewrite=identity,fresh-root:Function");
    puts("ordinary-throw-error-empty-stack="
         "metadata-max-stack:0;raw49:0->0;TypeError-not-underflow");
    puts("ordinary-throw-error-subtype0="
         "TypeError:'x'-is-read-only;terminal-no-return");
    puts("ordinary-throw-error-unicode-wire="
         "atom:x->U+00E9;size:47;rewrite:identity;"
         "fnv1a64:b733634a7dff678e;"
         "sha256:8228fdf15ff5551e6e14bac89e91d606c2aba6fe5d7ded834c309830842fd324");
    puts("ordinary-throw-error-unicode-message="
         "TypeError:'U+00E9'-is-read-only;utf8-hex:27c3a92720697320726561642d6f6e6c79");
    puts("ordinary-throw-error-realm="
         "defining-TypeError:true;caller-TypeError:false");
    puts("ordinary-throw-error-backtrace="
         "own-stack-before-catch;anonymous-frame-present");
    puts("ordinary-throw-error-pending="
         "direct-call-publishes;GetException-clears;caller-catch-clears");
    puts("ordinary-throw-error-caller-catch="
         "Unicode-TypeError:defining-realm;terminal-no-return;result:42");
    puts("ordinary-throw-error-subtype1="
         "fresh-read-write-exec:SyntaxError:redeclaration-of-x;Rust:Unadmitted");
    puts("ordinary-throw-error-subtype255="
         "fresh-read-write-exec:InternalError:invalid-throw-var-type-255;Rust:Unadmitted");
    puts("ordinary-throw-error-rust-admission="
         "raw49/subtype0-only;subtype1-255:Unadmitted");
    puts("ordinary-exception-admitted-count=2");
    puts("ordinary-exception-admitted-raw=48,49");
    puts("ordinary-exception-deferred-count=1");
    puts("ordinary-exception-deferred-raw=177");
    puts("ordinary-exception-deferred-detail="
         "raw177:nop-specialized-blocked");
    puts("ordinary-throw-error-oracle=passed");
    status = 0;

cleanup:
    if (unicode_rewritten && defining_context)
        js_free(defining_context, unicode_rewritten);
    if (rewritten && defining_context)
        js_free(defining_context, rewritten);
    if (caller_context) {
        if (JS_HasException(caller_context)) {
            JSValue pending = JS_GetException(caller_context);
            JS_FreeValue(caller_context, pending);
        }
        JS_FreeValue(caller_context, semantic_result);
        JS_FreeValue(caller_context, caller_type_error);
        JS_FreeValue(caller_context, caller_global);
        JS_FreeContext(caller_context);
    }
    if (defining_context) {
        if (JS_HasException(defining_context)) {
            JSValue pending = JS_GetException(defining_context);
            JS_FreeValue(defining_context, pending);
        }
        JS_FreeValue(defining_context, defining_type_error);
        JS_FreeValue(defining_context, defining_global);
        JS_FreeValue(defining_context, unicode_function);
        JS_FreeValue(defining_context, unicode_loaded);
        JS_FreeValue(defining_context, function);
        JS_FreeValue(defining_context, loaded);
        JS_FreeContext(defining_context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
    if (natural_wire)
        js_free(compile_context, natural_wire);
    JS_FreeValue(compile_context, compiled);
    return status;
}

static int expect_ordinary_expansion_cohort(JSContext *compile_context) {
    static const char unary_binary_sequence[] =
        "-6,6,7,6,-7,false,number,18,0,216,48,0,0,"
        "false,false,true,false,true,true,2,5,7";
    enum {
        IMPLICIT_CASE, PRIMITIVES_CASE, PREDICATES_CASE, CALLS_CASE,
        UNARY_BINARY_CASE, BRANCHES_UPDATES_CASE, WIDE_IF_TRUE_CASE,
    };
    JSRuntime *runtime = NULL;
    JSContext *context = NULL;
    JSValue functions[sizeof(ordinary_expansion_cases) /
                      sizeof(ordinary_expansion_cases[0])];
    JSValue sink = JS_UNDEFINED;
    JSValue html_dda = JS_UNDEFINED;
    JSValue normal = JS_UNDEFINED;
    uint8_t compiler_raws[256] = { 0 };
    uint8_t atom_free_raws[256] = { 0 };
    uint8_t call_raws[256] = { 0 };
    uint8_t emitted_raws[256] = { 0 };
    uint8_t missing_raws[256] = { 0 };
    uint8_t manual_raws[256] = { 0 };
    size_t emitted_count = 0;
    size_t compiler_raw_count = 0;
    size_t atom_free_emitted_count = 0;
    size_t call_emitted_count = 0;
    int status = -1;

    for (size_t index = 0;
         index < sizeof(functions) / sizeof(functions[0]); index++)
        functions[index] = JS_UNDEFINED;
    if (ordinary_build_raw_set("ordinary atom-free raws",
                               ordinary_expansion_atom_free_raws,
                               sizeof(ordinary_expansion_atom_free_raws),
                               atom_free_raws) ||
        ordinary_build_raw_set("ordinary plain-call raws",
                               ordinary_expansion_call_raws,
                               sizeof(ordinary_expansion_call_raws),
                               call_raws))
        goto cleanup;
    for (size_t index = 0;
         index < sizeof(ordinary_stack_cases) /
                     sizeof(ordinary_stack_cases[0]); index++) {
        uint8_t raw = ordinary_stack_cases[index].raw;
        if (manual_raws[raw]) {
            fprintf(stderr, "ordinary manual stack cases duplicate raw %u\n",
                    raw);
            goto cleanup;
        }
        manual_raws[raw] = 1;
    }
    for (unsigned raw = 0; raw < 256; raw++) {
        if (atom_free_raws[raw] && call_raws[raw]) {
            fputs("ordinary expansion cohort overlap\n", stderr);
            goto cleanup;
        }
    }
    runtime = JS_NewRuntime();
    context = runtime ? JS_NewContext(runtime) : NULL;
    if (!context) {
        fputs("ordinary expansion fresh runtime allocation failed\n", stderr);
        goto cleanup;
    }
    for (size_t index = 0;
         index < sizeof(ordinary_expansion_cases) /
                     sizeof(ordinary_expansion_cases[0]); index++) {
        if (ordinary_compile_load_case(compile_context, context,
                                       &ordinary_expansion_cases[index],
                                       &functions[index], compiler_raws, 0))
            goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++) {
        compiler_raw_count += compiler_raws[raw] != 0;
        if ((atom_free_raws[raw] || call_raws[raw]) && compiler_raws[raw]) {
            emitted_raws[raw] = 1;
            emitted_count++;
            atom_free_emitted_count += atom_free_raws[raw] != 0;
            call_emitted_count += call_raws[raw] != 0;
        } else if (atom_free_raws[raw] || call_raws[raw]) {
            missing_raws[raw] = 1;
        }
    }
    if (compiler_raw_count != 63 || emitted_count != 45 ||
        atom_free_emitted_count != 40 || call_emitted_count != 5) {
        fputs("ordinary compiler/manual evidence split drifted\n", stderr);
        goto cleanup;
    }
    for (unsigned raw = 0; raw < 256; raw++) {
        if (missing_raws[raw] != manual_raws[raw]) {
            fputs("ordinary manual stack evidence split drifted\n", stderr);
            goto cleanup;
        }
    }
    puts("ordinary-expansion-evidence=compile-only-write-read-write-fresh-runtime");
    puts("ordinary-expansion-atom-free-status=upstream-evidence-for-rust-admission");
    puts("ordinary-expansion-atom-free-count=57");
    ordinary_print_raw_set("ordinary-expansion-atom-free-raw",
                           atom_free_raws);
    puts("ordinary-expansion-plain-call-status=upstream-evidence-for-rust-admission");
    puts("ordinary-expansion-plain-call-count=5");
    ordinary_print_raw_set("ordinary-expansion-plain-call-raw", call_raws);
    puts("ordinary-expansion-physical-row-count=62");
    puts("ordinary-expansion-compiler-all-count=63");
    ordinary_print_raw_set("ordinary-expansion-compiler-all-raw",
                           compiler_raws);
    puts("ordinary-expansion-compiler-new-count=45");
    ordinary_print_raw_set("ordinary-expansion-compiler-new-raw",
                           emitted_raws);
    puts("ordinary-expansion-compiler-new-atom-free-count=40");
    puts("ordinary-expansion-compiler-new-plain-call-count=5");
    puts("ordinary-expansion-manual-stack-count=17");
    ordinary_print_raw_set("ordinary-expansion-manual-stack-raw",
                           missing_raws);
    puts("ordinary-expansion-manual-stack-provenance="
         "authenticated-wire-not-compiler-emitted");
    puts("ordinary-expansion-stack-evidence-split="
         "compiler:14,17;manual:15,16,18-32");
    if (ordinary_expect_call(context, functions[IMPLICIT_CASE], 0, NULL,
                             ORDINARY_CALL_UNDEFINED, 0,
                             "return_undef execution"))
        goto cleanup;
    ordinary_primitive_index = 0;
    ordinary_plain_receiver_count = 0;
    ordinary_sink_mode = 0;
    ordinary_bigint_tag = ordinary_float_tag = ordinary_float_norm_tag = -1;
    ordinary_float_bits = 0;
    sink = JS_NewCFunction(context, ordinary_sink,
                           "ordinaryPrimitiveSink", 1);
    if (ordinary_expect_call(context, functions[PRIMITIVES_CASE], 1, &sink,
                             ORDINARY_CALL_INT, 7,
                             "primitive cohort execution") ||
        ordinary_primitive_index != 7)
        goto cleanup;
    JS_FreeValue(context, sink);
    sink = JS_UNDEFINED;
    html_dda = JS_NewCFunction(context, ordinary_html_dda_call, "htmlDDA", 0);
    normal = JS_NewCFunction(context, ordinary_html_dda_call, "normal", 0);
    JS_SetIsHTMLDDA(context, html_dda);
    {
        JSValueConst values[] = {
            html_dda, html_dda, html_dda, html_dda, html_dda, normal, normal,
        };
        static const uint8_t selectors[] = { 0, 1, 2, 3, 4, 2, 3 };
        static const uint8_t expected[] = { 0, 0, 1, 0, 1, 0, 1 };
        for (size_t index = 0; index < sizeof(expected); index++) {
            if (ordinary_boolean_result(context, functions[PREDICATES_CASE],
                                        values[index], selectors[index]) !=
                expected[index]) {
                fputs("ordinary HTMLDDA predicate matrix drifted\n", stderr);
                goto cleanup;
            }
        }
    }
    ordinary_call_index = 0;
    ordinary_sink_mode = 1;
    sink = JS_NewCFunction(context, ordinary_sink,
                           "ordinaryCallSink", 4);
    {
        static const int values[] = { 11, 22, 33, 44 };
        if (ordinary_expect_i32_call(context, functions[CALLS_CASE], sink,
                                     values, 4, 10,
                                     "plain call cohort execution"))
            goto cleanup;
    }
    if (ordinary_call_index != 5)
        goto cleanup;
    JS_FreeValue(context, sink);
    sink = JS_UNDEFINED;
    ordinary_sink_mode = 2;
    sink = JS_NewCFunction(context, ordinary_sink,
                           "ordinaryGenericSink", 1);
    {
        static const int unary_values[] = { 6, 3 };
        static const int branch_values[] = { 0, 5 };
        ordinary_sink_sequence_length = 0;
        ordinary_sink_sequence[0] = '\0';
        if (ordinary_expect_i32_call(context,
                                     functions[UNARY_BINARY_CASE], sink,
                                     unary_values, 2, 42,
                                     "unary/binary execution") ||
            strcmp(ordinary_sink_sequence, unary_binary_sequence) != 0)
            goto cleanup;
        ordinary_sink_sequence_length = 0;
        ordinary_sink_sequence[0] = '\0';
        if (ordinary_expect_i32_call(context,
                                     functions[BRANCHES_UPDATES_CASE], sink,
                                     branch_values, 2, 4,
                                     "branch/update execution") ||
            strcmp(ordinary_sink_sequence, "0,5") != 0)
            goto cleanup;
    }
    JS_FreeValue(context, sink);
    sink = JS_NewCFunction(context, ordinary_html_dda_call,
                           "ordinaryUnreachedSink", 0);
    {
        JSValue arguments[2] = { sink, JS_TRUE };
        if (ordinary_expect_call(context, functions[WIDE_IF_TRUE_CASE], 2,
                                 arguments, ORDINARY_CALL_BOOL, 1,
                                 "wide if_true execution"))
            goto cleanup;
    }
    if (ordinary_plain_receiver_count != 36) {
        fputs("plain receiver observation count drifted\n", stderr);
        goto cleanup;
    }
    puts("ordinary-expansion-return-undef-max-stack=0");
    puts("ordinary-expansion-return-undef-fresh-eval=undefined");
    puts("ordinary-expansion-html-dda=exact-undefined:false,exact-null:false,"
         "typeof-undefined:true,typeof-function:false,equals-null:true");
    puts("ordinary-expansion-normal-function="
         "typeof-undefined:false,typeof-function:true");
    puts("ordinary-expansion-plain-call-this=undefined");
    puts("ordinary-expansion-plain-call-argc=0,1,2,3,4");
    puts("ordinary-expansion-plain-call-undefined-receiver-count=36");
    printf("ordinary-expansion-bigint-tag=%d,signed-i32=7\n",
           ordinary_bigint_tag);
    printf("ordinary-expansion-float64-tag=%d,normalized-tag=%d,bits=%016"
           PRIx64 "\n", ordinary_float_tag, ordinary_float_norm_tag,
           ordinary_float_bits);
    printf("ordinary-expansion-unary-binary-sink-sequence=%s\n",
           unary_binary_sequence);
    puts("ordinary-expansion-unary-binary-fresh-eval=42");
    puts("ordinary-expansion-branches-updates-fresh-eval=4");
    puts("ordinary-expansion-wide-if-true-fresh-eval=true");
    for (size_t index = 0;
         index < sizeof(ordinary_stack_cases) /
                     sizeof(ordinary_stack_cases[0]); index++) {
        if (expect_ordinary_stack_case(&ordinary_stack_cases[index]))
            goto cleanup;
    }
    puts("ordinary-expansion-compile-only-oracle=passed");
    status = 0;
cleanup:
    if (context) {
        JS_FreeValue(context, normal);
        JS_FreeValue(context, html_dda);
        JS_FreeValue(context, sink);
        for (size_t index = 0;
             index < sizeof(functions) / sizeof(functions[0]); index++)
            JS_FreeValue(context, functions[index]);
        JS_FreeContext(context);
    }
    if (runtime)
        JS_FreeRuntime(runtime);
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
    if (expect_ordinary_expansion_cohort(compile_context))
        goto cleanup;
    if (expect_ordinary_invocation_cohort(compile_context))
        goto cleanup;
    if (expect_ordinary_throw_completion(compile_context))
        goto cleanup;
    if (expect_ordinary_throw_error_completion(compile_context))
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
