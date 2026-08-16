use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::graph::model::AtomId;
use super::super::pinned_atoms::{PinnedAtomId, PinnedAtomKind};
use super::super::wire::{
    BcTag, ReaderMode, ResourceKind, WireCursor, WireError, WireLimits, WireString, WireWriter,
};
use super::{ImageAtom, ImageAtomError, ImageAtomTable, ImageKey};

const TEST_LIMITS: WireLimits = WireLimits::new(4096, 32, 128, 512);

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
