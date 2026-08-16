use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::code::{CodeError, CodeLimits, CodeResourceKind};
use super::super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits};
use super::super::graph::decode::{DataMachine, DecodeError};
use super::super::graph::model::{AtomId, GraphLimits, NodeId, WireNodeCarrier, WireValue};
use super::super::pinned_atoms::{PinnedAtomId, PinnedAtomKind};
use super::super::wire::{
    BcTag, ReaderMode, ResourceKind, WireCursor, WireError, WireLimits, WireString, WireWriter,
};
use super::{
    BytecodeImageBudgetError, BytecodeImageEncodeError, BytecodeImageEncodeOptions,
    BytecodeImageError, BytecodeImageLimits, BytecodeImageResourceKind, FunctionId, ImageAtom,
    ImageAtomError, ImageAtomTable, ImageKey, ImageValue, decode_bytecode_image,
    encode_bytecode_image,
};

const TEST_LIMITS: WireLimits = WireLimits::new(4096, 32, 128, 512);
const GRAPH_LIMITS: GraphLimits = GraphLimits::new(128, 128, 64, 256, 1024, 4096, 4096, 4096, 4096);
const ENVELOPE_LIMITS: FunctionEnvelopeLimits = FunctionEnvelopeLimits::new(
    256,
    256,
    256,
    4096,
    4096,
    8192,
    CodeLimits::new(4096, 4096, 4096),
);
const IMAGE_LIMITS: BytecodeImageLimits = BytecodeImageLimits::new(
    GRAPH_LIMITS,
    ENVELOPE_LIMITS,
    256,
    256,
    4096,
    4096,
    4096,
    16384,
    16384,
    16384,
    16384,
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

fn decode_image(input: &[u8]) -> Result<super::BytecodeImage, BytecodeImageError> {
    decode_image_with(input, ReaderMode::Strict, IMAGE_LIMITS, true)
}

fn encode_image(image: &super::BytecodeImage) -> Result<Vec<u8>, BytecodeImageEncodeError> {
    encode_bytecode_image(
        image,
        BytecodeImageEncodeOptions::new(true, 65536, IMAGE_LIMITS),
    )
}

fn decode_image_with(
    input: &[u8],
    mode: ReaderMode,
    limits: BytecodeImageLimits,
    references: bool,
) -> Result<super::BytecodeImage, BytecodeImageError> {
    decode_bytecode_image(input, mode, TEST_LIMITS, limits, references)
}

fn bounded_image_limits(
    functions: usize,
    whole_depth: usize,
    constant_pool_entries: usize,
    code_bytes: usize,
) -> BytecodeImageLimits {
    BytecodeImageLimits::new(
        GRAPH_LIMITS,
        ENVELOPE_LIMITS,
        functions,
        whole_depth,
        constant_pool_entries,
        4096,
        4096,
        code_bytes,
        16384,
        16384,
        16384,
    )
}

fn one_aggregate_limit(
    envelope: FunctionEnvelopeLimits,
    kind: BytecodeImageResourceKind,
    limit: usize,
) -> BytecodeImageLimits {
    assert!(matches!(
        kind,
        BytecodeImageResourceKind::TotalConstantPoolEntries
            | BytecodeImageResourceKind::TotalLocalVariables
            | BytecodeImageResourceKind::TotalClosureVariables
            | BytecodeImageResourceKind::TotalCodeBytes
            | BytecodeImageResourceKind::TotalInstructions
            | BytecodeImageResourceKind::TotalAtomRelocations
            | BytecodeImageResourceKind::TotalDebugBytes
    ));
    BytecodeImageLimits::new(
        GRAPH_LIMITS,
        envelope,
        256,
        256,
        if kind == BytecodeImageResourceKind::TotalConstantPoolEntries {
            limit
        } else {
            4096
        },
        if kind == BytecodeImageResourceKind::TotalLocalVariables {
            limit
        } else {
            4096
        },
        if kind == BytecodeImageResourceKind::TotalClosureVariables {
            limit
        } else {
            4096
        },
        if kind == BytecodeImageResourceKind::TotalCodeBytes {
            limit
        } else {
            16384
        },
        if kind == BytecodeImageResourceKind::TotalInstructions {
            limit
        } else {
            16384
        },
        if kind == BytecodeImageResourceKind::TotalAtomRelocations {
            limit
        } else {
            16384
        },
        if kind == BytecodeImageResourceKind::TotalDebugBytes {
            limit
        } else {
            16384
        },
    )
}

fn quickjs_42_record() -> Vec<u8> {
    bytes("0c000200a80100010001000000040100000000bb2acb28")
}

fn sibling_function_array(record: &[u8]) -> Vec<u8> {
    let mut image = vec![5, 0, BcTag::Array.to_byte(), 2];
    image.extend_from_slice(record);
    image.extend_from_slice(record);
    image
}

fn constant_record() -> Vec<u8> {
    let mut record = quickjs_42_record();
    record[12] = 1;
    record.push(BcTag::Null.to_byte());
    record
}

fn closure_record() -> Vec<u8> {
    let mut record = quickjs_42_record();
    record[11] = 1;
    record.splice(19..19, [0xa8, 0x01, 0, 0, 0]);
    record
}

fn atom_relocation_record() -> Vec<u8> {
    let mut record = quickjs_42_record();
    record[13] = 6;
    record.truncate(19);
    // push_atom_value JS_ATOM_null; return
    record.extend_from_slice(&[4, 1, 0, 0, 0, 0x28]);
    record
}

fn debug_record() -> Vec<u8> {
    let mut record = quickjs_42_record();
    record[2] |= 4;
    // filename, one pc2line byte, and one source byte.
    record.extend_from_slice(&[0xa8, 0x01, 1, 0xaa, 1, 0xbb]);
    record
}

fn function_id(value: &ImageValue) -> FunctionId {
    value
        .function_id()
        .expect("test value must be a function identity")
}

fn node_id(value: &ImageValue) -> NodeId {
    match value.as_wire().expect("test value must be data") {
        WireValue::Node(node) => *node,
        value => panic!("test value must be a node, got {value:?}"),
    }
}

fn pinned(raw: u32) -> PinnedAtomId {
    PinnedAtomId::from_raw(raw).expect("test atom must be release-pinned")
}

fn ordinary_pinned(spelling: &str) -> PinnedAtomId {
    (1..=242)
        .filter_map(PinnedAtomId::from_raw)
        .find(|atom| atom.kind() == PinnedAtomKind::String && atom.spelling() == spelling)
        .expect("test spelling must have an ordinary release-pinned identity")
}

fn narrow(value: &[u8]) -> WireString {
    WireString::Narrow(value.to_vec().into_boxed_slice())
}

fn wide(value: &[u16]) -> WireString {
    WireString::Wide(value.to_vec().into_boxed_slice())
}

fn ascii_wide(value: &[u8]) -> WireString {
    wide(
        &value
            .iter()
            .map(|&byte| u16::from(byte))
            .collect::<Vec<_>>(),
    )
}

fn header_bytes(strings: &[WireString], trailing: &[u8]) -> Vec<u8> {
    let mut writer = WireWriter::new(4096);
    writer
        .write_header(u32::try_from(strings.len()).unwrap())
        .unwrap();
    for string in strings {
        writer.write_string(string).unwrap();
    }
    writer.write_bytes(trailing).unwrap();
    writer.into_bytes()
}

fn read_table(input: &[u8]) -> (ImageAtomTable, WireCursor<'_>) {
    let mut cursor = WireCursor::new(input, ReaderMode::Strict, TEST_LIMITS).unwrap();
    let table = ImageAtomTable::read(&mut cursor).unwrap();
    (table, cursor)
}

#[test]
fn mixed_header_aliases_match_quickjs_atom_interning() {
    let strings = [
        narrow(b"42"),
        ascii_wide(b"42"),
        narrow(b"length"),
        ascii_wide(b"length"),
        narrow(&[0xe9]),
        wide(&[0xe9]),
        narrow(&[0xc3, 0xa9]),
        narrow(b"2147483648"),
        ascii_wide(b"2147483648"),
    ];
    let bytes = header_bytes(&strings, &[BcTag::FunctionBytecode.to_byte()]);
    let (table, mut cursor) = read_table(&bytes);
    let dynamic0 = AtomId::from_zero_based(0);
    let dynamic1 = AtomId::from_zero_based(1);
    let dynamic2 = AtomId::from_zero_based(2);

    assert_eq!(
        table.slot_atoms(),
        [
            ImageAtom::Index(42),
            ImageAtom::Index(42),
            ImageAtom::Predefined(pinned(50)),
            ImageAtom::Predefined(pinned(50)),
            ImageAtom::Dynamic(dynamic0),
            ImageAtom::Dynamic(dynamic0),
            ImageAtom::Dynamic(dynamic1),
            ImageAtom::Dynamic(dynamic2),
            ImageAtom::Dynamic(dynamic2),
        ]
    );
    assert_eq!(
        table.dynamic_atoms(),
        [
            // The first wire width/spelling is retained even though later
            // narrow/wide UTF-16-equivalent spellings share this identity.
            narrow(&[0xe9]),
            // Narrow bytes are Latin-1 code units, not UTF-8. C3 A9 therefore
            // does not alias the single wide code unit 00E9.
            narrow(&[0xc3, 0xa9]),
            narrow(b"2147483648"),
        ]
    );
    assert_eq!(table.raw_space().header_count(), 9);
    assert_eq!(table.slot_atoms().len(), 9);
    assert_eq!(table.dynamic_atoms().len(), 3);

    // Reading the atom table stops before the root value. Tag 12 remains
    // outside this module's admission boundary.
    assert_eq!(cursor.position(), bytes.len() - 1);
    assert_eq!(cursor.read_tag(), Ok(BcTag::FunctionBytecode));
    cursor.finish().unwrap();
}

#[test]
fn decimal_index_boundaries_match_js_new_atom_str() {
    let strings = [
        narrow(b"0"),
        narrow(b"00"),
        narrow(b"2147483647"),
        narrow(b"2147483648"),
        narrow(b"4294967295"),
        narrow(b"4294967296"),
        narrow(b""),
        narrow(b"-0"),
    ];
    let bytes = header_bytes(&strings, &[BcTag::Null.to_byte()]);
    let (table, _) = read_table(&bytes);

    assert_eq!(
        table.slot_atoms(),
        [
            ImageAtom::Index(0),
            ImageAtom::Dynamic(AtomId::from_zero_based(0)),
            ImageAtom::Index(i32::MAX as u32),
            ImageAtom::Dynamic(AtomId::from_zero_based(1)),
            ImageAtom::Dynamic(AtomId::from_zero_based(2)),
            ImageAtom::Dynamic(AtomId::from_zero_based(3)),
            ImageAtom::Predefined(ordinary_pinned("")),
            ImageAtom::Predefined(ordinary_pinned("-0")),
        ]
    );
    assert_eq!(
        table.dynamic_atoms(),
        [
            narrow(b"00"),
            narrow(b"2147483648"),
            narrow(b"4294967295"),
            narrow(b"4294967296"),
        ]
    );
}

#[test]
fn header_descriptions_never_gain_private_or_symbol_identity() {
    let strings = [
        narrow(b"<brand>"),
        narrow(b"Symbol.iterator"),
        ascii_wide(b"Symbol.iterator"),
    ];
    let bytes = header_bytes(&strings, &[BcTag::Null.to_byte()]);
    let (table, _) = read_table(&bytes);

    let ordinary_brand = pinned(124);
    let private_brand = pinned(229);
    let symbol_iterator = pinned(231);
    assert_eq!(ordinary_brand.kind(), PinnedAtomKind::String);
    assert_eq!(private_brand.kind(), PinnedAtomKind::Private);
    assert_eq!(symbol_iterator.kind(), PinnedAtomKind::Symbol);
    assert_eq!(ordinary_brand.spelling(), private_brand.spelling());

    // `<brand>` finds the existing ordinary string identity, never the private
    // identity with the same description. A symbol-only description remains a
    // dynamic string, and narrow/wide spellings alias it.
    assert_eq!(
        table.slot_atoms(),
        [
            ImageAtom::Predefined(ordinary_brand),
            ImageAtom::Dynamic(AtomId::from_zero_based(0)),
            ImageAtom::Dynamic(AtomId::from_zero_based(0)),
        ]
    );
    assert_ne!(table.slot_atoms()[0], ImageAtom::Predefined(private_brand));
    assert!(
        !table
            .slot_atoms()
            .contains(&ImageAtom::Predefined(symbol_iterator))
    );
}

#[test]
fn remapping_checks_namespace_slots_and_null_keys() {
    let bytes = header_bytes(&[narrow(b"dynamic")], &[BcTag::Null.to_byte()]);
    let (table, _) = read_table(&bytes);
    let space = table.raw_space();
    let slot = space.header_slot(0).unwrap();

    assert_eq!(
        table.remap_atom(space, BinaryAtom::Header(slot), 17),
        Ok(ImageAtom::Dynamic(AtomId::from_zero_based(0)))
    );
    assert_eq!(
        table.remap_key(space, BinaryAtom::Header(slot), ReaderMode::Strict, 17),
        Ok(Some(ImageKey::Dynamic(AtomId::from_zero_based(0))))
    );
    assert_eq!(
        table.remap_atom(space, BinaryAtom::Index(7), 18),
        Ok(ImageAtom::Index(7))
    );
    assert_eq!(
        table.remap_key(space, BinaryAtom::Null, ReaderMode::Strict, 19),
        Err(ImageAtomError::NullPropertyKey { offset: 19 })
    );
    assert_eq!(
        table.remap_key(space, BinaryAtom::Null, ReaderMode::QuickJsCompatible, 19,),
        Ok(None)
    );

    let data_space = AtomIndexSpace::new(BinaryObjectMode::Data, 1).unwrap();
    assert_eq!(
        table.remap_atom(
            data_space,
            BinaryAtom::Header(data_space.header_slot(0).unwrap()),
            20
        ),
        Err(ImageAtomError::AtomIndexSpaceMismatch {
            expected: space,
            actual: data_space,
        })
    );

    let larger_space = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 2).unwrap();
    let foreign_slot = larger_space.header_slot(1).unwrap();
    assert_eq!(
        table.remap_atom(space, BinaryAtom::Header(foreign_slot), 21),
        Err(ImageAtomError::ForeignHeaderSlot {
            slot: 1,
            header_count: 1,
        })
    );
}

