use super::*;

#[test]
fn decodes_the_exact_quickjs_42_function_as_non_executable_image() {
    let vector = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    let image = decode_image(&vector).unwrap();

    assert_eq!(image.input_atom_slot_count(), 0);
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
fn authenticated_native_plan_preserves_atom_semantics_and_input_provenance() {
    let manifest_record = atom_relocation_record_with_raw_atom(ordinary_pinned("length").raw());
    let manifest_image = decode_image(&header_bytes(&[], &manifest_record)).unwrap();
    let manifest = first_native_atom(&manifest_image);
    assert_eq!(manifest.class(), NativeAtomClass::String);
    assert!(!manifest.originates_from_input_atom_table());
    assert_eq!(manifest.manifest_string(), Some("length"));
    assert_eq!(manifest.index(), None);
    assert_eq!(manifest.dynamic_string(), None);

    let decimal_record = atom_relocation_record_with_raw_atom(ATOM_TAG_INT | 42);
    let decimal_image = decode_image(&header_bytes(&[], &decimal_record)).unwrap();
    let decimal = first_native_atom(&decimal_image);
    assert_eq!(decimal.class(), NativeAtomClass::Index);
    assert!(!decimal.originates_from_input_atom_table());
    assert_eq!(decimal.manifest_string(), None);
    assert_eq!(decimal.index(), Some(42));
    assert_eq!(decimal.dynamic_string(), None);

    let dynamic_spelling = wide(&[0x0100, 0xd800, 0x0000]);
    let dynamic_record = atom_relocation_record_with_raw_atom(FIRST_DYNAMIC_ATOM);
    let dynamic_image =
        decode_image(&header_bytes(&[dynamic_spelling.clone()], &dynamic_record)).unwrap();
    let dynamic = first_native_atom(&dynamic_image);
    assert_eq!(dynamic.class(), NativeAtomClass::String);
    assert!(dynamic.originates_from_input_atom_table());
    assert_eq!(dynamic.manifest_string(), None);
    assert_eq!(dynamic.index(), None);
    assert_eq!(dynamic.dynamic_string(), Some(&dynamic_spelling));
    assert!(std::ptr::eq(
        dynamic.dynamic_string().unwrap(),
        &dynamic_image.atoms()[0],
    ));

    // A unique header slot can semantically alias a release-manifest String
    // or a tagged decimal after QuickJS-style atom interning. Its raw operand
    // still authenticates that slot as the relocation's origin.
    let manifest_alias_image = decode_image(&header_bytes(
        &[narrow(b"length")],
        &atom_relocation_record_with_raw_atom(FIRST_DYNAMIC_ATOM),
    ))
    .unwrap();
    let manifest_alias = first_native_atom(&manifest_alias_image);
    assert_eq!(manifest_alias.class(), NativeAtomClass::String);
    assert!(manifest_alias.originates_from_input_atom_table());
    assert_eq!(manifest_alias.manifest_string(), Some("length"));
    assert_eq!(manifest_alias.index(), None);
    assert_eq!(manifest_alias.dynamic_string(), None);

    let decimal_alias_image = decode_image(&header_bytes(
        &[narrow(b"42")],
        &atom_relocation_record_with_raw_atom(FIRST_DYNAMIC_ATOM),
    ))
    .unwrap();
    let decimal_alias = first_native_atom(&decimal_alias_image);
    assert_eq!(decimal_alias.class(), NativeAtomClass::Index);
    assert!(decimal_alias.originates_from_input_atom_table());
    assert_eq!(decimal_alias.manifest_string(), None);
    assert_eq!(decimal_alias.index(), Some(42));
    assert_eq!(decimal_alias.dynamic_string(), None);
}

#[test]
fn authenticated_native_plan_distinguishes_every_non_string_atom_class() {
    let private = (1..=242)
        .filter_map(PinnedAtomId::from_raw)
        .find(|atom| atom.kind() == PinnedAtomKind::Private)
        .expect("pinned manifest must contain its private atom");
    let symbol = (1..=242)
        .filter_map(PinnedAtomId::from_raw)
        .find(|atom| atom.kind() == PinnedAtomKind::Symbol)
        .expect("pinned manifest must contain symbol atoms");

    for (raw_atom, expected) in [
        (0, NativeAtomClass::Null),
        (private.raw(), NativeAtomClass::Private),
        (symbol.raw(), NativeAtomClass::Symbol),
    ] {
        let record = atom_relocation_record_with_raw_atom(raw_atom);
        let image = decode_image(&header_bytes(&[], &record)).unwrap();
        let atom = first_native_atom(&image);
        assert_eq!(atom.class(), expected);
        assert!(!atom.originates_from_input_atom_table());
        assert_eq!(atom.index(), None);
        assert_eq!(atom.manifest_string(), None);
        assert_eq!(atom.dynamic_string(), None);
    }
}

#[test]
fn decoded_image_retains_non_dynamic_input_atom_slots() {
    // One unused empty-string header atom precedes the exact QuickJS 42
    // function. The slot remaps to a predefined atom, so it intentionally
    // contributes no image-local dynamic string.
    let vector = bytes("0501000c000200a80100010001000000040100000000bb2acb28");
    let image = decode_image(&vector).unwrap();

    assert_eq!(image.input_atom_slot_count(), 1);
    assert!(image.atoms().is_empty());
}
