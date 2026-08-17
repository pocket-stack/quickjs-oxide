use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::code::{CodeError, CodeLimits, CodeResourceKind};
use super::super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits};
use super::super::graph::decode::{DataMachine, DecodeError, decode_graph_with_sab_transport};
use super::super::graph::model::{
    AtomId, GraphError, GraphLimits, GraphResourceKind, NodeId, TypedArrayKind, WireNodeCarrier,
    WireValue,
};
use super::super::graph::sab_transport::{NativeSabToken, SabArchiveError, SabTransportInput};
use super::super::pinned_atoms::{PinnedAtomId, PinnedAtomKind};
use super::super::wire::{
    BcTag, ReaderMode, ResourceKind, WireCursor, WireError, WireLimits, WireString, WireWriter,
};
use super::{
    BytecodeImageBudgetError, BytecodeImageEncodeError, BytecodeImageEncodeOptions,
    BytecodeImageError, BytecodeImageLimits, BytecodeImageResourceKind, FunctionId, ImageAtom,
    ImageAtomError, ImageAtomTable, ImageFunctionEnvelope, ImageKey, ImageOpaque, ImageValue,
    ModuleBudgetError, ModuleField, ModuleLimits, ModuleResourceKind, decode_bytecode_image,
    decode_bytecode_image_with_sab_transport, encode_bytecode_image,
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
const MODULE_LIMITS: ModuleLimits = ModuleLimits::new(256, 256, 256, 256);
const IMAGE_LIMITS: BytecodeImageLimits = BytecodeImageLimits::new(
    GRAPH_LIMITS,
    ENVELOPE_LIMITS,
    MODULE_LIMITS,
    256,
    256,
    256,
    4096,
    4096,
    4096,
    16384,
    16384,
    16384,
    16384,
    4096,
    4096,
    4096,
    4096,
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

fn sab_image_limits() -> BytecodeImageLimits {
    sab_image_limits_for(1, 1, 4)
}

fn sab_image_limits_for(
    occurrences: usize,
    backings: usize,
    total_capacity: usize,
) -> BytecodeImageLimits {
    BytecodeImageLimits::new(
        GRAPH_LIMITS.with_shared_array_buffers(occurrences, backings, 4, total_capacity),
        ENVELOPE_LIMITS,
        MODULE_LIMITS,
        256,
        256,
        256,
        4096,
        4096,
        4096,
        16384,
        16384,
        16384,
        16384,
        4096,
        4096,
        4096,
        4096,
    )
}

fn decode_sab_image(
    input: &[u8],
    writer_occurrences: &[u64],
    mode: ReaderMode,
    references: bool,
) -> Result<super::ArchivedBytecodeImage, BytecodeImageError> {
    decode_sab_image_with_limits(
        input,
        writer_occurrences,
        mode,
        references,
        sab_image_limits(),
    )
}

fn decode_sab_image_with_limits(
    input: &[u8],
    writer_occurrences: &[u64],
    mode: ReaderMode,
    references: bool,
    limits: BytecodeImageLimits,
) -> Result<super::ArchivedBytecodeImage, BytecodeImageError> {
    let writer_occurrences = writer_occurrences
        .iter()
        .copied()
        .map(NativeSabToken::from_test_bits)
        .collect::<Vec<_>>();
    decode_bytecode_image_with_sab_transport(
        SabTransportInput::new(input, &writer_occurrences),
        mode,
        TEST_LIMITS,
        limits,
        references,
    )
}

fn function_bytecode_sab_reference_wire(token: u64) -> Vec<u8> {
    // Pinned QuickJS 2026-06-04 whole-image oracle. The checked-in transcript
    // zeroes the sole native token at byte 38 before it is printed.
    let mut wire = bytes(
        "050009040c000200a80100010001000000040100000000bb2acb280e0204001004ffffffff0f000000000000000013021302",
    );
    wire[38..46].copy_from_slice(&token.to_le_bytes());
    wire
}

fn two_sab_records_wire(first: u64, second: u64) -> Vec<u8> {
    let mut wire = vec![5, 0, 9, 2];
    for token in [first, second] {
        wire.extend_from_slice(&[16, 4, 0xff, 0xff, 0xff, 0xff, 0x0f]);
        wire.extend_from_slice(&token.to_le_bytes());
    }
    wire
}

#[derive(Debug, Eq, PartialEq)]
enum SabImageValueSnapshot {
    Data(WireValue),
    Function(u32),
    Module(u32),
}

fn snapshot_sab_image_value(value: &ImageValue) -> SabImageValueSnapshot {
    match value.as_wire() {
        Ok(value) => SabImageValueSnapshot::Data(value.clone()),
        Err(ImageOpaque::Function(function)) => {
            SabImageValueSnapshot::Function(function.zero_based())
        }
        Err(ImageOpaque::Module(module)) => SabImageValueSnapshot::Module(module.zero_based()),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum SabImageNodeSnapshot {
    Array(Vec<SabImageValueSnapshot>),
    TypedArray {
        kind: TypedArrayKind,
        length: u32,
        byte_offset: u32,
        buffer: u32,
    },
    SharedArrayBuffer {
        byte_length: u32,
        max_byte_length: Option<u32>,
        backing: u32,
        capacity: u32,
        growable: bool,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct SabImageFunctionSnapshot {
    envelope: ImageFunctionEnvelope,
    constants: Vec<SabImageValueSnapshot>,
}

#[derive(Debug, Eq, PartialEq)]
struct SabImageSemanticSnapshot {
    atoms: Vec<WireString>,
    nodes: Vec<SabImageNodeSnapshot>,
    references: Vec<u32>,
    functions: Vec<SabImageFunctionSnapshot>,
    module_count: usize,
    root: SabImageValueSnapshot,
    shared_backing_count: usize,
}

fn snapshot_sab_image(archive: &super::ArchivedBytecodeImage) -> SabImageSemanticSnapshot {
    let image = archive.test_image();
    let nodes = image
        .nodes()
        .iter()
        .map(|node| match node {
            WireNodeCarrier::Array { elements } => {
                SabImageNodeSnapshot::Array(elements.iter().map(snapshot_sab_image_value).collect())
            }
            WireNodeCarrier::TypedArray {
                kind,
                length,
                byte_offset,
                buffer,
            } => SabImageNodeSnapshot::TypedArray {
                kind: *kind,
                length: *length,
                byte_offset: *byte_offset,
                buffer: buffer.zero_based(),
            },
            WireNodeCarrier::SharedArrayBuffer {
                byte_length,
                max_byte_length,
                backing,
            } => {
                let descriptor = archive
                    .test_shared_backing_descriptor(*backing)
                    .expect("every archived SAB node must retain its backing descriptor");
                SabImageNodeSnapshot::SharedArrayBuffer {
                    byte_length: *byte_length,
                    max_byte_length: *max_byte_length,
                    backing: backing.zero_based(),
                    capacity: descriptor.capacity(),
                    growable: descriptor.is_growable(),
                }
            }
            _ => panic!("whole-image SAB oracle contains an unexpected node: {node:?}"),
        })
        .collect();
    let functions = image
        .functions()
        .iter()
        .map(|function| SabImageFunctionSnapshot {
            envelope: function.envelope().clone(),
            constants: function
                .constants()
                .iter()
                .map(snapshot_sab_image_value)
                .collect(),
        })
        .collect();

    SabImageSemanticSnapshot {
        atoms: image.atoms().to_vec(),
        nodes,
        references: image
            .reference_table()
            .iter()
            .map(|node| node.zero_based())
            .collect(),
        functions,
        module_count: image.modules().len(),
        root: snapshot_sab_image_value(image.root()),
        shared_backing_count: archive.shared_backing_count(),
    }
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
        MODULE_LIMITS,
        functions,
        256,
        whole_depth,
        constant_pool_entries,
        4096,
        4096,
        code_bytes,
        16384,
        16384,
        16384,
        4096,
        4096,
        4096,
        4096,
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
        MODULE_LIMITS,
        256,
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
        4096,
        4096,
        4096,
        4096,
    )
}

fn module_image_limits(
    module: ModuleLimits,
    modules: usize,
    requests: usize,
    exports: usize,
    star_exports: usize,
    imports: usize,
) -> BytecodeImageLimits {
    BytecodeImageLimits::new(
        GRAPH_LIMITS,
        ENVELOPE_LIMITS,
        module,
        256,
        modules,
        256,
        4096,
        4096,
        4096,
        16384,
        16384,
        16384,
        16384,
        requests,
        exports,
        star_exports,
        imports,
    )
}

fn one_module_aggregate_limit(
    kind: BytecodeImageResourceKind,
    limit: usize,
) -> BytecodeImageLimits {
    assert!(matches!(
        kind,
        BytecodeImageResourceKind::TotalModuleRequests
            | BytecodeImageResourceKind::TotalModuleExports
            | BytecodeImageResourceKind::TotalModuleStarExports
            | BytecodeImageResourceKind::TotalModuleImports
    ));
    module_image_limits(
        MODULE_LIMITS,
        256,
        if kind == BytecodeImageResourceKind::TotalModuleRequests {
            limit
        } else {
            4096
        },
        if kind == BytecodeImageResourceKind::TotalModuleExports {
            limit
        } else {
            4096
        },
        if kind == BytecodeImageResourceKind::TotalModuleStarExports {
            limit
        } else {
            4096
        },
        if kind == BytecodeImageResourceKind::TotalModuleImports {
            limit
        } else {
            4096
        },
    )
}

fn one_per_module_limit(kind: ModuleResourceKind, limit: usize) -> ModuleLimits {
    ModuleLimits::new(
        if kind == ModuleResourceKind::Requests {
            limit
        } else {
            256
        },
        if kind == ModuleResourceKind::Exports {
            limit
        } else {
            256
        },
        if kind == ModuleResourceKind::StarExports {
            limit
        } else {
            256
        },
        if kind == ModuleResourceKind::Imports {
            limit
        } else {
            256
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

fn self_contained_module_vector() -> Vec<u8> {
    bytes(
        "05032473656c662d636f6e7461696e65642e6d6a730c616e737765722e5f5f6d6f64756c6542797465636f6465526563656970740de60300010000e8030000000c200201a801000000020002001400e803001e00a00200050008e80229bb2adf3801006400003ff5000000062f",
    )
}

fn metadata_rich_module_vector() -> Vec<u8> {
    bytes(
        "050d226d657461646174612d726963682e6d6a73102e2f6465702e6a730874797065086d6f64651c2e2f6e616d6573706163652e6a73122e2f737461722e6a73146c6f63616c56616c75650a6e616d65641a696e64697265637456616c75651e6e616d6573706163654578706f72741864656661756c7456616c756518696d706f727465644e616d651c6e616d65737061636556616c75650de60305e8030802ea03070c6f7261636c65ec03070872696368ee0302e80302f00302ee0302030003f2030102f403f60301048402f80301030300002c000100f403000201840201010c200201a801000000010004001700fa03001f00fc03011f00fe03021e00f203001e0008e80229b4e26400000e6401000e6402000eb3890e062f",
    )
}

fn counted_module_record(
    request_count: u8,
    export_count: u8,
    star_export_count: u8,
    import_count: u8,
) -> Vec<u8> {
    let mut record = vec![BcTag::Module.to_byte(), 0, request_count];
    for _ in 0..request_count {
        // Null request name and an arbitrary null attributes value.
        record.extend_from_slice(&[0, BcTag::Null.to_byte()]);
    }
    record.push(export_count);
    for _ in 0..export_count {
        // Local export: type zero, variable index zero, null export name.
        record.extend_from_slice(&[0, 0, 0]);
    }
    record.push(star_export_count);
    record.extend(std::iter::repeat_n(0, usize::from(star_export_count)));
    record.push(import_count);
    for _ in 0..import_count {
        // Variable index, normalized is_star, null name, request index.
        record.extend_from_slice(&[0, 0, 0, 0]);
    }
    record.extend_from_slice(&[0, BcTag::Null.to_byte()]);
    record
}

fn sibling_counted_modules() -> Vec<u8> {
    let record = counted_module_record(1, 1, 1, 1);
    let mut image = vec![5, 0, BcTag::Array.to_byte(), 2];
    image.extend_from_slice(&record);
    image.extend_from_slice(&record);
    image
}

fn counted_module_image() -> Vec<u8> {
    let mut image = vec![5, 0];
    image.extend_from_slice(&counted_module_record(1, 1, 1, 1));
    image
}

fn mixed_module_function_vector(references: bool) -> Vec<u8> {
    let shared_object = if references {
        vec![BcTag::ObjectReference.to_byte(), 1]
    } else {
        vec![BcTag::Object.to_byte(), 0]
    };

    let mut nested_module = vec![
        BcTag::Module.to_byte(),
        0, // name
        0, // requests
        0, // exports
        0, // star exports
        0, // imports
        0, // has_tla
    ];
    nested_module.extend_from_slice(&shared_object);

    let mut function = quickjs_42_record();
    function[12] = 1;
    function.extend_from_slice(&nested_module);

    let mut outer_module = vec![
        BcTag::Module.to_byte(),
        0, // name
        1, // requests
        0, // request name
        BcTag::Object.to_byte(),
        0, // arbitrary attributes: empty ordinary object
        0, // exports
        0, // star exports
        0, // imports
        0, // has_tla
    ];
    outer_module.extend_from_slice(&function);

    let mut image = vec![5, 0, BcTag::Array.to_byte(), 2];
    image.extend_from_slice(&outer_module);
    image.extend_from_slice(&shared_object);
    image
}

fn function_id(value: &ImageValue) -> FunctionId {
    value
        .function_id()
        .expect("test value must be a function identity")
}

fn module_id(value: &ImageValue) -> super::ModuleId {
    value
        .module_id()
        .expect("test value must be a module identity")
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

fn image_key(atom: ImageAtom) -> ImageKey {
    match atom {
        ImageAtom::Null => panic!("null cannot be an image property key"),
        ImageAtom::Index(index) => ImageKey::Index(index),
        ImageAtom::Predefined(atom) => ImageKey::Predefined(atom),
        ImageAtom::Dynamic(atom) => ImageKey::Dynamic(atom),
    }
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
fn self_contained_quickjs_module_decodes_and_encodes_byte_exactly() {
    let vector = self_contained_module_vector();
    assert_eq!(vector.len(), 109);
    let (atoms, _) = read_table(&vector);

    for references in [false, true] {
        let image =
            decode_image_with(&vector, ReaderMode::Strict, IMAGE_LIMITS, references).unwrap();
        assert!(image.nodes().is_empty());
        assert!(image.reference_table().is_empty());
        assert_eq!(image.modules().len(), 1);
        assert_eq!(image.functions().len(), 1);

        let root = module_id(image.root());
        let module = image.module(root).unwrap();
        assert_eq!(module, &image.modules()[0]);
        assert_eq!(module.name(), atoms.slot_atoms()[0]);
        assert!(module.requests().is_empty());
        assert_eq!(module.exports().len(), 1);
        assert_eq!(module.exports()[0].export_type(), 0);
        assert_eq!(module.exports()[0].local_variable_index(), Some(0));
        assert_eq!(module.exports()[0].request_index(), None);
        assert_eq!(module.exports()[0].local_name(), None);
        assert_eq!(module.exports()[0].export_name(), atoms.slot_atoms()[1]);
        assert!(module.star_export_request_indices().is_empty());
        assert!(module.imports().is_empty());
        assert!(!module.has_tla());

        let function = function_id(module.func_obj());
        assert_eq!(function.zero_based(), 0);
        assert_eq!(root.source(), function.source());
        assert_eq!(image.function(function), image.functions().first());
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(vector.clone()),
        );
    }
}

#[test]
fn metadata_rich_quickjs_module_preserves_the_complete_topology() {
    let vector = metadata_rich_module_vector();
    assert_eq!(vector.len(), 283);
    let (atoms, _) = read_table(&vector);
    let slots = atoms.slot_atoms();

    for references in [false, true] {
        let image =
            decode_image_with(&vector, ReaderMode::Strict, IMAGE_LIMITS, references).unwrap();
        assert_eq!(image.modules().len(), 1);
        assert_eq!(image.functions().len(), 1);
        assert_eq!(image.nodes().len(), 1);
        if references {
            assert_eq!(image.reference_table(), [NodeId::from_zero_based(0)]);
        } else {
            assert!(image.reference_table().is_empty());
        }

        let module = image.module(module_id(image.root())).unwrap();
        assert_eq!(module.name(), slots[0]);
        assert_eq!(module.requests().len(), 5);
        assert_eq!(
            module
                .requests()
                .iter()
                .map(|request| request.name())
                .collect::<Vec<_>>(),
            [slots[1], slots[4], slots[1], slots[5], slots[4]],
        );
        assert_eq!(node_id(module.requests()[0].attributes()).zero_based(), 0);
        for request in &module.requests()[1..] {
            assert!(matches!(
                request.attributes().as_wire(),
                Ok(WireValue::Undefined)
            ));
        }

        let WireNodeCarrier::Ordinary { properties } = &image.nodes()[0] else {
            panic!("module attributes must retain their ordinary object");
        };
        assert_eq!(properties.len(), 2);
        assert_eq!(properties[0].key, image_key(slots[2]));
        assert_eq!(
            properties[0].value.as_wire(),
            Ok(&WireValue::String(narrow(b"oracle")))
        );
        assert_eq!(properties[1].key, image_key(slots[3]));
        assert_eq!(
            properties[1].value.as_wire(),
            Ok(&WireValue::String(narrow(b"rich")))
        );

        let exports = module.exports();
        assert_eq!(exports.len(), 3);
        assert_eq!(exports[0].export_type(), 0);
        assert_eq!(exports[0].local_variable_index(), Some(3));
        assert_eq!(exports[0].request_index(), None);
        assert_eq!(exports[0].local_name(), None);
        assert_eq!(exports[0].export_name(), slots[6]);
        assert_eq!(exports[1].export_type(), 1);
        assert_eq!(exports[1].local_variable_index(), None);
        assert_eq!(exports[1].request_index(), Some(2));
        assert_eq!(exports[1].local_name(), Some(slots[7]));
        assert_eq!(exports[1].export_name(), slots[8]);
        assert_eq!(exports[2].export_type(), 1);
        assert_eq!(exports[2].request_index(), Some(4));
        assert_eq!(
            exports[2].local_name(),
            Some(ImageAtom::Predefined(pinned(130)))
        );
        assert_eq!(exports[2].export_name(), slots[9]);

        assert_eq!(module.star_export_request_indices(), [3]);
        let imports = module.imports();
        assert_eq!(imports.len(), 3);
        assert_eq!(
            (
                imports[0].variable_index(),
                imports[0].is_star(),
                imports[0].import_name(),
                imports[0].request_index(),
            ),
            (0, false, ImageAtom::Predefined(pinned(22)), 0),
        );
        assert_eq!(
            (
                imports[1].variable_index(),
                imports[1].is_star(),
                imports[1].import_name(),
                imports[1].request_index(),
            ),
            (1, false, slots[7], 0),
        );
        assert_eq!(
            (
                imports[2].variable_index(),
                imports[2].is_star(),
                imports[2].import_name(),
                imports[2].request_index(),
            ),
            (2, true, ImageAtom::Predefined(pinned(130)), 1),
        );
        assert!(module.has_tla());
        assert_eq!(function_id(module.func_obj()).zero_based(), 0);

        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(vector.clone()),
        );
    }
}

#[test]
fn module_raw_export_type_is_preserved_but_boolean_bytes_are_canonicalized() {
    const EXPORT_TYPE_OFFSET: usize = 195;
    const IS_STAR_OFFSET: usize = 220;
    const HAS_TLA_OFFSET: usize = 224;

    let mut mutated = metadata_rich_module_vector();
    mutated[EXPORT_TYPE_OFFSET] = 0x7f;
    mutated[IS_STAR_OFFSET] = 0x7f;
    mutated[HAS_TLA_OFFSET] = 0x7f;

    let image = decode_image(&mutated).unwrap();
    let module = image.module(module_id(image.root())).unwrap();
    assert_eq!(module.exports()[1].export_type(), 0x7f);
    assert!(module.imports()[2].is_star());
    assert!(module.has_tla());

    let mut canonical = mutated;
    canonical[IS_STAR_OFFSET] = 1;
    canonical[HAS_TLA_OFFSET] = 1;
    for references in [false, true] {
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(canonical.clone()),
        );
    }
}

#[test]
fn compatible_module_ulebs_accept_non_minimal_counts_and_fields_then_write_canonically() {
    let mut canonical = vec![5, 0];
    canonical.extend_from_slice(&counted_module_record(0, 1, 0, 0));

    // Request count and the local export variable index are both zero in the
    // canonical vector. QuickJS accepts their two-byte spellings; Strict does
    // not, and the canonical writer must not retain the redundant byte.
    for (description, offset) in [("request count", 4), ("export variable index", 7)] {
        let mut non_minimal = canonical.clone();
        non_minimal.splice(offset..offset + 1, [0x80, 0x00]);

        assert_eq!(
            decode_image_with(&non_minimal, ReaderMode::Strict, IMAGE_LIMITS, true,),
            Err(BytecodeImageError::Wire(WireError::NonCanonicalUleb128 {
                offset
            })),
            "Strict accepted a non-minimal Module {description}",
        );

        let image = decode_image_with(
            &non_minimal,
            ReaderMode::QuickJsCompatible,
            IMAGE_LIMITS,
            true,
        )
        .unwrap_or_else(|error| panic!("compatible Module {description} must decode: {error}"));
        assert_eq!(
            encode_image(&image),
            Ok(canonical.clone()),
            "writer retained a non-minimal Module {description}",
        );
    }
}

#[test]
fn module_positive_int_boundary_has_typed_count_and_field_errors() {
    const FIRST_NON_POSITIVE_INT: u32 = 0x8000_0000;
    const ENCODED: [u8; 5] = [0x80, 0x80, 0x80, 0x80, 0x08];

    let mut request_count = vec![5, 0, BcTag::Module.to_byte(), 0];
    request_count.extend_from_slice(&ENCODED);

    let mut export_variable = vec![
        5,
        0,
        BcTag::Module.to_byte(),
        0, // name
        0, // requests
        1, // exports
        0, // local export type
    ];
    export_variable.extend_from_slice(&ENCODED);

    let cases = [
        (
            "request count",
            request_count,
            BytecodeImageError::ModuleCountOutOfRange {
                kind: ModuleResourceKind::Requests,
                offset: 4,
                count: FIRST_NON_POSITIVE_INT,
                maximum: i32::MAX as u32,
            },
        ),
        (
            "local export variable index",
            export_variable,
            BytecodeImageError::ModuleFieldOutOfRange {
                field: ModuleField::LocalExportVariable,
                offset: 7,
                value: FIRST_NON_POSITIVE_INT,
                maximum: i32::MAX as u32,
            },
        ),
    ];

    for (description, vector, expected) in cases {
        assert_eq!(
            decode_image(&vector),
            Err(expected),
            "wrong positive-int error for Module {description}",
        );
    }
}

#[test]
fn module_codec_preserves_relationally_invalid_metadata_without_linking_it() {
    let vector = vec![
        5,
        0,
        BcTag::Module.to_byte(),
        0,    // module name
        0,    // requests
        2,    // exports
        0,    // local export
        123,  // variable index, despite no function locals
        0,    // export name
        0x7f, // unknown non-local export type
        125,  // request index, despite no requests
        0,    // local name
        0,    // export name
        1,    // star exports
        126,  // request index, despite no requests
        1,    // imports
        127,  // variable index, despite no function locals
        1,    // is_star
        0,    // import name
        124,  // request index, despite no requests
        1,    // has_tla
        BcTag::Null.to_byte(),
    ];
    let image = decode_image(&vector).unwrap();
    let module = image.module(module_id(image.root())).unwrap();

    assert!(module.requests().is_empty());
    assert_eq!(module.exports()[0].local_variable_index(), Some(123));
    assert_eq!(module.exports()[1].export_type(), 0x7f);
    assert_eq!(module.exports()[1].request_index(), Some(125));
    assert_eq!(module.star_export_request_indices(), [126]);
    assert_eq!(module.imports()[0].variable_index(), 127);
    assert_eq!(module.imports()[0].request_index(), 124);
    assert_eq!(module.func_obj().as_wire(), Ok(&WireValue::Null));

    for references in [false, true] {
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(vector.clone()),
        );
    }
}

#[test]
fn module_writer_visits_request_children_before_tail_budget_checks() {
    let vector = vec![
        5,
        0,
        BcTag::Module.to_byte(),
        0, // module name
        1, // request count
        0, // request name
        BcTag::Array.to_byte(),
        1,
        BcTag::ObjectReference.to_byte(),
        0, // attributes Array refers to itself
        1, // export count
        0, // local export
        0, // variable index
        0, // export name
        0, // star exports
        0, // imports
        0, // has_tla
        BcTag::Null.to_byte(),
    ];
    let image = decode_image(&vector).unwrap();
    let limits = module_image_limits(
        ModuleLimits::new(256, 0, 256, 256),
        256,
        4096,
        4096,
        4096,
        4096,
    );

    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, limits),
        ),
        Err(BytecodeImageEncodeError::CircularReference {
            node: NodeId::from_zero_based(0),
        }),
    );
    assert_eq!(
        encode_bytecode_image(&image, BytecodeImageEncodeOptions::new(true, 65536, limits),),
        Err(BytecodeImageEncodeError::Module(
            ModuleBudgetError::ResourceLimit {
                kind: ModuleResourceKind::Exports,
                requested: 1,
                limit: 0,
            },
        )),
    );
}

#[test]
fn modules_functions_and_objects_share_one_authenticated_preorder_traversal() {
    let with_references = mixed_module_function_vector(true);
    let without_references = mixed_module_function_vector(false);
    assert!(matches!(
        decode_image_with(&with_references, ReaderMode::Strict, IMAGE_LIMITS, false,),
        Err(BytecodeImageError::Data(
            DecodeError::ObjectReferencesNotAllowed { .. }
        ))
    ));
    let image = decode_image(&with_references).unwrap();

    assert_eq!(image.modules().len(), 2);
    assert_eq!(image.functions().len(), 1);
    assert_eq!(image.nodes().len(), 2);
    assert_eq!(
        image.reference_table(),
        [NodeId::from_zero_based(0), NodeId::from_zero_based(1)]
    );

    let WireNodeCarrier::Array { elements } = &image.nodes()[0] else {
        panic!("mixed root must be an array");
    };
    let outer_id = module_id(&elements[0]);
    assert_eq!(outer_id.zero_based(), 0);
    assert_eq!(node_id(&elements[1]).zero_based(), 1);
    let outer = image.module(outer_id).unwrap();
    assert_eq!(outer.requests().len(), 1);
    assert_eq!(node_id(outer.requests()[0].attributes()).zero_based(), 1);

    let function = function_id(outer.func_obj());
    assert_eq!(function.zero_based(), 0);
    assert_eq!(function.source(), outer_id.source());
    let nested_id = module_id(&image.function(function).unwrap().constants()[0]);
    assert_eq!(nested_id.zero_based(), 1);
    assert_eq!(nested_id.source(), outer_id.source());
    assert_eq!(
        node_id(image.module(nested_id).unwrap().func_obj()).zero_based(),
        1
    );

    assert_eq!(encode_image(&image), Ok(with_references));
    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, IMAGE_LIMITS),
        ),
        Ok(without_references.clone()),
    );
    decode_image_with(&without_references, ReaderMode::Strict, IMAGE_LIMITS, false).unwrap();
}

