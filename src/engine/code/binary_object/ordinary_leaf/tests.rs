use super::super::graph::model::NodeId;
use super::super::wire::{BcTag, WireWriter};
use super::*;

// QuickJS 2026-06-04, JS_WriteObject(JS_WRITE_OBJ_BYTECODE) for:
// (function(a,b){var acc=.5;var step=b;while(a>0){if(a===2)
// acc=(acc+step)/1;else acc=(acc+1)/1;a=a-1;}return acc===5.5?42:0;})
const REAL_ORDINARY_LEAF_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c43020000020202020000022e040001000000010000000000",
    "0000010000bd00c7d0c8cfb3a3e81acfb5a9e809c3c49bb4",
    "99c7ea07c3b49bb499c7cfb49cd3eae3c3bd01a9e804bb2a",
    "28b32806000000000000e03f060000000000001640",
);
// Property-free raw49/subtype0 wire mechanically reduced from pinned
// QuickJS output for `(function(){'use strict';const x=0;x=1;})`.
const READ_ONLY_LEAF_HEX: &str = concat!(
    "050102780c000200a801000100010000",
    "01040100000000be00cb280c43020100",
    "00000000000000060031f300000000",
);
const NATURAL_READ_ONLY_LEAF_HEX: &str = concat!(
    "050102780c000200a801000100010000",
    "01040100000000be00cb280c43020100",
    "000100020000000d01000000b05e0000",
    "b3c7b41131f300000000",
);
const MINIMAL_FUNCTION_RECORD: [u8; 23] = [
    0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00,
    0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
];
const NATURAL_STRICT_PUSH_THIS_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c430201000001000100000004010001000008c7c328",
);
const NATURAL_SLOPPY_PUSH_THIS_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c430200000001000100000004010001000008c7c328",
);
const MINIMAL_STRICT_PUSH_THIS_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c430201000000000100000002000828",
);
const MINIMAL_SLOPPY_PUSH_THIS_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c430200000000000100000002000828",
);
const MINIMAL_STRICT_TO_PROPKEY_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c4302010001000101000000030100010000cf7028",
);
const MINIMAL_SLOPPY_TO_PROPKEY_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c4302000001000101000000030100010000cf7028",
);
const DUPLICATE_TO_PROPKEY_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c4302010001000101000000040100010000cf707028",
);
const REENTER_TO_PROPKEY_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c430201000200020200000012020001000000010000",
    "cf70d0680d0000000e09d4cf6af4ffffff28",
);
const UNDERFLOW_TO_PROPKEY_HEX: &str = concat!(
    "05000c000200a80100010001000001040100000000be00cb28",
    "0c43020100010001010000000201000100007028",
);

fn bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => panic!("test vector must be hexadecimal"),
            };
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect()
}

fn oracle() -> Vec<u8> {
    let object = bytes(REAL_ORDINARY_LEAF_HEX);
    assert_eq!(object.len(), 119);
    object
}

fn decode(input: &[u8]) -> Result<OrdinaryLeafDraft, OrdinaryLeafReadError> {
    decode_trusted_ordinary_leaf(input, RootFunctionConstantSelector::from_zero_based(0))
}

fn lower_ready(operation: FunctionOp<'_>) -> OrdinaryLeafOp {
    lower_operation(&operation, 4, 4, 4, 8).unwrap()
}

fn encode_primitive(value: &WireValue) -> Vec<u8> {
    let mut writer = WireWriter::new(256);
    match value {
        WireValue::Undefined => writer.write_tag(BcTag::Undefined).unwrap(),
        WireValue::Null => writer.write_tag(BcTag::Null).unwrap(),
        WireValue::Bool(false) => writer.write_tag(BcTag::BoolFalse).unwrap(),
        WireValue::Bool(true) => writer.write_tag(BcTag::BoolTrue).unwrap(),
        WireValue::Int32(value) => {
            writer.write_tag(BcTag::Int32).unwrap();
            writer.write_i32(*value).unwrap();
        }
        WireValue::Float64Bits(bits) => {
            writer.write_tag(BcTag::Float64).unwrap();
            writer.write_u64_le(*bits).unwrap();
        }
        WireValue::String(value) => {
            writer.write_tag(BcTag::String).unwrap();
            writer.write_string(value).unwrap();
        }
        WireValue::BigInt(bytes) => {
            writer.write_tag(BcTag::BigInt).unwrap();
            writer.write_uleb128(bytes.len() as u32).unwrap();
            writer.write_bytes(bytes).unwrap();
        }
        WireValue::Node(_) => panic!("node is not a primitive constant entry"),
    }
    writer.into_bytes()
}