#[test]
fn direct_predefined_private_and_symbol_atoms_preserve_identity() {
    let bytes = header_bytes(&[], &[BcTag::Null.to_byte()]);
    let (table, _) = read_table(&bytes);
    let space = table.raw_space();

    for atom in [pinned(229), pinned(230), pinned(242)] {
        let raw = space.resolve_opcode_atom(atom.raw(), 7).unwrap();
        assert_eq!(raw, BinaryAtom::Predefined(atom));
        assert_eq!(
            table.remap_atom(space, raw, 7),
            Ok(ImageAtom::Predefined(atom))
        );
        assert_eq!(
            table.remap_key(space, raw, ReaderMode::Strict, 7),
            Ok(Some(ImageKey::Predefined(atom)))
        );
    }
}

#[test]
fn reader_keeps_exact_cursor_position_and_enforces_wire_limits() {
    let strings = [narrow(b"one"), ascii_wide(b"two")];
    let bytes = header_bytes(&strings, &[0xaa, 0xbb]);
    let expected_position = bytes.len() - 2;
    let (table, mut cursor) = read_table(&bytes);
    assert_eq!(table.raw_space().mode(), BinaryObjectMode::Bytecode);
    assert_eq!(table.raw_space().header_count(), 2);
    assert_eq!(cursor.position(), expected_position);
    assert_eq!(cursor.read_bytes(2), Ok(&[0xaa, 0xbb][..]));
    cursor.finish().unwrap();

    let atom_limited = WireLimits::new(4096, 1, 128, 512);
    let mut cursor = WireCursor::new(&bytes, ReaderMode::Strict, atom_limited).unwrap();
    assert_eq!(
        ImageAtomTable::read(&mut cursor),
        Err(ImageAtomError::Wire(WireError::ResourceLimit {
            kind: ResourceKind::AtomCount,
            requested: 2,
            limit: 1,
        }))
    );
    assert_eq!(cursor.position(), 2);

    let string_limited = WireLimits::new(4096, 2, 2, 512);
    let mut cursor = WireCursor::new(&bytes, ReaderMode::Strict, string_limited).unwrap();
    assert_eq!(
        ImageAtomTable::read(&mut cursor),
        Err(ImageAtomError::Wire(WireError::ResourceLimit {
            kind: ResourceKind::StringCodeUnits,
            requested: 3,
            limit: 2,
        }))
    );
    assert_eq!(cursor.position(), 3);

    let total_limited = WireLimits::new(4096, 2, 128, 5);
    let mut cursor = WireCursor::new(&bytes, ReaderMode::Strict, total_limited).unwrap();
    assert_eq!(
        ImageAtomTable::read(&mut cursor),
        Err(ImageAtomError::Wire(WireError::ResourceLimit {
            kind: ResourceKind::TotalStringCodeUnits,
            requested: 6,
            limit: 5,
        }))
    );
    // Header (2), first narrow string (4), then the second length (1).
    assert_eq!(cursor.position(), 7);
}

