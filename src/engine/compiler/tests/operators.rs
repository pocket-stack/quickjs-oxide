use super::*;

#[test]
fn compiles_precedence_directly_to_stack_bytecode() {
    assert_eq!(evaluate("1 + 2 * 3"), Value::Int(7));
    assert_eq!(evaluate("(1 + 2) * 3"), Value::Int(9));
}

#[test]
fn bitwise_operators_follow_quickjs_precedence_and_numeric_semantics() {
    assert_eq!(evaluate("~0"), Value::Int(-1));
    assert_eq!(evaluate("~4294967296"), Value::Int(-1));
    assert_eq!(evaluate("-1.9 & 3.7"), Value::Int(3));
    assert_eq!(evaluate("'7' ^ true"), Value::Int(6));
    assert_eq!(evaluate("1 | 2 ^ 3 & 4"), Value::Int(3));
    assert_eq!(evaluate("1 | 2 === 3"), Value::Int(1));
    assert_eq!(evaluate("null ?? 1 | 2"), Value::Int(3));
    assert_eq!(evaluate("0 || 1 | 2"), Value::Int(3));

    assert_eq!(evaluate("~0n"), Value::BigInt(JsBigInt::from(-1)));
    assert_eq!(evaluate("-1n ^ 255n"), Value::BigInt(JsBigInt::from(-256)));
    assert_eq!(
        evaluate("123456789012345678901234567890n & -1n"),
        Value::BigInt(JsBigInt::parse_js_string("123456789012345678901234567890").unwrap())
    );
}

#[test]
fn shift_operators_follow_quickjs_precedence_and_numeric_semantics() {
    assert_eq!(evaluate("1 << 3"), Value::Int(8));
    assert_eq!(evaluate("-8 >> 2"), Value::Int(-2));
    assert_eq!(evaluate("-1 >>> 0"), Value::Float(4_294_967_295.0));
    assert_eq!(evaluate("1 << 33"), Value::Int(2));
    assert_eq!(evaluate("1 << -1"), Value::Int(i32::MIN));
    assert_eq!(evaluate("4294967295 >> 0"), Value::Int(-1));
    assert_eq!(evaluate("1 + 2 << 3"), Value::Int(24));
    assert_eq!(evaluate("16 >> 1 + 1"), Value::Int(4));
    assert_eq!(evaluate("1 << 2 < 5"), Value::Bool(true));
    assert_eq!(evaluate("8 >> 1 & 3"), Value::Int(0));
    assert_eq!(evaluate("64 >> 2 >> 1"), Value::Int(8));
    assert_eq!(evaluate("1 ?? 2 << 3"), Value::Int(1));

    assert_eq!(
        evaluate("1n << 65n"),
        Value::BigInt(JsBigInt::parse_js_string("36893488147419103232").unwrap())
    );
    assert_eq!(evaluate("-8n >> 2n"), Value::BigInt(JsBigInt::from(-2)));
    assert_eq!(evaluate("8n << -1n"), Value::BigInt(JsBigInt::from(4)));
    assert_eq!(evaluate("8n >> -2n"), Value::BigInt(JsBigInt::from(32)));
}

#[test]
fn exponentiation_follows_quickjs_precedence_associativity_and_unary_rules() {
    assert_eq!(evaluate("2 ** 3 ** 2"), Value::Int(512));
    assert_eq!(evaluate("2 * 3 ** 2"), Value::Int(18));
    assert_eq!(evaluate("2 ** 3 * 4"), Value::Int(32));
    assert_eq!(evaluate("2 ** -2"), Value::Float(0.25));
    assert_eq!(evaluate("(-2) ** 2"), Value::Int(4));
    assert!(evaluate("(typeof 2) ** 2").as_number().unwrap().is_nan());

    assert_eq!(evaluate("0n ** 0n"), Value::BigInt(JsBigInt::one()));
    assert_eq!(evaluate("(-2n) ** 3n"), Value::BigInt(JsBigInt::from(-8)));
    assert_eq!(
        evaluate("2n ** 100n"),
        Value::BigInt(JsBigInt::parse_js_string("1267650600228229401496703205376").unwrap())
    );

    for source in [
        "-2 ** 2",
        "+2 ** 2",
        "!2 ** 2",
        "~2 ** 2",
        "typeof 2 ** 2",
        "void 2 ** 2",
        "delete Function ** 2",
        "2 ** -2 ** 3",
    ] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "unparenthesized unary expression can't appear on the left-hand side of '**'",
            "source {source:?}"
        );
    }
}