#[test]
fn module_count_budget_is_exact_for_reader_writer_and_later_modules() {
    let single = counted_module_image();
    let single_image = decode_image(&single).unwrap();
    let exact = module_image_limits(MODULE_LIMITS, 1, 4096, 4096, 4096, 4096);
    assert!(decode_image_with(&single, ReaderMode::Strict, exact, true).is_ok());
    assert_eq!(
        encode_bytecode_image(
            &single_image,
            BytecodeImageEncodeOptions::new(true, 65536, exact),
        ),
        Ok(single),
    );

    let zero = module_image_limits(MODULE_LIMITS, 0, 4096, 4096, 4096, 4096);
    let expected_first = BytecodeImageBudgetError::ResourceLimit {
        kind: BytecodeImageResourceKind::Modules,
        requested: 1,
        limit: 0,
    };
    assert_eq!(
        decode_image_with(&counted_module_image(), ReaderMode::Strict, zero, true,),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::Modules,
            requested: 1,
            limit: 0,
        }),
    );
    assert_eq!(
        encode_bytecode_image(
            &single_image,
            BytecodeImageEncodeOptions::new(true, 65536, zero),
        ),
        Err(BytecodeImageEncodeError::Budget(expected_first)),
    );

    let siblings = sibling_counted_modules();
    let siblings_image = decode_image(&siblings).unwrap();
    let expected_later = BytecodeImageBudgetError::ResourceLimit {
        kind: BytecodeImageResourceKind::Modules,
        requested: 2,
        limit: 1,
    };
    assert_eq!(
        decode_image_with(&siblings, ReaderMode::Strict, exact, true),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::Modules,
            requested: 2,
            limit: 1,
        }),
    );
    assert_eq!(
        encode_bytecode_image(
            &siblings_image,
            BytecodeImageEncodeOptions::new(true, 65536, exact),
        ),
        Err(BytecodeImageEncodeError::Budget(expected_later)),
    );
}

