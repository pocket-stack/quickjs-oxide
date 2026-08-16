use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::code::{CodeError, CodeLimits, CodeResourceKind};
use super::super::pinned_atoms::PinnedAtomId;
use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits, WireWriter};
use super::*;

const WIRE_LIMITS: WireLimits = WireLimits::new(4096, 16, 1024, 2048);
const CODE_LIMITS: CodeLimits = CodeLimits::new(256, 128, 32);
const QUICKJS_FUNCTION_BYTECODE_ORACLE: &str =
    include_str!("../../../../tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt");

fn oracle_field(name: &str) -> &str {
    QUICKJS_FUNCTION_BYTECODE_ORACLE
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .unwrap_or_else(|| panic!("authenticated QuickJS transcript lost {name}"))
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("authenticated QuickJS transcript contains non-lowercase hex"),
    }
}

fn decode_oracle_hex(hex: &str) -> Vec<u8> {
    let bytes = hex.as_bytes();
    assert_eq!(bytes.len() % 2, 0, "oracle hex must contain whole bytes");
    bytes
        .chunks_exact(2)
        .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
        .collect()
}

fn assert_write_error_without_mutation(
    prefix: &FunctionRecordPrefix,
    expected: FunctionEnvelopeError,
) {
    let mut output = WireWriter::new(64);
    output.write_u8(0xaa).unwrap();
    assert_eq!(
        write_function_record_prefix_after_tag(prefix, &mut output),
        Err(expected)
    );
    assert_eq!(output.as_bytes(), [0xaa]);
}

fn limits() -> FunctionEnvelopeLimits {
    FunctionEnvelopeLimits::new(16, 16, 16, 64, 64, 96, CODE_LIMITS)
}

fn limits_with(
    locals: usize,
    closures: usize,
    constants: usize,
    pc2line: usize,
    source: usize,
    total_debug: usize,
    code: CodeLimits,
) -> FunctionEnvelopeLimits {
    FunctionEnvelopeLimits::new(
        locals,
        closures,
        constants,
        pc2line,
        source,
        total_debug,
        code,
    )
}

fn bytecode_space(header_count: u32) -> AtomIndexSpace {
    AtomIndexSpace::new(BinaryObjectMode::Bytecode, header_count).unwrap()
}

fn read_prefix(
    input: &[u8],
    mode: ReaderMode,
    header_count: u32,
    function_limits: FunctionEnvelopeLimits,
) -> Result<(FunctionRecordPrefix, usize), FunctionEnvelopeError> {
    let mut cursor = WireCursor::new(input, mode, WIRE_LIMITS).unwrap();
    assert_eq!(cursor.read_tag(), Ok(BcTag::FunctionBytecode));
    let prefix = read_function_record_prefix_after_tag(
        &mut cursor,
        bytecode_space(header_count),
        function_limits,
    )?;
    Ok((prefix, cursor.position()))
}

fn begin_function(writer: &mut WireWriter, flags: u16) {
    writer.write_tag(BcTag::FunctionBytecode).unwrap();
    writer.write_u16_le(flags).unwrap();
    writer.write_u8(0).unwrap();
    writer.write_uleb128(0).unwrap();
}

fn write_counts(
    writer: &mut WireWriter,
    frame: [u32; 5],
    closures: u32,
    constants: u32,
    code_length: u32,
    locals: u32,
) {
    for count in frame {
        writer.write_uleb128(count).unwrap();
    }
    writer.write_uleb128(closures).unwrap();
    writer.write_uleb128(constants).unwrap();
    writer.write_uleb128(code_length).unwrap();
    writer.write_uleb128(locals).unwrap();
}