#[test]
fn decodes_the_exact_quickjs_42_function_as_non_executable_image() {
    let vector = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    let image = decode_image(&vector).unwrap();

    assert!(image.atoms().is_empty());
    assert!(image.nodes().is_empty());
    assert!(image.reference_table().is_empty());
    assert_eq!(image.functions().len(), 1);
    let root = function_id(image.root());
    assert_eq!(root.zero_based(), 0);
    assert_eq!(image.function(root), image.functions().first());
    assert!(image.functions()[0].constants().is_empty());
    assert_eq!(
        image.functions()[0].envelope().code().as_bytes(),
        [0xbb, 0x2a, 0xcb, 0x28]
    );
    assert_eq!(
        image.functions()[0].envelope().code().instructions().len(),
        3
    );
}

#[test]
fn nested_functions_receive_preorder_ids_without_object_reference_ids() {
    let vector = bytes(
        "05020a6f757465720a696e6e65720c000200a80100010002000001090100000000be00bb28edb5edcb280c430200e60301010101010001080200010000000000e05e0000cfc7be00280c430200e803010001020001000e010001000000001000640000cf9b116500000e64000028",
    );
    let image = decode_image(&vector).unwrap();

    assert_eq!(image.functions().len(), 3);
    assert!(image.nodes().is_empty());
    assert!(image.reference_table().is_empty());
    let root = function_id(image.root());
    let outer = function_id(&image.functions()[0].constants()[0]);
    let inner = function_id(&image.functions()[1].constants()[0]);
    assert_eq!(
        [root.zero_based(), outer.zero_based(), inner.zero_based()],
        [0, 1, 2]
    );
    assert!(image.functions()[2].constants().is_empty());
    assert_eq!(image.functions()[2].envelope().closures().len(), 1);
    assert!(image.function(root).is_some());
    assert!(image.function(outer).is_some());
    assert!(image.function(inner).is_some());
}

