use super::*;

#[test]
fn compiler_marks_only_syntactic_eval_identifier_calls() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for source in ["eval(0)", "(eval)(0)", "((eval))(0)", r"\u0065val(0)"] {
        let root = context.compile(source).unwrap();
        let code = runtime.test_function_code(&root).unwrap();
        assert_eq!(
            code.iter()
                .filter(|instruction| {
                    matches!(
                        instruction,
                        Instruction::Eval {
                            argument_count: 1,
                            ..
                        }
                    )
                })
                .count(),
            1,
            "direct source: {source}"
        );
    }

    let local = context
        .compile("(function(eval){ return eval(0); })")
        .unwrap();
    let local = runtime.test_child_function_bytecode(&local, 0).unwrap();
    assert!(
        runtime
            .test_function_code(&local)
            .unwrap()
            .iter()
            .any(|instruction| {
                matches!(
                    instruction,
                    Instruction::Eval {
                        argument_count: 1,
                        ..
                    }
                )
            })
    );

    for source in [
        "(0, eval)(0)",
        "var alias = eval; alias(0)",
        "(true ? eval : eval)(0)",
        "(eval = Function)(0)",
        "globalThis.eval(0)",
        "eval.call(undefined, 0)",
        "new eval(0)",
    ] {
        let root = context.compile(source).unwrap();
        let code = runtime.test_function_code(&root).unwrap();
        assert!(
            !code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Eval { .. })),
            "indirect/non-call source: {source}"
        );
    }
}