fn full_prefix_vector() -> (Vec<u8>, usize) {
    let mut writer = WireWriter::new(256);
    writer.write_tag(BcTag::FunctionBytecode).unwrap();
    writer.write_u16_le(0x0fff).unwrap();
    writer.write_u8(0xff).unwrap();
    writer.write_uleb128(8).unwrap();
    write_counts(&mut writer, [1, 1, 1, 2, 1], 1, 2, 6, 2);

    writer.write_uleb128(0).unwrap();
    writer.write_uleb128(u32::MAX).unwrap();
    writer.write_uleb128(u32::from(u16::MAX)).unwrap();
    writer.write_u8(0xf2).unwrap();

    writer.write_uleb128(85).unwrap();
    writer.write_uleb128(0).unwrap();
    writer.write_uleb128(0).unwrap();
    writer.write_u8(0x0f).unwrap();

    writer.write_uleb128(8).unwrap();
    writer.write_uleb128(u32::from(u16::MAX)).unwrap();
    writer.write_u16_le(0x01ff).unwrap();

    writer.write_bytes(&[1, 42, 0, 0, 0, 40]).unwrap();

    writer.write_uleb128(486).unwrap();
    writer.write_uleb128(3).unwrap();
    writer.write_bytes(&[0, 0xff, 0x80]).unwrap();
    writer.write_uleb128(4).unwrap();
    writer.write_bytes(&[0, 0xff, b'J', b'S']).unwrap();

    let prefix_end = writer.as_bytes().len();
    writer
        .write_bytes(&[BcTag::BoolFalse.to_byte(), BcTag::BoolTrue.to_byte()])
        .unwrap();
    (writer.into_bytes(), prefix_end)
}

fn minimal_prefix(
    function_flags: u16,
    frame: [u32; 5],
    constants: u32,
    local_variable_reference: Option<u32>,
    closure: Option<(u32, u16)>,
    code: &[u8],
) -> Vec<u8> {
    let mut writer = WireWriter::new(256);
    begin_function(&mut writer, function_flags);
    write_counts(
        &mut writer,
        frame,
        u32::from(closure.is_some()),
        constants,
        u32::try_from(code.len()).unwrap(),
        u32::from(local_variable_reference.is_some()),
    );
    if let Some(variable_reference) = local_variable_reference {
        writer.write_uleb128(0).unwrap();
        writer.write_uleb128(0).unwrap();
        writer.write_uleb128(variable_reference).unwrap();
        writer.write_u8(0).unwrap();
    }
    if let Some((variable_index, flags)) = closure {
        writer.write_uleb128(0).unwrap();
        writer.write_uleb128(variable_index).unwrap();
        writer.write_u16_le(flags).unwrap();
    }
    writer.write_bytes(code).unwrap();
    writer.into_bytes()
}

fn narrow_value(prefix: &FunctionRecordPrefix, field: FunctionField) -> u16 {
    let envelope = prefix.envelope();
    match field {
        FunctionField::ArgumentCount => envelope.argument_count(),
        FunctionField::VariableCount => envelope.variable_count(),
        FunctionField::DefinedArgumentCount => envelope.defined_argument_count(),
        FunctionField::StackSize => envelope.stack_size(),
        FunctionField::VariableReferenceCount => envelope.variable_reference_count(),
        FunctionField::LocalVariableReferenceIndex => {
            envelope.locals()[0].variable_reference_index()
        }
        FunctionField::ClosureVariableIndex => envelope.closures()[0].variable_index(),
        _ => panic!("test field is not a narrowing u16"),
    }
}

fn narrowing_vector(field: FunctionField, value: u32) -> Vec<u8> {
    let mut frame = [0; 5];
    let mut local = None;
    let mut closure = None;
    match field {
        FunctionField::ArgumentCount => frame[0] = value,
        FunctionField::VariableCount => frame[1] = value,
        FunctionField::DefinedArgumentCount => frame[2] = value,
        FunctionField::StackSize => frame[3] = value,
        FunctionField::VariableReferenceCount => frame[4] = value,
        FunctionField::LocalVariableReferenceIndex => {
            frame[0] = 1;
            local = Some(value);
        }
        FunctionField::ClosureVariableIndex => closure = Some((value, 0)),
        _ => panic!("test field is not a narrowing u16"),
    }
    minimal_prefix(0, frame, 0, local, closure, &[41])
}