#[test]
fn one_arena_spans_functions_templates_and_trailing_references() {
    let vector = bytes(
        "05011074656d706c6174650802360c000200a80100010002000002070100000000be00bd01edcb280c0202000001000101000000020100010000cf280b010702780b0107027802e6031301",
    );
    let image = decode_image(&vector).unwrap();

    let root = node_id(image.root());
    assert_eq!(root.zero_based(), 0);
    assert_eq!(image.functions().len(), 2);
    assert_eq!(image.nodes().len(), 3);
    assert_eq!(
        image
            .reference_table()
            .iter()
            .map(|node| node.zero_based())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );

    let WireNodeCarrier::Ordinary { properties } = &image.nodes()[0] else {
        panic!("root must be ordinary");
    };
    assert_eq!(properties.len(), 2);
    assert_eq!(function_id(&properties[0].value).zero_based(), 0);
    assert_eq!(node_id(&properties[1].value).zero_based(), 1);
    assert_eq!(
        function_id(&image.functions()[0].constants()[0]).zero_based(),
        1
    );
    assert_eq!(
        node_id(&image.functions()[0].constants()[1]).zero_based(),
        1
    );
    assert!(image.functions()[1].constants().is_empty());

    let WireNodeCarrier::TemplateObject { elements, raw } = &image.nodes()[1] else {
        panic!("node one must be a template object");
    };
    assert_eq!(elements.len(), 1);
    assert_eq!(node_id(raw).zero_based(), 2);
}

