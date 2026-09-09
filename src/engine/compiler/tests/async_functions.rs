use super::*;

#[test]
fn ordinary_async_functions_publish_async_kind_and_await_bytecode() {
    let source = r#"
        async function declaration(value) { return await value; }
        (async function expression() {
            try {
                return await 41;
            } catch (error) {
                return error;
            }
        });
        { async function blockLocal() { return 42; } }
    "#;
    let tree = Parser::parse(source, JsString::from_static("<async-kind-test>")).unwrap();
    let async_functions = tree
        .functions
        .iter()
        .filter(|function| function.execution_kind == BytecodeFunctionKind::Async)
        .collect::<Vec<_>>();
    assert_eq!(async_functions.len(), 3);
    assert!(async_functions.iter().all(|function| {
        function.kind == FunctionKind::Ordinary
            && function.in_function_body
            && function.ops.iter().all(|operation| {
                !matches!(
                    operation.op,
                    super::IrOp::Bytecode(
                        Instruction::InitialYield | Instruction::Yield | Instruction::YieldStar
                    )
                )
            })
    }));
    assert_eq!(
        async_functions
            .iter()
            .flat_map(|function| &function.ops)
            .filter(|operation| matches!(operation.op, super::IrOp::Bytecode(Instruction::Await)))
            .count(),
        2
    );

    let script = compile_unlinked_script(source).unwrap();
    fn collect_async<'a>(
        function: &'a crate::engine::code::function::UnlinkedFunction,
        output: &mut Vec<&'a crate::engine::code::function::UnlinkedFunction>,
    ) {
        for constant in function.constants() {
            let Some(child) = constant.as_child() else {
                continue;
            };
            if child.metadata().function_kind == BytecodeFunctionKind::Async {
                output.push(child);
            }
            collect_async(child, output);
        }
    }
    let mut published = Vec::new();
    collect_async(&script, &mut published);
    assert_eq!(published.len(), 3);
    assert!(published.iter().all(|function| {
        !function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
    }));
}

#[test]
fn async_object_methods_publish_method_grammar_async_execution_and_full_source_ranges() {
    let source = concat!(
        "const object = {\n",
        "  async fixed(value = super.seed) { return await value; },\n",
        "  async ['computed'](value) { return await value; }\n",
        "};",
    );
    let tree = Parser::parse(
        source,
        JsString::from_static("<async-object-method-kind-test>"),
    )
    .unwrap();
    let methods = tree
        .functions
        .iter()
        .filter(|function| function.kind == FunctionKind::Method)
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|function| {
        function.execution_kind == BytecodeFunctionKind::Async
            && function.in_function_body
            && function.super_allowed
            && !function.super_call_allowed
            && function
                .ops
                .iter()
                .filter(|operation| {
                    matches!(operation.op, super::IrOp::Bytecode(Instruction::Await))
                })
                .count()
                == 1
    }));
    for (function, authored) in methods.iter().zip([
        "async fixed(value = super.seed) { return await value; }",
        "async ['computed'](value) { return await value; }",
    ]) {
        let start = source.find(authored).expect("authored async method");
        let range = function
            .source
            .range
            .as_ref()
            .expect("async method source range");
        assert_eq!(
            &source[function.source.span.start.byte_offset..function.source.span.end.byte_offset],
            "async"
        );
        assert_eq!(function.source.definition.as_usize(), start);
        assert_eq!(range.start.as_usize(), start);
        assert_eq!(range.end.as_usize(), start + authored.len());
    }

    let script = compile_unlinked_script(source).unwrap();
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethod {
            kind: DefineMethodKind::Method,
            enumerable: true,
            ..
        }
    )));
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethodComputed {
            kind: DefineMethodKind::Method,
            enumerable: true,
        }
    )));
    let published = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|function| {
        function.metadata().function_kind == BytecodeFunctionKind::Async
            && !function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
    }));
    assert!(published[0].metadata().needs_home_object);
}

