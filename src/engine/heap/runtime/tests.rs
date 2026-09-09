use super::{
    ActiveFrameFlags, ActiveFrameKind, DeferredRefOp, PropertyGetAction, PropertySetAction,
    Runtime, RuntimeError,
};
use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind, NativeErrorMessage};

use crate::engine::api::{EvalOptions, JsBigInt};
use crate::engine::builtins::native::{
    ArrayJoinKind, DynamicFunctionKind, FunctionDebugPosition, NativeCProto, NativeFunctionId,
    PrimitiveKind,
};
use crate::engine::code::bytecode::{ApplyKind, DetachedBytecode, Instruction};
use crate::engine::code::debug::{DebugInfoMode, Pc2LineEntry, Pc2LineTable};
use crate::engine::code::dynamic_source::DynamicSourceBuilder;
use crate::source::LineColumn;

use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
    EvalBinding, EvalBindingSource, EvalEnvironment, EvalKind, EvalScope, EvalScopeKind,
    EvalVariableEnvironment, FunctionKind, FunctionMetadata,
};
use crate::engine::code::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::engine::compiler::{
    CompileOptions, EvalCompileContext, compile_unlinked_eval_with_filename,
    compile_unlinked_module_with_filename,
};
use crate::engine::heap::roots::VarRefRoot;
use crate::engine::heap::{
    BytecodeConstant, HeapError, ObjectPayload, PrimitiveObjectData, PropertySlot, RawValue,
};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField,
    OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
};
use crate::engine::value::{JsString, JsStringError, Value};
use crate::engine::vm::call::CallableExecution;
use crate::engine::vm::host_bridge::RuntimeVmHost;

use crate::engine::vm::{
    Completion, DirectEvalInvocation, IteratorCloseOutcome, ToPrimitiveHint, Vm, VmHost,
};

const QUICKJS_SCALAR_42_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
];

// QuickJS 2026-06-04, GLOBAL | COMPILE_ONLY with JS_STRIP_DEBUG, for:
// (function(a,b){var acc=.5;var step=b;while(a>0){if(a===2)
// acc=(acc+step)/1;else acc=(acc+1)/1;a=a-1;}return acc===5.5?42:0;})
const QUICKJS_ORDINARY_LEAF_42_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x02, 0x02,
    0x02, 0x02, 0x00, 0x00, 0x02, 0x2e, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xbd, 0x00, 0xc7, 0xd0, 0xc8, 0xcf, 0xb3, 0xa3, 0xe8,
    0x1a, 0xcf, 0xb5, 0xa9, 0xe8, 0x09, 0xc3, 0xc4, 0x9b, 0xb4, 0x99, 0xc7, 0xea, 0x07, 0xc3, 0xb4,
    0x9b, 0xb4, 0x99, 0xc7, 0xcf, 0xb4, 0x9c, 0xd3, 0xea, 0xe3, 0xc3, 0xbd, 0x01, 0xa9, 0xe8, 0x04,
    0xbb, 0x2a, 0x28, 0xb3, 0x28, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe0, 0x3f, 0x06, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x16, 0x40,
];

const QUICKJS_ORDINARY_PLAIN_CALLS_CODE: &[u8] = &[
    0xcf, 0xec, 0x0e, // get_arg0; call0; drop
    0xcf, 0xb4, 0xed, 0x0e, // get_arg0; push_1; call1; drop
    0xcf, 0xb4, 0xb5, 0xee, 0x0e, // get_arg0; push_1; push_2; call2; drop
    0xcf, 0xb4, 0xb5, 0xb6, 0xef, 0x0e, // get_arg0; push_1; push_2; push_3; call3; drop
    0xcf, 0xb4, 0xb5, 0xb6, 0xb7, 0x22, 0x04, 0x00,
    0x28,
    // get_arg0; push_1..push_4; call 4; return
];

const QUICKJS_ORDINARY_CONSTRUCT_CODE: &[u8] = &[
    0xcf, 0xd0, 0xb4, 0xb5, 0x21, 0x02, 0x00,
    0x28,
    // get_arg0; get_arg1; push_1; push_2; call_constructor 2; return
];