#[test]
fn function_constant_pool_can_reference_its_enclosing_object_ancestor() {
    let vector = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    let image = decode_image(&vector).unwrap();

    let root = node_id(image.root());
    assert_eq!(root.zero_based(), 0);
    assert_eq!(image.reference_table(), [root]);
    assert_eq!(image.functions().len(), 1);
    assert_eq!(node_id(&image.functions()[0].constants()[0]), root);
    let WireNodeCarrier::Ordinary { properties } = &image.nodes()[0] else {
        panic!("root must be ordinary");
    };
    assert_eq!(properties.len(), 1);
    assert_eq!(function_id(&properties[0].value).zero_based(), 0);
}

#[test]
fn functions_are_valid_children_of_each_data_container_shape() {
    let record = quickjs_42_record();
    for (body, expected_kind) in [
        (vec![BcTag::Array.to_byte(), 1], "array"),
        (vec![BcTag::TemplateObject.to_byte(), 0], "template"),
    ] {
        let mut vector = vec![5, 0];
        vector.extend_from_slice(&body);
        vector.extend_from_slice(&record);
        let image = decode_image(&vector).unwrap();
        assert_eq!(image.functions().len(), 1);
        match (&image.nodes()[0], expected_kind) {
            (WireNodeCarrier::Array { elements }, "array") => {
                assert_eq!(function_id(&elements[0]).zero_based(), 0);
            }
            (WireNodeCarrier::TemplateObject { elements, raw }, "template") => {
                assert!(elements.is_empty());
                assert_eq!(function_id(raw).zero_based(), 0);
            }
            (node, kind) => panic!("expected {kind}, got {node:?}"),
        }
    }
}

#[test]
fn data_only_coercion_tags_reject_function_children_with_typed_errors() {
    let record = quickjs_42_record();
    let mut object_value = vec![5, 0, BcTag::ObjectValue.to_byte()];
    object_value.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&object_value),
        Err(BytecodeImageError::Data(DecodeError::OpaqueObjectValue {
            offset: 2,
            value,
        })) if value.zero_based() == 0
    ));

    let mut date = vec![5, 0, BcTag::Date.to_byte()];
    date.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&date),
        Err(BytecodeImageError::Data(DecodeError::OpaqueDateValue {
            offset: 2,
            value,
        })) if value.zero_based() == 0
    ));

    let mut typed_array = vec![5, 0, BcTag::TypedArray.to_byte(), 2, 1, 0];
    typed_array.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&typed_array),
        Err(BytecodeImageError::Data(
            DecodeError::OpaqueTypedArrayBacking {
                offset: 2,
                value,
            }
        )) if value.zero_based() == 0
    ));
}

