use super::*;

#[test]
fn runtime_compiler_executes_anonymous_iife_parameters_and_direct_call() {
    let source = "(function(a, b) { return a + b; })(20, 22)";
    assert_eq!(evaluate_in_context(source), Value::Int(42));

    let detached_error = compile_script(source).unwrap_err();
    assert!(
        detached_error
            .message()
            .contains("requires runtime publication")
    );

    let script = compile_unlinked_script("(function(a, b) {})").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 2);
    assert_eq!(function.metadata().defined_argument_count, 2);
    assert!(function.metadata().has_prototype);
    assert_eq!(function.metadata().constructor_kind, ConstructorKind::Base);

    let runtime = Runtime::new();
    let Value::Object(function) = runtime.new_context().eval("(function() {})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(runtime.is_constructor(&function).unwrap());
}

#[test]
fn compiler_lowers_spread_calls_to_quickjs_apply_abis() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let root = context
        .compile("f(1,...xs,3); obj.m(...xs); eval(...xs); new C(...xs)")
        .unwrap();
    let code = runtime.test_function_code(&root).unwrap();

    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(instruction, Instruction::Apply(ApplyKind::Call)))
            .count(),
        2
    );
    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(instruction, Instruction::Apply(ApplyKind::Construct)))
            .count(),
        1
    );
    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(instruction, Instruction::ApplyEval { environment: 0 }))
            .count(),
        1
    );
    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(instruction, Instruction::ArrayFrom(_)))
            .count(),
        4
    );
    assert!(code.windows(5).any(|window| matches!(
        window,
        [
            Instruction::Append,
            Instruction::PushI32(3),
            Instruction::DefineArrayEl,
            Instruction::Inc,
            Instruction::Drop
        ]
    )));
    assert!(code.windows(4).any(|window| matches!(
        window,
        [
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Insert2,
            Instruction::Drop
        ]
    )));
    assert!(code.windows(2).any(|window| matches!(
        window,
        [Instruction::Perm3, Instruction::Apply(ApplyKind::Call)]
    )));
    assert!(code.windows(2).any(|window| matches!(
        window,
        [Instruction::Perm3, Instruction::Apply(ApplyKind::Construct)]
    )));
}

#[test]
fn runtime_executes_call_method_construct_and_direct_eval_spread() {
    assert_eq!(
        evaluate_in_context(
            r#"
            function list() { return Array.prototype.join.call(arguments, ","); }
            var object = { value: 7, add: function (amount) { return this.value + amount; } };
            function C(a, b) { this.value = a + b; }
            function direct() { let value = 1; eval(...["value = 42", "ignored"]); return value; }
            [
                list(0, ...[1, 2], 3, ...[], 4,),
                object.add(...[5]),
                new C(...[20, 22]).value,
                direct()
            ].join("|")
            "#,
        ),
        Value::String(JsString::from_static("0,1,2,3,4|12|42|42"))
    );
}

#[test]
fn runtime_append_preserves_quickjs_double_probe_and_fast_array_copy() {
    assert_eq!(
        evaluate_in_context(
            r#"
            (function () {
                var reads = 0;
                var captured;
                var source = [1, 2];
                Object.defineProperty(source, Symbol.iterator, {
                    get: function () {
                        reads++;
                        if (reads === 1) return Array.prototype.values;
                        return function () {
                            captured = [9].values();
                            return captured;
                        };
                    }
                });
                function list() { return Array.prototype.join.call(arguments, ","); }
                var copied = list(...source);
                return copied + "|" + reads + "|" + captured.next().value;
            })()
            "#,
        ),
        Value::String(JsString::from_static("1,2|2|9"))
    );
}

#[test]
fn var_initializer_named_evaluation_follows_quickjs_set_name_marker() {
    for source in [
        "(function() { var f = function() {}; return f; })()",
        "(function() { var f = (((function() {}))); return f; })()",
        "(function() { var \\u0066 = function() {}; return f; })()",
        "(function() { var f; f = function() {}; return f; })()",
    ] {
        assert_eq!(
            evaluate_function_name(source),
            (JsString::from_static("f"), false, false, true),
            "direct anonymous initializer should inherit the binding name: {source}"
        );
    }

    for source in [
        "(function() { return function() {}; })()",
        "(function() { var f = (0, function() {}); return f; })()",
        "(function() { var f = true ? function() {} : function() {}; return f; })()",
        "(function() { var f = 0 || function() {}; return f; })()",
    ] {
        assert_eq!(
            evaluate_function_name(source),
            (JsString::from_static(""), false, false, true),
            "non-AnonymousFunctionDefinition expression must keep an empty name: {source}"
        );
    }
}

#[test]
fn new_and_new_target_follow_quickjs_base_constructor_semantics() {
    assert_eq!(
        evaluate_in_context(
            "(function(){ var F = function(){ return this; }; return typeof new F(); })()"
        ),
        Value::String(JsString::from_static("object"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var F = function(){ return new.target; }; return new F() === F; })()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var marker = function(){}; var F = function(a){ return a; }; return new F(marker) === marker; })()"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){ var F = function(){ return 1; }; return typeof new F; })()"
        ),
        Value::String(JsString::from_static("object"))
    );
    assert_eq!(
        evaluate_in_context("(function(){ return new.target; })()"),
        Value::Undefined
    );

    let error = compile_unlinked_script("new.target").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "new.target only allowed within functions");

    for source in [
        r"(function(){ return new.\u0074arget; })()",
        r"(function(){ return new.t\u0061rget; })()",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "expecting target");
    }
}