const QUICKJS_ORDINARY_CALL_METHOD_CODE: &[u8] = &[
    0xcf, 0xd0, 0xbb, 0x2a, 0x24, 0x01, 0x00,
    0x28,
    // get_arg0; get_arg1; push_i8 42; call_method 1; return
];

const QUICKJS_ORDINARY_ARRAY_FROM_CODE: &[u8] = &[
    0xb4, 0xb5, 0xb6, 0x26, 0x03, 0x00, 0x28,
    // push_1; push_2; push_3; array_from 3; return
];

// QuickJS 2026-06-04 qjsc -c -s (GLOBAL | COMPILE_ONLY with JS_STRIP_DEBUG)
// for `(function(a,b,c){ "use strict"; return a; })`. The child is anonymous,
// strict, and has three simple parameters. Its code begins at byte 51; tests
// replace only that authenticated code suffix and its declared stack/code
// lengths.
const QUICKJS_ORDINARY_THREE_ARGUMENT_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x03, 0x00,
    0x03, 0x01, 0x00, 0x00, 0x00, 0x02, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0xcf, 0x28,
];

// QuickJS 2026-06-04 qjsc -c -s for
// `(function(f,a,b){'use strict';return f(a,b);})`. The compiler folds the
// return into raw 35 (`tail_call`) and emits no separate return opcode.
const QUICKJS_ORDINARY_TAIL_CALL_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x03, 0x00,
    0x03, 0x03, 0x00, 0x00, 0x00, 0x06, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0xcf, 0xd0, 0xd1, 0x23, 0x02, 0x00,
];

// Property-free manual raw-37 wire authenticated from QuickJS's compiler
// output for an anonymous strict four-argument function. Its exact code is
// get_arg0..3; tail_call_method 2, with no property-producing opcode.
const QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x04, 0x00,
    0x04, 0x04, 0x00, 0x00, 0x00, 0x07, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xcf, 0xd0, 0xd1, 0xd2, 0x25, 0x02, 0x00,
];

// QuickJS 2026-06-04 qjsc -c -s for
// `(function(a){"use strict";throw a;})`. The anonymous child has one simple
// parameter and its complete body is the natural get_arg0; throw sequence
// ending in raw 48, with no synthetic return.
const QUICKJS_ORDINARY_THROW_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x30,
];

// QuickJS 2026-06-04 qjsc -c -s (GLOBAL | COMPILE_ONLY with JS_STRIP_DEBUG)
// for `(function(){'use strict';return {};})`. The child is an anonymous
// strict ordinary function with no atoms, constants, locals, or closures. Its
// complete compiler-natural body is raw11 (`object`); raw40 (`return`).
const QUICKJS_ORDINARY_OBJECT_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x0b, 0x28,
];

// QuickJS 2026-06-04 qjsc -c -s for the compiler-natural source
// `(function(a){'use strict';({}=a);return a;})`. The exact 56-byte wire has
// SHA-256 f5bdac14901bb6b752e2ca10a01dd31d6990456c43f78d5923b1da4a0ef3706e
// and provides the compiler provenance for the isolated manual wire below.
const QUICKJS_NATURAL_TO_OBJECT_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x02, 0x00, 0x00, 0x00, 0x0d, 0x01, 0x00, 0x01, 0x00, 0x00, 0xea, 0x06, 0x11, 0x6f, 0x0e,
    0xea, 0x04, 0xcf, 0xea, 0xf9, 0x0e, 0xcf, 0x28,
];

// Property-free manual wire mechanically reduced from the natural envelope.
// Its exact get_arg0; to_object; return body is cf6f28, and the 46-byte image
// has SHA-256 13f81e66520578393a57f3290636d4778c5cae8d014591e5daaaacdd3ffd5c95.
const QUICKJS_ORDINARY_TO_OBJECT_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x6f, 0x28,
];