#[test]
fn update_expressions_follow_quickjs_lvalue_and_power_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval("(function(){ var x = '01'; var old = x++; return old + '|' + x; })()")
            .unwrap(),
        Value::String(JsString::from_static("1|2"))
    );
    assert_eq!(
        context
            .eval("(function(){ var x = 2; return (++x ** 2) * 100 + (x++ ** 2) * 10 + x; })()")
            .unwrap(),
        Value::Int(994)
    );
    assert_eq!(
        context
            .eval("(function(){ var x = 4n; var old = x--; return old * 10n + --x; })()")
            .unwrap(),
        Value::BigInt(JsBigInt::from(42))
    );
    assert_eq!(
        context
            .eval("(function(){ Function.update = '4'; var old = Function.update++; return old + '|' + ++Function.update; })()")
            .unwrap(),
        Value::String(JsString::from_static("4|6"))
    );
    assert_eq!(
        context
            .eval("(function(){ Function['update'] = 5; var old = Function['update']--; return old * 10 + Function.update; })()")
            .unwrap(),
        Value::Int(54)
    );
    assert_eq!(
        context
            .eval("(function(){ var x = 1, y = 2; x\n++y; return x * 10 + y; })()")
            .unwrap(),
        Value::Int(13)
    );

    let prefix_argument = context
        .compile("(function(value){ return ++value; })")
        .unwrap();
    let prefix_argument = runtime
        .test_child_function_bytecode(&prefix_argument, 0)
        .unwrap();
    let prefix_code = runtime.test_function_code(&prefix_argument).unwrap();
    assert!(prefix_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::GetArg(0),
            Instruction::Inc,
            Instruction::SetArg(0)
        ]
    )));

    let postfix_argument = context
        .compile("(function(value){ return value++; })")
        .unwrap();
    let postfix_argument = runtime
        .test_child_function_bytecode(&postfix_argument, 0)
        .unwrap();
    let postfix_code = runtime.test_function_code(&postfix_argument).unwrap();
    assert!(postfix_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::GetArg(0),
            Instruction::PostInc,
            Instruction::PutArg(0)
        ]
    )));

    let fixed = context.compile("Function.update++").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(4).any(|window| matches!(
        window,
        [
            Instruction::GetField2(_),
            Instruction::PostInc,
            Instruction::Perm3,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("--Function['update']").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(4).any(|window| matches!(
        window,
        [
            Instruction::GetArrayEl3,
            Instruction::Dec,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    for source in ["++1", "1++", "++(1 + 2)", "(1 + 2)--"] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(error.message(), "invalid increment/decrement operand");
    }
    for source in ["'use strict'; ++eval", "'use strict'; arguments--"] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(error.message(), "invalid lvalue in strict mode");
    }
}

#[test]
fn compiles_primitive_coercion_and_equality() {
    assert_eq!(
        evaluate("'answer: ' + 42"),
        Value::String(JsString::from_static("answer: 42"))
    );
    assert_eq!(evaluate("'42' == 42"), Value::Bool(true));
    assert_eq!(evaluate("'42' === 42"), Value::Bool(false));
}

#[test]
fn compiles_short_circuit_and_conditional_control_flow() {
    assert_eq!(evaluate("false && 42"), Value::Bool(false));
    assert_eq!(
        evaluate("'left' || 'right'"),
        Value::String(JsString::from_static("left"))
    );
    assert_eq!(evaluate("false ? 1 : 2"), Value::Int(2));
    assert!(compile_script("true ? 1, 2 : 3").is_err());
    assert_eq!(evaluate("true ? 1 : 2, 3"), Value::Int(3));
}

#[test]
fn nullish_coalescing_uses_one_quickjs_short_circuit_join() {
    assert_eq!(evaluate("null ?? 42"), Value::Int(42));
    assert_eq!(evaluate("void 0 ?? 7"), Value::Int(7));
    assert_eq!(evaluate("false ?? true"), Value::Bool(false));
    assert_eq!(evaluate("-0 ?? 1"), Value::Float(-0.0));
    assert_eq!(
        evaluate("'' ?? 'fallback'"),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(evaluate("null ?? void 0 ?? 9"), Value::Int(9));
    assert_eq!(evaluate("null ?? 1 + 2 * 3"), Value::Int(7));
    assert_eq!(evaluate("0 ?? 1 ? 2 : 3"), Value::Int(3));

    let chain = compile_script("null ?? void 0 ?? 9").unwrap();
    let targets = chain
        .code
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::IfFalse(target) => Some(*target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0], targets[1]);
    let join = usize::try_from(targets[0]).unwrap();
    assert!(matches!(chain.code[join], Instruction::PutLocal(0)));
    assert!(matches!(chain.code[join + 1], Instruction::GetLocal(0)));
    assert!(matches!(chain.code[join + 2], Instruction::Return));

    for source in ["1 || 2 ?? 3", "1 && 2 ?? 3", "1 ?? 2 || 3", "1 ?? 2 && 3"] {
        assert!(compile_script(source).is_err(), "accepted {source:?}");
    }
    assert_eq!(evaluate("(false || 4) ?? 5"), Value::Int(4));
    assert_eq!(evaluate("null ?? (false || 6)"), Value::Int(6));
    assert_eq!(evaluate("(null ?? 0) || 7"), Value::Int(7));
    assert_eq!(evaluate("false || (null ?? 8)"), Value::Int(8));

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let call = context
        .compile(
            "Function.coalesce = function(){ return this === Function; }; \
             (Function.coalesce ?? Function)()",
        )
        .unwrap();
    let call_code = runtime.test_function_code(&call).unwrap();
    assert!(
        call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Call(0)))
    );
    assert!(
        !call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod(_)))
    );
    assert_eq!(
        context
            .eval(
                "Function.coalesce = function(){ return this === Function; }; \
                 (Function.coalesce ?? Function)()"
            )
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("inferred = null ?? function(){}; inferred.name")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        context.eval("1 ?? missingNullishRhs").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context
            .eval("Function.combo = 0; Function.combo ||= null ?? 4")
            .unwrap(),
        Value::Int(4)
    );
    assert_eq!(
        context
            .eval("Function.combo = null; Function.combo ??= void 0 ?? 5")
            .unwrap(),
        Value::Int(5)
    );
    assert!(
        context
            .compile("(Function.left ?? Function.right) = 1")
            .is_err()
    );
}