#[test]
fn async_generator_object_methods_compose_method_grammar_with_the_async_driver() {
    let source = concat!(
        "const object = {\n",
        "  async /* a */ * fixed(value = super.seed) { await value; yield value; },\n",
        "  async *['computed'](value) { yield value; }\n",
        "};",
    );
    let tree = Parser::parse(
        source,
        JsString::from_static("<async-generator-object-method-kind-test>"),
    )
    .unwrap();
    let methods = tree
        .functions
        .iter()
        .filter(|function| {
            function.kind == FunctionKind::Method
                && function.execution_kind == BytecodeFunctionKind::AsyncGenerator
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|function| {
        function.in_function_body
            && function.super_allowed
            && !function.super_call_allowed
            && function
                .ops
                .iter()
                .filter(|operation| {
                    matches!(
                        operation.op,
                        super::IrOp::Bytecode(Instruction::InitialYield)
                    )
                })
                .count()
                == 1
            && function
                .ops
                .iter()
                .any(|operation| matches!(operation.op, super::IrOp::Bytecode(Instruction::Yield)))
    }));
    assert!(
        methods[0]
            .ops
            .iter()
            .any(|operation| { matches!(operation.op, super::IrOp::Bytecode(Instruction::Await)) })
    );
    for (function, authored) in methods.iter().zip([
        "async /* a */ * fixed(value = super.seed) { await value; yield value; }",
        "async *['computed'](value) { yield value; }",
    ]) {
        let start = source
            .find(authored)
            .expect("authored async-generator object method");
        let range = function
            .source
            .range
            .as_ref()
            .expect("async-generator method source range");
        assert_eq!(
            &source[function.source.span.start.byte_offset..function.source.span.end.byte_offset],
            "async"
        );
        assert_eq!(function.source.definition.as_usize(), start);
        assert_eq!(range.start.as_usize(), start);
        assert_eq!(range.end.as_usize(), start + authored.len());
    }

    let script = compile_unlinked_script(source).unwrap();
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethod {
            kind: DefineMethodKind::Method,
            enumerable: true,
            ..
        }
    )));
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethodComputed {
            kind: DefineMethodKind::Method,
            enumerable: true,
        }
    )));
    let published = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .filter(|function| {
            function.metadata().function_kind == BytecodeFunctionKind::AsyncGenerator
        })
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|function| {
        function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
            && function
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::InitialYield))
                .count()
                == 1
    }));
    assert!(published[0].metadata().needs_home_object);
}

#[test]
fn async_generator_yield_star_uses_typed_async_iterator_and_await_protocol() {
    let source = "async function* delegate() { yield* source; }";
    let tree = Parser::parse(
        source,
        JsString::from_static("<async-generator-yield-star-lowering-test>"),
    )
    .unwrap();
    let function = tree
        .functions
        .iter()
        .find(|function| function.execution_kind == BytecodeFunctionKind::AsyncGenerator)
        .expect("async-generator definition");
    let instructions = function
        .ops
        .iter()
        .filter_map(|operation| match &operation.op {
            super::IrOp::Bytecode(instruction) => Some(instruction),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::AsyncIteratorStart))
            .count(),
        1
    );
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::AsyncYieldStar))
            .count(),
        1
    );
    assert!(!instructions.iter().any(|instruction| matches!(
        instruction,
        Instruction::IteratorStart | Instruction::YieldStar
    )));
    // QuickJS awaits next, the injected return value, return/throw method
    // results, the missing-throw close result, and the delegated return value.
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Await))
            .count(),
        6
    );
    for kind in [
        IteratorCallKind::ReturnWithValue,
        IteratorCallKind::ThrowWithValue,
        IteratorCallKind::ReturnWithoutValue,
    ] {
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| {
                    matches!(instruction, Instruction::IteratorCall(found) if *found == kind)
                })
                .count(),
            1
        );
    }

    let async_start = instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::AsyncIteratorStart))
        .unwrap();
    let next = instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IteratorNext))
        .unwrap();
    let first_await = instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Await))
        .unwrap();
    let check = instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IteratorCheckObject))
        .unwrap();
    let async_yield = instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::AsyncYieldStar))
        .unwrap();
    assert!(async_start < next && next < first_await && first_await < check && check < async_yield);

    let published = compile_unlinked_script(source).unwrap();
    let delegate = published
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .find(|function| function.metadata().function_kind == BytecodeFunctionKind::AsyncGenerator)
        .expect("published async-generator definition");
    assert!(
        delegate
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::AsyncYieldStar))
    );
}

