use super::*;

#[test]
fn quickjs_argument_slot_limit_uses_catchable_internal_error() {
    let parameters = std::iter::repeat_n("a", MAX_LOCAL_VARIABLES + 1)
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("(function({parameters}) {{}})");
    let error = compile_unlinked_script(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "too many arguments");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("argument overflow must materialize InternalError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("InternalError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("too many arguments"))
    );
}

#[test]
fn quickjs_call_argument_boundary_materializes_stack_overflow() {
    let arguments = std::iter::repeat_n("0", MAX_CALL_ARGUMENTS)
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("(function() {{}})({arguments})");
    let error = compile_unlinked_script(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "stack overflow");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("bytecode stack overflow must materialize InternalError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("InternalError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("stack overflow"))
    );

    let mut too_many = source.clone();
    let closing_parenthesis = too_many
        .rfind(')')
        .expect("generated call expression has a closing parenthesis");
    too_many.insert_str(closing_parenthesis, ",0");
    let error = compile_unlinked_script(&too_many).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "Too many call arguments");

    let unreachable = format!("(function(){{ return 1; {source}; }})");
    compile_unlinked_script(&unreachable).unwrap();

    // QuickJS checks the fixed-prefix count before noticing an ellipsis.
    // Therefore 65,535 fixed arguments followed by spread is a SyntaxError,
    // rather than entering the dynamic Array lowering.
    let mut fixed_then_spread = source;
    let closing_parenthesis = fixed_then_spread
        .rfind(')')
        .expect("generated call expression has a closing parenthesis");
    fixed_then_spread.insert_str(closing_parenthesis, ",...[]");
    let error = compile_unlinked_script(&fixed_then_spread).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "Too many call arguments");
}

#[test]
fn quickjs_template_stack_overflow_is_deferred_until_after_parsing() {
    // Each substitution is one concat argument; the kept receiver and
    // method let 65,532 arguments exactly reach JS_STACK_SIZE_MAX.
    let largest_valid = "${0}".repeat(MAX_BYTECODE_STACK - 2);
    let largest_valid = compile_unlinked_script(&format!("`{largest_valid}`")).unwrap();
    assert_eq!(
        largest_valid.metadata().max_stack,
        MAX_BYTECODE_STACK as u16
    );

    // One more argument exceeds the limit without passing through the
    // ordinary call parser's argument guard.
    let substitutions = "${0}".repeat(MAX_BYTECODE_STACK - 1);
    let source = format!("`{substitutions}`");
    let error = compile_unlinked_script(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "stack overflow");

    // QuickJS computes the bytecode stack only after parsing the whole
    // function, so a later reached lexical error has priority.
    let later_lexical_error = format!("{source}; \"unterminated");
    let error = compile_unlinked_script(&later_lexical_error).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "unexpected end of string");

    let later_parser_error = format!("{source} 0");
    let error = compile_unlinked_script(&later_parser_error).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "expecting ';'");

    // QuickJS computes stack depth over reachable bytecode PCs. The same
    // oversized call after a terminal return is encoded but ignored by
    // the control-flow walk.
    let unreachable = format!("(function(){{ return 1; {source}; }})");
    compile_unlinked_script(&unreachable).unwrap();

    // Once argc no longer fits u16, QuickJS encodes its low bits. A live
    // path has already crossed the stack cap before that call, while dead
    // bytecode remains valid and must not be diagnosed from the truncated
    // operand's residual stack effect.
    let wrapped_substitutions = "${0}".repeat(usize::from(u16::MAX) + 1);
    let wrapped = format!("`{wrapped_substitutions}`");
    let error = compile_unlinked_script(&wrapped).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "stack overflow");
    let unreachable = format!("(function(){{ return 1; {wrapped}; }})");
    compile_unlinked_script(&unreachable).unwrap();
}

#[test]
fn quickjs_closure_slot_limit_is_65534_and_uses_internal_error() {
    let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
    let mut function = FunctionIr::new(
        None,
        FunctionKind::Ordinary,
        FunctionSourceInfo {
            span,
            definition: SourceOffset::try_from_usize(0).unwrap(),
            range: None,
        },
        FunctionIrOptions {
            function_name: None,
            private_name_binding: false,
            class_constructor: false,
            derived_class_constructor: false,
            parameters: Vec::new(),
            defined_argument_count: 0,
            has_simple_parameter_list: true,
            rest_parameter: None,
            strict: false,
            super_capabilities: SuperCapabilities::NONE,
        },
    )
    .unwrap();
    function.closure_variables = (0..MAX_LOCAL_VARIABLES - 1)
        .map(|index| ClosureVariable {
            source: ClosureSource::ParentLocal(
                u16::try_from(index).expect("test index is below the QuickJS slot limit"),
            ),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        })
        .collect();

    assert_eq!(
        ensure_closure_variable(
            &mut function,
            ClosureVariable {
                source: ClosureSource::ParentArgument(0),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        )
        .unwrap(),
        65_533
    );
    let error = ensure_closure_variable(
        &mut function,
        ClosureVariable {
            source: ClosureSource::ParentArgument(1),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "too many closure variables");
}
