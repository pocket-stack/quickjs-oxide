use super::*;
use crate::engine::atom::ATOM_TAG_INT;

use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::code::{CodeError, CodeLimits, CodeResourceKind};
use super::super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits};
use super::super::graph::decode::{DataMachine, DecodeError, decode_graph_with_sab_transport};
use super::super::graph::model::{
    AtomId, GraphError, GraphLimits, GraphResourceKind, NodeId, TypedArrayKind, WireNodeCarrier,
    WireValue,
};
use super::super::graph::sab_transport::{NativeSabToken, SabArchiveError, SabTransportInput};
use super::super::pinned_atoms::{FIRST_DYNAMIC_ATOM, PinnedAtomId, PinnedAtomKind};
use super::super::wire::{
    BcTag, ReaderMode, ResourceKind, WireCursor, WireError, WireLimits, WireString, WireWriter,
};
use super::{
    BytecodeImageBudgetError, BytecodeImageEncodeError, BytecodeImageEncodeOptions,
    BytecodeImageError, BytecodeImageLimits, BytecodeImageResourceKind, FunctionId, ImageAtom,
    ImageAtomError, ImageAtomTable, ImageFunctionEnvelope, ImageKey, ImageOpaque, ImageValue,
    ModuleBudgetError, ModuleField, ModuleLimits, ModuleResourceKind, NativeAtomClass,
    NativeAtomRef, NativeOperands, decode_bytecode_image, decode_bytecode_image_with_sab_transport,
    decode_native_code_plan, encode_bytecode_image,
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
    atom_relocation_record_with_raw_atom(1)
}

fn atom_relocation_record_with_raw_atom(raw_atom: u32) -> Vec<u8> {
    let mut record = quickjs_42_record();
    record[13] = 6;
    record.truncate(19);
    record.push(4);
    record.extend_from_slice(&raw_atom.to_le_bytes());
    record.push(0x28);
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

fn first_native_atom(image: &super::BytecodeImage) -> NativeAtomRef<'_> {
    let plan = decode_native_code_plan(image, function_id(image.root()))
        .expect("test image must have an authenticated native plan");
    let instruction = plan
        .instructions()
        .first()
        .expect("test plan must contain its atom instruction");
    match instruction.operands() {
        NativeOperands::Atom(atom) => *atom,
        operands => panic!("test plan must begin with an Atom operand, got {operands:?}"),
    }
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

mod atoms;

mod reader;

mod native_decoding;

mod modules;

mod topology;

mod budgets;

mod shared_buffers;

mod writer;