#[test]
fn compiler_emits_independent_eval_root_and_external_relays() {
    let bindings = vec![
        EvalRootBinding {
            name: JsString::from_static("inner"),
            scope: 0,
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
        EvalRootBinding {
            name: JsString::from_static("outer"),
            scope: 1,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
    ];

    for debug_info in [
        DebugInfoMode::Full,
        DebugInfoMode::StripSource,
        DebugInfoMode::StripDebug,
    ] {
        let root = compile_unlinked_eval_with_filename(
            "(function () { return inner + outer; })",
            "<eval>",
            debug_info,
            EvalCompileContext::direct(true, bindings.clone()),
        )
        .unwrap();
        assert_eq!(root.metadata().eval_kind, EvalKind::Direct);
        assert!(root.metadata().strict);
        assert!(!root.metadata().has_prototype);
        assert_eq!(root.metadata().constructor_kind, ConstructorKind::None);
        assert_eq!(root.closure_variables().len(), bindings.len());
        for (index, descriptor) in root.closure_variables().iter().enumerate() {
            assert_eq!(
                descriptor.source,
                ClosureSource::EvalEnvironment(u16::try_from(index).unwrap()),
            );
            let ClosureVariableName::Constant(name) = descriptor.name else {
                panic!("eval root lost its external binding name in {debug_info:?}");
            };
            assert!(matches!(
                root.constants()[name as usize].as_primitive(),
                Some(crate::engine::value::PrimitiveValue::String(found)) if found == &bindings[index].name
            ));
        }

        let child = root
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("eval function expression did not produce child bytecode");
        assert_eq!(child.metadata().eval_kind, EvalKind::None);
        assert_eq!(child.closure_variables().len(), 2);
        assert_eq!(
            child.closure_variables()[0].source,
            ClosureSource::ParentClosure(0),
        );
        assert_eq!(
            child.closure_variables()[1].source,
            ClosureSource::ParentClosure(1),
        );
        assert!(child.closure_variables()[0].is_lexical);
        assert!(matches!(
            child.closure_variables()[0].name,
            ClosureVariableName::Constant(_)
        ));
    }
}

#[test]
fn direct_eval_super_capability_is_explicit_and_independent_from_imports() {
    let pseudo = |name: &'static str, scope| EvalRootBinding {
        name: JsString::from_static(name),
        scope,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let direct = |bindings: Vec<EvalRootBinding<JsString>>,
                  scope_kinds: Vec<EvalScopeKind>,
                  super_call_allowed,
                  super_allowed| {
        EvalCompileContext::direct_with_profile(
            true,
            bindings,
            EvalCallerProfile {
                scope_kinds: scope_kinds.into_boxed_slice(),
                variable_target: EvalCallerVariableTarget::StrictLocal,
            },
            super_call_allowed,
            super_allowed,
        )
    };
    let method_bindings = || {
        vec![
            pseudo(THIS_LOCAL_NAME, 0),
            pseudo(HOME_OBJECT_LOCAL_NAME, 0),
        ]
    };

    let denied_despite_bindings = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            false,
            false,
        ),
    )
    .expect_err("hidden bindings must not grant parser authority");
    assert_eq!(denied_despite_bindings.kind(), ErrorKind::Syntax);
    assert_eq!(
        denied_despite_bindings.message(),
        "'super' is only valid in a method"
    );

    let allowed = compile_unlinked_eval_with_filename(
        "eval('super.value'); (() => super.value)",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            false,
            true,
        ),
    )
    .expect("a method-owned direct eval retains SuperProperty capability");
    assert_eq!(allowed.metadata().eval_kind, EvalKind::Direct);
    assert!(allowed.metadata().super_allowed);
    assert!(!allowed.metadata().super_call_allowed);
    assert!(!allowed.metadata().needs_home_object);
    assert!(
        allowed
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Eval { .. }))
    );
    let arrow = allowed
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("direct eval lost its arrow child");
    assert!(
        arrow
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValue))
    );
    let nested_environment = &allowed.eval_environments()[0];
    assert!(nested_environment.super_allowed);
    assert!(!nested_environment.super_call_allowed);
    let imported_owner = nested_environment
        .scopes
        .iter()
        .rev()
        .find(|scope| scope.kind == EvalScopeKind::FunctionRoot)
        .expect("nested eval lost its imported method owner");
    for expected in [THIS_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME] {
        assert!(
            imported_owner
                .bindings
                .iter()
                .any(|binding| binding.name == JsString::from_static(expected)),
            "nested eval lost {expected}",
        );
    }

    let ordinary_boundary = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            vec![
                pseudo(THIS_LOCAL_NAME, 0),
                pseudo(THIS_LOCAL_NAME, 1),
                pseudo(HOME_OBJECT_LOCAL_NAME, 1),
            ],
            vec![EvalScopeKind::FunctionRoot, EvalScopeKind::FunctionRoot],
            false,
            false,
        ),
    )
    .expect_err("an ordinary caller must truncate an outer method capability");
    assert_eq!(ordinary_boundary.kind(), ErrorKind::Syntax);
    assert_eq!(
        ordinary_boundary.message(),
        "'super' is only valid in a method"
    );

    let global = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            vec![pseudo(THIS_LOCAL_NAME, 0)],
            vec![EvalScopeKind::FunctionRoot],
            false,
            false,
        ),
    )
    .expect_err("a direct eval without a method owner must reject SuperProperty");
    assert_eq!(global.kind(), ErrorKind::Syntax);
    assert_eq!(global.message(), "'super' is only valid in a method");

    let indirect = compile_unlinked_eval_with_filename(
        "super.value",
        "<eval>",
        DebugInfoMode::StripDebug,
        EvalCompileContext::indirect(),
    )
    .expect_err("indirect eval must reject SuperProperty");
    assert_eq!(indirect.kind(), ErrorKind::Syntax);
    assert_eq!(indirect.message(), "'super' is only valid in a method");

    let super_call = compile_unlinked_eval_with_filename(
        "super()",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            method_bindings(),
            vec![EvalScopeKind::FunctionRoot],
            false,
            true,
        ),
    )
    .expect_err("SuperProperty capability must not enable SuperCall");
    assert_eq!(super_call.kind(), ErrorKind::Syntax);
    assert_eq!(
        super_call.message(),
        "super() is only valid in a derived class constructor"
    );

    let mut derived_this = pseudo(THIS_LOCAL_NAME, 0);
    derived_this.is_lexical = true;
    let enabled_super_call = compile_unlinked_eval_with_filename(
        "super()",
        "<eval>",
        DebugInfoMode::StripDebug,
        direct(
            vec![
                derived_this,
                pseudo(HOME_OBJECT_LOCAL_NAME, 0),
                pseudo(ACTIVE_FUNCTION_LOCAL_NAME, 0),
                pseudo(NEW_TARGET_LOCAL_NAME, 0),
            ],
            vec![EvalScopeKind::FunctionRoot],
            true,
            true,
        ),
    )
    .expect("an authenticated derived caller enables direct-eval SuperCall");
    assert!(enabled_super_call.metadata().super_call_allowed);
    assert!(enabled_super_call.metadata().super_allowed);
    assert!(
        enabled_super_call
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuper))
    );
    assert!(
        enabled_super_call
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::InitializeDerivedVarRef(_)))
    );
    assert!(enabled_super_call.code().windows(4).any(|window| {
        matches!(
            window,
            [
                Instruction::GetVarRef(_),
                Instruction::GetSuper,
                Instruction::GetVarRef(_),
                Instruction::MarkSuperCall,
            ]
        )
    }));
    assert!(
        enabled_super_call
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ConstructSuper(0)))
    );
}

#[test]
fn nested_eval_in_derived_constructor_accepts_inner_private_fields() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = r#"
        var testStr = `
            class C extends Object {
                constructor() {
                    eval(\`a => b => {
                        class Q { #f = 1; }
                        super();
                    }\`);
                    super();
                }
            }
            new C;
        `;
        (function () { eval(testStr); })();
    "#;

    assert_eq!(context.eval(source).unwrap(), Value::Undefined);
}