fn oracle_with_both_constants(value: &WireValue) -> Vec<u8> {
    let entry = encode_primitive(value);
    let mut object = oracle();
    object.truncate(101);
    object.extend_from_slice(&entry);
    object.extend_from_slice(&entry);
    object
}

#[test]
fn lowers_the_real_pinned_quickjs_leaf_with_ir_control_flow_targets() {
    let draft = decode(&oracle()).expect("pinned QuickJS ordinary leaf must be admitted");
    assert_eq!(
        draft.metadata(),
        OrdinaryLeafMetadataDraft {
            argument_count: 2,
            defined_argument_count: 2,
            local_count: 2,
            max_stack: 2,
            is_strict: false,
            has_simple_parameter_list: true,
            has_prototype: true,
            allows_new_target: true,
            allows_arguments: true,
            strip_variable_debug: true,
        }
    );
    assert_eq!(
        draft.constants(),
        &[
            DetachedPrimitive::Float64Bits(0.5_f64.to_bits()),
            DetachedPrimitive::Float64Bits(5.5_f64.to_bits()),
        ]
    );
    assert_eq!(
        draft.code(),
        &[
            OrdinaryLeafOp::PushConst(0),
            OrdinaryLeafOp::PutLocal(0),
            OrdinaryLeafOp::GetArgument(1),
            OrdinaryLeafOp::PutLocal(1),
            OrdinaryLeafOp::GetArgument(0),
            OrdinaryLeafOp::PushI32(0),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::GreaterThan),
            OrdinaryLeafOp::IfFalse(30),
            OrdinaryLeafOp::GetArgument(0),
            OrdinaryLeafOp::PushI32(2),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual),
            OrdinaryLeafOp::IfFalse(19),
            OrdinaryLeafOp::GetLocal(0),
            OrdinaryLeafOp::GetLocal(1),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Add),
            OrdinaryLeafOp::PushI32(1),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Div),
            OrdinaryLeafOp::PutLocal(0),
            OrdinaryLeafOp::Goto(25),
            OrdinaryLeafOp::GetLocal(0),
            OrdinaryLeafOp::PushI32(1),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Add),
            OrdinaryLeafOp::PushI32(1),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Div),
            OrdinaryLeafOp::PutLocal(0),
            OrdinaryLeafOp::GetArgument(0),
            OrdinaryLeafOp::PushI32(1),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Sub),
            OrdinaryLeafOp::PutArgument(0),
            OrdinaryLeafOp::Goto(4),
            OrdinaryLeafOp::GetLocal(0),
            OrdinaryLeafOp::PushConst(1),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual),
            OrdinaryLeafOp::IfFalse(36),
            OrdinaryLeafOp::PushI32(42),
            OrdinaryLeafOp::Return,
            OrdinaryLeafOp::PushI32(0),
            OrdinaryLeafOp::Return,
        ]
    );
}

#[test]
fn lowers_property_free_read_only_with_owned_input_atom_spelling() {
    let object = bytes(READ_ONLY_LEAF_HEX);
    assert_eq!(object.len(), 47);
    let draft = decode(&object).expect("property-free raw49 leaf must be admitted");
    assert_eq!(draft.metadata().local_count(), 0);
    assert_eq!(draft.metadata().max_stack(), 0);
    assert!(draft.constants().is_empty());
    let [OrdinaryLeafOp::ThrowReadOnly(name)] = draft.code() else {
        panic!("raw49 did not lower to its owned read-only operation");
    };
    assert_eq!(name.0.as_ref(), "x".encode_utf16().collect::<Vec<_>>());
}

