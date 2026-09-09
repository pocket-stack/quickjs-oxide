use super::*;

#[test]
fn debug_metadata_tracks_operator_tail_call_and_root_call_sites() {
    let source = "(function outer(){ return (function inner(){ return 1n + 1; })(); })()";
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let root = context.compile_with_filename(source, "<cmdline>").unwrap();
    let outer = runtime.test_child_function_bytecode(&root, 0).unwrap();
    let inner = runtime.test_child_function_bytecode(&outer, 0).unwrap();

    let root_code = runtime.test_function_code(&root).unwrap();
    let outer_code = runtime.test_function_code(&outer).unwrap();
    let inner_code = runtime.test_function_code(&inner).unwrap();
    let root_call = root_code
        .iter()
        .rposition(|instruction| matches!(instruction, Instruction::Call(0)))
        .unwrap();
    let outer_call = outer_code
        .iter()
        .rposition(|instruction| matches!(instruction, Instruction::Call(0)))
        .unwrap();
    let inner_add = inner_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Add))
        .unwrap();

    assert_eq!(
        runtime
            .test_function_debug_location(&inner, Some(inner_add))
            .unwrap(),
        Some((
            JsString::from_static("<cmdline>"),
            crate::source::LineColumn::new(0, 55)
        ))
    );
    assert_eq!(
        runtime
            .test_function_debug_location(&outer, Some(outer_call))
            .unwrap(),
        Some((
            JsString::from_static("<cmdline>"),
            crate::source::LineColumn::new(0, 19)
        ))
    );
    assert_eq!(
        runtime
            .test_function_debug_location(&root, Some(root_call))
            .unwrap(),
        Some((
            JsString::from_static("<cmdline>"),
            crate::source::LineColumn::new(0, 68)
        ))
    );
    assert_eq!(runtime.test_function_debug_source(&root).unwrap(), None);
    assert_eq!(
        runtime.test_function_debug_source(&outer).unwrap(),
        Some(b"function outer(){ return (function inner(){ return 1n + 1; })(); }".to_vec())
    );
    assert_eq!(
        runtime.test_function_debug_source(&inner).unwrap(),
        Some(b"function inner(){ return 1n + 1; }".to_vec())
    );
}

#[test]
fn ordinary_assignment_inherits_last_rhs_marker_and_var_initializer_marks_equal() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let assignment_source = "\"use strict\"; missing = 1";
    let root = context
        .compile_with_filename(assignment_source, "globals.js")
        .unwrap();
    let code = runtime.test_function_code(&root).unwrap();
    let dup = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Dup))
        .unwrap();
    assert!(matches!(code.get(dup + 1), Some(Instruction::PutVar(_))));
    let lhs = u32::try_from(assignment_source.find("missing").unwrap()).unwrap();
    let expected = Some((
        JsString::from_static("globals.js"),
        crate::source::LineColumn::new(0, lhs),
    ));
    assert_eq!(
        runtime
            .test_function_debug_location(&root, Some(dup))
            .unwrap(),
        expected
    );
    assert_eq!(
        runtime
            .test_function_debug_location(&root, Some(dup + 1))
            .unwrap(),
        expected
    );

    let identifier_rhs_source = "(function(){ \"use strict\"; var y=1; missing = y; })";
    let identifier_rhs_root = context
        .compile_with_filename(identifier_rhs_source, "globals.js")
        .unwrap();
    let identifier_rhs = runtime
        .test_child_function_bytecode(&identifier_rhs_root, 0)
        .unwrap();
    let identifier_rhs_code = runtime.test_function_code(&identifier_rhs).unwrap();
    let identifier_put = identifier_rhs_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::PutVar(_)))
        .unwrap();
    let rhs_identifier = u32::try_from(identifier_rhs_source.rfind("y;").unwrap()).unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&identifier_rhs, Some(identifier_put))
            .unwrap(),
        Some((
            JsString::from_static("globals.js"),
            crate::source::LineColumn::new(0, rhs_identifier)
        ))
    );

    let operator_rhs_source = "\"use strict\"; missing = 1 + 2";
    let operator_rhs = context
        .compile_with_filename(operator_rhs_source, "globals.js")
        .unwrap();
    let operator_rhs_code = runtime.test_function_code(&operator_rhs).unwrap();
    let operator_put = operator_rhs_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::PutVar(_)))
        .unwrap();
    let plus = u32::try_from(operator_rhs_source.find('+').unwrap()).unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&operator_rhs, Some(operator_put))
            .unwrap(),
        Some((
            JsString::from_static("globals.js"),
            crate::source::LineColumn::new(0, plus)
        ))
    );

    let declaration_source = "(function(){ var x = 1; return x; })";
    let declaration_root = context
        .compile_with_filename(declaration_source, "globals.js")
        .unwrap();
    let declaration = runtime
        .test_child_function_bytecode(&declaration_root, 0)
        .unwrap();
    let declaration_code = runtime.test_function_code(&declaration).unwrap();
    let put_local = declaration_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::PutLocal(_)))
        .unwrap();
    let equal = u32::try_from(declaration_source.find("= 1").unwrap()).unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&declaration, Some(put_local))
            .unwrap(),
        Some((
            JsString::from_static("globals.js"),
            crate::source::LineColumn::new(0, equal)
        ))
    );
}