#[test]
fn every_module_table_budget_has_exact_off_by_one_and_remaining_attribution() {
    let single = counted_module_image();
    let single_image = decode_image(&single).unwrap();
    let siblings = sibling_counted_modules();
    let siblings_image = decode_image(&siblings).unwrap();
    let cases = [
        (
            ModuleResourceKind::Requests,
            BytecodeImageResourceKind::TotalModuleRequests,
        ),
        (
            ModuleResourceKind::Exports,
            BytecodeImageResourceKind::TotalModuleExports,
        ),
        (
            ModuleResourceKind::StarExports,
            BytecodeImageResourceKind::TotalModuleStarExports,
        ),
        (
            ModuleResourceKind::Imports,
            BytecodeImageResourceKind::TotalModuleImports,
        ),
    ];

    let exact = module_image_limits(ModuleLimits::new(1, 1, 1, 1), 1, 1, 1, 1, 1);
    assert!(decode_image_with(&single, ReaderMode::Strict, exact, true).is_ok());
    assert_eq!(
        encode_bytecode_image(
            &single_image,
            BytecodeImageEncodeOptions::new(true, 65536, exact),
        ),
        Ok(single.clone()),
    );

    for (module_kind, aggregate_kind) in cases {
        let per_record = module_image_limits(
            one_per_module_limit(module_kind, 0),
            256,
            4096,
            4096,
            4096,
            4096,
        );
        let per_record_error = ModuleBudgetError::ResourceLimit {
            kind: module_kind,
            requested: 1,
            limit: 0,
        };
        assert_eq!(
            decode_image_with(&single, ReaderMode::Strict, per_record, true),
            Err(BytecodeImageError::Module(per_record_error.clone())),
            "wrong reader per-record result for {module_kind:?}",
        );
        assert_eq!(
            encode_bytecode_image(
                &single_image,
                BytecodeImageEncodeOptions::new(true, 65536, per_record),
            ),
            Err(BytecodeImageEncodeError::Module(per_record_error)),
            "wrong writer per-record result for {module_kind:?}",
        );

        let aggregate_zero = one_module_aggregate_limit(aggregate_kind, 0);
        assert_eq!(
            decode_image_with(&single, ReaderMode::Strict, aggregate_zero, true),
            Err(BytecodeImageError::ResourceLimit {
                kind: aggregate_kind,
                requested: 1,
                limit: 0,
            }),
            "wrong reader aggregate result for {aggregate_kind:?}",
        );
        assert_eq!(
            encode_bytecode_image(
                &single_image,
                BytecodeImageEncodeOptions::new(true, 65536, aggregate_zero),
            ),
            Err(BytecodeImageEncodeError::Budget(
                BytecodeImageBudgetError::ResourceLimit {
                    kind: aggregate_kind,
                    requested: 1,
                    limit: 0,
                }
            )),
            "wrong writer aggregate result for {aggregate_kind:?}",
        );

        let aggregate_one = one_module_aggregate_limit(aggregate_kind, 1);
        assert_eq!(
            decode_image_with(&siblings, ReaderMode::Strict, aggregate_one, true),
            Err(BytecodeImageError::ResourceLimit {
                kind: aggregate_kind,
                requested: 2,
                limit: 1,
            }),
            "wrong later-module reader result for {aggregate_kind:?}",
        );
        assert_eq!(
            encode_bytecode_image(
                &siblings_image,
                BytecodeImageEncodeOptions::new(true, 65536, aggregate_one),
            ),
            Err(BytecodeImageEncodeError::Budget(
                BytecodeImageBudgetError::ResourceLimit {
                    kind: aggregate_kind,
                    requested: 2,
                    limit: 1,
                }
            )),
            "wrong later-module writer result for {aggregate_kind:?}",
        );
    }
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
        })) if matches!(value, ImageOpaque::Function(function) if function.zero_based() == 0)
    ));

    let mut date = vec![5, 0, BcTag::Date.to_byte()];
    date.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&date),
        Err(BytecodeImageError::Data(DecodeError::OpaqueDateValue {
            offset: 2,
            value,
        })) if matches!(value, ImageOpaque::Function(function) if function.zero_based() == 0)
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
        )) if matches!(value, ImageOpaque::Function(function) if function.zero_based() == 0)
    ));
}