#[test]
fn compiler_preserves_authenticated_with_environment_relays() {
    let with_object = EvalRootBinding {
        name: JsString::from_static(WITH_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::WithObject,
        is_catch_parameter: false,
    };
    let root = compile_unlinked_eval_with_filename(
        "(function relay() { eval('value'); })",
        "<eval>",
        DebugInfoMode::StripDebug,
        EvalCompileContext::direct_with_profile(
            false,
            vec![with_object.clone()],
            EvalCallerProfile {
                scope_kinds: vec![
                    EvalScopeKind::With,
                    EvalScopeKind::ProgramBody,
                    EvalScopeKind::FunctionRoot,
                ]
                .into_boxed_slice(),
                variable_target: EvalCallerVariableTarget::Global,
            },
            false,
            false,
        ),
    )
    .unwrap();

    let root_descriptor = root
        .closure_variables()
        .first()
        .expect("eval root lost its with object external");
    assert_eq!(root_descriptor.source, ClosureSource::EvalEnvironment(0));
    assert_eq!(root_descriptor.kind, ClosureVariableKind::WithObject);
    assert!(!root_descriptor.is_lexical);
    assert!(!root_descriptor.is_const);
    let ClosureVariableName::Constant(name) = root_descriptor.name else {
        panic!("eval root lost its with object sentinel name");
    };
    assert!(matches!(
        root.constants()[name as usize].as_primitive(),
        Some(crate::engine::value::PrimitiveValue::String(name)) if name == &with_object.name
    ));

    let relay = root
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("eval root lost its relay function");
    let relay_descriptor = relay
        .closure_variables()
        .iter()
        .find(|descriptor| descriptor.kind == ClosureVariableKind::WithObject)
        .expect("nested eval did not relay the with object");
    assert_eq!(relay_descriptor.source, ClosureSource::ParentClosure(0));
    let with_scope = relay.eval_environments()[0]
        .scopes
        .iter()
        .find(|scope| scope.kind == EvalScopeKind::With)
        .expect("nested eval profile lost its with scope");
    let [binding] = with_scope.bindings.as_ref() else {
        panic!("with scope did not retain exactly one binding");
    };
    assert_eq!(binding.name, with_object.name);
    assert_eq!(binding.kind, ClosureVariableKind::WithObject);
    assert!(matches!(binding.source, EvalBindingSource::Closure(_)));
    assert!(!binding.is_lexical);
    assert!(!binding.is_const);
    assert!(!binding.is_catch_parameter);
}

#[test]
fn with_statements_execute_ordered_environment_and_reference_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval("var withGet = { value: 42 }; with (withGet) value")
            .unwrap(),
        Value::Int(42),
    );
    assert_eq!(
        context
            .eval(
                "var value = 7; var withHidden = { value: 42, [Symbol.unscopables]: { value: true } }; with (withHidden) value",
            )
            .unwrap(),
        Value::Int(7),
    );
    assert_eq!(
        context
            .eval(
                "var withWrite = { value: 1 }; with (withWrite) { value = 2; value += 3; value++; ++value; value ||= 99; } withWrite.value",
            )
            .unwrap(),
        Value::Int(7),
    );
    assert_eq!(
        context
            .eval(
                "var withCall = { value: 42, method: function(){ return this.value; } }; with (withCall) method()",
            )
            .unwrap(),
        Value::Int(42),
    );
    assert_eq!(
        context
            .eval(
                "var disappearingCall = { method: function(){} }; Object.defineProperty(disappearingCall, Symbol.unscopables, { get: function(){ delete disappearingCall.method; return {}; } }); var strictCaller; with (disappearingCall) strictCaller = function(){ 'use strict'; return method(); }; var disappearingError; try { strictCaller(); } catch (error) { disappearingError = error.name; } disappearingError",
            )
            .unwrap(),
        Value::String(JsString::from_static("TypeError")),
    );
    assert_eq!(
        context
            .eval(
                "var withDelete = { value: 1 }; var deleted; with (withDelete) deleted = delete value; deleted && !('value' in withDelete)",
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        context
            .eval(
                "var withCapture = { value: 42 }; var captured; with (withCapture) captured = function(){ return value; }; captured()",
            )
            .unwrap(),
        Value::Int(42),
    );

    let strict = compile_unlinked_script("'use strict'; with ({}) 0").unwrap_err();
    assert_eq!(strict.kind(), ErrorKind::Syntax);
    assert_eq!(strict.message(), "invalid keyword: with");
    assert_eq!(context.eval("with (null) 0"), Err(RuntimeError::Exception),);

    assert_eq!(
        context
            .eval(
                "var withSideEffect = 0; const withReadonly = 1; try { with ({}) withReadonly = (withSideEffect = 1); } catch (error) {} withSideEffect",
            )
            .unwrap(),
        Value::Int(0),
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var object = { value: 1 }; var value; with (object) { var value = (delete object.value, 2); } return object.value + '|' + ('value' in object) + '|' + value; })()",
            )
            .unwrap(),
        Value::String(JsString::from_static("2|true|undefined")),
    );
    assert_eq!(
        context
            .eval(
                "if (false) { with ({}) let\n value = 1; } if (false) { with ({}) let\n {} } 'parsed'",
            )
            .unwrap(),
        Value::String(JsString::from_static("parsed")),
    );
}