// QuickJS 2026-06-04 qjsc -c -s for
// `(function(a){'use strict';return {[a]:1};})` and
// `(function(a){return {[a]:1};})`. These authenticate raw112 in source order,
// while raw78 (`define_array_el`) deliberately keeps both whole natural
// functions outside the current ordinary-leaf cohort. Their SHA-256 values are
// respectively
// 7bfb0fefdbd3ff894bdcc0996707fda98153aaaeccbe50f6ade1ffaab7f818f0 and
// c5f7a85af861402d57a8267f9af1be2d310b143a6972e5fe5d2068384b9f8fe0.
const QUICKJS_NATURAL_STRICT_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x03, 0x00, 0x00, 0x00, 0x07, 0x01, 0x00, 0x01, 0x00, 0x00, 0x0b, 0xcf, 0x70, 0xb4, 0x4e,
    0x0e, 0x28,
];
const QUICKJS_NATURAL_SLOPPY_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x01, 0x00,
    0x01, 0x03, 0x00, 0x00, 0x00, 0x07, 0x01, 0x00, 0x01, 0x00, 0x00, 0x0b, 0xcf, 0x70, 0xb4, 0x4e,
    0x0e, 0x28,
];

// Property-free compiler-envelope reductions used to isolate the exact
// get_arg0; to_propkey; return typed chain in strict and sloppy mode. Their
// SHA-256 values are respectively
// 7be331650765c34157ea3731e6f86d082451e0d60e7aeb7ecd09abfe0d524cb4 and
// 629fa63ab5c4bd4258a44e02e4171a82c7cb23ca3bce1ce11d4228e4ee10d822.
const QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x70, 0x28,
];
const QUICKJS_ORDINARY_SLOPPY_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x70, 0x28,
];

// Executable admission probes for repeated raw112 and a finite backedge which
// targets raw112 itself. The SHA-256 values are respectively
// b64eab0222e609fc0f5c70a2183c7558b2eecc9852d5bb795c8933e90a351ff5 and
// 85274c3f09639ee7538bdfafd43f8bb35fc8819f9f2d4c8051e5fb140bccb638.
const QUICKJS_DUPLICATE_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x70, 0x70, 0x28,
];
const QUICKJS_REENTER_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x02, 0x00,
    0x02, 0x02, 0x00, 0x00, 0x00, 0x12, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xcf,
    0x70, 0xd0, 0x68, 0x0d, 0x00, 0x00, 0x00, 0x0e, 0x09, 0xd4, 0xcf, 0x6a, 0xf4, 0xff, 0xff, 0xff,
    0x28,
];

// Same authenticated manual envelope with the initial get_arg0 removed. Its
// SHA-256 is b96daff364d2ca615035e2910533e5e77b3284c52309c0d30e333275682bc841;
// the existing typed bytecode verifier must reject the raw112 stack underflow.
const QUICKJS_UNDERFLOW_TO_PROPKEY_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x01, 0x00,
    0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x00, 0x00, 0x70, 0x28,
];

// QuickJS 2026-06-04 qjsc -c -s for the compiler-natural strict and sloppy
// anonymous functions that return `this`. Both exact wires preserve raw8 as
// source instruction zero, followed by the compiler's local round trip. Their
// SHA-256 values are respectively
// 786376192d5bfe7eb07115f62788707619ee54e8721acfa66dae1d110a580e39 and
// f0430a7c241caaf94703bd5de73289d4f90fea3ee9cfaf22a660ed80df3de0a6.
const QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x01, 0x00, 0x00, 0x08, 0xc7, 0xc3, 0x28,
];
const QUICKJS_NATURAL_SLOPPY_PUSH_THIS_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x01, 0x00, 0x00, 0x08, 0xc7, 0xc3, 0x28,
];

// Property-free compiler-envelope reductions used to isolate the exact raw8;
// raw40 typed path. The SHA-256 values are respectively
// 9b14c5245a78e0a069967089cf6f89aefac3e12749d16eba36e4c15b72a3c99e and
// 213b3b6a332d4cf69e4c726b372c1f0087e70fc9c263a6a2193ce4763fb62648.
const QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x08, 0x28,
];
const QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x08, 0x28,
];

// Mechanically executable upstream mismatch probe. Repeating raw8 before
// strict_eq boxes a sloppy primitive receiver twice in QuickJS and returns
// false; reusing Oxide's per-call receiver cache would return true. The archive
// protocol rejects the duplicate before publication. SHA-256:
// 9f0541bfd8a599e5f2575936d24df9a2487a1e8952fca1648afeef5c9f798a30.
const QUICKJS_DUPLICATE_PUSH_THIS_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x08, 0x08, 0xa9, 0x28,
];

