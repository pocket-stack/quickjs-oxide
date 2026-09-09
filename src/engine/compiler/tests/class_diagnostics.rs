use super::*;

#[test]
fn class_field_early_error_diagnostics_match_quickjs() {
    for (source, message, column) in [
        (
            "class C { field = arguments; }",
            "'arguments' identifier is not allowed in class field initializer",
            19,
        ),
        (
            "class C { static { arguments; } }",
            "'arguments' identifier is not allowed in class field initializer",
            20,
        ),
        (
            "class C { field = () => super(); }",
            "super() is only valid in a derived class constructor",
            30,
        ),
        ("class C { constructor; }", "invalid field name", 22),
        ("class C { static prototype; }", "invalid method name", 27),
        (
            "class C { static async prototype; }",
            "invalid property name",
            33,
        ),
        (
            "class C { static *prototype; }",
            "invalid property name",
            28,
        ),
        (
            "class C { static get prototype; }",
            "invalid property name",
            31,
        ),
        (
            "class C { static set prototype; }",
            "invalid property name",
            31,
        ),
        ("class C { get field; }", "invalid property name", 20),
        (
            "class C { static prototype() {} }",
            "invalid method name",
            27,
        ),
        ("class C { *constructor() {} }", "invalid method name", 23),
        ("class C { #constructor; }", "invalid method name", 23),
        (
            "class C { static #constructor; }",
            "invalid method name",
            30,
        ),
        (
            "class C { get #constructor() {} }",
            "invalid method name",
            27,
        ),
        (
            "class C { static get #constructor() {} }",
            "invalid method name",
            34,
        ),
        (
            r"class C { static #constr\u0075ctor; }",
            "invalid method name",
            35,
        ),
        (
            "class C { #constructor # }",
            "invalid first character of private name",
            24,
        ),
        (
            r"class C { async \u0023m() {} }",
            "invalid property name",
            17,
        ),
        ("class C { async @() {} }", "invalid property name", 17),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error
            .span()
            .expect("class-field syntax error lost its span");
        assert_eq!(
            (span.start.line, span.start.column),
            (1, column),
            "{source}"
        );
    }
}

#[test]
fn private_field_early_error_diagnostics_match_quickjs() {
    let undefined = compile_unlinked_script("class C { m(){ return this.#missing; } }")
        .expect_err("an undeclared private name must be rejected");
    assert_eq!(undefined.kind(), ErrorKind::Syntax);
    assert_eq!(undefined.message(), "undefined private field '#missing'");
    assert_eq!(undefined.span(), None);

    for (source, message, column) in [
        (
            "class C { #x; m(o){ delete o.#x; } }",
            "cannot delete a private class field",
            32,
        ),
        (
            "class C { #x; m(){ #x; } }",
            "unexpected token in expression: '#x'",
            20,
        ),
        (
            r"class C { #x; m(){ #\u0078; } }",
            r"unexpected token in expression: '#\u0078'",
            20,
        ),
        (
            "class C { #x; #x; }",
            "private class field is already defined",
            17,
        ),
        (
            "class C { static #x; static #x; }",
            "private class field is already defined",
            31,
        ),
        (
            "class C { #x; m(){ #x in {} = 0; } }",
            "invalid assignment left-hand side",
            31,
        ),
        (
            "class C { #x; m(){ #x in {} += 0; } }",
            "invalid assignment left-hand side",
            32,
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error
            .span()
            .expect("private-field syntax error lost its span");
        assert_eq!(
            (span.start.line, span.start.column),
            (1, column),
            "{source}"
        );
    }
}

#[test]
fn invalid_assignment_diagnostics_advance_to_rhs_like_quickjs() {
    for (source, column) in [
        ("1 = 0", 5),
        ("f() += 0", 8),
        ("1 = @", 5),
        ("1 &&= 0", 7),
        ("f() ||= 0", 9),
        ("a?.b = 0", 8),
        ("a?.b += 0", 9),
        ("a?.b ??= 0", 10),
        ("class C { #x; m(){ #x in {} &&= 0; } }", 33),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid assignment left-hand side",
            "{source}"
        );
        let span = error
            .span()
            .expect("invalid-assignment syntax error lost its span");
        assert_eq!(
            (span.start.line, span.start.column),
            (1, column),
            "{source}"
        );
    }
}

#[test]
fn invalid_for_in_of_assignment_target_diagnostics_match_quickjs() {
    for source in ["for(f() in {}) {}", "for(f() of []) {}"] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid for in/of left hand-side",
            "{source}"
        );
        let span = error
            .span()
            .expect("invalid for-in/of target error lost its span");
        assert_eq!((span.start.line, span.start.column), (1, 9), "{source}");
    }

    let ordinary_assignment = compile_unlinked_script("f() = 1").unwrap_err();
    assert_eq!(ordinary_assignment.kind(), ErrorKind::Syntax);
    assert_eq!(
        ordinary_assignment.message(),
        "invalid assignment left-hand side"
    );
    let span = ordinary_assignment
        .span()
        .expect("invalid assignment error lost its span");
    assert_eq!((span.start.line, span.start.column), (1, 7));
}