#[test]
fn authenticated_quickjs_return_42_prefix_round_trips_byte_for_byte() {
    assert_eq!(oracle_field("quickjs="), "2026-06-04");
    assert_eq!(oracle_field("source-hex="), "34323b");
    assert_eq!(oracle_field("strip-flags="), "2");
    assert_eq!(oracle_field("fresh-eval="), "42");

    let input = decode_oracle_hex(oracle_field("bytecode-hex="));
    assert_eq!(input.len().to_string(), oracle_field("bytecode-size="));
    let mut cursor = WireCursor::new(&input, ReaderMode::Strict, WIRE_LIMITS).unwrap();
    let header = cursor.read_header().unwrap();
    let header_atoms: Vec<_> = (0..header.atom_count)
        .map(|_| cursor.read_string().unwrap())
        .collect();
    assert_eq!(header.atom_count, 0);
    assert_eq!(cursor.read_tag(), Ok(BcTag::FunctionBytecode));
    let prefix = read_function_record_prefix_after_tag(
        &mut cursor,
        bytecode_space(header.atom_count),
        limits(),
    )
    .unwrap();
    assert_eq!(prefix.pending_constant_pool_count(), 0);
    assert!(prefix.envelope().debug().is_none());
    cursor.finish().unwrap();

    let mut canonical = WireWriter::new(input.len());
    canonical.write_header(header.atom_count).unwrap();
    for atom in &header_atoms {
        canonical.write_string(atom).unwrap();
    }
    canonical.write_tag(BcTag::FunctionBytecode).unwrap();
    write_function_record_prefix_after_tag(&prefix, &mut canonical).unwrap();
    assert_eq!(canonical.as_bytes(), input);
}

#[test]
fn prefix_writer_rejects_nonsemantic_internal_bits_and_scope_overflow() {
    let (mut function_bits, _) = read_prefix(
        &minimal_prefix(0, [0; 5], 0, None, None, &[41]),
        ReaderMode::Strict,
        0,
        limits(),
    )
    .unwrap();
    function_bits.envelope.flags = FunctionFlags(1 << 10);
    let mut output = WireWriter::new(64);
    output.write_u8(0xaa).unwrap();
    assert_eq!(
        write_function_record_prefix_after_tag(&function_bits, &mut output),
        Err(FunctionEnvelopeError::InvalidModelBits {
            field: FunctionField::FunctionFlags,
            bits: 1 << 10,
        })
    );
    assert_eq!(output.as_bytes(), [0xaa]);

    let (mut closure_bits, _) = read_prefix(
        &minimal_prefix(0, [0; 5], 0, None, Some((0, 0)), &[41]),
        ReaderMode::Strict,
        0,
        limits(),
    )
    .unwrap();
    closure_bits.envelope.closures[0].flags = ClosureVariableFlags(1 << 9);
    let mut output = WireWriter::new(64);
    output.write_u8(0xbb).unwrap();
    assert_eq!(
        write_function_record_prefix_after_tag(&closure_bits, &mut output),
        Err(FunctionEnvelopeError::InvalidModelBits {
            field: FunctionField::ClosureFlags,
            bits: 1 << 9,
        })
    );
    assert_eq!(output.as_bytes(), [0xbb]);

    let (mut scope_overflow, _) = read_prefix(
        &minimal_prefix(0, [1, 0, 0, 0, 0], 0, Some(0), None, &[41]),
        ReaderMode::Strict,
        0,
        limits(),
    )
    .unwrap();
    scope_overflow.envelope.locals[0].scope_next = ScopeLink(i32::MAX);
    let mut output = WireWriter::new(64);
    output.write_u8(0xcc).unwrap();
    assert_eq!(
        write_function_record_prefix_after_tag(&scope_overflow, &mut output),
        Err(FunctionEnvelopeError::CountOverflow {
            field: FunctionField::LocalScopeNext,
        })
    );
    assert_eq!(output.as_bytes(), [0xcc]);
}