#[test]
fn opaque_function_source_tokens_reject_cross_image_rebranding() {
    let first = decode_image(&bytes("05000c000200a80100010001000000040100000000bb2acb28")).unwrap();
    let second =
        decode_image(&bytes("05000c000200a80100010001000000040100000000bb2acb28")).unwrap();
    let first_id = function_id(first.root());
    let second_id = function_id(second.root());
    assert_eq!(first_id.zero_based(), second_id.zero_based());
    assert_ne!(first_id, second_id);
    assert!(first.function(first_id).is_some());
    assert!(first.function(second_id).is_none());

    let machine_b = DataMachine::<ImageValue, ImageKey>::new(GRAPH_LIMITS, true).unwrap();
    assert!(matches!(
        machine_b.wrap_opaque_value(first.root().clone()),
        Err(DecodeError::InvalidCompletionTarget)
    ));
}

#[test]
fn whole_image_limits_bound_functions_depth_and_aggregate_payloads() {
    let answer = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    assert_eq!(
        decode_image_with(
            &answer,
            ReaderMode::Strict,
            bounded_image_limits(0, 256, 4096, 16384),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::Functions,
            requested: 1,
            limit: 0,
        })
    );
    assert_eq!(
        decode_image_with(
            &answer,
            ReaderMode::Strict,
            bounded_image_limits(256, 256, 4096, 3),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalCodeBytes,
            requested: 4,
            limit: 3,
        })
    );

    let nested = bytes(
        "05020a6f757465720a696e6e65720c000200a80100010002000001090100000000be00bb28edb5edcb280c430200e60301010101010001080200010000000000e05e0000cfc7be00280c430200e803010001020001000e010001000000001000640000cf9b116500000e64000028",
    );
    assert_eq!(
        decode_image_with(
            &nested,
            ReaderMode::Strict,
            bounded_image_limits(256, 1, 4096, 16384),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::WholeDepth,
            requested: 2,
            limit: 1,
        })
    );

    let ancestor = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    assert_eq!(
        decode_image_with(
            &ancestor,
            ReaderMode::Strict,
            bounded_image_limits(256, 256, 0, 16384),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalConstantPoolEntries,
            requested: 1,
            limit: 0,
        })
    );
}

#[test]
fn aggregate_limits_reject_before_avoidable_prefix_work() {
    let mut invalid_code = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    invalid_code[21] = 0;
    assert_eq!(
        decode_image_with(
            &invalid_code,
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalCodeBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalCodeBytes,
            requested: 4,
            limit: 0,
        })
    );
    assert_eq!(
        decode_image_with(
            &invalid_code[..21],
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalCodeBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalCodeBytes,
            requested: 4,
            limit: 0,
        })
    );

    // The local count is present but its table is deliberately absent. The
    // aggregate count is known from the header and wins before any reserve or
    // child-field read.
    let truncated_local = bytes("05000c000200a801000100010000000401");
    assert_eq!(
        decode_image_with(
            &truncated_local,
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalLocalVariables,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalLocalVariables,
            requested: 1,
            limit: 0,
        })
    );

    // Debug lengths are both known before either slice is copied. Omitting the
    // final source byte therefore cannot mask the whole-image budget failure.
    let mut truncated_debug = vec![5, 0];
    truncated_debug.extend_from_slice(&debug_record());
    assert_eq!(truncated_debug.pop(), Some(0xbb));
    assert_eq!(
        decode_image_with(
            &truncated_debug,
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalDebugBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalDebugBytes,
            requested: 2,
            limit: 0,
        })
    );

    // Equal per-function and whole limits retain the established envelope
    // error instead of relabeling it as an aggregate failure.
    let envelope_zero_code = FunctionEnvelopeLimits::new(
        256,
        256,
        256,
        4096,
        4096,
        8192,
        CodeLimits::new(0, 4096, 4096),
    );
    let answer = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    assert_eq!(
        decode_image_with(
            &answer,
            ReaderMode::Strict,
            one_aggregate_limit(
                envelope_zero_code,
                BytecodeImageResourceKind::TotalCodeBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::Envelope(FunctionEnvelopeError::Code(
            CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested: 4,
                limit: 0,
            }
        )))
    );
}