#[test]
fn relational_membership_uses_runtime_object_protocols() {
    assert_eq!(
        evaluate_in_context("'prototype' in Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("'missingMembershipKey' in Function"),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context("'toString' in Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("Function instanceof Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("(function(){}) instanceof Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("1 instanceof Function"),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context("(function(){}).bind(null) instanceof Function"),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var target=function DeepTarget(){}; var bound=target; for(var i=0;i<512;i++) bound=bound.bind(null); return 1 instanceof bound; })()"
        ),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var result=false; for((result='prototype' in Function);false;); return result; })()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var result=false; for(result=Function instanceof Function;false;); return result; })()"
        ),
        Value::Bool(true)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            "Function.membershipTrace=''; Function[Symbol.toPrimitive]=function(hint){ Function.membershipTrace+=hint; return 'prototype'; };",
        )
        .unwrap();
    assert!(matches!(
        context.eval("Function in (Function.membershipTrace+='R',1)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("Function.membershipTrace").unwrap(),
        Value::String(JsString::from_static("R"))
    );
    assert_eq!(
        context.eval("Function in Function").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("Function.membershipTrace").unwrap(),
        Value::String(JsString::from_static("Rstring"))
    );
}

#[test]
fn untagged_templates_follow_quickjs_concat_lowering() {
    assert_eq!(
        evaluate("`plain`"),
        Value::String(JsString::from_static("plain"))
    );
    assert_eq!(
        evaluate_in_context("`a${1 + 2}b${4}c`"),
        Value::String(JsString::from_static("a3b4c"))
    );
    assert_eq!(
        evaluate_in_context("`a${1, 2}b`"),
        Value::String(JsString::from_static("a2b"))
    );
    assert_eq!(
        evaluate_in_context("`a${`b${1}c`}d`"),
        Value::String(JsString::from_static("ab1cd"))
    );
    assert_eq!(evaluate_in_context("`x${8 / 2}y`.length"), Value::Int(3));

    let no_substitution = compile_script("`plain`").unwrap();
    assert!(!no_substitution.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::GetField2(_) | Instruction::CallMethod(_)
    )));

    let interpolated = compile_unlinked_script("`a${1}b${2}c`").unwrap();
    assert!(
        interpolated
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
    );
    assert!(
        interpolated
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod(4)))
    );

    let invalid = compile_script("`\\8`").unwrap_err();
    assert_eq!(
        invalid.message(),
        "malformed escape sequence in string literal"
    );
    assert_eq!(
        compile_script("0`x`").unwrap_err().message(),
        "tagged template objects require runtime publication; use Context::compile or Context::eval"
    );
    assert_eq!(
        compile_script("0\n`x`").unwrap_err().message(),
        "tagged template objects require runtime publication; use Context::compile or Context::eval"
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .compile("(function tag(strings) { return strings[0]; })`x`")
        .expect("runtime publication should materialize tagged-template objects");
}