#[test]
fn async_class_methods_reuse_method_publication_and_preserve_full_source_ranges() {
    let source = concat!(
        "class Base { value() { return this.seed; } static value() { return this.seed; } }\n",
        "class Class extends Base {\n",
        "  async fixed(value = super.value()) { return await value; }\n",
        "  static async ['computed'](value = super.value()) { return await value; }\n",
        "}",
    );
    let tree = Parser::parse(
        source,
        JsString::from_static("<async-class-method-kind-test>"),
    )
    .unwrap();
    let methods = tree
        .functions
        .iter()
        .filter(|function| {
            function.kind == FunctionKind::Method
                && function.execution_kind == BytecodeFunctionKind::Async
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|function| {
        function.in_function_body
            && function.super_allowed
            && !function.super_call_allowed
            && function
                .ops
                .iter()
                .filter(|operation| {
                    matches!(operation.op, super::IrOp::Bytecode(Instruction::Await))
                })
                .count()
                == 1
    }));
    for (function, authored) in methods.iter().zip([
        "async fixed(value = super.value()) { return await value; }",
        "async ['computed'](value = super.value()) { return await value; }",
    ]) {
        let start = source.find(authored).expect("authored async class method");
        let range = function
            .source
            .range
            .as_ref()
            .expect("async class method source range");
        assert_eq!(
            &source[function.source.span.start.byte_offset..function.source.span.end.byte_offset],
            "async"
        );
        assert_eq!(function.source.definition.as_usize(), start);
        assert_eq!(range.start.as_usize(), start);
        assert_eq!(range.end.as_usize(), start + authored.len());
    }

    let script = compile_unlinked_script(source).unwrap();
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethod {
            kind: DefineMethodKind::Method,
            enumerable: false,
            ..
        }
    )));
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethodComputed {
            kind: DefineMethodKind::Method,
            enumerable: false,
        }
    )));
    let published = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .filter(|function| function.metadata().function_kind == BytecodeFunctionKind::Async)
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|function| {
        function.metadata().needs_home_object
            && !function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
    }));
}