#[test]
fn data_only_coercion_tags_reject_module_children_with_typed_errors() {
    let record = counted_module_record(0, 0, 0, 0);
    for (tag, mut vector) in [
        (BcTag::ObjectValue, vec![5, 0, BcTag::ObjectValue.to_byte()]),
        (BcTag::Date, vec![5, 0, BcTag::Date.to_byte()]),
        (
            BcTag::TypedArray,
            vec![5, 0, BcTag::TypedArray.to_byte(), 2, 1, 0],
        ),
    ] {
        vector.extend_from_slice(&record);
        let (offset, value) = match (tag, decode_image(&vector)) {
            (
                BcTag::ObjectValue,
                Err(BytecodeImageError::Data(DecodeError::OpaqueObjectValue { offset, value })),
            )
            | (
                BcTag::Date,
                Err(BytecodeImageError::Data(DecodeError::OpaqueDateValue { offset, value })),
            )
            | (
                BcTag::TypedArray,
                Err(BytecodeImageError::Data(DecodeError::OpaqueTypedArrayBacking {
                    offset,
                    value,
                })),
            ) => (offset, value),
            (_, result) => panic!("{tag:?} returned the wrong Module coercion result: {result:?}"),
        };
        assert_eq!(offset, 2);
        assert!(
            matches!(value, ImageOpaque::Module(module) if module.zero_based() == 0),
            "{tag:?} did not retain the typed Module identity",
        );
    }
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
fn opaque_module_source_tokens_reject_cross_image_rebranding() {
    let mut vector = vec![5, 0];
    vector.extend_from_slice(&counted_module_record(0, 0, 0, 0));
    let first = decode_image(&vector).unwrap();
    let second = decode_image(&vector).unwrap();
    let first_id = module_id(first.root());
    let second_id = module_id(second.root());

    assert_eq!(first_id.zero_based(), second_id.zero_based());
    assert_ne!(first_id, second_id);
    assert!(first.module(first_id).is_some());
    assert!(first.module(second_id).is_none());

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
fn module_metadata_errors_precede_their_child_whole_depth_check() {
    let tag = BcTag::Module.to_byte();
    let first_atom = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 0)
        .unwrap()
        .first_atom();

    assert_eq!(
        decode_image(&[5, 0, tag]),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 3,
            needed: 1,
            remaining: 0,
        }))
    );
    assert_eq!(
        decode_image(&[5, 0, tag, 0xe6, 0x03]),
        Err(BytecodeImageError::Wire(WireError::InvalidAtomIndex {
            offset: 5,
            index: first_atom,
            first_atom,
            atom_count: 0,
        }))
    );
    assert_eq!(
        decode_image(&[5, 0, tag, 0x80, 0]),
        Err(BytecodeImageError::Wire(WireError::NonCanonicalUleb128 {
            offset: 3,
        }))
    );

    let shallow = bounded_image_limits(256, 1, 4096, 16384);
    // The request name is parent metadata, so its failure wins before the
    // whole-depth check for the attributes child.
    assert_eq!(
        decode_image_with(&[5, 0, tag, 0, 1], ReaderMode::Strict, shallow, true,),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 5,
            needed: 1,
            remaining: 0,
        }))
    );
    assert_eq!(
        decode_image_with(
            &[5, 0, tag, 0, 1, 0xe6, 0x03],
            ReaderMode::Strict,
            shallow,
            true,
        ),
        Err(BytecodeImageError::Wire(WireError::InvalidAtomIndex {
            offset: 7,
            index: first_atom,
            first_atom,
            atom_count: 0,
        }))
    );
    assert_eq!(
        decode_image_with(&[5, 0, tag, 0, 1, 0], ReaderMode::Strict, shallow, true,),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::WholeDepth,
            requested: 2,
            limit: 1,
        })
    );

    // Exports, star exports, imports, and has_tla are likewise consumed before
    // the func_obj child receives its own depth check.
    assert_eq!(
        decode_image_with(&[5, 0, tag, 0, 0], ReaderMode::Strict, shallow, true,),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 5,
            needed: 1,
            remaining: 0,
        }))
    );
    assert_eq!(
        decode_image_with(
            &[5, 0, tag, 0, 0, 0, 0, 0, 0],
            ReaderMode::Strict,
            shallow,
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::WholeDepth,
            requested: 2,
            limit: 1,
        })
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

    assert_eq!(
        decode_image(&[5, 0, BcTag::SharedArrayBuffer.to_byte()]),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBuffersNotAllowed { offset: 2 }
        ))
    );
}