#[test]
fn eval_compiler_rejects_incoherent_caller_variable_profiles() {
    let strict_global = EvalCompileContext::direct_with_profile(
        true,
        Vec::new(),
        EvalCallerProfile {
            scope_kinds: Box::new([]),
            variable_target: EvalCallerVariableTarget::Global,
        },
        false,
        false,
    );
    assert!(
        compile_unlinked_eval_with_filename(
            "42",
            "<eval>",
            DebugInfoMode::StripDebug,
            strict_global,
        )
        .unwrap_err()
        .message()
        .contains("variable target is not authenticated")
    );

    let variable_object = EvalRootBinding {
        name: JsString::from_static("<var>"),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let sloppy_global = EvalCompileContext::direct_with_profile(
        false,
        vec![variable_object],
        EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::FunctionRoot].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::Global,
        },
        false,
        false,
    );
    assert!(
        compile_unlinked_eval_with_filename(
            "42",
            "<eval>",
            DebugInfoMode::StripDebug,
            sloppy_global,
        )
        .unwrap_err()
        .message()
        .contains("variable target is not authenticated")
    );

    let with_object = EvalRootBinding {
        name: JsString::from_static(WITH_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::WithObject,
        is_catch_parameter: false,
    };
    let with_as_variable_target = EvalCompileContext::direct_with_profile(
        false,
        vec![with_object],
        EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::With].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::ExternalBinding(0),
        },
        false,
        false,
    );
    assert!(
        compile_unlinked_eval_with_filename(
            "42",
            "<eval>",
            DebugInfoMode::StripDebug,
            with_as_variable_target,
        )
        .unwrap_err()
        .message()
        .contains("variable target is not authenticated")
    );
}

#[test]
fn strict_script_direct_eval_uses_a_local_eval_declaration_target() {
    let script = compile_unlinked_script("'use strict'; eval('var x = 42; x')").unwrap();
    assert!(script.metadata().strict);
    assert_eq!(script.eval_environments().len(), 1);
    assert!(script.eval_environments()[0].caller_strict);
    assert_eq!(
        script.eval_environments()[0].variable_environment,
        EvalVariableEnvironment::Global
    );

    assert_eq!(
        evaluate_in_context("'use strict'; eval('var x = 42; x')"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("'use strict'; eval('var x = 42'); typeof x"),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ return eval(\"'use strict'; eval('var x = 42; x')\") })()",
        ),
        Value::Int(42)
    );
}

#[test]
fn compiler_keeps_eval_lexicals_local_and_compiles_eval_declarations() {
    let root = compile_unlinked_eval_with_filename(
        "let answer = 40; const increment = 2; answer + increment",
        "<eval>",
        DebugInfoMode::StripDebug,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert_eq!(root.metadata().eval_kind, EvalKind::Indirect);
    assert!(!root.metadata().strict);
    let definitions = root
        .local_definitions()
        .iter()
        .filter_map(|definition| {
            definition
                .name
                .as_ref()
                .map(|name| (name.to_utf8_lossy(), definition.is_const))
        })
        .collect::<Vec<_>>();
    assert!(definitions.contains(&("answer".to_owned(), false)));
    assert!(definitions.contains(&("increment".to_owned(), true)));

    let declarations = compile_unlinked_eval_with_filename(
        "var answer = 42; function fortyTwo() { return answer; } fortyTwo()",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(declarations.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && descriptor.kind == ClosureVariableKind::GlobalFunction
    }));

    let strict = compile_unlinked_eval_with_filename(
        "'use strict'; var answer = 42; function fortyTwo() { return answer; }",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(strict.metadata().strict);
    assert!(strict.local_definitions().iter().any(|definition| {
        definition
            .name
            .as_ref()
            .is_some_and(|name| name.to_utf8_lossy() == "answer")
    }));

    let nested = compile_unlinked_eval_with_filename(
        "eval('40 + 2')",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(
        nested
            .code()
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::Eval { environment: 0, .. }) })
    );
    assert_eq!(nested.eval_environments().len(), 1);
    assert_eq!(
        nested.eval_environments()[0].variable_environment,
        EvalVariableEnvironment::Global
    );
}