#[test]
fn async_generator_class_methods_compose_method_grammar_with_the_async_driver() {
    let source = concat!(
        "class Base {}\n",
        "class Class extends Base {\n",
        "  async /* a */ * fixed(value = super.seed) { await value; yield value; }\n",
        "  static async *['computed'](value = super.seed) { yield value; }\n",
        "}",
    );
    let tree = Parser::parse(
        source,
        JsString::from_static("<async-generator-class-method-kind-test>"),
    )
    .unwrap();
    let methods = tree
        .functions
        .iter()
        .filter(|function| {
            function.kind == FunctionKind::Method
                && function.execution_kind == BytecodeFunctionKind::AsyncGenerator
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|function| {
        function.in_function_body
            && function.super_allowed
            && !function.super_call_allowed
            && function
                .ops
                .iter()
                .filter(|operation| {
                    matches!(
                        operation.op,
                        super::IrOp::Bytecode(Instruction::InitialYield)
                    )
                })
                .count()
                == 1
            && function
                .ops
                .iter()
                .any(|operation| matches!(operation.op, super::IrOp::Bytecode(Instruction::Yield)))
    }));
    assert!(
        methods[0]
            .ops
            .iter()
            .any(|operation| { matches!(operation.op, super::IrOp::Bytecode(Instruction::Await)) })
    );
    for (function, authored) in methods.iter().zip([
        "async /* a */ * fixed(value = super.seed) { await value; yield value; }",
        "async *['computed'](value = super.seed) { yield value; }",
    ]) {
        let start = source
            .find(authored)
            .expect("authored async-generator class method");
        let range = function
            .source
            .range
            .as_ref()
            .expect("async-generator class method source range");
        assert_eq!(
            &source[function.source.span.start.byte_offset..function.source.span.end.byte_offset],
            "async"
        );
        assert_eq!(function.source.definition.as_usize(), start);
        assert_eq!(range.start.as_usize(), start);
        assert_eq!(range.end.as_usize(), start + authored.len());
    }

    let script = compile_unlinked_script(source).unwrap();
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethod {
            kind: DefineMethodKind::Method,
            enumerable: false,
            ..
        }
    )));
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethodComputed {
            kind: DefineMethodKind::Method,
            enumerable: false,
        }
    )));
    let published = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .filter(|function| {
            function.metadata().function_kind == BytecodeFunctionKind::AsyncGenerator
        })
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|function| {
        function.metadata().needs_home_object
            && function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
            && function
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::InitialYield))
                .count()
                == 1
    }));
}

#[test]
fn async_class_method_contextual_boundaries_match_quickjs() {
    for source in [
        "class C { async method(){ return await 1; } }",
        "class C { static async ['computed'](){ return await 1; } }",
        "class C { async *generator(){ yield 1; } }",
        "class C { static async *['computed'](){ yield 1; } }",
        "class C { async/*\u{2028}*/*generator(){ yield 1; } }",
        "class C { async/*\u{2029}*/*generator(){ yield 1; } }",
        "class C { async await(){ return 1; } static async constructor(){ return 2; } }",
        "class C { async/*\u{2028}*/method(){ return 1; } }",
        "class C { async/*\u{2029}*/method(){ return 1; } }",
        "class C { async(){} static async(){} }",
        "class C { async #private(){} }",
        "class C { static async #private(){} }",
        "class C { async *#private(){ yield 1; } }",
        "class C { static async *#private(){ yield 1; } }",
        "class C { async\nmethod(){} }",
        "class C { async/*\n*/method(){} }",
        "class C { async\u{2028}method(){} }",
        "class C { async\u{2029}method(){} }",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("async class source rejected {source:?}: {error}"));
    }

    for source in [
        "class C { async constructor(){} }",
        "class C { static async prototype(){} }",
        "class C { async get method(){} }",
        "class C { async method(value = await 1){} }",
        r"class C { async method(\u0061wait){} }",
        r"class C { \u0061sync method(){} }",
        r"class C { \u0061sync *method(){} }",
        "class C { async *constructor(){} }",
        "class C { static async *prototype(){} }",
        "class C { async *method(value = await 1){} }",
        "class C { async *method(value = yield 1){} }",
        "class C { async; }",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }
    for source in [
        "class C { async *constructor(value = await 1) { yield* source; } }",
        "class C { static async *prototype(value = yield 1) { super(); } }",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), "invalid method name", "{source:?}");
    }
}