#[test]
fn read_only_rejects_other_subtypes_non_string_atoms_and_atom_table_drift() {
    let original = bytes(READ_ONLY_LEAF_HEX);

    for subtype in [1, 2, 3, 4, 5, u8::MAX] {
        let mut object = original.clone();
        object[46] = subtype;
        let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&object) else {
            panic!("throw_error subtype {subtype} was admitted");
        };
        assert!(
            message.contains("admitted read-only subtype 0"),
            "{message}"
        );
    }

    for (label, raw_atom) in [
        ("null", 0_u32),
        ("index", 0x8000_002a),
        ("private", 229),
        ("symbol", 230),
    ] {
        let mut object = original.clone();
        object[42..46].copy_from_slice(&raw_atom.to_le_bytes());
        let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&object) else {
            panic!("{label} read-only atom was admitted");
        };
        assert!(message.contains("not a String name"), "{label}: {message}");
    }

    let mut unused = original.clone();
    unused[39] = 1;
    unused.truncate(41);
    unused.push(0x29); // return_undef, leaving the sole header atom unused
    let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&unused) else {
        panic!("unused input atom slot was admitted");
    };
    assert!(message.contains("not used by an admitted read-only diagnostic"));

    let mut multiple = original;
    multiple[1] = 2;
    multiple.splice(4..4, [0x02, b'y']);
    let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&multiple) else {
        panic!("multiple input atom slots were admitted");
    };
    assert!(message.contains("instead of at most one"));
}

#[test]
fn read_only_accepts_only_string_names_under_zero_or_one_slot_provenance() {
    let original = bytes(READ_ONLY_LEAF_HEX);

    let mut predefined = original.clone();
    predefined[1] = 0;
    predefined.drain(2..4);
    // With the header removed, raw50 is the pinned `length` String atom.
    predefined[40..44].copy_from_slice(&50_u32.to_le_bytes());
    let draft = decode(&predefined).expect("predefined String needs no input atom slot");
    let [OrdinaryLeafOp::ThrowReadOnly(name)] = draft.code() else {
        panic!("predefined read-only atom did not lower");
    };
    assert_eq!(name.0.as_ref(), "length".encode_utf16().collect::<Vec<_>>());

    let mut manifest_alias = original.clone();
    manifest_alias.splice(2..4, [0x0c, b'l', b'e', b'n', b'g', b't', b'h']);
    let draft =
        decode(&manifest_alias).expect("the sole header slot may intern to a predefined String");
    let [OrdinaryLeafOp::ThrowReadOnly(name)] = draft.code() else {
        panic!("manifest-alias read-only atom did not lower");
    };
    assert_eq!(name.0.as_ref(), "length".encode_utf16().collect::<Vec<_>>());

    let mut decimal_alias = original;
    decimal_alias.splice(2..4, [0x04, b'4', b'2']);
    let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&decimal_alias) else {
        panic!("a decimal header alias was admitted as a String name");
    };
    assert!(message.contains("not a String name"));
}

#[test]
fn natural_read_only_wire_remains_outside_the_nonlexical_leaf_cohort() {
    let object = bytes(NATURAL_READ_ONLY_LEAF_HEX);
    assert_eq!(object.len(), 58);
    assert!(matches!(
        decode(&object),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));
}

#[test]
fn lowers_representative_sanitized_operations_without_consulting_diagnostics() {
    let cases = [
        (FunctionOp::Nop, OrdinaryLeafOp::Nop),
        (FunctionOp::Object, OrdinaryLeafOp::Object),
        (FunctionOp::ToObject, OrdinaryLeafOp::ToObject),
        (FunctionOp::ToPropKey, OrdinaryLeafOp::ToPropKey),
        (FunctionOp::PushThis, OrdinaryLeafOp::PushThis),
        (
            FunctionOp::PushI32(i32::MIN),
            OrdinaryLeafOp::PushI32(i32::MIN),
        ),
        (FunctionOp::PushConstant(3), OrdinaryLeafOp::PushConst(3)),
        (FunctionOp::GetLocal(3), OrdinaryLeafOp::GetLocal(3)),
        (FunctionOp::PutLocal(2), OrdinaryLeafOp::PutLocal(2)),
        (FunctionOp::SetLocal(1), OrdinaryLeafOp::SetLocal(1)),
        (FunctionOp::GetArgument(3), OrdinaryLeafOp::GetArgument(3)),
        (FunctionOp::PutArgument(2), OrdinaryLeafOp::PutArgument(2)),
        (FunctionOp::SetArgument(1), OrdinaryLeafOp::SetArgument(1)),
        (
            FunctionOp::Binary(FunctionBinaryOp::Add),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Add),
        ),
        (
            FunctionOp::Binary(FunctionBinaryOp::Sub),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Sub),
        ),
        (
            FunctionOp::Binary(FunctionBinaryOp::Div),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Div),
        ),
        (
            FunctionOp::Binary(FunctionBinaryOp::GreaterThan),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::GreaterThan),
        ),
        (
            FunctionOp::Binary(FunctionBinaryOp::StrictEqual),
            OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual),
        ),
        (FunctionOp::IfFalse(7), OrdinaryLeafOp::IfFalse(7)),
        (FunctionOp::IfTrue(7), OrdinaryLeafOp::IfTrue(7)),
        (FunctionOp::Goto(0), OrdinaryLeafOp::Goto(0)),
        (FunctionOp::Return, OrdinaryLeafOp::Return),
        (FunctionOp::ReturnUndefined, OrdinaryLeafOp::ReturnUndefined),
        (FunctionOp::Throw, OrdinaryLeafOp::Throw),
    ];
    for (operation, expected) in cases {
        assert_eq!(lower_ready(operation), expected);
    }

    assert!(matches!(
        lower_operation(&FunctionOp::PushConstant(4), 4, 4, 4, 8,),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));
    assert!(matches!(
        lower_operation(&FunctionOp::GetLocal(4), 4, 4, 4, 8,),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));
    assert!(matches!(
        lower_operation(&FunctionOp::Goto(8), 4, 4, 4, 8,),
        Err(OrdinaryLeafReadError::Internal(_))
    ));
}