// Exact-once upstream mismatch probe. The final goto explicitly re-enters
// typed instruction zero; pinned QuickJS therefore performs a second sloppy
// primitive conversion and returns false, while Oxide's per-call receiver
// cache would reuse the first wrapper. The archive protocol rejects that
// alternate entry before publication. SHA-256:
// 32b4c9e45f5191d21aa44d3437c54b00cfa1ff4b2530d1e4cdf942a87e8f3fb4.
const QUICKJS_REENTER_PUSH_THIS_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x00, 0x00, 0x02, 0x00,
    0x02, 0x02, 0x00, 0x00, 0x00, 0x12, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x08,
    0xcf, 0x69, 0x0c, 0x00, 0x00, 0x00, 0x0a, 0xd3, 0xd4, 0x6a, 0xf5, 0xff, 0xff, 0xff, 0xd0, 0xa9,
    0x28,
];

// Smallest property-free BC5 ordinary-function wire for raw177 (`nop`) under
// pinned QuickJS 2026-06-04. The compiler removes authored nops, so the
// authenticated zero-argument strict envelope carries the exact synthetic
// raw177; raw41 (`return_undef`) body needed to execute it.
const QUICKJS_ORDINARY_NOP_BC5: &[u8] = &[
    0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x04,
    0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0x29,
];

// Property-free raw49/subtype0 wire mechanically reduced from pinned QuickJS
// output for `(function(){'use strict';const x=0;x=1;})`. The natural 58-byte
// compiler wire retains lexical-local metadata and raw94, so it intentionally
// remains outside the ordinary synchronous leaf cohort.
const QUICKJS_ORDINARY_READ_ONLY_BC5: &[u8] = &[
    0x05, 0x01, 0x02, 0x78, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
    0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x31, 0xf3, 0x00, 0x00, 0x00, 0x00,
];

const QUICKJS_NATURAL_READ_ONLY_BC5: &[u8] = &[
    0x05, 0x01, 0x02, 0x78, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
    0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbe, 0x00, 0xcb, 0x28, 0x0c, 0x43, 0x02, 0x01, 0x00,
    0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x0d, 0x01, 0x00, 0x00, 0x00, 0xb0, 0x5e, 0x00, 0x00,
    0xb3, 0xc7, 0xb4, 0x11, 0x31, 0xf3, 0x00, 0x00, 0x00, 0x00,
];

const QUICKJS_SELF_CONTAINED_MODULE_BC5: &[u8] = &[
    0x05, 0x03, 0x24, 0x73, 0x65, 0x6c, 0x66, 0x2d, 0x63, 0x6f, 0x6e, 0x74, 0x61, 0x69, 0x6e, 0x65,
    0x64, 0x2e, 0x6d, 0x6a, 0x73, 0x0c, 0x61, 0x6e, 0x73, 0x77, 0x65, 0x72, 0x2e, 0x5f, 0x5f, 0x6d,
    0x6f, 0x64, 0x75, 0x6c, 0x65, 0x42, 0x79, 0x74, 0x65, 0x63, 0x6f, 0x64, 0x65, 0x52, 0x65, 0x63,
    0x65, 0x69, 0x70, 0x74, 0x0d, 0xe6, 0x03, 0x00, 0x01, 0x00, 0x00, 0xe8, 0x03, 0x00, 0x00, 0x00,
    0x0c, 0x20, 0x02, 0x01, 0xa8, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x14, 0x00, 0xe8,
    0x03, 0x00, 0x1e, 0x00, 0xa0, 0x02, 0x00, 0x05, 0x00, 0x08, 0xe8, 0x02, 0x29, 0xbb, 0x2a, 0xdf,
    0x38, 0x01, 0x00, 0x64, 0x00, 0x00, 0x3f, 0xf5, 0x00, 0x00, 0x00, 0x06, 0x2f,
];

fn quickjs_scalar_with_code(code: &[u8]) -> Vec<u8> {
    let mut object = QUICKJS_SCALAR_42_BC5.to_vec();
    object[15] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
    object.splice(21.., code.iter().copied());
    object
}

