use super::*;

#[test]
fn runtime_preloads_quickjs_typeof_atoms_as_narrow_canonical_strings() {
    const TYPEOF: u8 = 0x95;

    let runtime = Runtime::new();
    let initial_atom_count = runtime.test_atom_count();
    let mut canonical_string = None;
    for spelling in crate::engine::vm::host_bridge::TYPEOF_STATIC_ATOMS {
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