#[test]
fn await_arrow_like_sequences_keep_async_diagnostic_precedence() {
    for (source, message, column) in [
        (
            "async function outer(){ (await => 0); }",
            "unexpected token in expression: '=>'",
            32,
        ),
        (
            "async(a = await => {}) => {}",
            "await in default expression",
            11,
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error
            .span()
            .expect("await/arrow syntax error lost its span");
        assert_eq!(
            (span.start.line, span.start.column),
            (1, column),
            "{source}"
        );
    }

    let error = compile_unlinked_module_with_filename(
        "await => 0;",
        "await-arrow.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "unexpected token in expression: '=>'");
    let span = error
        .span()
        .expect("module await/arrow error lost its span");
    assert_eq!((span.start.line, span.start.column), (1, 7));
}

#[test]
fn async_object_method_contextual_boundaries_match_quickjs() {
    for source in [
        "({ async method(){ return await 1; } });",
        "({ async ['computed'](value = (nested = (await) => await) => nested){ return await value; } });",
        "({ async await(){ return 1; }, async yield(){ return 2; } });",
        "({ async/*\u{2028}*/method(){ return 1; } });",
        "({ async/*\u{2029}*/method(){ return 1; } });",
        "({ async(){} });",
        "({ async\n: 1 });",
        "({ async *generator(){ yield 1; } });",
        "({ async *['computed'](){ yield 1; } });",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("async object method source rejected {source:?}: {error}")
        });
    }

    for source in [
        "({ async method(value = await 1){} });",
        r"({ async method(\u0061wait){} });",
        "({ async method(value, value){} });",
        r"({ \u0061sync method(){} });",
        "({ async\nmethod(){} });",
        "({ async/*\n*/method(){} });",
        "({ async\u{2028}method(){} });",
        "({ async\u{2029}method(){} });",
        "({ async get method(){} });",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }
}

#[test]
fn async_arrows_publish_arrow_grammar_async_execution_and_full_source_ranges() {
    let source = concat!(
        "const first = async value => await value;\n",
        "const second = async (value) => { return await value; };",
    );
    let tree = Parser::parse(source, JsString::from_static("<async-arrow-kind-test>")).unwrap();
    let arrows = tree
        .functions
        .iter()
        .filter(|function| function.kind == FunctionKind::Arrow)
        .collect::<Vec<_>>();
    assert_eq!(arrows.len(), 2);
    assert!(arrows.iter().all(|function| {
        function.execution_kind == BytecodeFunctionKind::Async
            && function.in_function_body
            && function
                .ops
                .iter()
                .filter(|operation| {
                    matches!(operation.op, super::IrOp::Bytecode(Instruction::Await))
                })
                .count()
                == 1
    }));

    for (function, authored) in arrows.iter().zip([
        "async value => await value",
        "async (value) => { return await value; }",
    ]) {
        let start = source.find(authored).expect("authored async arrow");
        let range = function
            .source
            .range
            .as_ref()
            .expect("async arrow source range");
        assert_eq!(
            &source[function.source.span.start.byte_offset..function.source.span.end.byte_offset],
            "async"
        );
        assert_eq!(function.source.definition.as_usize(), start);
        assert_eq!(range.start.as_usize(), start);
        assert_eq!(range.end.as_usize(), start + authored.len());
    }

    let script = compile_unlinked_script(source).unwrap();
    let published = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|function| {
        function.metadata().function_kind == BytecodeFunctionKind::Async
            && !function.metadata().has_prototype
            && function.metadata().constructor_kind == ConstructorKind::None
    }));
}