fn quickjs_ordinary_with_code_and_constants(code: &[u8], constants: &[&[u8]]) -> Vec<u8> {
    let mut object = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    object[36] = u8::try_from(constants.len()).expect("test constant count fits one-byte ULEB");
    object[37] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
    object.truncate(55);
    object.extend_from_slice(code);
    for constant in constants {
        object.extend_from_slice(constant);
    }
    object
}

fn quickjs_ordinary_three_argument_with_code(code: &[u8], max_stack: u8) -> Vec<u8> {
    let mut object = QUICKJS_ORDINARY_THREE_ARGUMENT_BC5.to_vec();
    object[33] = max_stack;
    object[37] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
    object.truncate(51);
    object.extend_from_slice(code);
    object
}

fn quickjs_ordinary_four_argument_with_code(code: &[u8], max_stack: u8) -> Vec<u8> {
    let mut object = QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5.to_vec();
    object[33] = max_stack;
    object[37] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
    object.truncate(55);
    object.extend_from_slice(code);
    object
}

fn quickjs_ordinary_one_argument_with_code(code: &[u8], max_stack: u8) -> Vec<u8> {
    let mut object = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    object[33] = max_stack;
    object[37] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
    object.truncate(43);
    object.extend_from_slice(code);
    object
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn expect_string_value(value: Value) -> JsString {
    let Value::String(value) = value else {
        panic!("scalar String execution returned another value kind");
    };
    value
}

fn quickjs_scalar_with_float_constant(code: &[u8], bits: u64) -> Vec<u8> {
    let entry = quickjs_float_constant_entry(bits);
    quickjs_scalar_with_constants(code, &[entry.as_slice()])
}

fn quickjs_float_constant_entry(bits: u64) -> Vec<u8> {
    let mut entry = vec![0x06];
    entry.extend_from_slice(&bits.to_le_bytes());
    entry
}

fn quickjs_scalar_with_bigint_constant(code: &[u8], payload: &[u8]) -> Vec<u8> {
    let entry = quickjs_bigint_constant_entry(payload);
    quickjs_scalar_with_constants(code, &[entry.as_slice()])
}

fn quickjs_bigint_constant_entry(payload: &[u8]) -> Vec<u8> {
    let mut entry = vec![
        0x0a,
        u8::try_from(payload.len()).expect("test BigInt length fits one-byte ULEB"),
    ];
    entry.extend_from_slice(payload);
    entry
}

fn quickjs_scalar_with_string_constant(code: &[u8], units: &[u16], wide: bool) -> Vec<u8> {
    let entry = quickjs_string_constant_entry(units, wide);
    quickjs_scalar_with_constants(code, &[entry.as_slice()])
}

fn quickjs_string_constant_entry(units: &[u16], wide: bool) -> Vec<u8> {
    let mut entry = vec![0x07];
    entry.extend(quickjs_wire_string_entry(units, wide));
    entry
}

fn quickjs_scalar_with_atom_value(atom: u32) -> Vec<u8> {
    let mut code = vec![0x04];
    code.extend_from_slice(&atom.to_le_bytes());
    code.extend_from_slice(&[0xcb, 0x28]);
    quickjs_scalar_with_code(&code)
}

fn quickjs_scalar_with_atom_slot(units: &[u16], wide: bool) -> Vec<u8> {
    let mut object = quickjs_scalar_with_atom_value(243);
    object[1] = 1;
    object.splice(2..2, quickjs_wire_string_entry(units, wide));
    object
}

fn quickjs_scalar_with_unused_atom_slot(atom: u32, units: &[u16], wide: bool) -> Vec<u8> {
    let mut object = quickjs_scalar_with_atom_value(atom);
    object[1] = 1;
    object.splice(2..2, quickjs_wire_string_entry(units, wide));
    object
}

fn quickjs_scalar_with_two_atom_slots() -> Vec<u8> {
    let mut object = quickjs_scalar_with_atom_value(243);
    object[1] = 2;
    let mut slots = quickjs_wire_string_entry(&[u16::from(b'x')], false);
    slots.extend(quickjs_wire_string_entry(&[u16::from(b'y')], false));
    object.splice(2..2, slots);
    object
}

fn quickjs_wire_string_entry(units: &[u16], wide: bool) -> Vec<u8> {
    let length = u8::try_from(units.len()).expect("test String length fits one-byte ULEB");
    let mut entry = vec![(length << 1) | u8::from(wide)];
    if wide {
        for unit in units {
            entry.extend_from_slice(&unit.to_le_bytes());
        }
    } else {
        entry.extend(
            units
                .iter()
                .map(|unit| u8::try_from(*unit).expect("test narrow String contains only Latin-1")),
        );
    }
    entry
}

fn quickjs_scalar_with_constants(code: &[u8], constants: &[&[u8]]) -> Vec<u8> {
    let mut object = quickjs_scalar_with_code(code);
    object[14] = u8::try_from(constants.len()).expect("test constant count fits one-byte ULEB");
    for constant in constants {
        object.extend_from_slice(constant);
    }
    object
}

fn data_descriptor(
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(writable),
        enumerable: DescriptorField::Present(enumerable),
        configurable: DescriptorField::Present(configurable),
        ..OrdinaryPropertyDescriptor::new()
    }
}

