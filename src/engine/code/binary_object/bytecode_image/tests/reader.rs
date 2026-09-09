use super::*;

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