#[test]
fn push_this_wires_preserve_strictness_source_order_and_one_to_one_lowering() {
    for (label, hex, expected_strict, expected_code) in [
        (
            "natural strict",
            NATURAL_STRICT_PUSH_THIS_HEX,
            true,
            vec![
                OrdinaryLeafOp::PushThis,
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::Return,
            ],
        ),
        (
            "natural sloppy",
            NATURAL_SLOPPY_PUSH_THIS_HEX,
            false,
            vec![
                OrdinaryLeafOp::PushThis,
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::Return,
            ],
        ),
        (
            "minimal strict",
            MINIMAL_STRICT_PUSH_THIS_HEX,
            true,
            vec![OrdinaryLeafOp::PushThis, OrdinaryLeafOp::Return],
        ),
        (
            "minimal sloppy",
            MINIMAL_SLOPPY_PUSH_THIS_HEX,
            false,
            vec![OrdinaryLeafOp::PushThis, OrdinaryLeafOp::Return],
        ),
    ] {
        let wire = bytes(hex);
        let draft = decode(&wire).unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(draft.metadata().is_strict(), expected_strict, "{label}");
        assert_eq!(draft.metadata().max_stack(), 1, "{label}");
        assert_eq!(draft.code(), expected_code, "{label}");
        assert!(draft.constants().is_empty(), "{label}");
    }
}

#[test]
fn push_this_protocol_rejects_duplicate_nonzero_and_branch_target_zero() {
    let base = bytes(MINIMAL_STRICT_PUSH_THIS_HEX);

    let mut duplicate = base.clone();
    duplicate[37] = 3;
    duplicate.insert(40, 8);

    let mut nonzero = base.clone();
    nonzero[37] = 3;
    nonzero.insert(39, 177);

    let mut branch_target_zero = base;
    branch_target_zero[37] = 4;
    branch_target_zero.truncate(39);
    branch_target_zero.extend_from_slice(&[8, 234, (-2_i8) as u8, 40]);

    for (label, wire, expected) in [
        ("duplicate", duplicate, "push_this must occur exactly once"),
        (
            "nonzero",
            nonzero,
            "push_this must be typed instruction zero",
        ),
        (
            "branch target zero",
            branch_target_zero,
            "must not explicitly target the push_this prologue",
        ),
    ] {
        let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&wire) else {
            panic!("{label} push_this protocol violation was admitted");
        };
        assert!(message.contains(expected), "{label}: {message}");
    }
}

#[test]
fn push_this_protocol_preserves_raw8_absent_branch_target_zero() {
    let mut no_push_this = bytes(MINIMAL_STRICT_PUSH_THIS_HEX);
    no_push_this[37] = 4;
    no_push_this.truncate(39);
    no_push_this.extend_from_slice(&[177, 234, (-2_i8) as u8, 41]);

    let draft = decode(&no_push_this).unwrap();
    assert_eq!(
        draft.code(),
        [
            OrdinaryLeafOp::Nop,
            OrdinaryLeafOp::Goto(0),
            OrdinaryLeafOp::ReturnUndefined,
        ]
    );
}