fn set_property(
    runtime: &Runtime,
    object: &crate::engine::api::ObjectRef,
    key: &PropertyKey,
    value: Value,
) -> Result<bool, RuntimeError> {
    match runtime.prepare_set_property(object, key, value)? {
        PropertySetAction::Complete => Ok(true),
        PropertySetAction::Rejected(_) => Ok(false),
        PropertySetAction::Throw(_) => Err(RuntimeError::Invariant(
            "context-free property test produced a JavaScript throw",
        )),
        PropertySetAction::Call { .. } => Err(RuntimeError::Invariant(
            "ordinary-property test helper unexpectedly reached a setter",
        )),
    }
}

fn set_property_with_receiver(
    runtime: &Runtime,
    object: &crate::engine::api::ObjectRef,
    key: &PropertyKey,
    value: Value,
    receiver: Value,
) -> Result<bool, RuntimeError> {
    match runtime.prepare_set_property_with_receiver(object, key, value, receiver)? {
        PropertySetAction::Complete => Ok(true),
        PropertySetAction::Rejected(_) => Ok(false),
        PropertySetAction::Throw(_) => Err(RuntimeError::Invariant(
            "context-free property test produced a JavaScript throw",
        )),
        PropertySetAction::Call { .. } => Err(RuntimeError::Invariant(
            "ordinary-property test helper unexpectedly reached a setter",
        )),
    }
}

fn get_property(
    runtime: &Runtime,
    object: &crate::engine::api::ObjectRef,
    key: &PropertyKey,
) -> Result<Value, RuntimeError> {
    match runtime.prepare_get_property(object, key)? {
        PropertyGetAction::Complete(value) => Ok(value),
        PropertyGetAction::Call { .. } => Err(RuntimeError::Invariant(
            "ordinary-property test helper unexpectedly reached a getter",
        )),
    }
}