#[test]
fn sloppy_eval_function_owns_hidden_variable_object_and_orders_eval_pseudo_bindings() {
    let script =
        compile_unlinked_script("(function named(a) { eval(0); arguments; named; return a; })")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    let object_local = function
        .metadata()
        .eval_variable_object_local
        .expect("sloppy direct eval did not allocate <var>");
    assert!(function.code().windows(2).any(|window| matches!(
        window,
        [Instruction::VariableEnvironment, Instruction::PutLocal(local)]
            if *local == object_local
    )));
    assert_eq!(
        function.local_definitions()[usize::from(object_local)].kind,
        ClosureVariableKind::EvalVariableObject
    );

    let root_scope = function.eval_environments()[0]
        .scopes
        .iter()
        .find(|scope| scope.kind == EvalScopeKind::FunctionRoot)
        .unwrap();
    let names = root_scope
        .bindings
        .iter()
        .map(|binding| binding.name.to_utf8_lossy())
        .collect::<Vec<_>>();
    let authored = names.iter().position(|name| name == "a").unwrap();
    let object = names
        .iter()
        .position(|name| name == EVAL_VARIABLE_OBJECT_LOCAL_NAME)
        .unwrap();
    let arguments = names.iter().position(|name| name == "arguments").unwrap();
    let private_name = names.iter().position(|name| name == "named").unwrap();
    assert!(authored < object);
    assert!(object < arguments);
    assert!(object < private_name);
    let function_name_local = function
        .metadata()
        .function_name_local
        .expect("direct eval did not materialize the private function name");
    assert!(
        function
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetLocal(local) if *local == function_name_local)),
        "authored code did not read the private function name statically",
    );
    let dynamically_resolves_private_name = function
        .code()
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::HasEvalVariable { name, .. }
            | Instruction::GetEvalVariable { name, .. }
            | Instruction::PutEvalVariable { name, .. }
            | Instruction::DeleteEvalVariable { name, .. } => Some(*name),
            _ => None,
        })
        .any(|name| {
            matches!(
                function.constants()[usize::try_from(name).unwrap()].as_primitive(),
                Some(crate::engine::value::PrimitiveValue::String(value)) if value == &JsString::from_static("named")
            )
        });
    assert!(
        !dynamically_resolves_private_name,
        "authored private name was incorrectly wrapped by the eval variable object",
    );
}

#[test]
fn parameter_environment_descendant_arguments_capture_keeps_a_body_only_prologue() {
    let script = compile_unlinked_script(
        "(function(a=(()=>[eval('typeof arguments'),arguments.length].join('|'))()){return a})",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    let layout = function
        .parameter_environment()
        .expect("outer function has a parameter environment");
    assert_eq!(layout.synthetic_arguments_local, None);
    validate_parameter_bytecode_layout(
        function.metadata(),
        function.code(),
        &vec![false; usize::from(function.metadata().local_count)],
        Some(layout),
    )
    .unwrap();
    let arguments_local = function
        .code()
        .windows(2)
        .find_map(|window| match window {
            [
                Instruction::Arguments(ArgumentsKind::Unmapped),
                Instruction::PutLocal(local),
            ] => Some(*local),
            _ => None,
        })
        .expect("outer function has one body-only arguments prologue");
    assert!(
        function.local_definitions()[usize::from(arguments_local)]
            .name
            .as_ref()
            .is_some_and(|name| name.to_utf8_lossy() == "arguments")
    );
    let arrow = function
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("parameter initializer arrow");
    let binding = arrow.eval_environments()[0]
        .scopes
        .iter()
        .flat_map(|scope| scope.bindings.iter())
        .find(|binding| binding.name.to_utf8_lossy() == "arguments")
        .expect("late arguments closure is visible to direct eval");
    let EvalBindingSource::Closure(closure) = binding.source else {
        panic!("late arguments binding did not use the caller closure suffix");
    };
    let descriptor = arrow.closure_variables()[usize::from(closure)];
    assert!(
        matches!(descriptor.source, ClosureSource::ParentLocal(local) if local == arguments_local)
    );
}

#[test]
fn binding_pattern_initializers_can_read_and_capture_parameter_arguments() {
    assert_eq!(
        evaluate_in_context(
            "(function({x=arguments[0].answer}={},a=eval('0')){return x})({x:undefined,answer:42})",
        ),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context(
            "(function({x=()=>arguments[0].answer}={},a=eval('0')){return x()})({x:undefined,answer:42})",
        ),
        Value::Int(42)
    );
}

#[test]
fn sloppy_direct_eval_keeps_source_ordered_dynamic_declarations() {
    let bindings = vec![EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    }];
    let eval = compile_unlinked_eval_with_filename(
        "var fresh; function f() {} var fresh;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, bindings),
    )
    .unwrap();
    assert!(matches!(
        eval.code(),
        [
            Instruction::Undefined,
            Instruction::DefineEvalVariable {
                source: EvalVariableSource::Closure(0),
                ..
            },
            Instruction::FClosure(_),
            Instruction::DefineEvalVariable {
                source: EvalVariableSource::Closure(0),
                ..
            },
            Instruction::Undefined,
            Instruction::DefineEvalVariable {
                source: EvalVariableSource::Closure(0),
                ..
            },
            ..
        ]
    ));
}