#[test]
fn sab_transport_whole_image_oracle_preserves_topology_and_normalizes_tokens() {
    const FIRST_TOKEN: u64 = 0x0123_4567_89ab_cdef;
    const RENAMED_TOKEN: u64 = 0xfedc_ba98_7654_3210;

    let first_wire = function_bytecode_sab_reference_wire(FIRST_TOKEN);
    assert_eq!(first_wire.len(), 50);
    let first = decode_sab_image(&first_wire, &[FIRST_TOKEN], ReaderMode::Strict, true)
        .expect("pinned FunctionBytecode/SAB oracle must decode");
    let first_snapshot = snapshot_sab_image(&first);

    assert_eq!(first_snapshot.atoms, []);
    assert_eq!(
        first_snapshot.root,
        SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(0)))
    );
    assert_eq!(first_snapshot.references, [0, 1, 2]);
    assert_eq!(first_snapshot.shared_backing_count, 1);
    assert_eq!(first_snapshot.module_count, 0);
    assert_eq!(first_snapshot.functions.len(), 1);
    assert_eq!(
        first_snapshot.functions[0].envelope.code().as_bytes(),
        [0xbb, 0x2a, 0xcb, 0x28]
    );
    assert!(first_snapshot.functions[0].envelope.debug().is_none());
    assert!(first_snapshot.functions[0].constants.is_empty());
    assert_eq!(
        first_snapshot.nodes.as_slice(),
        [
            SabImageNodeSnapshot::Array(vec![
                SabImageValueSnapshot::Function(0),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(1))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
            ]),
            SabImageNodeSnapshot::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 4,
                byte_offset: 0,
                buffer: 2,
            },
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
        ]
    );
    assert_eq!(
        encode_image(first.test_image()),
        Err(BytecodeImageEncodeError::ArchivedBackingContextRequired {
            node: NodeId::from_zero_based(2),
        })
    );

    let renamed_wire = function_bytecode_sab_reference_wire(RENAMED_TOKEN);
    let renamed = decode_sab_image(&renamed_wire, &[RENAMED_TOKEN], ReaderMode::Strict, true)
        .expect("alpha-renamed native token must decode to the same semantic image");
    assert_eq!(snapshot_sab_image(&renamed), first_snapshot);

    for (archive, raw_token) in [(&first, FIRST_TOKEN), (&renamed, RENAMED_TOKEN)] {
        let debug = format!("{archive:#?}");
        for spelling in [
            raw_token.to_string(),
            format!("{raw_token:x}"),
            format!("{raw_token:X}"),
            format!("0x{raw_token:016x}"),
            format!("0x{raw_token:016X}"),
        ] {
            assert!(
                !debug.contains(&spelling),
                "ArchivedBytecodeImage Debug leaked native token spelling {spelling}"
            );
        }
    }
}

