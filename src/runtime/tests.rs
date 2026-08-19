use crate::JsBigInt;
use crate::bytecode::{ApplyKind, BytecodeFunction, Instruction};
use crate::compiler::{
    CompileOptions, EvalCompileContext, compile_unlinked_eval_with_filename,
    compile_unlinked_module_with_filename,
};
use crate::debug::{DebugInfoMode, LineColumn, Pc2LineEntry, Pc2LineTable};
use crate::error::{Error, ErrorKind, NativeErrorKind, NativeErrorMessage};
use crate::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::heap::{
    ArrayJoinKind, BytecodeConstant, ClosureSource, ClosureVariable, ClosureVariableKind,
    ClosureVariableName, ConstructorKind, DynamicFunctionKind, EvalBinding, EvalBindingSource,
    EvalEnvironment, EvalKind, EvalScope, EvalScopeKind, EvalVariableEnvironment,
    FunctionDebugPosition, FunctionKind, FunctionMetadata, HeapError, NativeCProto,
    NativeFunctionId, ObjectPayload, PrimitiveKind, PrimitiveObjectData, PropertySlot, RawValue,
};
use crate::object::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField,
    OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
};
use crate::shape::PropertyFlags;
use crate::value::{JsString, JsStringError, Value};
use crate::vm::{Completion, DirectEvalInvocation, IteratorCloseOutcome, Vm, VmHost};

use super::vm_host::RuntimeVmHost;
use super::{
    ActiveFrameFlags, ActiveFrameKind, CallableExecution, DeferredRefOp, DynamicSourceBuilder,
    EvalOptions, PropertyGetAction, PropertySetAction, Runtime, RuntimeError, ToPrimitiveHint,
    VarRefRoot,
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

const QUICKJS_SELF_CONTAINED_MODULE_BC5: &[u8] = &[
    0x05, 0x03, 0x24, 0x73, 0x65, 0x6c, 0x66, 0x2d, 0x63, 0x6f, 0x6e, 0x74, 0x61, 0x69, 0x6e, 0x65,
    0x64, 0x2e, 0x6d, 0x6a, 0x73, 0x0c, 0x61, 0x6e, 0x73, 0x77, 0x65, 0x72, 0x2e, 0x5f, 0x5f, 0x6d,
    0x6f, 0x64, 0x75, 0x6c, 0x65, 0x42, 0x79, 0x74, 0x65, 0x63, 0x6f, 0x64, 0x65, 0x52, 0x65, 0x63,
    0x65, 0x69, 0x70, 0x74, 0x0d, 0xe6, 0x03, 0x00, 0x01, 0x00, 0x00, 0xe8, 0x03, 0x00, 0x00, 0x00,
    0x0c, 0x20, 0x02, 0x01, 0xa8, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x14, 0x00, 0xe8,
    0x03, 0x00, 0x1e, 0x00, 0xa0, 0x02, 0x00, 0x05, 0x00, 0x08, 0xe8, 0x02, 0x29, 0xbb, 0x2a, 0xdf,
    0x38, 0x01, 0x00, 0x64, 0x00, 0x00, 0x3f, 0xf5, 0x00, 0x00, 0x00, 0x06, 0x2f,
];

#[test]
fn trusted_quickjs_scalar_script_uses_verified_runtime_publication() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    let function = context
        .read_trusted_scalar_script(QUICKJS_SCALAR_42_BC5)
        .unwrap();
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline + 1);
    assert_eq!(context.execute(&function).unwrap(), Value::Int(42));

    let mut same_runtime_realm = runtime.new_context();
    assert_eq!(
        same_runtime_realm.execute(&function).unwrap(),
        Value::Int(42)
    );

    let foreign_runtime = Runtime::new();
    let mut foreign_context = foreign_runtime.new_context();
    let foreign_objects = foreign_runtime.heap_counts().object_nodes;
    assert_eq!(
        foreign_context.execute(&function),
        Err(RuntimeError::WrongRuntime("function bytecode"))
    );
    assert_eq!(foreign_runtime.heap_counts().object_nodes, foreign_objects);
}

#[test]
fn trusted_quickjs_ordinary_leaf_executes_real_control_flow_and_function_properties() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();

    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_LEAF_42_BC5, 0)
        .unwrap();
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + 1
    );
    assert_eq!(
        runtime.heap_counts().object_nodes,
        baseline.object_nodes + 1
    );
    assert!(runtime.is_constructor(function.as_object()).unwrap());
    assert_eq!(
        runtime.get_prototype_of(function.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );

    assert_eq!(
        context
            .call(&function, Value::Undefined, &[Value::Int(3), Value::Int(3)],)
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[Value::Int(3), Value::Int(4)],)
            .unwrap(),
        Value::Int(0)
    );

    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let caller = runtime.intern_property_key("caller").unwrap();
    let arguments = runtime.intern_property_key("arguments").unwrap();
    assert_eq!(
        runtime.own_property_keys(function.as_object()).unwrap(),
        vec![length.clone(), name.clone(), prototype_key.clone()]
    );
    assert!(matches!(
        runtime
            .get_own_property(function.as_object(), &length)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name_value),
        writable: false,
        enumerable: false,
        configurable: true,
    }) = runtime
        .get_own_property(function.as_object(), &name)
        .unwrap()
    else {
        panic!("trusted ordinary leaf has the wrong name descriptor");
    };
    assert!(name_value.is_empty());
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: true,
        enumerable: false,
        configurable: false,
    }) = runtime
        .get_own_property(function.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("trusted ordinary leaf has the wrong prototype descriptor");
    };
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(constructor_value),
        writable: true,
        enumerable: false,
        configurable: true,
    }) = runtime.get_own_property(&prototype, &constructor).unwrap()
    else {
        panic!("trusted ordinary leaf prototype has the wrong constructor descriptor");
    };
    assert_eq!(constructor_value, function.as_object().clone());
    assert_eq!(
        context.get_property(function.as_object(), &caller).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context
            .get_property(function.as_object(), &arguments)
            .unwrap(),
        Value::Undefined
    );
    let Value::Object(instance) = context
        .construct(&function, &[Value::Int(3), Value::Int(3)])
        .unwrap()
    else {
        panic!("trusted ordinary leaf constructor returned a primitive");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(prototype)
    );
}

#[test]
fn trusted_quickjs_ordinary_leaf_preserves_strict_and_capability_metadata() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let mut strict = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    strict[28] = 0x01;
    let strict = context.read_trusted_ordinary_function(&strict, 0).unwrap();
    for property in ["caller", "arguments"] {
        let key = runtime.intern_property_key(property).unwrap();
        assert_eq!(
            context.get_property(strict.as_object(), &key),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_name_and_message(&runtime, &mut context),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("invalid property access"),
            )
        );
    }

    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let mut no_new_target = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    no_new_target[26] &= !0x40;
    let mut no_arguments = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    no_arguments[27] &= !0x02;
    for (label, image) in [
        ("new.target capability", no_new_target),
        ("arguments capability", no_arguments),
    ] {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("ordinary leaf without {label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts(), baseline);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_ordinary_leaf_preserves_frontier_and_malformed_provenance() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_LEAF_42_BC5, 1)
        .unwrap_err()
    else {
        panic!("out-of-range root selector did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(QUICKJS_SCALAR_42_BC5, 0)
        .unwrap_err()
    else {
        panic!("scalar root did not preserve the ordinary-leaf frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let mut verifier_rejected = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    verifier_rejected[57] = 0x9b; // `add` is typed but underflows this CFG.
    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(&verifier_rejected, 0)
        .unwrap_err()
    else {
        panic!("typed-verifier rejection did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("typed verification"));
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let mut truncated = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    truncated.pop();
    assert_eq!(
        context.read_trusted_ordinary_function(&truncated, 0),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("read after the end of the buffer"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes
    );
}

#[test]
fn trusted_quickjs_ordinary_leaf_publication_rolls_back_a_stale_realm() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let stale_realm = context.realm;
    drop(context);
    runtime.run_gc().unwrap();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    assert!(
        runtime
            .read_trusted_ordinary_function_in_realm(stale_realm, QUICKJS_ORDINARY_LEAF_42_BC5, 0,)
            .is_err()
    );
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn runtime_preloads_quickjs_typeof_atoms_as_narrow_canonical_strings() {
    const TYPEOF: u8 = 0x95;

    let runtime = Runtime::new();
    let initial_atom_count = runtime.test_atom_count();
    let mut canonical_string = None;
    for spelling in super::vm_host::TYPEOF_STATIC_ATOMS {
        let key = runtime.intern_property_key(spelling).unwrap();
        let canonical = runtime.property_key_to_js_string(&key).unwrap();
        assert!(!canonical.is_wide());
        assert_eq!(runtime.test_atom_count(), initial_atom_count);
        if spelling == "string" {
            canonical_string = Some(canonical);
        }
    }
    let canonical_string = canonical_string.expect("typeof static set contains string");

    let mut context = runtime.new_context();
    let atom_count = runtime.test_atom_count();
    let wide_atom = quickjs_scalar_with_atom_slot(
        &[
            u16::from(b's'),
            u16::from(b't'),
            u16::from(b'r'),
            u16::from(b'i'),
            u16::from(b'n'),
            u16::from(b'g'),
        ],
        true,
    );
    let wide_atom = context.read_trusted_scalar_script(&wide_atom).unwrap();
    let wide_atom = expect_string_value(context.execute(&wide_atom).unwrap());
    assert_eq!(runtime.test_atom_count(), atom_count);
    assert!(!wide_atom.is_wide());
    assert!(wide_atom.same_representation(&canonical_string));

    let typeof_string = quickjs_scalar_with_string_constant(
        &[0xbd, 0x00, TYPEOF, 0xcb, 0x28],
        &[u16::from(b'x')],
        false,
    );
    let typeof_string = context.read_trusted_scalar_script(&typeof_string).unwrap();
    let typeof_string = expect_string_value(context.execute(&typeof_string).unwrap());
    assert!(typeof_string.same_representation(&canonical_string));
}

#[test]
fn trusted_quickjs_scalar_script_executes_the_full_direct_int32_family() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let mut functions = Vec::new();
    let cases: &[(&[u8], i32)] = &[
        (&[0xb2, 0xcb, 0x28], -1),
        (&[0xb3, 0xcb, 0x28], 0),
        (&[0xba, 0xcb, 0x28], 7),
        (&[0xbb, 0x80, 0xcb, 0x28], -128),
        (&[0xbb, 0x7f, 0xcb, 0x28], 127),
        (&[0xbc, 0x7f, 0xff, 0xcb, 0x28], -129),
        (&[0xbc, 0xff, 0x7f, 0xcb, 0x28], 32_767),
        // Wider-than-canonical encodings remain valid QuickJS reader inputs.
        (&[0xbc, 0x01, 0x00, 0xcb, 0x28], 1),
        (&[0x01, 0xff, 0x7f, 0xff, 0xff, 0xcb, 0x28], -32_769),
        (&[0x01, 0x00, 0x80, 0x00, 0x00, 0xcb, 0x28], 32_768),
        (&[0x01, 0xff, 0xff, 0xff, 0x7f, 0xcb, 0x28], i32::MAX),
        (&[0x01, 0x00, 0x00, 0x00, 0x80, 0xcb, 0x28], i32::MIN),
        (&[0x01, 0x01, 0x00, 0x00, 0x00, 0xcb, 0x28], 1),
    ];

    for (code, expected) in cases {
        let image = quickjs_scalar_with_code(code);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::Int(*expected));
        functions.push(function);
    }
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline + cases.len()
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_direct_atom_free_primitives() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_functions = runtime.heap_counts().function_bytecode_nodes;
    let baseline_atoms = runtime.test_atom_count();
    let mut functions = Vec::new();
    let cases = vec![
        (vec![0x06, 0xcb, 0x28], Value::Undefined),
        (vec![0x07, 0xcb, 0x28], Value::Null),
        (vec![0x09, 0xcb, 0x28], Value::Bool(false)),
        (vec![0x0a, 0xcb, 0x28], Value::Bool(true)),
        (
            vec![0xb0, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28],
            Value::BigInt(JsBigInt::zero()),
        ),
        (
            vec![0xb0, 0xff, 0xff, 0xff, 0xff, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(-1)),
        ),
        (
            vec![0xb0, 0xff, 0xff, 0xff, 0x7f, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(i32::MAX)),
        ),
        (
            vec![0xb0, 0x01, 0x00, 0x00, 0x80, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(-2_147_483_647)),
        ),
        (
            vec![0xb0, 0x00, 0x00, 0x00, 0x80, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(i32::MIN)),
        ),
    ];

    for (code, expected) in &cases {
        let image = quickjs_scalar_with_code(code);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(context.execute(&function).unwrap(), expected.clone());
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        functions.push(function);
    }
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline_functions + cases.len()
    );
}

#[test]
fn trusted_quickjs_scalar_script_preserves_constant_pool_string_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let cases: &[(&[u16], bool)] = &[
        (&[], false),
        (&[u16::from(b'a'), 0, 0x00e9], false),
        (&[0x0100, 0xd83d, 0xde00, 0xd800], true),
        // Reader-compatible wide storage containing only Latin-1.
        (&[u16::from(b'4'), u16::from(b'2')], true),
    ];

    for &(units, wide) in cases {
        let image = quickjs_scalar_with_string_constant(&[0xbd, 0x00, 0xcb, 0x28], units, wide);
        let first_function = context.read_trusted_scalar_script(&image).unwrap();
        let first = expect_string_value(context.execute(&first_function).unwrap());
        let repeated = expect_string_value(context.execute(&first_function).unwrap());
        assert_eq!(first.utf16_units().collect::<Vec<_>>(), units);
        assert!(first.same_representation(&repeated));

        let second_function = context.read_trusted_scalar_script(&image).unwrap();
        let independent = expect_string_value(context.execute(&second_function).unwrap());
        assert_eq!(independent, first);
        assert!(!first.same_representation(&independent));
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_scalar_script_republishes_ordinary_atom_strings_by_runtime_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut sibling = runtime.new_context();

    let pinned = quickjs_scalar_with_atom_value(50);
    let first_function = context.read_trusted_scalar_script(&pinned).unwrap();
    let second_function = sibling.read_trusted_scalar_script(&pinned).unwrap();
    let first = expect_string_value(context.execute(&first_function).unwrap());
    let repeated = expect_string_value(context.execute(&first_function).unwrap());
    let sibling_value = expect_string_value(sibling.execute(&second_function).unwrap());
    let source_value = expect_string_value(context.eval("'length'").unwrap());
    assert_eq!(first, JsString::from_static("length"));
    assert!(first.same_representation(&repeated));
    assert!(first.same_representation(&sibling_value));
    assert!(first.same_representation(&source_value));

    let dynamic_image = quickjs_scalar_with_atom_slot(
        "quickjs-oxide-string-scalar"
            .encode_utf16()
            .collect::<Vec<_>>()
            .as_slice(),
        false,
    );
    let dynamic_left = context.read_trusted_scalar_script(&dynamic_image).unwrap();
    let dynamic_right = sibling.read_trusted_scalar_script(&dynamic_image).unwrap();
    let dynamic_left = expect_string_value(context.execute(&dynamic_left).unwrap());
    let dynamic_right = expect_string_value(sibling.execute(&dynamic_right).unwrap());
    let dynamic_source =
        expect_string_value(context.eval("'quickjs-oxide-string-scalar'").unwrap());
    assert!(dynamic_left.same_representation(&dynamic_right));
    assert!(dynamic_left.same_representation(&dynamic_source));
}

#[test]
fn trusted_quickjs_empty_string_uses_the_runtime_canonical_atom() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut sibling = runtime.new_context();

    let direct = quickjs_scalar_with_code(&[0xbf, 0xcb, 0x28]);
    let atom_backed = quickjs_scalar_with_atom_value(47);
    let constant_pool = quickjs_scalar_with_string_constant(&[0xbd, 0, 0xcb, 0x28], &[], false);
    let direct_function = context.read_trusted_scalar_script(&direct).unwrap();
    let atom_function = sibling.read_trusted_scalar_script(&atom_backed).unwrap();
    let constant_pool_function = context.read_trusted_scalar_script(&constant_pool).unwrap();
    let direct_value = expect_string_value(context.execute(&direct_function).unwrap());
    let repeated = expect_string_value(context.execute(&direct_function).unwrap());
    let atom_value = expect_string_value(sibling.execute(&atom_function).unwrap());
    let constant_pool_value =
        expect_string_value(context.execute(&constant_pool_function).unwrap());
    let source_value = expect_string_value(context.eval("''").unwrap());
    assert!(direct_value.is_empty());
    assert!(direct_value.same_representation(&repeated));
    assert!(direct_value.same_representation(&atom_value));
    assert!(direct_value.same_representation(&source_value));
    assert!(!direct_value.same_representation(&constant_pool_value));

    let foreign_runtime = Runtime::new();
    let mut foreign_context = foreign_runtime.new_context();
    let foreign_value = expect_string_value(foreign_context.eval("''").unwrap());
    assert!(!direct_value.same_representation(&foreign_value));
}

#[test]
fn trusted_quickjs_ordinary_leaf_synthesizes_bigint_and_canonical_empty_atom_constants() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut sibling = runtime.new_context();

    let mut bigint_code = vec![0xb0];
    bigint_code.extend_from_slice(&42_i32.to_le_bytes());
    bigint_code.push(0x28);
    let bigint_image = quickjs_ordinary_with_code_and_constants(&bigint_code, &[]);
    let bigint_function = context
        .read_trusted_ordinary_function(&bigint_image, 0)
        .unwrap();
    assert_eq!(
        context
            .call(&bigint_function, Value::Undefined, &[])
            .unwrap(),
        Value::BigInt(JsBigInt::from(42))
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&bigint_function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::PushConst(0), Instruction::Return]
    ));
    assert!(matches!(
        snapshot.constants.as_ref(),
        [BytecodeConstant::Value(RawValue::BigInt(value))]
            if value == &JsBigInt::from(42)
    ));

    let direct_image = quickjs_ordinary_with_code_and_constants(&[0xbf, 0x28], &[]);
    let empty_entry = quickjs_string_constant_entry(&[], false);
    let constant_image =
        quickjs_ordinary_with_code_and_constants(&[0xbd, 0x00, 0x28], &[empty_entry.as_slice()]);
    let direct_function = context
        .read_trusted_ordinary_function(&direct_image, 0)
        .unwrap();
    let sibling_function = sibling
        .read_trusted_ordinary_function(&direct_image, 0)
        .unwrap();
    let constant_function = context
        .read_trusted_ordinary_function(&constant_image, 0)
        .unwrap();
    let direct = expect_string_value(
        context
            .call(&direct_function, Value::Undefined, &[])
            .unwrap(),
    );
    let repeated = expect_string_value(
        context
            .call(&direct_function, Value::Undefined, &[])
            .unwrap(),
    );
    let sibling_value = expect_string_value(
        sibling
            .call(&sibling_function, Value::Undefined, &[])
            .unwrap(),
    );
    let constant = expect_string_value(
        context
            .call(&constant_function, Value::Undefined, &[])
            .unwrap(),
    );
    let source = expect_string_value(context.eval("''").unwrap());
    assert!(direct.is_empty());
    assert!(direct.same_representation(&repeated));
    assert!(direct.same_representation(&sibling_value));
    assert!(direct.same_representation(&source));
    assert!(!direct.same_representation(&constant));

    let original = quickjs_float_constant_entry(0.5_f64.to_bits());
    let mut mixed_code = vec![0xbd, 0x00, 0x0e, 0xb0];
    mixed_code.extend_from_slice(&7_i32.to_le_bytes());
    mixed_code.extend_from_slice(&[0x0e, 0xbf, 0x0e, 0xb0]);
    mixed_code.extend_from_slice(&(-3_i32).to_le_bytes());
    mixed_code.extend_from_slice(&[0x0e, 0xbf, 0x28]);
    let mut mixed_image =
        quickjs_ordinary_with_code_and_constants(&mixed_code, &[original.as_slice()]);
    mixed_image[33] = 1;
    let mixed_function = context
        .read_trusted_ordinary_function(&mixed_image, 0)
        .unwrap();
    let mixed_result = expect_string_value(
        context
            .call(&mixed_function, Value::Undefined, &[])
            .unwrap(),
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&mixed_function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::PushConst(0),
            Instruction::Drop,
            Instruction::PushConst(1),
            Instruction::Drop,
            Instruction::PushConst(2),
            Instruction::Drop,
            Instruction::PushConst(3),
            Instruction::Drop,
            Instruction::PushConst(4),
            Instruction::Return,
        ]
    ));
    assert_eq!(snapshot.constants.len(), 5);
    assert!(matches!(
        &snapshot.constants[0],
        BytecodeConstant::Value(RawValue::Float(value))
            if value.to_bits() == 0.5_f64.to_bits()
    ));
    assert!(matches!(
        &snapshot.constants[1],
        BytecodeConstant::Value(RawValue::BigInt(value)) if value == &JsBigInt::from(7)
    ));
    assert!(matches!(
        &snapshot.constants[3],
        BytecodeConstant::Value(RawValue::BigInt(value)) if value == &JsBigInt::from(-3)
    ));
    let BytecodeConstant::Value(RawValue::String(first_empty)) = &snapshot.constants[2] else {
        panic!("first synthesized empty atom lost its String payload");
    };
    let BytecodeConstant::Value(RawValue::String(second_empty)) = &snapshot.constants[4] else {
        panic!("second synthesized empty atom lost its String payload");
    };
    assert!(first_empty.same_representation(second_empty));
    assert!(mixed_result.same_representation(first_empty));
}

#[test]
fn trusted_quickjs_ordinary_leaf_return_undefined_is_a_zero_stack_terminal() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut image = quickjs_ordinary_with_code_and_constants(&[0x29], &[]);
    image[33] = 0;

    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 0);
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::ReturnUndefined]
    ));
}

#[test]
fn trusted_quickjs_ordinary_plain_calls_preserve_receiver_arity_and_argument_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_PLAIN_CALLS_CODE, &[]);
    image[33] = 5;

    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary plain-call leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 5);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Call(0),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::Call(1),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::Call(2),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Call(3),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::PushI32(4),
            Instruction::Call(4),
            Instruction::Return,
        ]
    ));
    drop(snapshot);

    let callback = context
        .eval(
            r#"
                globalThis.__qjo_plain_call_log = "";
                (0, function () {
                    "use strict";
                    __qjo_plain_call_log += (this === undefined ? "u" : "x") + ":" + arguments.length;
                    var index = 0;
                    while (index < arguments.length) {
                        __qjo_plain_call_log += ":" + arguments[index];
                        index++;
                    }
                    __qjo_plain_call_log += "|";
                    return arguments.length;
                })
            "#,
        )
        .unwrap();
    assert!(matches!(callback, Value::Object(_)));
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[callback])
            .unwrap(),
        Value::Int(4)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_plain_call_log").unwrap()),
        JsString::from_static("u:0|u:1:1|u:2:1:2|u:3:1:2:3|u:4:1:2:3:4|")
    );
}

#[test]
fn trusted_quickjs_ordinary_plain_call_non_callable_exception_is_recoverable() {
    let mut image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_PLAIN_CALLS_CODE, &[]);
    image[33] = 5;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Int(0)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("not a function"),
        )
    );
    assert!(!context.has_exception());

    let callback = context
        .eval("(0, function () { 'use strict'; return arguments.length; })")
        .unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[callback])
            .unwrap(),
        Value::Int(4)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {
    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);
    assert_eq!(
        fnv1a64(QUICKJS_ORDINARY_TAIL_CALL_BC5),
        0x8ff9_d2c1_0c7e_2228
    );
    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5.len(), 62);
    assert_eq!(
        fnv1a64(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5),
        0xe87d_54c0_a2a1_40ca
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let tail_call = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_BC5, 0)
        .unwrap();
    let tail_method = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5, 0)
        .unwrap();

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&tail_call).unwrap()
    else {
        panic!("trusted raw35 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 3);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::GetArg(2),
            Instruction::TailCall(2),
        ]
    ));
    drop(snapshot);

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&tail_method).unwrap()
    else {
        panic!("trusted raw37 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 4);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::GetArg(2),
            Instruction::GetArg(3),
            Instruction::TailCallMethod(2),
        ]
    ));
    drop(snapshot);

    let plain_target = context
        .eval(
            r#"
                globalThis.__qjo_tail_plain_log = "";
                (0, function (a, b) {
                    "use strict";
                    __qjo_tail_plain_log =
                        (this === undefined) + ":" + arguments.length + ":" + a + ":" + b;
                    return a * 10 + b;
                })
            "#,
        )
        .unwrap();
    assert_eq!(
        context
            .call(
                &tail_call,
                Value::Int(99),
                &[plain_target.clone(), Value::Int(4), Value::Int(2)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_tail_plain_log").unwrap()),
        JsString::from_static("true:2:4:2")
    );

    let receiver = context
        .eval("(globalThis.__qjo_tail_receiver = { base: 40 })")
        .unwrap();
    let method = context
        .eval(
            r#"
                (function (a, b) {
                    "use strict";
                    globalThis.__qjo_tail_method_log =
                        (this === globalThis.__qjo_tail_receiver) + ":" +
                        arguments.length + ":" + a + ":" + b;
                    return this.base + a + b;
                })
            "#,
        )
        .unwrap();
    assert_eq!(
        context
            .call(
                &tail_method,
                Value::Undefined,
                &[receiver, method.clone(), Value::Int(1), Value::Int(1)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_tail_method_log").unwrap()),
        JsString::from_static("true:2:1:1")
    );

    // A forged instruction after raw35 remains authenticated but unreachable:
    // the tail completion must never fall through to the underflowing Drop.
    let unreachable = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x23, 0x02, 0x00, 0x0e],
                3,
            ),
            0,
        )
        .unwrap();
    assert_eq!(
        context
            .call(
                &unreachable,
                Value::Undefined,
                &[plain_target, Value::Int(4), Value::Int(2)],
            )
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn trusted_quickjs_ordinary_tail_invocation_failures_are_recoverable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let tail_call = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_BC5, 0)
        .unwrap();
    let tail_method = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5, 0)
        .unwrap();

    for (label, function, arguments) in [
        (
            "plain",
            &tail_call,
            vec![Value::Int(0), Value::Int(1), Value::Int(2)],
        ),
        (
            "method",
            &tail_method,
            vec![Value::Null, Value::Int(0), Value::Int(1), Value::Int(2)],
        ),
    ] {
        assert_eq!(
            context.call(function, Value::Undefined, &arguments),
            Err(RuntimeError::Exception),
            "{label}"
        );
        assert_eq!(
            take_error_name_and_message(&runtime, &mut context),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("not a function"),
            ),
            "{label}"
        );
        assert!(!context.has_exception(), "{label}");
    }

    let target = context
        .eval("(0, function (a, b) { 'use strict'; return a + b; })")
        .unwrap();
    assert_eq!(
        context
            .call(
                &tail_call,
                Value::Undefined,
                &[target.clone(), Value::Int(20), Value::Int(22)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context
            .call(
                &tail_method,
                Value::Undefined,
                &[Value::Null, target, Value::Int(20), Value::Int(22)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_tail_verification_rolls_back_heap_and_atoms() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let cases = [
        (
            "tail_call underflow",
            quickjs_ordinary_three_argument_with_code(&[0x23, 0x00, 0x00], 0),
            UNDERFLOW,
        ),
        (
            "tail_call_method underflow",
            quickjs_ordinary_four_argument_with_code(&[0x25, 0x00, 0x00], 0),
            UNDERFLOW,
        ),
        (
            "tail_call maximum",
            quickjs_ordinary_three_argument_with_code(&[0xcf, 0xd0, 0xd1, 0x23, 0x02, 0x00], 2),
            DECLARED_MAXIMUM,
        ),
        (
            "tail_call_method maximum",
            quickjs_ordinary_four_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0xd2, 0x25, 0x02, 0x00],
                3,
            ),
            DECLARED_MAXIMUM,
        ),
        (
            "tail_call u16 maximum operand",
            quickjs_ordinary_three_argument_with_code(&[0x23, 0xff, 0xff], 0),
            UNDERFLOW,
        ),
        (
            "tail_call_method u16 maximum operand",
            quickjs_ordinary_four_argument_with_code(&[0x25, 0xff, 0xff], 0),
            UNDERFLOW,
        ),
    ];

    for (label, image, expected_message) in cases {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected_message, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_throw_uses_the_exact_wire_metadata_and_value_identity() {
    assert_eq!(QUICKJS_ORDINARY_THROW_BC5.len(), 45);
    assert_eq!(fnv1a64(QUICKJS_ORDINARY_THROW_BC5), 0x73cf_217e_06c5_fee2);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted raw48 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::GetArg(0), Instruction::Throw]
    ));
    assert_eq!(snapshot.metadata.argument_count, 1);
    assert_eq!(snapshot.metadata.defined_argument_count, 1);
    assert_eq!(snapshot.metadata.local_count, 0);
    assert_eq!(snapshot.metadata.max_stack, 1);
    assert!(snapshot.metadata.strict);
    assert!(snapshot.metadata.strip_variable_debug);
    assert_eq!(snapshot.metadata.function_kind, FunctionKind::Normal);
    assert!(snapshot.metadata.has_prototype);
    assert_eq!(snapshot.metadata.constructor_kind, ConstructorKind::Base);
    assert!(!snapshot.metadata.arguments_forbidden);
    drop(snapshot);

    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Int(9)]),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
    assert!(!context.has_exception());

    let object = context.new_object().unwrap();
    assert_eq!(
        context.call(
            &function,
            Value::Undefined,
            &[Value::Object(object.clone())],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::Object(object.clone()))
    );
    assert!(!context.has_exception());

    let Value::Object(error) = context.eval("new Error('raw48 identity')").unwrap() else {
        panic!("Error construction did not return an object");
    };
    let stack = runtime.intern_property_key("stack").unwrap();
    assert!(runtime.delete_property(&error, &stack).unwrap());
    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Object(error.clone())],),
        Err(RuntimeError::Exception)
    );
    let Value::Object(thrown_error) = context.take_exception().unwrap().unwrap() else {
        panic!("trusted raw48 throw lost its Error object");
    };
    assert_eq!(thrown_error, error);
    assert!(
        own_stack_string(&runtime, &error)
            .to_utf8_lossy()
            .contains("<anonymous>")
    );
    assert!(!context.has_exception());

    // Taking the pending completion restores the public API for a clean retry.
    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_reenters_caller_catch_backtrace_and_iterator_close() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    let global = context.global_object().unwrap();
    for (name, value) in [("rawThrow", Value::Object(function.as_object().clone()))] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(&global, &key, &data_descriptor(value, true, true, true),)
                .unwrap()
        );
    }

    let Value::Object(error) = context.eval("new Error('caught raw48')").unwrap() else {
        panic!("Error construction did not return an object");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(runtime.delete_property(&error, &stack_key).unwrap());
    let held_key = runtime.intern_property_key("heldThrowError").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &held_key,
                &data_descriptor(Value::Object(error.clone()), true, true, true),
            )
            .unwrap()
    );

    assert_eq!(
        context
            .eval_with_filename(
                "(function caller(){try{rawThrow(heldThrowError)}catch(caught){return caught===heldThrowError}})()",
                "raw-throw-caller.js",
            )
            .unwrap(),
        Value::Bool(true)
    );
    assert!(!context.has_exception());
    let stack = own_stack_string(&runtime, &error).to_utf8_lossy();
    assert!(stack.contains("    at <anonymous>\n"), "{stack}");
    assert!(stack.contains("caller (raw-throw-caller.js:"), "{stack}");

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            globalThis.__qjo_throw_close_log = "";
                            var original = { kind: "raw48" };
                            var closeFailure = { kind: "iterator-close" };
                            var iterable = {};
                            iterable[Symbol.iterator] = function () {
                                var delivered = false;
                                return {
                                    next: function () {
                                        if (delivered) return { done: true };
                                        delivered = true;
                                        return { value: 1, done: false };
                                    },
                                    return: function () {
                                        __qjo_throw_close_log += "close";
                                        throw closeFailure;
                                    }
                                };
                            };
                            try {
                                for (var value of iterable) rawThrow(original);
                            } catch (caught) {
                                return __qjo_throw_close_log + ":" +
                                    (caught === original) + ":" +
                                    (caught === closeFailure);
                            }
                            return "missing throw";
                        })()
                    "#,
                )
                .unwrap(),
        ),
        JsString::from_static("close:true:false")
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_is_terminal_and_branch_targetable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    // The underflowing Drop is translated and published but is unreachable
    // after the explicit throw completion.
    let terminal = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_one_argument_with_code(&[0xcf, 0x30, 0x0e], 1),
            0,
        )
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&terminal).unwrap()
    else {
        panic!("terminal raw48 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Throw,
            Instruction::Drop,
        ]
    ));
    drop(snapshot);
    assert_eq!(
        context.call(&terminal, Value::Undefined, &[Value::Int(17)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));

    // goto8 lands directly on raw48 while retaining the argument value. The
    // skipped push proves target rewriting uses the throw's typed IR index.
    let branch = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_one_argument_with_code(&[0xcf, 0xea, 0x02, 0xb4, 0x30], 1),
            0,
        )
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&branch).unwrap()
    else {
        panic!("branch-to-raw48 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Goto(3),
            Instruction::PushI32(1),
            Instruction::Throw,
        ]
    ));
    drop(snapshot);
    let object = context.new_object().unwrap();
    assert_eq!(
        context.call(&branch, Value::Undefined, &[Value::Object(object.clone())],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::Object(object))
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_verification_rejects_transactionally_and_retries() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for (label, code, max_stack, expected) in [
        ("throw underflow", &[0x30][..], 0, UNDERFLOW),
        (
            "throw declared maximum",
            &[0xcf, 0x30][..],
            0,
            DECLARED_MAXIMUM,
        ),
        (
            "branch-to-throw underflow",
            &[0xea, 0x02, 0xb4, 0x30][..],
            1,
            UNDERFLOW,
        ),
    ] {
        let image = quickjs_ordinary_one_argument_with_code(code, max_stack);
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let retry = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    assert_eq!(
        context.call(&retry, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    let mut async_function = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    async_function[28] |= 1 << 2; // strict + async JS mode
    let mut generator = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    generator[26] |= 1 << 4; // normal -> generator FunctionKind
    let mut derived = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    derived[26] |= 1 << 2; // ordinary -> derived class constructor

    for (label, image) in [
        ("async", async_function),
        ("generator", generator),
        ("derived constructor", derived),
    ] {
        assert_eq!(
            &image[43..],
            [0xcf, 0x30],
            "{label} mutation changed the natural raw48 body"
        );
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} raw48 metadata did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(
            error.message(),
            "trusted QuickJS ordinary leaf is not admitted: function metadata is outside the ordinary synchronous leaf cohort",
            "{label}"
        );
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let retry = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    assert_eq!(
        context.call(&retry, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_non_tail_invocations_preserve_stack_contracts() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let mut construct_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CONSTRUCT_CODE, &[]);
    construct_image[33] = 4;
    let construct = context
        .read_trusted_ordinary_function(&construct_image, 0)
        .unwrap();
    let constructor = context
        .eval(
            r#"
                (globalThis.__qjo_construct = function C(a, b) {
                    globalThis.__qjo_construct_log =
                        (new.target === globalThis.__qjo_new_target) + ":" +
                        (Object.getPrototypeOf(this) === new.target.prototype) + ":" +
                        a + ":" + b;
                    this.sum = a + b;
                })
            "#,
        )
        .unwrap();
    let new_target = context
        .eval("(globalThis.__qjo_new_target = function N() {})")
        .unwrap();
    let Value::Object(instance) = context
        .call(&construct, Value::Undefined, &[constructor, new_target])
        .unwrap()
    else {
        panic!("call_constructor did not return an Object");
    };
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&instance, &sum).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_construct_log").unwrap()),
        JsString::from_static("true:true:1:2")
    );

    let mut method_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CALL_METHOD_CODE, &[]);
    method_image[33] = 3;
    let call_method = context
        .read_trusted_ordinary_function(&method_image, 0)
        .unwrap();
    let receiver = context
        .eval("(globalThis.__qjo_method_receiver = { base: 7 })")
        .unwrap();
    let method = context
        .eval(
            "(function (value) { 'use strict'; return this === globalThis.__qjo_method_receiver ? this.base + value : -1; })",
        )
        .unwrap();
    assert_eq!(
        context
            .call(&call_method, Value::Undefined, &[receiver, method])
            .unwrap(),
        Value::Int(49)
    );

    let mut array_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_ARRAY_FROM_CODE, &[]);
    array_image[33] = 3;
    let array_from = context
        .read_trusted_ordinary_function(&array_image, 0)
        .unwrap();
    let Value::Object(first) = context.call(&array_from, Value::Undefined, &[]).unwrap() else {
        panic!("array_from did not return an Array");
    };
    let Value::Object(second) = context.call(&array_from, Value::Undefined, &[]).unwrap() else {
        panic!("array_from repeat did not return an Array");
    };
    assert_ne!(first, second);
    let defining_array_prototype = context.array_prototype().unwrap();
    for array in [&first, &second] {
        assert!(runtime.is_array_object(array).unwrap());
        assert_eq!(
            runtime.get_prototype_of(array).unwrap(),
            Some(defining_array_prototype.clone())
        );
    }
    for (key, expected) in [("0", 1), ("1", 2), ("2", 3), ("length", 3)] {
        let key = runtime.intern_property_key(key).unwrap();
        assert_eq!(
            context.get_property(&first, &key).unwrap(),
            Value::Int(expected)
        );
    }
    let mut empty_array_image =
        quickjs_ordinary_with_code_and_constants(&[0x26, 0x00, 0x00, 0x28], &[]);
    empty_array_image[33] = 1;
    let empty_array_from = context
        .read_trusted_ordinary_function(&empty_array_image, 0)
        .unwrap();
    let Value::Object(empty) = context
        .call(&empty_array_from, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("zero-element array_from did not return an Array");
    };
    assert!(runtime.is_array_object(&empty).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&empty).unwrap(),
        Some(defining_array_prototype.clone())
    );
    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        context.get_property(&empty, &length).unwrap(),
        Value::Int(0)
    );

    let mut foreign_caller = runtime.new_context();
    let foreign_array_prototype = foreign_caller.array_prototype().unwrap();
    assert_ne!(defining_array_prototype, foreign_array_prototype);
    let Value::Object(cross_realm) = foreign_caller
        .call(&array_from, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("cross-realm array_from did not return an Array");
    };
    assert!(runtime.is_array_object(&cross_realm).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&cross_realm).unwrap(),
        Some(defining_array_prototype)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&construct).unwrap()
    else {
        panic!("trusted ordinary construct leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::Construct(2),
            Instruction::Return,
        ]
    ));
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&call_method).unwrap()
    else {
        panic!("trusted ordinary call_method leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::PushI32(42),
            Instruction::CallMethod(1),
            Instruction::Return,
        ]
    ));
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&array_from).unwrap()
    else {
        panic!("trusted ordinary array_from leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::ArrayFrom(3),
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_apply_admits_only_canonical_typed_kinds() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (magic, expected_kind) in [(0, ApplyKind::Call), (1, ApplyKind::Construct)] {
        let code = [0xcf, 0xd0, 0xd1, 0x27, magic, 0x00, 0x28];
        let image = quickjs_ordinary_three_argument_with_code(&code, 3);
        let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
        let CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(&function).unwrap()
        else {
            panic!("trusted ordinary apply leaf did not publish bytecode");
        };
        let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        assert!(matches!(
            snapshot.code.as_ref(),
            [
                Instruction::GetArg(0),
                Instruction::GetArg(1),
                Instruction::GetArg(2),
                Instruction::Apply(actual),
                Instruction::Return,
            ] if *actual == expected_kind
        ));
    }

    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for magic in [2_u16, u16::MAX] {
        let [low, high] = magic.to_le_bytes();
        let image = quickjs_ordinary_three_argument_with_code(
            &[0xcf, 0xd0, 0xd1, 0x27, low, high, 0x28],
            3,
        );
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("noncanonical apply magic did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(
            error.message(),
            format!(
                "trusted QuickJS ordinary leaf is not admitted: apply operand must be canonical 0 (call) or 1 (construct), found {magic}"
            )
        );
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts(), baseline);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_ordinary_apply_preserves_object_list_call_and_construct_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let call = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let construct = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();

    let receiver = context
        .eval("(globalThis.__qjo_apply_receiver = { marker: 7 })")
        .unwrap();
    let target = context
        .eval(
            r#"
                (function (a, b) {
                    "use strict";
                    return (this === globalThis.__qjo_apply_receiver) + ":" +
                        arguments.length + ":" + a + ":" + b + ":" +
                        (new.target === undefined);
                })
            "#,
        )
        .unwrap();
    let list = context.eval("[11, 22]").unwrap();
    assert_eq!(
        expect_string_value(
            context
                .call(&call, Value::Undefined, &[target, receiver, list])
                .unwrap()
        ),
        JsString::from_static("true:2:11:22:true")
    );

    let constructor = context
        .eval(
            r#"
                (globalThis.__qjo_apply_constructor = function C(a, b) {
                    globalThis.__qjo_apply_construct_log =
                        (new.target === globalThis.__qjo_apply_new_target) + ":" + a + ":" + b;
                    this.sum = a + b;
                })
            "#,
        )
        .unwrap();
    let new_target = context
        .eval("(globalThis.__qjo_apply_new_target = function N() {})")
        .unwrap();
    let list = context.eval("[20, 22]").unwrap();
    let Value::Object(instance) = context
        .call(
            &construct,
            Value::Undefined,
            &[constructor, new_target.clone(), list],
        )
        .unwrap()
    else {
        panic!("apply construct did not return an object");
    };
    assert_eq!(
        expect_string_value(context.eval("__qjo_apply_construct_log").unwrap()),
        JsString::from_static("true:20:22")
    );
    let Value::Object(new_target) = new_target else {
        panic!("newTarget was not an object");
    };
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(prototype) = context.get_property(&new_target, &prototype).unwrap() else {
        panic!("newTarget prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(prototype)
    );
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&instance, &sum).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn trusted_quickjs_ordinary_apply_nullish_lists_use_raw_receiver_for_both_kinds() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let receiver = context
        .eval("(globalThis.__qjo_nullish_receiver = { marker: 42 })")
        .unwrap();
    let target = context
        .eval(
            r#"
                (globalThis.__qjo_nullish_target = ({
                    probe() {
                        "use strict";
                        return (this === globalThis.__qjo_nullish_receiver) + ":" +
                            arguments.length + ":" + (new.target === undefined);
                    }
                }).probe)
            "#,
        )
        .unwrap();
    let Value::Object(target_object) = &target else {
        panic!("nullish apply target was not an object");
    };
    assert!(!runtime.is_constructor(target_object).unwrap());

    for magic in [0_u8, 1] {
        let wrapper = context
            .read_trusted_ordinary_function(
                &quickjs_ordinary_three_argument_with_code(
                    &[0xcf, 0xd0, 0xd1, 0x27, magic, 0x00, 0x28],
                    3,
                ),
                0,
            )
            .unwrap();
        for argument_array in [Value::Null, Value::Undefined] {
            assert_eq!(
                expect_string_value(
                    context
                        .call(
                            &wrapper,
                            Value::Undefined,
                            &[target.clone(), receiver.clone(), argument_array],
                        )
                        .unwrap()
                ),
                JsString::from_static("true:0:true"),
                "magic {magic}"
            );
            assert!(!context.has_exception());
        }
    }
}

#[test]
fn trusted_quickjs_ordinary_apply_preserves_error_order_realm_and_pending_identity() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let call = defining
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let construct = defining
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let defining_type_error_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(defining.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();

    let mut caller = runtime.new_context();
    let poison = caller
        .eval(
            r#"
                globalThis.__qjo_apply_length_reads = 0;
                globalThis.__qjo_apply_sentinel = {};
                ({ get length() {
                    __qjo_apply_length_reads++;
                    throw __qjo_apply_sentinel;
                } })
            "#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &call,
            Value::Undefined,
            &[Value::Int(0), Value::Undefined, poison.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(not_callable) = caller.take_exception().unwrap().unwrap() else {
        panic!("non-callable apply did not throw an object");
    };
    assert_eq!(
        runtime
            .get_prototype_of(&not_callable)
            .unwrap()
            .unwrap()
            .object_id(),
        defining_type_error_prototype
    );
    assert_eq!(
        caller.eval("__qjo_apply_length_reads").unwrap(),
        Value::Int(0)
    );
    assert!(!caller.has_exception());

    let target = caller
        .eval("(globalThis.__qjo_apply_plain = ({ plain() {} }).plain)")
        .unwrap();
    assert_eq!(
        caller.call(
            &construct,
            Value::Undefined,
            &[target, Value::Int(17), poison.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    let thrown = caller.take_exception().unwrap().unwrap();
    let sentinel = caller.eval("__qjo_apply_sentinel").unwrap();
    assert_eq!(thrown, sentinel);
    assert_eq!(
        caller.eval("__qjo_apply_length_reads").unwrap(),
        Value::Int(1)
    );
    assert!(!caller.has_exception());

    let constructor = caller.eval("(function C() {})").unwrap();
    assert_eq!(
        caller.call(
            &construct,
            Value::Undefined,
            &[constructor, Value::Null, poison],
        ),
        Err(RuntimeError::Exception)
    );
    let thrown = caller.take_exception().unwrap().unwrap();
    assert_eq!(thrown, sentinel);
    assert_eq!(
        caller.eval("__qjo_apply_length_reads").unwrap(),
        Value::Int(2)
    );
    assert!(!caller.has_exception());

    let receiver = caller.eval("({})").unwrap();
    let target = caller
        .eval("({ retry() { 'use strict'; return this; } }).retry")
        .unwrap();
    assert_eq!(
        caller
            .call(
                &construct,
                Value::Undefined,
                &[target, receiver.clone(), Value::Null],
            )
            .unwrap(),
        receiver
    );
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_apply_raw_prototype_get_preserves_order_and_receiver() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let wrapper = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let constructor = context
        .eval(
            "globalThis.__qjo_raw_get_body_count = 0; \
             (function RawGetTarget() { \
                 __qjo_raw_get_body_count++; \
                 globalThis.__qjo_raw_get_new_target = new.target; \
             })",
        )
        .unwrap();
    let empty = context.eval("[]").unwrap();

    let Value::Object(undefined_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Undefined, empty.clone()],
        )
        .unwrap()
    else {
        panic!("undefined raw newTarget did not construct an object");
    };
    assert_eq!(
        context.eval("__qjo_raw_get_new_target").unwrap(),
        Value::Undefined
    );
    assert_eq!(
        runtime.get_prototype_of(&undefined_instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );

    context.eval("__qjo_raw_get_body_count = 0").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Null, empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );
    assert_eq!(
        context.eval("__qjo_raw_get_body_count").unwrap(),
        Value::Int(0)
    );

    context
        .eval(
            r#"
                globalThis.__qjo_number_receiver = undefined;
                globalThis.__qjo_number_custom_prototype = {};
                Object.defineProperty(Number.prototype, "prototype", {
                    configurable: true,
                    get: function () {
                        "use strict";
                        globalThis.__qjo_number_receiver = this;
                        return globalThis.__qjo_number_custom_prototype;
                    }
                });
            "#,
        )
        .unwrap();
    let Value::Object(number_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Int(17), empty.clone()],
        )
        .unwrap()
    else {
        panic!("primitive prototype getter did not construct an object");
    };
    let Value::Object(number_prototype) = context.eval("__qjo_number_custom_prototype").unwrap()
    else {
        panic!("primitive getter prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&number_instance).unwrap(),
        Some(number_prototype)
    );
    assert_eq!(
        context.eval("__qjo_number_receiver").unwrap(),
        Value::Int(17)
    );
    context.eval("delete Number.prototype.prototype").unwrap();

    context
        .eval(
            r#"
                globalThis.__qjo_raw_get_sentinel = {};
                globalThis.__qjo_throwing_new_target = {};
                Object.defineProperty(__qjo_throwing_new_target, "prototype", {
                    get: function () { throw __qjo_raw_get_sentinel; }
                });
                __qjo_raw_get_body_count = 0;
            "#,
        )
        .unwrap();
    let throwing_new_target = context.eval("__qjo_throwing_new_target").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), throwing_new_target, empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap().unwrap(),
        context.eval("__qjo_raw_get_sentinel").unwrap()
    );
    assert_eq!(
        context.eval("__qjo_raw_get_body_count").unwrap(),
        Value::Int(0)
    );

    let non_callable_new_target = context.eval("({ prototype: 1 })").unwrap();
    let Value::Object(fallback_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), non_callable_new_target, empty.clone()],
        )
        .unwrap()
    else {
        panic!("noncallable raw newTarget did not use the caller realm fallback");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );

    let revoking_new_target = context
        .eval(
            r#"
                globalThis.__qjo_realm_revoke_record = undefined;
                var realmRevokeHandler = {
                    get: function (target, key, receiver) {
                        if (key === "prototype") {
                            __qjo_realm_revoke_record.revoke();
                            return 1;
                        }
                        return Reflect.get(target, key, receiver);
                    }
                };
                __qjo_realm_revoke_record =
                    Proxy.revocable(function RealmRevokeTarget() {}, realmRevokeHandler);
                __qjo_realm_revoke_record.proxy
            "#,
        )
        .unwrap();
    context.eval("__qjo_raw_get_body_count = 0").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[constructor, revoking_new_target, empty],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("revoked proxy"),
        )
    );
    assert_eq!(
        context.eval("__qjo_raw_get_body_count").unwrap(),
        Value::Int(0)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_apply_construct_preserves_raw_new_target_across_dispatch() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let wrapper = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let empty = context.eval("[]").unwrap();

    let constructor = context
        .eval(
            r#"
                (globalThis.__qjo_raw_constructor = function C() {
                    globalThis.__qjo_raw_base_new_target = new.target;
                    this.marker = 42;
                })
            "#,
        )
        .unwrap();
    let Value::Object(instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Int(17), empty.clone()],
        )
        .unwrap()
    else {
        panic!("raw primitive newTarget did not produce a base instance");
    };
    assert_eq!(
        context.eval("__qjo_raw_base_new_target").unwrap(),
        Value::Int(17)
    );
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );

    let new_target = context
        .eval(
            r#"
                globalThis.__qjo_raw_custom_prototype = { custom: true };
                (globalThis.__qjo_raw_new_target_object = {
                    prototype: globalThis.__qjo_raw_custom_prototype
                })
            "#,
        )
        .unwrap();
    let Value::Object(instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor, new_target.clone(), empty.clone()],
        )
        .unwrap()
    else {
        panic!("raw object newTarget did not produce a base instance");
    };
    let Value::Object(expected_prototype) = context.eval("__qjo_raw_custom_prototype").unwrap()
    else {
        panic!("custom newTarget prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(expected_prototype)
    );
    assert_eq!(
        context.eval("__qjo_raw_base_new_target").unwrap(),
        new_target
    );

    let function_prototype = context.function_prototype().unwrap();
    let native = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ConstructorProbe,
            0,
        )
        .unwrap();
    runtime
        .set_constructor_bit(native.as_object(), true)
        .unwrap();
    assert_eq!(
        expect_string_value(
            context
                .call(
                    &wrapper,
                    Value::Undefined,
                    &[
                        Value::Object(native.as_object().clone()),
                        Value::Int(17),
                        empty.clone(),
                    ],
                )
                .unwrap()
        ),
        JsString::from_static("0|0|0|false")
    );

    let bound = context
        .eval(
            r#"
                globalThis.__qjo_bound_target = function BoundTarget() {
                    globalThis.__qjo_bound_rewrite =
                        new.target === globalThis.__qjo_bound_target;
                };
                (globalThis.__qjo_bound = __qjo_bound_target.bind(null))
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[bound.clone(), bound, empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_bound_rewrite").unwrap(),
        Value::Bool(true)
    );

    let nested_bound = context
        .eval(
            r#"
                globalThis.__qjo_nested_bound_target = function (a, b, c) {
                    globalThis.__qjo_nested_bound_log =
                        new.target + ":" + a + ":" + b + ":" + c;
                };
                globalThis.__qjo_nested_bound_inner =
                    __qjo_nested_bound_target.bind(null, 1);
                __qjo_nested_bound_inner.bind(null, 2)
            "#,
        )
        .unwrap();
    let third_argument = context.eval("[3]").unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[nested_bound, Value::Int(17), third_argument],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        expect_string_value(context.eval("__qjo_nested_bound_log").unwrap()),
        JsString::from_static("17:1:2:3")
    );

    let proxy = context
        .eval(
            r#"
                (globalThis.__qjo_raw_proxy = new Proxy(function ProxyTarget() {}, {
                    construct(target, args, newTarget) {
                        globalThis.__qjo_proxy_new_target = newTarget;
                        return { proxy: true };
                    }
                }))
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[proxy, Value::Int(17), empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_proxy_new_target").unwrap(),
        Value::Int(17)
    );

    let proxy_without_trap = context
        .eval(
            r#"
                globalThis.__qjo_proxy_without_trap_target = function () {
                    globalThis.__qjo_proxy_without_trap_new_target = new.target;
                };
                new Proxy(__qjo_proxy_without_trap_target, {})
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[proxy_without_trap, Value::Int(17), empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_proxy_without_trap_new_target").unwrap(),
        Value::Int(17)
    );

    let primitive_proxy_result = context
        .eval("new Proxy(function () {}, { construct() { return 1; } })")
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[primitive_proxy_result, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );

    let revoked_target = context
        .eval(
            "globalThis.__qjo_revoked_target = \
                Proxy.revocable(function () {}, {}); \
             __qjo_revoked_target.revoke(); \
             __qjo_revoked_target.proxy",
        )
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[revoked_target, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );

    let revoked_new_target = context
        .eval(
            "globalThis.__qjo_revoked_new_target = \
                Proxy.revocable(function () {}, {}); \
             __qjo_revoked_new_target.revoke(); \
             __qjo_revoked_new_target.proxy",
        )
        .unwrap();
    let body_probe = context
        .eval(
            "globalThis.__qjo_revoked_body_count = 0; \
             (function () { __qjo_revoked_body_count++; })",
        )
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[body_probe, revoked_new_target, empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );
    assert_eq!(
        context.eval("__qjo_revoked_body_count").unwrap(),
        Value::Int(0)
    );

    let outer_proxy = context
        .eval(
            r#"
                globalThis.__qjo_outer_construct_gets = 0;
                globalThis.__qjo_inner_construct_gets = 0;
                var innerHandler = {};
                Object.defineProperty(innerHandler, "construct", {
                    get() {
                        __qjo_inner_construct_gets++;
                        return undefined;
                    }
                });
                globalThis.__qjo_inner_record =
                    Proxy.revocable(function InnerTarget() {}, innerHandler);
                var outerHandler = {};
                Object.defineProperty(outerHandler, "construct", {
                    get() {
                        __qjo_outer_construct_gets++;
                        return undefined;
                    }
                });
                new Proxy(__qjo_inner_record.proxy, outerHandler)
            "#,
        )
        .unwrap();
    let Value::Object(inner_proxy) = context.eval("__qjo_inner_record.proxy").unwrap() else {
        panic!("inner Proxy was not an object");
    };
    runtime.set_constructor_bit(&inner_proxy, false).unwrap();
    context.eval("__qjo_inner_record.revoke()").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[outer_proxy, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("not a constructor"),
        )
    );
    assert_eq!(
        context.eval("__qjo_outer_construct_gets").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("__qjo_inner_construct_gets").unwrap(),
        Value::Int(0)
    );

    let non_callable_trap = context
        .eval(
            "globalThis.__qjo_trap_order_target = \
                function TrapOrderTarget() {}; \
             new Proxy(__qjo_trap_order_target, { construct: 1 })",
        )
        .unwrap();
    let Value::Object(trap_order_target) = context.eval("__qjo_trap_order_target").unwrap() else {
        panic!("trap-order target was not an object");
    };
    runtime
        .set_constructor_bit(&trap_order_target, false)
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[non_callable_trap, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("TrapOrderTarget is not a constructor"),
        )
    );

    let derived = context
        .eval(
            r#"
                class RawBase {
                    constructor() {
                        globalThis.__qjo_raw_super_new_target = new.target;
                        this.base = true;
                    }
                }
                class RawDerived extends RawBase {
                    constructor() {
                        super();
                        globalThis.__qjo_raw_derived_new_target = new.target;
                    }
                }
                RawDerived
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[derived, Value::Int(17), empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_raw_super_new_target").unwrap(),
        Value::Int(17)
    );
    assert_eq!(
        context.eval("__qjo_raw_derived_new_target").unwrap(),
        Value::Int(17)
    );

    let spread_derived = context
        .eval(
            r#"
                class RawSpreadBase {
                    constructor(a, b) {
                        globalThis.__qjo_raw_spread_super =
                            new.target + ":" + a + ":" + b;
                        this.sum = a + b;
                    }
                }
                class RawSpreadDerived extends RawSpreadBase {
                    constructor(...args) { super(...args); }
                }
                RawSpreadDerived
            "#,
        )
        .unwrap();
    let spread_arguments = context.eval("[20, 22]").unwrap();
    let Value::Object(spread_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[spread_derived, Value::Int(17), spread_arguments],
        )
        .unwrap()
    else {
        panic!("ApplySuper with raw primitive newTarget did not return an object");
    };
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&spread_instance, &sum).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_raw_spread_super").unwrap()),
        JsString::from_static("17:20:22")
    );

    let array = context.eval("Array").unwrap();
    let list = context.eval("[1, 2]").unwrap();
    let Value::Object(array) = context
        .call(&wrapper, Value::Undefined, &[array, Value::Int(17), list])
        .unwrap()
    else {
        panic!("Array with raw primitive newTarget did not return an object");
    };
    assert!(runtime.is_array_object(&array).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(context.array_prototype().unwrap())
    );
    assert!(!context.has_exception());
}

#[test]
fn construct_only_proxy_and_new_target_do_not_require_call_capability() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let target = context.new_object().unwrap();
    runtime.set_constructor_bit(&target, true).unwrap();
    let global = context.global_object().unwrap();
    let target_key = runtime
        .intern_property_key("__qjo_construct_only_target")
        .unwrap();
    assert!(
        context
            .set_property(&global, &target_key, Value::Object(target.clone()))
            .unwrap()
    );

    let Value::Object(proxy) = context
        .eval(
            r#"
                globalThis.__qjo_construct_only_log = "";
                globalThis.__qjo_construct_only_proxy =
                    new Proxy(__qjo_construct_only_target, {
                        construct(target, args, newTarget) {
                            __qjo_construct_only_seen_new_target = newTarget;
                            __qjo_construct_only_log =
                                (target === __qjo_construct_only_target) + ":" +
                                args[0] + ":" + newTarget;
                            return { trapped: true };
                        }
                    });
                __qjo_construct_only_proxy
            "#,
        )
        .unwrap()
    else {
        panic!("construct-only Proxy was not an object");
    };
    assert!(runtime.is_constructor(&proxy).unwrap());
    assert!(runtime.as_callable(&proxy).unwrap().is_none());

    let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
    assert!(matches!(
        VmHost::construct(
            &mut host,
            Value::Object(proxy.clone()),
            Value::Int(17),
            vec![Value::Int(42)],
        )
        .unwrap(),
        Completion::Return(Value::Object(_))
    ));
    assert_eq!(
        expect_string_value(context.eval("__qjo_construct_only_log").unwrap()),
        JsString::from_static("true:42:17")
    );
    assert_eq!(
        context
            .eval("__qjo_construct_only_seen_new_target")
            .unwrap(),
        Value::Int(17)
    );

    let new_target = context.new_object().unwrap();
    runtime.set_constructor_bit(&new_target, true).unwrap();
    let prototype = context.new_object().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        context
            .set_property(
                &new_target,
                &prototype_key,
                Value::Object(prototype.clone()),
            )
            .unwrap()
    );
    let new_target_key = runtime
        .intern_property_key("__qjo_construct_only_new_target")
        .unwrap();
    assert!(
        context
            .set_property(&global, &new_target_key, Value::Object(new_target.clone()),)
            .unwrap()
    );
    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            function Base() {}
                            var instance = Reflect.construct(
                                Base,
                                [],
                                __qjo_construct_only_new_target
                            );
                            var trapped = Reflect.construct(
                                __qjo_construct_only_proxy,
                                [9],
                                __qjo_construct_only_new_target
                            );
                            var source = [1];
                            source.constructor = __qjo_construct_only_proxy;
                            return (
                                Object.getPrototypeOf(instance) ===
                                __qjo_construct_only_new_target.prototype
                            ) + ":" + trapped.trapped + ":" +
                                Array.isArray(source.slice());
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("true:true:true")
    );
    assert_eq!(
        context
            .eval("__qjo_construct_only_seen_new_target")
            .unwrap(),
        Value::Object(new_target)
    );

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            var missing = new Proxy(__qjo_construct_only_target, {});
                            var record = Proxy.revocable(
                                __qjo_construct_only_target,
                                { construct() { return {}; } }
                            );
                            record.revoke();
                            var results = [];
                            for (var candidate of [missing, record.proxy]) {
                                try { Reflect.construct(candidate, []); }
                                catch (error) { results.push(error.message); }
                            }
                            return results.join("|");
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("not a function|revoked proxy")
    );

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            var log = [];
                            var innerHandler = {};
                            Object.defineProperty(innerHandler, "apply", {
                                get() { log.push("inner"); return undefined; }
                            });
                            var inner = new Proxy({}, innerHandler);
                            var outerHandler = {};
                            Object.defineProperty(outerHandler, "apply", {
                                get() { log.push("outer"); return undefined; }
                            });
                            var outer = new Proxy(inner, outerHandler);
                            var candidate = new Proxy(
                                __qjo_construct_only_target,
                                { construct: outer }
                            );
                            try { Reflect.construct(candidate, []); }
                            catch (error) {
                                return log.join("|") + "|" +
                                    error.name + ":" + error.message;
                            }
                            return "missing";
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("outer|TypeError:not a function")
    );
}

#[test]
fn trusted_quickjs_ordinary_apply_raw_calling_realm_functions_fall_back_to_the_caller() {
    let runtime = Runtime::new();
    let mut caller = runtime.new_context();
    let wrapper = caller
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let array = caller.eval("Array").unwrap();
    let empty = caller.eval("[]").unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();

    let mut foreign = runtime.new_context();
    let promise_resolve = foreign
        .eval("new Promise(function (resolve) { globalThis.resolve = resolve; }); resolve")
        .unwrap();
    let proxy_revoke = foreign.eval("Proxy.revocable({}, {}).revoke").unwrap();

    for (label, raw_new_target) in [
        ("Promise resolving function", promise_resolve),
        ("Proxy revoke function", proxy_revoke),
    ] {
        let Value::Object(array_instance) = caller
            .call(
                &wrapper,
                Value::Undefined,
                &[array.clone(), raw_new_target, empty.clone()],
            )
            .unwrap()
        else {
            panic!("{label} raw newTarget did not construct an Array");
        };
        assert_eq!(
            runtime.get_prototype_of(&array_instance).unwrap(),
            Some(caller_array_prototype.clone()),
            "{label}"
        );
    }

    foreign
        .eval(
            r#"
                globalThis.__qjo_calling_realm_caught = undefined;
                globalThis.__qjo_calling_realm_promise = new Promise(function (resolve) {
                    globalThis.__qjo_calling_realm_resolve = resolve;
                });
                __qjo_calling_realm_promise.catch(function (error) {
                    globalThis.__qjo_calling_realm_caught = error;
                });
            "#,
        )
        .unwrap();
    let promise = foreign.eval("__qjo_calling_realm_promise").unwrap();
    let resolve = foreign.eval("__qjo_calling_realm_resolve").unwrap();
    let promise_list = foreign.eval("[__qjo_calling_realm_promise]").unwrap();
    let Value::Object(resolve_object) = resolve.clone() else {
        panic!("Promise resolving function was not an object");
    };
    runtime.set_constructor_bit(&resolve_object, true).unwrap();
    assert_eq!(
        caller
            .call(
                &wrapper,
                Value::Undefined,
                &[resolve, Value::Int(17), promise_list],
            )
            .unwrap(),
        Value::Undefined
    );
    runtime.set_constructor_bit(&resolve_object, false).unwrap();
    assert_eq!(
        runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(caller.realm)
    );
    let Value::Object(caught) = foreign.eval("__qjo_calling_realm_caught").unwrap() else {
        panic!("Promise self-resolution did not capture an error");
    };
    let Value::Object(caller_type_error_prototype) = caller.eval("TypeError.prototype").unwrap()
    else {
        panic!("caller TypeError prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&caught).unwrap(),
        Some(caller_type_error_prototype)
    );
    drop(promise);

    foreign
        .eval(
            "globalThis.__qjo_revoke_record = Proxy.revocable({}, {}); \
             globalThis.__qjo_revoke_proxy = __qjo_revoke_record.proxy;",
        )
        .unwrap();
    let revoke = foreign.eval("__qjo_revoke_record.revoke").unwrap();
    let Value::Object(revoke_object) = revoke.clone() else {
        panic!("Proxy revoke function was not an object");
    };
    runtime.set_constructor_bit(&revoke_object, true).unwrap();
    assert_eq!(
        caller
            .call(&wrapper, Value::Undefined, &[revoke, Value::Int(17), empty],)
            .unwrap(),
        Value::Undefined
    );
    runtime.set_constructor_bit(&revoke_object, false).unwrap();
    assert_eq!(
        foreign.eval("__qjo_revoke_proxy.marker"),
        Err(RuntimeError::Exception)
    );
    assert!(matches!(
        foreign.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
}

#[test]
fn trusted_quickjs_ordinary_apply_raw_native_constructors_use_class_fallbacks() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let wrapper = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();

    let direct_arguments = super::NativeArguments {
        actual_arg_count: 0,
        readable: Vec::new(),
    };
    let Completion::Return(Value::Object(direct_array)) = runtime
        .call_array_constructor(
            context.realm,
            super::NativeInvocation::Construct {
                new_target: Value::Undefined,
            },
            &direct_arguments,
        )
        .unwrap()
    else {
        panic!("Array undefined newTarget required an active function");
    };
    assert_eq!(
        runtime.get_prototype_of(&direct_array).unwrap(),
        Some(context.array_prototype().unwrap())
    );

    let array_constructor = context.eval("Array").unwrap();
    let empty_arguments = context.eval("[]").unwrap();
    let Value::Object(raw_undefined_array) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[array_constructor, Value::Undefined, empty_arguments],
        )
        .unwrap()
    else {
        panic!("raw Array undefined newTarget did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&raw_undefined_array).unwrap(),
        Some(context.array_prototype().unwrap())
    );

    let function_prototype = context.function_prototype().unwrap();
    let custom_array = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ArrayConstructor,
            1,
        )
        .unwrap();
    runtime
        .set_constructor_bit(custom_array.as_object(), true)
        .unwrap();
    let custom_prototype = context.new_object().unwrap();
    assert!(
        context
            .define_own_property(
                custom_array.as_object(),
                &prototype_key,
                &data_descriptor(Value::Object(custom_prototype.clone()), true, false, true,),
            )
            .unwrap()
    );
    let custom_empty_arguments = context.eval("[]").unwrap();
    let Value::Object(custom_array_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[
                Value::Object(custom_array.as_object().clone()),
                Value::Undefined,
                custom_empty_arguments,
            ],
        )
        .unwrap()
    else {
        panic!("custom native Array did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_array_instance).unwrap(),
        Some(context.array_prototype().unwrap())
    );
    assert_ne!(
        runtime.get_prototype_of(&custom_array_instance).unwrap(),
        Some(custom_prototype)
    );

    let cases = [
        ("Array", "Array", "[]"),
        ("Object", "Object", "[]"),
        ("Function", "Function", "['return 1']"),
        (
            "GeneratorFunction",
            "(function* () {}).constructor",
            "['yield 1']",
        ),
        (
            "AsyncFunction",
            "(async function () {}).constructor",
            "['return 1']",
        ),
        (
            "AsyncGeneratorFunction",
            "(async function* () {}).constructor",
            "['yield 1']",
        ),
        ("Boolean", "Boolean", "[true]"),
        ("Number", "Number", "[1]"),
        ("String", "String", "['x']"),
        ("Error", "Error", "['x']"),
        ("EvalError", "EvalError", "['x']"),
        ("RangeError", "RangeError", "['x']"),
        ("ReferenceError", "ReferenceError", "['x']"),
        ("SyntaxError", "SyntaxError", "['x']"),
        ("TypeError", "TypeError", "['x']"),
        ("URIError", "URIError", "['x']"),
        ("InternalError", "InternalError", "['x']"),
        ("AggregateError", "AggregateError", "[[], 'x']"),
        ("Date", "Date", "[0]"),
        ("RegExp", "RegExp", "['a']"),
        ("Map", "Map", "[]"),
        ("Set", "Set", "[]"),
        ("WeakMap", "WeakMap", "[]"),
        ("WeakSet", "WeakSet", "[]"),
        ("Promise", "Promise", "[function (resolve) { resolve(1); }]"),
        ("ArrayBuffer", "ArrayBuffer", "[8]"),
        ("SharedArrayBuffer", "SharedArrayBuffer", "[8]"),
        ("DataView", "DataView", "[new ArrayBuffer(8)]"),
        ("Uint8ClampedArray", "Uint8ClampedArray", "[2]"),
        ("Int8Array", "Int8Array", "[2]"),
        ("Uint8Array", "Uint8Array", "[2]"),
        ("Int16Array", "Int16Array", "[2]"),
        ("Uint16Array", "Uint16Array", "[2]"),
        ("Int32Array", "Int32Array", "[2]"),
        ("Uint32Array", "Uint32Array", "[2]"),
        ("BigInt64Array", "BigInt64Array", "[2]"),
        ("BigUint64Array", "BigUint64Array", "[2]"),
        ("Float16Array", "Float16Array", "[2]"),
        ("Float32Array", "Float32Array", "[2]"),
        ("Float64Array", "Float64Array", "[2]"),
        ("WeakRef", "WeakRef", "[{}]"),
        (
            "FinalizationRegistry",
            "FinalizationRegistry",
            "[function () {}]",
        ),
    ];

    for (label, constructor_source, arguments_source) in cases {
        let constructor = context.eval(constructor_source).unwrap();
        let Value::Object(constructor_object) = constructor.clone() else {
            panic!("{label} constructor was not an object");
        };
        let Value::Object(expected_prototype) = context
            .get_property(&constructor_object, &prototype_key)
            .unwrap()
        else {
            panic!("{label}.prototype was not an object");
        };
        let arguments = context.eval(arguments_source).unwrap();
        let Value::Object(instance) = context
            .call(
                &wrapper,
                Value::Undefined,
                &[constructor, Value::Int(17), arguments],
            )
            .unwrap()
        else {
            panic!("{label} with raw primitive newTarget did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&instance).unwrap(),
            Some(expected_prototype),
            "{label}"
        );
        assert!(!context.has_exception(), "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    for (label, code, max_stack, expected) in [
        (
            "apply underflow",
            &[0x27, 0x00, 0x00, 0x28][..],
            0,
            UNDERFLOW,
        ),
        (
            "apply declared maximum",
            &[0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28][..],
            2,
            DECLARED_MAXIMUM,
        ),
    ] {
        let image = quickjs_ordinary_three_argument_with_code(code, max_stack);
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    // The taken label8 jump skips raw rot3l. rot3l expands to Perm3, Swap,
    // so the source target at OP_apply becomes IR instruction 9.
    let code = [
        0xcf, 0xd0, 0xd1, 0xea, 0x05, 0xb4, 0xb5, 0xb6, 0x1d, 0x27, 0x00, 0x00, 0x28,
    ];
    let image = quickjs_ordinary_three_argument_with_code(&code, 3);
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let receiver = context
        .eval("(globalThis.__qjo_branch_receiver = {})")
        .unwrap();
    let target = context
        .eval("(function () { 'use strict'; return this === globalThis.__qjo_branch_receiver; })")
        .unwrap();
    let list = context.eval("[]").unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[target, receiver, list])
            .unwrap(),
        Value::Bool(true)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("branch-to-apply leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::GetArg(2),
            Instruction::Goto(9),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::Apply(ApplyKind::Call),
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_non_tail_invocation_exceptions_are_recoverable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let mut construct_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CONSTRUCT_CODE, &[]);
    construct_image[33] = 4;
    let construct = context
        .read_trusted_ordinary_function(&construct_image, 0)
        .unwrap();
    assert_eq!(
        context.call(
            &construct,
            Value::Undefined,
            &[Value::Int(0), Value::Int(0)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );
    assert!(!context.has_exception());

    let constructor = context
        .eval(
            "(function C(a, b) { \
                globalThis.__qjo_construct_raw_new_target = new.target; \
                this.sum = a + b; \
            })",
        )
        .unwrap();
    let Value::Object(instance) = context
        .call(
            &construct,
            Value::Undefined,
            &[constructor.clone(), Value::Int(0)],
        )
        .unwrap()
    else {
        panic!("raw nonconstructor newTarget did not construct an object");
    };
    assert_eq!(
        context.eval("__qjo_construct_raw_new_target").unwrap(),
        Value::Int(0)
    );
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&instance, &sum).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert!(!context.has_exception());

    let mut method_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CALL_METHOD_CODE, &[]);
    method_image[33] = 3;
    let call_method = context
        .read_trusted_ordinary_function(&method_image, 0)
        .unwrap();
    let receiver = context.eval("({ base: 1 })").unwrap();
    assert_eq!(
        context.call(&call_method, Value::Undefined, &[receiver, Value::Int(0)],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("not a function"),
        )
    );
    assert!(!context.has_exception());

    let new_target = context.eval("(function N() {})").unwrap();
    assert!(matches!(
        context
            .call(&construct, Value::Undefined, &[constructor, new_target],)
            .unwrap(),
        Value::Object(_)
    ));
    let receiver = context.eval("({ base: 1 })").unwrap();
    let method = context
        .eval("(function (value) { 'use strict'; return this.base + value; })")
        .unwrap();
    assert_eq!(
        context
            .call(&call_method, Value::Undefined, &[receiver, method])
            .unwrap(),
        Value::Int(43)
    );
}

#[test]
fn trusted_quickjs_ordinary_non_tail_invocation_verification_rejects_before_publication() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";
    const CASES: [(&str, &[u8], u8, &str); 6] = [
        (
            "construct underflow",
            &[0x21, 0x00, 0x00, 0x28],
            0,
            UNDERFLOW,
        ),
        ("method underflow", &[0x24, 0x00, 0x00, 0x28], 0, UNDERFLOW),
        ("array underflow", &[0x26, 0x01, 0x00, 0x28], 0, UNDERFLOW),
        (
            "construct declared maximum",
            &[0x06, 0x06, 0x21, 0x00, 0x00, 0x28],
            1,
            DECLARED_MAXIMUM,
        ),
        (
            "method declared maximum",
            &[0x06, 0x06, 0x24, 0x00, 0x00, 0x28],
            1,
            DECLARED_MAXIMUM,
        ),
        (
            "array declared maximum",
            &[0xb4, 0x26, 0x01, 0x00, 0x28],
            0,
            DECLARED_MAXIMUM,
        ),
    ];

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    for (label, code, max_stack, expected_message) in CASES {
        let mut image = quickjs_ordinary_with_code_and_constants(code, &[]);
        image[33] = max_stack;
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected_message, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_can_land_on_call_method_after_expansion() {
    let code = [
        0xcf, 0xd0, 0xea, 0x05, 0xb4, 0xb5, 0xb6, 0x1d, 0x24, 0x00, 0x00, 0x28,
    ];
    let mut image = quickjs_ordinary_with_code_and_constants(&code, &[]);
    image[33] = 2;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let receiver = context.eval("({ base: 42 })").unwrap();
    let method = context
        .eval("(function () { 'use strict'; return this.base; })")
        .unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[receiver, method])
            .unwrap(),
        Value::Int(42)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary branch-to-method leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::Goto(8),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::CallMethod(0),
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_plain_call_verification_rejections_do_not_publish() {
    const CASES: [(&str, &[u8], u8, &str); 2] = [
        (
            "call operand underflow",
            &[0x22, 0x04, 0x00, 0x28],
            0,
            "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow",
        ),
        (
            "declared maximum stack",
            &[0x06, 0xb4, 0xb5, 0xb6, 0xb7, 0x22, 0x04, 0x00, 0x28],
            4,
            "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required",
        ),
    ];

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    for (label, code, max_stack, expected_message) in CASES {
        let mut image = quickjs_ordinary_with_code_and_constants(code, &[]);
        image[33] = max_stack;
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected_message, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_follow_the_first_expanded_instruction() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    // The taken label8 jump skips a raw rot3l. rot3l expands to Perm3, Swap,
    // so the native target at source instruction 6 becomes IR 7.
    let code = [0xbb, 1, 0xbb, 2, 0xbb, 3, 0x0a, 0xe9, 2, 0x1d, 0x28];
    let mut image = quickjs_ordinary_with_code_and_constants(&code, &[]);
    image[33] = 4;

    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Int(3)
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::PushTrue,
            Instruction::IfTrue(7),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_can_land_on_plain_call_after_expansion() {
    let code = [0xcf, 0xea, 0x05, 0xb4, 0xb5, 0xb6, 0x1d, 0xec, 0x28];
    let mut image = quickjs_ordinary_with_code_and_constants(&code, &[]);
    image[33] = 1;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let callback = context
        .eval("(0, function () { 'use strict'; return this === undefined; })")
        .unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[callback])
            .unwrap(),
        Value::Bool(true)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary branch-to-call leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 1);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Goto(7),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::Call(0),
            Instruction::Return,
        ]
    ));
}

#[cfg(feature = "test262-host")]
#[test]
fn trusted_quickjs_ordinary_fused_predicates_distinguish_htmldda() {
    fn predicate(context: &mut super::Context, raw: u8, value: Value) -> Value {
        let image = quickjs_ordinary_with_code_and_constants(&[0xcf, raw, 0x28], &[]);
        let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
        context.call(&function, Value::Undefined, &[value]).unwrap()
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(normal_callable) = context.eval("(function () {})").unwrap() else {
        panic!("function expression did not produce an Object");
    };
    let Value::Object(callable_proxy) = context.eval("new Proxy(function () {}, {})").unwrap()
    else {
        panic!("callable Proxy did not produce an Object");
    };
    let Value::Object(html_dda_callable) = context.eval("(function () {})").unwrap() else {
        panic!("function expression did not produce an Object");
    };
    runtime.set_object_is_html_dda(&html_dda_callable).unwrap();
    let Value::Object(ordinary_object) = context.eval("({})").unwrap() else {
        panic!("object literal did not produce an Object");
    };

    assert_eq!(
        predicate(&mut context, 0xf0, Value::Undefined),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf0, Value::Object(html_dda_callable.clone())),
        Value::Bool(false)
    );
    assert_eq!(
        predicate(&mut context, 0xf1, Value::Null),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf1, Value::Object(html_dda_callable.clone())),
        Value::Bool(false)
    );
    assert_eq!(
        predicate(&mut context, 0xf2, Value::Undefined),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf2, Value::Object(html_dda_callable.clone())),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(normal_callable)),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(callable_proxy)),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(html_dda_callable)),
        Value::Bool(false)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(ordinary_object)),
        Value::Bool(false)
    );
}

#[test]
fn trusted_quickjs_tagged_atom_strings_are_fresh_on_every_execution() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    for (atom, expected) in [
        (0x8000_0000, "0"),
        (0x8000_002a, "42"),
        (0xffff_ffff, "2147483647"),
    ] {
        let image = quickjs_scalar_with_atom_value(atom);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        let first = expect_string_value(context.execute(&function).unwrap());
        let second = expect_string_value(context.execute(&function).unwrap());
        assert_eq!(first, JsString::from_static(expected));
        assert_eq!(second, first);
        assert!(!first.same_representation(&second));
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    let slot_alias = quickjs_scalar_with_atom_slot(&[u16::from(b'4'), u16::from(b'2')], false);
    let function = context.read_trusted_scalar_script(&slot_alias).unwrap();
    let first = expect_string_value(context.execute(&function).unwrap());
    let second = expect_string_value(context.execute(&function).unwrap());
    assert_eq!(first, JsString::from_static("42"));
    assert!(!first.same_representation(&second));
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn trusted_quickjs_non_string_atoms_remain_unsupported_without_publication() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let cases = [
        quickjs_scalar_with_atom_value(0),
        quickjs_scalar_with_atom_value(229),
        quickjs_scalar_with_atom_value(230),
        quickjs_scalar_with_unused_atom_slot(50, &[u16::from(b'x')], false),
        quickjs_scalar_with_two_atom_slots(),
    ];
    for image in cases {
        let RuntimeError::Engine(error) = context.read_trusted_scalar_script(&image).unwrap_err()
        else {
            panic!("non-String atom did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts(), baseline);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_atom_string_publication_rolls_back_a_stale_realm() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let stale_realm = context.realm;
    drop(context);
    runtime.run_gc().unwrap();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let image = quickjs_scalar_with_atom_slot(
        &"quickjs-oxide-rollback-string-scalar"
            .encode_utf16()
            .collect::<Vec<_>>(),
        false,
    );

    assert!(
        runtime
            .read_trusted_scalar_script_in_realm(stale_realm, &image)
            .is_err()
    );
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn trusted_quickjs_scalar_script_executes_exact_float64_constant_pairs() {
    const SHORT_INDEX_ZERO: &[u8] = &[0xbd, 0x00, 0xcb, 0x28];
    const WIDE_INDEX_ZERO: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let mut functions = Vec::new();
    let bits_cases = [
        0.5_f64.to_bits(),
        2_147_483_648_f64.to_bits(),
        1,
        f64::MAX.to_bits(),
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        0.0_f64.to_bits(),
        (-0.0_f64).to_bits(),
        42.0_f64.to_bits(),
        0x7ff8_0000_0000_0042,
        0x7ff0_0000_0000_0042,
    ];

    for bits in bits_cases {
        let image = quickjs_scalar_with_float_constant(SHORT_INDEX_ZERO, bits);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        let Value::Float(actual) = context.execute(&function).unwrap() else {
            panic!("Float64 constant did not retain its runtime value kind");
        };
        assert_eq!(actual.to_bits(), bits);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    let image = quickjs_scalar_with_float_constant(WIDE_INDEX_ZERO, 0.5_f64.to_bits());
    let function = context.read_trusted_scalar_script(&image).unwrap();
    let Value::Float(actual) = context.execute(&function).unwrap() else {
        panic!("wide Float64 constant did not retain its runtime value kind");
    };
    assert_eq!(actual.to_bits(), 0.5_f64.to_bits());
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
    functions.push(function);
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + bits_cases.len() + 1
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_exact_bigint_constant_pairs() {
    const SHORT_INDEX_ZERO: &[u8] = &[0xbd, 0x00, 0xcb, 0x28];
    const WIDE_INDEX_ZERO: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let cases = vec![
        (Vec::new(), JsBigInt::zero()),
        (vec![0x01], JsBigInt::one()),
        (vec![0xff], JsBigInt::from(-1)),
        (
            vec![0x00, 0x00, 0x00, 0x80, 0x00],
            JsBigInt::from(2_147_483_648_i64),
        ),
        (
            vec![0xff, 0xff, 0xff, 0x7f, 0xff],
            JsBigInt::from(-2_147_483_649_i64),
        ),
        (
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            JsBigInt::from(i64::MAX),
        ),
        (
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00],
            JsBigInt::parse_js_string("9223372036854775808").unwrap(),
        ),
        (
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff],
            JsBigInt::parse_js_string("-9223372036854775809").unwrap(),
        ),
        (
            vec![
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01,
            ],
            JsBigInt::parse_js_string("340282366920938463463374607431768211456").unwrap(),
        ),
        // Compatible BC5 accepts redundant sign extension; the archival
        // reader normalizes it before runtime publication.
        (vec![0x00], JsBigInt::zero()),
        (vec![0x01, 0x00], JsBigInt::one()),
        (vec![0xff, 0xff], JsBigInt::from(-1)),
    ];
    let mut functions = Vec::new();

    for (payload, expected) in &cases {
        let image = quickjs_scalar_with_bigint_constant(SHORT_INDEX_ZERO, payload);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(
            context.execute(&function).unwrap(),
            Value::BigInt(expected.clone())
        );
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    let image =
        quickjs_scalar_with_bigint_constant(WIDE_INDEX_ZERO, &[0x00, 0x00, 0x00, 0x80, 0x00]);
    let function = context.read_trusted_scalar_script(&image).unwrap();
    assert_eq!(
        context.execute(&function).unwrap(),
        Value::BigInt(JsBigInt::from(2_147_483_648_i64))
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
    functions.push(function);
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + cases.len() + 1
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_bigint_negation_at_runtime() {
    const SHORT_INDEX_ZERO_NEG: &[u8] = &[0xbd, 0x00, 0x8a, 0xcb, 0x28];
    const WIDE_INDEX_ZERO_NEG: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0x8a, 0xcb, 0x28];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let constant_cases = vec![
        (Vec::new(), JsBigInt::zero()),
        (vec![0x01], JsBigInt::from(-1)),
        (vec![0xff], JsBigInt::one()),
        (
            vec![0x00, 0x00, 0x00, 0x80, 0x00],
            JsBigInt::from(-2_147_483_648_i64),
        ),
        (
            vec![0xff, 0xff, 0xff, 0x7f, 0xff],
            JsBigInt::from(2_147_483_649_i64),
        ),
        // Negating the largest-magnitude short BigInt must promote rather
        // than overflow: i64::MIN becomes the positive heap value 2^63.
        (
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
            JsBigInt::parse_js_string("9223372036854775808").unwrap(),
        ),
        (
            vec![
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01,
            ],
            JsBigInt::parse_js_string("-340282366920938463463374607431768211456").unwrap(),
        ),
        // Compatible redundant sign extension is normalized before the
        // runtime receives the value, while negation still executes in the VM.
        (vec![0x00], JsBigInt::zero()),
        (vec![0x01, 0x00], JsBigInt::from(-1)),
        (vec![0xff, 0xff], JsBigInt::one()),
    ];
    let direct_cases = [
        (0_i32, JsBigInt::zero()),
        (1, JsBigInt::from(-1)),
        (-1, JsBigInt::one()),
        (i32::MAX, JsBigInt::from(-i64::from(i32::MAX))),
        (i32::MIN, JsBigInt::parse_js_string("2147483648").unwrap()),
    ];
    let mut functions = Vec::new();

    for (payload, expected) in &constant_cases {
        let image = quickjs_scalar_with_bigint_constant(SHORT_INDEX_ZERO_NEG, payload);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(
            context.execute(&function).unwrap(),
            Value::BigInt(expected.clone())
        );
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    let image =
        quickjs_scalar_with_bigint_constant(WIDE_INDEX_ZERO_NEG, &[0x00, 0x00, 0x00, 0x80, 0x00]);
    let function = context.read_trusted_scalar_script(&image).unwrap();
    assert_eq!(
        context.execute(&function).unwrap(),
        Value::BigInt(JsBigInt::from(-2_147_483_648_i64))
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
    functions.push(function);

    for (input, expected) in direct_cases {
        let mut code = vec![0xb0];
        code.extend_from_slice(&input.to_le_bytes());
        code.extend_from_slice(&[0x8a, 0xcb, 0x28]);
        let image = quickjs_scalar_with_code(&code);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::BigInt(expected));
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + functions.len()
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_table_driven_unary_chains() {
    const NEG: u8 = 0x8a;
    const PLUS: u8 = 0x8b;
    const DEC: u8 = 0x8c;
    const INC: u8 = 0x8d;
    const BIT_NOT: u8 = 0x93;
    const LOGICAL_NOT: u8 = 0x94;
    const TYPEOF: u8 = 0x95;

    fn direct_code(push: &[u8], unary_ops: &[u8]) -> Vec<u8> {
        let mut code = Vec::with_capacity(push.len() + unary_ops.len() + 2);
        code.extend_from_slice(push);
        code.extend_from_slice(unary_ops);
        code.extend_from_slice(&[0xcb, 0x28]);
        code
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let direct_cases = [
        (direct_code(&[0xb4], &[NEG]), Value::Int(-1)),
        (direct_code(&[0x09], &[PLUS]), Value::Int(0)),
        (direct_code(&[0xb3], &[DEC]), Value::Int(-1)),
        (direct_code(&[0xb3], &[INC]), Value::Int(1)),
        (direct_code(&[0xb3], &[BIT_NOT]), Value::Int(-1)),
        (direct_code(&[0x0a], &[LOGICAL_NOT]), Value::Bool(false)),
        // Exercises all six value-transforming operations before `typeof`;
        // reader and publisher snapshots separately pin their exact order.
        (
            direct_code(
                &[0x0a],
                &[PLUS, INC, DEC, NEG, BIT_NOT, LOGICAL_NOT, TYPEOF],
            ),
            Value::String(JsString::from_static("boolean")),
        ),
        (direct_code(&[0xb4], &[NEG, NEG]), Value::Int(1)),
    ];
    for (code, expected) in &direct_cases {
        let function = context
            .read_trusted_scalar_script(&quickjs_scalar_with_code(code))
            .unwrap();
        assert_eq!(context.execute(&function).unwrap(), expected.clone());
    }

    for (bits, op, expected_bits) in [
        (42.0_f64.to_bits(), NEG, (-42.0_f64).to_bits()),
        ((-0.0_f64).to_bits(), PLUS, (-0.0_f64).to_bits()),
        (
            2_147_483_647.0_f64.to_bits(),
            INC,
            2_147_483_648.0_f64.to_bits(),
        ),
        (42.0_f64.to_bits(), DEC, 41.0_f64.to_bits()),
    ] {
        let code = [0xbd, 0x00, op, 0xcb, 0x28];
        let image = quickjs_scalar_with_float_constant(&code, bits);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        let Value::Float(actual) = context.execute(&function).unwrap() else {
            panic!("Float64 scalar unary result lost its QuickJS value tag");
        };
        assert_eq!(actual.to_bits(), expected_bits);
    }

    let bitnot_nan = quickjs_scalar_with_float_constant(
        &[0xbd, 0x00, BIT_NOT, 0xcb, 0x28],
        0x7ff8_0000_0000_0042,
    );
    let bitnot_nan = context.read_trusted_scalar_script(&bitnot_nan).unwrap();
    assert_eq!(context.execute(&bitnot_nan).unwrap(), Value::Int(-1));

    let string_plus = quickjs_scalar_with_string_constant(
        &[0xbd, 0x00, PLUS, 0xcb, 0x28],
        &[u16::from(b'4'), u16::from(b'2')],
        false,
    );
    let string_plus = context.read_trusted_scalar_script(&string_plus).unwrap();
    assert_eq!(context.execute(&string_plus).unwrap(), Value::Int(42));

    let int_zero_neg = context
        .read_trusted_scalar_script(&quickjs_scalar_with_code(&direct_code(&[0xb3], &[NEG])))
        .unwrap();
    let Value::Float(negative_zero) = context.execute(&int_zero_neg).unwrap() else {
        panic!("negating Int32 zero did not produce QuickJS Float64 negative zero");
    };
    assert_eq!(negative_zero.to_bits(), (-0.0_f64).to_bits());

    for (op, expected) in [
        (NEG, JsBigInt::from(-1)),
        (DEC, JsBigInt::zero()),
        (INC, JsBigInt::from(2)),
        (BIT_NOT, JsBigInt::from(-2)),
    ] {
        let code = direct_code(&[0xb0, 0x01, 0x00, 0x00, 0x00], &[op]);
        let function = context
            .read_trusted_scalar_script(&quickjs_scalar_with_code(&code))
            .unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::BigInt(expected));
    }

    // Pinned QuickJS performs unsigned opcode arithmetic in the heap-BigInt
    // decrement slow path, adding UINT32_MAX instead of subtracting one.
    // This is release behavior, not eager decoder normalization.
    let heap_bigint_dec = quickjs_scalar_with_bigint_constant(
        &[0xbd, 0x00, DEC, 0xcb, 0x28],
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
    );
    let heap_bigint_dec = context
        .read_trusted_scalar_script(&heap_bigint_dec)
        .unwrap();
    assert_eq!(
        context.execute(&heap_bigint_dec).unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("-9223372032559808513").unwrap())
    );

    let typeof_image = quickjs_scalar_with_string_constant(
        &[0xbd, 0x00, TYPEOF, 0xcb, 0x28],
        &[u16::from(b'x')],
        false,
    );
    let typeof_function = context.read_trusted_scalar_script(&typeof_image).unwrap();
    let first_type = expect_string_value(context.execute(&typeof_function).unwrap());
    let repeated_type = expect_string_value(context.execute(&typeof_function).unwrap());
    let atoms_after_typeof = runtime.test_atom_count();
    let string_key = runtime.intern_property_key("string").unwrap();
    let canonical_type = runtime.property_key_to_js_string(&string_key).unwrap();
    assert_eq!(first_type, JsString::from_static("string"));
    assert!(!first_type.is_wide());
    assert!(first_type.same_representation(&repeated_type));
    assert!(first_type.same_representation(&canonical_type));
    assert_eq!(runtime.test_atom_count(), atoms_after_typeof);

    // Publication is independent from execution. QuickJS accepts OP_plus in
    // the bytecode and raises TypeError only when it executes on a BigInt.
    let plus_bigint =
        quickjs_scalar_with_code(&direct_code(&[0xb0, 0x01, 0x00, 0x00, 0x00], &[PLUS, NEG]));
    let function = context.read_trusted_scalar_script(&plus_bigint).unwrap();
    for _ in 0..2 {
        assert_eq!(context.execute(&function), Err(RuntimeError::Exception));
        let (name, message) = take_error_name_and_message(&runtime, &mut context);
        assert_eq!(name, JsString::from_static("TypeError"));
        assert_eq!(
            message,
            JsString::from_static("bigint argument with unary +")
        );
        assert!(!context.has_exception());
    }
}

#[test]
fn trusted_quickjs_scalar_script_accepts_compatible_wire_spellings() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (operand, expected) in [(0x29, 41), (0xff, -1)] {
        let mut scalar = QUICKJS_SCALAR_42_BC5.to_vec();
        scalar[22] = operand;
        let function = context.read_trusted_scalar_script(&scalar).unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::Int(expected));
    }

    let mut trailing = QUICKJS_SCALAR_42_BC5.to_vec();
    trailing.extend_from_slice(&[0xde, 0xad]);
    let function = context.read_trusted_scalar_script(&trailing).unwrap();
    assert_eq!(context.execute(&function).unwrap(), Value::Int(42));

    let mut non_minimal_header = vec![0x05, 0x80, 0x00];
    non_minimal_header.extend_from_slice(&QUICKJS_SCALAR_42_BC5[2..]);
    let function = context
        .read_trusted_scalar_script(&non_minimal_header)
        .unwrap();
    assert_eq!(context.execute(&function).unwrap(), Value::Int(42));
}

#[test]
fn trusted_quickjs_scalar_script_preserves_frontier_and_malformed_provenance() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let baseline_objects = runtime.heap_counts().object_nodes;
    let baseline_atoms = runtime.test_atom_count();

    // A valid BC5 Int32 data root is not a scalar Script. The public API must
    // preserve that implementation frontier instead of fabricating a parse
    // failure or publishing any heap bytecode.
    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(&[0x05, 0x00, 0x05, 0x54])
        .unwrap_err()
    else {
        panic!("valid but unadmitted BC5 root did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut unused_atom_slot = QUICKJS_SCALAR_42_BC5.to_vec();
    unused_atom_slot.splice(1..2, [0x01, 0x00]);
    let float_entry = quickjs_float_constant_entry(0.5_f64.to_bits());
    let other_float_entry = quickjs_float_constant_entry(1.5_f64.to_bits());
    let bigint_entry = quickjs_bigint_constant_entry(&[0x00, 0x00, 0x00, 0x80, 0x00]);
    let other_bigint_entry = quickjs_bigint_constant_entry(&[0x01]);
    let well_formed_unadmitted = [
        (
            "push_this scalar Script",
            quickjs_scalar_with_code(&[0x08, 0xcb, 0x28]),
        ),
        (
            "scalar Script with await outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x89, 0xcb, 0x28]),
        ),
        (
            "scalar Script with postfix decrement outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x8e, 0xcb, 0x28]),
        ),
        (
            "scalar Script with postfix increment outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x8f, 0xcb, 0x28]),
        ),
        (
            "scalar Script with delete outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x96, 0xcb, 0x28]),
        ),
        (
            "scalar Script with an unused input atom slot",
            unused_atom_slot,
        ),
        (
            "push_const8 scalar Script with an empty constant pool",
            quickjs_scalar_with_code(&[0xbd, 0x00, 0xcb, 0x28]),
        ),
        (
            "direct scalar Script with a Float64 constant",
            quickjs_scalar_with_constants(&[0xb3, 0xcb, 0x28], &[float_entry.as_slice()]),
        ),
        (
            "push_const8 scalar Script with a nonzero index",
            quickjs_scalar_with_constants(&[0xbd, 0x01, 0xcb, 0x28], &[float_entry.as_slice()]),
        ),
        (
            "push_const8 scalar Script with an extra constant",
            quickjs_scalar_with_constants(
                &[0xbd, 0x00, 0xcb, 0x28],
                &[float_entry.as_slice(), other_float_entry.as_slice()],
            ),
        ),
        (
            "push_const8 scalar Script with an Int32 constant",
            quickjs_scalar_with_constants(&[0xbd, 0x00, 0xcb, 0x28], &[&[0x05, 0x54]]),
        ),
        (
            "direct scalar Script with a BigInt constant",
            quickjs_scalar_with_constants(&[0xb3, 0xcb, 0x28], &[bigint_entry.as_slice()]),
        ),
        (
            "push_const8 BigInt scalar Script with a nonzero index",
            quickjs_scalar_with_constants(&[0xbd, 0x01, 0xcb, 0x28], &[bigint_entry.as_slice()]),
        ),
        (
            "push_const8 BigInt scalar Script with an extra constant",
            quickjs_scalar_with_constants(
                &[0xbd, 0x00, 0xcb, 0x28],
                &[bigint_entry.as_slice(), other_bigint_entry.as_slice()],
            ),
        ),
        (
            "negated BigInt constant scalar Script with a nonzero index",
            quickjs_scalar_with_constants(
                &[0xbd, 0x01, 0x8a, 0xcb, 0x28],
                &[bigint_entry.as_slice()],
            ),
        ),
        (
            "negated BigInt constant scalar Script with an extra constant",
            quickjs_scalar_with_constants(
                &[0xbd, 0x00, 0x8a, 0xcb, 0x28],
                &[bigint_entry.as_slice(), other_bigint_entry.as_slice()],
            ),
        ),
        (
            "negated direct BigInt scalar Script with a constant",
            quickjs_scalar_with_constants(
                &[0xb0, 0x01, 0x00, 0x00, 0x00, 0x8a, 0xcb, 0x28],
                &[bigint_entry.as_slice()],
            ),
        ),
    ];
    for (label, image) in well_formed_unadmitted {
        let RuntimeError::Engine(error) = context.read_trusted_scalar_script(&image).unwrap_err()
        else {
            panic!("valid {label} did not preserve the admission frontier");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    let mut wrapping_scope_link = QUICKJS_SCALAR_42_BC5.to_vec();
    wrapping_scope_link.splice(18..19, [0x80, 0x80, 0x80, 0x80, 0x08]);
    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(&wrapping_scope_link)
        .unwrap_err()
    else {
        panic!("compatible wrapping scope link did not preserve the admission frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(QUICKJS_SELF_CONTAINED_MODULE_BC5)
        .unwrap_err()
    else {
        panic!("valid Module root did not preserve the admission frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut wrapping_module_field = QUICKJS_SELF_CONTAINED_MODULE_BC5.to_vec();
    wrapping_module_field.splice(58..59, [0x80, 0x80, 0x80, 0x80, 0x08]);
    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(&wrapping_module_field)
        .unwrap_err()
    else {
        panic!("compatible wrapping Module field did not preserve the admission frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut truncated = QUICKJS_SCALAR_42_BC5.to_vec();
    truncated.pop();
    assert_eq!(
        context.read_trusted_scalar_script(&truncated),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("read after the end of the buffer"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut wrong_version = QUICKJS_SCALAR_42_BC5.to_vec();
    wrong_version[0] = 4;
    assert_eq!(
        context.read_trusted_scalar_script(&wrong_version),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("invalid version (4 expected=5)"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    assert_eq!(
        context.read_trusted_scalar_script(&[0x05, 0x80, 0x80, 0x80, 0x80, 0x80]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("read after the end of the buffer"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut invalid_atom = QUICKJS_SCALAR_42_BC5.to_vec();
    invalid_atom.splice(6..8, [0xe6, 0x03]);
    assert_eq!(
        context.read_trusted_scalar_script(&invalid_atom),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("invalid atom index (pos=8)"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    assert_eq!(
        context.read_trusted_scalar_script(&[0x05, 0x01, 0x80, 0x80, 0x80, 0x80, 0x08]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("InternalError"),
            JsString::from_static("string too long"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut negative_bytecode_length = QUICKJS_SCALAR_42_BC5.to_vec();
    negative_bytecode_length.splice(15..16, [0x80, 0x80, 0x80, 0x80, 0x08]);
    assert_eq!(
        context.read_trusted_scalar_script(&negative_bytecode_length),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("InternalError"),
            JsString::from_static("out of memory"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut negative_module_count = QUICKJS_SELF_CONTAINED_MODULE_BC5.to_vec();
    negative_module_count.splice(55..56, [0x80, 0x80, 0x80, 0x80, 0x08]);
    assert_eq!(
        context.read_trusted_scalar_script(&negative_module_count),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("InternalError"),
            JsString::from_static("out of memory"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    assert_eq!(
        context.read_trusted_scalar_script(&[0x05, 0x00, 0x13, 0x00]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("invalid object reference (0 >= 0)"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
}

#[test]
fn trusted_quickjs_scalar_script_preserves_pinned_data_reader_errors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let cases: [(&[u8], &str, &str); 10] = [
        (
            &[
                0x05, 0x00, 0x12, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00,
                0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
            ],
            "TypeError",
            "cannot convert to object",
        ),
        (
            &[
                0x05, 0x00, 0x11, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00,
                0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
            ],
            "TypeError",
            "Number tag expected for date",
        ),
        (
            &[
                0x05, 0x00, 0x0e, 0x02, 0x01, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01,
                0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb,
                0x28,
            ],
            "TypeError",
            "ArrayBuffer object expected",
        ),
        (
            &[0x05, 0x00, 0x0f, 0x01, 0x00],
            "TypeError",
            "invalid array buffer",
        ),
        (
            &[0x05, 0x00, 0x0e, 0xff],
            "TypeError",
            "invalid typed array",
        ),
        (
            &[0x05, 0x00, 0x0e, 0x02, 0x00, 0x00, 0x01],
            "TypeError",
            "ArrayBuffer object expected",
        ),
        (
            &[0x05, 0x00, 0x12, 0x01],
            "TypeError",
            "cannot convert to object",
        ),
        (
            &[0x05, 0x00, 0x11, 0x01],
            "TypeError",
            "Number tag expected for date",
        ),
        (
            &[0x05, 0x00, 0x0e, 0x04, 0x00, 0x01, 0x0f, 0x00, 0x00],
            "RangeError",
            "invalid offset",
        ),
        (
            &[0x05, 0x00, 0x0e, 0x02, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x00],
            "RangeError",
            "invalid length",
        ),
    ];

    for (object, expected_name, expected_message) in cases {
        assert_eq!(
            context.read_trusted_scalar_script(object),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_name_and_message(&runtime, &mut context),
            (
                JsString::try_from_utf8(expected_name).unwrap(),
                JsString::try_from_utf8(expected_message).unwrap(),
            )
        );
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
    }
}

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

#[test]
fn can_block_policy_is_runtime_wide_and_isolated() {
    let runtime = Runtime::new();
    let clone = runtime.clone();
    let context = runtime.new_context();
    let other = Runtime::new();

    assert!(!runtime.can_block());
    assert!(!clone.can_block());
    assert!(!context.runtime().can_block());
    assert!(!other.can_block());

    clone.set_can_block(true);
    assert!(runtime.can_block());
    assert!(clone.can_block());
    assert!(context.runtime().can_block());
    assert!(!other.can_block());

    context.runtime().set_can_block(false);
    assert!(!runtime.can_block());
    assert!(!clone.can_block());
    assert!(!context.runtime().can_block());
}

#[test]
fn native_error_message_preserves_raw_printf_and_js_new_string_boundaries() {
    let mut embedded_nul = NativeErrorMessage::new();
    embedded_nul.push_utf8("P");
    embedded_nul.push_bytes([0, b'T', b'A', b'I', b'L']);
    assert_eq!(
        embedded_nul
            .to_js_string()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        [u16::from(b'P')]
    );

    let mut invalid_run = NativeErrorMessage::new();
    invalid_run.push_c_string_bytes([0x80, b'A', 0, b'B']);
    assert_eq!(
        invalid_run
            .to_js_string()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        [0xfffd]
    );

    let mut surrogate = NativeErrorMessage::new();
    surrogate.push_c_string_bytes([0xed, 0xa0, 0x80, 0]);
    assert_eq!(
        surrogate
            .to_js_string()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        [0xd800]
    );
}

#[test]
fn native_error_sidecar_survives_atom_and_parser_materializers() {
    fn message_units(runtime: &Runtime, context: &mut super::Context, error: Value) -> Vec<u16> {
        let Value::Object(error) = error else {
            panic!("native Error materializer did not return an object");
        };
        let key = runtime.intern_property_key("message").unwrap();
        let Value::String(message) = context.get_property(&error, &key).unwrap() else {
            panic!("native Error message was not a String");
        };
        message.utf16_units().collect()
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let spelling = JsString::try_from_utf16(
        vec![u16::from(b'A'); 55]
            .into_iter()
            .chain([0xd83d, 0xde42]),
    )
    .unwrap();
    let key = runtime.intern_property_key_js_string(&spelling).unwrap();
    let atom_error = runtime
        .native_atom_error(ErrorKind::Reference, "'", &key, "' is not defined")
        .unwrap();
    assert_eq!(
        atom_error.message(),
        format!("'{}�' is not defined", "A".repeat(55))
    );
    let materialized = runtime
        .new_native_error_from_error(
            context.realm,
            NativeErrorKind::Reference,
            &atom_error.clone(),
        )
        .unwrap();
    assert_eq!(
        message_units(&runtime, &mut context, materialized),
        [
            vec![u16::from(b'\'')],
            vec![u16::from(b'A'); 55],
            vec![0xd83d],
            "' is not defined".encode_utf16().collect(),
        ]
        .concat()
    );

    let mut raw = NativeErrorMessage::new();
    raw.push_bytes([0xed, 0xa0, 0x80]);
    let syntax_error = Error::from_native_message(ErrorKind::Syntax, raw);
    let materialized = runtime
        .new_native_error_without_backtrace_from_error(
            context.realm,
            NativeErrorKind::Syntax,
            &syntax_error,
        )
        .unwrap();
    assert_eq!(
        message_units(&runtime, &mut context, materialized),
        vec![0xd800]
    );
}

#[test]
fn atom_named_vm_and_global_errors_use_the_runtime_atom_table() {
    fn expected(prefix: &str, suffix: &str) -> Vec<u16> {
        [
            prefix.encode_utf16().collect(),
            vec![u16::from(b'A'); 55],
            vec![0xd83d],
            suffix.encode_utf16().collect(),
        ]
        .concat()
    }

    fn global_get(
        runtime: &Runtime,
        context: &super::Context,
        name: &JsString,
        is_lexical: bool,
    ) -> crate::FunctionBytecodeRef {
        runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new_with_closure_variables(
                    vec![Instruction::GetVar(0), Instruction::Return],
                    vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
                    FunctionMetadata {
                        closure_count: 1,
                        max_stack: 1,
                        strict: true,
                        ..FunctionMetadata::default()
                    },
                    vec![ClosureVariable {
                        source: ClosureSource::Global,
                        name: ClosureVariableName::Constant(0),
                        is_lexical,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    }],
                ),
            )
            .unwrap()
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let name = JsString::try_from_utf16(
        vec![u16::from(b'A'); 55]
            .into_iter()
            .chain([0xd83d, 0xde42]),
    )
    .unwrap();

    let read_only = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::ThrowReadOnly(0)],
                vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    assert_eq!(context.execute(&read_only), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        expected("'", "' is read-only")
    );

    let missing = global_get(&runtime, &context, &name, false);
    assert_eq!(context.execute(&missing), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        expected("'", "' is not defined")
    );

    runtime
        .create_global_lexical_js_string_for_test(context.realm, &name, false, None)
        .unwrap();
    let tdz = global_get(&runtime, &context, &name, true);
    assert_eq!(context.execute(&tdz), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        expected("", " is not initialized")
    );
}

#[test]
fn dynamic_source_builder_latches_utf16_length_failure() {
    let mut exact = DynamicSourceBuilder::with_limit(3);
    assert_eq!(exact.push_str("a😀"), Ok(()));
    assert_eq!(exact.finish(), Ok(JsString::try_from_utf8("a😀").unwrap()));

    let high = JsString::try_from_utf16([0xd801]).unwrap();
    let low = JsString::try_from_utf16([0xdc00]).unwrap();
    let mut split_pair = DynamicSourceBuilder::with_limit(2);
    assert_eq!(split_pair.push_js_string(&high), Ok(()));
    assert_eq!(split_pair.push_js_string(&low), Ok(()));
    assert_eq!(
        split_pair
            .finish()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        vec![0xd801, 0xdc00]
    );

    let mut failed = DynamicSourceBuilder::with_limit(5);
    assert_eq!(failed.push_str("a😀"), Ok(()));

    assert_eq!(failed.push_str("abc"), Err(JsStringError::TooLong));

    assert_eq!(failed.push_str("b"), Err(JsStringError::TooLong));
    assert_eq!(failed.push_str(""), Err(JsStringError::TooLong));
    assert_eq!(failed.finish(), Err(JsStringError::TooLong));
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
    object: &crate::ObjectRef,
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
    object: &crate::ObjectRef,
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
    object: &crate::ObjectRef,
    key: &PropertyKey,
) -> Result<Value, RuntimeError> {
    match runtime.prepare_get_property(object, key)? {
        PropertyGetAction::Complete(value) => Ok(value),
        PropertyGetAction::Call { .. } => Err(RuntimeError::Invariant(
            "ordinary-property test helper unexpectedly reached a getter",
        )),
    }
}

#[test]
fn direct_eval_identity_is_realm_local_and_independent_of_the_global_property() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_eval = global_callable(&runtime, &mut first, "eval");
    let second_eval = global_callable(&runtime, &mut second, "eval");
    let first_value = Value::Object(first_eval.as_object().clone());
    let second_value = Value::Object(second_eval.as_object().clone());

    let mut first_host = RuntimeVmHost::empty_for_test(runtime.clone(), first.realm);
    assert!(VmHost::is_original_eval(&mut first_host, &first_value).unwrap());
    assert!(!VmHost::is_original_eval(&mut first_host, &second_value).unwrap());

    let mut second_host = RuntimeVmHost::empty_for_test(runtime.clone(), second.realm);
    assert!(VmHost::is_original_eval(&mut second_host, &second_value).unwrap());
    assert!(!VmHost::is_original_eval(&mut second_host, &first_value).unwrap());

    let foreign_runtime = Runtime::new();
    let mut foreign_context = foreign_runtime.new_context();
    let foreign_eval = global_callable(&foreign_runtime, &mut foreign_context, "eval");
    assert_eq!(
        runtime.is_original_eval(
            first.realm,
            &Value::Object(foreign_eval.as_object().clone())
        ),
        Err(RuntimeError::WrongRuntime("eval function"))
    );

    assert_eq!(
        second.eval("delete globalThis.eval").unwrap(),
        Value::Bool(true)
    );
    assert!(VmHost::is_original_eval(&mut second_host, &second_value).unwrap());
    second
        .eval("globalThis.eval = function replacement() { return 17; }")
        .unwrap();
    let replacement = global_callable(&runtime, &mut second, "eval");
    assert!(
        !VmHost::is_original_eval(
            &mut second_host,
            &Value::Object(replacement.as_object().clone())
        )
        .unwrap()
    );
    assert!(VmHost::is_original_eval(&mut second_host, &second_value).unwrap());
}

#[test]
fn string_direct_eval_materializes_exact_caller_cells_but_non_string_stays_lazy() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let environment = EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::Block,
                bindings: vec![EvalBinding {
                    name: JsString::from_static("localBinding"),
                    source: EvalBindingSource::Local(0),
                    is_lexical: true,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                    is_catch_parameter: false,
                }]
                .into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![
                    EvalBinding {
                        name: JsString::from_static("argumentBinding"),
                        source: EvalBindingSource::Argument(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    },
                    EvalBinding {
                        name: JsString::from_static("<var>"),
                        source: EvalBindingSource::Local(1),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::EvalVariableObject,
                        is_catch_parameter: false,
                    },
                ]
                .into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: vec![EvalBinding {
                    name: JsString::from_static("outerBinding"),
                    source: EvalBindingSource::Closure(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                    is_catch_parameter: false,
                }]
                .into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::VariableObject {
            scope: 2,
            source: EvalBindingSource::Local(1),
        },
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    };
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(1),
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("outerBinding")))
                .unwrap(),
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            local_count: 2,
            eval_variable_object_local: Some(1),
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    )
    .with_variable_definitions(
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("argumentBinding"),
        ))],
        vec![
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("localBinding")), false),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<var>"))),
        ],
    )
    .with_eval_environments(vec![environment]);
    let parent = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("outerBinding"),
        ))],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let child = runtime.test_child_function_bytecode(&parent, 0).unwrap();
    let closure = runtime
        .new_var_ref(Value::Int(30), false, false, ClosureVariableKind::Normal)
        .unwrap();
    let eval_variable_object = runtime.new_object(None).unwrap();

    let mut non_string = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert_eq!(
        VmHost::direct_eval(
            &mut non_string,
            DirectEvalInvocation {
                input: Value::Int(42),
                environment: u16::MAX,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(!non_string.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(!non_string.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));

    let mut syntax_error = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert!(matches!(
        VmHost::direct_eval(
            &mut syntax_error,
            DirectEvalInvocation {
                input: Value::String(JsString::from_static(")")),
                environment: 0,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Throw(_)
    ));
    assert!(!syntax_error.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(!syntax_error.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));

    let mut declaration = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert_eq!(
        VmHost::direct_eval(
            &mut declaration,
            DirectEvalInvocation {
                input: Value::String(JsString::from_static("var evalVar = 1")),
                environment: 0,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Return(Value::Undefined),
    );
    assert!(declaration.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(declaration.eval_binding_is_captured_for_test(EvalBindingSource::Local(1)));
    assert!(declaration.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));

    let mut string = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert_eq!(
        VmHost::direct_eval(
            &mut string,
            DirectEvalInvocation {
                input: Value::String(JsString::from_static("40 + 2")),
                environment: 0,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Local(1)));
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Closure(0)));
    assert_eq!(runtime.read_var_ref(&closure).unwrap(), Value::Int(30));
}

#[test]
fn array_class_roots_length_layout_values_and_realm_prototype() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_prototype = first.array_prototype().unwrap();
    let second_prototype = second.array_prototype().unwrap();
    assert_ne!(first_prototype, second_prototype);
    assert!(runtime.is_array_object(&first_prototype).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&first_prototype).unwrap(),
        Some(first.object_prototype().unwrap())
    );

    let foreign_realm_value = second.new_object().unwrap();
    let array = first
        .new_array_from_values(vec![Value::Int(10), Value::Object(foreign_realm_value)])
        .unwrap();
    assert!(runtime.is_array_object(&array).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(first_prototype)
    );

    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        first.get_own_property(&array, &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: true,
            enumerable: false,
            configurable: false,
        })
    );
    let keys = runtime
        .own_property_keys(&array)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .0
                .state
                .borrow()
                .atoms
                .to_string(key.atom())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["0", "1", "length"]);
    let zero = runtime.intern_property_key("0").unwrap();
    assert_eq!(first.get_property(&array, &zero).unwrap(), Value::Int(10));
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(array.object_id()).unwrap();
        let ObjectPayload::Array { dense: Some(dense) } = &object.payload else {
            panic!("fresh Array did not retain its physical dense payload");
        };
        assert_eq!(dense.len(), 2);
        let shape = state.heap.shape(object.shape).unwrap();
        assert_eq!(shape.entries().len(), 1);
        assert!(
            state
                .atoms
                .array_index(shape.entries()[0].atom)
                .unwrap()
                .is_none()
        );
    }

    let other_runtime = Runtime::new();
    let mut other_context = other_runtime.new_context();
    let wrong_runtime_value = other_context.new_object().unwrap();
    assert!(matches!(
        first.new_array_from_values(vec![Value::Object(wrong_runtime_value)]),
        Err(RuntimeError::WrongRuntime("Array element"))
    ));
}

#[test]
fn array_dense_tail_delete_and_interior_delete_match_quickjs_conversion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array = context
        .new_array_from_values(vec![Value::Int(10), Value::Int(20), Value::Int(30)])
        .unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    let one = runtime.intern_property_key("1").unwrap();
    let two = runtime.intern_property_key("2").unwrap();

    assert!(runtime.delete_property(&array, &two).unwrap());
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(3)
    );
    assert!(!runtime.has_own_property(&array, &two).unwrap());
    assert_eq!(runtime.array_fast_len(&array).unwrap(), Some(2));

    assert!(context.set_property(&array, &two, Value::Int(31)).unwrap());
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(3)
    );
    assert_eq!(runtime.array_fast_len(&array).unwrap(), Some(3));

    assert!(runtime.delete_property(&array, &zero).unwrap());
    assert_eq!(runtime.array_fast_len(&array).unwrap(), None);
    assert!(!runtime.has_own_property(&array, &zero).unwrap());
    assert_eq!(context.get_property(&array, &one).unwrap(), Value::Int(20));
    assert_eq!(context.get_property(&array, &two).unwrap(), Value::Int(31));
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(3)
    );
}

#[test]
fn array_indices_grow_length_and_obey_readonly_and_extensible_state() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array = context.new_array().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let five = runtime.intern_property_key("5").unwrap();
    assert!(
        context
            .define_own_property(
                &array,
                &five,
                &data_descriptor(Value::Int(50), true, true, true)
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(6)
    );

    let non_index = runtime.intern_property_key("4294967295").unwrap();
    assert!(
        context
            .define_own_property(
                &array,
                &non_index,
                &data_descriptor(Value::Int(99), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(6)
    );

    assert!(
        context
            .define_own_property(
                &array,
                &length,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let six = runtime.intern_property_key("6").unwrap();
    assert!(
        !context
            .define_own_property(
                &array,
                &six,
                &data_descriptor(Value::Int(60), true, true, true)
            )
            .unwrap()
    );
    assert!(!context.set_property(&array, &six, Value::Int(60)).unwrap());
    assert!(context.set_property(&array, &five, Value::Int(51)).unwrap());
    assert_eq!(context.get_property(&array, &five).unwrap(), Value::Int(51));

    let extensible = context.new_array().unwrap();
    runtime.prevent_extensions(&extensible).unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    assert!(
        !context
            .define_own_property(
                &extensible,
                &zero,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&extensible, &length).unwrap(),
        Value::Int(0)
    );
}

#[test]
fn array_length_shrink_deletes_descending_and_rolls_back_at_fixed_index() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array = context
        .new_array_from_values((0..5).map(Value::Int).collect())
        .unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let fixed = runtime.intern_property_key("3").unwrap();
    assert!(
        context
            .define_own_property(
                &array,
                &fixed,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    assert!(
        !context
            .define_own_property(
                &array,
                &length,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.get_own_property(&array, &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(4),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );
    let four = runtime.intern_property_key("4").unwrap();
    assert!(!runtime.has_own_property(&array, &four).unwrap());
    assert!(runtime.has_own_property(&array, &fixed).unwrap());
    let two = runtime.intern_property_key("2").unwrap();
    assert!(runtime.has_own_property(&array, &two).unwrap());
    assert!(!context.set_property(&array, &four, Value::Int(44)).unwrap());
    assert!(context.set_property(&array, &two, Value::Int(22)).unwrap());
}

#[test]
fn array_assignment_rejections_keep_quickjs_length_diagnostics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let length = runtime.intern_property_key("length").unwrap();

    let read_only = context.new_array().unwrap();
    assert!(
        context
            .define_own_property(
                &read_only,
                &length,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let read_only_key = runtime.intern_property_key("readOnlyArray").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &read_only_key,
                &data_descriptor(Value::Object(read_only), true, true, true),
            )
            .unwrap()
    );
    for source in [
        "(function(){'use strict';readOnlyArray[0]=1})()",
        "(function(){'use strict';readOnlyArray.length=0})()",
    ] {
        assert_eq!(context.eval(source), Err(RuntimeError::Exception));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from_static("'length' is read-only")
        );
    }

    let fixed = context.new_array_from_values(vec![Value::Int(1)]).unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    assert!(
        context
            .define_own_property(
                &fixed,
                &zero,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let fixed_key = runtime.intern_property_key("fixedArray").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &fixed_key,
                &data_descriptor(Value::Object(fixed), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("(function(){'use strict';fixedArray.length=0})()"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not configurable")
    );
}

#[test]
fn array_join_separator_overflow_still_gets_nullish_slots_and_later_throw_wins() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(source) = context
        .eval(
            r#"(function(){
                var source=Object();globalThis.joinOverflowLog="";source.length=4;
                source[0]="a";
                source.__defineGetter__("1",function(){joinOverflowLog+="1";return null});
                source.__defineGetter__("2",function(){joinOverflowLog+="2";return undefined});
                source.__defineGetter__("3",function(){joinOverflowLog+="3";throw 77});
                return source;
            })()"#,
        )
        .unwrap()
    else {
        panic!("Array.join overflow fixture was not an object");
    };
    let completion = runtime
        .call_array_prototype_join_with_string_limit(
            context.realm,
            ArrayJoinKind::Join,
            super::NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &super::NativeArguments {
                actual_arg_count: 1,
                readable: vec![Value::String(JsString::from_static("xx"))],
            },
            2,
        )
        .unwrap();
    assert!(matches!(completion, Completion::Throw(Value::Int(77))));
    assert_eq!(
        context.eval("joinOverflowLog").unwrap(),
        Value::String(JsString::from_static("123"))
    );
}

#[test]
fn array_locale_separator_overflow_invokes_method_but_skips_result_to_string() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(source) = context
        .eval(
            r#"(function(){
                var source=Object(),element=Object();globalThis.localeOverflowLog="";
                source.length=2;source[0]="aa";source[1]=element;
                element.toLocaleString=function(){
                    var result=Object();localeOverflowLog+="M";
                    result.toString=function(){localeOverflowLog+="C";return "result"};
                    return result;
                };
                return source;
            })()"#,
        )
        .unwrap()
    else {
        panic!("Array.toLocaleString overflow fixture was not an object");
    };
    let error = runtime
        .call_array_prototype_join_with_string_limit(
            context.realm,
            ArrayJoinKind::ToLocaleString,
            super::NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &super::NativeArguments {
                actual_arg_count: 0,
                readable: Vec::new(),
            },
            2,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::Engine(ref error)
            if error.kind() == ErrorKind::JsInternal
                && error.message() == "string too long"
    ));
    assert_eq!(
        context.eval("localeOverflowLog").unwrap(),
        Value::String(JsString::from_static("M"))
    );
}

#[test]
fn array_locale_method_throw_replaces_pending_separator_overflow() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(source) = context
        .eval(
            r#"(function(){
                var source=Object(),element=Object();source.length=2;
                source[0]="aa";source[1]=element;
                element.toLocaleString=function(){throw 88};
                return source;
            })()"#,
        )
        .unwrap()
    else {
        panic!("Array.toLocaleString throwing overflow fixture was not an object");
    };
    let completion = runtime
        .call_array_prototype_join_with_string_limit(
            context.realm,
            ArrayJoinKind::ToLocaleString,
            super::NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &super::NativeArguments {
                actual_arg_count: 0,
                readable: Vec::new(),
            },
            2,
        )
        .unwrap();
    assert!(matches!(completion, Completion::Throw(Value::Int(88))));
}

#[test]
fn array_length_uses_quickjs_double_conversion_and_caller_realm_errors() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut caller = runtime.new_context();
    let array = first.new_array_from_values(vec![Value::Int(1)]).unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let value = caller
        .eval("(function(){function V(){this.count=0}V.prototype.valueOf=function(){this.count++;return this.count};return new V})()")
        .unwrap();
    assert_eq!(
        caller.define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value.clone()),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut caller),
        JsString::from_static("invalid array length")
    );
    let Value::Object(value) = value else {
        panic!("length conversion fixture was not an object");
    };
    let count = runtime.intern_property_key("count").unwrap();
    assert_eq!(caller.get_property(&value, &count).unwrap(), Value::Int(2));
    assert_eq!(caller.get_property(&array, &length).unwrap(), Value::Int(1));

    assert_eq!(
        caller.define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Float(1.5)),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("invalid Array length did not materialize a RangeError");
    };
    let expected = runtime
        .0
        .state
        .borrow()
        .heap
        .context(caller.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Range.index()]
    .unwrap();
    assert_eq!(
        runtime
            .get_prototype_of(&error)
            .unwrap()
            .unwrap()
            .object_id(),
        expected
    );

    let throwing = caller
        .eval("(function(){function V(){}V.prototype.valueOf=function(){throw 77};return new V})()")
        .unwrap();
    assert_eq!(
        caller.define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(throwing),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(caller.take_exception().unwrap(), Some(Value::Int(77)));
}

#[test]
fn array_slots_and_realm_roots_survive_gc_then_collect() {
    let runtime = Runtime::new();
    let baseline_atoms = runtime.test_atom_count();
    let (array, zero, one) = {
        let mut context = runtime.new_context();
        let element = context.new_object().unwrap();
        let symbol = runtime
            .new_symbol(Some(JsString::from_static("dense")))
            .unwrap();
        let array = context
            .new_array_from_values(vec![Value::Object(element.clone()), Value::Symbol(symbol)])
            .unwrap();
        let zero = runtime.intern_property_key("0").unwrap();
        let one = runtime.intern_property_key("1").unwrap();
        assert!(
            context
                .define_own_property(
                    &array,
                    &zero,
                    &OrdinaryPropertyDescriptor {
                        enumerable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(runtime.array_fast_len(&array).unwrap(), None);
        (array, zero, one)
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    assert!(matches!(
        get_property(&runtime, &array, &zero).unwrap(),
        Value::Object(_)
    ));
    assert!(matches!(
        get_property(&runtime, &array, &one).unwrap(),
        Value::Symbol(_)
    ));
    drop(array);
    drop(zero);
    drop(one);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn array_dense_self_cycle_is_visible_to_cycle_collection() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        let array = context.new_array().unwrap();
        let zero = runtime.intern_property_key("0").unwrap();
        assert!(
            context
                .set_property(&array, &zero, Value::Object(array.clone()))
                .unwrap()
        );
    }
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[cfg(feature = "test262-host")]
#[test]
fn test262_gc_host_is_quickjs_shaped_reentrant_and_preserves_active_roots() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let gc = context.new_test262_gc_function().unwrap();

    assert_eq!(
        NativeFunctionId::Test262Gc.descriptor().cproto,
        NativeCProto::Generic
    );
    assert!(!runtime.is_constructor(gc.as_object()).unwrap());
    assert_eq!(runtime.callable_realm(&gc).unwrap(), context.realm);
    assert_eq!(
        runtime.get_prototype_of(gc.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(matches!(
        runtime.get_own_property(gc.as_object(), &name).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("gc")
    ));
    assert!(matches!(
        runtime.get_own_property(gc.as_object(), &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(0),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));

    runtime.run_gc().unwrap();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let cycle = context.new_object().unwrap();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(
        context
            .define_own_property(
                &cycle,
                &self_key,
                &data_descriptor(Value::Object(cycle.clone()), true, true, true),
            )
            .unwrap()
    );
    drop(self_key);
    drop(cycle);
    let live_argument = context.new_object().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    assert_eq!(
        context
            .call(
                &gc,
                Value::Object(live_argument.clone()),
                &[Value::Object(live_argument.clone()), Value::Int(42)],
            )
            .unwrap(),
        Value::Undefined
    );
    assert_eq!(
        runtime.heap_counts().object_nodes,
        baseline_objects + 1,
        "the native host hook did not run cycle collection"
    );

    let global = context.global_object().unwrap();
    let host_gc = runtime.intern_property_key("hostGc").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &host_gc,
                &data_descriptor(Value::Object(gc.as_object().clone()), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context
            .eval(
                r#"
                (function activeFrame() {
                    var local = { answer: 42 };
                    activeFrame = null;
                    if (hostGc.call(local, local, "ignored") !== undefined) {
                        throw new Error("gc did not return undefined");
                    }
                    return local.answer;
                })()
                "#,
            )
            .unwrap(),
        Value::Int(42),
        "reentrant collection released an active frame or local root"
    );

    assert_eq!(context.construct(&gc, &[]), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("gc is not a constructor")
    );
}

#[test]
fn array_of_uses_set_for_a_custom_result_length() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context.new_object().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let setter = eval_callable(
        &runtime,
        &mut context,
        "(function(value){ this.seen = value; })",
    );
    assert!(
        context
            .define_own_property(
                &result,
                &length,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let global = context.global_object().unwrap();
    let result_key = runtime.intern_property_key("arrayOfResult").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &result_key,
                &data_descriptor(Value::Object(result.clone()), true, true, true),
            )
            .unwrap()
    );
    let constructor = eval_callable(
        &runtime,
        &mut context,
        "(function CustomArray(){ return arrayOfResult; })",
    );
    let array = global_callable(&runtime, &mut context, "Array");
    let of = property_callable(&runtime, &mut context, array.as_object(), "of");
    assert_eq!(
        context
            .call(
                &of,
                Value::Object(constructor.as_object().clone()),
                &[Value::Int(11), Value::Int(22)],
            )
            .unwrap(),
        Value::Object(result.clone())
    );

    for (name, expected) in [("0", 11), ("1", 22), ("seen", 2)] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(
            context.get_property(&result, &key).unwrap(),
            Value::Int(expected)
        );
    }
    assert!(matches!(
        context.get_own_property(&result, &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor { .. })
    ));
}

#[test]
fn array_of_create_data_property_reports_the_quickjs_rejection() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let array = global_callable(&runtime, &mut context, "Array");
    let of = property_callable(&runtime, &mut context, array.as_object(), "of");

    let blocked = context.new_object().unwrap();
    runtime.prevent_extensions(&blocked).unwrap();
    let blocked_key = runtime.intern_property_key("blockedArrayResult").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &blocked_key,
                &data_descriptor(Value::Object(blocked), true, true, true),
            )
            .unwrap()
    );
    let blocked_constructor = eval_callable(
        &runtime,
        &mut context,
        "(function BlockedArray(){ return blockedArrayResult; })",
    );
    assert_eq!(
        context.call(
            &of,
            Value::Object(blocked_constructor.as_object().clone()),
            &[Value::Int(1)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("object is not extensible")
    );

    let frozen = context.new_array().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(
        context
            .define_own_property(
                &frozen,
                &length,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let frozen_key = runtime.intern_property_key("frozenArrayResult").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &frozen_key,
                &data_descriptor(Value::Object(frozen), true, true, true),
            )
            .unwrap()
    );
    let frozen_constructor = eval_callable(
        &runtime,
        &mut context,
        "(function FrozenArray(){ return frozenArrayResult; })",
    );
    assert_eq!(
        context.call(
            &of,
            Value::Object(frozen_constructor.as_object().clone()),
            &[Value::Int(1)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("'length' is read-only")
    );
}

#[test]
fn array_constructor_sets_through_inherited_indices_like_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.array_prototype().unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    let setter = eval_callable(
        &runtime,
        &mut context,
        "(function(value){ this.hit = value; })",
    );
    assert!(
        context
            .define_own_property(
                &prototype,
                &zero,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let constructor = global_callable(&runtime, &mut context, "Array");
    let Value::Object(array) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("x"))],
        )
        .unwrap()
    else {
        panic!("Array call did not return an object");
    };
    let hit = runtime.intern_property_key("hit").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        context.get_property(&array, &hit).unwrap(),
        Value::String(JsString::from_static("x"))
    );
    assert!(!runtime.has_own_property(&array, &zero).unwrap());
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(0)
    );

    let one = runtime.intern_property_key("1").unwrap();
    assert!(
        context
            .define_own_property(
                &prototype,
                &one,
                &data_descriptor(Value::Int(1), false, false, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[Value::Int(10), Value::Int(20)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("'1' is read-only")
    );
}

fn global_callable(runtime: &Runtime, context: &mut super::Context, name: &str) -> CallableRef {
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

fn eval_callable(runtime: &Runtime, context: &mut super::Context, source: &str) -> CallableRef {
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
    context: &mut super::Context,
    object: &crate::ObjectRef,
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

fn own_key_names(runtime: &Runtime, object: &crate::ObjectRef) -> Vec<String> {
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

fn own_data_value(runtime: &Runtime, object: &crate::ObjectRef, name: &str) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data { value, .. }) =
        runtime.get_own_property(object, &key).unwrap()
    else {
        panic!("{name} was not an own data property");
    };
    value
}

fn own_stack_string(runtime: &Runtime, object: &crate::ObjectRef) -> JsString {
    let Value::String(stack) = own_data_value(runtime, object, "stack") else {
        panic!("stack was not a string");
    };
    stack
}

fn take_error_message(runtime: &Runtime, context: &mut super::Context) -> JsString {
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
    context: &mut super::Context,
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
    context: &super::Context,
    code: Vec<Instruction>,
    metadata: FunctionMetadata,
) -> CallableRef {
    let function = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(code, Vec::new(), metadata),
        )
        .unwrap();
    runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap()
}

fn push_named_script_active_frame(
    runtime: &Runtime,
    context: &mut super::Context,
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
    context: &super::Context,
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
    super::bytecode_publish::verify_unlinked_eval_tree(&function, kind, false, &[], false, false)
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
    context: &super::Context,
    filename: &str,
) -> super::ActiveFrameGuard {
    let module =
        compile_unlinked_module_with_filename("", filename, runtime.debug_info_mode()).unwrap();
    super::bytecode_publish::verify_unlinked_module_tree(&module).unwrap();
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

#[test]
fn contexts_share_runtime_but_have_distinct_identity() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let second = runtime.new_context();
    assert_ne!(first.id(), second.id());
    assert!(first.runtime().is_same_runtime(second.runtime()));
    let first_prototype = first.object_prototype().unwrap();
    let second_prototype = second.object_prototype().unwrap();
    assert_ne!(first_prototype, second_prototype);
    let function_prototype = first.function_prototype().unwrap();
    let global_object = first.global_object().unwrap();
    let global_var_object = first.global_var_object().unwrap();
    assert_eq!(
        runtime.get_prototype_of(&function_prototype).unwrap(),
        Some(first_prototype.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&global_object).unwrap(),
        Some(first_prototype.clone())
    );
    assert_eq!(runtime.get_prototype_of(&global_var_object).unwrap(), None);
    assert!(runtime.set_prototype_of(&global_var_object, None).unwrap());
    assert!(
        !runtime
            .set_prototype_of(&global_var_object, Some(&first_prototype))
            .unwrap()
    );
    let object = first.new_object().unwrap();
    assert_eq!(
        runtime.get_prototype_of(&object).unwrap(),
        Some(first_prototype.clone())
    );
    assert!(runtime.set_prototype_of(&first_prototype, None).unwrap());
    assert!(
        !runtime
            .set_prototype_of(&first_prototype, Some(&object))
            .unwrap()
    );
}

#[test]
fn number_intrinsic_graph_payload_constants_and_aliases_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let constructor = global_callable(&runtime, &mut context, "Number");
    let prototype = context.number_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        [
            "length",
            "name",
            "parseInt",
            "parseFloat",
            "isNaN",
            "isFinite",
            "isInteger",
            "isSafeInteger",
            "MAX_VALUE",
            "MIN_VALUE",
            "NaN",
            "NEGATIVE_INFINITY",
            "POSITIVE_INFINITY",
            "EPSILON",
            "MAX_SAFE_INTEGER",
            "MIN_SAFE_INTEGER",
            "prototype",
        ]
    );
    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "toExponential",
            "toFixed",
            "toPrecision",
            "toString",
            "toLocaleString",
            "valueOf",
            "constructor",
        ]
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value))
            if value.to_bits() == 0.0_f64.to_bits()
    ));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(context.realm)
            .unwrap()
            .primitive_prototypes[PrimitiveKind::Number.index()],
        Some(prototype.object_id())
    );

    for name in ["parseInt", "parseFloat"] {
        let key = runtime.intern_property_key(name).unwrap();
        let global_value = context.get_property(&global, &key).unwrap();
        let static_value = context.get_property(constructor.as_object(), &key).unwrap();
        let (Value::Object(global_value), Value::Object(static_value)) =
            (global_value, static_value)
        else {
            panic!("Number.{name} alias was not an object");
        };
        assert_eq!(static_value, global_value, "Number.{name} identity");
    }

    for (name, expected) in [
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", f64::from_bits(1)),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
        ("EPSILON", f64::EPSILON),
        ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
        ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            matches!(
                runtime
                    .get_own_property(constructor.as_object(), &key)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value,
                    writable: false,
                    enumerable: false,
                    configurable: false,
                }) if value.same_value(&Value::Float(expected))
            ),
            "Number.{name}"
        );
    }

    assert_eq!(
        context.call(&constructor, Value::Undefined, &[]).unwrap(),
        Value::Int(0)
    );
    let explicit_undefined = context
        .call(&constructor, Value::Undefined, &[Value::Undefined])
        .unwrap();
    assert!(matches!(explicit_undefined, Value::Float(value) if value.is_nan()));
    let Value::Object(wrapper) = context
        .construct(&constructor, &[Value::Float(-0.0)])
        .unwrap()
    else {
        panic!("new Number did not return an object");
    };
    assert_eq!(runtime.get_prototype_of(&wrapper).unwrap(), Some(prototype));
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value))
            if value.to_bits() == (-0.0_f64).to_bits()
    ));
}

#[test]
fn boolean_intrinsic_graph_payload_and_brand_methods_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = global_callable(&runtime, &mut context, "Boolean");
    let prototype = context.boolean_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        ["length", "name", "prototype"]
    );
    assert_eq!(
        own_key_names(&runtime, &prototype),
        ["toString", "valueOf", "constructor"]
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Boolean(false))
    ));
    let bigint_prototype = context.bigint_prototype().unwrap();
    let symbol_prototype = context.symbol_prototype().unwrap();
    {
        let state = runtime.0.state.borrow();
        let slots = state
            .heap
            .context(context.realm)
            .unwrap()
            .primitive_prototypes;
        assert_eq!(
            slots[PrimitiveKind::Boolean.index()],
            Some(prototype.object_id())
        );
        assert_eq!(
            slots[PrimitiveKind::BigInt.index()],
            Some(bigint_prototype.object_id())
        );
        assert_eq!(
            slots[PrimitiveKind::Symbol.index()],
            Some(symbol_prototype.object_id())
        );
        assert!(
            slots[PrimitiveKind::String.index()].is_some(),
            "String exotic-core prototype slot was not initialized"
        );
    }

    assert_eq!(
        context.call(&constructor, Value::Undefined, &[]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .call(&constructor, Value::Undefined, &[Value::Int(1)])
            .unwrap(),
        Value::Bool(true)
    );
    let wrapper = context
        .construct(&constructor, &[Value::Bool(false)])
        .unwrap();
    let Value::Object(wrapper) = wrapper else {
        panic!("new Boolean did not return an object");
    };
    assert_eq!(runtime.own_property_keys(&wrapper).unwrap(), []);
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(prototype.clone())
    );
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper.clone()), &[])
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(wrapper.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("false"))
    );
    assert_eq!(
        context.call(&value_of, Value::Bool(true), &[]).unwrap(),
        Value::Bool(true)
    );
    let spoof = runtime.new_object(Some(&prototype)).unwrap();
    assert!(matches!(
        context.call(&value_of, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not a boolean")
    );
    assert_eq!(
        context
            .eval("true.toString() + '|' + false.valueOf()")
            .unwrap(),
        Value::String(JsString::from_static("true|false"))
    );
    assert_eq!(context.eval("+new Boolean(false)").unwrap(), Value::Int(0));
}

#[test]
fn string_wrapper_exotic_indices_length_define_delete_and_order_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let string_prototype_id = runtime
        .0
        .state
        .borrow()
        .heap
        .context(context.realm)
        .unwrap()
        .primitive_prototypes[PrimitiveKind::String.index()]
    .expect("String exotic-core prototype slot was absent");
    let string_prototype =
        crate::ObjectRef::from_borrowed_handle(runtime.clone(), string_prototype_id).unwrap();

    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(string_prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::String(value)) if value.is_empty()
    ));
    assert_eq!(
        own_key_names(&runtime, &string_prototype),
        [
            "length",
            "at",
            "charCodeAt",
            "charAt",
            "concat",
            "codePointAt",
            "isWellFormed",
            "toWellFormed",
            "indexOf",
            "lastIndexOf",
            "includes",
            "endsWith",
            "startsWith",
            "match",
            "matchAll",
            "search",
            "split",
            "substring",
            "substr",
            "slice",
            "repeat",
            "replace",
            "replaceAll",
            "padEnd",
            "padStart",
            "trim",
            "trimEnd",
            "trimRight",
            "trimStart",
            "trimLeft",
            "toString",
            "valueOf",
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
            "constructor",
            "normalize",
            "localeCompare",
            "Symbol.iterator",
        ],
        "implemented-key filtered order, not the complete String prototype table"
    );
    let length_key = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        runtime
            .get_own_property(&string_prototype, &length_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(0),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    );

    let payload = JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xd800]).unwrap();
    let wrapper = runtime
        .new_string_object(&string_prototype, payload.clone(), false)
        .unwrap();
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::String(value)) if value == &payload
    ));
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(string_prototype.clone())
    );
    assert_eq!(
        own_key_names(&runtime, &wrapper),
        ["0", "1", "2", "3", "length"]
    );
    assert_eq!(
        runtime.get_own_property(&wrapper, &length_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(4),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );

    for (index, unit) in [0x41, 0xd83d, 0xde00, 0xd800].into_iter().enumerate() {
        let key = runtime.intern_property_key(&index.to_string()).unwrap();
        let expected = Value::String(JsString::try_from_utf16([unit]).unwrap());
        assert_eq!(
            runtime.get_own_property(&wrapper, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: expected.clone(),
                writable: false,
                enumerable: true,
                configurable: false,
            })
        );
        assert!(runtime.has_own_property(&wrapper, &key).unwrap());
        assert!(
            runtime
                .define_own_property(
                    &wrapper,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(expected),
                        writable: DescriptorField::Present(false),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(!runtime.delete_property(&wrapper, &key).unwrap());
    }

    let zero = runtime.intern_property_key("0").unwrap();
    for descriptor in [
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::String(JsString::from_static("X"))),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            writable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            enumerable: DescriptorField::Present(false),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            get: DescriptorField::Present(AccessorValue::Undefined),
            ..OrdinaryPropertyDescriptor::new()
        },
    ] {
        assert!(
            !runtime
                .define_own_property(&wrapper, &zero, &descriptor)
                .unwrap()
        );
    }
    assert!(!runtime.delete_property(&wrapper, &length_key).unwrap());

    let eight = runtime.intern_property_key("8").unwrap();
    let foo = runtime.intern_property_key("foo").unwrap();
    let leading_zero = runtime.intern_property_key("01").unwrap();
    let symbol = PropertyKey::from(
        runtime
            .new_symbol(Some(JsString::from_static("tail")))
            .unwrap(),
    );
    for key in [&foo, &leading_zero, &eight, &symbol] {
        assert!(
            runtime
                .define_own_property(
                    &wrapper,
                    key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        writable: DescriptorField::Present(true),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    assert_eq!(
        own_key_names(&runtime, &wrapper),
        ["0", "1", "2", "3", "8", "length", "foo", "01", "tail"]
    );

    runtime.prevent_extensions(&wrapper).unwrap();
    assert!(
        runtime
            .define_own_property(&wrapper, &zero, &OrdinaryPropertyDescriptor::new(),)
            .unwrap()
    );
    let nine = runtime.intern_property_key("9").unwrap();
    assert!(
        !runtime
            .define_own_property(
                &wrapper,
                &nine,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let sloppy = eval_callable(&runtime, &mut context, "(function(){ return this; })");
    let Value::Object(escaped) = context
        .call(&sloppy, Value::String(payload.clone()), &[])
        .unwrap()
    else {
        panic!("sloppy String this did not escape as a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&escaped).unwrap(),
        Some(string_prototype)
    );
    assert_eq!(
        own_key_names(&runtime, &escaped),
        ["0", "1", "2", "3", "length"]
    );
}

#[test]
fn string_method_slice_matches_quickjs_table_and_code_unit_rules() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();

    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "length",
            "at",
            "charCodeAt",
            "charAt",
            "concat",
            "codePointAt",
            "isWellFormed",
            "toWellFormed",
            "indexOf",
            "lastIndexOf",
            "includes",
            "endsWith",
            "startsWith",
            "match",
            "matchAll",
            "search",
            "split",
            "substring",
            "substr",
            "slice",
            "repeat",
            "replace",
            "replaceAll",
            "padEnd",
            "padStart",
            "trim",
            "trimEnd",
            "trimRight",
            "trimStart",
            "trimLeft",
            "toString",
            "valueOf",
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
            "constructor",
            "normalize",
            "localeCompare",
            "Symbol.iterator",
        ],
        "implemented entries must retain their pinned QuickJS table order"
    );

    let methods = [
        ("at", "at", 1, NativeCProto::GenericMagic, 1),
        ("charCodeAt", "charCodeAt", 1, NativeCProto::Generic, 1),
        ("charAt", "charAt", 1, NativeCProto::GenericMagic, 1),
        ("concat", "concat", 1, NativeCProto::Generic, 0),
        ("codePointAt", "codePointAt", 1, NativeCProto::Generic, 1),
        ("isWellFormed", "isWellFormed", 0, NativeCProto::Generic, 0),
        ("toWellFormed", "toWellFormed", 0, NativeCProto::Generic, 0),
        ("substring", "substring", 2, NativeCProto::Generic, 2),
        ("substr", "substr", 2, NativeCProto::Generic, 2),
        ("slice", "slice", 2, NativeCProto::Generic, 2),
        ("padEnd", "padEnd", 1, NativeCProto::GenericMagic, 1),
        ("padStart", "padStart", 1, NativeCProto::GenericMagic, 1),
        ("trim", "trim", 0, NativeCProto::GenericMagic, 0),
        ("trimEnd", "trimEnd", 0, NativeCProto::GenericMagic, 0),
        ("trimRight", "trimEnd", 0, NativeCProto::GenericMagic, 0),
        ("trimStart", "trimStart", 0, NativeCProto::GenericMagic, 0),
        ("trimLeft", "trimStart", 0, NativeCProto::GenericMagic, 0),
        (
            "toLowerCase",
            "toLowerCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        (
            "toUpperCase",
            "toUpperCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        (
            "toLocaleLowerCase",
            "toLocaleLowerCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        (
            "toLocaleUpperCase",
            "toLocaleUpperCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        ("anchor", "anchor", 1, NativeCProto::GenericMagic, 1),
        ("big", "big", 0, NativeCProto::GenericMagic, 0),
        ("blink", "blink", 0, NativeCProto::GenericMagic, 0),
        ("bold", "bold", 0, NativeCProto::GenericMagic, 0),
        ("fixed", "fixed", 0, NativeCProto::GenericMagic, 0),
        ("fontcolor", "fontcolor", 1, NativeCProto::GenericMagic, 1),
        ("fontsize", "fontsize", 1, NativeCProto::GenericMagic, 1),
        ("italics", "italics", 0, NativeCProto::GenericMagic, 0),
        ("link", "link", 1, NativeCProto::GenericMagic, 1),
        ("small", "small", 0, NativeCProto::GenericMagic, 0),
        ("strike", "strike", 0, NativeCProto::GenericMagic, 0),
        ("sub", "sub", 0, NativeCProto::GenericMagic, 0),
        ("sup", "sup", 0, NativeCProto::GenericMagic, 0),
    ];
    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    for (name, function_name, length, cproto, min_readable_args) in methods {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&prototype, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(_),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));
        let method = property_callable(&runtime, &mut context, &prototype, name);
        assert!(!runtime.is_constructor(method.as_object()).unwrap());
        assert!(matches!(
            runtime
                .get_own_property(method.as_object(), &length_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == length
        ));
        assert!(matches!(
            runtime
                .get_own_property(method.as_object(), &name_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::try_from_utf8(function_name).unwrap()
        ));
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(method.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("String method was not a native function: {name}");
        };
        assert_eq!(data.target.descriptor().cproto, cproto);
        assert_eq!(data.min_readable_args, min_readable_args);
    }

    let payload = JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xd800, 0x5a]).unwrap();
    let at = property_callable(&runtime, &mut context, &prototype, "at");
    assert_eq!(
        context
            .call(&at, Value::String(payload.clone()), &[Value::Int(-1)])
            .unwrap(),
        Value::String(JsString::from_static("Z"))
    );
    assert_eq!(
        context
            .call(&at, Value::String(payload.clone()), &[Value::Int(1)])
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xd83d]).unwrap())
    );
    assert_eq!(
        context
            .call(&at, Value::String(payload.clone()), &[Value::Int(5)])
            .unwrap(),
        Value::Undefined
    );

    let char_at = property_callable(&runtime, &mut context, &prototype, "charAt");
    assert_eq!(
        context
            .call(&char_at, Value::String(payload.clone()), &[Value::Int(-1)],)
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    let char_code_at = property_callable(&runtime, &mut context, &prototype, "charCodeAt");
    assert_eq!(
        context
            .call(
                &char_code_at,
                Value::String(payload.clone()),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::Int(0xde00)
    );
    assert!(matches!(
        context
            .call(
                &char_code_at,
                Value::String(payload.clone()),
                &[Value::Int(5)],
            )
            .unwrap(),
        Value::Float(value) if value.is_nan()
    ));

    let code_point_at = property_callable(&runtime, &mut context, &prototype, "codePointAt");
    assert_eq!(
        context
            .call(
                &code_point_at,
                Value::String(payload.clone()),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::Int(0x1f600)
    );
    assert_eq!(
        context
            .call(
                &code_point_at,
                Value::String(payload.clone()),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::Int(0xde00)
    );

    let concat = property_callable(&runtime, &mut context, &prototype, "concat");
    assert_eq!(
        context
            .call(
                &concat,
                Value::String(JsString::from_static("R")),
                &[
                    Value::Undefined,
                    Value::Null,
                    Value::Bool(true),
                    Value::BigInt(JsBigInt::one()),
                ],
            )
            .unwrap(),
        Value::String(JsString::from_static("Rundefinednulltrue1"))
    );
    let mut near_limit = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
    for _ in 0..16 {
        near_limit = near_limit.try_concat(&near_limit).unwrap();
    }
    assert_eq!(near_limit.len(), 536_936_448);
    assert!(matches!(
        near_limit.try_concat(&near_limit),
        Err(crate::value::JsStringError::TooLong)
    ));

    let is_well_formed = property_callable(&runtime, &mut context, &prototype, "isWellFormed");
    let to_well_formed = property_callable(&runtime, &mut context, &prototype, "toWellFormed");
    assert_eq!(
        context
            .call(
                &is_well_formed,
                Value::String(payload.clone()),
                &[Value::Symbol(runtime.new_symbol(None).unwrap())],
            )
            .unwrap(),
        Value::Bool(false),
        "well-formed methods ignore all actual arguments"
    );
    assert_eq!(
        context
            .call(&to_well_formed, Value::String(payload), &[])
            .unwrap(),
        Value::String(JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xfffd, 0x5a]).unwrap(),)
    );

    assert_eq!(
        context.call(&at, Value::Null, &[Value::Int(0)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("null or undefined are forbidden")
    );
    assert_eq!(
        context.eval("'abc'.at(-1)").unwrap(),
        Value::String(JsString::from_static("c"))
    );
    assert_eq!(context.eval("'abc'.charCodeAt(1)").unwrap(), Value::Int(98));
}

#[test]
fn string_rope_vm_and_native_concat_overflow_use_the_defining_realms() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_string = first.string_prototype().unwrap();
    let concat = property_callable(&runtime, &mut first, &first_string, "concat");
    let internal_error = global_callable(&runtime, &mut first, "InternalError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(first_internal_error_prototype) = first
        .get_property(internal_error.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("InternalError.prototype was not an object");
    };

    let mut near_limit = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
    for _ in 0..16 {
        near_limit = near_limit.try_concat(&near_limit).unwrap();
    }
    assert_eq!(near_limit.len(), 536_936_448);
    assert!(!near_limit.is_flat());

    let global = first.global_object().unwrap();
    let near_key = runtime.intern_property_key("nearLimitString").unwrap();
    assert!(
        runtime
            .define_own_property(
                &global,
                &near_key,
                &data_descriptor(Value::String(near_limit.clone()), true, false, true),
            )
            .unwrap()
    );
    assert_eq!(
        first.eval("nearLimitString + nearLimitString"),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(vm_error)) = first.take_exception().unwrap() else {
        panic!("VM String overflow did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&vm_error).unwrap(),
        Some(first_internal_error_prototype.clone())
    );
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        first.get_property(&vm_error, &message).unwrap(),
        Value::String(JsString::from_static("string too long"))
    );

    assert_eq!(
        second.call(
            &concat,
            Value::String(JsString::from_static("")),
            &[Value::String(near_limit.clone()), Value::String(near_limit),],
        ),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(native_error)) = second.take_exception().unwrap() else {
        panic!("native String.concat overflow did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(first_internal_error_prototype)
    );
    assert_eq!(
        second.get_property(&native_error, &message).unwrap(),
        Value::String(JsString::from_static("string too long"))
    );
}

#[test]
fn string_conversion_core_brand_lookup_object_routes_and_overrides_match_quickjs_slice() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");

    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "length",
            "at",
            "charCodeAt",
            "charAt",
            "concat",
            "codePointAt",
            "isWellFormed",
            "toWellFormed",
            "indexOf",
            "lastIndexOf",
            "includes",
            "endsWith",
            "startsWith",
            "match",
            "matchAll",
            "search",
            "split",
            "substring",
            "substr",
            "slice",
            "repeat",
            "replace",
            "replaceAll",
            "padEnd",
            "padStart",
            "trim",
            "trimEnd",
            "trimRight",
            "trimStart",
            "trimLeft",
            "toString",
            "valueOf",
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
            "constructor",
            "normalize",
            "localeCompare",
            "Symbol.iterator",
        ],
        "implemented-key filtered order, not the complete String prototype table"
    );
    for name in ["toString", "valueOf"] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&prototype, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(_),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));
    }

    let payload = JsString::try_from_utf16([0x41, 0xd800, 0x42]).unwrap();
    let wrapper = runtime
        .new_string_object(&prototype, payload.clone(), false)
        .unwrap();
    for method in [&to_string, &value_of] {
        assert_eq!(
            context
                .call(method, Value::String(payload.clone()), &[Value::Int(99)])
                .unwrap(),
            Value::String(payload.clone())
        );
        assert_eq!(
            context
                .call(method, Value::Object(wrapper.clone()), &[])
                .unwrap(),
            Value::String(payload.clone())
        );
        assert_eq!(
            context
                .call(method, Value::Object(prototype.clone()), &[])
                .unwrap(),
            Value::String(JsString::from_static(""))
        );
    }

    let spoof = runtime.new_object(Some(&prototype)).unwrap();
    assert_eq!(
        context.call(&to_string, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("String brand failure did not publish an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("not a string"))
    );

    assert!(
        runtime
            .set_prototype_of(&wrapper, Some(&object_prototype))
            .unwrap()
    );
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper.clone()), &[])
            .unwrap(),
        Value::String(payload.clone())
    );

    let conversion_wrapper = runtime
        .new_string_object(&prototype, payload.clone(), false)
        .unwrap();
    let override_to_string =
        eval_callable(&runtime, &mut context, "(function(){ return 'override'; })");
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    assert!(
        runtime
            .define_own_property(
                &conversion_wrapper,
                &to_string_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(
                        override_to_string.as_object().clone(),
                    )),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        runtime
            .to_primitive(
                context.realm,
                Value::Object(conversion_wrapper.clone()),
                ToPrimitiveHint::String,
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("override")))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(conversion_wrapper), &[])
            .unwrap(),
        Value::String(payload.clone()),
        "saved brand method must ignore ordinary conversion overrides"
    );

    assert_eq!(
        context.eval("'source'.toString()").unwrap(),
        Value::String(JsString::from_static("source"))
    );
    assert_eq!(
        context.eval("'source'.valueOf()").unwrap(),
        Value::String(JsString::from_static("source"))
    );

    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    assert_eq!(
        context
            .call(&object_to_string, Value::String(payload.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object String]"))
    );
    assert_eq!(
        context
            .call(
                &object_to_locale_string,
                Value::String(payload.clone()),
                &[],
            )
            .unwrap(),
        Value::String(payload.clone())
    );
    let Value::Object(first_box) = context
        .call(&object_value_of, Value::String(payload.clone()), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box String");
    };
    let Value::Object(second_box) = context
        .call(&object_value_of, Value::String(payload), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box String");
    };
    assert_ne!(first_box, second_box);
    assert_eq!(
        runtime.get_prototype_of(&first_box).unwrap(),
        Some(prototype.clone())
    );

    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &tag,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from_static("Custom"))),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(
                &object_to_string,
                Value::String(JsString::from_static("x")),
                &[]
            )
            .unwrap(),
        Value::String(JsString::from_static("[object Custom]"))
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &tag,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(
                &object_to_string,
                Value::String(JsString::from_static("x")),
                &[]
            )
            .unwrap(),
        Value::String(JsString::from_static("[object String]"))
    );
    assert!(runtime.delete_property(&prototype, &tag).unwrap());
}

#[test]
fn symbol_intrinsic_graph_registry_brand_and_wrapper_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = global_callable(&runtime, &mut context, "Symbol");
    let prototype = context.symbol_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Ordinary
    ));
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        [
            "length",
            "name",
            "for",
            "keyFor",
            "toPrimitive",
            "iterator",
            "match",
            "matchAll",
            "replace",
            "search",
            "split",
            "toStringTag",
            "isConcatSpreadable",
            "hasInstance",
            "species",
            "unscopables",
            "asyncIterator",
            "prototype",
        ]
    );
    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "toString",
            "valueOf",
            "description",
            "constructor",
            "Symbol.toPrimitive",
            "Symbol.toStringTag",
        ]
    );

    let to_primitive_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    assert!(matches!(
        runtime
            .get_own_property(&prototype, &to_primitive_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(_),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    let description_key = runtime.intern_property_key("description").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(description_getter),
        set: None,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&prototype, &description_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Symbol.prototype.description was not a getter-only accessor");
    };
    assert_eq!(
        runtime
            .get_prototype_of(description_getter.as_object())
            .unwrap(),
        Some(context.function_prototype().unwrap())
    );

    let Value::Symbol(no_description) = context.call(&constructor, Value::Undefined, &[]).unwrap()
    else {
        panic!("Symbol() did not return a Symbol primitive");
    };
    let Value::Symbol(empty_description) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static(""))],
        )
        .unwrap()
    else {
        panic!("Symbol(\"\") did not return a Symbol primitive");
    };
    assert_ne!(no_description, empty_description);
    assert_eq!(runtime.symbol_description(&no_description).unwrap(), None);
    assert_eq!(
        runtime.symbol_description(&empty_description).unwrap(),
        Some(JsString::from_static(""))
    );
    let ignored_argument = context.new_object().unwrap();
    assert!(matches!(
        context.construct(&constructor, &[Value::Object(ignored_argument)]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("Symbol is not a constructor")
    );

    let symbol_for = property_callable(&runtime, &mut context, constructor.as_object(), "for");
    let key_for = property_callable(&runtime, &mut context, constructor.as_object(), "keyFor");
    let registry_key = Value::String(JsString::from_static("registry"));
    let first_registered = context
        .call(
            &symbol_for,
            Value::Undefined,
            std::slice::from_ref(&registry_key),
        )
        .unwrap();
    let second_registered = context
        .call(
            &symbol_for,
            Value::Null,
            std::slice::from_ref(&registry_key),
        )
        .unwrap();
    assert_eq!(first_registered, second_registered);
    assert_eq!(
        context
            .call(
                &key_for,
                Value::Undefined,
                std::slice::from_ref(&first_registered),
            )
            .unwrap(),
        registry_key
    );
    assert_eq!(
        context
            .call(
                &key_for,
                Value::Undefined,
                &[Value::Symbol(no_description.clone())],
            )
            .unwrap(),
        Value::Undefined
    );

    for (name, symbol) in [
        ("toPrimitive", WellKnownSymbol::ToPrimitive),
        ("iterator", WellKnownSymbol::Iterator),
        ("match", WellKnownSymbol::Match),
        ("matchAll", WellKnownSymbol::MatchAll),
        ("replace", WellKnownSymbol::Replace),
        ("search", WellKnownSymbol::Search),
        ("split", WellKnownSymbol::Split),
        ("toStringTag", WellKnownSymbol::ToStringTag),
        ("isConcatSpreadable", WellKnownSymbol::IsConcatSpreadable),
        ("hasInstance", WellKnownSymbol::HasInstance),
        ("species", WellKnownSymbol::Species),
        ("unscopables", WellKnownSymbol::Unscopables),
        ("asyncIterator", WellKnownSymbol::AsyncIterator),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime
                .get_own_property(constructor.as_object(), &key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Symbol(value),
                writable: false,
                enumerable: false,
                configurable: false,
            }) if value == runtime.well_known_symbol(symbol)
        ));
    }

    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    assert_eq!(
        context
            .call(&to_string, Value::Symbol(empty_description.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("Symbol()"))
    );
    assert_eq!(
        context
            .call(&value_of, Value::Symbol(no_description.clone()), &[],)
            .unwrap(),
        Value::Symbol(no_description.clone())
    );

    let object_prototype = context.object_prototype().unwrap();
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let Value::Object(wrapper) = context
        .call(&object_value_of, Value::Symbol(no_description.clone()), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box a Symbol primitive");
    };
    assert_eq!(runtime.get_prototype_of(&wrapper).unwrap(), Some(prototype));
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom))
            if atom == &no_description.atom()
    ));
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper), &[])
            .unwrap(),
        Value::Symbol(no_description)
    );
    assert_eq!(
        context.eval("Symbol('source').toString()").unwrap(),
        Value::String(JsString::from_static("Symbol(source)"))
    );
    assert_eq!(
        context.eval("Symbol('source').description").unwrap(),
        Value::String(JsString::from_static("source"))
    );
    assert_eq!(
        context.eval("Symbol().description").unwrap(),
        Value::Undefined
    );
}

#[test]
fn symbol_wrapper_owns_its_atom_and_realm_until_final_collection() {
    let runtime = Runtime::new();
    let (atom, wrapper) = {
        let mut context = runtime.new_context();
        let object_prototype = context.object_prototype().unwrap();
        let object_value_of =
            property_callable(&runtime, &mut context, &object_prototype, "valueOf");
        let symbol = runtime
            .new_symbol(Some(JsString::from_static("wrapper-only")))
            .unwrap();
        let atom = symbol.atom();
        let Value::Object(wrapper) = context
            .call(&object_value_of, Value::Symbol(symbol), &[])
            .unwrap()
        else {
            panic!("Object.prototype.valueOf did not return a Symbol wrapper");
        };
        (atom, wrapper)
    };

    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "the live wrapper must retain its Symbol prototype realm graph"
    );
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .atoms
            .resolve(atom)
            .unwrap()
            .ref_count,
        Some(1),
        "the wrapper must own the local symbol atom after the primitive root drops"
    );

    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
    assert!(
        runtime.0.state.borrow().atoms.resolve(atom).is_err(),
        "the final wrapper release must return its symbol atom ownership"
    );
}

#[test]
fn bigint_intrinsic_graph_conversion_truncation_and_wrappers_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = global_callable(&runtime, &mut context, "BigInt");
    let prototype = context.bigint_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Ordinary
    ));

    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let tag_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert_eq!(
        runtime.own_property_keys(&prototype).unwrap(),
        [
            to_string_key,
            value_of_key,
            constructor_key,
            tag_key.clone(),
        ]
    );
    assert!(matches!(
        runtime.get_own_property(&prototype, &tag_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("BigInt")
    ));
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        ["length", "name", "asUintN", "asIntN", "prototype"]
    );

    assert_eq!(
        context
            .call(&constructor, Value::Undefined, &[Value::Int(42)])
            .unwrap(),
        Value::BigInt(JsBigInt::from(42))
    );
    assert_eq!(
        context
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from_static("0x10000000000000000"))],
            )
            .unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("0x10000000000000000").unwrap())
    );
    assert!(matches!(
        context.construct(&constructor, &[Value::Int(1)]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("BigInt is not a constructor")
    );

    let as_uint_n = property_callable(&runtime, &mut context, constructor.as_object(), "asUintN");
    let as_int_n = property_callable(&runtime, &mut context, constructor.as_object(), "asIntN");
    assert_eq!(
        context
            .call(
                &as_uint_n,
                Value::Undefined,
                &[Value::Int(64), Value::BigInt(JsBigInt::from(-1))],
            )
            .unwrap(),
        Value::BigInt(JsBigInt::from(-1))
    );
    assert_eq!(
        context
            .call(
                &as_int_n,
                Value::Undefined,
                &[Value::Int(8), Value::BigInt(JsBigInt::from(255))],
            )
            .unwrap(),
        Value::BigInt(JsBigInt::from(-1))
    );

    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    assert_eq!(
        context
            .call(
                &to_string,
                Value::BigInt(JsBigInt::from(255)),
                &[Value::Int(16)],
            )
            .unwrap(),
        Value::String(JsString::from_static("ff"))
    );
    assert!(matches!(
        context.call(&value_of, Value::Object(prototype.clone()), &[]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not a BigInt")
    );

    let object_prototype = context.object_prototype().unwrap();
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let Value::Object(wrapper) = context
        .call(&object_value_of, Value::BigInt(JsBigInt::from(123)), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box a BigInt primitive");
    };
    assert_eq!(runtime.get_prototype_of(&wrapper).unwrap(), Some(prototype));
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value))
            if value == &JsBigInt::from(123)
    ));
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper), &[])
            .unwrap(),
        Value::BigInt(JsBigInt::from(123))
    );
}

#[test]
fn global_numeric_parsers_match_quickjs_graph_conversion_order_and_results() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let parse_int = global_callable(&runtime, &mut context, "parseInt");
    let parse_float = global_callable(&runtime, &mut context, "parseFloat");

    for (name, callable, length) in [("parseInt", &parse_int, 2), ("parseFloat", &parse_float, 1)] {
        assert_eq!(
            own_key_names(&runtime, callable.as_object()),
            ["length", "name"]
        );
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "length"),
            Value::Int(length)
        );
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "name"),
            Value::String(JsString::try_from_utf8(name).unwrap())
        );
        for property in ["length", "name"] {
            let property = runtime.intern_property_key(property).unwrap();
            assert!(matches!(
                runtime
                    .get_own_property(callable.as_object(), &property)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    ..
                })
            ));
        }
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));
    }

    assert_eq!(
        context
            .call(
                &parse_int,
                Value::Bool(true),
                &[Value::String(JsString::from_static("0x10"))],
            )
            .unwrap(),
        Value::Int(16)
    );
    assert_eq!(
        context
            .call(
                &parse_int,
                Value::Undefined,
                &[
                    Value::String(JsString::from_static("10")),
                    Value::Float(4_294_967_298.0),
                ],
            )
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context
            .call(
                &parse_float,
                Value::Undefined,
                &[Value::String(JsString::from_static(
                    "1.0000000000000001110223024625156540423631668090820313",
                ))],
            )
            .unwrap(),
        Value::Int(1)
    );
    let Value::Float(negative_zero) = context
        .call(
            &parse_int,
            Value::Undefined,
            &[Value::String(JsString::from_static("-0"))],
        )
        .unwrap()
    else {
        panic!("parseInt('-0') did not preserve the float tag");
    };
    assert_eq!(negative_zero.to_bits(), (-0.0_f64).to_bits());

    let log_key = runtime.intern_property_key("parseLog").unwrap();
    assert!(
        runtime
            .define_own_property(
                &global,
                &log_key,
                &data_descriptor(Value::String(JsString::from_static("")), true, true, true),
            )
            .unwrap()
    );
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    let input = context.new_object().unwrap();
    let input_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { parseLog = parseLog + 'input:' + hint + '|'; return '10'; })",
    );
    assert!(
        runtime
            .define_own_property(
                &input,
                &to_primitive,
                &data_descriptor(
                    Value::Object(input_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    let radix = context.new_object().unwrap();
    let radix_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { parseLog = parseLog + 'radix:' + hint + '|'; return 2; })",
    );
    assert!(
        runtime
            .define_own_property(
                &radix,
                &to_primitive,
                &data_descriptor(
                    Value::Object(radix_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(
                &parse_int,
                Value::Undefined,
                &[Value::Object(input.clone()), Value::Object(radix)],
            )
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context.get_property(&global, &log_key).unwrap(),
        Value::String(JsString::from_static("input:string|radix:number|"))
    );

    assert!(
        runtime
            .define_own_property(
                &global,
                &log_key,
                &data_descriptor(Value::String(JsString::from_static("")), true, true, true),
            )
            .unwrap()
    );
    let throwing_input = context.new_object().unwrap();
    let input_throw = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { parseLog = parseLog + 'input-throw:' + hint + '|'; throw 'input boom'; })",
    );
    assert!(
        runtime
            .define_own_property(
                &throwing_input,
                &to_primitive,
                &data_descriptor(
                    Value::Object(input_throw.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    let late_radix = context.new_object().unwrap();
    let late_radix_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { parseLog = parseLog + 'late-radix|'; return 2; })",
    );
    assert!(
        runtime
            .define_own_property(
                &late_radix,
                &to_primitive,
                &data_descriptor(
                    Value::Object(late_radix_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(
            &parse_int,
            Value::Undefined,
            &[Value::Object(throwing_input), Value::Object(late_radix),],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("input boom")))
    );
    assert_eq!(
        context.get_property(&global, &log_key).unwrap(),
        Value::String(JsString::from_static("input-throw:string|"))
    );

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("parse")))
        .unwrap();
    assert!(
        runtime
            .define_own_property(
                &global,
                &log_key,
                &data_descriptor(Value::String(JsString::from_static("")), true, true, true),
            )
            .unwrap()
    );
    let symbol_radix = context.new_object().unwrap();
    let symbol_radix_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { parseLog = parseLog + 'symbol-radix|'; return 2; })",
    );
    assert!(
        runtime
            .define_own_property(
                &symbol_radix,
                &to_primitive,
                &data_descriptor(
                    Value::Object(symbol_radix_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(
            &parse_int,
            Value::Undefined,
            &[Value::Symbol(symbol.clone()), Value::Object(symbol_radix),],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("cannot convert symbol to string")
    );
    assert_eq!(
        context.get_property(&global, &log_key).unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert!(matches!(
        context.call(&parse_float, Value::Undefined, &[Value::Symbol(symbol)],),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("cannot convert symbol to string")
    );

    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(defining_type_error_prototype) = context
        .get_property(type_error.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("defining TypeError.prototype was not an object");
    };
    let mut caller = runtime.new_context();
    assert_eq!(
        caller.call(
            &parse_int,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("10")),
                Value::BigInt(JsBigInt::one()),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm parseInt did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error_prototype)
    );
}

#[test]
fn global_numeric_predicates_match_quickjs_graph_and_coercion_split() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    for name in ["isNaN", "isFinite"] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(runtime.is_auto_init_own_property(&global, &key).unwrap());
    }
    let is_nan = global_callable(&runtime, &mut context, "isNaN");
    let is_finite = global_callable(&runtime, &mut context, "isFinite");
    let number = global_callable(&runtime, &mut context, "Number");
    let number_is_nan = property_callable(&runtime, &mut context, number.as_object(), "isNaN");
    let number_is_finite =
        property_callable(&runtime, &mut context, number.as_object(), "isFinite");
    assert_ne!(is_nan.as_object(), number_is_nan.as_object());
    assert_ne!(is_finite.as_object(), number_is_finite.as_object());

    for (name, callable) in [("isNaN", &is_nan), ("isFinite", &is_finite)] {
        assert_eq!(
            own_key_names(&runtime, callable.as_object()),
            ["length", "name"]
        );
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "length"),
            Value::Int(1)
        );
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "name"),
            Value::String(JsString::try_from_utf8(name).unwrap())
        );
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));
    }

    for (input, nan, finite) in [
        (Value::Undefined, true, false),
        (Value::Null, false, true),
        (Value::Bool(false), false, true),
        (Value::String(JsString::from_static("")), false, true),
        (Value::String(JsString::from_static("number")), true, false),
        (Value::Float(f64::NAN), true, false),
        (Value::Float(f64::INFINITY), false, false),
        (Value::Float(f64::NEG_INFINITY), false, false),
        (Value::Float(f64::from_bits(1)), false, true),
    ] {
        assert_eq!(
            context
                .call(
                    &is_nan,
                    Value::String(JsString::from_static("ignored")),
                    std::slice::from_ref(&input)
                )
                .unwrap(),
            Value::Bool(nan)
        );
        assert_eq!(
            context
                .call(&is_finite, Value::Null, &[input, Value::Int(99)])
                .unwrap(),
            Value::Bool(finite)
        );
    }
    assert_eq!(
        context.call(&is_nan, Value::Undefined, &[]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.call(&is_finite, Value::Undefined, &[]).unwrap(),
        Value::Bool(false)
    );
    for callable in [&is_nan, &is_finite] {
        assert_eq!(
            context.call(
                callable,
                Value::Undefined,
                &[Value::BigInt(JsBigInt::one())],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from_static("cannot convert bigint to number")
        );
    }
    assert_eq!(
        context.eval("isNaN('x') + '|' + isFinite('1')").unwrap(),
        Value::String(JsString::from_static("true|true"))
    );
}

#[test]
fn global_uri_codecs_match_quickjs_graph_and_utf16_kernel() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let names = [
        "decodeURI",
        "decodeURIComponent",
        "encodeURI",
        "encodeURIComponent",
        "escape",
        "unescape",
    ];
    for name in names {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(runtime.is_auto_init_own_property(&global, &key).unwrap());
        let callable = global_callable(&runtime, &mut context, name);
        assert_eq!(
            own_key_names(&runtime, callable.as_object()),
            ["length", "name"]
        );
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "length"),
            Value::Int(1)
        );
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "name"),
            Value::String(JsString::try_from_utf8(name).unwrap())
        );
        assert!(matches!(
            runtime.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));
    }

    for (name, input, expected) in [
        ("decodeURI", "%2f%20", "%2f "),
        ("decodeURIComponent", "%2f%20", "/ "),
        ("encodeURI", ";/ ?", ";/%20?"),
        ("encodeURIComponent", ";/ ?", "%3B%2F%20%3F"),
        ("unescape", "%E9%u0100", "éĀ"),
    ] {
        let callable = global_callable(&runtime, &mut context, name);
        assert_eq!(
            context
                .call(
                    &callable,
                    Value::Null,
                    &[
                        Value::String(JsString::try_from_utf8(input).unwrap()),
                        Value::Int(99),
                    ],
                )
                .unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap())
        );
    }
    let escape = global_callable(&runtime, &mut context, "escape");
    assert_eq!(
        context
            .call(
                &escape,
                Value::Undefined,
                &[Value::String(
                    JsString::try_from_utf16([0x00e9, 0xd83d, 0xde00]).unwrap(),
                )],
            )
            .unwrap(),
        Value::String(JsString::from_static("%E9%uD83D%uDE00"))
    );

    for (name, input, message) in [
        ("decodeURI", "%", "expecting hex digit"),
        ("decodeURIComponent", "%E0%A0", "expecting %"),
    ] {
        let callable = global_callable(&runtime, &mut context, name);
        assert_eq!(
            context.call(
                &callable,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8(input).unwrap())],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::try_from_utf8(message).unwrap()
        );
    }
    let encode = global_callable(&runtime, &mut context, "encodeURI");
    assert_eq!(
        context.call(
            &encode,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf16([0xdc00]).unwrap(),)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("invalid character")
    );
}

#[test]
fn global_primitive_constants_match_quickjs_frozen_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let supported_global_prefix = own_key_names(&runtime, &global)
        .into_iter()
        .filter(|name| {
            matches!(
                name.as_str(),
                "parseInt"
                    | "parseFloat"
                    | "isNaN"
                    | "isFinite"
                    | "decodeURI"
                    | "decodeURIComponent"
                    | "encodeURI"
                    | "encodeURIComponent"
                    | "escape"
                    | "unescape"
                    | "Infinity"
                    | "NaN"
                    | "undefined"
                    | "Number"
                    | "Boolean"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        supported_global_prefix,
        [
            "parseInt",
            "parseFloat",
            "isNaN",
            "isFinite",
            "decodeURI",
            "decodeURIComponent",
            "encodeURI",
            "encodeURIComponent",
            "escape",
            "unescape",
            "Infinity",
            "NaN",
            "undefined",
            "Number",
            "Boolean",
        ]
    );
    for (name, expected) in [
        ("undefined", Value::Undefined),
        ("NaN", Value::Float(f64::NAN)),
        ("Infinity", Value::Float(f64::INFINITY)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let Some(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }) = runtime.get_own_property(&global, &key).unwrap()
        else {
            panic!("global {name} was not an own data property");
        };
        assert!(value.same_value(&expected), "global {name}");
        assert!(!writable, "global {name}");
        assert!(!enumerable, "global {name}");
        assert!(!configurable, "global {name}");
        assert!(!runtime.delete_property(&global, &key).unwrap());
        assert_eq!(
            context.eval(&format!("delete {name}")).unwrap(),
            Value::Bool(false)
        );
    }
}

#[test]
fn global_to_string_tag_matches_quickjs_descriptor_and_class_tag() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(matches!(
        runtime.get_own_property(&global, &tag).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("global")
    ));
    assert_eq!(
        runtime.own_property_keys(&global).unwrap().last(),
        Some(&tag)
    );

    let object_prototype = context.object_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    assert_eq!(
        context
            .call(&object_to_string, Value::Object(global.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("[object global]"))
    );
    assert!(runtime.delete_property(&global, &tag).unwrap());
    assert_eq!(
        context
            .call(&object_to_string, Value::Object(global), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object Object]"))
    );
}

#[test]
fn global_this_matches_quickjs_identity_descriptor_and_mutation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("globalThis").unwrap();
    assert!(matches!(
        runtime.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == global
    ));
    assert_eq!(
        context
            .eval("globalThis === globalThis.globalThis")
            .unwrap(),
        Value::Bool(true)
    );

    assert!(context.set_property(&global, &key, Value::Int(17)).unwrap());
    assert_eq!(context.eval("globalThis").unwrap(), Value::Int(17));
    assert!(matches!(
        runtime.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(17),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    assert!(runtime.delete_property(&global, &key).unwrap());
    assert_eq!(
        context.eval("typeof globalThis").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert!(
        context
            .set_property(&global, &key, Value::Object(global.clone()))
            .unwrap()
    );
    assert!(matches!(
        runtime.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: true,
            configurable: true,
        }) if value == global
    ));
}

#[test]
fn eval_uses_the_compiler_and_vm_path() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval("6 * 7").unwrap(), Value::Int(42));
    assert_eq!(
        context.eval("this").unwrap(),
        Value::Object(context.global_object().unwrap())
    );
}

#[test]
fn raw_script_context_apis_compile_and_evaluate() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for function in [
        context.compile_bytes(b"6 * 7").unwrap(),
        context
            .compile_bytes_with_filename(b"40 + 2", "raw-filename.js")
            .unwrap(),
        context
            .compile_bytes_with_options(b"41 + 1", &CompileOptions::new("raw-options.js"))
            .unwrap(),
    ] {
        assert_eq!(context.execute(&function).unwrap(), Value::Int(42));
    }

    assert_eq!(context.eval_bytes(b"6 * 7").unwrap(), Value::Int(42));
    assert_eq!(
        context
            .eval_bytes_with_filename(b"40 + 2", "raw-filename.js")
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context
            .eval_bytes_with_options(b"41 + 1", &EvalOptions::new("raw-options.js"))
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn raw_script_syntax_error_uses_authored_bytes_and_filename() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval_bytes_with_filename(b"/*\x80*/@", "raw-parse.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("raw SyntaxError was not an object");
    };
    assert_eq!(
        own_data_value(&runtime, &error, "fileName"),
        Value::String(JsString::from_static("raw-parse.js"))
    );
    assert_eq!(
        own_data_value(&runtime, &error, "lineNumber"),
        Value::Int(1)
    );
    assert_eq!(
        own_data_value(&runtime, &error, "columnNumber"),
        Value::Int(5)
    );
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static("    at raw-parse.js:1:5\n")
    );
}

#[test]
fn raw_script_function_source_respects_debug_info_mode() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
    let raw = b"(function raw(){/*\x80X*/return 42;})";

    let Value::Object(full) = context
        .eval_bytes_with_filename(raw, "raw-full.js")
        .unwrap()
    else {
        panic!("raw full-debug source did not return a function");
    };
    let expected = JsString::try_from_bytes(b"function raw(){/*\x80X*/return 42;}").unwrap();
    assert_eq!(
        context
            .call(&to_string, Value::Object(full.clone()), &[])
            .unwrap(),
        Value::String(expected.clone())
    );
    let expected_units = expected.utf16_units().collect::<Vec<_>>();
    assert!(expected_units.contains(&0xfffd));
    assert!(!expected_units.contains(&u16::from(b'X')));

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let Value::Object(source_stripped) = context
        .eval_bytes_with_filename(raw, "raw-source-stripped.js")
        .unwrap()
    else {
        panic!("raw source-stripped source did not return a function");
    };
    let file_name = runtime.intern_property_key("fileName").unwrap();
    assert_eq!(
        context.get_property(&source_stripped, &file_name).unwrap(),
        Value::String(JsString::from_static("raw-source-stripped.js"))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function raw() {\n    [native code]\n}"
        ))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let Value::Object(debug_stripped) = context
        .eval_bytes_with_filename(raw, "raw-debug-stripped.js")
        .unwrap()
    else {
        panic!("raw debug-stripped source did not return a function");
    };
    assert_eq!(
        context.get_property(&debug_stripped, &file_name).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(debug_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function raw() {\n    [native code]\n}"
        ))
    );
}

#[test]
fn boolean_wrappers_lookup_and_new_target_use_the_required_realms() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_constructor = global_callable(&runtime, &mut first, "Boolean");
    let second_constructor = global_callable(&runtime, &mut second, "Boolean");
    let first_prototype = first.boolean_prototype().unwrap();
    let second_prototype = second.boolean_prototype().unwrap();
    let first_object_prototype = first.object_prototype().unwrap();
    let first_object_value_of =
        property_callable(&runtime, &mut first, &first_object_prototype, "valueOf");
    let Value::Object(method_wrapper) = second
        .call(&first_object_value_of, Value::Bool(false), &[])
        .unwrap()
    else {
        panic!("cross-realm Object.prototype.valueOf did not box Boolean");
    };
    assert_eq!(
        runtime.get_prototype_of(&method_wrapper).unwrap(),
        Some(first_prototype.clone())
    );

    let Value::Object(cross_wrapper) = second
        .construct_with_new_target(
            &first_constructor,
            &second_constructor,
            &[Value::Bool(true)],
        )
        .unwrap()
    else {
        panic!("cross-realm Boolean construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&cross_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    let second_value_of = property_callable(&runtime, &mut second, &second_prototype, "valueOf");
    assert_eq!(
        first
            .call(&second_value_of, Value::Object(cross_wrapper.clone()), &[],)
            .unwrap(),
        Value::Bool(true)
    );

    let marker = runtime.intern_property_key("realmMarker").unwrap();
    assert!(
        first
            .define_own_property(
                &first_prototype,
                &marker,
                &data_descriptor(Value::Int(1), true, false, true),
            )
            .unwrap()
    );
    assert!(
        second
            .define_own_property(
                &second_prototype,
                &marker,
                &data_descriptor(Value::Int(2), true, false, true),
            )
            .unwrap()
    );
    let callable = |runtime: &Runtime, context: &mut super::Context, source: &str| {
        let Value::Object(function) = context.eval(source).unwrap() else {
            panic!("realm lookup probe did not produce a function");
        };
        runtime.as_callable(&function).unwrap().unwrap()
    };
    let first_reader = callable(
        &runtime,
        &mut first,
        "(function(){ return true.realmMarker; })",
    );
    let second_reader = callable(
        &runtime,
        &mut second,
        "(function(){ return true.realmMarker; })",
    );
    assert_eq!(
        second.call(&first_reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        first.call(&second_reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );

    let custom_prototype = second.new_object().unwrap();
    let new_target = runtime
        .new_bound_native_function(
            &second.function_prototype().unwrap(),
            second.realm,
            NativeFunctionId::ConstructorProbe,
            0,
        )
        .unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        second
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &data_descriptor(Value::Object(custom_prototype.clone()), true, false, true,),
            )
            .unwrap()
    );
    let Value::Object(custom_wrapper) = first
        .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)])
        .unwrap()
    else {
        panic!("custom newTarget did not produce a Boolean wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_wrapper).unwrap(),
        Some(custom_prototype)
    );
    assert!(
        second
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback_wrapper) = first
        .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)])
        .unwrap()
    else {
        panic!("fallback newTarget did not produce a Boolean wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    let throwing_getter = bytecode_callable(
        &runtime,
        &second,
        vec![Instruction::PushI32(77), Instruction::Throw],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        second
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(throwing_getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        first.construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(first.take_exception().unwrap(), Some(Value::Int(77)));

    let escaped_this = callable(&runtime, &mut first, "(function(){ return this; })");
    let Value::Object(boxed_this) = second.call(&escaped_this, Value::Bool(false), &[]).unwrap()
    else {
        panic!("sloppy Boolean this did not escape as a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed_this).unwrap(),
        Some(first_prototype)
    );
    let stable_this = callable(
        &runtime,
        &mut first,
        "(function(){ return this === this; })",
    );
    assert_eq!(
        second.call(&stable_this, Value::Bool(false), &[]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn boolean_primitive_accessors_writes_and_delete_preserve_raw_receiver_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.boolean_prototype().unwrap();
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    let strict_getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let sloppy_getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    for (name, getter) in [
        ("strictReceiver", strict_getter),
        ("sloppyReceiver", sloppy_getter),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(
                    &prototype,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    assert_eq!(
        context.eval("false.strictReceiver").unwrap(),
        Value::Bool(false)
    );
    let Value::Object(sloppy_receiver) = context.eval("false.sloppyReceiver").unwrap() else {
        panic!("sloppy primitive getter did not receive a Boolean wrapper");
    };
    assert_eq!(
        context
            .call(&value_of, Value::Object(sloppy_receiver), &[])
            .unwrap(),
        Value::Bool(false)
    );

    let strict_setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Throw],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let sloppy_setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Throw],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    for (name, setter) in [("strictSink", strict_setter), ("sloppySink", sloppy_setter)] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(
                    &prototype,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    assert_eq!(
        context.eval("false.strictSink = 7"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Bool(false)));
    assert_eq!(
        context.eval("false.sloppySink = 7"),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(sloppy_receiver)) = context.take_exception().unwrap() else {
        panic!("sloppy primitive setter did not receive a Boolean wrapper");
    };
    assert_eq!(
        context
            .call(&value_of, Value::Object(sloppy_receiver), &[])
            .unwrap(),
        Value::Bool(false)
    );

    let writable = runtime.intern_property_key("writablePrimitive").unwrap();
    let read_only = runtime.intern_property_key("readOnlyPrimitive").unwrap();
    assert!(
        context
            .define_own_property(
                &prototype,
                &writable,
                &data_descriptor(Value::Int(1), true, false, true),
            )
            .unwrap()
    );
    assert!(
        context
            .define_own_property(
                &prototype,
                &read_only,
                &data_descriptor(Value::Int(1), false, false, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("false.writablePrimitive = 7").unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        context.eval("false.writablePrimitive").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("'use strict'; false.writablePrimitive = 7"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not an object")
    );
    assert_eq!(
        context.eval("'use strict'; false.readOnlyPrimitive = 7"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("'readOnlyPrimitive' is read-only")
    );
    assert_eq!(
        context.eval("delete false.writablePrimitive").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("false.writablePrimitive").unwrap(),
        Value::Int(1)
    );
}

#[test]
fn object_prototype_boolean_methods_box_only_the_quickjs_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let boolean_prototype = context.boolean_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let boolean_value_of = property_callable(&runtime, &mut context, &boolean_prototype, "valueOf");

    assert_eq!(
        context
            .call(&object_to_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object Boolean]"))
    );
    assert_eq!(
        context
            .call(&object_to_locale_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("false"))
    );
    let Value::Object(first_wrapper) = context
        .call(&object_value_of, Value::Bool(false), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box Boolean primitive");
    };
    let Value::Object(second_wrapper) = context
        .call(&object_value_of, Value::Bool(false), &[])
        .unwrap()
    else {
        panic!("second Object.prototype.valueOf did not box Boolean primitive");
    };
    assert_ne!(first_wrapper, second_wrapper);
    assert_eq!(
        runtime.get_prototype_of(&first_wrapper).unwrap(),
        Some(boolean_prototype.clone())
    );
    assert_eq!(
        context
            .call(&boolean_value_of, Value::Object(first_wrapper), &[])
            .unwrap(),
        Value::Bool(false)
    );

    let tag_receiver = runtime.intern_property_key("tagReceiver").unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &tag_receiver,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let Value::Object(tag_getter) = context
        .eval("(function(){ 'use strict'; tagReceiver = typeof this; return 'CustomBoolean'; })")
        .unwrap()
    else {
        panic!("@@toStringTag probe did not produce a function");
    };
    let tag_getter = runtime.as_callable(&tag_getter).unwrap().unwrap();
    let to_string_tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(
        context
            .define_own_property(
                &boolean_prototype,
                &to_string_tag,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(tag_getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&object_to_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object CustomBoolean]"))
    );
    assert_eq!(
        context
            .get_property(&context.global_object().unwrap(), &tag_receiver)
            .unwrap(),
        Value::String(JsString::from_static("object"))
    );

    let locale_receiver = runtime.intern_property_key("localeReceiver").unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &locale_receiver,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let Value::Object(locale_method) = context
        .eval("(function(){ 'use strict'; return typeof this; })")
        .unwrap()
    else {
        panic!("toLocaleString method probe did not produce a function");
    };
    let locale_method = runtime.as_callable(&locale_method).unwrap().unwrap();
    let locale_method_key = runtime.intern_property_key("localeMethod").unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &locale_method_key,
                &data_descriptor(
                    Value::Object(locale_method.as_object().clone()),
                    true,
                    true,
                    true,
                ),
            )
            .unwrap()
    );
    let Value::Object(locale_getter) = context
        .eval("(function(){ 'use strict'; localeReceiver = typeof this; return localeMethod; })")
        .unwrap()
    else {
        panic!("toLocaleString getter probe did not produce a function");
    };
    let locale_getter = runtime.as_callable(&locale_getter).unwrap().unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    assert!(
        context
            .define_own_property(
                &boolean_prototype,
                &to_string_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(locale_getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&object_to_locale_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("boolean"))
    );
    assert_eq!(
        context
            .get_property(&context.global_object().unwrap(), &locale_receiver)
            .unwrap(),
        Value::String(JsString::from_static("boolean"))
    );
}

#[test]
fn iterator_to_string_tag_accessor_matches_quickjs_metadata_and_setter_semantics() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let iterator_prototype = first.iterator_prototype().unwrap();
    let function_prototype = first.function_prototype().unwrap();
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(getter),
        set: Some(setter),
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&iterator_prototype, &tag)
        .unwrap()
        .unwrap()
    else {
        panic!("Iterator.prototype @@toStringTag was not the QuickJS accessor pair");
    };

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    for (callable, target, cproto, expected_name, expected_length) in [
        (
            &getter,
            NativeFunctionId::IteratorPrototypeToStringTagGetter,
            NativeCProto::Getter,
            "get [Symbol.toStringTag]",
            0,
        ),
        (
            &setter,
            NativeFunctionId::IteratorPrototypeToStringTagSetter,
            NativeCProto::Setter,
            "set [Symbol.toStringTag]",
            1,
        ),
    ] {
        assert_eq!(runtime.callable_realm(callable).unwrap(), first.realm);
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(function_prototype.clone())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            runtime
                .get_own_property(callable.as_object(), &prototype)
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime
                .get_own_property(callable.as_object(), &name)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::try_from_utf8(expected_name).unwrap()
        ));
        assert!(matches!(
            runtime
                .get_own_property(callable.as_object(), &length)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == expected_length
        ));
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(callable.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("Iterator tag accessor was not a native function");
        };
        assert_eq!(data.target, target);
        assert_eq!(data.target.descriptor().cproto, cproto);
    }

    for receiver in [
        Value::Null,
        Value::Int(1),
        Value::Object(first.new_object().unwrap()),
    ] {
        assert_eq!(
            second.call(&getter, receiver, &[]).unwrap(),
            Value::String(JsString::from_static("Iterator"))
        );
    }

    let iterator_global = runtime.intern_property_key("Iterator").unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(iterator_constructor),
        writable: true,
        enumerable: false,
        configurable: true,
    }) = runtime
        .get_own_property(&first.global_object().unwrap(), &iterator_global)
        .unwrap()
    else {
        panic!("global Iterator did not match the QuickJS property descriptor");
    };
    assert!(
        runtime
            .as_callable(&iterator_constructor)
            .unwrap()
            .is_some()
    );
    assert!(runtime.is_constructor(&iterator_constructor).unwrap());
    assert_eq!(
        runtime
            .get_own_property(&iterator_constructor, &prototype)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(iterator_prototype.clone()),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );

    let inherited = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(!runtime.has_own_property(&inherited, &tag).unwrap());
    assert!(
        first
            .set_property(
                &inherited,
                &tag,
                Value::String(JsString::from_static("Custom")),
            )
            .unwrap()
    );
    assert_eq!(
        runtime.get_own_property(&inherited, &tag).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(JsString::from_static("Custom")),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );

    let existing = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(
        runtime
            .define_own_property(
                &existing,
                &tag,
                &data_descriptor(
                    Value::String(JsString::from_static("old")),
                    true,
                    false,
                    false,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        first
            .call(
                &setter,
                Value::Object(existing.clone()),
                &[Value::String(JsString::from_static("new"))],
            )
            .unwrap(),
        Value::Undefined
    );
    assert_eq!(
        runtime.get_own_property(&existing, &tag).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(JsString::from_static("new")),
            writable: true,
            enumerable: false,
            configurable: false,
        })
    );

    let seen = runtime.intern_property_key("iteratorTagSeen").unwrap();
    let first_global = first.global_object().unwrap();
    assert!(
        first
            .define_own_property(
                &first_global,
                &seen,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let recording_setter = eval_callable(
        &runtime,
        &mut first,
        "(function(value) { iteratorTagSeen = value; })",
    );
    let own_accessor = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(
        runtime
            .define_own_property(
                &own_accessor,
                &tag,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(recording_setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        first
            .call(
                &setter,
                Value::Object(own_accessor),
                &[Value::String(JsString::from_static("seen"))],
            )
            .unwrap(),
        Value::Undefined
    );
    assert_eq!(
        first.get_property(&first_global, &seen).unwrap(),
        Value::String(JsString::from_static("seen"))
    );

    let readonly = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(
        runtime
            .define_own_property(
                &readonly,
                &tag,
                &data_descriptor(
                    Value::String(JsString::from_static("fixed")),
                    false,
                    false,
                    false,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        first.call(
            &setter,
            Value::Object(readonly),
            &[Value::String(JsString::from_static("no"))],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut first),
        JsString::from_static("'Symbol.toStringTag' is read-only")
    );

    assert_eq!(
        first.set_property(
            &iterator_prototype,
            &tag,
            Value::String(JsString::from_static("no")),
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut first),
        JsString::from_static("Cannot assign to read only property")
    );

    assert_eq!(
        second.call(
            &setter,
            Value::Int(1),
            &[Value::String(JsString::from_static("no"))],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(type_error) = second.take_exception().unwrap().unwrap() else {
        panic!("primitive Iterator tag receiver did not throw an Error object");
    };
    let type_error_constructor = global_callable(&runtime, &mut first, "TypeError");
    let Value::Object(type_error_prototype) = first
        .get_property(type_error_constructor.as_object(), &prototype)
        .unwrap()
    else {
        panic!("TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(type_error_prototype),
        "native accessor errors must use the accessor's defining realm"
    );
}

#[test]
fn string_iterator_inherits_iterator_tag_after_own_tag_is_deleted() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let string_prototype = context.string_prototype().unwrap();
    let string_iterator_prototype = context.string_iterator_prototype().unwrap();
    let iterator_prototype = context.iterator_prototype().unwrap();
    let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert_eq!(
        own_key_names(&runtime, &iterator_prototype),
        [
            "drop",
            "filter",
            "flatMap",
            "map",
            "take",
            "every",
            "find",
            "forEach",
            "some",
            "reduce",
            "toArray",
            "constructor",
            "Symbol.iterator",
            "Symbol.toStringTag",
        ]
    );

    let Value::Object(method) = context.get_property(&string_prototype, &iterator).unwrap() else {
        panic!("String.prototype @@iterator was not an object");
    };
    let method = runtime.as_callable(&method).unwrap().unwrap();
    let Value::Object(string_iterator) = context
        .call(&method, Value::String(JsString::from_static("x")), &[])
        .unwrap()
    else {
        panic!("String.prototype @@iterator did not return an object");
    };
    let object_prototype = context.object_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    assert_eq!(
        context
            .call(
                &object_to_string,
                Value::Object(string_iterator.clone()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::from_static("[object String Iterator]"))
    );

    assert!(
        runtime
            .delete_property(&string_iterator_prototype, &tag)
            .unwrap()
    );
    assert_eq!(
        context.get_property(&string_iterator, &tag).unwrap(),
        Value::String(JsString::from_static("Iterator"))
    );
    assert_eq!(
        context
            .call(&object_to_string, Value::Object(string_iterator), &[],)
            .unwrap(),
        Value::String(JsString::from_static("[object Iterator]"))
    );
}

#[test]
fn iterator_from_wraps_a_string_when_its_iterator_method_is_missing() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval(
            r#"
            (function () {
                var originalIterator = String.prototype[Symbol.iterator];
                var originalNext = String.prototype.next;
                var originalReturn = String.prototype.return;
                try {
                    String.prototype[Symbol.iterator] = null;
                    var missingNext = Iterator.from("abc");
                    var missingNextThrows = false;
                    try {
                        missingNext.next();
                    } catch (error) {
                        missingNextThrows = error instanceof TypeError;
                    }

                    var nextThis;
                    var returnThis;
                    var returnArgc = -1;
                    var returnResult = { done: true, value: 42 };
                    String.prototype.next = function () {
                        "use strict";
                        nextThis = this;
                        return { done: false, value: 7 };
                    };
                    String.prototype.return = function () {
                        "use strict";
                        returnThis = this;
                        returnArgc = arguments.length;
                        return returnResult;
                    };
                    delete String.prototype[Symbol.iterator];

                    var wrapped = Iterator.from("abc");
                    var nextResult = wrapped.next();
                    var actualReturn = wrapped.return("ignored");
                    return missingNextThrows
                        && nextResult.done === false
                        && nextResult.value === 7
                        && nextThis === "abc"
                        && returnThis === "abc"
                        && returnArgc === 0
                        && actualReturn === returnResult;
                } finally {
                    String.prototype[Symbol.iterator] = originalIterator;
                    if (originalNext === undefined) {
                        delete String.prototype.next;
                    } else {
                        String.prototype.next = originalNext;
                    }
                    if (originalReturn === undefined) {
                        delete String.prototype.return;
                    } else {
                        String.prototype.return = originalReturn;
                    }
                }
            })()
            "#,
        )
        .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn iterator_close_skips_only_result_brand_check_for_pending_exception() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let primitive_return = eval_callable(&runtime, &mut context, "(function(){ return 1; })");
    let return_key = runtime.intern_property_key("return").unwrap();
    let iterator = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &iterator,
                &return_key,
                &data_descriptor(
                    Value::Object(primitive_return.as_object().clone()),
                    true,
                    true,
                    true,
                ),
            )
            .unwrap()
    );
    let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
    assert!(matches!(
        VmHost::iterator_close(&mut host, Value::Object(iterator.clone()), true).unwrap(),
        IteratorCloseOutcome::Closed
    ));
    assert!(matches!(
        VmHost::iterator_close(&mut host, Value::Object(iterator), false).unwrap(),
        IteratorCloseOutcome::Throw(Value::Object(_))
    ));

    let non_callable = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &non_callable,
                &return_key,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        VmHost::iterator_close(&mut host, Value::Object(non_callable), true).unwrap(),
        IteratorCloseOutcome::Throw(Value::Object(_))
    ));
}

#[test]
fn native_iterator_next_wraps_public_calls_but_for_of_consumes_raw_outcomes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let iterator_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let string_prototype = context.string_prototype().unwrap();
    let Value::Object(iterator_method) = context
        .get_property(&string_prototype, &iterator_key)
        .unwrap()
    else {
        panic!("String.prototype @@iterator was not an object");
    };
    let iterator_method = runtime.as_callable(&iterator_method).unwrap().unwrap();
    let Value::Object(iterator) = context
        .call(
            &iterator_method,
            Value::String(JsString::from_static("A")),
            &[],
        )
        .unwrap()
    else {
        panic!("String.prototype @@iterator did not return an object");
    };
    let next_key = runtime.intern_property_key("next").unwrap();
    let Value::Object(next) = context.get_property(&iterator, &next_key).unwrap() else {
        panic!("String Iterator next was not an object");
    };
    let next = runtime.as_callable(&next).unwrap().unwrap();
    {
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(next.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("String Iterator next was not a direct native function");
        };
        assert_eq!(data.target, NativeFunctionId::StringIteratorNext);
        assert_eq!(data.target.descriptor().cproto, NativeCProto::IteratorNext);
    }

    let before_public = runtime.0.state.borrow().iterator_result_allocations;
    let Value::Object(result) = context.call(&next, Value::Object(iterator), &[]).unwrap() else {
        panic!("ordinary String Iterator next call did not return an object");
    };
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_public + 1
    );
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    let value_key = runtime.intern_property_key("value").unwrap();
    let done_key = runtime.intern_property_key("done").unwrap();
    assert_eq!(
        runtime.get_own_property(&result, &value_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(JsString::from_static("A")),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );
    assert_eq!(
        runtime.get_own_property(&result, &done_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Bool(false),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );

    let before_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval("(function(){var s='';for(var value of 'ab')s+=value;return s})()")
            .unwrap(),
        Value::String(JsString::from_static("ab"))
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_raw,
        "direct native ForOfNext must not allocate iterator-result wrappers"
    );

    let before_array_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval("(function(){var sum=0;for(var value of [20,22])sum+=value;return sum})()")
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_array_raw,
        "direct Array Iterator next must use the raw native ABI"
    );

    let before_concat_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval(
                "(function(){var sum=0;for(var value of Iterator.concat([20],[22]))sum+=value;return sum})()",
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_concat_raw,
        "direct Iterator Concat next and its inner built-ins must use the raw native ABI"
    );

    let before_bound = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval(
                "(function(){var iterator='z'[Symbol.iterator]();iterator.next=iterator.next.bind(iterator);function Iterable(){};Iterable.prototype[Symbol.iterator]=function(){return iterator};var s='';for(var value of new Iterable)s+=value;return s})()",
            )
            .unwrap(),
        Value::String(JsString::from_static("z"))
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_bound + 2,
        "a bound iterator-next method must retain generic call wrapping"
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn native_iterator_next_raw_dispatch_keeps_the_outer_operation_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let iterator_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let next_key = runtime.intern_property_key("next").unwrap();
    let string_prototype = defining.string_prototype().unwrap();
    let Value::Object(iterator_method) = defining
        .get_property(&string_prototype, &iterator_key)
        .unwrap()
    else {
        panic!("defining String.prototype @@iterator was not an object");
    };
    let iterator_method = runtime.as_callable(&iterator_method).unwrap().unwrap();
    let Value::Object(foreign_iterator) = caller
        .call(
            &iterator_method,
            Value::String(JsString::from_static("xy")),
            &[],
        )
        .unwrap()
    else {
        panic!("cross-realm String iterator creation did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&foreign_iterator).unwrap(),
        Some(defining.string_iterator_prototype().unwrap())
    );
    let Value::Object(foreign_next) = defining.get_property(&foreign_iterator, &next_key).unwrap()
    else {
        panic!("cross-realm String Iterator next was not an object");
    };

    let foreign_iterator_key = runtime.intern_property_key("foreignIterator").unwrap();
    let foreign_next_key = runtime.intern_property_key("foreignNext").unwrap();
    let caller_global = caller.global_object().unwrap();
    for (key, value) in [
        (
            &foreign_iterator_key,
            Value::Object(foreign_iterator.clone()),
        ),
        (&foreign_next_key, Value::Object(foreign_next.clone())),
    ] {
        assert!(
            caller
                .define_own_property(
                    &caller_global,
                    key,
                    &data_descriptor(value, true, true, true),
                )
                .unwrap()
        );
    }

    let before_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        caller
            .eval("(function(){var s='';for(var value of foreignIterator)s+=value;return s})()",)
            .unwrap(),
        Value::String(JsString::from_static("xy"))
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_raw
    );

    let Value::Object(error) = caller
        .eval(
            "(function(){function Invalid(){};Invalid.prototype[Symbol.iterator]=function(){return this};Invalid.prototype.next=foreignNext;try{for(var value of new Invalid)value}catch(error){return error}})()",
        )
        .unwrap()
    else {
        panic!("wrong-brand raw iterator-next call did not return its caught Error");
    };
    let type_error = global_callable(&runtime, &mut caller, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(type_error_prototype) = caller
        .get_property(type_error.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(type_error_prototype),
        "QuickJS's direct native iterator-next path keeps the outer operation realm"
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_raw
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn boolean_wrapper_keeps_its_realm_graph_alive_until_collection() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        let constructor = global_callable(&runtime, &mut context, "Boolean");
        let Value::Object(wrapper) = context
            .construct(&constructor, &[Value::Bool(true)])
            .unwrap()
        else {
            panic!("Boolean construction did not return a wrapper");
        };
        wrapper
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn bytecode_is_rooted_and_calls_separate_caller_from_callee_realm() {
    let runtime = Runtime::new();
    let mut compiler_context = runtime.new_context();
    let compiler_realm = compiler_context.realm;
    let intrinsic_realm_roots = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(compiler_realm)
        .unwrap();
    let function = compiler_context.compile("this").unwrap();
    let bytecode_id = function.bytecode_id();

    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode_strong_count(bytecode_id),
        Ok(1)
    );
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(compiler_realm),
        Ok(intrinsic_realm_roots + 1)
    );
    let duplicate = function.clone();
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode_strong_count(bytecode_id),
        Ok(2)
    );
    drop(duplicate);

    let mut caller_context = runtime.new_context();
    let caller_global = caller_context.global_object().unwrap();
    drop(compiler_context);
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    let snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
    assert_eq!(snapshot.realm, compiler_realm);
    drop(snapshot);
    assert_eq!(
        caller_context.execute(&function).unwrap(),
        Value::Object(caller_global)
    );

    drop(function);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.heap_counts().context_nodes, 2);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn publication_rejects_value_opcode_for_child_bytecode_before_heap_changes() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let child = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let root = UnlinkedFunction::new(
        vec![Instruction::PushConst(0), Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, root),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_mismatched_regexp_constants_even_in_dead_code() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let pattern = JsString::from_static("a");
    let flags = JsString::from_static("g");
    let program = std::rc::Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());

    let regexp_as_value = UnlinkedFunction::new(
        vec![Instruction::PushConst(0), Instruction::Return],
        vec![UnlinkedConstant::regexp(pattern, program)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, regexp_as_value),
        Err(RuntimeError::Engine(_))
    ));

    let value_as_regexp = UnlinkedFunction::new(
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::RegExp(0),
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("a"))).unwrap()],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, value_as_regexp),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_string_key_opcodes_with_non_string_constants() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    for (code, max_stack) in [
        (
            vec![
                Instruction::Undefined,
                Instruction::SetName(0),
                Instruction::Return,
            ],
            1,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::GetField(0),
                Instruction::Return,
            ],
            1,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::GetField2(0),
                Instruction::Drop,
                Instruction::Return,
            ],
            2,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::PutField(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            2,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineField(0),
                Instruction::Return,
            ],
            2,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineMethod {
                    key: 0,
                    kind: crate::bytecode::DefineMethodKind::Method,
                    enumerable: true,
                },
                Instruction::Return,
            ],
            2,
        ),
    ] {
        let function = UnlinkedFunction::new(
            code,
            vec![UnlinkedConstant::primitive(Value::Int(1)).unwrap()],
            FunctionMetadata {
                max_stack,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    let child = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let function = UnlinkedFunction::new(
        vec![
            Instruction::Undefined,
            Instruction::GetField(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn call_frame_loads_arguments_and_moves_values_through_locals() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = UnlinkedFunction::new(
        vec![
            Instruction::GetArg(0),
            Instruction::PutLocal(0),
            Instruction::GetLocal(0),
            Instruction::GetArg(1),
            Instruction::Add,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();

    assert_eq!(
        context
            .call(
                &callable,
                Value::Undefined,
                &[Value::Int(20), Value::Int(22)]
            )
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn runtime_typeof_distinguishes_callable_and_ordinary_objects() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = UnlinkedFunction::new(
        vec![
            Instruction::GetArg(0),
            Instruction::TypeOf,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();
    let ordinary = runtime.new_object(None).unwrap();

    let function_type = expect_string_value(
        context
            .call(
                &callable,
                Value::Undefined,
                &[Value::Object(callable.as_object().clone())],
            )
            .unwrap(),
    );
    assert_eq!(function_type, JsString::from_static("function"));
    let repeated_function_type = expect_string_value(
        context
            .call(
                &callable,
                Value::Undefined,
                &[Value::Object(callable.as_object().clone())],
            )
            .unwrap(),
    );
    assert!(function_type.same_representation(&repeated_function_type));
    let function_key = runtime.intern_property_key("function").unwrap();
    let canonical_function = runtime.property_key_to_js_string(&function_key).unwrap();
    assert!(function_type.same_representation(&canonical_function));

    let foreign_runtime = Runtime::new();
    let foreign_key = foreign_runtime.intern_property_key("function").unwrap();
    let foreign_function = foreign_runtime
        .property_key_to_js_string(&foreign_key)
        .unwrap();
    assert!(!function_type.same_representation(&foreign_function));
    assert_eq!(
        context
            .call(&callable, Value::Undefined, &[Value::Object(ordinary)],)
            .unwrap(),
        Value::String(JsString::from_static("object"))
    );
}

#[test]
fn ordinary_function_object_properties_match_quickjs_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(function) = context.eval("(function(a, b) {})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(runtime.is_constructor(&function).unwrap());

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();

    assert_eq!(
        runtime.own_property_keys(&function).unwrap(),
        vec![length.clone(), name.clone(), prototype_key.clone()]
    );

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name_value),
        writable: false,
        enumerable: false,
        configurable: true,
    } = runtime.get_own_property(&function, &name).unwrap().unwrap()
    else {
        panic!("unexpected function name descriptor");
    };
    assert!(name_value.is_empty());
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Int(2),
        writable: false,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&function, &length)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected function length descriptor");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: true,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected function prototype descriptor");
    };
    assert_eq!(
        context.get_property(&prototype, &constructor).unwrap(),
        Value::Object(function.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap().unwrap(),
        context.object_prototype().unwrap()
    );

    drop(prototype);
    drop(function);
    assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 2);
}

#[test]
fn function_prototype_autoinit_preserves_keys_without_eager_object_cycle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let Value::Object(unread) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
    drop(unread);
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

    let Value::Object(function) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);

    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert_eq!(
        runtime.own_property_keys(&function).unwrap(),
        vec![length, name, prototype_key.clone()]
    );
    assert!(runtime.has_own_property(&function, &prototype_key).unwrap());
    assert!(!runtime.delete_property(&function, &prototype_key).unwrap());
    assert!(
        !runtime
            .define_own_property(
                &function,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: true,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("prototype autoinit produced the wrong descriptor");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(second),
        ..
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("second prototype read did not return an object");
    };
    assert_eq!(prototype, second);

    drop(second);
    drop(prototype);
    drop(function);
    assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 2);
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
}

#[test]
fn compatible_define_materializes_function_prototype_but_value_override_releases_it() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let prototype_key = runtime.intern_property_key("prototype").unwrap();

    let Value::Object(empty_define) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(
        runtime
            .define_own_property(
                &empty_define,
                &prototype_key,
                &OrdinaryPropertyDescriptor::new(),
            )
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    drop(empty_define);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

    let Value::Object(value_define) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(
        runtime
            .define_own_property(
                &value_define,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
    assert!(matches!(
        runtime
            .get_own_property(&value_define, &prototype_key)
            .unwrap()
            .unwrap(),
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(1),
            writable: true,
            enumerable: false,
            configurable: false,
        }
    ));
}

#[test]
fn autoinit_define_checks_lazy_flags_before_materializing_and_retries() {
    let runtime = Runtime::new();
    let call_key = runtime.intern_property_key("call").unwrap();

    let configurable_context = runtime.new_context();
    let configurable_fp = configurable_context.function_prototype().unwrap();
    assert!(
        runtime
            .is_auto_init_own_property(&configurable_fp, &call_key)
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &configurable_fp,
                &call_key,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(
        !runtime
            .is_auto_init_own_property(&configurable_fp, &call_key)
            .unwrap()
    );
    assert!(matches!(
        runtime
            .get_own_property(&configurable_fp, &call_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            writable: true,
            enumerable: false,
            configurable: true,
            ..
        })
    ));

    let enumerable_context = runtime.new_context();
    let enumerable_fp = enumerable_context.function_prototype().unwrap();
    assert!(
        runtime
            .define_own_property(
                &enumerable_fp,
                &call_key,
                &OrdinaryPropertyDescriptor {
                    enumerable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        runtime.get_own_property(&enumerable_fp, &call_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            writable: true,
            enumerable: true,
            configurable: true,
            ..
        })
    ));

    let mut accessor_context = runtime.new_context();
    let accessor_fp = accessor_context.function_prototype().unwrap();
    let Value::Object(getter) = accessor_context
        .eval("(function replacementCall(){ return 7; })")
        .unwrap()
    else {
        panic!("replacement getter was not an object");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    assert!(
        runtime
            .define_own_property(
                &accessor_fp,
                &call_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter.clone())),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        runtime.get_own_property(&accessor_fp, &call_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor {
            get: Some(ref actual),
            set: None,
            enumerable: false,
            configurable: true,
        }) if actual == &getter
    ));
    assert_eq!(
        accessor_context
            .get_property(&accessor_fp, &call_key)
            .unwrap(),
        Value::Int(7)
    );

    let has_instance_context = runtime.new_context();
    let has_instance_fp = has_instance_context.function_prototype().unwrap();
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    assert!(
        runtime
            .is_auto_init_own_property(&has_instance_fp, &has_instance_key)
            .unwrap()
    );
    assert!(
        !runtime
            .define_own_property(
                &has_instance_fp,
                &has_instance_key,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(
        runtime
            .is_auto_init_own_property(&has_instance_fp, &has_instance_key)
            .unwrap()
    );
}

#[test]
fn failed_autoinit_commits_undefined_and_releases_initializer_realm() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = context.new_object().unwrap();
    let key = runtime.intern_property_key("failureProbe").unwrap();
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();
    runtime
        .define_failure_auto_init(&object, context.realm, "failureProbe")
        .unwrap();
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(before + 1)
    );

    assert!(matches!(
        runtime.get_own_property(&object, &key),
        Err(RuntimeError::Invariant("autoinit failure probe"))
    ));
    assert!(!runtime.is_auto_init_own_property(&object, &key).unwrap());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(before)
    );
    assert!(matches!(
        runtime.get_own_property(&object, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));
}

#[test]
fn function_prototype_autoinit_owns_and_uses_closure_creation_realm() {
    let runtime = Runtime::new();
    let compiler_context = runtime.new_context();
    let creation_context = runtime.new_context();
    let creation_realm = creation_context.realm;
    let function = runtime
        .publish_unlinked_function(
            compiler_context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    has_prototype: true,
                    constructor_kind: ConstructorKind::Base,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(creation_realm, &function)
        .unwrap();
    drop(creation_context);
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(creation_realm)
            .is_ok()
    );

    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        ..
    } = runtime
        .get_own_property(callable.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("prototype autoinit did not materialize an object");
    };
    let creation_object_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(creation_realm)
        .unwrap()
        .object_prototype;
    assert_eq!(
        runtime
            .get_prototype_of(&prototype)
            .unwrap()
            .unwrap()
            .object_id(),
        creation_object_prototype
    );

    drop(prototype);
    drop(callable);
    drop(function);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(creation_realm)
            .is_err()
    );
}

#[test]
fn base_construct_uses_explicit_new_target_prototype_and_realm_fallback() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();

    let Value::Object(constructor_object) = constructor_context
        .eval("(0, function(){ return 1; })")
        .unwrap()
    else {
        panic!("constructor source did not produce an object");
    };
    let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
    let Value::Object(target_object) = target_context.eval("(0, function(){})").unwrap() else {
        panic!("new-target source did not produce an object");
    };
    let new_target = runtime.as_callable(&target_object).unwrap().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();

    let explicit_prototype = target_context.new_object().unwrap();
    assert!(
        target_context
            .define_own_property(
                &target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(explicit_prototype.clone())),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(instance) = constructor_context
        .construct_with_new_target(&constructor, &new_target, &[])
        .unwrap()
    else {
        panic!("base constructor did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(explicit_prototype)
    );

    assert!(
        target_context
            .define_own_property(
                &target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback_instance) = constructor_context
        .construct_with_new_target(&constructor, &new_target, &[])
        .unwrap()
    else {
        panic!("base constructor fallback did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_instance).unwrap(),
        Some(target_context.object_prototype().unwrap())
    );
    assert_ne!(
        runtime.get_prototype_of(&fallback_instance).unwrap(),
        Some(constructor_context.object_prototype().unwrap())
    );
}

#[test]
fn new_target_prototype_getter_throw_short_circuits_constructor_body() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(constructor_object) = context
        .eval("(0, function(){ return function(){}; })")
        .unwrap()
    else {
        panic!("constructor source did not produce an object");
    };
    let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
    let new_target = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::Undefined, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            constructor_kind: ConstructorKind::Base,
            ..FunctionMetadata::default()
        },
    );
    let Value::Object(getter_object) = context.eval("(0, function(){ throw 9; })").unwrap() else {
        panic!("getter source did not produce an object");
    };
    let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        context
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    assert_eq!(
        context.construct_with_new_target(&constructor, &new_target, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
}

#[test]
fn construct_rejects_non_constructor_callable_with_caller_realm_type_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let callable = runtime.as_callable(&function_prototype).unwrap().unwrap();

    assert_eq!(
        context.construct(&callable, &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("construct failure did not materialize TypeError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(" is not a constructor"))
    );
}

#[test]
fn function_prototype_is_callable_non_constructable_and_has_no_prototype_property() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let callable = runtime
        .callable_from_value(Value::Object(function_prototype.clone()))
        .unwrap();
    assert_eq!(
        context
            .call(&callable, Value::Undefined, &[Value::Int(1)])
            .unwrap(),
        Value::Undefined
    );
    assert!(!runtime.is_constructor(&function_prototype).unwrap());
    assert_eq!(
        runtime
            .get_prototype_of(&function_prototype)
            .unwrap()
            .unwrap(),
        context.object_prototype().unwrap()
    );

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let caller = runtime.intern_property_key("caller").unwrap();
    let arguments = runtime.intern_property_key("arguments").unwrap();
    let call = runtime.intern_property_key("call").unwrap();
    let apply = runtime.intern_property_key("apply").unwrap();
    let bind = runtime.intern_property_key("bind").unwrap();
    let to_string = runtime.intern_property_key("toString").unwrap();
    let file_name = runtime.intern_property_key("fileName").unwrap();
    let line_number = runtime.intern_property_key("lineNumber").unwrap();
    let column_number = runtime.intern_property_key("columnNumber").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let has_instance = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert_eq!(
        runtime.own_property_keys(&function_prototype).unwrap(),
        vec![
            length.clone(),
            name.clone(),
            caller,
            arguments,
            call,
            apply,
            bind,
            to_string,
            file_name,
            line_number,
            column_number,
            constructor,
            has_instance,
        ]
    );
    assert!(matches!(
        runtime
            .get_own_property(&function_prototype, &name)
            .unwrap()
            .unwrap(),
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        } if value.is_empty()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&function_prototype, &length)
            .unwrap()
            .unwrap(),
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(0),
            writable: false,
            enumerable: false,
            configurable: true,
        }
    ));
    assert_eq!(
        runtime
            .get_own_property(&function_prototype, &prototype)
            .unwrap(),
        None
    );
}

#[test]
fn function_constructor_intrinsic_and_dynamic_source_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let global = context.global_object().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let function_key = runtime.intern_property_key("Function").unwrap();

    assert!(runtime.is_constructor(constructor.as_object()).unwrap());
    assert_eq!(runtime.callable_realm(&constructor).unwrap(), context.realm);
    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(function_prototype.clone())
    );
    assert_eq!(
        runtime.own_property_keys(constructor.as_object()).unwrap(),
        vec![length.clone(), name.clone(), prototype.clone()]
    );
    assert!(matches!(
        runtime
            .get_own_property(constructor.as_object(), &length)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(1),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    assert!(matches!(
        runtime
            .get_own_property(constructor.as_object(), &name)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("Function")
    ));
    assert!(matches!(
        runtime
            .get_own_property(constructor.as_object(), &prototype)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: false,
            enumerable: false,
            configurable: false,
        }) if value == function_prototype
    ));
    assert!(matches!(
        runtime.get_own_property(&global, &function_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == constructor.as_object().clone()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&function_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == constructor.as_object().clone()
    ));
    {
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(constructor.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("Function was not a native constructor");
        };
        assert_eq!(
            data.target,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal)
        );
        assert_eq!(
            data.target.descriptor().cproto,
            NativeCProto::ConstructorOrFunctionMagic
        );
        assert_eq!(data.min_readable_args, 1);
    }

    let to_string = runtime.intern_property_key("toString").unwrap();
    let Value::Object(to_string) = context
        .get_property(&function_prototype, &to_string)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not callable");
    };
    let to_string = runtime.as_callable(&to_string).unwrap().unwrap();
    assert_eq!(
        context
            .call(
                &to_string,
                Value::Object(constructor.as_object().clone()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "function Function() {\n    [native code]\n}"
        ))
    );

    let Value::Object(empty) = context.call(&constructor, Value::Null, &[]).unwrap() else {
        panic!("Function() did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&empty).unwrap(),
        Some(function_prototype)
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(empty.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("function anonymous(\n) {\n\n}"))
    );
    for (property_name, expected) in [
        ("name", Value::String(JsString::from_static("anonymous"))),
        ("length", Value::Int(0)),
        ("fileName", Value::String(JsString::from_static("<input>"))),
        ("lineNumber", Value::Int(1)),
        ("columnNumber", Value::Int(2)),
    ] {
        let key = runtime.intern_property_key(property_name).unwrap();
        assert_eq!(context.get_property(&empty, &key).unwrap(), expected);
    }

    let Value::Object(add) = context
        .call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("b")),
                Value::String(JsString::from_static("return a + b")),
            ],
        )
        .unwrap()
    else {
        panic!("Function parameters did not produce an object");
    };
    let add_callable = runtime.as_callable(&add).unwrap().unwrap();
    assert_eq!(
        context
            .call(
                &add_callable,
                Value::Undefined,
                &[Value::Int(20), Value::Int(22)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.call(&to_string, Value::Object(add), &[]).unwrap(),
        Value::String(JsString::from_static(
            "function anonymous(a,b\n) {\nreturn a + b\n}"
        ))
    );

    let Value::Object(duplicate) = context
        .call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("return a")),
            ],
        )
        .unwrap()
    else {
        panic!("sloppy duplicate parameters were rejected");
    };
    let duplicate = runtime.as_callable(&duplicate).unwrap().unwrap();
    assert_eq!(
        context
            .call(
                &duplicate,
                Value::Undefined,
                &[Value::Int(1), Value::Int(2)],
            )
            .unwrap(),
        Value::Int(2)
    );

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("\"use strict\"; return a")),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("strict duplicate parameters did not throw an Error");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf16([0xd800]).unwrap(),)],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("lone-surrogate source did not throw an Error");
    };
    for (property_name, expected) in [
        ("name", Value::String(JsString::from_static("SyntaxError"))),
        (
            "message",
            Value::String(JsString::from_static("unexpected character")),
        ),
        ("fileName", Value::String(JsString::from_static("<input>"))),
        ("lineNumber", Value::Int(3)),
        ("columnNumber", Value::Int(1)),
    ] {
        let key = runtime.intern_property_key(property_name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }
}

#[test]
fn function_constructor_html_comment_boundaries_match_quickjs_wrapper() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
                    typeof Function("-->"),
                    (function () {
                        try {
                            Function("-->", "");
                            return "missing";
                        } catch (error) {
                            return error.name;
                        }
                    })(),
                    typeof Function("\n-->", ""),
                    typeof Function("<!--"),
                    typeof Function("<!--", "")
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "function|SyntaxError|function|function|function"
        ))
    );
}

#[test]
fn function_constructor_uses_defining_realm_and_new_target_prototype() {
    let runtime = Runtime::new();
    let first = runtime.new_context();
    let mut second = runtime.new_context();
    let constructor = first.function_constructor().unwrap();
    let first_function_prototype = first.function_prototype().unwrap();
    let second_function_prototype = second.function_prototype().unwrap();
    let first_object_prototype = first.object_prototype().unwrap();
    let marker = runtime.intern_property_key("dynamicRealmMarker").unwrap();
    runtime
        .define_function_data_property(
            &first.global_object().unwrap(),
            "dynamicRealmMarker",
            Value::Int(11),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &second.global_object().unwrap(),
            "dynamicRealmMarker",
            Value::Int(22),
            true,
            true,
        )
        .unwrap();
    assert_eq!(
        second
            .get_property(&first.global_object().unwrap(), &marker)
            .unwrap(),
        Value::Int(11)
    );

    let Value::Object(dynamic) = second
        .call(
            &constructor,
            Value::Object(second.global_object().unwrap()),
            &[Value::String(JsString::from_static(
                "return dynamicRealmMarker",
            ))],
        )
        .unwrap()
    else {
        panic!("cross-realm Function call did not return an object");
    };
    let dynamic_callable = runtime.as_callable(&dynamic).unwrap().unwrap();
    assert_eq!(
        runtime.callable_realm(&dynamic_callable).unwrap(),
        first.realm
    );
    assert_eq!(
        runtime.get_prototype_of(&dynamic).unwrap(),
        Some(first_function_prototype.clone())
    );
    assert_eq!(
        second
            .call(&dynamic_callable, Value::Undefined, &[])
            .unwrap(),
        Value::Int(11)
    );

    let Value::Object(new_target) = second.eval("(function NewTarget(){})").unwrap() else {
        panic!("newTarget source did not return a function");
    };
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let custom_prototype = second.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(custom_prototype.clone())),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(customized) = second
        .construct_with_new_target(
            &constructor,
            &new_target,
            &[Value::String(JsString::from_static(
                "return dynamicRealmMarker",
            ))],
        )
        .unwrap()
    else {
        panic!("custom newTarget did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&customized).unwrap(),
        Some(custom_prototype)
    );
    let customized_callable = runtime.as_callable(&customized).unwrap().unwrap();
    assert_eq!(
        second
            .call(&customized_callable, Value::Undefined, &[])
            .unwrap(),
        Value::Int(11)
    );
    let Value::Object(instance_prototype) =
        second.get_property(&customized, &prototype_key).unwrap()
    else {
        panic!("dynamic function prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance_prototype).unwrap(),
        Some(first_object_prototype)
    );

    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback) = second
        .construct_with_new_target(
            &constructor,
            &new_target,
            &[Value::String(JsString::from_static("return 3"))],
        )
        .unwrap()
    else {
        panic!("fallback newTarget did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback).unwrap(),
        Some(second_function_prototype)
    );
    assert_eq!(runtime.callable_realm(&constructor).unwrap(), first.realm);
}

#[test]
fn function_constructor_samples_strip_mode_and_keeps_parse_locations() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let Value::Object(to_string) = context
        .get_property(&function_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string).unwrap().unwrap();
    let keys = ["fileName", "lineNumber", "columnNumber"]
        .map(|name| runtime.intern_property_key(name).unwrap());

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let Value::Object(source_stripped) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("return 1"))],
        )
        .unwrap()
    else {
        panic!("strip-source Function did not return an object");
    };
    for (key, expected) in keys.iter().zip([
        Value::String(JsString::from_static("<input>")),
        Value::Int(1),
        Value::Int(2),
    ]) {
        assert_eq!(
            context.get_property(&source_stripped, key).unwrap(),
            expected
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function anonymous() {\n    [native code]\n}"
        ))
    );
    let name = runtime.intern_property_key("name").unwrap();
    assert!(
        runtime
            .define_own_property(
                &source_stripped,
                &name,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from_static(
                        "renamed"
                    ))),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function renamed() {\n    [native code]\n}"
        ))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let Value::Object(debug_stripped) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("return 2"))],
        )
        .unwrap()
    else {
        panic!("strip-debug Function did not return an object");
    };
    for key in &keys {
        assert_eq!(
            context.get_property(&debug_stripped, key).unwrap(),
            Value::Undefined
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(debug_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function anonymous() {\n    [native code]\n}"
        ))
    );

    let message = runtime.intern_property_key("message").unwrap();
    for (description, body, expected_name, expected_message) in [
        (
            "local TDZ",
            "for(let value=value;false;){}",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "direct-eval local TDZ",
            "eval('');for(let value=value;false;){}",
            "ReferenceError",
            "value is not initialized",
        ),
        (
            "direct-eval environment TDZ",
            "let value=eval('value')",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "nested direct-eval environment TDZ",
            "let value=eval(\"eval('');value\")",
            "ReferenceError",
            "value is not initialized",
        ),
        (
            "closure TDZ read",
            "let read=()=>value;return read();let value",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "closure TDZ write",
            "let write=()=>value=1;return write();let value",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "parent direct-eval child closure TDZ",
            "eval('');let read=()=>value;return read();let value",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "child direct-eval closure TDZ",
            "let read=()=>{eval('');return value};return read();let value",
            "ReferenceError",
            "value is not initialized",
        ),
        (
            "derived this TDZ with descendant eval",
            "class A extends Object{constructor(){let f=()=>eval('');this.x;super()}}new A",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "closure readonly write",
            "let write;{const value=1;write=()=>value=2}return write()",
            "TypeError",
            "'value' is read-only",
        ),
    ] {
        let Value::Object(function) = context
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from_static(body))],
            )
            .unwrap_or_else(|error| panic!("strip-debug {description} failed to compile: {error}"))
        else {
            panic!("strip-debug {description} did not return an object");
        };
        let function = runtime.as_callable(&function).unwrap().unwrap();
        assert_eq!(
            context.call(&function, Value::Undefined, &[]),
            Err(RuntimeError::Exception),
            "{description}"
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("strip-debug {description} did not throw an Error");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static(expected_name)),
            "{description}"
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static(expected_message)),
            "{description}"
        );
    }

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a-")),
                Value::String(JsString::from_static("return 1")),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("malformed Function did not throw an Error");
    };
    for (name, expected) in [
        ("fileName", Value::String(JsString::from_static("<input>"))),
        ("lineNumber", Value::Int(1)),
        ("columnNumber", Value::Int(22)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }
    let stack = runtime.intern_property_key("stack").unwrap();
    let Value::String(stack) = context.get_property(&error, &stack).unwrap() else {
        panic!("Function syntax error had no stack");
    };
    assert_eq!(
        stack,
        JsString::from_static("    at <input>:1:22\n    at Function (native)\n")
    );
}

#[test]
fn function_constructor_orders_source_conversion_parse_and_prototype_get() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let global = context.global_object().unwrap();
    runtime
        .define_function_data_property(
            &global,
            "functionOrder",
            Value::String(JsString::from_static("")),
            true,
            true,
        )
        .unwrap();
    let custom_prototype = context.new_object().unwrap();
    runtime
        .define_function_data_property(
            &global,
            "functionCustomPrototype",
            Value::Object(custom_prototype.clone()),
            true,
            true,
        )
        .unwrap();

    let (
        parameter_to_string,
        body_to_string,
        bad_parameter_to_string,
        throwing_to_string,
        prototype_getter,
    ) = {
        let mut eval_callable = |source: &str| {
            let Value::Object(function) = context.eval(source).unwrap() else {
                panic!("conversion helper was not a function");
            };
            runtime.as_callable(&function).unwrap().unwrap()
        };
        (
            eval_callable("(function(){ functionOrder = functionOrder + \"p\"; return \"a\"; })"),
            eval_callable(
                "(function(){ functionOrder = functionOrder + \"b\"; return \"return a\"; })",
            ),
            eval_callable("(function(){ functionOrder = functionOrder + \"p\"; return \"a-\"; })"),
            eval_callable("(function(){ functionOrder = functionOrder + \"t\"; throw \"stop\"; })"),
            eval_callable(
                "(function(){ functionOrder = functionOrder + \"x\"; return functionCustomPrototype; })",
            ),
        )
    };

    let to_string = runtime.intern_property_key("toString").unwrap();
    let parameter = context.new_object().unwrap();
    let body = context.new_object().unwrap();
    runtime
        .define_function_data_property(
            &parameter,
            "toString",
            Value::Object(parameter_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &body,
            "toString",
            Value::Object(body_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    assert!(runtime.has_own_property(&parameter, &to_string).unwrap());

    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind).unwrap().unwrap();
    let Value::Object(new_target) = context
        .call(
            &bind,
            Value::Object(constructor.as_object().clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("bound Function was not an object");
    };
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(prototype_getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let Value::Object(function) = context
        .construct_with_new_target(
            &constructor,
            &new_target,
            &[
                Value::Object(parameter.clone()),
                Value::Object(body.clone()),
            ],
        )
        .unwrap()
    else {
        panic!("converted Function source did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&function).unwrap(),
        Some(custom_prototype)
    );
    let order = runtime.intern_property_key("functionOrder").unwrap();
    assert_eq!(
        context.get_property(&global, &order).unwrap(),
        Value::String(JsString::from_static("pbx"))
    );

    runtime
        .define_function_data_property(
            &global,
            "functionOrder",
            Value::String(JsString::from_static("")),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &parameter,
            "toString",
            Value::Object(bad_parameter_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    assert_eq!(
        context.construct_with_new_target(
            &constructor,
            &new_target,
            &[
                Value::Object(parameter.clone()),
                Value::Object(body.clone())
            ],
        ),
        Err(RuntimeError::Exception)
    );
    drop(context.take_exception().unwrap());
    assert_eq!(
        context.get_property(&global, &order).unwrap(),
        Value::String(JsString::from_static("pb"))
    );

    runtime
        .define_function_data_property(
            &global,
            "functionOrder",
            Value::String(JsString::from_static("")),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &parameter,
            "toString",
            Value::Object(throwing_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[Value::Object(parameter), Value::Object(body)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("stop")))
    );
    assert_eq!(
        context.get_property(&global, &order).unwrap(),
        Value::String(JsString::from_static("t"))
    );
}

#[test]
fn function_constructor_typed_realm_root_and_cycles_are_collectable() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let realm = context.realm;
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let global = context.global_object().unwrap();
    let function_key = runtime.intern_property_key("Function").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();

    assert!(runtime.delete_property(&global, &function_key).unwrap());
    assert!(
        runtime
            .delete_property(&function_prototype, &constructor_key)
            .unwrap()
    );
    drop(constructor);
    let rooted = context.function_constructor().unwrap();
    assert!(runtime.is_constructor(rooted.as_object()).unwrap());
    assert!(matches!(
        runtime.bytecode_for_callable(&rooted).unwrap(),
        CallableExecution::Native {
            target: NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
            realm: target_realm,
            min_readable_args: 1,
        } if target_realm == realm
    ));

    drop(rooted);
    drop(function_key);
    drop(constructor_key);
    drop(global);
    drop(function_prototype);
    drop(context);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.context(realm).is_err());
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
}

#[test]
fn dynamic_function_keeps_its_defining_realm_alive() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let defining_realm = defining.realm;
    let constructor = defining.function_constructor().unwrap();
    let Value::Object(function_object) = defining
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("return 9"))],
        )
        .unwrap()
    else {
        panic!("Function did not return a bytecode function");
    };
    let function = runtime.as_callable(&function_object).unwrap().unwrap();
    drop(function_object);
    let mut caller = runtime.new_context();

    drop(constructor);
    drop(defining);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_ok()
    );
    assert_eq!(
        caller.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Int(9)
    );

    drop(function);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_err()
    );
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn function_constructor_failure_paths_release_temporary_graphs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let live_counts = || {
        let counts = runtime.heap_counts();
        (
            counts.object_nodes,
            counts.shape_nodes,
            counts.var_ref_nodes,
            counts.context_nodes,
            counts.function_bytecode_nodes,
        )
    };

    let parse_baseline = live_counts();
    let parse_atom_baseline = runtime.test_atom_count();
    for _ in 0..3 {
        assert_eq!(
            context.call(
                &constructor,
                Value::Undefined,
                &[
                    Value::String(JsString::from_static("a-")),
                    Value::String(JsString::from_static("return 1")),
                ],
            ),
            Err(RuntimeError::Exception)
        );
        drop(context.take_exception().unwrap());
        runtime.run_gc().unwrap();
        assert_eq!(live_counts(), parse_baseline);
        assert_eq!(runtime.test_atom_count(), parse_atom_baseline);
    }

    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind).unwrap().unwrap();
    let Value::Object(new_target) = context
        .call(
            &bind,
            Value::Object(constructor.as_object().clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("bound Function was not an object");
    };
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let Value::Object(getter) = context
        .eval("(function(){ throw \"prototype\"; })")
        .unwrap()
    else {
        panic!("prototype getter was not an object");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    runtime.run_gc().unwrap();
    let getter_baseline = live_counts();
    let getter_atom_baseline = runtime.test_atom_count();
    for _ in 0..3 {
        assert_eq!(
            context.construct_with_new_target(
                &constructor,
                &new_target,
                &[Value::String(JsString::from_static("return 1"))],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            context.take_exception().unwrap(),
            Some(Value::String(JsString::from_static("prototype")))
        );
        runtime.run_gc().unwrap();
        assert_eq!(live_counts(), getter_baseline);
        assert_eq!(runtime.test_atom_count(), getter_atom_baseline);
    }
}

#[test]
fn function_debug_accessors_match_quickjs_descriptors_realms_and_receivers() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let function_prototype = first.function_prototype().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let specs = [
        (
            "fileName",
            "get fileName",
            NativeFunctionId::FunctionPrototypeFileName,
            NativeCProto::Getter,
        ),
        (
            "lineNumber",
            "get lineNumber",
            NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Line),
            NativeCProto::GetterMagic,
        ),
        (
            "columnNumber",
            "get columnNumber",
            NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Column),
            NativeCProto::GetterMagic,
        ),
    ];
    let mut getters = Vec::new();
    for (property_name, getter_name, target, cproto) in specs {
        let key = runtime.intern_property_key(property_name).unwrap();
        let CompleteOrdinaryPropertyDescriptor::Accessor {
            get: Some(getter),
            set: None,
            enumerable: false,
            configurable: true,
        } = runtime
            .get_own_property(&function_prototype, &key)
            .unwrap()
            .unwrap()
        else {
            panic!("{property_name} was not the expected getter-only accessor");
        };
        assert_eq!(
            runtime.get_prototype_of(getter.as_object()).unwrap(),
            Some(function_prototype.clone())
        );
        assert_eq!(runtime.callable_realm(&getter).unwrap(), first.realm);
        assert!(!runtime.is_constructor(getter.as_object()).unwrap());
        assert_eq!(
            runtime
                .get_own_property(getter.as_object(), &prototype_key)
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime
                .get_own_property(getter.as_object(), &length_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(0),
                writable: false,
                enumerable: false,
                configurable: true,
            })
        ));
        assert!(matches!(
            runtime
                .get_own_property(getter.as_object(), &name_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::try_from_utf8(getter_name).unwrap()
        ));
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(getter.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("debug accessor getter was not a native function");
        };
        assert_eq!(data.target, target);
        assert_eq!(data.target.descriptor().cproto, cproto);
        drop(state);
        getters.push((key, getter));
    }
    assert_ne!(getters[0].1.as_object(), getters[1].1.as_object());
    assert_ne!(getters[1].1.as_object(), getters[2].1.as_object());

    let source = "\n  (function named(){})";
    let Value::Object(function) = second.eval_with_filename(source, "receiver.js").unwrap() else {
        panic!("debug receiver source did not return a function");
    };
    for (index, expected) in [
        Value::String(JsString::from_static("receiver.js")),
        Value::Int(2),
        Value::Int(4),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            first.get_property(&function, &getters[index].0).unwrap(),
            expected
        );
        assert_eq!(
            first
                .call(
                    &getters[index].1,
                    Value::Object(function.clone()),
                    &[Value::Int(1), Value::Int(2)],
                )
                .unwrap(),
            expected,
            "getter ABI must ignore arguments and inspect the receiver"
        );
    }

    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind_object) = first.get_property(&function_prototype, &bind_key).unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(bound) = first
        .call(&bind, Value::Object(function.clone()), &[Value::Null])
        .unwrap()
    else {
        panic!("bind did not return an object");
    };
    let ordinary = first.new_object().unwrap();
    for (_, getter) in &getters {
        for receiver in [
            Value::Undefined,
            Value::Null,
            Value::Int(1),
            Value::Object(ordinary.clone()),
            Value::Object(function_prototype.clone()),
            Value::Object(bound.clone()),
            Value::Object(getter.as_object().clone()),
        ] {
            assert_eq!(first.call(getter, receiver, &[]).unwrap(), Value::Undefined);
        }
    }

    // If an embedder enables the constructor bit, QuickJS's getter cproto
    // receives newTarget as its receiver.
    let target = runtime.as_callable(&function).unwrap().unwrap();
    runtime
        .set_constructor_bit(getters[0].1.as_object(), true)
        .unwrap();
    assert_eq!(
        first
            .construct_with_new_target(&getters[0].1, &target, &[])
            .unwrap(),
        Value::String(JsString::from_static("receiver.js"))
    );
    runtime
        .set_constructor_bit(getters[0].1.as_object(), false)
        .unwrap();
}

#[test]
fn runtime_debug_info_mode_matches_quickjs_strip_source_and_strip_debug() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let keys = ["fileName", "lineNumber", "columnNumber"]
        .map(|name| runtime.intern_property_key(name).unwrap());
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let Value::Object(to_string_object) = context
        .get_property(&function_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();
    let expression = "\n  (function stripped() {})";

    let Value::Object(full) = context.eval_with_filename(expression, "full.js").unwrap() else {
        panic!("full debug compile did not return a function");
    };
    assert_eq!(runtime.debug_info_mode(), DebugInfoMode::Full);
    assert_eq!(
        context
            .call(&to_string, Value::Object(full.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("function stripped() {}"))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let Value::Object(source_stripped) = context
        .eval_with_filename(expression, "source-stripped.js")
        .unwrap()
    else {
        panic!("source-stripped compile did not return a function");
    };
    for (key, expected) in keys.iter().zip([
        Value::String(JsString::from_static("source-stripped.js")),
        Value::Int(2),
        Value::Int(4),
    ]) {
        assert_eq!(
            context.get_property(&source_stripped, key).unwrap(),
            expected
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function stripped() {\n    [native code]\n}"
        ))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let Value::Object(debug_stripped) = context
        .eval_with_filename(expression, "debug-stripped.js")
        .unwrap()
    else {
        panic!("debug-stripped compile did not return a function");
    };
    for key in &keys {
        assert_eq!(
            context.get_property(&debug_stripped, key).unwrap(),
            Value::Undefined
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(debug_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function stripped() {\n    [native code]\n}"
        ))
    );

    // Changing the runtime policy never mutates already-published bytecode.
    assert_eq!(
        context.call(&to_string, Value::Object(full), &[]).unwrap(),
        Value::String(JsString::from_static("function stripped() {}"))
    );
}

#[test]
fn function_debug_position_distinguishes_missing_debug_from_missing_pc_table() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let keys = ["fileName", "lineNumber", "columnNumber"]
        .map(|name| runtime.intern_property_key(name).unwrap());

    let with_debug = runtime
        .publish_unlinked_function(
            context.realm,
            debug_draft(UnlinkedFunctionDebug {
                filename: JsString::from_static("no-pc-table.js"),
                pc2line: None,
                source: None,
            }),
        )
        .unwrap();
    let with_debug = runtime
        .new_bytecode_closure(context.realm, &with_debug)
        .unwrap();
    for (key, expected) in keys.iter().zip([
        Value::String(JsString::from_static("no-pc-table.js")),
        Value::Int(0),
        Value::Int(0),
    ]) {
        assert_eq!(
            context.get_property(with_debug.as_object(), key).unwrap(),
            expected
        );
    }

    let without_debug = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let without_debug = runtime
        .new_bytecode_closure(context.realm, &without_debug)
        .unwrap();
    for key in &keys {
        assert_eq!(
            context
                .get_property(without_debug.as_object(), key)
                .unwrap(),
            Value::Undefined
        );
    }
}

#[test]
fn function_bind_and_to_string_use_quickjs_payload_and_source_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let Value::Object(bind_object) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(to_string_object) = context
        .get_property(&function_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

    let authored = "function /*keep*/ named(a, b) { return a + b; }";
    let Value::Object(target_object) = context.eval(&format!("({authored})")).unwrap() else {
        panic!("function source did not evaluate to an object");
    };
    assert_eq!(
        context
            .call(&to_string, Value::Object(target_object.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8(authored).unwrap())
    );
    let name_key = runtime.intern_property_key("name").unwrap();
    assert!(
        context
            .define_own_property(
                &target_object,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from_static(
                        "changed"
                    ))),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(target_object.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8(authored).unwrap()),
        "stored bytecode source must not read the mutable name property"
    );

    let Value::Object(zero_argument_bound) = context
        .call(&bind, Value::Object(target_object.clone()), &[])
        .unwrap()
    else {
        panic!("zero-argument bind did not return an object");
    };
    assert_eq!(
        own_data_value(&runtime, &zero_argument_bound, "length"),
        Value::Int(2)
    );
    assert_eq!(
        own_data_value(&runtime, &zero_argument_bound, "name"),
        Value::String(JsString::from_static("bound changed"))
    );

    let bound = context
        .call(
            &bind,
            Value::Object(target_object.clone()),
            &[Value::Undefined, Value::Int(4)],
        )
        .unwrap();
    let Value::Object(bound_object) = bound else {
        panic!("bind did not return an object");
    };
    let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
    assert_eq!(
        context.call(&bound, Value::Null, &[Value::Int(5)]).unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        runtime.get_prototype_of(&bound_object).unwrap(),
        Some(function_prototype.clone())
    );
    assert_eq!(own_key_names(&runtime, &bound_object), ["length", "name"]);
    assert_eq!(
        own_data_value(&runtime, &bound_object, "length"),
        Value::Int(1)
    );
    assert_eq!(
        own_data_value(&runtime, &bound_object, "name"),
        Value::String(JsString::from_static("bound changed"))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(bound_object.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function bound changed() {\n    [native code]\n}"
        ))
    );

    let Value::Object(sum_target_object) = context
        .eval("(function(a,b,c){return a * 100 + b * 10 + c;})")
        .unwrap()
    else {
        panic!("sum target was not a function");
    };
    let inner = context
        .call(
            &bind,
            Value::Object(sum_target_object),
            &[Value::Undefined, Value::Int(1)],
        )
        .unwrap();
    let Value::Object(inner_object) = inner else {
        panic!("inner bind was not an object");
    };
    let outer = context
        .call(
            &bind,
            Value::Object(inner_object),
            &[Value::Undefined, Value::Int(2)],
        )
        .unwrap();
    let Value::Object(outer_object) = outer else {
        panic!("outer bind was not an object");
    };
    let outer = runtime.as_callable(&outer_object).unwrap().unwrap();
    assert_eq!(
        context.call(&outer, Value::Null, &[Value::Int(3)]).unwrap(),
        Value::Int(123)
    );

    let first_this = context.new_object().unwrap();
    let second_this = context.new_object().unwrap();
    let Value::Object(this_target) = context.eval("(function(){return this;})").unwrap() else {
        panic!("this target was not a function");
    };
    let inner = context
        .call(
            &bind,
            Value::Object(this_target),
            &[Value::Object(first_this.clone())],
        )
        .unwrap();
    let Value::Object(inner) = inner else {
        panic!("bound this function was not an object");
    };
    let rebound = context
        .call(&bind, Value::Object(inner), &[Value::Object(second_this)])
        .unwrap();
    let Value::Object(rebound) = rebound else {
        panic!("rebound function was not an object");
    };
    let rebound = runtime.as_callable(&rebound).unwrap().unwrap();
    assert_eq!(
        context.call(&rebound, Value::Null, &[]).unwrap(),
        Value::Object(first_this)
    );

    let Value::Object(constructor_object) = context
        .eval("(function Constructor(){return new.target;})")
        .unwrap()
    else {
        panic!("constructor target was not a function");
    };
    let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
    let Value::Object(bound_constructor) = context
        .call(
            &bind,
            Value::Object(constructor_object.clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("bound constructor was not an object");
    };
    let bound_constructor = runtime.as_callable(&bound_constructor).unwrap().unwrap();
    assert_eq!(
        context.construct(&bound_constructor, &[]).unwrap(),
        Value::Object(constructor_object)
    );
    let Value::Object(other_object) = context.eval("(function Other(){})").unwrap() else {
        panic!("explicit new target was not a function");
    };
    let other = runtime.as_callable(&other_object).unwrap().unwrap();
    assert_eq!(
        context
            .construct_with_new_target(&bound_constructor, &other, &[])
            .unwrap(),
        Value::Object(other_object)
    );
    drop(constructor);

    assert_eq!(
        context.eval("(function named(){}) + \"\"").unwrap(),
        Value::String(JsString::from_static("function named(){}"))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(function_prototype.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("function () {\n    [native code]\n}"))
    );

    for (function_kind, expected) in [
        (
            crate::heap::FunctionKind::Generator,
            "function *fallback() {\n    [native code]\n}",
        ),
        (
            crate::heap::FunctionKind::Async,
            "async function fallback() {\n    [native code]\n}",
        ),
        (
            crate::heap::FunctionKind::AsyncGenerator,
            "async function *fallback() {\n    [native code]\n}",
        ),
    ] {
        let (code, metadata) = if matches!(
            function_kind,
            crate::heap::FunctionKind::Generator | crate::heap::FunctionKind::AsyncGenerator
        ) {
            (
                vec![
                    Instruction::InitialYield,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                FunctionMetadata {
                    max_stack: 1,
                    function_kind,
                    has_prototype: true,
                    ..FunctionMetadata::default()
                },
            )
        } else {
            (
                vec![Instruction::Undefined, Instruction::Return],
                FunctionMetadata {
                    max_stack: 1,
                    function_kind,
                    ..FunctionMetadata::default()
                },
            )
        };
        let function = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(code, Vec::new(), metadata)
                    .with_name(Some(JsString::from_static("fallback"))),
            )
            .unwrap();
        let callable = runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap();
        assert_eq!(
            context
                .call(&to_string, Value::Object(callable.as_object().clone()), &[],)
                .unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap())
        );
    }

    assert!(
        context
            .define_own_property(
                &to_string_object,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(3)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(to_string_object.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static(
            "function 3() {\n    [native code]\n}"
        ))
    );

    let Value::Object(name_getter_object) = context.eval("(function(){throw \"NAME\";})").unwrap()
    else {
        panic!("name getter was not a function");
    };
    let name_getter = runtime.as_callable(&name_getter_object).unwrap().unwrap();
    let throwing_name = OrdinaryPropertyDescriptor {
        get: DescriptorField::Present(AccessorValue::Callable(name_getter.clone())),
        set: DescriptorField::Present(AccessorValue::Undefined),
        enumerable: DescriptorField::Present(false),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        context
            .define_own_property(&target_object, &name_key, &throwing_name)
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(target_object), &[])
            .unwrap(),
        Value::String(JsString::try_from_utf8(authored).unwrap()),
        "stored bytecode source must bypass a throwing name getter"
    );
    assert!(
        context
            .define_own_property(&to_string_object, &name_key, &throwing_name)
            .unwrap()
    );
    assert_eq!(
        context.call(&to_string, Value::Object(to_string_object), &[],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("NAME")))
    );

    let symbol_name = runtime
        .new_symbol(Some(JsString::from_static("native-name")))
        .unwrap();
    assert!(
        context
            .define_own_property(
                &bind_object,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Symbol(symbol_name)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.call(&to_string, Value::Object(bind_object), &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Symbol function name did not throw an Error object");
    };
    let message_key = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message_key).unwrap(),
        Value::String(JsString::from_static("cannot convert symbol to string"))
    );
}

#[test]
fn bound_function_payload_owns_symbols_and_cycles_across_layout_changes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind_object) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(target_object) = context.eval("(function(value){return value;})").unwrap()
    else {
        panic!("bound payload target was not a function");
    };
    let baseline_atoms = runtime.test_atom_count();
    let symbol = runtime
        .new_symbol(Some(JsString::from_static("bound-payload")))
        .unwrap();
    let Value::Object(bound_object) = context
        .call(
            &bind,
            Value::Object(target_object.clone()),
            &[Value::Undefined, Value::Symbol(symbol.clone())],
        )
        .unwrap()
    else {
        panic!("symbol-bound function was not an object");
    };
    let extra_key = runtime.intern_property_key("bound-extra").unwrap();
    assert!(
        context
            .define_own_property(
                &bound_object,
                &extra_key,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
    drop(symbol);
    let returned = context.call(&bound, Value::Undefined, &[]).unwrap();
    assert!(matches!(returned, Value::Symbol(_)));
    drop(returned);
    drop(bound);
    drop(bound_object);
    drop(extra_key);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    runtime.run_gc().unwrap();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let argument = context.new_object().unwrap();
    let Value::Object(bound_object) = context
        .call(
            &bind,
            Value::Object(target_object.clone()),
            &[Value::Undefined, Value::Object(argument.clone())],
        )
        .unwrap()
    else {
        panic!("cycle-bound function was not an object");
    };
    let back_key = runtime.intern_property_key("bound-back").unwrap();
    assert!(
        set_property(
            &runtime,
            &argument,
            &back_key,
            Value::Object(bound_object.clone()),
        )
        .unwrap()
    );
    drop(bound_object);
    drop(argument);
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    let stats = runtime.run_gc().unwrap();
    assert!(stats.cleanup.finalized_objects >= 2);
    assert!(runtime.as_callable(&target_object).unwrap().is_some());
    assert_eq!(
        runtime.heap_counts().object_nodes,
        baseline_objects,
        "unexpected GC delta: {stats:?}"
    );
}

#[test]
fn bound_function_uses_bind_realm_but_delegates_function_realm_and_has_instance() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_function_prototype = first.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind_object) = first
        .get_property(&first_function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("first realm bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(target_object) = second.eval("(function Target(){})").unwrap() else {
        panic!("second realm target was not a function");
    };

    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let Value::Object(custom_method) = second.eval("(function(value){return value;})").unwrap()
    else {
        panic!("custom hasInstance method was not a function");
    };
    assert!(
        second
            .define_own_property(
                &target_object,
                &has_instance_key,
                &data_descriptor(Value::Object(custom_method), true, false, true),
            )
            .unwrap()
    );

    let Value::Object(bound_object) = first
        .call(&bind, Value::Object(target_object), &[Value::Undefined])
        .unwrap()
    else {
        panic!("cross-realm bound function was not an object");
    };
    let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
    assert_eq!(
        runtime.get_prototype_of(&bound_object).unwrap(),
        Some(first_function_prototype.clone()),
        "bound [[Prototype]] must come from the bind method realm"
    );
    assert_eq!(runtime.callable_realm(&bound).unwrap(), second.realm);

    let Value::Object(nested_bound_object) = first
        .call(
            &bind,
            Value::Object(bound_object.clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("nested cross-realm bound function was not an object");
    };
    let nested_bound = runtime.as_callable(&nested_bound_object).unwrap().unwrap();
    assert_eq!(runtime.callable_realm(&nested_bound).unwrap(), second.realm);

    let Value::Object(has_instance_object) = first
        .get_property(&first_function_prototype, &has_instance_key)
        .unwrap()
    else {
        panic!("Function.prototype[Symbol.hasInstance] was not an object");
    };
    let has_instance = runtime.as_callable(&has_instance_object).unwrap().unwrap();
    assert_eq!(
        first
            .call(
                &has_instance,
                Value::Object(nested_bound_object),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::Bool(true),
        "bound ordinary hasInstance must delegate the primitive candidate to target @@hasInstance"
    );
}

#[test]
fn deep_standard_bound_has_instance_delegation_is_host_stack_safe() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let Value::Object(has_instance_object) = context
        .get_property(&function_prototype, &has_instance_key)
        .unwrap()
    else {
        panic!("Function.prototype[Symbol.hasInstance] was not an object");
    };
    let has_instance = runtime.as_callable(&has_instance_object).unwrap().unwrap();
    let Value::Object(target_object) = context.eval("(function Target(){})").unwrap() else {
        panic!("target was not a function");
    };
    let mut target = runtime.as_callable(&target_object).unwrap().unwrap();

    // Pinned QuickJS still completes at this depth with its default stack
    // budget. The Rust path must preserve that result without recursively
    // consuming the host stack.
    for _ in 0..512 {
        target = runtime
            .new_bound_function(context.realm, &target, &Value::Undefined, &[])
            .unwrap();
    }
    assert_eq!(
        context
            .call(
                &has_instance,
                Value::Object(target.into_object()),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::Bool(false)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn error_intrinsic_graph_and_lazy_methods_match_quickjs_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let aggregate_error = global_callable(&runtime, &mut context, "AggregateError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let is_error_key = runtime.intern_property_key("isError").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let aggregate_key = runtime.intern_property_key("AggregateError").unwrap();

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        writable: false,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error.prototype descriptor did not match QuickJS");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(type_error_prototype),
        writable: false,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("TypeError.prototype descriptor did not match QuickJS");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(aggregate_error_prototype),
        writable: false,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(aggregate_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("AggregateError.prototype descriptor did not match QuickJS");
    };

    let object_count = runtime.heap_counts().object_nodes;
    let realm_strong_count = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();
    assert_eq!(
        own_key_names(&runtime, error.as_object()),
        ["length", "name", "isError", "prototype"]
    );
    assert_eq!(
        own_key_names(&runtime, &error_prototype),
        ["toString", "name", "message", "constructor"]
    );
    assert_eq!(
        own_key_names(&runtime, aggregate_error.as_object()),
        ["length", "name", "prototype"]
    );
    assert_eq!(
        own_key_names(&runtime, &aggregate_error_prototype),
        ["name", "message", "constructor"]
    );
    assert!(
        runtime
            .has_own_property(error.as_object(), &is_error_key)
            .unwrap()
    );
    assert!(
        runtime
            .has_own_property(&error_prototype, &to_string_key)
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, object_count);

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(is_error),
        writable: true,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(error.as_object(), &is_error_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error.isError did not materialize as a native data property");
    };
    assert_eq!(runtime.heap_counts().object_nodes, object_count + 1);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(realm_strong_count)
    );
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(to_string),
        writable: true,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&error_prototype, &to_string_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error.prototype.toString did not materialize as native data");
    };
    assert_eq!(runtime.heap_counts().object_nodes, object_count + 2);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(realm_strong_count)
    );
    assert!(runtime.as_callable(&is_error).unwrap().is_some());
    assert!(runtime.as_callable(&to_string).unwrap().is_some());

    assert!(matches!(
        runtime.get_own_property(&error_prototype, &name_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("Error")
    ));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(realm_strong_count - 1)
    );

    assert_eq!(
        runtime.get_prototype_of(error.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(type_error.as_object()).unwrap(),
        Some(error.as_object().clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&error_prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&type_error_prototype).unwrap(),
        Some(error_prototype.clone())
    );
    assert_eq!(
        runtime
            .get_prototype_of(aggregate_error.as_object())
            .unwrap(),
        Some(error.as_object().clone())
    );
    assert_eq!(
        runtime
            .get_prototype_of(&aggregate_error_prototype)
            .unwrap(),
        Some(error_prototype.clone())
    );
    assert!(matches!(
        runtime
            .get_own_property(&error_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == *error.as_object()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&type_error_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == *type_error.as_object()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&aggregate_error_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == *aggregate_error.as_object()
    ));
    assert!(matches!(
        runtime
            .get_own_property(aggregate_error.as_object(), &length_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    assert!(!runtime.is_error_object(&error_prototype).unwrap());
    assert!(!runtime.is_error_object(&type_error_prototype).unwrap());
    assert!(!runtime.is_error_object(&aggregate_error_prototype).unwrap());
    assert_eq!(
        context
            .get_property(&context.global_object().unwrap(), &aggregate_key)
            .unwrap(),
        Value::Object(aggregate_error.as_object().clone())
    );
}

#[test]
fn error_constructors_to_string_is_error_and_cause_follow_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    let cause_key = runtime.intern_property_key("cause").unwrap();
    let is_error_key = runtime.intern_property_key("isError").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        ..
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error constructor had no object prototype");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(type_error_prototype),
        ..
    } = runtime
        .get_own_property(type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("TypeError constructor had no object prototype");
    };
    let Value::Object(is_error_object) = context
        .get_property(error.as_object(), &is_error_key)
        .unwrap()
    else {
        panic!("Error.isError was not an object");
    };
    let is_error = runtime.as_callable(&is_error_object).unwrap().unwrap();
    let Value::Object(to_string_object) = context
        .get_property(&error_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Error.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

    let Value::Object(empty) = context.call(&error, Value::Undefined, &[]).unwrap() else {
        panic!("Error() did not return an object");
    };
    assert!(runtime.is_error_object(&empty).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&empty).unwrap(),
        Some(error_prototype.clone())
    );
    assert!(!runtime.has_own_property(&empty, &message_key).unwrap());
    assert_eq!(
        context
            .call(&to_string, Value::Object(empty.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("Error"))
    );

    let Value::Object(with_message) = context
        .call(&error, Value::Undefined, &[Value::Int(42)])
        .unwrap()
    else {
        panic!("Error(42) did not return an object");
    };
    assert!(matches!(
        runtime.get_own_property(&with_message, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("42")
    ));
    assert_eq!(
        context
            .call(&to_string, Value::Object(with_message.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("Error: 42"))
    );

    let Value::Object(typed) = context
        .construct(&type_error, &[Value::String(JsString::from_static("boom"))])
        .unwrap()
    else {
        panic!("new TypeError did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&typed).unwrap(),
        Some(type_error_prototype)
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(typed.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("TypeError: boom"))
    );
    assert_eq!(
        context
            .call(&is_error, Value::Undefined, &[Value::Object(typed.clone())],)
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context
            .call(
                &is_error,
                Value::Undefined,
                &[Value::Object(error_prototype.clone())],
            )
            .unwrap(),
        Value::Bool(false)
    );
    let spoof = context.new_object().unwrap();
    assert!(
        runtime
            .set_prototype_of(&spoof, Some(&error_prototype))
            .unwrap()
    );
    assert_eq!(
        context
            .call(&is_error, Value::Undefined, &[Value::Object(spoof)],)
            .unwrap(),
        Value::Bool(false)
    );

    let options = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &options,
                &cause_key,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let Value::Object(with_cause) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(options)],
        )
        .unwrap()
    else {
        panic!("Error(undefined, options) did not return an object");
    };
    assert!(!runtime.has_own_property(&with_cause, &message_key).unwrap());
    assert!(matches!(
        runtime.get_own_property(&with_cause, &cause_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    let inherited_cause_holder = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &inherited_cause_holder,
                &cause_key,
                &data_descriptor(Value::Int(5), true, true, true),
            )
            .unwrap()
    );
    let inherited_options = context.new_object().unwrap();
    assert!(
        runtime
            .set_prototype_of(&inherited_options, Some(&inherited_cause_holder))
            .unwrap()
    );
    let Value::Object(with_inherited_cause) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(inherited_options)],
        )
        .unwrap()
    else {
        panic!("inherited cause Error construction did not return an object");
    };
    assert!(matches!(
        runtime
            .get_own_property(&with_inherited_cause, &cause_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(5),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    assert!(matches!(
        context.call(&to_string, Value::Int(1), &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("non-object Error.prototype.toString throw was not an object");
    };
    assert!(matches!(
        runtime.get_own_property(&exception, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("not an object")
    ));
    assert_eq!(
        own_stack_string(&runtime, &exception),
        JsString::from_static("    at toString (native)\n")
    );
}

#[test]
fn error_stack_eager_capture_matches_quickjs_frames_sites_and_descriptor() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source =
        "(function outer(){ return (function inner(){ return new Error(\"boom\"); })(); })()";
    let Value::Object(error) = context.eval_with_filename(source, "<cmdline>").unwrap() else {
        panic!("nested Error constructor did not return an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static(
            "    at inner (<cmdline>:1:62)\n    at outer (<cmdline>:1:20)\n    at <eval> (<cmdline>:1:80)\n"
        )
    );
    assert_eq!(own_key_names(&runtime, &error), ["message", "stack"]);
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(matches!(
        runtime.get_own_property(&error, &stack_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(_),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    let error_constructor = global_callable(&runtime, &mut context, "Error");
    let Value::Object(direct) = context
        .call(&error_constructor, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("direct Error() did not return an object");
    };
    assert_eq!(own_key_names(&runtime, &direct), ["stack"]);
    assert_eq!(
        own_stack_string(&runtime, &direct),
        JsString::from_static("")
    );
}

#[test]
fn error_constructor_skips_only_itself_and_preserves_other_native_frames() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            probe.as_object(),
            "name",
            Value::String(JsString::from_static("probe")),
            false,
            true,
        )
        .unwrap();
    let probe_key = runtime.intern_property_key("probe").unwrap();
    let global = context.global_object().unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &probe_key,
                &data_descriptor(Value::Object(probe.as_object().clone()), true, true, true,),
            )
            .unwrap()
    );

    let source = "(function viaNative(){ return probe(function callback(){ return new Error(\"x\"); }); })()";
    let Value::Object(error) = context.eval_with_filename(source, "native.js").unwrap() else {
        panic!("native callback did not return an Error");
    };
    let callback_construct = source.find("Error").unwrap() + "Error".len() + 1;
    let outer_return = source.find("return").unwrap() + 1;
    let root_call = source.rfind("()").unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!(
            "    at callback (native.js:1:{callback_construct})\n    at probe (native)\n    at viaNative (native.js:1:{outer_return})\n    at <eval> (native.js:1:{root_call})\n"
        ))
        .unwrap()
    );
}

#[test]
fn native_rethrow_pops_its_frame_before_bytecode_captures_missing_stack() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error_constructor = global_callable(&runtime, &mut context, "Error");
    let Value::Object(error) = context
        .call(&error_constructor, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("Error() did not return an object");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(runtime.delete_property(&error, &stack_key).unwrap());

    let function_prototype = context.function_prototype().unwrap();
    let rethrow = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            rethrow.as_object(),
            "name",
            Value::String(JsString::from_static("rethrowProbe")),
            false,
            true,
        )
        .unwrap();

    assert_eq!(
        context.call(
            &rethrow,
            Value::Undefined,
            &[Value::Object(error.clone()), Value::Bool(false)],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(direct) = context.take_exception().unwrap().unwrap() else {
        panic!("direct native rethrow lost its Error");
    };
    assert_eq!(direct, error);
    assert!(!runtime.has_own_property(&error, &stack_key).unwrap());

    let global = context.global_object().unwrap();
    for (name, value) in [
        ("rethrowProbe", Value::Object(rethrow.as_object().clone())),
        ("heldError", Value::Object(error.clone())),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(&global, &key, &data_descriptor(value, true, true, true),)
                .unwrap()
        );
    }
    let source = "rethrowProbe(heldError, false)";
    assert!(matches!(
        context.eval_with_filename(source, "rethrow.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(from_bytecode) = context.take_exception().unwrap().unwrap() else {
        panic!("bytecode native rethrow lost its Error");
    };
    assert_eq!(from_bytecode, error);
    let call_column = source.find('(').unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!("    at <eval> (rethrow.js:1:{call_column})\n")).unwrap()
    );
}

#[test]
fn vm_error_stack_uses_fault_tail_call_and_root_call_sites() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = "(function outer(){ return (function inner(){ return 1n + 1; })(); })()";
    assert!(matches!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("VM TypeError was not an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static(
            "    at inner (<cmdline>:1:56)\n    at outer (<cmdline>:1:20)\n    at <eval> (<cmdline>:1:69)\n"
        )
    );
    assert_eq!(own_key_names(&runtime, &error), ["message", "stack"]);
}

#[test]
fn syntax_error_stack_prepends_parse_location_and_metadata_in_quickjs_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval_with_filename("1 +", "parse.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("SyntaxError was not an object");
    };
    assert_eq!(
        own_key_names(&runtime, &error),
        ["message", "fileName", "lineNumber", "columnNumber", "stack"]
    );
    assert_eq!(
        own_data_value(&runtime, &error, "fileName"),
        Value::String(JsString::from_static("parse.js"))
    );
    assert_eq!(
        own_data_value(&runtime, &error, "lineNumber"),
        Value::Int(1)
    );
    assert_eq!(
        own_data_value(&runtime, &error, "columnNumber"),
        Value::Int(4)
    );
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static("    at parse.js:1:4\n")
    );
}

#[test]
fn eval_backtrace_barrier_marks_only_the_preexisting_caller_frame() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let caller = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let caller_frame = runtime
        .push_native_active_frame(
            caller.as_object().clone(),
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
            0,
        )
        .unwrap();
    let options = EvalOptions {
        filename: "barrier.js".to_owned(),
        backtrace_barrier: true,
    };

    assert!(matches!(
        context.eval_with_options("1n + 1", &options),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(runtime_error) = context.take_exception().unwrap().unwrap() else {
        panic!("barrier VM error was not an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &runtime_error),
        JsString::from_static("    at <eval> (barrier.js:1:4)\n")
    );
    assert!(
        !runtime
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .unwrap()
            .flags
            .backtrace_barrier
    );

    assert!(matches!(
        context.eval_with_options("1 +", &options),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(parse_error) = context.take_exception().unwrap().unwrap() else {
        panic!("barrier parse error was not an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &parse_error),
        JsString::from_static("    at barrier.js:1:4\n")
    );
    assert!(
        !runtime
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .unwrap()
            .flags
            .backtrace_barrier
    );
    caller_frame.finish().unwrap();
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_script_or_module_name_walks_scripts_modules_and_eval_roots() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(runtime.active_script_or_module_name().unwrap(), None);

    let outer = push_named_script_active_frame(&runtime, &mut context, "outer.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("outer.js"))
    );

    let nested = push_named_script_active_frame(&runtime, &mut context, "nested.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("nested.js"))
    );
    nested.finish().unwrap();

    let module = push_named_module_active_frame(&runtime, &context, "entry.mjs");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("entry.mjs"))
    );
    module.finish().unwrap();

    let direct =
        push_named_eval_active_frame(&runtime, &context, "direct-eval.js", EvalKind::Direct);
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("outer.js"))
    );
    let indirect =
        push_named_eval_active_frame(&runtime, &context, "indirect-eval.js", EvalKind::Indirect);
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("outer.js"))
    );
    indirect.finish().unwrap();
    direct.finish().unwrap();
    outer.finish().unwrap();

    assert_eq!(runtime.active_script_or_module_name().unwrap(), None);
}

#[test]
fn active_script_or_module_name_skips_hidden_native_frames_but_stops_at_visible_native() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let outer = push_named_script_active_frame(&runtime, &mut context, "caller.js");
    let barrier = runtime.install_backtrace_barrier(true).unwrap();
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("caller.js")),
        "backtrace barriers do not delimit ScriptOrModule lookup"
    );

    let function_prototype = context.function_prototype().unwrap();
    let native = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let hidden = runtime
        .push_active_frame(
            native.as_object().clone(),
            None,
            context.realm,
            ActiveFrameFlags {
                backtrace_hidden: true,
                ..ActiveFrameFlags::default()
            },
            ActiveFrameKind::Native {
                target: NativeFunctionId::ActiveFrameProbe,
                actual_arg_count: 0,
                readable_arg_count: 0,
            },
            false,
        )
        .unwrap();
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("caller.js"))
    );
    hidden.finish().unwrap();

    let visible = runtime
        .push_native_active_frame(
            native.as_object().clone(),
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
            0,
        )
        .unwrap();
    assert_eq!(runtime.active_script_or_module_name().unwrap(), None);
    visible.finish().unwrap();

    barrier.finish().unwrap();
    outer.finish().unwrap();
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_script_or_module_name_observes_strip_source_and_strip_debug() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let source_stripped =
        push_named_script_active_frame(&runtime, &mut context, "source-stripped.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("source-stripped.js"))
    );
    source_stripped.finish().unwrap();

    runtime.set_debug_info_mode(DebugInfoMode::Full);
    let outer = push_named_script_active_frame(&runtime, &mut context, "debug-caller.js");
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let debug_stripped =
        push_named_script_active_frame(&runtime, &mut context, "debug-stripped.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        None,
        "a non-eval bytecode frame without debug data is terminal"
    );
    debug_stripped.finish().unwrap();
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("debug-caller.js"))
    );
    outer.finish().unwrap();
}

#[test]
fn backtrace_capture_respects_own_stack_and_real_error_class() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error_constructor = global_callable(&runtime, &mut context, "Error");
    let Value::Object(error) = context
        .call(&error_constructor, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("Error() did not return an object");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(
        runtime
            .define_own_property(
                &error,
                &stack_key,
                &data_descriptor(Value::Undefined, true, false, true),
            )
            .unwrap()
    );

    let held_key = runtime.intern_property_key("heldError").unwrap();
    let global = context.global_object().unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &held_key,
                &data_descriptor(Value::Object(error.clone()), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.eval_with_filename("throw heldError", "throw.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(first_throw) = context.take_exception().unwrap().unwrap() else {
        panic!("held Error throw lost its object identity");
    };
    assert_eq!(first_throw, error);
    assert_eq!(
        own_data_value(&runtime, &first_throw, "stack"),
        Value::Undefined
    );

    assert!(runtime.delete_property(&error, &stack_key).unwrap());
    assert!(matches!(
        context.eval_with_filename("throw heldError", "throw.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(second_throw) = context.take_exception().unwrap().unwrap() else {
        panic!("rethrow lost its Error object");
    };
    assert_eq!(second_throw, error);
    assert_eq!(
        own_stack_string(&runtime, &second_throw),
        JsString::from_static("    at <eval> (throw.js:1:1)\n")
    );

    let Value::Object(error_prototype) =
        own_data_value(&runtime, error_constructor.as_object(), "prototype")
    else {
        panic!("Error.prototype was not an object");
    };
    let spoof = context
        .new_object_with_prototype(Some(&error_prototype))
        .unwrap();
    let spoof_key = runtime.intern_property_key("spoofError").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &spoof_key,
                &data_descriptor(Value::Object(spoof.clone()), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.eval("throw spoofError"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(thrown_spoof) = context.take_exception().unwrap().unwrap() else {
        panic!("ordinary spoof throw lost its object");
    };
    assert_eq!(thrown_spoof, spoof);
    assert!(!runtime.has_own_property(&spoof, &stack_key).unwrap());
}

#[test]
fn backtrace_function_name_lookup_is_raw_and_only_one_prototype_deep() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = "(function leaf(){ return 1n + 1; })";
    let Value::Object(leaf_object) = context.eval_with_filename(source, "name.js").unwrap() else {
        panic!("leaf function was not an object");
    };
    let leaf = runtime.as_callable(&leaf_object).unwrap().unwrap();
    let Value::Object(getter_object) = context
        .eval("(function nameGetter(){ throw \"name getter ran\"; })")
        .unwrap()
    else {
        panic!("name getter was not an object");
    };
    let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    assert!(
        runtime
            .define_own_property(
                leaf.as_object(),
                &name_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(&leaf, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("leaf TypeError was replaced by the name getter");
    };
    let plus_column = source.find('+').unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!("    at <anonymous> (name.js:1:{plus_column})\n"))
            .unwrap()
    );

    for (name, expected) in [
        (
            JsString::try_from_utf16([u16::from(b'a'), 0, u16::from(b'b')]).unwrap(),
            "a",
        ),
        (
            JsString::try_from_utf16([0, u16::from(b'a'), u16::from(b'b')]).unwrap(),
            "<anonymous>",
        ),
    ] {
        assert!(
            runtime
                .define_own_property(
                    leaf.as_object(),
                    &name_key,
                    &data_descriptor(Value::String(name), false, false, true),
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(&leaf, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("renamed leaf TypeError was not an object");
        };
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::try_from_utf8(&format!("    at {expected} (name.js:1:{plus_column})\n"))
                .unwrap()
        );
    }
}

#[test]
fn cross_realm_backtrace_uses_each_bytecode_filename_and_throwing_realm_error() {
    let runtime = Runtime::new();
    let mut realm_a = runtime.new_context();
    let mut realm_b = runtime.new_context();
    let source_a = "(function inA(){ return 1n + 1; })";
    let Value::Object(in_a) = realm_a.eval_with_filename(source_a, "a.js").unwrap() else {
        panic!("realm A function was not an object");
    };
    let global_b = realm_b.global_object().unwrap();
    let in_a_key = runtime.intern_property_key("inA").unwrap();
    assert!(
        realm_b
            .define_own_property(
                &global_b,
                &in_a_key,
                &data_descriptor(Value::Object(in_a), true, true, true),
            )
            .unwrap()
    );

    let source_b = "(function inB(){ return inA(); })()";
    assert!(matches!(
        realm_b.eval_with_filename(source_b, "b.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = realm_b.take_exception().unwrap().unwrap() else {
        panic!("cross-realm TypeError was not an object");
    };
    let plus_column = source_a.find('+').unwrap() + 1;
    let return_column = source_b.find("return").unwrap() + 1;
    let root_call_column = source_b.rfind("()").unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!(
            "    at inA (a.js:1:{plus_column})\n    at inB (b.js:1:{return_column})\n    at <eval> (b.js:1:{root_call_column})\n"
        ))
        .unwrap()
    );

    let type_error_a = global_callable(&runtime, &mut realm_a, "TypeError");
    let Value::Object(type_error_prototype_a) =
        own_data_value(&runtime, type_error_a.as_object(), "prototype")
    else {
        panic!("realm A TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(type_error_prototype_a)
    );
}

#[test]
fn error_constructor_fallback_uses_explicit_new_target_realm() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();
    let type_error = global_callable(&runtime, &mut constructor_context, "TypeError");
    let target_type_error = global_callable(&runtime, &mut target_context, "TypeError");
    let aggregate_error = global_callable(&runtime, &mut constructor_context, "AggregateError");
    let target_aggregate_error = global_callable(&runtime, &mut target_context, "AggregateError");
    let constructor_array = global_callable(&runtime, &mut constructor_context, "Array");
    let target_array = global_callable(&runtime, &mut target_context, "Array");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(target_type_error_prototype),
        ..
    } = runtime
        .get_own_property(target_type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("target-realm TypeError prototype was not an object");
    };
    let Value::Object(new_target_object) = target_context.eval("(0, function(){})").unwrap() else {
        panic!("new.target probe did not produce a function");
    };
    let new_target = runtime.as_callable(&new_target_object).unwrap().unwrap();
    assert!(
        runtime
            .define_own_property(
                &new_target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(instance) = constructor_context
        .construct_with_new_target(&type_error, &new_target, &[])
        .unwrap()
    else {
        panic!("cross-realm TypeError construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(target_type_error_prototype)
    );
    assert!(runtime.is_error_object(&instance).unwrap());

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(target_aggregate_error_prototype),
        ..
    } = runtime
        .get_own_property(target_aggregate_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("target-realm AggregateError prototype was not an object");
    };
    let errors = constructor_context.eval("[1, 2]").unwrap();
    let Value::Object(instance) = constructor_context
        .construct_with_new_target(&aggregate_error, &new_target, &[errors])
        .unwrap()
    else {
        panic!("cross-realm AggregateError construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(target_aggregate_error_prototype)
    );
    assert!(runtime.is_error_object(&instance).unwrap());
    let Value::Object(errors) = own_data_value(&runtime, &instance, "errors") else {
        panic!("cross-realm AggregateError errors was not an object");
    };
    assert!(runtime.is_array_object(&errors).unwrap());
    let Value::Object(constructor_array_prototype) =
        own_data_value(&runtime, constructor_array.as_object(), "prototype")
    else {
        panic!("constructor-realm Array prototype was not an object");
    };
    let Value::Object(target_array_prototype) =
        own_data_value(&runtime, target_array.as_object(), "prototype")
    else {
        panic!("target-realm Array prototype was not an object");
    };
    assert_ne!(constructor_array_prototype, target_array_prototype);
    assert_eq!(
        runtime.get_prototype_of(&errors).unwrap(),
        Some(constructor_array_prototype)
    );
}

#[test]
fn strict_function_name_write_throws_a_type_error_from_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let mut caller_context = runtime.new_context();
    let defining_type_error = global_callable(&runtime, &mut defining_context, "TypeError");
    let caller_type_error = global_callable(&runtime, &mut caller_context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(defining_type_error_prototype),
        ..
    } = runtime
        .get_own_property(defining_type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("defining-realm TypeError prototype was not an object");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(caller_type_error_prototype),
        ..
    } = runtime
        .get_own_property(caller_type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("caller-realm TypeError prototype was not an object");
    };
    assert_ne!(defining_type_error_prototype, caller_type_error_prototype);

    let Value::Object(function) = defining_context
        .eval("(0, function self(){ 'use strict'; self = 1; })")
        .unwrap()
    else {
        panic!("strict named function probe was not an object");
    };
    let function = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller_context.call(&function, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("strict function-name write did not materialize an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(defining_type_error_prototype)
    );
}

#[test]
fn error_constructor_preserves_getter_throw_and_defining_realm_conversion_error() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let mut caller_context = runtime.new_context();
    let error = global_callable(&runtime, &mut defining_context, "Error");
    let type_error = global_callable(&runtime, &mut defining_context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let cause_key = runtime.intern_property_key("cause").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        ..
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("defining-realm Error prototype was not an object");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(type_error_prototype),
        ..
    } = runtime
        .get_own_property(type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("defining-realm TypeError prototype was not an object");
    };

    let Value::Object(cross_realm_error) = caller_context
        .call(&error, Value::Undefined, &[Value::Int(7)])
        .unwrap()
    else {
        panic!("cross-realm Error call did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&cross_realm_error).unwrap(),
        Some(error_prototype)
    );

    let Value::Object(getter_object) = caller_context.eval("(0, function(){ throw 9; })").unwrap()
    else {
        panic!("cause getter probe was not a function");
    };
    let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
    let options = caller_context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &options,
                &cause_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        caller_context.call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(options)],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        caller_context.take_exception().unwrap(),
        Some(Value::Int(9))
    );

    let symbol = runtime.new_symbol(None).unwrap();
    assert!(matches!(
        caller_context.call(&error, Value::Undefined, &[Value::Symbol(symbol)],),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("symbol ToString failure was not an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(type_error_prototype)
    );
}

#[test]
fn object_to_primitive_string_drives_error_message_and_to_string_values() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        ..
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error prototype was not an object");
    };
    let Value::Object(error_to_string_object) = context
        .get_property(&error_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Error.prototype.toString was not an object");
    };
    let error_to_string = runtime
        .as_callable(&error_to_string_object)
        .unwrap()
        .unwrap();

    let ordinary = context.new_object().unwrap();
    let Value::Object(ordinary_error) = context
        .call(&error, Value::Undefined, &[Value::Object(ordinary)])
        .unwrap()
    else {
        panic!("Error(object) did not return an object");
    };
    assert!(matches!(
        runtime
            .get_own_property(&ordinary_error, &message_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("[object Object]")
    ));

    let Value::Object(custom_to_string_object) =
        context.eval("(0, function(){ return 'custom'; })").unwrap()
    else {
        panic!("custom toString probe was not a function");
    };
    let custom = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &custom,
                &to_string_key,
                &data_descriptor(Value::Object(custom_to_string_object), true, true, true,),
            )
            .unwrap()
    );
    let Value::Object(custom_error) = context
        .call(&error, Value::Undefined, &[Value::Object(custom)])
        .unwrap()
    else {
        panic!("Error(custom object) did not return an object");
    };
    assert!(matches!(
        runtime.get_own_property(&custom_error, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("custom")
    ));

    let Value::Object(exotic_method_object) =
        context.eval("(0, function(hint){ return hint; })").unwrap()
    else {
        panic!("@@toPrimitive probe was not a function");
    };
    let exotic = context.new_object().unwrap();
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    assert!(
        runtime
            .define_own_property(
                &exotic,
                &to_primitive,
                &data_descriptor(Value::Object(exotic_method_object), true, true, true),
            )
            .unwrap()
    );
    let Value::Object(exotic_error) = context
        .call(&error, Value::Undefined, &[Value::Object(exotic)])
        .unwrap()
    else {
        panic!("Error(exotic object) did not return an object");
    };
    assert!(matches!(
        runtime.get_own_property(&exotic_error, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("string")
    ));

    let Value::Object(name_conversion_object) =
        context.eval("(0, function(){ return 'N'; })").unwrap()
    else {
        panic!("name conversion probe was not a function");
    };
    let Value::Object(message_conversion_object) =
        context.eval("(0, function(){ return 'M'; })").unwrap()
    else {
        panic!("message conversion probe was not a function");
    };
    let name_value = context.new_object().unwrap();
    let message_value = context.new_object().unwrap();
    for (object, method) in [
        (&name_value, name_conversion_object),
        (&message_value, message_conversion_object),
    ] {
        assert!(
            runtime
                .define_own_property(
                    object,
                    &to_string_key,
                    &data_descriptor(Value::Object(method), true, true, true),
                )
                .unwrap()
        );
    }
    let receiver = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &receiver,
                &name_key,
                &data_descriptor(Value::Object(name_value), true, true, true),
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &receiver,
                &message_key,
                &data_descriptor(Value::Object(message_value), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&error_to_string, Value::Object(receiver), &[],)
            .unwrap(),
        Value::String(JsString::from_static("N: M"))
    );
}

#[test]
fn to_primitive_string_rejects_exotic_failures_and_skips_noncallable_ordinary_methods() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let message_key = runtime.intern_property_key("message").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));

    let noncallable = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &noncallable,
                &to_primitive,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(&error, Value::Undefined, &[Value::Object(noncallable)],),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("noncallable @@toPrimitive did not throw an Error object");
    };
    assert!(matches!(
        runtime.get_own_property(&exception, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("not a function")
    ));

    let Value::Object(object_result_method) = context
        .eval("(0, function(){ return (0, function(){}); })")
        .unwrap()
    else {
        panic!("object-result @@toPrimitive probe was not a function");
    };
    let object_result = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &object_result,
                &to_primitive,
                &data_descriptor(Value::Object(object_result_method), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(&error, Value::Undefined, &[Value::Object(object_result)],),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("object-result @@toPrimitive did not throw an Error object");
    };
    assert!(matches!(
        runtime.get_own_property(&exception, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("toPrimitive")
    ));

    let Value::Object(value_of_method) = context.eval("(0, function(){ return 7; })").unwrap()
    else {
        panic!("valueOf probe was not a function");
    };
    let ordinary = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &ordinary,
                &to_string_key,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &ordinary,
                &value_of_key,
                &data_descriptor(Value::Object(value_of_method), true, true, true),
            )
            .unwrap()
    );
    let Value::Object(converted) = context
        .call(&error, Value::Undefined, &[Value::Object(ordinary)])
        .unwrap()
    else {
        panic!("ordinary valueOf fallback did not create an Error object");
    };
    assert!(matches!(
        runtime.get_own_property(&converted, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("7")
    ));
}

#[test]
fn object_prototype_prefix_methods_are_lazy_and_report_core_tags() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let to_locale_string_key = runtime.intern_property_key("toLocaleString").unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let baseline_objects = runtime.heap_counts().object_nodes;
    assert_eq!(
        own_key_names(&runtime, &object_prototype),
        [
            "toString",
            "toLocaleString",
            "valueOf",
            "hasOwnProperty",
            "isPrototypeOf",
            "propertyIsEnumerable",
            "__proto__",
            "__defineGetter__",
            "__defineSetter__",
            "__lookupGetter__",
            "__lookupSetter__",
            "constructor",
        ]
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

    let Value::Object(to_string_object) = context
        .get_property(&object_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Object.prototype.toString was not an object");
    };
    let Value::Object(to_locale_string_object) = context
        .get_property(&object_prototype, &to_locale_string_key)
        .unwrap()
    else {
        panic!("Object.prototype.toLocaleString was not an object");
    };
    let Value::Object(value_of_object) = context
        .get_property(&object_prototype, &value_of_key)
        .unwrap()
    else {
        panic!("Object.prototype.valueOf was not an object");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 3);
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();
    let to_locale_string = runtime
        .as_callable(&to_locale_string_object)
        .unwrap()
        .unwrap();
    let value_of = runtime.as_callable(&value_of_object).unwrap().unwrap();
    let object = context.new_object().unwrap();
    let function = context.eval("(0, function(){})").unwrap();
    let error = global_callable(&runtime, &mut context, "Error");
    let error = context.call(&error, Value::Undefined, &[]).unwrap();
    for (value, expected) in [
        (Value::Null, "[object Null]"),
        (Value::Undefined, "[object Undefined]"),
        (Value::Object(object.clone()), "[object Object]"),
        (function, "[object Function]"),
        (error, "[object Error]"),
    ] {
        assert_eq!(
            context.call(&to_string, value, &[]).unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap())
        );
    }
    assert_eq!(
        context
            .call(&value_of, Value::Object(object.clone()), &[])
            .unwrap(),
        Value::Object(object.clone())
    );
    assert_eq!(
        context
            .call(&to_locale_string, Value::Object(object), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object Object]"))
    );
}

#[test]
fn object_define_properties_filters_lazy_entries_without_materializing_them() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let lazy_keys = [
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "__defineGetter__",
        "__defineSetter__",
        "__lookupGetter__",
        "__lookupSetter__",
    ]
    .map(|name| runtime.intern_property_key(name).unwrap());
    for key in &lazy_keys {
        assert!(
            runtime
                .is_auto_init_own_property(&object_prototype, key)
                .unwrap()
        );
    }

    let object_constructor = global_callable(&runtime, &mut context, "Object");
    let define_properties = property_callable(
        &runtime,
        &mut context,
        object_constructor.as_object(),
        "defineProperties",
    );
    let target = context.new_object().unwrap();
    assert_eq!(
        context
            .call(
                &define_properties,
                Value::Undefined,
                &[
                    Value::Object(target.clone()),
                    Value::Object(object_prototype.clone()),
                ],
            )
            .unwrap(),
        Value::Object(target.clone())
    );
    assert!(runtime.own_property_keys(&target).unwrap().is_empty());
    for key in &lazy_keys {
        assert!(
            runtime
                .is_auto_init_own_property(&object_prototype, key)
                .unwrap()
        );
    }
}

#[test]
fn native_function_retains_and_dispatches_in_its_defining_realm() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let defining_realm = defining_context.realm;
    let function_prototype = defining_context.function_prototype().unwrap();
    let callable = runtime
        .callable_from_value(Value::Object(function_prototype.clone()))
        .unwrap();

    let before_context_drop = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(defining_realm)
        .unwrap();
    drop(defining_context);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(defining_realm),
        Ok(before_context_drop - 1)
    );
    assert!(matches!(
        runtime.bytecode_for_callable(&callable).unwrap(),
        CallableExecution::Native {
            target: NativeFunctionId::FunctionPrototype,
            realm,
            min_readable_args: 0,
        } if realm == defining_realm
    ));

    let mut caller_context = runtime.new_context();
    assert_eq!(
        caller_context
            .call(&callable, Value::Undefined, &[])
            .unwrap(),
        Value::Undefined
    );

    drop(callable);
    drop(function_prototype);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_err()
    );
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn native_call_preserves_actual_argc_padding_and_restores_active_frame() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let defining_realm = defining_context.realm;
    let function_prototype = defining_context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            defining_realm,
            NativeFunctionId::ArgumentProbe,
            2,
        )
        .unwrap();
    runtime
        .define_function_data_property(probe.as_object(), "length", Value::Int(99), false, true)
        .unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(runtime.delete_property(probe.as_object(), &length).unwrap());
    assert_eq!(
        runtime
            .get_own_property(probe.as_object(), &length)
            .unwrap(),
        None
    );

    let caller_context = runtime.new_context();
    let no_args = runtime
        .call_internal(caller_context.realm, &probe, Value::Undefined, &[])
        .unwrap();
    assert_eq!(
        no_args,
        Completion::Return(Value::String(JsString::from_static("0|2|2|false")))
    );
    let extra_args = runtime
        .call_internal(
            caller_context.realm,
            &probe,
            Value::Undefined,
            &[Value::Int(1), Value::Int(2), Value::Int(3)],
        )
        .unwrap();
    assert_eq!(
        extra_args,
        Completion::Return(Value::String(JsString::from_static("3|3|0|false")))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert_eq!(
        runtime
            .call_internal(
                caller_context.realm,
                &probe,
                Value::Undefined,
                &[Value::Bool(false)],
            )
            .unwrap(),
        Completion::Throw(Value::String(JsString::from_static("native probe throw")))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert!(matches!(
        runtime.call_internal(
            caller_context.realm,
            &probe,
            Value::Undefined,
            &[Value::Bool(true)],
        ),
        Err(RuntimeError::Invariant("native probe engine error"))
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn native_constructor_bit_is_independent_from_generic_cproto() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ArgumentProbe,
            0,
        )
        .unwrap();

    assert!(!runtime.is_constructor(probe.as_object()).unwrap());
    runtime
        .set_constructor_bit(probe.as_object(), true)
        .unwrap();
    assert!(runtime.is_constructor(probe.as_object()).unwrap());
    assert_eq!(
        context.construct(&probe, &[]).unwrap(),
        Value::String(JsString::from_static("0|0|0|true"))
    );
    runtime
        .set_constructor_bit(probe.as_object(), false)
        .unwrap();
    assert!(!runtime.is_constructor(probe.as_object()).unwrap());

    let ordinary = context.new_object().unwrap();
    runtime.set_constructor_bit(&ordinary, true).unwrap();
    assert!(runtime.is_constructor(&ordinary).unwrap());
}

#[test]
fn native_float_cproto_construct_adapter_uses_the_call_kernel() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let math_key = runtime.intern_property_key("Math").unwrap();
    let Value::Object(math) = context.get_property(&global, &math_key).unwrap() else {
        panic!("global Math was not an object");
    };
    let abs = property_callable(&runtime, &mut context, &math, "abs");
    let pow = property_callable(&runtime, &mut context, &math, "pow");

    for function in [&abs, &pow] {
        assert!(!runtime.is_constructor(function.as_object()).unwrap());
        runtime
            .set_constructor_bit(function.as_object(), true)
            .unwrap();
    }
    assert_eq!(
        context.construct(&abs, &[Value::Int(-3)]).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        context
            .construct(&pow, &[Value::Int(2), Value::Int(5)])
            .unwrap(),
        Value::Int(32)
    );
    for function in [&abs, &pow] {
        runtime
            .set_constructor_bit(function.as_object(), false)
            .unwrap();
        assert!(!runtime.is_constructor(function.as_object()).unwrap());
    }
}

#[test]
fn native_constructor_cproto_adapters_use_defining_realm_and_restore_frames() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let defining_realm = defining_context.realm;
    let function_prototype = defining_context.function_prototype().unwrap();
    let constructor_only = runtime
        .new_bound_native_function(
            &function_prototype,
            defining_realm,
            NativeFunctionId::ConstructorProbe,
            0,
        )
        .unwrap();
    let constructor_or_function = runtime
        .new_bound_native_function(
            &function_prototype,
            defining_realm,
            NativeFunctionId::ConstructorOrFunctionProbe,
            0,
        )
        .unwrap();
    let caller_context = runtime.new_context();

    let called_without_new = runtime
        .call_internal(
            caller_context.realm,
            &constructor_only,
            Value::Undefined,
            &[],
        )
        .unwrap();
    let Completion::Throw(Value::Object(exception)) = called_without_new else {
        panic!("constructor-only native did not throw an object");
    };
    let defining_type_error_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(defining_realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();
    assert_eq!(
        runtime
            .get_prototype_of(&exception)
            .unwrap()
            .unwrap()
            .object_id(),
        defining_type_error_prototype
    );
    let message = runtime.intern_property_key("message").unwrap();
    assert!(matches!(
        runtime.get_own_property(&exception, &message).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("must be called with new")
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert_eq!(
        runtime
            .construct_internal(
                caller_context.realm,
                &constructor_only,
                &constructor_only,
                &[],
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("0|0|0|true")))
    );
    assert_eq!(
        runtime
            .call_internal(
                caller_context.realm,
                &constructor_or_function,
                Value::Undefined,
                &[],
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("0|0|0|false")))
    );
    assert_eq!(
        runtime
            .construct_internal(
                caller_context.realm,
                &constructor_or_function,
                &constructor_or_function,
                &[],
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("0|0|0|true")))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn unified_active_frames_preserve_order_caller_pc_and_defining_realms() {
    let runtime = Runtime::new();
    let outer_context = runtime.new_context();
    let native_context = runtime.new_context();
    let callback_context = runtime.new_context();

    let function_prototype = native_context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            native_context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let callback = bytecode_callable(
        &runtime,
        &callback_context,
        vec![
            Instruction::GetArg(0),
            Instruction::Call(0),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: false,
            ..FunctionMetadata::default()
        },
    );
    let outer = bytecode_callable(
        &runtime,
        &outer_context,
        vec![
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::Call(1),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let (outer_bytecode, callback_bytecode) = {
        let state = runtime.0.state.borrow();
        let ObjectPayload::BytecodeFunction { bytecode, .. } = &state
            .heap
            .object(outer.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("outer probe caller was not bytecode");
        };
        let outer_bytecode = *bytecode;
        let ObjectPayload::BytecodeFunction { bytecode, .. } = &state
            .heap
            .object(callback.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("probe callback was not bytecode");
        };
        (outer_bytecode, *bytecode)
    };

    assert_eq!(
        runtime
            .call_internal(
                outer_context.realm,
                &outer,
                Value::Undefined,
                &[
                    Value::Object(probe.as_object().clone()),
                    Value::Object(callback.as_object().clone()),
                ],
            )
            .unwrap(),
        Completion::Return(Value::Undefined)
    );

    let snapshot = runtime
        .0
        .state
        .borrow_mut()
        .active_frame_probe_snapshots
        .pop()
        .expect("deep native probe should capture the active chain");
    assert_eq!(snapshot.len(), 4);
    assert_eq!(snapshot[0].function, outer.as_object().object_id());
    assert_eq!(snapshot[0].realm, outer_context.realm);
    assert!(snapshot[0].flags.strict);
    assert!(matches!(
        snapshot[0].kind,
        ActiveFrameKind::Bytecode {
            bytecode,
            pc: Some(pc),
        } if bytecode == outer_bytecode && pc.index() == 2
    ));
    assert_eq!(snapshot[1].function, probe.as_object().object_id());
    assert_eq!(snapshot[1].realm, native_context.realm);
    assert!(matches!(
        snapshot[1].kind,
        ActiveFrameKind::Native {
            target: NativeFunctionId::ActiveFrameProbe,
            actual_arg_count: 1,
            readable_arg_count: 1,
        }
    ));
    assert_eq!(snapshot[2].function, callback.as_object().object_id());
    assert_eq!(snapshot[2].realm, callback_context.realm);
    assert!(!snapshot[2].flags.strict);
    assert!(matches!(
        snapshot[2].kind,
        ActiveFrameKind::Bytecode {
            bytecode,
            pc: Some(pc),
        } if bytecode == callback_bytecode && pc.index() == 1
    ));
    assert_eq!(snapshot[3].function, probe.as_object().object_id());
    assert_eq!(snapshot[3].realm, native_context.realm);
    assert!(matches!(
        snapshot[3].kind,
        ActiveFrameKind::Native {
            target: NativeFunctionId::ActiveFrameProbe,
            actual_arg_count: 0,
            readable_arg_count: 0,
        }
    ));
    assert!(
        snapshot
            .windows(2)
            .all(|frames| frames[0].token != frames[1].token)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn unified_active_frames_restore_after_return_throw_and_engine_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let no_argument_call = bytecode_callable(
        &runtime,
        &context,
        vec![
            Instruction::GetArg(0),
            Instruction::Call(0),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let command_call = bytecode_callable(
        &runtime,
        &context,
        vec![
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::Call(1),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );

    assert_eq!(
        context
            .call(
                &no_argument_call,
                Value::Undefined,
                &[Value::Object(probe.as_object().clone())],
            )
            .unwrap(),
        Value::Undefined
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert_eq!(
        context.call(
            &command_call,
            Value::Undefined,
            &[Value::Object(probe.as_object().clone()), Value::Bool(false),],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static(
            "active frame probe throw"
        )))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert!(matches!(
        context.call(
            &command_call,
            Value::Undefined,
            &[
                Value::Object(probe.as_object().clone()),
                Value::Bool(true),
            ],
        ),
        Err(RuntimeError::Engine(error))
            if error.message().contains("active frame probe engine error")
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_frame_guard_roots_function_and_bytecode_through_gc_and_drop_fallback() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let bytecode = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let bytecode_id = bytecode.bytecode_id();
    let callable = runtime
        .new_bytecode_closure(context.realm, &bytecode)
        .unwrap();
    let function_id = callable.as_object().object_id();
    let guard = runtime
        .push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            context.realm,
            true,
        )
        .unwrap();
    drop(callable);
    drop(bytecode);

    assert_eq!(runtime.run_gc().unwrap().cleanup.finalized_objects, 0);
    {
        let state = runtime.0.state.borrow();
        assert!(state.heap.object(function_id).is_ok());
        assert!(state.heap.function_bytecode(bytecode_id).is_ok());
        assert_eq!(state.active_frames.len(), 1);
    }

    drop(guard);
    assert!(runtime.0.state.borrow().active_frames.is_empty());
    assert!(runtime.0.state.borrow().heap.object(function_id).is_err());
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(bytecode_id)
            .is_err()
    );
}

#[test]
fn bytecode_active_frame_rejects_a_realm_other_than_the_bytecode_realm() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let other_context = runtime.new_context();
    let bytecode = runtime
        .publish_unlinked_function(
            defining_context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(defining_context.realm, &bytecode)
        .unwrap();

    assert!(matches!(
        runtime.push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            other_context.realm,
            true,
        ),
        Err(RuntimeError::Invariant(
            "bytecode active frame realm disagrees with its bytecode"
        ))
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_frame_drop_defers_nested_pops_until_the_state_borrow_ends() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let bytecode = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let bytecode_id = bytecode.bytecode_id();
    let callable = runtime
        .new_bytecode_closure(context.realm, &bytecode)
        .unwrap();
    let function_id = callable.as_object().object_id();
    let outer_guard = runtime
        .push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            context.realm,
            true,
        )
        .unwrap();
    let outer_token = outer_guard.token();
    let inner_guard = runtime
        .push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            context.realm,
            true,
        )
        .unwrap();
    let inner_token = inner_guard.token();
    drop(callable);
    drop(bytecode);

    let state_borrow = runtime.0.state.borrow();
    drop(inner_guard);
    drop(outer_guard);

    // The state borrow forces both guard drops through the deferred path.
    // `push_front` reverses unwind order so the outer pop removes the whole
    // nested suffix before either guard releases its raw heap roots.
    assert_eq!(state_borrow.active_frames.len(), 2);
    let deferred = runtime.0.deferred_references.borrow();
    let frame_pops = deferred
        .iter()
        .filter_map(|operation| match operation {
            DeferredRefOp::ActiveFramePop { token, .. } => Some(*token),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(frame_pops, vec![outer_token, inner_token]);
    assert!(matches!(
        deferred.front(),
        Some(DeferredRefOp::ActiveFramePop { token, .. }) if *token == outer_token
    ));
    drop(deferred);
    drop(state_borrow);

    runtime.drain_deferred_references().unwrap();
    assert!(runtime.0.deferred_references.borrow().is_empty());
    let state = runtime.0.state.borrow();
    assert!(state.active_frames.is_empty());
    assert!(state.heap.object(function_id).is_err());
    assert!(state.heap.function_bytecode(bytecode_id).is_err());
}

#[test]
fn fclosure_call_and_call_method_follow_quickjs_stack_layout() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let add_one = UnlinkedFunction::new(
        vec![
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let caller = UnlinkedFunction::new(
        vec![
            Instruction::FClosure(0),
            Instruction::PushI32(41),
            Instruction::Call(1),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(add_one)],
        FunctionMetadata {
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let caller = runtime
        .publish_unlinked_function(context.realm, caller)
        .unwrap();
    let caller = runtime
        .new_bytecode_closure(context.realm, &caller)
        .unwrap();
    assert_eq!(
        context.call(&caller, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );

    let return_this = UnlinkedFunction::new(
        vec![Instruction::PushThis, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let method_caller = UnlinkedFunction::new(
        vec![
            Instruction::PushThis,
            Instruction::FClosure(0),
            Instruction::CallMethod(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(return_this)],
        FunctionMetadata {
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let method_caller = runtime
        .publish_unlinked_function(context.realm, method_caller)
        .unwrap();
    let method_caller = runtime
        .new_bytecode_closure(context.realm, &method_caller)
        .unwrap();
    let receiver = runtime.new_object(None).unwrap();
    assert_eq!(
        context
            .call(&method_caller, Value::Object(receiver.clone()), &[])
            .unwrap(),
        Value::Object(receiver)
    );
}

#[test]
fn nested_call_propagates_throw_without_publishing_it_early() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let throwing = UnlinkedFunction::new(
        vec![Instruction::PushI32(9), Instruction::Throw],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let caller = UnlinkedFunction::new(
        vec![
            Instruction::FClosure(0),
            Instruction::Call(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(throwing)],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let caller = runtime
        .publish_unlinked_function(context.realm, caller)
        .unwrap();
    let caller = runtime
        .new_bytecode_closure(context.realm, &caller)
        .unwrap();

    assert_eq!(
        context.call(&caller, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
}

#[test]
fn published_tail_call_uses_backtrace_and_current_activation_catch_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let wrapper = UnlinkedFunction::new(
        vec![Instruction::GetArg(0), Instruction::TailCall(0)],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_debug(UnlinkedFunctionDebug {
        filename: JsString::from_static("tail-wrapper.js"),
        pc2line: Some(Pc2LineTable::new(
            LineColumn::new(0, 0),
            vec![Pc2LineEntry {
                pc: 1,
                position: LineColumn::new(2, 4),
            }],
        )),
        source: None,
    });
    let wrapper = runtime
        .publish_unlinked_function(context.realm, wrapper)
        .unwrap();
    let wrapper = runtime
        .new_bytecode_closure(context.realm, &wrapper)
        .unwrap();
    let target = context
        .eval_with_filename(
            "(function tailTarget(){ return 1n + 1; })",
            "tail-target.js",
        )
        .unwrap();
    assert_eq!(
        context.call(&wrapper, Value::Undefined, &[target]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("tail target TypeError was not an object");
    };
    let stack = own_stack_string(&runtime, &error).to_utf8_lossy();
    assert!(stack.contains("tail-target.js:"), "{stack}");
    assert!(stack.contains("tail-wrapper.js:3:5"), "{stack}");
    assert!(!context.has_exception());

    let throwing = UnlinkedFunction::new(
        vec![Instruction::PushI32(17), Instruction::Throw],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let catcher = UnlinkedFunction::new(
        vec![
            Instruction::Catch(4),
            Instruction::FClosure(0),
            Instruction::TailCall(0),
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(throwing)],
        FunctionMetadata {
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let catcher = runtime
        .publish_unlinked_function(context.realm, catcher)
        .unwrap();
    let catcher = runtime
        .new_bytecode_closure(context.realm, &catcher)
        .unwrap();
    assert_eq!(
        context.call(&catcher, Value::Undefined, &[]).unwrap(),
        Value::Int(17)
    );
    assert!(!context.has_exception());
}

#[test]
fn push_this_applies_strict_and_sloppy_callee_realm_rules() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let code = vec![Instruction::PushThis, Instruction::Return];

    let sloppy = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                code.clone(),
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let sloppy = runtime
        .new_bytecode_closure(context.realm, &sloppy)
        .unwrap();
    assert_eq!(
        context.call(&sloppy, Value::Undefined, &[]).unwrap(),
        Value::Object(global)
    );
    let boxed_number = context.call(&sloppy, Value::Int(1), &[]).unwrap();
    let Value::Object(boxed_number) = boxed_number else {
        panic!("sloppy Number this did not escape as a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed_number).unwrap(),
        Some(context.number_prototype().unwrap())
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(boxed_number.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value)) if *value == 1.0
    ));
    let ignores_this = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                vec![Instruction::PushI32(7), Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let ignores_this = runtime
        .new_bytecode_closure(context.realm, &ignores_this)
        .unwrap();
    assert_eq!(
        context.call(&ignores_this, Value::Int(1), &[]).unwrap(),
        Value::Int(7)
    );

    let strict = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                code,
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let strict = runtime
        .new_bytecode_closure(context.realm, &strict)
        .unwrap();
    assert_eq!(
        context.call(&strict, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context.call(&strict, Value::Int(1), &[]).unwrap(),
        Value::Int(1)
    );
}

#[test]
fn context_invokes_getters_and_setters_with_the_original_receiver() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = runtime.new_object(None).unwrap();
    let child = runtime.new_object(Some(&prototype)).unwrap();
    let explicit_receiver = runtime.new_object(None).unwrap();

    let getter_key = runtime.intern_property_key("getter").unwrap();
    let getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &getter_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&child, &getter_key).unwrap(),
        Value::Object(child.clone())
    );
    assert_eq!(
        context
            .get_property_with_receiver(
                &prototype,
                &getter_key,
                Value::Object(explicit_receiver.clone())
            )
            .unwrap(),
        Value::Object(explicit_receiver)
    );

    let setter_key = runtime.intern_property_key("setter").unwrap();
    let setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushFalse, Instruction::Return],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &setter_key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert!(
        context
            .set_property(&child, &setter_key, Value::Int(7))
            .unwrap()
    );

    let throwing_key = runtime.intern_property_key("throwing-setter").unwrap();
    let throwing_setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::GetArg(0), Instruction::Throw],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &throwing_key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(throwing_setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert_eq!(
        context.set_property(&child, &throwing_key, Value::Int(9)),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));

    let faulting_key = runtime.intern_property_key("faulting-setter").unwrap();
    let faulting_setter = bytecode_callable(
        &runtime,
        &context,
        vec![
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &faulting_key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(faulting_setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert_eq!(
        context.set_property(&child, &faulting_key, Value::BigInt(JsBigInt::one())),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("expected setter TypeError");
    };
    assert!(runtime.is_error_object(&error).unwrap());
}

#[test]
fn prepared_getter_action_keeps_callable_alive_after_property_deletion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("x").unwrap();
    let getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushI32(42), Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );

    let action = runtime.prepare_get_property(&object, &key).unwrap();
    assert!(runtime.delete_property(&object, &key).unwrap());
    let PropertyGetAction::Call { getter, receiver } = action else {
        panic!("expected a rooted getter action");
    };
    assert_eq!(
        context.call(&getter, receiver, &[]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.get_property(&object, &key).unwrap(),
        Value::Undefined
    );
}

#[test]
fn prepared_setter_action_roots_callable_receiver_and_argument() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let argument = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("x").unwrap();
    let setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::GetArg(0), Instruction::Return],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );

    let action = runtime
        .prepare_set_property(&object, &key, Value::Object(argument.clone()))
        .unwrap();
    assert!(runtime.delete_property(&object, &key).unwrap());
    drop(argument);
    let super::PropertySetAction::Call {
        setter,
        receiver,
        argument,
    } = action
    else {
        panic!("expected a rooted setter action");
    };
    let returned = context.call(&setter, receiver, &[argument]).unwrap();
    assert!(matches!(returned, Value::Object(_)));
    assert_eq!(
        context.get_property(&object, &key).unwrap(),
        Value::Undefined
    );
}

#[test]
fn published_lexical_locals_use_named_tdz_errors_and_checked_mutation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let lexical = |code, is_const, max_stack| {
        UnlinkedFunction::new(
            code,
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("namedLexical")),
                is_const,
            )],
        )
    };

    let tdz = runtime
        .publish_unlinked_function(
            context.realm,
            lexical(
                vec![Instruction::GetLocalCheck(0), Instruction::Return],
                false,
                1,
            ),
        )
        .unwrap();
    let tdz = runtime.new_bytecode_closure(context.realm, &tdz).unwrap();
    assert!(matches!(
        context.call(&tdz, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("lexical TDZ did not throw an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("namedLexical is not initialized"))
    );

    let mutable = runtime
        .publish_unlinked_function(
            context.realm,
            lexical(
                vec![
                    Instruction::SetLocalUninitialized(0),
                    Instruction::PushI32(40),
                    Instruction::InitializeLocal(0),
                    Instruction::PushI32(42),
                    Instruction::SetLocalCheck(0),
                    Instruction::Return,
                ],
                false,
                1,
            ),
        )
        .unwrap();
    let mutable = runtime
        .new_bytecode_closure(context.realm, &mutable)
        .unwrap();
    assert_eq!(
        context.call(&mutable, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );

    let plain_put = runtime
        .publish_unlinked_function(
            context.realm,
            lexical(
                vec![
                    Instruction::PushI32(1),
                    Instruction::InitializeLocal(0),
                    Instruction::PushI32(2),
                    Instruction::InitializeLocal(0),
                    Instruction::GetLocalCheck(0),
                    Instruction::Return,
                ],
                false,
                1,
            ),
        )
        .unwrap();
    let plain_put = runtime
        .new_bytecode_closure(context.realm, &plain_put)
        .unwrap();
    assert_eq!(
        context.call(&plain_put, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn published_exception_regions_catch_native_callee_and_accessor_throws() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let native_error = UnlinkedFunction::new(
        vec![
            Instruction::Catch(5),
            Instruction::Null,
            Instruction::GetField(0),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("field"))).unwrap()],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let native_error = runtime
        .publish_unlinked_function(context.realm, native_error)
        .unwrap();
    let native_error = runtime
        .new_bytecode_closure(context.realm, &native_error)
        .unwrap();
    let Value::Object(error) = context.call(&native_error, Value::Undefined, &[]).unwrap() else {
        panic!("caught native throw was not an Error object");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    let stack = runtime.intern_property_key("stack").unwrap();
    assert!(matches!(
        context.get_property(&error, &stack).unwrap(),
        Value::String(_)
    ));

    let child = UnlinkedFunction::new(
        vec![Instruction::PushI32(17), Instruction::Throw],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let caller = UnlinkedFunction::new(
        vec![
            Instruction::Catch(5),
            Instruction::FClosure(0),
            Instruction::Call(0),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let caller = runtime
        .publish_unlinked_function(context.realm, caller)
        .unwrap();
    let caller = runtime
        .new_bytecode_closure(context.realm, &caller)
        .unwrap();
    assert_eq!(
        context.call(&caller, Value::Undefined, &[]).unwrap(),
        Value::Int(17)
    );

    let Value::Object(getter) = context.eval("(function(){throw 23})").unwrap() else {
        panic!("accessor getter was not callable");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let global = context.global_object().unwrap();
    let accessor_name = runtime.intern_property_key("__caught_accessor").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &accessor_name,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let accessor = UnlinkedFunction::new_with_closure_variables(
        vec![
            Instruction::Catch(4),
            Instruction::GetVar(0),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("__caught_accessor")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let accessor = runtime
        .publish_unlinked_function(context.realm, accessor)
        .unwrap();
    let accessor = runtime
        .new_bytecode_closure(context.realm, &accessor)
        .unwrap();
    assert_eq!(
        context.call(&accessor, Value::Undefined, &[]).unwrap(),
        Value::Int(23)
    );
}

#[test]
fn publication_rejects_malformed_exception_and_gosub_regions() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    for code in [
        vec![
            Instruction::Catch(99),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Gosub(99),
        ],
        vec![Instruction::PushI32(0), Instruction::Ret],
        vec![
            Instruction::Gosub(3),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
    ] {
        let malformed = UnlinkedFunction::new(
            code,
            Vec::new(),
            FunctionMetadata {
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, malformed),
            Err(RuntimeError::Engine(_))
        ));
    }
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
}

#[test]
fn caught_throw_reuses_captured_lexical_without_close_local() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("exceptionReused")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(1),
            Instruction::Catch(9),
            Instruction::PushI32(0),
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Drop,
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(2),
            Instruction::GetLocal(1),
            Instruction::Call(0),
            Instruction::PushConst(1),
            Instruction::Add,
            Instruction::GetLocal(2),
            Instruction::Call(0),
            Instruction::Add,
            Instruction::PushConst(1),
            Instruction::Add,
            Instruction::GetLocal(1),
            Instruction::GetLocal(2),
            Instruction::StrictEq,
            Instruction::Add,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::child(child),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("|"))).unwrap(),
        ],
        FunctionMetadata {
            local_count: 3,
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("exceptionReused")),
                false,
            ),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("first"))),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("second"))),
        ],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    assert_eq!(
        context.call(&parent, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::from_static("2|2|false"))
    );
}

#[test]
fn finally_overridden_return_reuses_captured_lexical_without_close_local() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = concat!(
        "(function(){var f,g,i=0;while(i<2){i++;try{{let x=i;",
        "if(i===1)f=function(){return x};else g=function(){return x};",
        "return 9}}finally{continue}}return f()+'|'+g()})()"
    );

    assert_eq!(
        context.eval(source).unwrap(),
        Value::String(JsString::from_static("2|2"))
    );
}

#[test]
fn close_local_detaches_a_captured_uninitialized_lexical_lifetime() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("closedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::CloseLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::PushI32(3),
            Instruction::InitializeLocal(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("closedLexical")),
            false,
        )],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    assert!(runtime.test_atom_count() > baseline_atoms);
    let parent_callable = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    drop(parent);
    let Value::Object(child_object) = context
        .call(&parent_callable, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("parent did not return its captured child closure");
    };
    drop(parent_callable);
    let child_callable = runtime.as_callable(&child_object).unwrap().unwrap();
    drop(child_object);
    assert_eq!(
        context
            .call(&child_callable, Value::Undefined, &[])
            .unwrap(),
        Value::Int(1)
    );
    drop(child_callable);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn lexical_scope_entry_rejects_an_initialized_capture_without_close_local() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::Undefined, Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("reenteredLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::Undefined,
            Instruction::Gosub(11),
            Instruction::Drop,
            Instruction::SetLocalUninitialized(0),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Ret,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("reenteredLexical")),
            false,
        )],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let RuntimeError::Engine(error) = context
        .call(&parent, Value::Undefined, &[])
        .expect_err("initialized captured lifetime was accepted without CloseLocal")
    else {
        panic!("initialized captured lifetime did not report an engine invariant");
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(
        error.message(),
        "captured local entered a new lexical lifetime before CloseLocal"
    );
}

#[test]
fn close_local_exposes_direct_and_fresh_captured_plain_put_values() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("observedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(1),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::CloseLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::GetLocalCheck(0),
            Instruction::PutLocal(3),
            Instruction::FClosure(0),
            Instruction::PutLocal(2),
            Instruction::PushI32(3),
            Instruction::InitializeLocal(0),
            Instruction::GetLocal(3),
            Instruction::PushI32(100),
            Instruction::Mul,
            Instruction::GetLocal(1),
            Instruction::Call(0),
            Instruction::PushI32(10),
            Instruction::Mul,
            Instruction::Add,
            Instruction::GetLocal(2),
            Instruction::Call(0),
            Instruction::Add,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 4,
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("observedLexical")),
                false,
            ),
            UnlinkedVariableDefinition::ordinary(None),
            UnlinkedVariableDefinition::ordinary(None),
            UnlinkedVariableDefinition::ordinary(None),
        ],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();

    // Hundreds: the Direct value after CloseLocal. Tens: the old cell.
    // Ones: the fresh captured cell after a second plain initialization.
    assert_eq!(
        context.call(&parent, Value::Undefined, &[]).unwrap(),
        Value::Int(213)
    );
}

#[test]
fn escaped_uninitialized_lexical_cell_keeps_its_named_tdz() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("escapedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("escapedLexical")),
            false,
        )],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let Value::Object(child) = context.call(&parent, Value::Undefined, &[]).unwrap() else {
        panic!("parent did not return its uninitialized lexical closure");
    };
    let child = runtime.as_callable(&child).unwrap().unwrap();
    assert!(matches!(
        context.call(&child, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("escaped lexical TDZ did not throw an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("escapedLexical is not initialized"))
    );
}

#[test]
fn named_ordinary_definitions_capture_through_unnamed_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::PushI32(7),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_variable_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("ordinaryCapture"),
        ))],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let Value::Object(child) = context.call(&parent, Value::Undefined, &[]).unwrap() else {
        panic!("parent did not return its child closure");
    };
    let child = runtime.as_callable(&child).unwrap().unwrap();
    assert_eq!(
        context.call(&child, Value::Undefined, &[]).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn publication_rejects_vardef_and_checked_opcode_mismatches_before_allocation() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let reject = |function| {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    };

    reject(
        UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(Vec::new(), Vec::new()),
    );
    reject(
        UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("rejectedLexicalArgument")),
                false,
            )],
            Vec::new(),
        ),
    );
    reject(
        UnlinkedFunction::new(
            vec![Instruction::GetLocal(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("rejectedLexicalRead")),
                false,
            )],
        ),
    );
    reject(
        UnlinkedFunction::new(
            vec![Instruction::GetLocalCheck(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("rejectedOrdinaryRead"),
            ))],
        ),
    );
    reject(
        UnlinkedFunction::new(
            vec![
                Instruction::PushI32(1),
                Instruction::PutLocalCheck(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("rejectedConstWrite")),
                true,
            )],
        ),
    );

    let mismatched_child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static(
                "differentLexicalName",
            )))
            .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    reject(
        UnlinkedFunction::new(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(mismatched_child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("expectedLexicalName")),
                false,
            )],
        ),
    );
}

#[test]
fn publication_rejects_out_of_range_frame_operands() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let function = UnlinkedFunction::new(
        vec![Instruction::GetArg(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

    let function = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            local_count: u16::MAX,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

    let function = UnlinkedFunction::new(
        vec![Instruction::DeleteVar(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

    let function = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            function_name_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_malformed_function_name_metadata_and_writes() {
    fn descriptor(
        source: ClosureSource,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> ClosureVariable {
        ClosureVariable {
            source,
            name: ClosureVariableName::None,
            is_lexical,
            is_const,
            kind,
        }
    }

    fn child(code: Vec<Instruction>, mut descriptor: ClosureVariable) -> UnlinkedFunction {
        let constants = if descriptor.kind == ClosureVariableKind::FunctionName {
            descriptor.name = ClosureVariableName::Constant(0);
            vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("self"))).unwrap()]
        } else {
            Vec::new()
        };
        UnlinkedFunction::new_with_closure_variables(
            code,
            constants,
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![descriptor],
        )
    }

    fn parent(
        child: UnlinkedFunction,
        metadata: FunctionMetadata,
        name: Option<&str>,
    ) -> UnlinkedFunction {
        let function = UnlinkedFunction::new(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            metadata,
        );
        function.with_name(name.map(|name| JsString::try_from_utf8(name).unwrap()))
    }

    fn named_parent(child: UnlinkedFunction, strict: bool) -> UnlinkedFunction {
        parent(
            child,
            FunctionMetadata {
                local_count: 1,
                function_name_local: Some(0),
                max_stack: 1,
                strict,
                ..FunctionMetadata::default()
            },
            Some("self"),
        )
    }

    let runtime = Runtime::new();
    let context = runtime.new_context();
    let reject = |function| {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    };

    for name in [None, Some("")] {
        reject(
            UnlinkedFunction::new(
                vec![Instruction::GetLocal(0), Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    local_count: 1,
                    function_name_local: Some(0),
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_name(name.map(|name| JsString::try_from_utf8(name).unwrap())),
        );
    }

    reject(parent(
        child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentArgument(0),
                false,
                false,
                ClosureVariableKind::FunctionName,
            ),
        ),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        None,
    ));
    reject(parent(
        child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentLocal(0),
                false,
                false,
                ClosureVariableKind::FunctionName,
            ),
        ),
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        None,
    ));
    reject(named_parent(
        child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentLocal(0),
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        ),
        false,
    ));
    reject(named_parent(
        UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("other"))).unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::FunctionName,
            }],
        ),
        false,
    ));

    for (strict, is_lexical, is_const) in [
        (true, false, false),
        (false, false, true),
        (false, true, false),
    ] {
        reject(named_parent(
            child(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                descriptor(
                    ClosureSource::ParentLocal(0),
                    is_lexical,
                    is_const,
                    ClosureVariableKind::FunctionName,
                ),
            ),
            strict,
        ));
    }

    let inner = child(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        descriptor(
            ClosureSource::ParentClosure(0),
            false,
            false,
            ClosureVariableKind::Normal,
        ),
    );
    let middle = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![
            UnlinkedConstant::child(inner),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("self"))).unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(1),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::FunctionName,
        }],
    );
    reject(named_parent(middle, false));

    for code in [
        vec![
            Instruction::PushI32(1),
            Instruction::PutLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::SetLocal(0),
            Instruction::Return,
        ],
    ] {
        reject(
            UnlinkedFunction::new(
                code,
                Vec::new(),
                FunctionMetadata {
                    local_count: 1,
                    function_name_local: Some(0),
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_name(Some(JsString::from_static("self"))),
        );
    }

    for code in [
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarRef(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::SetVarRef(0),
            Instruction::Return,
        ],
    ] {
        reject(named_parent(
            child(
                code,
                descriptor(
                    ClosureSource::ParentLocal(0),
                    false,
                    false,
                    ClosureVariableKind::FunctionName,
                ),
            ),
            false,
        ));
    }
}

#[test]
fn publication_rejects_mixed_global_and_lexical_closure_opcodes() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let reject = |function| {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    };

    for code in [
        vec![Instruction::GetVar(0), Instruction::Return],
        vec![Instruction::GetVarUndef(0), Instruction::Return],
        vec![Instruction::DeleteVar(0), Instruction::Return],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVar(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarInit(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
    ] {
        let child = UnlinkedFunction::new_with_closure_variables(
            code,
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        reject(UnlinkedFunction::new(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        ));
    }

    for code in [
        vec![Instruction::GetVarRef(0), Instruction::Return],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarRef(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::SetVarRef(0),
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarInit(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
    ] {
        reject(UnlinkedFunction::new_with_closure_variables(
            code,
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("global")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::Global,
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        ));
    }
}

#[test]
fn publication_rejects_duplicate_lexical_global_declaration_descriptors() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let descriptor = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let function = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::Undefined, Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("duplicate"))).unwrap(),
        ],
        FunctionMetadata {
            closure_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![descriptor, descriptor],
    );

    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_accepts_duplicate_program_var_declaration_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let descriptor = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let function = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVar(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("duplicateVar")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![descriptor, descriptor],
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();

    assert_eq!(context.execute(&function).unwrap(), Value::Undefined);
    let key = runtime.intern_property_key("duplicateVar").unwrap();
    let global = context.global_object().unwrap();
    assert_eq!(
        context.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: true,
            configurable: false,
        })
    );
}

#[test]
fn publication_accepts_annex_masked_mixed_global_declaration_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let ordinary = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let lexical_let = ClosureVariable {
        is_lexical: true,
        ..ordinary
    };
    let lexical_const = ClosureVariable {
        is_const: true,
        ..lexical_let
    };
    let function = UnlinkedFunction::new_with_closure_variables(
        vec![
            Instruction::PushI32(7),
            Instruction::PutVarInit(0),
            Instruction::GetVar(0),
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("annexMaskedMixed")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 4,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ordinary, lexical_let, lexical_const, ordinary],
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();

    assert_eq!(context.execute(&function).unwrap(), Value::Int(7));
}

#[test]
fn publication_restricts_global_function_initializer_to_the_first_name_slot() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let ordinary = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let global_function = ClosureVariable {
        kind: ClosureVariableKind::GlobalFunction,
        ..ordinary
    };
    let draft = |code| {
        let child = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_name(Some(JsString::from_static("functionSlot")));
        UnlinkedFunction::new_with_closure_variables(
            code,
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("functionSlot")))
                    .unwrap(),
                UnlinkedConstant::child(child),
            ],
            FunctionMetadata {
                closure_count: 3,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ordinary, ordinary, global_function],
        )
    };

    for code in [
        vec![
            Instruction::FClosure(1),
            Instruction::PutVarInit(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarInit(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![Instruction::FClosure(1), Instruction::Return],
    ] {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, draft(code)),
            Err(RuntimeError::Engine(_))
        ));
    }
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    runtime
        .publish_unlinked_function(
            context.realm,
            draft(vec![
                Instruction::FClosure(1),
                Instruction::PutVarInit(0),
                Instruction::Undefined,
                Instruction::Return,
            ]),
        )
        .expect("the first same-name declaration slot should be a valid raw target");
}

#[test]
fn put_var_init_initializes_a_const_global_lexical_once() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .create_global_lexical_for_test("initializedLexical", true, None)
        .unwrap();
    let function = UnlinkedFunction::new_with_closure_variables(
        vec![
            Instruction::PushI32(9),
            Instruction::PutVarInit(0),
            Instruction::GetVar(0),
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("initializedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: true,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    let function = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Int(9)
    );
    assert!(matches!(
        context.eval("initializedLexical = 10"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const lexical reassignment did not throw an object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'initializedLexical' is read-only"))
    );
}

#[test]
fn publication_preflights_closure_descriptors_before_heap_changes() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let missing_descriptor = UnlinkedFunction::new(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, missing_descriptor),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let root_parent_local = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            local_count: 1,
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, root_parent_local),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let out_of_bounds_argument_child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentArgument(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(out_of_bounds_argument_child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, parent),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let out_of_bounds_child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: crate::heap::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(out_of_bounds_child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, parent),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn publication_rejects_inconsistent_closure_metadata() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let child = |is_lexical| {
        UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: crate::heap::ClosureVariableName::None,
                is_lexical,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
    };
    let inconsistent_siblings = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        vec![
            UnlinkedConstant::child(child(false)),
            UnlinkedConstant::child(child(true)),
        ],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, inconsistent_siblings),
        Err(RuntimeError::Engine(_))
    ));

    let illegal_const = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: crate::heap::ClosureVariableName::None,
            is_lexical: false,
            is_const: true,
            kind: ClosureVariableKind::Normal,
        }],
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, illegal_const),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn deeply_nested_child_publication_and_release_are_iterative() {
    const DEPTH: usize = 50_000;

    let runtime = Runtime::new();
    let context = runtime.new_context();
    let metadata = FunctionMetadata {
        max_stack: 1,
        ..FunctionMetadata::default()
    };
    let mut function = UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        metadata,
    );
    for _ in 0..DEPTH {
        function = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(function)],
            metadata,
        );
    }

    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, DEPTH + 1);
    drop(function);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn function_closures_share_runtime_rooted_var_ref_cells() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
    let child = UnlinkedFunction::new_with_closure_variables(
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
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let root = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                vec![UnlinkedConstant::child(child)],
                FunctionMetadata {
                    local_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_variable_definitions(
                Vec::new(),
                vec![UnlinkedVariableDefinition::ordinary(None)],
            ),
        )
        .unwrap();
    let function = runtime.test_child_function_bytecode(&root, 0).unwrap();
    let cell = runtime
        .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
        .unwrap();
    let cell_id = cell.id();
    let first = runtime
        .new_bytecode_closure_with_slots(context.realm, &function, std::slice::from_ref(&cell))
        .unwrap();
    let second = runtime
        .new_bytecode_closure_with_slots(context.realm, &function, std::slice::from_ref(&cell))
        .unwrap();
    assert_eq!(
        runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
        Ok(3)
    );

    let mut caller = context.clone();
    assert_eq!(
        caller.call(&first, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        caller.call(&second, Value::Undefined, &[]).unwrap(),
        Value::Int(3)
    );

    runtime.write_var_ref(&cell, Value::Int(7)).unwrap();
    assert_eq!(runtime.read_var_ref(&cell).unwrap(), Value::Int(7));
    drop(cell);
    assert_eq!(
        runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
        Ok(2)
    );
    let promoted = VarRefRoot::from_borrowed_handle(runtime.clone(), cell_id).unwrap();
    assert_eq!(runtime.read_var_ref(&promoted).unwrap(), Value::Int(7));
    drop(first);
    drop(second);
    assert_eq!(
        runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
        Ok(1)
    );
    drop(promoted);
    assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
}

fn incrementing_closure(source: ClosureSource) -> UnlinkedFunction {
    UnlinkedFunction::new_with_closure_variables(
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
            name: crate::heap::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    )
}

#[test]
fn fclosure_captures_parent_local_and_isolates_each_invocation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::PushI32(10),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(incrementing_closure(
            ClosureSource::ParentLocal(0),
        ))],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();

    let first = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let second = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    assert_eq!(
        context.call(&first, Value::Undefined, &[]).unwrap(),
        Value::Int(11)
    );
    assert_eq!(
        context.call(&first, Value::Undefined, &[]).unwrap(),
        Value::Int(12)
    );
    assert_eq!(
        context.call(&second, Value::Undefined, &[]).unwrap(),
        Value::Int(11)
    );

    drop(first);
    drop(second);
    assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
}

#[test]
fn parent_local_writes_after_fclosure_update_the_shared_cell() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: crate::heap::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::PushI32(1),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::PushI32(7),
            Instruction::PutLocal(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let child = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();

    assert_eq!(
        context.call(&child, Value::Undefined, &[]).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn repeated_fclosure_in_one_frame_reuses_the_parent_cell() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let parent = UnlinkedFunction::new(
        vec![
            Instruction::PushI32(0),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::FClosure(0),
            Instruction::Call(0),
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(incrementing_closure(
            ClosureSource::ParentLocal(0),
        ))],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let survivor = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();

    assert_eq!(
        context.call(&survivor, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn parent_argument_and_transitive_parent_closure_capture_share_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let inner = incrementing_closure(ClosureSource::ParentClosure(0));
    let middle = UnlinkedFunction::new_with_closure_variables(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(inner)],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentArgument(0),
            name: crate::heap::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let outer = UnlinkedFunction::new(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(middle)],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let outer = runtime
        .publish_unlinked_function(context.realm, outer)
        .unwrap();
    let outer = runtime.new_bytecode_closure(context.realm, &outer).unwrap();
    let middle = context
        .call(&outer, Value::Undefined, &[Value::Int(40)])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let first_inner = context
        .call(&middle, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let second_inner = context
        .call(&middle, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();

    assert_eq!(
        context.call(&first_inner, Value::Undefined, &[]).unwrap(),
        Value::Int(41)
    );
    assert_eq!(
        context.call(&second_inner, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.call(&first_inner, Value::Undefined, &[]).unwrap(),
        Value::Int(43)
    );

    let isolated_middle = context
        .call(&outer, Value::Undefined, &[Value::Int(40)])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let isolated_inner = context
        .call(&isolated_middle, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    assert_eq!(
        context
            .call(&isolated_inner, Value::Undefined, &[])
            .unwrap(),
        Value::Int(41)
    );
}

#[test]
fn executing_foreign_runtime_bytecode_is_rejected_before_instantiation() {
    let first = Runtime::new();
    let second = Runtime::new();
    let mut compiler_context = first.new_context();
    let function = compiler_context.compile("42").unwrap();
    let mut caller_context = second.new_context();
    let caller_realm_objects = second.heap_counts().object_nodes;

    assert!(matches!(
        caller_context.execute(&function),
        Err(RuntimeError::WrongRuntime("function bytecode"))
    ));
    assert_eq!(second.heap_counts().object_nodes, caller_realm_objects);
}

#[test]
fn pending_exception_slot_owns_and_transfers_object_roots() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let object_id = object.object_id();
    runtime
        .set_pending_exception(Value::Object(object.clone()))
        .unwrap();
    assert!(runtime.has_pending_exception());
    assert_eq!(
        runtime.0.state.borrow().heap.object_strong_count(object_id),
        Ok(2)
    );
    drop(object);

    let exception = runtime.take_pending_exception().unwrap().unwrap();
    assert!(!runtime.has_pending_exception());
    assert!(matches!(
        &exception,
        Value::Object(value) if value.object_id() == object_id
    ));
    assert_eq!(
        runtime.0.state.borrow().heap.object_strong_count(object_id),
        Ok(1)
    );
    drop(exception);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn pending_exception_roots_survive_gc_and_preserve_symbol_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let object_id = object.object_id();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(
        context
            .set_property(&object, &self_key, Value::Object(object.clone()))
            .unwrap()
    );
    runtime
        .set_pending_exception(Value::Object(object.clone()))
        .unwrap();
    drop(object);

    assert_eq!(runtime.run_gc().unwrap().cleanup.finalized_objects, 0);
    let exception = runtime.take_pending_exception().unwrap().unwrap();
    assert!(matches!(
        &exception,
        Value::Object(object) if object.object_id() == object_id
    ));
    drop(exception);
    assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 1);

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("boom")))
        .unwrap();
    let expected = symbol.clone();
    runtime
        .set_pending_exception(Value::Symbol(symbol))
        .unwrap();
    let exception = runtime.take_pending_exception().unwrap().unwrap();
    assert!(matches!(exception, Value::Symbol(symbol) if symbol == expected));
}

#[test]
fn throw_completion_moves_the_value_into_the_runtime_exception_slot() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval("throw 9"), Err(RuntimeError::Exception));
    assert!(context.has_exception());
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
    assert!(!context.has_exception());
}

#[test]
fn vm_fault_materializes_a_native_error_in_the_callee_realm() {
    let runtime = Runtime::new();
    let mut compiler_context = runtime.new_context();
    let function = compiler_context.compile("1n + 1").unwrap();
    let expected_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(compiler_context.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();
    let mut caller_context = runtime.new_context();
    let caller_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(caller_context.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();
    assert_ne!(expected_prototype, caller_prototype);

    assert_eq!(
        caller_context.execute(&function),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("expected a native Error object");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .object(error.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Error
    ));
    assert_eq!(
        runtime
            .get_prototype_of(&error)
            .unwrap()
            .unwrap()
            .object_id(),
        expected_prototype
    );

    let message = runtime.intern_property_key("message").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = runtime.get_own_property(&error, &message).unwrap().unwrap()
    else {
        panic!("native Error message must be an own data property");
    };
    assert_eq!(
        value,
        Value::String(JsString::from_static("cannot convert bigint to number"))
    );
    assert!(writable);
    assert!(!enumerable);
    assert!(configurable);

    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        caller_context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    let prototype = runtime.get_prototype_of(&error).unwrap().unwrap();
    assert!(!runtime.is_error_object(&prototype).unwrap());
}

#[test]
fn nested_fault_non_callable_and_compile_syntax_use_exception_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context.eval("(function(){ return 1n + 1; })()"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(nested) = context.take_exception().unwrap().unwrap() else {
        panic!("expected nested TypeError");
    };
    assert!(runtime.is_error_object(&nested).unwrap());

    assert_eq!(context.eval("(1)()"), Err(RuntimeError::Exception));
    let Value::Object(not_callable) = context.take_exception().unwrap().unwrap() else {
        panic!("expected non-callable TypeError");
    };
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .object(not_callable.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Error
    ));

    assert_eq!(context.compile("throw\n9"), Err(RuntimeError::Exception));
    let Value::Object(syntax) = context.take_exception().unwrap().unwrap() else {
        panic!("expected SyntaxError");
    };
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .object(syntax.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Error
    ));
    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        context.get_property(&syntax, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
}

#[test]
fn rooted_handles_enforce_runtime_domain_and_dup_free_counts() {
    let first = Runtime::new();
    let second = Runtime::new();
    let first_key = first.intern_property_key("first").unwrap();
    let second_key = second.intern_property_key("second").unwrap();
    assert_eq!(first_key.atom().raw(), second_key.atom().raw());
    assert_ne!(first_key, second_key);

    let object = first.new_object(None).unwrap();
    assert!(matches!(
        second.define_own_property(
            &object,
            &second_key,
            &data_descriptor(Value::Int(1), true, true, true)
        ),
        Err(RuntimeError::WrongRuntime("object"))
    ));
    assert!(matches!(
        first.define_own_property(
            &object,
            &second_key,
            &data_descriptor(Value::Int(1), true, true, true)
        ),
        Err(RuntimeError::WrongRuntime("property key"))
    ));
    let foreign_object = second.new_object(None).unwrap();
    assert!(matches!(
        set_property(&first, &object, &first_key, Value::Object(foreign_object)),
        Err(RuntimeError::WrongRuntime("property value"))
    ));

    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(1)
    );
    let value = Value::Object(object.clone());
    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(2)
    );
    let duplicate = value.clone();
    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(3)
    );
    drop(duplicate);
    drop(value);
    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(1)
    );
}

#[test]
fn shape_sharing_and_descriptor_defaults_follow_quickjs_layout() {
    let runtime = Runtime::new();
    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("x").unwrap();

    let empty_shapes = {
        let state = runtime.0.state.borrow();
        (
            state.heap.object(first.object_id()).unwrap().shape,
            state.heap.object(second.object_id()).unwrap().shape,
        )
    };
    assert_eq!(empty_shapes.0, empty_shapes.1);

    let defaulted = OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(7)),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        runtime
            .define_own_property(&first, &key, &defaulted)
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(&second, &key, &defaulted)
            .unwrap()
    );
    assert_eq!(
        runtime.get_own_property(&first, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(7),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );
    let populated_shapes = {
        let state = runtime.0.state.borrow();
        (
            state.heap.object(first.object_id()).unwrap().shape,
            state.heap.object(second.object_id()).unwrap().shape,
        )
    };
    assert_eq!(populated_shapes.0, populated_shapes.1);
    assert_ne!(populated_shapes.0, empty_shapes.0);
}

#[test]
fn own_keys_preserve_quickjs_category_order_and_utf16_identity() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let symbol_a = runtime
        .new_symbol(Some(JsString::from_static("a")))
        .unwrap();
    let symbol_b = runtime
        .new_symbol(Some(JsString::from_static("b")))
        .unwrap();
    let symbol_key_a = PropertyKey::from(&symbol_a);
    let symbol_key_b = PropertyKey::from(&symbol_b);

    for (key, value) in [
        (runtime.intern_property_key("beta").unwrap(), 1),
        (runtime.intern_property_key("4294967295").unwrap(), 2),
        (runtime.intern_property_key("2147483648").unwrap(), 3),
        (runtime.intern_property_key("01").unwrap(), 4),
        (runtime.intern_property_key("4294967294").unwrap(), 5),
        (runtime.intern_property_key("0").unwrap(), 6),
        (runtime.intern_property_key("-0").unwrap(), 7),
    ] {
        assert!(set_property(&runtime, &object, &key, Value::Int(value)).unwrap());
    }
    assert!(set_property(&runtime, &object, &symbol_key_a, Value::Int(8)).unwrap());
    assert!(
        set_property(
            &runtime,
            &object,
            &runtime.intern_property_key("2").unwrap(),
            Value::Int(9)
        )
        .unwrap()
    );
    assert!(set_property(&runtime, &object, &symbol_key_b, Value::Int(10)).unwrap());

    let expected = [
        runtime.intern_property_key("0").unwrap(),
        runtime.intern_property_key("2").unwrap(),
        runtime.intern_property_key("2147483648").unwrap(),
        runtime.intern_property_key("4294967294").unwrap(),
        runtime.intern_property_key("beta").unwrap(),
        runtime.intern_property_key("4294967295").unwrap(),
        runtime.intern_property_key("01").unwrap(),
        runtime.intern_property_key("-0").unwrap(),
        symbol_key_a.clone(),
        symbol_key_b.clone(),
    ];
    assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);

    let surrogate = runtime
        .intern_property_key_js_string(&JsString::try_from_utf16([0xd800]).unwrap())
        .unwrap();
    let replacement = runtime
        .intern_property_key_js_string(&JsString::try_from_utf16([0xfffd]).unwrap())
        .unwrap();
    assert_ne!(surrogate, replacement);
    assert!(set_property(&runtime, &object, &surrogate, Value::Int(11)).unwrap());
    assert!(set_property(&runtime, &object, &replacement, Value::Int(12)).unwrap());
    assert_eq!(
        runtime.property_key_to_js_string(&surrogate).unwrap(),
        JsString::try_from_utf16([0xd800]).unwrap()
    );
}

#[test]
fn delete_readd_and_frozen_same_value_rules_match_oracle() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let a = runtime.intern_property_key("a").unwrap();
    let b = runtime.intern_property_key("b").unwrap();
    let c = runtime.intern_property_key("c").unwrap();
    for key in [&a, &b, &c] {
        assert!(set_property(&runtime, &object, key, Value::Int(1)).unwrap());
    }
    assert!(runtime.delete_property(&object, &a).unwrap());
    assert!(set_property(&runtime, &object, &a, Value::Int(2)).unwrap());
    assert_eq!(
        runtime.own_property_keys(&object).unwrap(),
        vec![b.clone(), c.clone(), a.clone()]
    );

    let nan = runtime.intern_property_key("nan").unwrap();
    let zero = runtime.intern_property_key("zero").unwrap();
    assert!(
        runtime
            .define_own_property(
                &object,
                &nan,
                &data_descriptor(Value::Float(f64::NAN), false, true, false)
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &zero,
                &data_descriptor(Value::Int(0), false, true, false)
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &nan,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Float(f64::NAN)),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert!(
        !runtime
            .define_own_property(
                &object,
                &nan,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(0)),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert!(
        !runtime
            .define_own_property(
                &object,
                &zero,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Float(-0.0)),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
}

#[test]
fn inherited_set_and_prototype_constraints_match_ordinary_semantics() {
    let runtime = Runtime::new();
    let parent = runtime.new_object(None).unwrap();
    let writable = runtime.intern_property_key("writable").unwrap();
    let readonly = runtime.intern_property_key("readonly").unwrap();
    assert!(
        runtime
            .define_own_property(
                &parent,
                &writable,
                &data_descriptor(Value::Int(1), true, true, true)
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &parent,
                &readonly,
                &data_descriptor(Value::Int(1), false, true, true)
            )
            .unwrap()
    );
    let child = runtime.new_object(Some(&parent)).unwrap();
    assert!(set_property(&runtime, &child, &writable, Value::Int(2)).unwrap());
    assert!(!set_property(&runtime, &child, &readonly, Value::Int(2)).unwrap());
    assert_eq!(
        get_property(&runtime, &child, &writable).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        get_property(&runtime, &child, &readonly).unwrap(),
        Value::Int(1)
    );

    let receiver = runtime.new_object(None).unwrap();
    assert!(
        set_property_with_receiver(
            &runtime,
            &parent,
            &writable,
            Value::Int(3),
            Value::Object(receiver.clone()),
        )
        .unwrap()
    );
    assert_eq!(
        get_property(&runtime, &parent, &writable).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        get_property(&runtime, &receiver, &writable).unwrap(),
        Value::Int(3)
    );

    let mut context = runtime.new_context();
    let Value::Object(receiver_setter) = context.eval("(function(value) {})").unwrap() else {
        panic!("receiver setter probe did not produce a function");
    };
    let receiver_setter = runtime.as_callable(&receiver_setter).unwrap().unwrap();
    let accessor_receiver = runtime.new_object(None).unwrap();
    assert!(
        runtime
            .define_own_property(
                &accessor_receiver,
                &writable,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(receiver_setter)),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(
        !set_property_with_receiver(
            &runtime,
            &parent,
            &writable,
            Value::Int(4),
            Value::Object(accessor_receiver),
        )
        .unwrap()
    );

    let fixed = runtime.new_object(None).unwrap();
    runtime.prevent_extensions(&fixed).unwrap();
    assert!(runtime.set_prototype_of(&fixed, None).unwrap());
    assert!(!runtime.set_prototype_of(&fixed, Some(&parent)).unwrap());

    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    assert!(runtime.set_prototype_of(&first, Some(&second)).unwrap());
    assert!(!runtime.set_prototype_of(&second, Some(&first)).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&first).unwrap(),
        Some(second.clone())
    );
    assert_eq!(runtime.get_prototype_of(&second).unwrap(), None);
}

#[test]
fn object_property_cycle_is_collected_only_by_explicit_gc() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(set_property(&runtime, &object, &self_key, Value::Object(object.clone())).unwrap());
    assert_eq!(runtime.heap_counts().object_nodes, 1);
    let state = runtime.0.state.borrow_mut();
    drop(object);
    drop(state);
    let stats = runtime.run_gc().unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn named_function_self_capture_cycle_is_collected() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let closure = context
        .eval(
            "(function() {\
                var child;\
                var owner = function self() {\
                    child = function() { return self; };\
                    return child;\
                };\
                return owner();\
            })()",
        )
        .unwrap();
    let retained = runtime.heap_counts();
    assert!(retained.object_nodes >= baseline.object_nodes + 2);
    assert!(retained.var_ref_nodes >= baseline.var_ref_nodes + 2);
    drop(closure);
    assert!(runtime.heap_counts().object_nodes > baseline.object_nodes);

    let stats = runtime.run_gc().unwrap();
    assert!(stats.cleanup.finalized_objects >= 2);
    assert!(stats.cleanup.finalized_var_refs >= 2);
    let collected = runtime.heap_counts();
    assert_eq!(collected.object_nodes, baseline.object_nodes);
    assert_eq!(collected.var_ref_nodes, baseline.var_ref_nodes);
    assert_eq!(
        collected.function_bytecode_nodes,
        baseline.function_bytecode_nodes
    );
}

#[test]
fn symbols_are_runtime_owned_and_distinct_from_registry_entries() {
    let runtime = Runtime::new();
    let name = JsString::from_static("Symbol.iterator");
    let unique = runtime.well_known_symbol(WellKnownSymbol::Iterator);
    let repeated = runtime.well_known_symbol(WellKnownSymbol::Iterator);
    let registry = runtime.symbol_for(&name).unwrap();
    assert_eq!(unique, repeated);
    assert_ne!(unique, registry);
    assert_ne!(PropertyKey::from(&unique), PropertyKey::from(&registry));
    assert_eq!(runtime.symbol_key_for(&unique).unwrap(), None);
    assert_eq!(runtime.symbol_key_for(&registry).unwrap(), Some(name));
}

#[test]
fn exceptional_vm_exit_releases_local_frame_roots_immediately() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let function = BytecodeFunction {
        name: None,
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushConst(1),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![
            Value::Object(object.clone()),
            Value::BigInt(JsBigInt::one()),
        ],
        local_count: 0,
        max_stack: 3,
    };
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .object_strong_count(object.object_id())
        .unwrap();
    assert!(Vm::new().execute(&function).is_err());
    let after = runtime
        .0
        .state
        .borrow()
        .heap
        .object_strong_count(object.object_id())
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn drops_during_runtime_borrow_are_deferred_to_the_next_safe_point() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("queued").unwrap();
    let state = runtime.0.state.borrow_mut();
    drop(object);
    drop(key);
    assert_eq!(runtime.0.deferred_references.borrow().len(), 2);
    drop(state);

    let context = runtime.new_context();
    assert!(runtime.0.deferred_references.borrow().is_empty());
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn finalized_shapes_unlink_exact_weak_cache_entries() {
    let runtime = Runtime::new();
    let mut objects = Vec::new();
    for index in 0..2_000 {
        let object = runtime.new_object(None).unwrap();
        let key = runtime
            .intern_property_key(&format!("unique-{index}"))
            .unwrap();
        assert!(set_property(&runtime, &object, &key, Value::Int(index)).unwrap());
        objects.push(object);
    }
    assert!(runtime.0.state.borrow().shape_cache.len() >= objects.len());
    drop(objects);
    let state = runtime.0.state.borrow();
    assert!(state.shape_cache.is_empty());
    assert!(state.shape_fingerprints.is_empty());
}

#[test]
fn unique_shape_append_never_mutates_a_shared_shape() {
    let runtime = Runtime::new();
    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    let shared_keys = (0..super::properties::MIN_UNIQUE_SHAPE_APPEND_ENTRIES)
        .map(|index| {
            runtime
                .intern_property_key(&format!("shared-{index}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let b = runtime.intern_property_key("unique-b").unwrap();
    let c = runtime.intern_property_key("unique-c").unwrap();

    for key in &shared_keys {
        assert!(set_property(&runtime, &first, key, Value::Int(1)).unwrap());
        assert!(set_property(&runtime, &second, key, Value::Int(2)).unwrap());
    }
    let shared_shape = {
        let state = runtime.0.state.borrow();
        let first_shape = state.heap.object(first.object_id()).unwrap().shape;
        let second_shape = state.heap.object(second.object_id()).unwrap().shape;
        assert_eq!(first_shape, second_shape);
        assert_eq!(state.heap.shape_strong_count(first_shape), Ok(2));
        first_shape
    };

    assert!(set_property(&runtime, &first, &b, Value::Int(3)).unwrap());
    let unique_shape = runtime
        .0
        .state
        .borrow()
        .heap
        .object(first.object_id())
        .unwrap()
        .shape;
    assert_ne!(unique_shape, shared_shape);
    assert!(set_property(&runtime, &first, &c, Value::Int(4)).unwrap());
    let state = runtime.0.state.borrow();
    assert_eq!(
        state.heap.object(first.object_id()).unwrap().shape,
        unique_shape,
        "the second unique addition should append to the existing shape"
    );
    assert_eq!(
        state.heap.object(second.object_id()).unwrap().shape,
        shared_shape,
        "mutating the unique successor must not alter the shared predecessor"
    );
    assert_eq!(
        state
            .heap
            .shape(shared_shape)
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.atom)
            .collect::<Vec<_>>(),
        shared_keys
            .iter()
            .map(PropertyKey::atom)
            .collect::<Vec<_>>()
    );
    let mut unique_atoms = shared_keys
        .iter()
        .map(PropertyKey::atom)
        .collect::<Vec<_>>();
    unique_atoms.extend([b.atom(), c.atom()]);
    assert_eq!(
        state
            .heap
            .shape(unique_shape)
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.atom)
            .collect::<Vec<_>>(),
        unique_atoms
    );
    assert!(!state.shape_fingerprints.contains_key(&unique_shape));
}

#[test]
fn unique_shape_append_preserves_key_categories_and_readd_order() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let fillers = (0..5)
        .map(|index| {
            runtime
                .intern_property_key(&format!("unique-fill-{index}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let beta = runtime.intern_property_key("unique-beta").unwrap();
    let alpha = runtime.intern_property_key("unique-alpha").unwrap();
    let index = runtime.intern_property_key("2").unwrap();
    let symbol = runtime
        .new_symbol(Some(JsString::from_static("unique-symbol")))
        .unwrap();
    let symbol_key = PropertyKey::from(&symbol);
    for key in &fillers {
        assert!(set_property(&runtime, &object, key, Value::Int(0)).unwrap());
    }
    for key in [&beta, &symbol_key, &index, &alpha] {
        assert!(set_property(&runtime, &object, key, Value::Int(1)).unwrap());
    }
    let mut expected = vec![index.clone()];
    expected.extend(fillers.iter().cloned());
    expected.extend([beta.clone(), alpha.clone(), symbol_key.clone()]);
    assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);

    assert!(runtime.delete_property(&object, &beta).unwrap());
    assert!(set_property(&runtime, &object, &beta, Value::Int(2)).unwrap());
    let mut expected = vec![index];
    expected.extend(fillers);
    expected.extend([alpha, beta, symbol_key]);
    assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);
}

#[test]
fn unique_shape_append_releases_its_key_atom_with_the_final_object() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let seeds = (0..super::properties::MIN_UNIQUE_SHAPE_APPEND_ENTRIES)
        .map(|index| {
            runtime
                .intern_property_key(&format!("unique-append-seed-{index}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let key = runtime
        .intern_property_key("unique-append-final-key")
        .unwrap();
    let atom = key.atom();
    for seed in &seeds {
        assert!(set_property(&runtime, &object, seed, Value::Int(1)).unwrap());
    }
    assert!(set_property(&runtime, &object, &key, Value::Int(2)).unwrap());
    drop(seeds);
    drop(key);
    assert!(
        runtime.0.state.borrow().atoms.resolve(atom).is_ok(),
        "the in-place shape must own its appended key atom"
    );

    drop(object);
    runtime.run_gc().unwrap();
    assert!(
        runtime.0.state.borrow().atoms.resolve(atom).is_err(),
        "final shape collection must release its appended key atom"
    );
}

#[test]
fn failed_unique_shape_append_restores_cache_and_atom_ownership() {
    let runtime = Runtime::new();
    let owner = runtime.new_object(None).unwrap();
    let stale = runtime.new_object(None).unwrap();
    let stale_id = stale.object_id();
    drop(stale);
    let key = runtime.intern_property_key("failed-unique-append").unwrap();
    let atom = key.atom();

    let mut state = runtime.0.state.borrow_mut();
    let shape = state.heap.object(owner.object_id()).unwrap().shape;
    assert_eq!(state.heap.shape_strong_count(shape), Ok(1));
    let before_ref_count = state.atoms.resolve(atom).unwrap().ref_count;
    let fingerprint = state
        .shape_fingerprints
        .get(&shape)
        .expect("the untouched empty shape should still be cached")
        .clone();
    assert_eq!(state.shape_cache.get(&fingerprint), Some(&shape));

    assert!(matches!(
        state.append_unique_layout(
            owner.object_id(),
            atom,
            PropertyFlags::data(true, true, true),
            PropertySlot::Data(RawValue::Object(stale_id)),
        ),
        Err(RuntimeError::Heap(HeapError::Stale { .. }))
    ));
    assert_eq!(
        state.atoms.resolve(atom).unwrap().ref_count,
        before_ref_count,
        "a rejected append must roll back the shape's tentative atom root"
    );
    assert!(state.heap.shape(shape).unwrap().entries().is_empty());
    assert_eq!(state.shape_fingerprints.get(&shape), Some(&fingerprint));
    assert_eq!(state.shape_cache.get(&fingerprint), Some(&shape));
}

fn debug_draft(debug: UnlinkedFunctionDebug) -> UnlinkedFunction {
    UnlinkedFunction::new(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_debug(debug)
}

#[test]
fn publication_accepts_arbitrary_debug_source_bytes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source =
        b"function f(){/*\x80Q*/\0\xef\xbb\xbf\xed\xa0\xbd\xed\xb8\x80\xed\xa0\x80\xff}".to_vec();
    let function = runtime
        .publish_unlinked_function(
            context.realm,
            debug_draft(UnlinkedFunctionDebug {
                filename: JsString::from_static("raw-source.js"),
                pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
                source: Some(source.clone().into_boxed_slice()),
            }),
        )
        .unwrap();

    assert_eq!(
        runtime.test_function_debug_source(&function).unwrap(),
        Some(source.clone())
    );

    let callable = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
    let actual = context
        .call(&to_string, Value::Object(callable.as_object().clone()), &[])
        .unwrap();
    let expected = JsString::try_from_bytes(&source).unwrap();
    assert_eq!(actual, Value::String(expected.clone()));
    let units = expected.utf16_units().collect::<Vec<_>>();
    assert!(units.contains(&0xfffd));
    assert!(!units.contains(&u16::from(b'Q')));
    assert!(units.contains(&0));
    assert!(units.contains(&0xd800));
}

#[test]
fn publication_rejects_malformed_debug_pc_order_range_and_position() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    let malformed = [
        UnlinkedFunctionDebug {
            filename: JsString::from_static("range.js"),
            pc2line: Some(Pc2LineTable::new(
                LineColumn::new(0, 0),
                vec![Pc2LineEntry {
                    pc: 2,
                    position: LineColumn::new(0, 0),
                }],
            )),
            source: None,
        },
        UnlinkedFunctionDebug {
            filename: JsString::from_static("order.js"),
            pc2line: Some(Pc2LineTable::new(
                LineColumn::new(0, 0),
                vec![
                    Pc2LineEntry {
                        pc: 1,
                        position: LineColumn::new(0, 1),
                    },
                    Pc2LineEntry {
                        pc: 0,
                        position: LineColumn::new(0, 0),
                    },
                ],
            )),
            source: None,
        },
        UnlinkedFunctionDebug {
            filename: JsString::from_static("position.js"),
            pc2line: Some(Pc2LineTable::new(LineColumn::new(u32::MAX, 0), Vec::new())),
            source: None,
        },
    ];

    for debug in malformed {
        assert!(
            runtime
                .publish_unlinked_function(context.realm, debug_draft(debug))
                .is_err()
        );
        assert_eq!(
            runtime.heap_counts().function_bytecode_nodes,
            baseline,
            "malformed debug metadata changed the heap"
        );
    }
}

#[test]
fn publication_keeps_duplicate_last_and_unreachable_pc_metadata() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let duplicate = debug_draft(UnlinkedFunctionDebug {
        filename: JsString::from_static("duplicate.js"),
        pc2line: Some(Pc2LineTable::new(
            LineColumn::new(0, 0),
            vec![
                Pc2LineEntry {
                    pc: 0,
                    position: LineColumn::new(1, 1),
                },
                Pc2LineEntry {
                    pc: 0,
                    position: LineColumn::new(2, 2),
                },
            ],
        )),
        source: None,
    });
    let duplicate = runtime
        .publish_unlinked_function(context.realm, duplicate)
        .unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&duplicate, Some(0))
            .unwrap(),
        Some((JsString::from_static("duplicate.js"), LineColumn::new(2, 2)))
    );

    let unreachable = UnlinkedFunction::new(
        vec![
            Instruction::Goto(2),
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_debug(UnlinkedFunctionDebug {
        filename: JsString::from_static("unreachable.js"),
        pc2line: Some(Pc2LineTable::new(
            LineColumn::new(0, 0),
            vec![Pc2LineEntry {
                pc: 1,
                position: LineColumn::new(9, 4),
            }],
        )),
        source: None,
    });
    let unreachable = runtime
        .publish_unlinked_function(context.realm, unreachable)
        .unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&unreachable, Some(1))
            .unwrap(),
        Some((
            JsString::from_static("unreachable.js"),
            LineColumn::new(9, 4)
        ))
    );
}

#[test]
fn publication_rollback_releases_the_new_debug_filename_atom() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let stale_realm = context.realm;
    drop(context);
    runtime.run_gc().unwrap();
    let baseline_atoms = runtime.test_atom_count();
    let baseline_bytecode = runtime.heap_counts().function_bytecode_nodes;

    let function = debug_draft(UnlinkedFunctionDebug {
        filename: JsString::from_static("rollback-debug-filename.js"),
        pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
        source: None,
    });
    assert!(
        runtime
            .publish_unlinked_function(stale_realm, function)
            .is_err()
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline_bytecode
    );
}

#[test]
fn pending_promise_reaction_does_not_retain_the_species_result_object() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(result) = context
        .eval(
            r#"
var pendingSource = new Promise(function () {});
function CustomCapability(executor) {
    executor(function () {}, function () {});
    return { marker: 42 };
}
pendingSource.constructor = { [Symbol.species]: CustomCapability };
pendingSource.then();
"#,
        )
        .unwrap()
    else {
        panic!("custom Promise capability did not return its result object");
    };
    let result_id = result.object_id();
    drop(result);

    runtime.run_gc().unwrap();
    assert!(
        runtime.0.state.borrow().heap.object(result_id).is_err(),
        "a pending reaction retained the custom species result object"
    );
    assert_eq!(
        context.eval("typeof pendingSource.then").unwrap(),
        Value::String(JsString::from_static("function"))
    );
}

#[test]
fn weak_ref_and_finalization_registry_globals_match_intrinsic_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
(function () {
    function flags(descriptor) {
        return Number(descriptor.writable) +
            "" + Number(descriptor.enumerable) +
            Number(descriptor.configurable);
    }
    var weakGlobal = Object.getOwnPropertyDescriptor(globalThis, "WeakRef");
    var weakPrototype = Object.getOwnPropertyDescriptor(WeakRef, "prototype");
    var weakConstructor = Object.getOwnPropertyDescriptor(
        WeakRef.prototype,
        "constructor"
    );
    var deref = Object.getOwnPropertyDescriptor(WeakRef.prototype, "deref");
    var weakTag = Object.getOwnPropertyDescriptor(
        WeakRef.prototype,
        Symbol.toStringTag
    );
    var registryGlobal = Object.getOwnPropertyDescriptor(
        globalThis,
        "FinalizationRegistry"
    );
    var registryPrototype = Object.getOwnPropertyDescriptor(
        FinalizationRegistry,
        "prototype"
    );
    var registryConstructor = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        "constructor"
    );
    var register = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        "register"
    );
    var unregister = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        "unregister"
    );
    var registryTag = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        Symbol.toStringTag
    );
    return [
        typeof WeakRef,
        flags(weakGlobal),
        WeakRef.name,
        WeakRef.length,
        flags(weakPrototype),
        flags(weakConstructor),
        deref.value.name,
        deref.value.length,
        flags(deref),
        Object.prototype.toString.call(WeakRef.prototype),
        flags(weakTag),
        typeof FinalizationRegistry,
        flags(registryGlobal),
        FinalizationRegistry.name,
        FinalizationRegistry.length,
        flags(registryPrototype),
        flags(registryConstructor),
        register.value.name,
        register.value.length,
        flags(register),
        unregister.value.name,
        unregister.value.length,
        flags(unregister),
        Object.prototype.toString.call(FinalizationRegistry.prototype),
        flags(registryTag)
    ].join("|");
})()
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "function|101|WeakRef|1|000|101|deref|0|101|[object WeakRef]|001|\
             function|101|FinalizationRegistry|1|000|101|register|2|101|\
             unregister|1|101|[object FinalizationRegistry]|001"
        ))
    );
}

#[test]
fn weak_ref_and_finalization_registry_reject_invalid_calls_brands_and_registered_symbols() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
(function () {
    function throwsTypeError(callback) {
        try {
            callback();
            return false;
        } catch (error) {
            return error instanceof TypeError;
        }
    }
    var registry = new FinalizationRegistry(function () {});
    var registered = Symbol.for("weak-target-probe");
    return [
        throwsTypeError(function () { WeakRef({}); }),
        throwsTypeError(function () { FinalizationRegistry(function () {}); }),
        throwsTypeError(function () { new WeakRef(1); }),
        throwsTypeError(function () { new FinalizationRegistry({}); }),
        throwsTypeError(function () { WeakRef.prototype.deref.call({}); }),
        throwsTypeError(function () {
            FinalizationRegistry.prototype.register.call({}, {}, 1);
        }),
        throwsTypeError(function () {
            FinalizationRegistry.prototype.unregister.call({}, {});
        }),
        throwsTypeError(function () { new WeakRef(registered); }),
        throwsTypeError(function () { registry.register(registered, 1); }),
        throwsTypeError(function () { registry.register({}, 1, registered); }),
        throwsTypeError(function () { registry.unregister(registered); })
    ].join("|");
})()
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "true|true|true|true|true|true|true|true|true|true|true"
        ))
    );
}

#[test]
fn weak_ref_dereferences_object_local_and_well_known_symbol_targets() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
var weakObjectTarget = { marker: 42 };
var weakLocalTarget = Symbol("local-target");
var weakObjectReference = new WeakRef(weakObjectTarget);
var weakLocalReference = new WeakRef(weakLocalTarget);
var weakWellKnownReference = new WeakRef(Symbol.iterator);
[
    weakObjectReference.deref() === weakObjectTarget,
    weakObjectReference.deref().marker === 42,
    weakLocalReference.deref() === weakLocalTarget,
    weakWellKnownReference.deref() === Symbol.iterator
].join("|");
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true|true|true"))
    );

    context
        .eval("weakObjectTarget = null; weakLocalTarget = null;")
        .unwrap();
    runtime.run_gc().unwrap();
    assert_eq!(
        context
            .eval(
                r#"[
    weakObjectReference.deref() === undefined,
    weakLocalReference.deref() === undefined,
    weakWellKnownReference.deref() === Symbol.iterator
].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true|true"))
    );
}

#[test]
fn weak_ref_temporaries_are_not_kept_alive_until_the_end_of_the_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
    new WeakRef({}).deref() === undefined,
    new WeakRef(Symbol("temporary")).deref() === undefined
].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true"))
    );
}

#[test]
fn finalization_registry_register_unregister_and_same_value_are_observable_from_js() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
var weakRegistrationLog = "";
var weakRegistry = new FinalizationRegistry(function (held) {
    weakRegistrationLog += held;
});
var weakObjectTarget = {};
var weakSecondObjectTarget = {};
var weakObjectToken = {};
var weakSymbolTarget = Symbol("target");
var weakSymbolToken = Symbol("token");
var weakSameObject = {};
var weakSameSymbol = Symbol("same");
var weakRegistrationResult = [
    weakRegistry.register(weakObjectTarget, "object", weakObjectToken) === undefined,
    weakRegistry.register(weakSecondObjectTarget, "second", weakObjectToken) === undefined,
    weakRegistry.register(weakSymbolTarget, "symbol", weakSymbolToken) === undefined,
    weakRegistry.unregister(weakObjectToken),
    weakRegistry.unregister(weakObjectToken) === false,
    weakRegistry.unregister(weakSymbolToken),
    weakRegistry.unregister(weakSymbolToken) === false,
    (function () {
        try {
            weakRegistry.register(weakSameObject, weakSameObject);
            return false;
        } catch (error) {
            return error instanceof TypeError;
        }
    })(),
    (function () {
        try {
            weakRegistry.register(weakSameSymbol, weakSameSymbol);
            return false;
        } catch (error) {
            return error instanceof TypeError;
        }
    })()
].join("|");
weakObjectTarget = null;
weakSecondObjectTarget = null;
weakObjectToken = null;
weakSymbolTarget = null;
weakSymbolToken = null;
weakSameObject = null;
weakSameSymbol = null;
weakRegistrationResult;
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "true|true|true|true|true|true|true|true|true"
        ))
    );

    runtime.run_gc().unwrap();
    assert!(!runtime.is_job_pending());
    assert_eq!(
        context.eval("weakRegistrationLog").unwrap(),
        Value::String(JsString::from_static(""))
    );
}

#[test]
fn finalization_jobs_share_the_promise_fifo() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    context
        .eval(
            r#"
var weakFifoLog = "";
var weakFifoThis = null;
var weakFifoRegistry = new FinalizationRegistry(function (held) {
    "use strict";
    weakFifoThis = this;
    weakFifoLog += "finalizer-" + held + ",";
});
var weakFifoTarget = {};
weakFifoRegistry.register(weakFifoTarget, "held");
Promise.resolve().then(function () { weakFifoLog += "promise-before,"; });
weakFifoTarget = null;
"#,
        )
        .unwrap();
    runtime.run_gc().unwrap();
    context
        .eval(
            r#"Promise.resolve().then(function () {
    weakFifoLog += "promise-after,";
});"#,
        )
        .unwrap();

    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static("promise-before,"))
    );
    assert_eq!(
        runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static("promise-before,finalizer-held,"))
    );
    assert_eq!(
        context.eval("weakFifoThis === undefined").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static(
            "promise-before,finalizer-held,promise-after,"
        ))
    );
    assert!(
        !runtime
            .execute_pending_job_with_context()
            .unwrap()
            .executed()
    );
}

#[test]
fn finalization_target_death_observes_mixed_weak_object_construction_order() {
    let one_pass_runtime = Runtime::new();
    let mut one_pass = one_pass_runtime.new_context();
    one_pass
        .eval(
            r#"
var onePassLog = "";
var onePassMap = new WeakMap();
var onePassRegistry = new FinalizationRegistry(function (held) {
    onePassLog += held;
});
var onePassKey = {};
var onePassTarget = {};
onePassMap.set(onePassKey, onePassTarget);
onePassRegistry.register(onePassTarget, "one-pass");
onePassKey = null;
onePassTarget = null;
"#,
        )
        .unwrap();

    one_pass_runtime.run_gc().unwrap();
    assert!(one_pass_runtime.is_job_pending());
    assert_eq!(
        one_pass_runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(one_pass.realm)
    );
    assert_eq!(
        one_pass.eval("onePassLog").unwrap(),
        Value::String(JsString::from_static("one-pass"))
    );
    assert!(!one_pass_runtime.is_job_pending());

    let two_pass_runtime = Runtime::new();
    let mut two_pass = two_pass_runtime.new_context();
    two_pass
        .eval(
            r#"
var twoPassLog = "";
var twoPassRegistry = new FinalizationRegistry(function (held) {
    twoPassLog += held;
});
var twoPassMap = new WeakMap();
var twoPassKey = {};
var twoPassTarget = {};
twoPassMap.set(twoPassKey, twoPassTarget);
twoPassRegistry.register(twoPassTarget, "two-pass");
twoPassKey = null;
twoPassTarget = null;
"#,
        )
        .unwrap();

    two_pass_runtime.run_gc().unwrap();
    assert!(!two_pass_runtime.is_job_pending());
    assert_eq!(
        two_pass.eval("twoPassLog").unwrap(),
        Value::String(JsString::from_static(""))
    );
    two_pass_runtime.run_gc().unwrap();
    assert!(two_pass_runtime.is_job_pending());
    assert_eq!(
        two_pass_runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(two_pass.realm)
    );
    assert_eq!(
        two_pass.eval("twoPassLog").unwrap(),
        Value::String(JsString::from_static("two-pass"))
    );
    assert!(!two_pass_runtime.is_job_pending());
}

#[test]
fn throwing_finalization_callback_does_not_discard_the_next_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
var weakThrowLog = "";
var weakThrowRegistry = new FinalizationRegistry(function (held) {
    weakThrowLog += held + ",";
    if (held === "first")
        throw "cleanup failed";
});
var weakThrowFirst = {};
var weakThrowSecond = {};
weakThrowRegistry.register(weakThrowFirst, "first");
weakThrowRegistry.register(weakThrowSecond, "second");
weakThrowFirst = null;
weakThrowSecond = null;
"#,
        )
        .unwrap();
    runtime.run_gc().unwrap();

    let error = runtime.execute_pending_job_with_context().unwrap_err();
    assert_eq!(error.context(), Some(context.realm));
    assert_eq!(error.error(), &RuntimeError::Exception);
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("cleanup failed")))
    );
    assert!(runtime.is_job_pending());
    assert_eq!(
        runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakThrowLog").unwrap(),
        Value::String(JsString::from_static("first,second,"))
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn unregister_cannot_cancel_an_already_queued_finalization_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
var queuedFinalizationLog = "";
var queuedFinalizationRegistry = new FinalizationRegistry(function (held) {
    queuedFinalizationLog += held;
});
var queuedFinalizationToken = {};
var queuedFinalizationTarget = {};
queuedFinalizationRegistry.register(
    queuedFinalizationTarget,
    "already-queued",
    queuedFinalizationToken
);
queuedFinalizationTarget = null;
"#,
        )
        .unwrap();

    runtime.run_gc().unwrap();
    assert!(runtime.is_job_pending());
    assert_eq!(
        context
            .eval("queuedFinalizationRegistry.unregister(queuedFinalizationToken)")
            .unwrap(),
        Value::Bool(false)
    );
    assert!(runtime.is_job_pending());
    assert_eq!(
        runtime
            .execute_pending_job_with_context()
            .unwrap()
            .context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("queuedFinalizationLog").unwrap(),
        Value::String(JsString::from_static("already-queued"))
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn weak_intrinsic_constructor_fallback_uses_the_new_target_realm() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();
    let weak_ref = global_callable(&runtime, &mut constructor_context, "WeakRef");
    let target_weak_ref = global_callable(&runtime, &mut target_context, "WeakRef");
    let finalization_registry =
        global_callable(&runtime, &mut constructor_context, "FinalizationRegistry");
    let target_finalization_registry =
        global_callable(&runtime, &mut target_context, "FinalizationRegistry");
    let Value::Object(constructor_weak_ref_prototype) =
        own_data_value(&runtime, weak_ref.as_object(), "prototype")
    else {
        panic!("constructor-realm WeakRef prototype was not an object");
    };
    let Value::Object(target_weak_ref_prototype) =
        own_data_value(&runtime, target_weak_ref.as_object(), "prototype")
    else {
        panic!("target-realm WeakRef prototype was not an object");
    };
    let Value::Object(constructor_registry_prototype) =
        own_data_value(&runtime, finalization_registry.as_object(), "prototype")
    else {
        panic!("constructor-realm FinalizationRegistry prototype was not an object");
    };
    let Value::Object(target_registry_prototype) = own_data_value(
        &runtime,
        target_finalization_registry.as_object(),
        "prototype",
    ) else {
        panic!("target-realm FinalizationRegistry prototype was not an object");
    };
    assert_ne!(constructor_weak_ref_prototype, target_weak_ref_prototype);
    assert_ne!(constructor_registry_prototype, target_registry_prototype);
    let Value::Object(new_target_object) = target_context
        .eval("(function ForeignWeakNewTarget(){})")
        .unwrap()
    else {
        panic!("foreign new.target source did not return a function");
    };
    let new_target = runtime
        .as_callable(&new_target_object)
        .unwrap()
        .expect("foreign new.target was not callable");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        target_context
            .define_own_property(
                &new_target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let weak_target = constructor_context.new_object().unwrap();
    let Value::Object(weak_instance) = constructor_context
        .construct_with_new_target(&weak_ref, &new_target, &[Value::Object(weak_target)])
        .unwrap()
    else {
        panic!("cross-realm WeakRef construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&weak_instance).unwrap(),
        Some(target_weak_ref_prototype)
    );

    let callback = eval_callable(
        &runtime,
        &mut constructor_context,
        "(function cleanupCallback(){})",
    );
    let Value::Object(registry_instance) = constructor_context
        .construct_with_new_target(
            &finalization_registry,
            &new_target,
            &[Value::Object(callback.as_object().clone())],
        )
        .unwrap()
    else {
        panic!("cross-realm FinalizationRegistry construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&registry_instance).unwrap(),
        Some(target_registry_prototype)
    );
}

#[cfg(feature = "test262-host")]
#[test]
fn dynamic_import_policy_rejects_precompiled_trees_before_instantiation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .compile("function hidden() { return import('fixture'); }")
        .unwrap();

    runtime.set_dynamic_import_bytecode_allowed(false);
    let RuntimeError::Engine(error) = context.execute(&function).unwrap_err() else {
        panic!("restricted precompiled tree did not return an engine policy error");
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(error.message().contains("dynamic-import bytecode policy"));

    runtime.set_dynamic_import_bytecode_allowed(true);
    assert_eq!(
        context
            .eval("Object.prototype.hasOwnProperty.call(globalThis, 'hidden')")
            .unwrap(),
        Value::Bool(false)
    );
}

#[cfg(feature = "test262-host")]
#[test]
fn dynamic_import_policy_vm_guard_precedes_specifier_conversion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            "var policyConversionCount = 0;\n\
             var policyImport = function () {\n\
               return import({ toString: function () { policyConversionCount++; return 'fixture'; } });\n\
             };",
        )
        .unwrap();
    let wrapper = context.compile("policyImport();").unwrap();

    runtime.set_dynamic_import_bytecode_allowed(false);
    let RuntimeError::Engine(error) = context.execute(&wrapper).unwrap_err() else {
        panic!("restricted instantiated callable did not return an engine policy error");
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(error.message().contains("dynamic-import bytecode policy"));

    runtime.set_dynamic_import_bytecode_allowed(true);
    assert_eq!(
        context.eval("policyConversionCount").unwrap(),
        Value::Int(0)
    );
    assert!(!runtime.is_job_pending());
}