#[test]
fn eval_destructuring_novel_vars_retest_the_dynamic_environment_before_global_fallback() {
    let bindings = vec![EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    }];
    let eval = compile_unlinked_eval_with_filename(
        "var { x = 41 } = {}; var [y = 42] = [];",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, bindings),
    )
    .unwrap();

    assert_eq!(
        eval.code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::DefineEvalVariable { .. }))
            .count(),
        2
    );
    assert_eq!(
        eval.code()
            .iter()
            .filter(|instruction| matches!(
                instruction,
                Instruction::HasDynamicBinding {
                    source: DynamicEnvironmentSource::Eval(EvalVariableSource::Closure(0)),
                    ..
                }
            ))
            .count(),
        2
    );
    assert_eq!(
        eval.code()
            .iter()
            .filter(|instruction| matches!(
                instruction,
                Instruction::PutDynamicBinding {
                    source: DynamicEnvironmentSource::Eval(EvalVariableSource::Closure(0)),
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(
        !eval
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GlobalReference(_)))
    );
}

#[test]
fn sloppy_direct_eval_initializes_novel_destructured_var_bindings() {
    assert_eq!(
        evaluate_in_context("(function(){eval('var {x=41}={}');return x})()"),
        Value::Int(41)
    );
    assert_eq!(
        evaluate_in_context("(function(){eval('var [x=42]=[]');return x})()"),
        Value::Int(42)
    );
}

#[test]
fn eval_redeclaration_throw_precedes_global_and_dynamic_value_writes() {
    let caller_lexical = EvalRootBinding {
        name: JsString::from_static("x"),
        scope: 0,
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let global = compile_unlinked_eval_with_filename(
        "function f() {} var y; var x;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![caller_lexical.clone()]),
    )
    .unwrap();
    assert!(matches!(
        global.code(),
        [
            Instruction::ThrowRedeclaration(_),
            Instruction::FClosure(_),
            ..
        ]
    ));
    assert_eq!(
        global
            .closure_variables()
            .iter()
            .filter(|descriptor| descriptor.source == ClosureSource::GlobalDeclaration)
            .count(),
        3
    );

    let object = EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 1,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let dynamic_conflict = compile_unlinked_eval_with_filename(
        "var y; var x;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![caller_lexical, object.clone()]),
    )
    .unwrap();
    assert!(matches!(
        dynamic_conflict.code(),
        [
            Instruction::ThrowRedeclaration(_),
            Instruction::Undefined,
            Instruction::DefineEvalVariable { .. },
            ..
        ]
    ));

    let outer_lexical = EvalRootBinding {
        name: JsString::from_static("outer"),
        scope: 2,
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let shadows_outer = compile_unlinked_eval_with_filename(
        "var outer;",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![object, outer_lexical]),
    )
    .unwrap();
    assert!(matches!(
        shadows_outer.code(),
        [
            Instruction::Undefined,
            Instruction::DefineEvalVariable { .. },
            ..
        ]
    ));
    assert!(
        !shadows_outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowRedeclaration(_)))
    );
}

#[test]
fn eval_annex_b_functions_target_global_or_dynamic_variable_environments() {
    let indirect = compile_unlinked_eval_with_filename(
        "{ function f() {} }",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(indirect.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && descriptor.kind == ClosureVariableKind::Normal
    }));

    let object = EvalRootBinding {
        name: JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let direct = compile_unlinked_eval_with_filename(
        "{ function f() {} }",
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![object.clone()]),
    )
    .unwrap();
    assert!(
        direct
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineEvalVariable { .. }))
    );
    assert!(direct.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::HasDynamicBinding {
            source: DynamicEnvironmentSource::Eval(_),
            ..
        }
    )));

    let catch_source = "f; try { throw null; } catch (f) {{ function f() {} }} typeof f;";
    let catch_direct = compile_unlinked_eval_with_filename(
        catch_source,
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::direct(false, vec![object]),
    )
    .unwrap();
    assert!(matches!(
        catch_direct.code(),
        [
            Instruction::Undefined,
            Instruction::DefineEvalVariable { .. },
            ..
        ]
    ));

    let catch_indirect = compile_unlinked_eval_with_filename(
        catch_source,
        "<eval>",
        DebugInfoMode::Full,
        EvalCompileContext::indirect(),
    )
    .unwrap();
    assert!(catch_indirect.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && descriptor.kind == ClosureVariableKind::Normal
    }));
}

