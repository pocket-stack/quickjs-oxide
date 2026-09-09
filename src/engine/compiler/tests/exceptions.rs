use super::*;

#[test]
fn compiles_throw_as_a_terminal_completion_and_enforces_no_line_terminator() {
    let bytecode = compile_script("throw 9").unwrap();
    assert!(bytecode.code.iter().any(|instruction| matches!(
        instruction,
        crate::engine::code::bytecode::Instruction::Throw
    )));
    assert!(compile_script("throw\n9").is_err());
}

#[test]
fn try_catch_lowering_keeps_parameter_and_body_scopes_distinct() {
    let source = r#"
        try { throw 1; }
        catch (e) { let x = e; function f(){ return e + x; } }
    "#;
    let tree = Parser::parse(source, JsString::from_static("<try-scope-test>")).unwrap();
    let root = &tree.functions[0];
    let catch_scope = root
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Catch)
        .map(super::ScopeId)
        .unwrap();
    let catch_body_scope = root
        .scopes
        .iter()
        .enumerate()
        .find(|(_, scope)| scope.kind == ScopeKind::Block && scope.parent == Some(catch_scope))
        .map(|(index, _)| super::ScopeId(index))
        .unwrap();
    let catch_binding = root.binding_in_scope(catch_scope, "e").unwrap();
    let BindingStorage::Local(catch_local) = catch_binding.storage else {
        panic!("catch parameter did not use local storage");
    };
    assert!(catch_binding.is_catch_parameter);
    assert_eq!(catch_binding.kind, BindingKind::Lexical { is_const: false });
    assert!(root.binding_in_scope(catch_body_scope, "x").is_some());
    assert!(root.binding_in_scope(catch_body_scope, "f").is_some());
    assert_eq!(
        tree.functions
            .iter()
            .find(|function| function.function_name.as_deref() == Some("f"))
            .and_then(|function| function.parent)
            .map(|parent| parent.definition_scope),
        Some(catch_body_scope)
    );

    let bytecode = compile_unlinked_script(source).unwrap();
    assert_eq!(
        bytecode
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Catch(_)))
            .count(),
        2
    );
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::SetLocalUninitialized(index) if *index == catch_local)
    ));
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::CloseLocal(index) if *index == catch_local)
    ));
    assert!(
        bytecode
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Ret))
    );
}

#[test]
fn catch_binding_conflicts_and_var_initializer_follow_quickjs() {
    for (source, keyword) in [("catch (e) {}", "catch"), ("finally {}", "finally")] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            format!("unexpected token in expression: '{keyword}'"),
            "{source}"
        );
    }
    let extra_catch = compile_unlinked_script("try {} finally {} catch (e) {}").unwrap_err();
    assert_eq!(
        extra_catch.message(),
        "unexpected token in expression: 'catch'"
    );

    for source in [
        "try {} catch (e) { let e; }",
        "try {} catch (e) { function e(){} }",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }
    compile_unlinked_script("try {} catch (e) { { let e = 1; e; } }").unwrap();

    let source = "try { throw 1; } catch (e) { var e = e + 1; e; }";
    let tree = Parser::parse(source, JsString::from_static("<catch-var-test>")).unwrap();
    let catch_local = tree.functions[0]
        .bindings
        .iter()
        .find(|binding| binding.is_catch_parameter)
        .and_then(|binding| match binding.storage {
            BindingStorage::Local(index) => Some(index),
            _ => None,
        })
        .unwrap();
    let bytecode = compile_unlinked_script(source).unwrap();
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::GetLocalCheck(index) if *index == catch_local)
    ));
    assert!(bytecode.code().iter().any(
        |instruction| matches!(instruction, Instruction::PutLocalCheck(index) if *index == catch_local)
    ));
    assert_eq!(evaluate_in_context(source), Value::Int(2));

    let strict_source = "\"use strict\"; try {} catch (eval) {}";
    let strict_error = compile_unlinked_script(strict_source).unwrap_err();
    assert_eq!(
        strict_error.message(),
        "invalid variable name in strict mode"
    );
    assert_eq!(
        strict_error.span().unwrap().start.column,
        u32::try_from(strict_source.find(')').unwrap() + 1).unwrap()
    );
}

