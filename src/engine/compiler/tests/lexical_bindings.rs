use super::*;

#[test]
fn ordinary_function_body_lexicals_execute_local_capture_and_constructor_paths() {
    assert_eq!(
        evaluate_in_context(
            "(function(){let x=1,y=x+1,z;return x*10+y+(typeof z==='undefined'?0:100)})()"
        ),
        Value::Int(12)
    );
    assert_eq!(
        evaluate_in_context("(function(){let arguments=3;let eval=4;return arguments+eval})()"),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context("(function(){var read=function(){return x};let x=7;return read()})()"),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){let x=function(){return x};return x()===x&&x.name==='x'})()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){let x=1;var next=function(){x+=1;return x};return next()*10+next()})()"
        ),
        Value::Int(23)
    );
    assert_eq!(
        evaluate_in_context("(function(){const x=4;return function(){return x}()})()"),
        Value::Int(4)
    );
    assert_eq!(
        evaluate_in_context("Function('let x=1;const y=2;return x+y')()"),
        Value::Int(3)
    );
    assert_eq!(
        evaluate_in_context("(function(){var result=delete x;let x=1;return result})()"),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context("(function(){const x=0;return x&&=missing})()"),
        Value::Int(0)
    );
}

#[test]
fn lexical_lowering_publishes_tdz_vardefs_and_checked_capture_relays() {
    let script =
        compile_unlinked_script("(function(){let x=1;const y=2;return function(){x+=y;return x}})")
            .unwrap();
    let outer = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its outer function");
    assert!(matches!(
        outer.code(),
        [
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(1),
            ..
        ]
    ));
    assert_eq!(outer.local_definitions().len(), 2);
    assert_eq!(
        outer.local_definitions()[0].name.as_ref(),
        Some(&JsString::from_static("x"))
    );
    assert!(outer.local_definitions()[0].is_lexical);
    assert!(!outer.local_definitions()[0].is_const);
    assert_eq!(
        outer.local_definitions()[1].name.as_ref(),
        Some(&JsString::from_static("y"))
    );
    assert!(outer.local_definitions()[1].is_lexical);
    assert!(outer.local_definitions()[1].is_const);

    let inner = outer
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("outer function lost its captured child");
    assert_eq!(inner.closure_variables().len(), 2);
    for (index, expected_name, is_const) in [(0, "x", false), (1, "y", true)] {
        let descriptor = inner.closure_variables()[index];
        assert_eq!(
            descriptor.source,
            ClosureSource::ParentLocal(u16::try_from(index).unwrap())
        );
        assert!(descriptor.is_lexical);
        assert_eq!(descriptor.is_const, is_const);
        assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
        let ClosureVariableName::Constant(name) = descriptor.name else {
            panic!("lexical descriptor lost its source name");
        };
        assert_eq!(
            inner.constants()[usize::try_from(name).unwrap()].as_primitive(),
            Some(&crate::engine::value::PrimitiveValue::String(
                JsString::from_static(expected_name)
            ))
        );
    }
    assert!(inner.code().windows(5).any(|window| matches!(
        window,
        [
            Instruction::GetVarRefCheck(0),
            Instruction::GetVarRefCheck(1),
            Instruction::Add,
            Instruction::Dup,
            Instruction::PutVarRefCheck(0),
        ]
    )));
    assert!(inner.code().windows(2).any(|window| matches!(
        window,
        [Instruction::GetVarRefCheck(0), Instruction::Return]
    )));
}

#[test]
fn nested_block_and_switch_lexicals_lower_scope_lifetimes() {
    let script = compile_unlinked_script(
        "(function(){var read;{read=function(){return ++value};let value=40;}return read()*100+read();})()",
    )
    .unwrap();
    let outer = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its block function");
    assert!(
        outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocalUninitialized(1)))
    );
    assert!(
        outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CloseLocal(1)))
    );
    let child = outer
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("block function lost its captured child");
    assert_eq!(
        child.closure_variables()[0].source,
        ClosureSource::ParentLocal(1)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var read;{read=function(){return ++value};let value=40;}return read()*100+read();})()"
        ),
        Value::Int(4142)
    );

    let switch_script = compile_unlinked_script(
        "(function(){var read;switch(0){case 0:let value=40;read=function(){return ++value};break;}return read()*100+read();})()",
    )
    .unwrap();
    let switch_function = switch_script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its switch function");
    assert!(switch_function.code().windows(2).any(|window| matches!(
        window,
        [
            Instruction::PushI32(0),
            Instruction::SetLocalUninitialized(1)
        ]
    )));
    assert!(
        switch_function
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::Drop, Instruction::CloseLocal(1)]))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var read;switch(0){case 0:let value=40;read=function(){return ++value};break;}return read()*100+read();})()"
        ),
        Value::Int(4142)
    );
    assert_eq!(evaluate_in_context("{let value=42;value;}"), Value::Int(42));
    assert_eq!(
        evaluate_in_context("switch(0){case 0:const value=42;value;}"),
        Value::Int(42)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval("(function(){{let value=40;throw function(){return ++value};}})()"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(thrown) = context.take_exception().unwrap().unwrap() else {
        panic!("nested lexical throw did not preserve the escaped closure");
    };
    let callable = runtime.as_callable(&thrown).unwrap().unwrap();
    assert_eq!(
        context.call(&callable, Value::Undefined, &[]).unwrap(),
        Value::Int(41)
    );
    assert_eq!(
        context.call(&callable, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn nested_lexical_cleanup_shadowing_and_quickjs_var_quirks_execute() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,index=0;while(index<2){{let value=index++;if(index===1){first=function(){return ++value};continue;}second=function(){return ++value};}}return first()*100+second()*10+first()+second();})()"
        ),
        Value::Int(125)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var first,second,index=0;outer:while(index<2){switch(0){case 0:let value=index++;if(index===1){first=function(){return ++value};continue outer;}second=function(){return ++value};break outer;}}return first()*100+second()*10+first()+second();})()"
        ),
        Value::Int(125)
    );
    assert_eq!(
        evaluate_in_context(
            "(function self(parameter){var outer='O',result;{let parameter='P',outer='B',self='S';result=parameter+outer+self;}return result+'|'+parameter+'|'+outer+'|'+typeof self;})('p')"
        ),
        Value::String(JsString::from_static("PBS|p|O|function"))
    );
    for source in [
        "(function(){var value;{var value;let value;}return 1})()",
        "(function(value){{var value;let value;}return 1})(0)",
    ] {
        assert_eq!(evaluate_in_context(source), Value::Int(1), "{source}");
    }
    for (source, message) in [
        (
            "(function(){var value;{let value;var value;}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){{var value;}let value;})",
            "invalid redefinition of a variable",
        ),
        (
            "(function(){let value;{var value;}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){switch(0){case 0:let value;case 1:const value=1;}})",
            "invalid redefinition of lexical identifier",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }
}