#[test]
fn sab_transport_whole_image_canonicalizes_repeated_and_distinct_backings() {
    const FIRST_TOKEN: u64 = 0x0123_4567_89ab_cdef;
    const SECOND_TOKEN: u64 = 0xfedc_ba98_7654_3210;
    let limits = sab_image_limits_for(2, 2, 8);

    let repeated_wire = two_sab_records_wire(FIRST_TOKEN, FIRST_TOKEN);
    let repeated = decode_sab_image_with_limits(
        &repeated_wire,
        &[FIRST_TOKEN, FIRST_TOKEN],
        ReaderMode::Strict,
        true,
        limits,
    )
    .expect("two complete SAB records with one token must share one archived backing");
    let repeated = snapshot_sab_image(&repeated);
    assert_eq!(repeated.references, [0, 1, 2]);
    assert_eq!(repeated.shared_backing_count, 1);
    assert_eq!(
        repeated.root,
        SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(0)))
    );
    assert_eq!(
        repeated.nodes,
        [
            SabImageNodeSnapshot::Array(vec![
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(1))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
            ]),
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
        ]
    );

    let distinct_wire = two_sab_records_wire(FIRST_TOKEN, SECOND_TOKEN);
    let distinct = decode_sab_image_with_limits(
        &distinct_wire,
        &[FIRST_TOKEN, SECOND_TOKEN],
        ReaderMode::Strict,
        true,
        limits,
    )
    .expect("two complete SAB records with distinct tokens must retain distinct backings");
    let distinct = snapshot_sab_image(&distinct);
    assert_eq!(distinct.references, [0, 1, 2]);
    assert_eq!(distinct.shared_backing_count, 2);
    assert_eq!(
        distinct.nodes,
        [
            SabImageNodeSnapshot::Array(vec![
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(1))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
            ]),
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 1,
                capacity: 4,
                growable: false,
            },
        ]
    );
}