#[test]
fn call_and_construct_debug_sites_follow_quickjs_tokens() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let call_source = "Error()";
    let call_root = context
        .compile_with_filename(call_source, "calls.js")
        .unwrap();
    let call_code = runtime.test_function_code(&call_root).unwrap();
    let call_pc = call_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Call(0)))
        .unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&call_root, Some(call_pc))
            .unwrap(),
        Some((
            JsString::from_static("calls.js"),
            crate::source::LineColumn::new(0, 5)
        ))
    );

    let construct_source = "(function f(){ return new Error('x'); })";
    let construct_root = context
        .compile_with_filename(construct_source, "construct.js")
        .unwrap();
    let constructor = runtime
        .test_child_function_bytecode(&construct_root, 0)
        .unwrap();
    let constructor_code = runtime.test_function_code(&constructor).unwrap();
    let construct_pc = constructor_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Construct(1)))
        .unwrap();
    let left_paren = construct_source.find("Error(").unwrap() + "Error".len();
    assert_eq!(
        runtime
            .test_function_debug_location(&constructor, Some(construct_pc))
            .unwrap(),
        Some((
            JsString::from_static("construct.js"),
            crate::source::LineColumn::new(0, u32::try_from(left_paren).unwrap())
        ))
    );

    let no_parens_source = "(function f(){ return new Error; })";
    let no_parens_root = context
        .compile_with_filename(no_parens_source, "construct.js")
        .unwrap();
    let no_parens_constructor = runtime
        .test_child_function_bytecode(&no_parens_root, 0)
        .unwrap();
    let no_parens_code = runtime.test_function_code(&no_parens_constructor).unwrap();
    let no_parens_pc = no_parens_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Construct(0)))
        .unwrap();
    let semicolon = no_parens_source.find("Error;").unwrap() + "Error".len();
    assert_eq!(
        runtime
            .test_function_debug_location(&no_parens_constructor, Some(no_parens_pc))
            .unwrap(),
        Some((
            JsString::from_static("construct.js"),
            crate::source::LineColumn::new(0, u32::try_from(semicolon).unwrap())
        ))
    );
}

#[test]
fn primitive_and_function_primaries_do_not_emit_source_markers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(1)",
        "('x')",
        "(null)",
        "(true)",
        "(this)",
        "(function(){})",
        "(!1)",
        "(void 1)",
        "(typeof 1)",
        "(true && false)",
        "(1 ? 2 : 3)",
        "(1, 2)",
    ] {
        let root = context.compile_with_filename(source, "primary.js").unwrap();
        for pc in 0..runtime.test_function_code(&root).unwrap().len() {
            assert_eq!(
                runtime
                    .test_function_debug_location(&root, Some(pc))
                    .unwrap(),
                Some((
                    JsString::from_static("primary.js"),
                    crate::source::LineColumn::new(0, 0)
                )),
                "source: {source}, pc: {pc}"
            );
        }
    }
}

#[test]
fn root_and_ordinary_function_names_stay_distinct() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let root = context.compile("(function(){ return 1; })").unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();

    assert_eq!(
        runtime.test_function_name(&root).unwrap(),
        Some(JsString::from_static("<eval>"))
    );
    assert_eq!(runtime.test_function_name(&child).unwrap(), None);
    assert_eq!(
        runtime.test_function_debug_location(&root, None).unwrap(),
        Some((
            JsString::from_static(super::DEFAULT_EVAL_FILENAME),
            crate::source::LineColumn::new(0, 0)
        ))
    );
}

#[test]
fn filename_atom_ownership_counts_every_function_and_same_atom_use() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let root = context
        .compile_with_filename("(function(){ return same; })", "same")
        .unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();

    assert_eq!(
        runtime.test_debug_filename_atom_ownership(&root).unwrap(),
        Some((2, Some(4)))
    );
    assert_eq!(
        runtime.test_debug_filename_atom_ownership(&child).unwrap(),
        Some((2, Some(4)))
    );
    drop(root);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 1);
    assert_eq!(
        runtime.test_function_debug_source(&child).unwrap(),
        Some(b"function(){ return same; }".to_vec())
    );
    drop(child);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn runtime_strip_mode_controls_debug_payload_and_filename_atom_ownership() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let root = context
        .compile_with_filename("(function(){})", "strip-source-unique.js")
        .unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();
    assert_eq!(
        runtime.test_function_debug_location(&child, None).unwrap(),
        Some((
            JsString::from_static("strip-source-unique.js"),
            crate::source::LineColumn::new(0, 1),
        ))
    );
    assert_eq!(runtime.test_function_debug_source(&child).unwrap(), None);
    assert!(
        runtime
            .test_debug_filename_atom_ownership(&child)
            .unwrap()
            .is_some()
    );
    drop(root);
    drop(child);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let root = context
        .compile_with_filename("(function(){})", "strip-debug-unique.js")
        .unwrap();
    let child = runtime.test_child_function_bytecode(&root, 0).unwrap();
    assert_eq!(
        runtime.test_function_debug_location(&child, None).unwrap(),
        None
    );
    assert_eq!(runtime.test_function_debug_source(&child).unwrap(), None);
    assert_eq!(
        runtime.test_debug_filename_atom_ownership(&child).unwrap(),
        None
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    drop(root);
    drop(child);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}