#[test]
fn aggregate_remaining_budget_bounds_each_later_function_resource() {
    let cases = [
        (
            constant_record(),
            BytecodeImageResourceKind::TotalConstantPoolEntries,
            1,
            2,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalLocalVariables,
            1,
            2,
        ),
        (
            closure_record(),
            BytecodeImageResourceKind::TotalClosureVariables,
            1,
            2,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalCodeBytes,
            4,
            8,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalInstructions,
            3,
            4,
        ),
        (
            atom_relocation_record(),
            BytecodeImageResourceKind::TotalAtomRelocations,
            1,
            2,
        ),
        (
            debug_record(),
            BytecodeImageResourceKind::TotalDebugBytes,
            2,
            4,
        ),
    ];

    for (record, kind, limit, requested) in cases {
        let image = sibling_function_array(&record);
        decode_image(&image).unwrap_or_else(|error| {
            panic!("synthetic {kind:?} boundary vector must be valid: {error}")
        });
        assert_eq!(
            decode_image_with(
                &image,
                ReaderMode::Strict,
                one_aggregate_limit(ENVELOPE_LIMITS, kind, limit),
                true,
            ),
            Err(BytecodeImageError::ResourceLimit {
                kind,
                requested,
                limit,
            }),
            "wrong remaining-budget result for {kind:?}",
        );
    }
}

#[test]
fn parent_property_key_errors_precede_recursive_whole_depth_limits() {
    let shallow = bounded_image_limits(256, 1, 4096, 16384);
    assert_eq!(
        decode_image_with(
            &[5, 0, BcTag::Object.to_byte(), 1],
            ReaderMode::Strict,
            shallow,
            true,
        ),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 4,
            needed: 1,
            remaining: 0,
        }))
    );
}

#[test]
fn finalization_keeps_mode_reference_flags_and_unsupported_tags_observable() {
    let mut trailing = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    trailing.push(0xff);
    assert_eq!(
        decode_image(&trailing),
        Err(BytecodeImageError::Wire(WireError::TrailingBytes {
            offset: 25,
            remaining: 1,
        }))
    );
    assert!(
        decode_image_with(&trailing, ReaderMode::QuickJsCompatible, IMAGE_LIMITS, true,).is_ok()
    );

    let ancestor = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    assert_eq!(
        decode_image_with(&ancestor, ReaderMode::Strict, IMAGE_LIMITS, false),
        Err(BytecodeImageError::Data(
            DecodeError::ObjectReferencesNotAllowed { offset: 31 }
        ))
    );

    for tag in [BcTag::Module, BcTag::SharedArrayBuffer] {
        assert_eq!(
            decode_image(&[5, 0, tag.to_byte()]),
            Err(BytecodeImageError::Data(DecodeError::UnsupportedTag {
                tag,
                offset: 2,
            }))
        );
    }
}

#[test]
fn canonical_writer_round_trips_pinned_quickjs_bytecode_images_byte_exactly() {
    let vectors = [
        bytes("05000c000200a80100010001000000040100000000bb2acb28"),
        bytes(
            "05020a6f757465720a696e6e65720c000200a80100010002000001090100000000be00bb28edb5edcb280c430200e60301010101010001080200010000000000e05e0000cfc7be00280c430200e803010001020001000e010001000000001000640000cf9b116500000e64000028",
        ),
        bytes(
            "05011074656d706c6174650802360c000200a80100010002000002070100000000be00bd01edcb280c0202000001000101000000020100010000cf280b010702780b0107027802e6031301",
        ),
        bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300"),
    ];

    for vector in vectors {
        let image = decode_image(&vector).unwrap();
        assert_eq!(encode_image(&image), Ok(vector));
    }
}

#[test]
fn canonical_writer_preserves_pinned_debug_and_strip_shapes_byte_exactly() {
    let vectors = [
        bytes(
            "05041e7772697465722d666c6167732e6a730a6f7574657208736565640c616e737765720c000600a801000100020000010801aa01000000be00bb28edeccb28e603080000000408040708000c430600e803010001010100010301ea03010040be0028e6030400010d024f66756e6374696f6e206f75746572287365656429207b0a202072657475726e2066756e6374696f6e20616e737765722829207b0a2020202072657475726e2073656564202b20323b0a20207d3b0a7d0c430600ec03000000020001000400ea03000100dbb59b28e60308010903040c0a07172c66756e6374696f6e20616e737765722829207b0a2020202072657475726e2073656564202b20323b0a20207d",
        ),
        bytes(
            "05041e7772697465722d666c6167732e6a730a6f7574657208736565640c616e737765720c000600a801000100020000010801aa01000000be00bb28edeccb28e603080000000408040708000c430600e803010001010100010301ea03010040be0028e6030400010d02000c430600ec03000000020001000400ea03000100dbb59b28e60308010903040c0a071700",
        ),
        bytes(
            "05020a6f757465720c616e737765720c000200a80100010002000001080100000000be00bb28edeccb280c430200e60301000101010001030100010040be00280c430200e80300000002000100040000000100dbb59b28",
        ),
    ];

    for vector in vectors {
        let image = decode_image(&vector).unwrap();
        assert_eq!(encode_image(&image), Ok(vector));
    }
}