#[test]
fn sab_transport_whole_image_rejects_split_and_truncated_inputs() {
    const TOKEN: u64 = 0x0123_4567_89ab_cdef;
    let wire = function_bytecode_sab_reference_wire(TOKEN);

    assert_eq!(
        decode_sab_image(&wire, &[], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableTooShort {
                offset: 38,
                ordinal: 0,
                entry_count: 0,
            })
        ))
    );
    assert_eq!(
        decode_sab_image(&wire, &[TOKEN ^ 1], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableTokenMismatch {
                offset: 38,
                ordinal: 0,
            })
        ))
    );
    assert_eq!(
        decode_sab_image(&wire, &[TOKEN, TOKEN ^ 1], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableHasExtra {
                consumed: 1,
                entry_count: 2,
            })
        ))
    );
    assert_eq!(
        decode_sab_image(&wire[..45], &[TOKEN], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(DecodeError::Wire(
            WireError::Truncated {
                offset: 38,
                needed: 8,
                remaining: 7,
            }
        )))
    );
}

#[test]
fn sab_transport_whole_image_preserves_reference_and_finalization_policy() {
    const TOKEN: u64 = 0x0123_4567_89ab_cdef;
    let wire = function_bytecode_sab_reference_wire(TOKEN);
    let writer_occurrences = [NativeSabToken::from_test_bits(TOKEN)];
    assert_eq!(
        decode_bytecode_image_with_sab_transport(
            SabTransportInput::new(&wire, &writer_occurrences),
            ReaderMode::Strict,
            TEST_LIMITS,
            IMAGE_LIMITS,
            true,
        ),
        Err(BytecodeImageError::Data(DecodeError::Graph(
            GraphError::ResourceLimit {
                kind: GraphResourceKind::SharedArrayBufferOccurrences,
                requested: 1,
                limit: 0,
            }
        )))
    );
    assert_eq!(
        decode_sab_image(&wire, &[TOKEN], ReaderMode::Strict, false),
        Err(BytecodeImageError::Data(
            DecodeError::ObjectReferencesNotAllowed { offset: 46 }
        ))
    );

    let mut trailing = wire;
    trailing.push(0xff);
    assert_eq!(
        decode_sab_image(&trailing, &[TOKEN, TOKEN ^ 1], ReaderMode::Strict, true),
        Err(BytecodeImageError::Wire(WireError::TrailingBytes {
            offset: 50,
            remaining: 1,
        }))
    );
    assert_eq!(
        decode_sab_image(
            &trailing,
            &[TOKEN, TOKEN ^ 1],
            ReaderMode::QuickJsCompatible,
            true,
        ),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableHasExtra {
                consumed: 1,
                entry_count: 2,
            })
        ))
    );
}