#[test]
fn lexical_tdz_and_readonly_errors_follow_checked_local_and_capture_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(function(){return x;let x=1})()",
        "(function(){return typeof x;let x=1})()",
        "(function(){x=1;let x})()",
        "(function(){return function(){return x};let x=1})()()",
        "(function(){var set=function(){x=1};set();let x})()",
        "(function(){var add=function(){x+=missing};add();const x=1})()",
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("ReferenceError"),
                JsString::from_static("x is not initialized")
            ),
            "{source}"
        );
    }
    for source in [
        "(function(){const x=1;x=2})()",
        "(function(){const x=1;return function(){x=2}})()()",
        "(function(){var set=function(){x=1};set();const x=2})()",
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("'x' is read-only")
            ),
            "{source}"
        );
    }
}

#[test]
fn lexical_parser_matches_redefinition_priority_contextual_let_and_boundaries() {
    let syntax_cases = [
        (
            "(function(){\nlet x;\nlet x;\n})",
            "invalid redefinition of lexical identifier",
            3,
            6,
        ),
        (
            "(function(){\nvar x;\nlet x;\n})",
            "invalid redefinition of a variable",
            3,
            6,
        ),
        (
            "(function(){\nlet x;\nvar x;\n})",
            "invalid redefinition of lexical identifier",
            3,
            6,
        ),
        (
            "(function(x){\nlet x;\n})",
            "invalid redefinition of parameter name",
            2,
            6,
        ),
        (
            "(function(){\nconst x;\n})",
            "missing initializer for const variable",
            2,
            8,
        ),
    ];
    for (source, message, line, column) in syntax_cases {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error.span().expect("syntax error lost its source span");
        assert_eq!(
            (span.start.line, span.start.column),
            (line, column),
            "{source}"
        );
    }
    for (source, message) in [
        (
            "(function(){let let=1})",
            "'let' is not a valid lexical identifier",
        ),
        (
            "(function(){'use strict';let eval=1})",
            "invalid variable name in strict mode",
        ),
        (
            "(function(){if(true) let x=1})",
            "lexical declarations can't appear in single-statement context",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message
        );
    }

    assert_eq!(
        evaluate_in_context("(function(){var let=0;let=2;return let})()"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate_in_context("(function(){var x=0;if(false) let\nx=1;return x})()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function named(){let named=3;return named})()"),
        Value::Int(3)
    );

    assert_eq!(
        evaluate_in_context("(function(){let [[item]]=[[1]];return item})()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function(){{let [[item]=[2]]=[];return item}})()"),
        Value::Int(2)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){switch(0){case 0:let [...[first,second]]=[3,4];return first+second}})()"
        ),
        Value::Int(7)
    );
    assert_eq!(
        evaluate_in_context("(function(){{let nested=1;return nested}})()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function(){switch(0){case 0:let inCase=1;return inCase}})()"),
        Value::Int(1)
    );
}

#[test]
fn strip_debug_removes_lexical_tdz_names_but_not_readonly_atoms() {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let mut context = runtime.new_context();
    for source in [
        "(function(){return localName;let localName=1})()",
        "(function(){return function probe(){return capturedName};let capturedName=1})()()",
        "(function(){var read;outer:{read=function(){return blockName};break outer;let blockName=1;}return read();})()",
        "(function(){var read;switch(1){case 0:let switchName=1;case 1:read=function(){return switchName};break;}return read();})()",
    ] {
        assert_eq!(
            evaluate_error(&runtime, &mut context, source),
            (
                JsString::from_static("ReferenceError"),
                JsString::from_static("lexical variable is not initialized")
            )
        );
    }
    assert_eq!(
        evaluate_error(
            &runtime,
            &mut context,
            "(function(){var write;{const retainedName=1;write=function(){retainedName=2};}write();})()"
        ),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("'retainedName' is read-only")
        )
    );
}