#[test]
fn compiler_links_and_deduplicates_direct_eval_scope_descriptors() {
    let source = r#"
        (function outer(outerArg) {
            let outerLex = 1;
            return function middle() {
                return function inner(local) {
                    outerArg;
                    {
                        let outerLex = 2;
                        let blockOnly = 3;
                        eval(0);
                        eval(1);
                    }
                    eval(2);
                };
            };
        })
    "#;
    let script = compile_unlinked_script(source).unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let middle = outer.constants()[0].as_child().unwrap();
    let inner = middle.constants()[0].as_child().unwrap();

    let instructions = inner
        .code()
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Eval {
                argument_count,
                environment,
            } => Some((*argument_count, *environment)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(instructions, [(1, 0), (1, 0), (1, 1)]);
    assert_eq!(inner.eval_environments().len(), 2);
    let eval_variable_object = inner.metadata().eval_variable_object_local.unwrap();

    let block = &inner.eval_environments()[0];
    assert_eq!(
        block.variable_environment,
        EvalVariableEnvironment::VariableObject {
            scope: 2,
            source: EvalBindingSource::Local(eval_variable_object),
        }
    );
    assert!(!block.caller_strict);
    assert_eq!(block.scopes[0].kind, EvalScopeKind::Block);
    assert_eq!(block.scopes[1].kind, EvalScopeKind::FunctionBody);
    assert_eq!(block.scopes[2].kind, EvalScopeKind::FunctionRoot);
    assert_eq!(block.scopes[3].kind, EvalScopeKind::FunctionBody);
    assert_eq!(block.scopes[4].kind, EvalScopeKind::FunctionRoot);
    assert_eq!(block.scopes[5].kind, EvalScopeKind::FunctionBody);
    assert_eq!(block.scopes[6].kind, EvalScopeKind::FunctionRoot);
    assert_eq!(block.scopes[7].kind, EvalScopeKind::ProgramBody);
    assert_eq!(block.scopes[8].kind, EvalScopeKind::FunctionRoot);

    let names =
        |environment: &crate::engine::code::function::metadata::EvalEnvironment<JsString>| {
            environment
                .scopes
                .iter()
                .flat_map(|scope| scope.bindings.iter())
                .map(|binding| {
                    (
                        binding.name.to_utf8_lossy(),
                        binding.source,
                        binding.is_lexical,
                        binding.kind,
                    )
                })
                .collect::<Vec<_>>()
        };
    let block_names = names(block);
    assert_eq!(block_names[0].0, "blockOnly");
    assert_eq!(block_names[1].0, "outerLex");
    assert!(matches!(block_names[0].1, EvalBindingSource::Local(_)));
    assert!(matches!(block_names[1].1, EvalBindingSource::Local(_)));
    assert!(block_names[0].2 && block_names[1].2);
    assert!(block_names.iter().any(|binding| {
        binding.0 == "outerLex" && matches!(binding.1, EvalBindingSource::Closure(_))
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "outerArg" && matches!(binding.1, EvalBindingSource::Closure(_))
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "outer"
            && matches!(binding.1, EvalBindingSource::Closure(_))
            && binding.3 == ClosureVariableKind::Normal
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "middle"
            && matches!(binding.1, EvalBindingSource::Closure(_))
            && binding.3 == ClosureVariableKind::Normal
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "inner"
            && matches!(binding.1, EvalBindingSource::Local(_))
            && binding.3 == ClosureVariableKind::FunctionName
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "arguments" && matches!(binding.1, EvalBindingSource::Local(_))
    }));
    assert!(block_names.iter().any(|binding| {
        binding.0 == "local" && matches!(binding.1, EvalBindingSource::Argument(0))
    }));

    let body_names = names(&inner.eval_environments()[1]);
    assert!(!body_names.iter().any(|binding| binding.0 == "blockOnly"));
    let first_outer_lex = body_names
        .iter()
        .find(|binding| binding.0 == "outerLex")
        .unwrap();
    assert!(matches!(first_outer_lex.1, EvalBindingSource::Closure(_)));
    assert_eq!(
        inner.eval_environments()[1].variable_environment,
        EvalVariableEnvironment::VariableObject {
            scope: 1,
            source: EvalBindingSource::Local(eval_variable_object),
        }
    );

    let block_only = inner
        .local_definitions()
        .iter()
        .position(|definition| {
            definition
                .name
                .as_ref()
                .is_some_and(|name| name.to_utf8_lossy() == "blockOnly")
        })
        .and_then(|index| u16::try_from(index).ok())
        .unwrap();
    assert!(inner.code().iter().any(
        |instruction| matches!(instruction, Instruction::CloseLocal(index) if *index == block_only)
    ));
    assert!(outer.metadata().function_name_local.is_some());
    assert!(middle.metadata().function_name_local.is_some());
    assert!(inner.metadata().function_name_local.is_some());
    let outer_lex_closure = block_names
        .iter()
        .find_map(|binding| match (binding.0.as_str(), binding.1) {
            ("outerLex", EvalBindingSource::Closure(index)) => Some(index),
            _ => None,
        })
        .unwrap();
    let ClosureSource::ParentClosure(relay) =
        inner.closure_variables()[usize::from(outer_lex_closure)].source
    else {
        panic!("outer eval binding did not cross the intermediate function relay");
    };
    assert!(matches!(
        middle.closure_variables()[usize::from(relay)].source,
        ClosureSource::ParentLocal(_)
    ));

    // The eval entry prepass allocates and names this exact relay chain
    // before ordinary identifier resolution; the later reference reuses
    // the first-slot-wins descriptors without creating duplicates.
    let outer_arg_closure = block_names
        .iter()
        .find_map(|binding| match (binding.0.as_str(), binding.1) {
            ("outerArg", EvalBindingSource::Closure(index)) => Some(index),
            _ => None,
        })
        .unwrap();
    let inner_outer_arg = inner.closure_variables()[usize::from(outer_arg_closure)];
    assert!(inner.code().iter().any(
        |instruction| matches!(instruction, Instruction::GetVarRef(index) if *index == outer_arg_closure)
    ));
    let ClosureVariableName::Constant(inner_name) = inner_outer_arg.name else {
        panic!("eval-visible ordinary closure did not retain its name");
    };
    assert!(matches!(
        inner.constants()[usize::try_from(inner_name).unwrap()].as_primitive(),
        Some(crate::engine::value::PrimitiveValue::String(name)) if name.to_utf8_lossy() == "outerArg"
    ));
    let ClosureSource::ParentClosure(middle_outer_arg) = inner_outer_arg.source else {
        panic!("outer argument did not cross the intermediate relay");
    };
    assert_eq!(
        inner
            .closure_variables()
            .iter()
            .filter(|descriptor| descriptor.source == inner_outer_arg.source)
            .count(),
        1,
        "ordinary resolution must reuse the eval-created inner slot"
    );
    let middle_outer_arg = middle.closure_variables()[usize::from(middle_outer_arg)];
    assert_eq!(
        middle
            .closure_variables()
            .iter()
            .filter(|descriptor| descriptor.source == middle_outer_arg.source)
            .count(),
        1,
        "ordinary resolution must reuse the eval-created relay slot"
    );
    let ClosureVariableName::Constant(middle_name) = middle_outer_arg.name else {
        panic!("intermediate eval relay did not retain its ordinary name");
    };
    assert!(matches!(
        middle.constants()[usize::try_from(middle_name).unwrap()].as_primitive(),
        Some(crate::engine::value::PrimitiveValue::String(name)) if name.to_utf8_lossy() == "outerArg"
    ));
}

#[test]
fn eval_scope_descriptors_are_semantic_metadata_in_strip_debug_mode() {
    let source = r#"
        (function outer(argument) {
            let captured = 1;
            return function inner() {
                { let local = 2; eval(0); }
                return captured;
            };
        })
    "#;
    let full =
        compile_unlinked_script_with_filename(source, "eval-descriptor.js", DebugInfoMode::Full)
            .unwrap();
    let stripped = compile_unlinked_script_with_filename(
        source,
        "eval-descriptor.js",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let full_outer = full.constants()[0].as_child().unwrap();
    let stripped_outer = stripped.constants()[0].as_child().unwrap();
    let full_inner = full_outer.constants()[0].as_child().unwrap();
    let stripped_inner = stripped_outer.constants()[0].as_child().unwrap();

    assert_eq!(
        full_inner.eval_environments(),
        stripped_inner.eval_environments()
    );
    assert_eq!(
        full_inner.closure_variables(),
        stripped_inner.closure_variables()
    );
    assert!(stripped_inner.local_definitions().iter().any(|definition| {
        definition
            .name
            .as_ref()
            .is_some_and(|name| name.to_utf8_lossy() == "local")
    }));

    let runtime = Runtime::new();
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    runtime.new_context().compile(source).unwrap();
}