#[test]
fn to_propkey_wires_preserve_strictness_and_one_to_one_lowering() {
    for (label, hex, expected_strict) in [
        ("minimal strict", MINIMAL_STRICT_TO_PROPKEY_HEX, true),
        ("minimal sloppy", MINIMAL_SLOPPY_TO_PROPKEY_HEX, false),
    ] {
        let wire = bytes(hex);
        assert_eq!(wire.len(), 46, "{label}");
        let draft = decode(&wire).unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(draft.metadata().is_strict(), expected_strict, "{label}");
        assert_eq!(draft.metadata().max_stack(), 1, "{label}");
        assert_eq!(
            draft.code(),
            [
                OrdinaryLeafOp::GetArgument(0),
                OrdinaryLeafOp::ToPropKey,
                OrdinaryLeafOp::Return,
            ],
            "{label}"
        );
        assert!(draft.constants().is_empty(), "{label}");
    }
}

#[test]
fn to_propkey_duplicate_and_finite_loop_reentry_are_ordinary_verified() {
    let duplicate = decode(&bytes(DUPLICATE_TO_PROPKEY_HEX)).unwrap();
    assert_eq!(
        duplicate.code(),
        [
            OrdinaryLeafOp::GetArgument(0),
            OrdinaryLeafOp::ToPropKey,
            OrdinaryLeafOp::ToPropKey,
            OrdinaryLeafOp::Return,
        ]
    );
    assert_eq!(duplicate.metadata().max_stack(), 1);
    assert!(duplicate.constants().is_empty());

    let reentry = decode(&bytes(REENTER_TO_PROPKEY_HEX)).unwrap();
    assert_eq!(
        reentry.code(),
        [
            OrdinaryLeafOp::GetArgument(0),
            OrdinaryLeafOp::ToPropKey,
            OrdinaryLeafOp::GetArgument(1),
            OrdinaryLeafOp::IfFalse(9),
            OrdinaryLeafOp::Stack(OrdinaryLeafStackOp::Drop),
            OrdinaryLeafOp::PushBool(false),
            OrdinaryLeafOp::PutArgument(1),
            OrdinaryLeafOp::GetArgument(0),
            OrdinaryLeafOp::Goto(1),
            OrdinaryLeafOp::Return,
        ]
    );
    assert_eq!(reentry.metadata().max_stack(), 2);
    assert!(reentry.constants().is_empty());
}

#[test]
fn to_propkey_underflow_reaches_the_existing_ordinary_verifier_unchanged() {
    let draft = decode(&bytes(UNDERFLOW_TO_PROPKEY_HEX)).unwrap();
    assert_eq!(
        draft.code(),
        [OrdinaryLeafOp::ToPropKey, OrdinaryLeafOp::Return]
    );
    assert!(draft.constants().is_empty());
}

#[test]
fn plain_call_argument_count_reaches_the_ordinary_dto_unchanged() {
    for argument_count in 0..=4 {
        assert_eq!(
            lower_ready(FunctionOp::Call(argument_count)),
            OrdinaryLeafOp::Call(argument_count)
        );
    }
}

#[test]
fn non_tail_invocation_operands_reach_the_ordinary_dto_unchanged() {
    for (operation, expected) in [
        (
            FunctionOp::Construct(65_535),
            OrdinaryLeafOp::Construct(65_535),
        ),
        (
            FunctionOp::CallMethod(65_535),
            OrdinaryLeafOp::CallMethod(65_535),
        ),
        (
            FunctionOp::ArrayFrom(65_535),
            OrdinaryLeafOp::ArrayFrom(65_535),
        ),
    ] {
        assert_eq!(lower_ready(operation), expected);
    }
}

#[test]
fn tail_invocation_operands_reach_the_ordinary_dto_unchanged() {
    for (operation, expected) in [
        (
            FunctionOp::TailCall(u16::MAX),
            OrdinaryLeafOp::TailCall(u16::MAX),
        ),
        (
            FunctionOp::TailCallMethod(u16::MAX),
            OrdinaryLeafOp::TailCallMethod(u16::MAX),
        ),
    ] {
        assert_eq!(lower_ready(operation), expected);
    }
}

