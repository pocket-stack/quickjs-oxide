use super::*;

#[test]
fn compiles_bigint_literals_without_a_fixed_width_limit() {
    let expected = JsBigInt::parse_radix("10000000000000000000000000000000000000000", 16).unwrap();
    assert_eq!(
        evaluate("0x10000000000000000000000000000000000000000n"),
        Value::BigInt(expected)
    );
}

#[test]
fn use_strict_directive_rejects_legacy_literals() {
    assert!(compile_script("'use strict'; 010").is_err());
    assert!(compile_script("'use strict'; '\\1'").is_err());
    assert!(compile_script("'\\1'; 'use strict'; 0").is_err());
    assert!(compile_script("'\\8'; 'use strict'; 0").is_err());
    assert!(compile_script("; 'use strict'; 010").is_ok());
    assert!(compile_script("'use\\x20strict'; 010").is_ok());
    assert!(compile_script("'not strict'\n'use strict'\n010").is_err());
    assert!(compile_script("'not strict'\n+ 'use strict'; 010").is_ok());
    assert!(compile_script("'use strict' + ''; 010").is_ok());
    assert!(compile_script("'use strict'\n!0; 010").is_ok());
    assert!(compile_script("'use strict'\nvoid 0; 010").is_ok());
}

#[test]
fn unlinked_script_preserves_strict_mode_metadata() {
    let strict = compile_unlinked_script("'use strict'; 0").unwrap();
    let sloppy = compile_unlinked_script("'use\\x20strict'; 0").unwrap();

    assert!(strict.metadata().strict);
    assert!(!sloppy.metadata().strict);
}

#[test]
fn unlinked_script_preserves_verified_maximum_stack() {
    let source = "'left' + (0.5 * 2.5)";
    let bytecode = compile_script(source).unwrap();
    let verified = bytecode.verify().unwrap();
    let unlinked = compile_unlinked_script(source).unwrap();

    assert_eq!(bytecode.max_stack, 3);
    assert_eq!(unlinked.metadata().max_stack, bytecode.max_stack);
    assert_eq!(unlinked.metadata().max_stack, verified.max_stack);
}

#[test]
fn unlinked_script_converts_every_compiled_constant_to_a_primitive() {
    let source = "'\\ud800x'; 3.5; 0x100000000000000000000000000000000n";
    let bytecode = compile_script(source).unwrap();
    let unlinked = compile_unlinked_script(source).unwrap();

    assert_eq!(bytecode.constants.len(), 3);
    assert_eq!(unlinked.constants().len(), bytecode.constants.len());
    for (constant, expected) in unlinked.constants().iter().zip(&bytecode.constants) {
        assert_eq!(constant.as_primitive(), Some(expected));
        assert!(constant.as_child().is_none());
    }
}

#[test]
fn detached_string_literals_follow_quickjs_atom_identity_boundaries() {
    let atomized = compile_script("['same','same']").unwrap();
    let crate::engine::value::PrimitiveValue::String(first) = &atomized.constants[0] else {
        panic!("first atomized literal was not a String");
    };
    let crate::engine::value::PrimitiveValue::String(second) = &atomized.constants[1] else {
        panic!("second atomized literal was not a String");
    };
    assert!(first.same_representation(second));

    let immediate = compile_script("['2147483647','2147483647']").unwrap();
    let crate::engine::value::PrimitiveValue::String(first) = &immediate.constants[0] else {
        panic!("first immediate-atom literal was not a String");
    };
    let crate::engine::value::PrimitiveValue::String(second) = &immediate.constants[1] else {
        panic!("second immediate-atom literal was not a String");
    };
    assert!(!first.same_representation(second));

    let table_backed = compile_script("['2147483648','2147483648']").unwrap();
    let crate::engine::value::PrimitiveValue::String(first) = &table_backed.constants[0] else {
        panic!("first table-backed numeric literal was not a String");
    };
    let crate::engine::value::PrimitiveValue::String(second) = &table_backed.constants[1] else {
        panic!("second table-backed numeric literal was not a String");
    };
    assert!(first.same_representation(second));
}