#[test]
fn class_field_initializers_reset_async_lexing_to_the_normal_hidden_child() {
    let source = r#"
        var aw\u0061it = 40;
        async function build() {
            class Fields {
                instance = await + 1;
                static staticField = await + 2;
                #private = await + 3;
                static #staticPrivate = await + 4;
                arrow = () => await + 5;
                static escaped = aw\u0061it + 6;
                [await Promise.resolve("computed")] = 47;
                read() { return this.#private; }
                static read() { return this.#staticPrivate; }
            }
            return Fields;
        }
    "#;
    let tree = Parser::parse(source, JsString::from_static("<class-field-await-test>"))
        .expect("class field await IdentifierReferences should compile");

    let initializers = tree
        .functions
        .iter()
        .filter(|function| {
            matches!(
                function.class_initializer_kind,
                Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(initializers.len(), 2);
    assert!(initializers.iter().all(|function| {
        function.kind == FunctionKind::Method
            && function.execution_kind == BytecodeFunctionKind::Normal
            && function.strict
            && function
                .ops
                .iter()
                .all(|operation| !matches!(operation.op, super::IrOp::Bytecode(Instruction::Await)))
    }));

    let build = tree
        .functions
        .iter()
        .find(|function| {
            function.function_name.as_deref() == Some("build")
                && function.execution_kind == BytecodeFunctionKind::Async
        })
        .expect("outer async function");
    assert_eq!(
        build
            .ops
            .iter()
            .filter(|operation| matches!(operation.op, super::IrOp::Bytecode(Instruction::Await)))
            .count(),
        1,
        "the computed key should retain the outer async context"
    );

    let field_arrow = tree
        .functions
        .iter()
        .find(|function| function.kind == FunctionKind::Arrow)
        .expect("field initializer arrow");
    assert_eq!(field_arrow.execution_kind, BytecodeFunctionKind::Normal);
    assert!(
        field_arrow
            .ops
            .iter()
            .all(|operation| !matches!(operation.op, super::IrOp::Bytecode(Instruction::Await)))
    );
}

#[test]
fn class_field_await_context_diagnostics_keep_neighboring_boundaries() {
    for (source, expected) in [
        (
            "async function build(){ return class { [await] = 1; }; }",
            "unexpected token in expression: ']'",
        ),
        (
            "async function build(){ return class { field = await 1; }; }",
            "expecting ';'",
        ),
        (
            "async function build(){ return class { static { await; } }; }",
            "unexpected 'await' keyword",
        ),
        (
            "class Fields { static { await; } }",
            "unexpected 'await' keyword",
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), expected, "{source:?}");
    }
}

#[test]
fn class_static_block_diagnostic_precedence_matches_quickjs() {
    for (source, message, column) in [
        (
            "class C { static { (await => 0); } }",
            "unexpected 'await' keyword",
            21,
        ),
        (
            "class C { static { (class await {}); } }",
            "expecting '{'",
            27,
        ),
        ("class C { static { ({ await }); } }", "expecting ':'", 29),
        (
            "class C { static { await: 0; } }",
            "unexpected 'await' keyword",
            20,
        ),
        (
            "class C { static { class await {} } }",
            "class statement requires a name",
            26,
        ),
        (
            "class C { static { return; } }",
            "return in a static initializer block",
            20,
        ),
        (
            "class C { static { const await = 0; } }",
            "variable name expected",
            26,
        ),
        (
            "class C { static { function await() {} } }",
            "function name expected",
            29,
        ),
        (
            "class C { static { let await; } }",
            "variable name expected",
            24,
        ),
        (
            "class C { static { try {} catch (await) {} } }",
            "identifier expected",
            34,
        ),
        (
            "class C { static { var [await] = []; } }",
            "invalid destructuring target",
            25,
        ),
        (
            "class C { static { var {await} = {}; } }",
            "invalid destructuring target",
            32,
        ),
        (
            "class C { static { var await; } }",
            "variable name expected",
            24,
        ),
        (
            "class C { static { await; } }",
            "unexpected 'await' keyword",
            20,
        ),
        (
            r"class C { static { ({ \u0061wait }); } }",
            "expecting ':'",
            34,
        ),
        (
            r"class C { static { try {} catch (\u0061wait) {} } }",
            "identifier expected",
            34,
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error
            .span()
            .expect("class-static-block syntax error lost its span");
        assert_eq!(
            (span.start.line, span.start.column),
            (1, column),
            "{source}"
        );
    }
}