#[test]
fn apply_kind_reaches_the_ordinary_dto_without_raw_magic() {
    for (operation, expected) in [
        (
            FunctionOp::Apply(FunctionApplyKind::Call),
            OrdinaryLeafOp::Apply(OrdinaryLeafApplyKind::Call),
        ),
        (
            FunctionOp::Apply(FunctionApplyKind::Construct),
            OrdinaryLeafOp::Apply(OrdinaryLeafApplyKind::Construct),
        ),
    ] {
        assert_eq!(lower_ready(operation), expected);
    }
}

#[test]
fn root_constant_selector_authenticates_the_child_without_fixing_an_image_id() {
    assert!(matches!(
        decode_trusted_ordinary_leaf(&oracle(), RootFunctionConstantSelector::from_zero_based(1)),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    let mut non_function = oracle();
    // Replace the root constant's FunctionBytecode tag with Null. The
    // compatible reader deliberately accepts the now-trailing child bytes;
    // selection must still reject the non-function constant.
    non_function[25] = 0x01;
    assert!(matches!(
        decode(&non_function),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    assert!(matches!(
        decode(&[0x05, 0x00, 0x05, 0x54]),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    // Insert a complete sibling before the target in root cpool order.
    // The same target is now FunctionId 2 instead of FunctionId 1; only
    // the root selector changes, and no image identity crosses the API.
    let mut reindexed = oracle();
    reindexed[14] = 0x02;
    reindexed.splice(25..25, MINIMAL_FUNCTION_RECORD);
    let reindexed =
        decode_trusted_ordinary_leaf(&reindexed, RootFunctionConstantSelector::from_zero_based(1))
            .unwrap();
    assert_eq!(reindexed.code()[34], OrdinaryLeafOp::PushI32(42));
}

#[test]
fn detaches_every_runtime_independent_primitive_without_losing_bits() {
    let cases = [
        (WireValue::Undefined, DetachedPrimitive::Undefined),
        (WireValue::Null, DetachedPrimitive::Null),
        (WireValue::Bool(false), DetachedPrimitive::Bool(false)),
        (WireValue::Bool(true), DetachedPrimitive::Bool(true)),
        (WireValue::Int32(i32::MIN), DetachedPrimitive::Int(i32::MIN)),
        (
            WireValue::Float64Bits(0x8000_0000_0000_0000),
            DetachedPrimitive::Float64Bits(0x8000_0000_0000_0000),
        ),
        (
            WireValue::Float64Bits(0xfff8_0000_0000_0042),
            DetachedPrimitive::Float64Bits(0xfff8_0000_0000_0042),
        ),
        (
            WireValue::String(WireString::Narrow(Box::from([0x00, 0xff]))),
            DetachedPrimitive::String(Box::from([0x0000, 0x00ff])),
        ),
        (
            WireValue::String(WireString::Wide(Box::from([
                0x0100, 0x0000, 0xd800, 0xdc00,
            ]))),
            DetachedPrimitive::String(Box::from([0x0100, 0x0000, 0xd800, 0xdc00])),
        ),
        (
            WireValue::BigInt(Box::from([])),
            DetachedPrimitive::BigIntSignedLeCanonical(Box::from([])),
        ),
        (
            WireValue::BigInt(Box::from([0x00, 0x80])),
            DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0x00, 0x80])),
        ),
        (
            WireValue::BigInt(Box::from([0xff, 0x7f])),
            DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0xff, 0x7f])),
        ),
    ];
    for (wire, expected) in cases {
        assert_eq!(project_primitive(&wire), Ok(expected.clone()));
        let draft = decode(&oracle_with_both_constants(&wire)).unwrap();
        assert_eq!(draft.constants(), &[expected.clone(), expected]);
    }

    // Compatible whole-image decoding normalizes redundant BigInt sign
    // extension before the detached primitive crosses this boundary.
    let draft = decode(&oracle_with_both_constants(&WireValue::BigInt(Box::from(
        [0x01, 0x00],
    ))))
    .unwrap();
    assert_eq!(
        draft.constants(),
        &[
            DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0x01])),
            DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0x01])),
        ]
    );
    assert!(matches!(
        project_primitive(&WireValue::Node(NodeId::from_zero_based(0))),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));
}