#[test]
fn canonical_writer_rebuilds_reference_state_and_rejects_too_small_output() {
    let vector = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    let image = decode_image(&vector).unwrap();
    assert_eq!(encode_image(&image), Ok(vector.clone()));

    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(true, vector.len() - 1, IMAGE_LIMITS),
        ),
        Err(BytecodeImageEncodeError::Wire(WireError::ResourceLimit {
            kind: ResourceKind::OutputBytes,
            requested: vector.len(),
            limit: vector.len() - 1,
        }))
    );
    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, IMAGE_LIMITS),
        ),
        Err(BytecodeImageEncodeError::CircularReference {
            node: NodeId::from_zero_based(0),
        })
    );
}

#[test]
fn canonical_writer_filters_non_string_properties_without_visiting_their_values() {
    // Root ID 0 owns `keep: 42`, a symbol-keyed self-cycling Array (ID 1),
    // and a private-keyed alias to that Array. Pinned JS_WriteObject filters
    // both non-string keys before visiting either value, so refs-off writing
    // succeeds and exactly matches the authenticated public-C-API oracle.
    let input = bytes("0501086b6565700803e6030554ce0309011301ca031301");
    let expected = bytes("0501086b6565700801e6030554");
    let image = decode_image(&input).unwrap();

    for references in [false, true] {
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(expected.clone()),
        );
    }
}

#[test]
fn canonical_writer_prunes_unused_atoms_and_expands_acyclic_aliases_without_references() {
    let unused_atom_input = header_bytes(&[narrow(b"unused")], &quickjs_42_record());
    let answer = decode_image(&unused_atom_input).unwrap();
    assert_eq!(
        encode_image(&answer),
        Ok(bytes("05000c000200a80100010001000000040100000000bb2acb28")),
    );

    // Root ID 0 contains one Array (ID 1) twice. References-on preserves the
    // alias spelling; references-off performs the same acyclic expansion as
    // pinned JS_WriteObject without JS_WRITE_OBJ_REFERENCE.
    let aliased = bytes("050008020109010554031301");
    let image = decode_image(&aliased).unwrap();
    assert_eq!(encode_image(&image), Ok(aliased));
    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, IMAGE_LIMITS),
        ),
        Ok(bytes("0500080201090105540309010554")),
    );
}

#[test]
fn canonical_writer_shares_decoder_aggregate_error_attribution() {
    let cases = [
        (
            constant_record(),
            BytecodeImageResourceKind::TotalConstantPoolEntries,
            1,
            2,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalLocalVariables,
            1,
            2,
        ),
        (
            closure_record(),
            BytecodeImageResourceKind::TotalClosureVariables,
            1,
            2,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalCodeBytes,
            4,
            8,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalInstructions,
            3,
            4,
        ),
        (
            atom_relocation_record(),
            BytecodeImageResourceKind::TotalAtomRelocations,
            1,
            2,
        ),
        (
            debug_record(),
            BytecodeImageResourceKind::TotalDebugBytes,
            2,
            4,
        ),
    ];

    for (record, kind, limit, requested) in cases {
        let image = decode_image(&sibling_function_array(&record)).unwrap();
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(
                    true,
                    65536,
                    one_aggregate_limit(ENVELOPE_LIMITS, kind, limit),
                ),
            ),
            Err(BytecodeImageEncodeError::Budget(
                BytecodeImageBudgetError::ResourceLimit {
                    kind,
                    requested,
                    limit,
                }
            )),
            "wrong writer remaining-budget result for {kind:?}",
        );
    }

    // Equal per-function and aggregate caps remain a per-function error, just
    // like the decoder contract above; only a strictly smaller remaining
    // aggregate budget changes the diagnostic owner.
    let envelope_zero_code = FunctionEnvelopeLimits::new(
        256,
        256,
        256,
        4096,
        4096,
        8192,
        CodeLimits::new(0, 4096, 4096),
    );
    let answer =
        decode_image(&bytes("05000c000200a80100010001000000040100000000bb2acb28")).unwrap();
    assert_eq!(
        encode_bytecode_image(
            &answer,
            BytecodeImageEncodeOptions::new(
                true,
                65536,
                one_aggregate_limit(
                    envelope_zero_code,
                    BytecodeImageResourceKind::TotalCodeBytes,
                    0,
                ),
            ),
        ),
        Err(BytecodeImageEncodeError::Envelope(
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested: 4,
                limit: 0,
            })
        )),
    );
}