#[test]
fn image_writer_requires_archived_context_for_reachable_shared_backings_only() {
    const TOKEN: u64 = 0xfeed_face_dead_beef;
    let mut wire = vec![
        5,
        0,
        BcTag::SharedArrayBuffer.to_byte(),
        4,
        0xff,
        0xff,
        0xff,
        0xff,
        0x0f,
    ];
    wire.extend_from_slice(&TOKEN.to_le_bytes());
    let writer_occurrences = [NativeSabToken::from_test_bits(TOKEN)];
    let archive = decode_graph_with_sab_transport(
        SabTransportInput::new(&wire, &writer_occurrences),
        ReaderMode::Strict,
        TEST_LIMITS,
        GRAPH_LIMITS.with_shared_array_buffers(1, 1, 4, 4),
        true,
    )
    .unwrap();
    let WireNodeCarrier::SharedArrayBuffer {
        byte_length,
        max_byte_length,
        backing,
    } = archive.test_graph().nodes[0]
    else {
        panic!("transport must archive one SharedArrayBuffer node");
    };

    let image_with_graph = |nodes: Vec<WireNodeCarrier<ImageValue, ImageKey>>, root: WireValue| {
        let machine = DataMachine::<ImageValue, ImageKey>::new(GRAPH_LIMITS, true).unwrap();
        super::BytecodeImage::new(
            machine.source(),
            Box::default(),
            nodes.into_boxed_slice(),
            Box::default(),
            Box::default(),
            Box::default(),
            ImageValue::from_wire(root),
        )
    };
    let shared_node = || WireNodeCarrier::SharedArrayBuffer {
        byte_length,
        max_byte_length,
        backing,
    };

    let direct = image_with_graph(
        vec![shared_node()],
        WireValue::Node(NodeId::from_zero_based(0)),
    );
    assert_eq!(
        encode_image(&direct),
        Err(BytecodeImageEncodeError::ArchivedBackingContextRequired {
            node: NodeId::from_zero_based(0),
        })
    );

    let viewed = image_with_graph(
        vec![
            WireNodeCarrier::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 4,
                byte_offset: 0,
                buffer: NodeId::from_zero_based(1),
            },
            shared_node(),
        ],
        WireValue::Node(NodeId::from_zero_based(0)),
    );
    assert_eq!(
        encode_image(&viewed),
        Err(BytecodeImageEncodeError::ArchivedBackingContextRequired {
            node: NodeId::from_zero_based(1),
        })
    );

    let unreachable = image_with_graph(vec![shared_node()], WireValue::Int32(42));
    assert_eq!(encode_image(&unreachable), Ok(vec![5, 0, 5, 84]));
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