#[test]
fn async_arrow_lexical_context_matches_quickjs_token_timing() {
    for source in [
        "const arrow = async value => await value;",
        "const arrow = async await => 42;",
        "'use strict'; const arrow = async await => 42;",
        r"const arrow = async \u0061wait => 42;",
        "const arrow = async yield => 42;",
        "const arrow = async () => { function nested(){ var await=1; return await; } return await 42; }; var await=1;",
        "function* outer(){ return async (value) => await value; }",
        "function* outer(){ return async () => yield; }",
        "function* outer(){ return async (value = (yield) => yield) => value; }",
        "function* outer(){ return async (value = (nested = (yield) => yield) => nested) => value; }",
        "async function outer(){ return (value = (await) => await) => value; }",
        "async function outer(){ return async (value = (nested = (await) => await) => nested) => value; }",
        "class C { static { (value = (await) => await) => value; } }",
        "class C { static { async (value = (nested = (await) => await) => nested) => value; } }",
        "function* outer(){ return (value = (nested = async yield => 1) => nested) => value; }",
        "async function outer(){ return (value = (nested = async await => 1) => nested) => value; }",
        "class C { static { (value = (nested = async await => 1) => nested) => value; } }",
        "async\nvalue => value",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("async arrow source rejected {source:?}: {error}"));
    }

    let newline_tree = Parser::parse(
        "async\nvalue => value",
        JsString::from_static("<async-arrow-newline-test>"),
    )
    .unwrap();
    let newline_arrow = newline_tree
        .functions
        .iter()
        .find(|function| function.kind == FunctionKind::Arrow)
        .expect("newline-separated ordinary arrow");
    assert_eq!(newline_arrow.execution_kind, BytecodeFunctionKind::Normal);
    assert_eq!(newline_arrow.source.span.start.byte_offset, "async\n".len());

    for (source, message) in [
        (
            "const arrow = async (await) => 42;",
            "missing formal parameter",
        ),
        (
            "const arrow = async (value = await 1) => value;",
            "await in default expression",
        ),
        (
            r"const arrow = async (\u0061wait) => 42;",
            "'await' is a reserved identifier",
        ),
        (
            "function* outer(){ return async (yield) => 42; }",
            "missing formal parameter",
        ),
        (
            "function* outer(){ return async yield => 0; }",
            "expecting ';'",
        ),
        (
            "function* outer(){ return (value = async yield => 1) => value; }",
            "expecting ','",
        ),
        (
            "function outer(){ 'use strict'; return async yield => 0; }",
            "expecting ';'",
        ),
        (
            "async function outer(){ return async await => 42; }",
            "expecting ';'",
        ),
        (
            "async function outer(){ return (value = async await => 1) => value; }",
            "expecting ','",
        ),
        (
            "class C { static { (value = async await => 1) => value; } }",
            "expecting ','",
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), message, "{source:?}");
    }
}

#[test]
fn async_function_lexical_context_and_async_generator_frontiers_match_quickjs() {
    for source in [
        "async function await(){}",
        "async function outer(){ function inner(){ var await=1; return await; } return await inner(); }",
        "async\nfunction ordinary(){}",
        "(async function(){ return await -1; })",
        "(async function(){ return (await 2) ** 3; })",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("async source rejected {source:?}: {error}"));
    }

    for (source, message) in [
        (
            "(async function await(){})",
            "'await' is a reserved identifier",
        ),
        (
            "async function f(value = await 1){}",
            "await in default expression",
        ),
        (
            "async function f(){ return await value ** 2; }",
            "unparenthesized unary expression can't appear on the left-hand side of '**'",
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), message, "{source:?}");
    }

    for source in ["async function* generator(){}", "(async function*(){})"] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("async-generator source rejected {source:?}: {error}"));
    }

    compile_unlinked_script("async function* generator(){ yield* source; }")
        .expect("async-generator delegation should use the typed async iterator protocol");

    for source in [
        "async function* generator(){ for (let value of source) { yield value; } }",
        "async function* generator(){ for (let value of source) { return value; } }",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("async-generator active iterator source rejected {source:?}: {error}")
        });
    }
}

#[test]
fn function_expression_await_name_preserves_quickjs_parent_token_asymmetry() {
    for source in [
        "async function outer(){ return function await(){}; }",
        "async function outer(){ return function* await(){}; }",
        "async function outer(){ return async function await(){}; }",
        "async function outer(value = function await(){}) { return value; }",
        "async function outer(){ 'use strict'; return async function await(){}; }",
        "async function await(){}",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("contextual await name rejected {source:?}: {error}"));
    }

    for source in [
        "(async function await(){})",
        "async function outer(){ function await(){} }",
        "async function outer(){ function* await(){} }",
        "async function outer(){ async function await(){} }",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }
}
