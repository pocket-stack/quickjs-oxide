use super::*;

#[test]
fn array_literals_lower_dense_fixed_hole_and_spread_phases() {
    let dense = compile_unlinked_script("[1,2,3]").unwrap();
    assert!(
        dense
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ArrayFrom(3)))
    );
    assert!(!dense.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineField(_) | Instruction::DefineArrayEl | Instruction::Append
    )));

    let large_source = format!(
        "[{}]",
        (0..33)
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let large = compile_unlinked_script(&large_source).unwrap();
    assert!(
        large
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ArrayFrom(32)))
    );
    let fixed_key = large
        .code()
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::DefineField(index) => Some(*index),
            _ => None,
        })
        .expect("33rd Array element must use DefineField");
    assert!(matches!(
        large.constants()[usize::try_from(fixed_key).unwrap()].as_primitive(),
        Some(crate::engine::value::PrimitiveValue::String(value)) if value == &JsString::from_static("32")
    ));

    let holes = compile_unlinked_script("[,1,,]").unwrap();
    let hole_code = holes.code();
    assert!(
        hole_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ArrayFrom(0)))
    );
    assert!(
        hole_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
    );
    assert!(hole_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Dup,
            Instruction::PushI32(3),
            Instruction::PutField(_)
        ]
    )));

    let spread = compile_unlinked_script("[1,...'ab',,4]").unwrap();
    assert!(
        spread
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Append))
    );
    assert!(spread.code().windows(3).any(|window| matches!(
        window,
        [
            Instruction::DefineArrayEl,
            Instruction::Inc,
            Instruction::Drop
        ]
    )));
}

#[test]
fn array_literal_grammar_keeps_quickjs_boundaries_and_reference_state() {
    for source in [
        "[]",
        "[,]",
        "[,,]",
        "[1,]",
        "[...'',]",
        "for([1 in Function];false;);",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid Array literal {source:?}: {error}"));
    }
    assert_eq!(
        compile_unlinked_script("[1 2]").unwrap_err().message(),
        "expecting ']'"
    );
    compile_unlinked_script("[/a/]").unwrap();
    for source in ["[... ]", "[1,, 2 3]"] {
        assert!(
            compile_unlinked_script(source).is_err(),
            "invalid Array literal unexpectedly compiled: {source}"
        );
    }

    let named = compile_unlinked_script("var named=[function(){}]").unwrap();
    assert!(
        !named
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetName(_)))
    );
    let child = named
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("Array element function child");
    assert_eq!(child.func_name(), None);
}