#[test]
fn catch_binding_patterns_compile_with_ordinary_lexical_provenance() {
    for source in [
        "try { throw [1, 2]; } catch ([first, second]) { first + second; }",
        "try { throw { value: 42 }; } catch ({value}) { value; }",
        "try { throw { nested: [42] }; } catch ({nested: [value]}) { value; }",
        "try { throw [1, 2, 3]; } catch ([head, ...tail]) { head + tail.length; }",
        "try { throw { value: 1, extra: 2 }; } catch ({value, ...rest}) { value + rest.extra; }",
        "try { throw []; } catch ([value = function () {}]) { value.name; }",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("catch binding pattern did not compile: {source}: {error}")
        });
    }

    let source = concat!(
        "try { throw { value: 1, nested: [], extra: 2 }; } ",
        "catch ({value, nested: [fallback = 40], ...rest}) { ",
        "value + fallback + rest.extra; }",
    );
    let tree = Parser::parse(source, JsString::from_static("<catch-pattern-scope-test>"))
        .expect("nested catch binding pattern should parse");
    let root = &tree.functions[0];
    let catch_scope = root
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Catch)
        .map(ScopeId)
        .expect("catch pattern lost its scope");

    for name in ["value", "fallback", "rest"] {
        let binding = root
            .binding_in_scope(catch_scope, name)
            .unwrap_or_else(|| panic!("catch pattern lost binding {name}"));
        assert_eq!(binding.storage_scope, catch_scope, "{name}");
        assert_eq!(binding.declaration_scope, catch_scope, "{name}");
        assert_eq!(
            binding.kind,
            BindingKind::Lexical { is_const: false },
            "{name}"
        );
        assert!(
            !binding.is_catch_parameter,
            "pattern leaf {name} incorrectly received the simple-catch marker"
        );
    }

    let simple = Parser::parse(
        "try { throw 1; } catch (value) { value; }",
        JsString::from_static("<simple-catch-scope-test>"),
    )
    .expect("simple catch binding should parse");
    let simple_root = &simple.functions[0];
    let simple_scope = simple_root
        .scopes
        .iter()
        .position(|scope| scope.kind == ScopeKind::Catch)
        .map(ScopeId)
        .expect("simple catch binding lost its scope");
    assert!(
        simple_root
            .binding_in_scope(simple_scope, "value")
            .expect("simple catch binding was not registered")
            .is_catch_parameter,
        "only a simple catch binding receives the Annex-B marker"
    );

    let catch_scope_entry = root
        .ops
        .iter()
        .position(|operation| {
            matches!(operation.op, super::IrOp::EnterScope(scope) if scope == catch_scope)
        })
        .expect("catch scope has no EnterScope marker");
    let handler_target = root
        .ops
        .iter()
        .find_map(|operation| match &operation.op {
            super::IrOp::Bytecode(Instruction::Catch(target)) => {
                Some(usize::try_from(*target).expect("catch target fits usize"))
            }
            _ => None,
        })
        .expect("try statement has no Catch handler");
    assert!(
        catch_scope_entry < handler_target,
        "the exceptional handler must skip QuickJS's pre-label Catch EnterScope"
    );
    assert!(
        matches!(
            root.ops.get(handler_target).map(|operation| &operation.op),
            Some(super::IrOp::PrepareCatchScope(scope)) if *scope == catch_scope
        ),
        "the Catch target must land on its default-undefined preparation"
    );
}

#[test]
fn catch_binding_pattern_diagnostics_follow_quickjs() {
    for source in [
        "try {} catch ([value, value]) {}",
        "try {} catch ({first: value, second: value}) {}",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }

    for source in [
        "\"use strict\"; try {} catch ({eval}) {}",
        "\"use strict\"; try {} catch ([arguments]) {}",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "invalid destructuring target", "{source}");
    }
}