#[test]
fn prefix_writer_prevalidates_the_complete_atom_namespace_without_output() {
    let (base, _) = read_prefix(
        &minimal_prefix(0, [0; 5], 0, None, None, &[41]),
        ReaderMode::Strict,
        0,
        limits(),
    )
    .unwrap();

    let mut data_namespace = base.clone();
    data_namespace.envelope.atom_space = AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap();
    assert_write_error_without_mutation(
        &data_namespace,
        FunctionEnvelopeError::InvalidAtomMode {
            found: BinaryObjectMode::Data,
        },
    );

    let mut mismatched_code = base;
    mismatched_code.envelope.atom_space = bytecode_space(1);
    assert_write_error_without_mutation(
        &mismatched_code,
        FunctionEnvelopeError::MismatchedAtomSpace {
            envelope: bytecode_space(1),
            code: bytecode_space(0),
        },
    );

    let (input, _) = full_prefix_vector();
    let (template, _) = read_prefix(&input, ReaderMode::Strict, 1, limits()).unwrap();
    let invalid_atom = BinaryAtom::Header(bytecode_space(2).header_slot(1).unwrap());
    for field in 0..4 {
        let mut prefix = template.clone();
        match field {
            0 => prefix.envelope.name = invalid_atom,
            1 => prefix.envelope.locals[0].name = invalid_atom,
            2 => prefix.envelope.closures[0].name = invalid_atom,
            3 => prefix.envelope.debug.as_mut().unwrap().filename = invalid_atom,
            _ => unreachable!(),
        }
        assert_write_error_without_mutation(
            &prefix,
            FunctionEnvelopeError::InvalidModelAtom {
                atom: invalid_atom,
                atom_space: bytecode_space(1),
            },
        );
    }
}