#[test]
fn rejects_metadata_outside_the_structural_leaf_cohort() {
    let mutations = [
        (26, 0x42), // no prototype
        (26, 0x53), // generator kind
        (28, 0x04), // async JS mode
        (29, 0x01), // non-null function name
        (32, 0x01), // defined arguments differ from arguments
        (34, 0x01), // captured variable-reference count
        (39, 0x01), // named local descriptor
        (42, 0x01), // non-zero local flags
    ];
    for (offset, replacement) in mutations {
        let mut object = oracle();
        object[offset] = replacement;
        assert!(
            matches!(decode(&object), Err(OrdinaryLeafReadError::Unadmitted(_))),
            "metadata mutation at byte {offset} must be rejected"
        );
    }

    let mut strict = oracle();
    strict[28] = 0x01;
    assert!(decode(&strict).unwrap().metadata().is_strict());
}

#[test]
fn rejects_outside_opcodes_operands_and_native_cfg_targets() {
    let mut unsupported_opcode = oracle();
    unsupported_opcode[72] = 0xa5; // instanceof in place of add
    assert_eq!(
            decode(&unsupported_opcode),
            Err(OrdinaryLeafReadError::Unadmitted(
                "native operation instanceof with None operands is outside the admitted ordinary-leaf cohort"
                    .into()
            ))
        );

    let mut scalar_only_opcode = oracle();
    scalar_only_opcode[55] = 0x04; // replace push_const with equal-width push_atom_value
    assert_eq!(
            decode(&scalar_only_opcode),
            Err(OrdinaryLeafReadError::Unadmitted(
                "native operation push_atom_value with Atom operands is outside the admitted ordinary-leaf cohort"
                    .into()
            ))
        );

    let mut constant_out_of_bounds = oracle();
    constant_out_of_bounds[56] = 0x02;
    assert!(matches!(
        decode(&constant_out_of_bounds),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    let mut argument_out_of_bounds = oracle();
    argument_out_of_bounds[60] = 0xd2; // get_arg3 in a two-argument leaf
    assert!(matches!(
        decode(&argument_out_of_bounds),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    let mut local_out_of_bounds = oracle();
    local_out_of_bounds[70] = 0xc6; // get_loc3 in a two-local leaf
    assert!(matches!(
        decode(&local_out_of_bounds),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    let mut non_boundary_label = oracle();
    non_boundary_label[64] = 0x05;
    assert!(matches!(
        decode(&non_boundary_label),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    let mut opaque_constant = oracle();
    let second_float = opaque_constant[110..119].to_vec();
    opaque_constant.truncate(101);
    opaque_constant.extend_from_slice(&MINIMAL_FUNCTION_RECORD);
    opaque_constant.extend_from_slice(&second_float);
    assert!(matches!(
        decode(&opaque_constant),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));

    let mut object_constant = oracle();
    let second_float = object_constant[110..119].to_vec();
    object_constant.truncate(101);
    object_constant.extend_from_slice(&[BcTag::Object.to_byte(), 0x00]);
    object_constant.extend_from_slice(&second_float);
    assert!(matches!(
        decode(&object_constant),
        Err(OrdinaryLeafReadError::Unadmitted(_))
    ));
}

#[test]
fn admission_is_structural_not_a_source_hash_or_exact_byte_vector() {
    let mut different_immediate = oracle();
    different_immediate[97] = 41;
    let draft = decode(&different_immediate).unwrap();
    assert_eq!(draft.code()[34], OrdinaryLeafOp::PushI32(41));

    let mut different_float_bits = oracle();
    different_float_bits[102..110].copy_from_slice(&(-0.0_f64).to_bits().to_le_bytes());
    let draft = decode(&different_float_bits).unwrap();
    assert_eq!(
        draft.constants()[0],
        DetachedPrimitive::Float64Bits((-0.0_f64).to_bits())
    );

    let mut different_scope_link = oracle();
    different_scope_link[40] = 0x02;
    assert!(decode(&different_scope_link).is_ok());

    let mut trailing = oracle();
    trailing.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    assert!(decode(&trailing).is_ok());
}

#[test]
fn preserves_reader_and_resource_error_classes_before_cohort_admission() {
    assert!(matches!(
        decode(&oracle()[..118]),
        Err(OrdinaryLeafReadError::Malformed(_))
    ));
    assert!(matches!(
        decode(&vec![0; MAX_INPUT_BYTES + 1]),
        Err(OrdinaryLeafReadError::Resource(_))
    ));
}