#[test]
fn direct_eval_profile_distinguishes_pattern_and_simple_catch_bindings() {
    let direct = |is_catch_parameter| {
        EvalCompileContext::direct_with_profile(
            false,
            vec![EvalRootBinding {
                name: JsString::from_static("value"),
                scope: 0,
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
                is_catch_parameter,
            }],
            EvalCallerProfile {
                scope_kinds: vec![EvalScopeKind::Catch].into_boxed_slice(),
                variable_target: EvalCallerVariableTarget::Global,
            },
            false,
            false,
        )
    };

    let read = compile_unlinked_eval_with_filename(
        "value",
        "<catch-pattern-eval>",
        DebugInfoMode::StripDebug,
        direct(false),
    )
    .expect("direct eval should read an imported catch-pattern lexical");
    let [descriptor] = read.closure_variables() else {
        panic!("catch-pattern eval did not retain exactly one imported binding");
    };
    assert_eq!(descriptor.source, ClosureSource::EvalEnvironment(0));
    assert!(descriptor.is_lexical);
    assert!(!descriptor.is_const);
    assert!(
        read.code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVarRefCheck(0))),
        "catch-pattern eval read lost its lexical TDZ check"
    );

    let pattern_var = compile_unlinked_eval_with_filename(
        "var value;",
        "<catch-pattern-eval>",
        DebugInfoMode::StripDebug,
        direct(false),
    )
    .expect("the caller lexical conflict is represented by entry bytecode");
    assert!(
        matches!(pattern_var.code(), [Instruction::ThrowRedeclaration(_), ..]),
        "a pattern catch lexical must reject sloppy eval var redeclaration"
    );

    let simple_var = compile_unlinked_eval_with_filename(
        "var value;",
        "<simple-catch-eval>",
        DebugInfoMode::StripDebug,
        direct(true),
    )
    .expect("a simple catch binding should retain QuickJS's var exception");
    assert!(
        !simple_var
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowRedeclaration(_))),
        "the simple-catch marker did not preserve the sloppy eval var exception"
    );
}

#[test]
fn nested_finally_abrupt_edges_use_typed_cleanup_and_shared_subroutines() {
    let source = r#"
        (function f(){
            outer: while (1) {
                try {
                    try { return 1; }
                    finally { break outer; }
                } finally { return 3; }
            }
            return 4;
        })()
    "#;
    let bytecode = compile_unlinked_script(source).unwrap();
    let function = bytecode
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .unwrap();
    assert_eq!(
        function
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Catch(_)))
            .count(),
        2
    );
    assert_eq!(
        function
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Ret))
            .count(),
        2
    );
    assert!(
        function
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::NipCatch))
            .count()
            >= 2
    );
    assert!(
        function
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::DropGosub, Instruction::Drop]))
    );
    assert_eq!(evaluate_in_context(source), Value::Int(3));
}

#[test]
fn script_finally_saves_and_normally_restores_eval_completion() {
    let source = "1; try { 2; } finally { 3; }";
    let bytecode = compile_unlinked_script(source).unwrap();
    assert_eq!(bytecode.local_definitions().len(), 2);
    assert!(
        bytecode
            .local_definitions()
            .iter()
            .all(|definition| definition.name.is_none() && !definition.is_lexical)
    );
    assert!(bytecode.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::GetLocal(0),
            Instruction::PutLocal(1),
            Instruction::Undefined,
            Instruction::PutLocal(0)
        ]
    )));
    assert!(bytecode.code().windows(3).any(|window| matches!(
        window,
        [
            Instruction::GetLocal(1),
            Instruction::PutLocal(0),
            Instruction::Ret
        ]
    )));
    assert_eq!(evaluate_in_context(source), Value::Int(2));
    assert_eq!(
        evaluate_in_context("try { throw 1; } catch { 4; } finally { 5; }"),
        Value::Int(4)
    );
}