fn global_callable(
    runtime: &Runtime,
    context: &mut crate::engine::api::context::Context,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(object) = context
        .get_property(&context.global_object().unwrap(), &key)
        .unwrap()
    else {
        panic!("global {name} was not an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("global {name} was not callable"))
}

fn eval_callable(
    runtime: &Runtime,
    context: &mut crate::engine::api::context::Context,
    source: &str,
) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("callable source did not produce an object: {source:?}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("source did not produce a callable: {source:?}"))
}

fn property_callable(
    runtime: &Runtime,
    context: &mut crate::engine::api::context::Context,
    object: &crate::engine::api::ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("property {name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("property {name} was not callable"))
}

fn own_key_names(runtime: &Runtime, object: &crate::engine::api::ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn own_data_value(runtime: &Runtime, object: &crate::engine::api::ObjectRef, name: &str) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data { value, .. }) =
        runtime.get_own_property(object, &key).unwrap()
    else {
        panic!("{name} was not an own data property");
    };
    value
}

fn own_stack_string(runtime: &Runtime, object: &crate::engine::api::ObjectRef) -> JsString {
    let Value::String(stack) = own_data_value(runtime, object, "stack") else {
        panic!("stack was not a string");
    };
    stack
}

fn take_error_message(
    runtime: &Runtime,
    context: &mut crate::engine::api::context::Context,
) -> JsString {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("pending exception was not an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    let Value::String(message) = context.get_property(&error, &message).unwrap() else {
        panic!("Error.message was not a string");
    };
    message
}

fn take_error_name_and_message(
    runtime: &Runtime,
    context: &mut crate::engine::api::context::Context,
) -> (JsString, JsString) {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("pending exception was not an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    let Value::String(name) = context.get_property(&error, &name).unwrap() else {
        panic!("Error.name was not a string");
    };
    let Value::String(message) = context.get_property(&error, &message).unwrap() else {
        panic!("Error.message was not a string");
    };
    (name, message)
}

fn bytecode_callable(
    runtime: &Runtime,
    context: &crate::engine::api::context::Context,
    code: Vec<Instruction>,
    metadata: FunctionMetadata,
) -> CallableRef {
    let function = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(code, Vec::new(), metadata),
        )
        .unwrap();
    runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap()
}

fn push_named_script_active_frame(
    runtime: &Runtime,
    context: &mut crate::engine::api::context::Context,
    filename: &str,
) -> super::ActiveFrameGuard {
    let bytecode = context
        .compile_with_filename("'use strict'; void 0;", filename)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(context.realm, &bytecode)
        .unwrap();
    runtime
        .push_bytecode_active_frame(callable.as_object().clone(), bytecode, context.realm, true)
        .unwrap()
}

fn push_named_eval_active_frame(
    runtime: &Runtime,
    context: &crate::engine::api::context::Context,
    filename: &str,
    kind: EvalKind,
) -> super::ActiveFrameGuard {
    let compile_context = match kind {
        EvalKind::Direct => EvalCompileContext::direct(false, Vec::new()),
        EvalKind::Indirect => EvalCompileContext::indirect(),
        EvalKind::None => panic!("test eval frame requires an eval kind"),
    };
    let function = compile_unlinked_eval_with_filename(
        "",
        filename,
        runtime.debug_info_mode(),
        compile_context,
    )
    .unwrap();
    crate::engine::code::bytecode_publish::verify_unlinked_eval_tree(
        &function,
        kind,
        false,
        &[],
        false,
        false,
    )
    .unwrap();
    let bytecode = runtime
        .publish_verified_unlinked_function(context.realm, function)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure_with_slots(context.realm, &bytecode, &[])
        .unwrap();
    runtime
        .push_bytecode_active_frame(callable.as_object().clone(), bytecode, context.realm, false)
        .unwrap()
}

fn push_named_module_active_frame(
    runtime: &Runtime,
    context: &crate::engine::api::context::Context,
    filename: &str,
) -> super::ActiveFrameGuard {
    let module =
        compile_unlinked_module_with_filename("", filename, runtime.debug_info_mode()).unwrap();
    crate::engine::code::bytecode_publish::verify_unlinked_module_tree(&module).unwrap();
    let function = module.into_parts().function;
    let bytecode = runtime
        .publish_verified_unlinked_function(context.realm, function)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure_with_slots(context.realm, &bytecode, &[])
        .unwrap();
    runtime
        .push_bytecode_active_frame(callable.as_object().clone(), bytecode, context.realm, true)
        .unwrap()
}

fn incrementing_closure(source: ClosureSource) -> UnlinkedFunction {
    UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::GetVarRef(0),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::SetVarRef(0),
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source,
            name: crate::engine::code::function::metadata::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    )
}

fn debug_draft(debug: UnlinkedFunctionDebug) -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_debug(debug)
}

mod binary_scalar;

mod binary_publication;

mod atoms;

mod binary_objects;

mod binary_property_keys;

mod binary_this;

mod binary_calls;

mod binary_throw;

mod binary_read_only;

mod binary_apply;

mod host_policy;

mod errors;

mod dynamic_functions;

mod eval;

mod arrays;

mod host_gc;

mod realms;

mod primitive_intrinsics;

mod strings;

mod symbols;

mod globals;

mod source;

mod boolean_objects;

mod iterators;

mod publication;

mod calls;

mod function_objects;

mod constructors;

mod debug;

mod bound_functions;

mod backtraces;

mod coercion;

mod properties;

mod native_calls;

mod active_frames;

mod accessors;

mod lexical_cells;

mod exceptions;

mod closures;

mod ownership;

mod gc;

mod shapes;

mod weak_references;

mod dynamic_import;