#[test]
fn fixed_prefix_round_trips_every_field_and_stops_before_the_pool() {
    let (input, prefix_end) = full_prefix_vector();
    let mut cursor = WireCursor::new(&input, ReaderMode::Strict, WIRE_LIMITS).unwrap();
    assert_eq!(cursor.read_tag(), Ok(BcTag::FunctionBytecode));
    let prefix =
        read_function_record_prefix_after_tag(&mut cursor, bytecode_space(1), limits()).unwrap();
    assert_eq!(cursor.position(), prefix_end);
    assert_eq!(prefix.pending_constant_pool_count(), 2);
    assert_eq!(cursor.read_tag(), Ok(BcTag::BoolFalse));
    assert_eq!(cursor.read_tag(), Ok(BcTag::BoolTrue));
    cursor.finish().unwrap();

    let envelope = prefix.envelope();
    let flags = envelope.flags();
    assert_eq!(flags.raw(), 0x0bff);
    assert!(flags.has_prototype());
    assert!(flags.has_simple_parameter_list());
    assert!(flags.is_derived_class_constructor());
    assert!(flags.needs_home_object());
    assert_eq!(flags.kind(), FunctionKind::AsyncGenerator);
    assert!(flags.allows_new_target());
    assert!(flags.allows_super_call());
    assert!(flags.allows_super_property());
    assert!(flags.allows_arguments());
    assert!(flags.is_direct_or_indirect_eval());

    assert_eq!(envelope.js_mode().raw(), 0xff);
    assert!(envelope.js_mode().is_strict());
    assert!(envelope.js_mode().is_async());
    assert!(envelope.js_mode().is_backtrace_barrier());
    assert_eq!(
        envelope.name(),
        BinaryAtom::Predefined(PinnedAtomId::from_raw(4).unwrap())
    );
    assert_eq!(envelope.argument_count(), 1);
    assert_eq!(envelope.variable_count(), 1);
    assert_eq!(envelope.defined_argument_count(), 1);
    assert_eq!(envelope.stack_size(), 2);
    assert_eq!(envelope.variable_reference_count(), 1);

    let [first, second] = envelope.locals() else {
        panic!("expected two locals");
    };
    assert_eq!(first.name(), BinaryAtom::Null);
    assert_eq!(first.scope_next().value(), -2);
    assert_eq!(first.variable_reference_index(), u16::MAX);
    assert_eq!(first.flags().kind().raw(), 2);
    assert!(first.flags().is_const());
    assert!(first.flags().is_lexical());
    assert!(first.flags().is_captured());
    assert!(first.flags().has_scope());
    assert_eq!(second.name(), BinaryAtom::Index(42));
    assert_eq!(second.scope_next().value(), -1);
    assert_eq!(second.flags().kind().raw(), 15);

    let [closure] = envelope.closures() else {
        panic!("expected one closure");
    };
    assert_eq!(
        closure.name(),
        BinaryAtom::Predefined(PinnedAtomId::from_raw(4).unwrap())
    );
    assert_eq!(closure.variable_index(), u16::MAX);
    assert_eq!(closure.flags().raw(), 0x01ff);
    assert_eq!(closure.flags().closure_type(), ClosureType::ModuleImport);
    assert!(closure.flags().is_const());
    assert!(closure.flags().is_lexical());
    assert_eq!(closure.flags().kind().raw(), 15);

    assert_eq!(envelope.code().as_bytes(), [1, 42, 0, 0, 0, 40]);
    let debug = envelope.debug().unwrap();
    assert_eq!(
        debug.filename(),
        BinaryAtom::Header(bytecode_space(1).header_slot(0).unwrap())
    );
    assert_eq!(debug.pc2line(), [0, 0xff, 0x80]);
    assert_eq!(debug.source(), [0, 0xff, b'J', b'S']);

    let mut rewritten = WireWriter::new(256);
    write_function_record_prefix_after_tag(&prefix, &mut rewritten).unwrap();
    assert_eq!(rewritten.as_bytes(), &input[1..prefix_end]);
    assert_ne!(
        rewritten.as_bytes().first().copied(),
        Some(BcTag::FunctionBytecode.to_byte())
    );
}

#[test]
fn all_quickjs_u16_narrowing_fields_have_strict_and_compatible_modes() {
    for field in [
        FunctionField::ArgumentCount,
        FunctionField::VariableCount,
        FunctionField::DefinedArgumentCount,
        FunctionField::StackSize,
        FunctionField::VariableReferenceCount,
        FunctionField::LocalVariableReferenceIndex,
        FunctionField::ClosureVariableIndex,
    ] {
        let maximum = narrowing_vector(field, u32::from(u16::MAX));
        let (prefix, end) = read_prefix(&maximum, ReaderMode::Strict, 0, limits()).unwrap();
        assert_eq!(end, maximum.len(), "{field:?}");
        assert_eq!(narrow_value(&prefix, field), u16::MAX, "{field:?}");

        let aliased = narrowing_vector(field, u32::from(u16::MAX) + 1);
        assert!(matches!(
            read_prefix(&aliased, ReaderMode::Strict, 0, limits()),
            Err(FunctionEnvelopeError::FieldOutOfRange {
                field: found,
                value: 65_536,
                maximum: 65_535,
                ..
            }) if found == field
        ));
        let (prefix, end) =
            read_prefix(&aliased, ReaderMode::QuickJsCompatible, 0, limits()).unwrap();
        assert_eq!(end, aliased.len(), "{field:?}");
        assert_eq!(narrow_value(&prefix, field), 0, "{field:?}");

        let mut canonical = WireWriter::new(256);
        write_function_record_prefix_after_tag(&prefix, &mut canonical).unwrap();
        assert!(
            canonical.as_bytes().len() < aliased.len() - 1,
            "{field:?} should rewrite the truncated count canonically"
        );
    }
}

#[test]
fn reserved_function_and_closure_bits_reject_strictly_and_normalize_compatibly() {
    let function_reserved = minimal_prefix(1 << 12, [0; 5], 0, None, None, &[41]);
    assert!(matches!(
        read_prefix(&function_reserved, ReaderMode::Strict, 0, limits()),
        Err(FunctionEnvelopeError::ReservedBits {
            field: FunctionField::FunctionFlags,
            bits: 0x1000,
            offset: 1,
        })
    ));
    let (prefix, _) = read_prefix(
        &function_reserved,
        ReaderMode::QuickJsCompatible,
        0,
        limits(),
    )
    .unwrap();
    let mut canonical = WireWriter::new(64);
    write_function_record_prefix_after_tag(&prefix, &mut canonical).unwrap();
    assert_eq!(&canonical.as_bytes()[..2], [0, 0]);

    let closure_reserved = minimal_prefix(0, [0; 5], 0, None, Some((0, 1 << 9)), &[41]);
    assert!(matches!(
        read_prefix(&closure_reserved, ReaderMode::Strict, 0, limits()),
        Err(FunctionEnvelopeError::ReservedBits {
            field: FunctionField::ClosureFlags,
            bits: 0x0200,
            ..
        })
    ));
    let (prefix, _) = read_prefix(
        &closure_reserved,
        ReaderMode::QuickJsCompatible,
        0,
        limits(),
    )
    .unwrap();
    assert_eq!(prefix.envelope().closures()[0].flags().raw(), 0);
}

#[test]
fn scope_sentinels_round_trip_but_signed_decrement_overflow_is_rejected() {
    let (prefix, _) = read_prefix(
        &minimal_prefix(0, [1, 0, 0, 0, 0], 0, Some(0), None, &[41]),
        ReaderMode::Strict,
        0,
        limits(),
    )
    .unwrap();
    assert_eq!(prefix.envelope().locals()[0].scope_next().value(), -1);

    let mut invalid = WireWriter::new(64);
    begin_function(&mut invalid, 0);
    write_counts(&mut invalid, [1, 0, 0, 0, 0], 0, 0, 1, 1);
    invalid.write_uleb128(0).unwrap();
    invalid.write_uleb128(0x8000_0000).unwrap();
    invalid.write_uleb128(0).unwrap();
    invalid.write_u8(0).unwrap();
    invalid.write_u8(41).unwrap();
    for mode in [ReaderMode::Strict, ReaderMode::QuickJsCompatible] {
        assert!(matches!(
            read_prefix(invalid.as_bytes(), mode, 0, limits()),
            Err(FunctionEnvelopeError::InvalidScopeEncoding {
                encoded: 0x8000_0000,
                ..
            })
        ));
    }
}

#[test]
fn signed_count_bit_patterns_are_safely_rejected_in_both_modes() {
    for field in [
        FunctionField::ClosureVariableCount,
        FunctionField::ConstantPoolCount,
        FunctionField::ByteCodeLength,
        FunctionField::LocalCount,
    ] {
        let mut writer = WireWriter::new(64);
        begin_function(&mut writer, 0);
        for _ in 0..5 {
            writer.write_uleb128(0).unwrap();
        }
        for current in [
            FunctionField::ClosureVariableCount,
            FunctionField::ConstantPoolCount,
            FunctionField::ByteCodeLength,
            FunctionField::LocalCount,
        ] {
            writer
                .write_uleb128(if current == field { 0x8000_0000 } else { 0 })
                .unwrap();
        }
        for mode in [ReaderMode::Strict, ReaderMode::QuickJsCompatible] {
            assert!(matches!(
                read_prefix(writer.as_bytes(), mode, 0, limits()),
                Err(FunctionEnvelopeError::FieldOutOfRange {
                    field: found,
                    value: 0x8000_0000,
                    maximum: 0x7fff_ffff,
                    ..
                }) if found == field
            ));
        }
    }
}

#[test]
fn signed_debug_length_bit_patterns_are_rejected_in_both_modes() {
    for field in [FunctionField::Pc2LineLength, FunctionField::SourceLength] {
        let mut writer = WireWriter::new(64);
        begin_function(&mut writer, 1 << 10);
        write_counts(&mut writer, [0; 5], 0, 0, 1, 0);
        writer.write_u8(41).unwrap();
        writer.write_uleb128(0).unwrap();
        if field == FunctionField::SourceLength {
            writer.write_uleb128(0).unwrap();
        }
        writer.write_uleb128(0x8000_0000).unwrap();

        for mode in [ReaderMode::Strict, ReaderMode::QuickJsCompatible] {
            assert!(matches!(
                read_prefix(writer.as_bytes(), mode, 0, limits()),
                Err(FunctionEnvelopeError::FieldOutOfRange {
                    field: found,
                    value: 0x8000_0000,
                    maximum: 0x7fff_ffff,
                    ..
                }) if found == field
            ));
        }
    }
}

#[test]
fn local_table_shape_is_strict_and_compatible_input_cannot_be_reemitted() {
    let mut mismatched = WireWriter::new(64);
    begin_function(&mut mismatched, 0);
    write_counts(&mut mismatched, [1, 1, 0, 0, 0], 0, 0, 1, 1);
    mismatched.write_uleb128(0).unwrap();
    mismatched.write_uleb128(0).unwrap();
    mismatched.write_uleb128(0).unwrap();
    mismatched.write_u8(0).unwrap();
    mismatched.write_u8(41).unwrap();

    assert!(matches!(
        read_prefix(mismatched.as_bytes(), ReaderMode::Strict, 0, limits(),),
        Err(FunctionEnvelopeError::NonCanonicalLocalTableLength {
            argument_count: 1,
            variable_count: 1,
            local_count: 1,
        })
    ));
    let (prefix, _) = read_prefix(
        mismatched.as_bytes(),
        ReaderMode::QuickJsCompatible,
        0,
        limits(),
    )
    .unwrap();
    assert!(matches!(
        write_function_record_prefix_after_tag(&prefix, &mut WireWriter::new(64),),
        Err(FunctionEnvelopeError::NonCanonicalLocalTableLength {
            argument_count: 1,
            variable_count: 1,
            local_count: 1,
        })
    ));

    let stripped = minimal_prefix(0, [1, 1, 0, 0, 0], 0, None, None, &[41]);
    assert!(read_prefix(&stripped, ReaderMode::Strict, 0, limits()).is_ok());
}

#[test]
fn debug_lengths_preserve_binary_bytes_and_enforce_each_budget() {
    let (input, prefix_end) = full_prefix_vector();
    for (function_limits, expected_kind) in [
        (
            limits_with(16, 16, 16, 2, 64, 96, CODE_LIMITS),
            FunctionResourceKind::Pc2LineBytes,
        ),
        (
            limits_with(16, 16, 16, 64, 3, 96, CODE_LIMITS),
            FunctionResourceKind::SourceBytes,
        ),
        (
            limits_with(16, 16, 16, 64, 64, 6, CODE_LIMITS),
            FunctionResourceKind::TotalDebugBytes,
        ),
    ] {
        assert!(matches!(
            read_prefix(
                &input,
                ReaderMode::Strict,
                1,
                function_limits,
            ),
            Err(FunctionEnvelopeError::ResourceLimit { kind, .. })
                if kind == expected_kind
        ));
    }

    let empty_debug = minimal_prefix(1 << 10, [0; 5], 0, None, None, &[41]);
    let mut empty_debug = empty_debug;
    empty_debug.extend_from_slice(&[0, 0, 0]);
    let (prefix, end) = read_prefix(&empty_debug, ReaderMode::Strict, 0, limits()).unwrap();
    assert_eq!(end, empty_debug.len());
    let debug = prefix.envelope().debug().unwrap();
    assert!(debug.pc2line().is_empty());
    assert!(debug.source().is_empty());

    let truncated = input[..prefix_end - 1].to_vec();
    assert!(matches!(
        read_prefix(&truncated, ReaderMode::Strict, 1, limits()),
        Err(FunctionEnvelopeError::Wire(WireError::Truncated { .. }))
    ));
}

#[test]
fn structural_resource_limits_fail_before_container_allocations() {
    let local = minimal_prefix(0, [1, 0, 0, 0, 0], 0, Some(0), None, &[41]);
    let closure = minimal_prefix(0, [0; 5], 0, None, Some((0, 0)), &[41]);
    let constants = minimal_prefix(0, [0; 5], 2, None, None, &[41]);
    for (input, function_limits, expected_kind) in [
        (
            local,
            limits_with(0, 16, 16, 64, 64, 96, CODE_LIMITS),
            FunctionResourceKind::LocalVariables,
        ),
        (
            closure,
            limits_with(16, 0, 16, 64, 64, 96, CODE_LIMITS),
            FunctionResourceKind::ClosureVariables,
        ),
        (
            constants,
            limits_with(16, 16, 1, 64, 64, 96, CODE_LIMITS),
            FunctionResourceKind::ConstantPoolEntries,
        ),
    ] {
        assert!(matches!(
            read_prefix(
                &input,
                ReaderMode::Strict,
                0,
                function_limits,
            ),
            Err(FunctionEnvelopeError::ResourceLimit { kind, .. })
                if kind == expected_kind
        ));
    }

    let code = minimal_prefix(0, [0; 5], 0, None, None, &[1, 0, 0, 0, 0, 40]);
    assert!(matches!(
        read_prefix(
            &code,
            ReaderMode::Strict,
            0,
            limits_with(16, 16, 16, 64, 64, 96, CodeLimits::new(5, 128, 32),),
        ),
        Err(FunctionEnvelopeError::Code(CodeError::ResourceLimit {
            kind: CodeResourceKind::Bytes,
            requested: 6,
            limit: 5,
        }))
    ));
}

#[test]
fn code_atom_diagnostics_use_the_consumed_payload_end() {
    let input = minimal_prefix(0, [0; 5], 0, None, None, &[4, 0xf3, 0, 0, 0]);
    assert!(matches!(
        read_prefix(&input, ReaderMode::Strict, 0, limits()),
        Err(FunctionEnvelopeError::Code(CodeError::InvalidAtomIndex {
            offset,
            index: 243,
            ..
        })) if offset == input.len()
    ));
}

#[test]
fn bytecode_atom_mode_is_checked_before_any_function_byte_is_consumed() {
    let input = minimal_prefix(0, [0; 5], 0, None, None, &[41]);
    let mut cursor = WireCursor::new(&input, ReaderMode::Strict, WIRE_LIMITS).unwrap();
    assert_eq!(cursor.read_tag(), Ok(BcTag::FunctionBytecode));
    let body_offset = cursor.position();
    assert_eq!(
        read_function_record_prefix_after_tag(
            &mut cursor,
            AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap(),
            limits(),
        ),
        Err(FunctionEnvelopeError::InvalidAtomMode {
            found: BinaryObjectMode::Data,
        })
    );
    assert_eq!(cursor.position(), body_offset);
}

#[test]
fn arbitrary_short_inputs_fail_without_panics_or_cursor_overflow() {
    for length in 0..=96 {
        let bytes: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(37).wrapping_add(length as u8))
            .collect();
        for mode in [ReaderMode::Strict, ReaderMode::QuickJsCompatible] {
            let mut cursor = WireCursor::new(&bytes, mode, WIRE_LIMITS).unwrap();
            let _ = read_function_record_prefix_after_tag(&mut cursor, bytecode_space(0), limits());
            assert!(cursor.position() <= bytes.len());
        }
    }
}
