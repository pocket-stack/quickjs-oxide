use super::*;

#[test]
fn source_members_preserve_quickjs_reads_keys_references_and_method_receivers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let data = |value| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };

    let base = context.new_object().unwrap();
    for (name, value) in [("x", Value::Int(7)), ("default", Value::Int(8))] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(&base, &key, &data(value))
                .unwrap()
        );
    }
    let base_name = runtime.intern_property_key("base").unwrap();
    assert!(
        context
            .define_own_property(&global, &base_name, &data(Value::Object(base.clone())))
            .unwrap()
    );

    let Value::Object(method) = context
        .eval("(function(){ return this === base; })")
        .unwrap()
    else {
        panic!("method source did not produce a function");
    };
    let method_key = runtime.intern_property_key("m").unwrap();
    assert!(
        context
            .define_own_property(&base, &method_key, &data(Value::Object(method)))
            .unwrap()
    );

    let Value::Object(getter) = context.eval("(function(){ return this; })").unwrap() else {
        panic!("getter source did not produce a function");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let getter_key = runtime.intern_property_key("receiver").unwrap();
    assert!(
        context
            .define_own_property(
                &base,
                &getter_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    assert_eq!(context.eval("base.x").unwrap(), Value::Int(7));
    assert_eq!(context.eval("base['x']").unwrap(), Value::Int(7));
    assert_eq!(context.eval("base.default").unwrap(), Value::Int(8));
    assert_eq!(context.eval("base\n.x").unwrap(), Value::Int(7));
    assert_eq!(context.eval("base\n['x']").unwrap(), Value::Int(7));
    assert_eq!(
        context.eval("base.receiver === base").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(context.eval("base.m()").unwrap(), Value::Bool(true));
    assert_eq!(context.eval("base['m']()").unwrap(), Value::Bool(true));
    assert_eq!(context.eval("((base.m))()").unwrap(), Value::Bool(true));
    assert_eq!(context.eval("(0, base.m)()").unwrap(), Value::Bool(false));
    assert_eq!(
        context.eval("(true ? base.m : base.m)()").unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context.eval("(true && base.m)()").unwrap(),
        Value::Bool(false)
    );

    let hint = runtime.intern_property_key("keyHint").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &hint,
                &data(Value::String(JsString::from_static("none"))),
            )
            .unwrap()
    );
    let Value::Object(to_key) = context
        .eval("(function(hint){ keyHint = hint; return 'x'; })")
        .unwrap()
    else {
        panic!("ToPropertyKey source did not produce a function");
    };
    let key_object = context.new_object().unwrap();
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    assert!(
        context
            .define_own_property(&key_object, &to_primitive, &data(Value::Object(to_key)))
            .unwrap()
    );
    let key_name = runtime.intern_property_key("keyObject").unwrap();
    assert!(
        context
            .define_own_property(&global, &key_name, &data(Value::Object(key_object)),)
            .unwrap()
    );
    assert_eq!(context.eval("base[keyObject]").unwrap(), Value::Int(7));
    assert_eq!(
        context.eval("keyHint").unwrap(),
        Value::String(JsString::from_static("string"))
    );
    assert_eq!(
        context.eval("keyHint = 'none'").unwrap(),
        Value::String(JsString::from_static("none"))
    );
    assert!(matches!(
        context.eval("null[keyObject]"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("keyHint").unwrap(),
        Value::String(JsString::from_static("none"))
    );

    assert_eq!(context.eval("'abc'.length").unwrap(), Value::Int(3));
    assert_eq!(
        context.eval("'abc'[1]").unwrap(),
        Value::String(JsString::from_static("b"))
    );
    assert!(matches!(
        context.eval("Function().toString()"),
        Ok(Value::String(_))
    ));
}

#[test]
fn member_assignment_and_delete_lower_through_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.fixed = 1").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(
        fixed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert2, Instruction::PutField(_)]))
    );
    assert!(
        !fixed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField(_)))
    );

    let computed = context.compile("Function['computed'] = 2").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(
        computed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert3, Instruction::PutArrayEl]))
    );
    assert!(
        !computed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArrayEl))
    );

    let fixed_compound = context.compile("Function.fixed += 3").unwrap();
    let fixed_compound_code = runtime.test_function_code(&fixed_compound).unwrap();
    assert!(
        fixed_compound_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
    );
    assert!(fixed_compound_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Add,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed_compound = context.compile("Function['computed'] += 4").unwrap();
    let computed_compound_code = runtime.test_function_code(&computed_compound).unwrap();
    assert!(
        computed_compound_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArrayEl3))
    );
    assert!(computed_compound_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Add,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let fixed_delete = context.compile("delete Function.fixed").unwrap();
    let fixed_delete_code = runtime.test_function_code(&fixed_delete).unwrap();
    assert!(
        fixed_delete_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Delete))
    );
    assert!(
        !fixed_delete_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField(_)))
    );

    assert_eq!(
        context
            .eval("Function.paren = 1; (Function.paren) = 2; Function.paren")
            .unwrap(),
        Value::Int(2)
    );
    assert!(context.compile("(0, Function.fixed) = 1").is_err());
    assert!(
        context
            .compile("(true ? Function.fixed : Function.fixed) = 1")
            .is_err()
    );

    assert_eq!(
        context
            .eval("Function.keep = 3; delete (0, Function.keep); Function.keep")
            .unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        context
            .eval("Function.gone = 4; delete (Function.gone); Function.gone")
            .unwrap(),
        Value::Undefined
    );
    assert!(context.compile("'use strict'; delete Function").is_err());
    let direct_delete = context.compile("delete __qjo_delete_global").unwrap();
    assert!(
        runtime
            .test_function_code(&direct_delete)
            .unwrap()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DeleteVar(0)))
    );
}

#[test]
fn direct_identifier_delete_uses_quickjs_scope_resolution() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context.eval("delete __qjo_missing_delete").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 1; return delete value; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(value){ return delete value; })(1)")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(value){ return (function(){ return delete value; })(); })(1)")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function named(){ return delete named; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(){ return delete arguments; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 1; return delete (value); })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 1; return delete (0, value); })()")
            .unwrap(),
        Value::Bool(true)
    );

    assert_eq!(
        context
            .eval("__qjo_delete_global = 1; delete __qjo_delete_global")
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("typeof __qjo_delete_global").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );

    let Value::Object(reader) = context
        .eval("(function(){ return __qjo_delete_reconnect; })")
        .unwrap()
    else {
        panic!("delete/reconnect probe did not produce a function");
    };
    let reader = runtime.as_callable(&reader).unwrap().unwrap();
    assert_eq!(
        context.eval("__qjo_delete_reconnect = 1").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("delete __qjo_delete_reconnect").unwrap(),
        Value::Bool(true)
    );
    assert!(matches!(
        context.call(&reader, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    assert!(context.take_exception().unwrap().is_some());
    assert_eq!(
        context.eval("__qjo_delete_reconnect = 2").unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );

    for name in ["undefined", "NaN", "Infinity"] {
        assert_eq!(
            context.eval(&format!("delete {name}")).unwrap(),
            Value::Bool(false),
            "global constant {name}"
        );
    }

    let mut caller = runtime.new_context();
    let realm_key = runtime.intern_property_key("__qjo_delete_realm").unwrap();
    let descriptor = OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(1)),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        context
            .define_own_property(&context.global_object().unwrap(), &realm_key, &descriptor)
            .unwrap()
    );
    assert!(
        caller
            .define_own_property(&caller.global_object().unwrap(), &realm_key, &descriptor)
            .unwrap()
    );
    let Value::Object(deleter) = context
        .eval("(function(){ return delete __qjo_delete_realm; })")
        .unwrap()
    else {
        panic!("cross-realm delete source did not produce a function");
    };
    let deleter = runtime.as_callable(&deleter).unwrap().unwrap();
    assert_eq!(
        caller.call(&deleter, Value::Undefined, &[]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !runtime
            .has_own_property(&context.global_object().unwrap(), &realm_key)
            .unwrap()
    );
    assert!(
        runtime
            .has_own_property(&caller.global_object().unwrap(), &realm_key)
            .unwrap()
    );

    let global = context.compile("delete __qjo_delete_opcode").unwrap();
    assert!(
        runtime
            .test_function_code(&global)
            .unwrap()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DeleteVar(0)))
    );
    let local_root = context
        .compile("(function(value){ return delete value; })")
        .unwrap();
    let local = runtime
        .test_child_function_bytecode(&local_root, 0)
        .unwrap();
    assert!(
        runtime
            .test_function_code(&local)
            .unwrap()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PushFalse))
    );

    for source in [
        "'use strict'; delete direct",
        "'use strict'; delete (direct)",
    ] {
        let error = compile_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "cannot delete a direct reference in strict mode"
        );
        assert_eq!(
            error.span().unwrap().start.column,
            u32::try_from(source.len() + 1).unwrap()
        );
    }
}

#[test]
fn bitwise_compound_assignment_reuses_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.bits &= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::BitAnd,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("Function['bits'] ^= 4").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::BitXor,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let identifier_root = context
        .compile("(function(value){ value |= 8; return value; })")
        .unwrap();
    let identifier = runtime
        .test_child_function_bytecode(&identifier_root, 0)
        .unwrap();
    let identifier_code = runtime.test_function_code(&identifier).unwrap();
    assert!(
        identifier_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitOr, Instruction::SetArg(0)]))
    );

    let local_root = context
        .compile("(function(){ var value = 7; value &= 3; return value; })")
        .unwrap();
    let local = runtime
        .test_child_function_bytecode(&local_root, 0)
        .unwrap();
    let local_code = runtime.test_function_code(&local).unwrap();
    assert!(
        local_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitAnd, Instruction::SetLocal(0)]))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value ^= 3; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure).unwrap();
    assert!(
        closure_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitXor, Instruction::SetVarRef(0)]))
    );

    let global = context.compile("__qjo_bit_global |= 8").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(
        global_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVar(_)))
    );
    assert!(global_code.windows(3).any(|window| matches!(
        window,
        [Instruction::BitOr, Instruction::Dup, Instruction::PutVar(_)]
    )));

    let sloppy_private_root = context
        .compile("(function named(){ named &= 1; return named; })")
        .unwrap();
    let sloppy_private = runtime
        .test_child_function_bytecode(&sloppy_private_root, 0)
        .unwrap();
    let sloppy_private_code = runtime.test_function_code(&sloppy_private).unwrap();
    assert!(
        sloppy_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitAnd, Instruction::Nop]))
    );

    let strict_private_root = context
        .compile("(function named(){ 'use strict'; named |= 1; })")
        .unwrap();
    let strict_private = runtime
        .test_child_function_bytecode(&strict_private_root, 0)
        .unwrap();
    let strict_private_code = runtime.test_function_code(&strict_private).unwrap();
    assert!(
        strict_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::BitOr, Instruction::ThrowReadOnly(_)]))
    );

    assert_eq!(
        context
            .eval(
                "(function(){ var value = 14; value &= 11; value ^= 3; \
                 value |= 4; return value; })()"
            )
            .unwrap(),
        Value::Int(13)
    );
    assert_eq!(
        context
            .eval("(function(value){ value |= 8; return value; })(1)")
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value ^= 3; \
                 return value; })(); })(5)"
            )
            .unwrap(),
        Value::Int(6)
    );
    assert_eq!(
        context
            .eval(
                "Function.bits = 14; Function.bits &= 11; \
                 Function['bits'] ^= 3; Function.bits |= 4"
            )
            .unwrap(),
        Value::Int(13)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var left = 1, right = 3; \
                 left |= right &= 2; return left * 10 + right; })()"
            )
            .unwrap(),
        Value::Int(32)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var value = -1n; (value) &= \
                 123456789012345678901234567890n; return value; })()"
            )
            .unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("123456789012345678901234567890").unwrap())
    );
    assert_eq!(
        context
            .eval("(function(value){ (value) |= 2; return value; })(1)")
            .unwrap(),
        Value::Int(3)
    );

    assert!(context.compile("(Function.bits) |= 1").is_ok());
    assert!(context.compile("(bitwiseIdentifier) &= 1").is_ok());
    assert!(context.compile("(0, Function.bits) |= 1").is_err());
    assert!(
        context
            .compile("(true ? Function.bits : Function.bits) |= 1")
            .is_err()
    );
    assert!(context.compile("(Function.bits & 1) |= 1").is_err());
}

#[test]
fn shift_compound_assignment_reuses_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.shift <<= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Shl,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("Function['shift'] >>= 2").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Sar,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let argument_root = context
        .compile("(function(value){ value >>>= 1; return value; })")
        .unwrap();
    let argument = runtime
        .test_child_function_bytecode(&argument_root, 0)
        .unwrap();
    let argument_code = runtime.test_function_code(&argument).unwrap();
    assert!(
        argument_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Shr, Instruction::SetArg(0)]))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value >>= 2; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure).unwrap();
    assert!(
        closure_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Sar, Instruction::SetVarRef(0)]))
    );

    let global = context.compile("__qjo_shift_global <<= 1").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(global_code.windows(3).any(|window| matches!(
        window,
        [Instruction::Shl, Instruction::Dup, Instruction::PutVar(_)]
    )));

    assert_eq!(
        context
            .eval(
                "(function(){ var value = 3; value <<= 2; value >>= 1; \
                 value >>>= 1; return value; })()"
            )
            .unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        context
            .eval(
                "Function.shift = -8; Function.shift >>= 1; \
                 Function['shift'] >>>= 1"
            )
            .unwrap(),
        Value::Int(2_147_483_646)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var left = 1, right = 3; \
                 left <<= right >>= 1; return left * 10 + right; })()"
            )
            .unwrap(),
        Value::Int(21)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value >>= 2; \
                 return value; })(); })(-8)"
            )
            .unwrap(),
        Value::Int(-2)
    );

    assert!(context.compile("(Function.shift) >>>= 1").is_ok());
    assert!(context.compile("(shiftIdentifier) <<= 1").is_ok());
    assert!(context.compile("(0, Function.shift) >>= 1").is_err());
    assert!(context.compile("(Function.shift << 1) >>= 1").is_err());
}

#[test]
fn exponent_compound_assignment_reuses_quickjs_lvalue_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.power **= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(fixed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Pow,
            Instruction::Insert2,
            Instruction::PutField(_)
        ]
    )));

    let computed = context.compile("Function['power'] **= 2").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(computed_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Pow,
            Instruction::Insert3,
            Instruction::PutArrayEl
        ]
    )));

    let argument_root = context
        .compile("(function(value){ value **= 3; return value; })")
        .unwrap();
    let argument = runtime
        .test_child_function_bytecode(&argument_root, 0)
        .unwrap();
    let argument_code = runtime.test_function_code(&argument).unwrap();
    assert!(
        argument_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::SetArg(0)]))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value **= 2; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure).unwrap();
    assert!(
        closure_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::SetVarRef(0)]))
    );

    let global = context.compile("__qjo_power_global **= 2").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(global_code.windows(3).any(|window| matches!(
        window,
        [Instruction::Pow, Instruction::Dup, Instruction::PutVar(_)]
    )));

    let sloppy_private_root = context
        .compile("(function named(){ named **= 1; return named; })")
        .unwrap();
    let sloppy_private = runtime
        .test_child_function_bytecode(&sloppy_private_root, 0)
        .unwrap();
    let sloppy_private_code = runtime.test_function_code(&sloppy_private).unwrap();
    assert!(
        sloppy_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::Nop]))
    );

    let strict_private_root = context
        .compile("(function named(){ 'use strict'; named **= 1; })")
        .unwrap();
    let strict_private = runtime
        .test_child_function_bytecode(&strict_private_root, 0)
        .unwrap();
    let strict_private_code = runtime.test_function_code(&strict_private).unwrap();
    assert!(
        strict_private_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Pow, Instruction::ThrowReadOnly(_)]))
    );

    assert_eq!(
        context
            .eval("(function(){ var value = 2; value **= 3; return value; })()")
            .unwrap(),
        Value::Int(8)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value **= 2; \
                 return value; })(); })(3)"
            )
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var left = 2, right = 3; \
                 left **= right **= 2; return left + right; })()"
            )
            .unwrap(),
        Value::Int(521)
    );
    assert_eq!(
        context
            .eval("(function(){ var value = 2n; (value) **= 100n; return value; })()")
            .unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("1267650600228229401496703205376").unwrap())
    );

    assert!(context.compile("(Function.power) **= 2").is_ok());
    assert!(context.compile("(powerIdentifier) **= 2").is_ok());
    assert!(context.compile("(0, Function.power) **= 2").is_err());
    assert!(context.compile("(Function.power ** 1) **= 2").is_err());
}

#[test]
fn logical_member_assignment_uses_quickjs_branch_cleanup_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let fixed = context.compile("Function.fixed &&= 3").unwrap();
    let fixed_code = runtime.test_function_code(&fixed).unwrap();
    assert!(
        fixed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
    );
    assert!(
        fixed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert2, Instruction::PutField(_)]))
    );
    let fixed_branch = fixed_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IfFalse(_)))
        .unwrap();
    let Instruction::IfFalse(fixed_short) = fixed_code[fixed_branch] else {
        unreachable!();
    };
    assert!(matches!(
        fixed_code[usize::try_from(fixed_short).unwrap()],
        Instruction::Nip
    ));
    assert_eq!(
        fixed_code
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Nip))
            .count(),
        1
    );

    let computed = context.compile("Function['computed'] ||= 4").unwrap();
    let computed_code = runtime.test_function_code(&computed).unwrap();
    assert!(
        computed_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArrayEl3))
    );
    assert!(
        computed_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Insert3, Instruction::PutArrayEl]))
    );
    let computed_branch = computed_code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IfTrue(_)))
        .unwrap();
    let Instruction::IfTrue(computed_short) = computed_code[computed_branch] else {
        unreachable!();
    };
    let computed_short = usize::try_from(computed_short).unwrap();
    assert!(matches!(computed_code[computed_short], Instruction::Nip));
    assert!(matches!(
        computed_code[computed_short + 1],
        Instruction::Nip
    ));
    assert_eq!(
        computed_code
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Nip))
            .count(),
        2
    );
    let computed_goto = computed_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::Goto(target) => Some(usize::try_from(*target).unwrap()),
            _ => None,
        })
        .unwrap();
    assert_eq!(computed_goto, computed_short + 2);

    let nullish = context.compile("Function.nullish ??= 5").unwrap();
    let nullish_code = runtime.test_function_code(&nullish).unwrap();
    assert!(nullish_code.windows(3).any(|window| matches!(
        window,
        [
            Instruction::Dup,
            Instruction::IsUndefinedOrNull,
            Instruction::IfFalse(_)
        ]
    )));

    assert_eq!(
        context
            .eval("Function.logic = 0; Function.logic ||= 7")
            .unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        context
            .eval("Function.logic = 0; Function.logic &&= 8")
            .unwrap(),
        Value::Int(0)
    );
    assert_eq!(
        context
            .eval("Function.logic = null; Function.logic ??= 9")
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval(
                "Function.left = 1; Function.right = 0; \
                 Function.left &&= Function.right ||= 9; \
                 Function.left + Function.right"
            )
            .unwrap(),
        Value::Int(18)
    );
    assert_eq!(
        context
            .eval(
                "Function.outer = 1; Function.inner = 0; \
                 Function['outer'] += (Function['inner'] ||= 2); \
                 Function.outer + Function.inner"
            )
            .unwrap(),
        Value::Int(5)
    );

    let logical_call = context
        .compile("(Function.callable ||= function(){ return this === Function; })()")
        .unwrap();
    let logical_call_code = runtime.test_function_code(&logical_call).unwrap();
    assert!(
        logical_call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Call(0)))
    );
    assert!(
        !logical_call_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod(_)))
    );
    assert_eq!(
        context
            .eval("(Function.callable ||= function(){ return this === Function; })()")
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .eval("delete Function.anon; (Function.anon ??= function(){}).name")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );

    assert!(context.compile("(Function.fixed) &&= 1").is_ok());
    assert!(context.compile("(0, Function.fixed) &&= 1").is_err());
    assert!(
        context
            .compile("(true ? Function.fixed : Function.fixed) &&= 1")
            .is_err()
    );
    assert!(
        context
            .compile("(Function.fixed || Function.computed) &&= 1")
            .is_err()
    );
}

#[test]
fn identifier_compound_assignment_uses_resolved_get_set_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let argument_root = context
        .compile("(function(value){ value += 2; return value; })")
        .unwrap();
    let argument = runtime
        .test_child_function_bytecode(&argument_root, 0)
        .unwrap();
    let argument_code = runtime.test_function_code(&argument).unwrap();
    assert!(
        argument_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetArg(0)))
    );
    assert!(
        argument_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetArg(0)))
    );

    let local_root = context
        .compile("(function(){ var value = 1; value ||= 4; return value; })")
        .unwrap();
    let local = runtime
        .test_child_function_bytecode(&local_root, 0)
        .unwrap();
    let local_code = runtime.test_function_code(&local).unwrap();
    assert!(
        local_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetLocal(0)))
    );
    assert!(
        local_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocal(0)))
    );
    let branch = local_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::IfTrue(target) => Some(usize::try_from(*target).unwrap()),
            _ => None,
        })
        .unwrap();
    let end = local_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::Goto(target) => Some(usize::try_from(*target).unwrap()),
            _ => None,
        })
        .unwrap();
    assert_eq!(branch, end);
    assert!(
        !local_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Nip))
    );

    let closure_root = context
        .compile("(function(value){ return function(){ value += 2; return value; }; })")
        .unwrap();
    let closure_outer = runtime
        .test_child_function_bytecode(&closure_root, 0)
        .unwrap();
    let closure_inner = runtime
        .test_child_function_bytecode(&closure_outer, 0)
        .unwrap();
    let closure_code = runtime.test_function_code(&closure_inner).unwrap();
    assert!(
        closure_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVarRef(0)))
    );
    assert!(
        closure_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetVarRef(0)))
    );

    let global = context.compile("identifierCompoundGlobal ||= 2").unwrap();
    let global_code = runtime.test_function_code(&global).unwrap();
    assert!(
        global_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetVar(_)))
    );
    assert!(
        global_code
            .windows(2)
            .any(|window| matches!(window, [Instruction::Dup, Instruction::PutVar(_)]))
    );
    let global_branch = global_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::IfTrue(target) => Some(*target),
            _ => None,
        })
        .unwrap();
    let global_end = global_code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::Goto(target) => Some(*target),
            _ => None,
        })
        .unwrap();
    assert_eq!(global_branch, global_end);

    let sloppy_self_root = context
        .compile("(function named(){ named += ''; return named; })")
        .unwrap();
    let sloppy_self = runtime
        .test_child_function_bytecode(&sloppy_self_root, 0)
        .unwrap();
    let sloppy_self_code = runtime.test_function_code(&sloppy_self).unwrap();
    assert!(
        sloppy_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Nop))
    );
    assert!(
        !sloppy_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocal(_)))
    );

    let strict_self_root = context
        .compile("(function named(){ 'use strict'; named &&= 1; })")
        .unwrap();
    let strict_self = runtime
        .test_child_function_bytecode(&strict_self_root, 0)
        .unwrap();
    let strict_self_code = runtime.test_function_code(&strict_self).unwrap();
    assert!(
        strict_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowReadOnly(_)))
    );
    assert!(
        !strict_self_code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetLocal(_)))
    );

    assert_eq!(
        context
            .eval("(function(value){ value += 2; return value; })(3)")
            .unwrap(),
        Value::Int(5)
    );
    assert_eq!(
        context
            .eval(
                "(function(){ var value = 20; value += 2; value -= 4; \
                 value *= 3; value /= 2; value %= 5; return value; })()"
            )
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context
            .eval(
                "(function(value){ return (function(){ value += 2; return value; })() \
                 + (function(){ value += 3; return value; })(); })(1)"
            )
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context
            .eval("identifierCompoundGlobal = 1; identifierCompoundGlobal += 2")
            .unwrap(),
        Value::Int(3)
    );

    for (source, expected) in [
        (
            "(function(value){ value &&= 9; return value; })(2)",
            Value::Int(9),
        ),
        (
            "(function(value){ value &&= 9; return value; })(0)",
            Value::Int(0),
        ),
        (
            "(function(value){ value ||= 9; return value; })(0)",
            Value::Int(9),
        ),
        (
            "(function(value){ value ||= 9; return value; })(2)",
            Value::Int(2),
        ),
        (
            "(function(value){ value ??= 9; return value; })(null)",
            Value::Int(9),
        ),
        (
            "(function(value){ value ??= 9; return value; })(false)",
            Value::Bool(false),
        ),
    ] {
        assert_eq!(context.eval(source).unwrap(), expected, "{source}");
    }

    assert_eq!(
        context
            .eval("(function(){ var named; named ??= function(){}; return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static("named"))
    );
    assert_eq!(
        context
            .eval("(function(){ var named; (named) ??= function(){}; return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        context
            .eval("(function(){ var named; (named = function(){}); return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static("named"))
    );
    assert_eq!(
        context
            .eval("(function(){ var named; (named) = function(){}; return named.name; })()")
            .unwrap(),
        Value::String(JsString::from_static(""))
    );

    assert_eq!(
        context
            .eval("(function(value){ (value) += 2; return value; })(3)")
            .unwrap(),
        Value::Int(5)
    );
    assert!(
        context
            .compile("(function(a,b){ (0, a) += 1; return a; })")
            .is_err()
    );
    assert!(
        context
            .compile("(function(a,b){ (true ? a : b) ||= 1; return a; })")
            .is_err()
    );
    assert!(
        context
            .compile("(function(){ 'use strict'; eval += 1; })")
            .is_err()
    );
    assert!(
        context
            .compile("(function(){ 'use strict'; (arguments) ??= 1; })")
            .is_err()
    );

    assert_eq!(
        context
            .eval(
                "(function named(){ var result = named += ''; \
                 return typeof result + '|' + typeof named; })()"
            )
            .unwrap(),
        Value::String(JsString::from_static("string|function"))
    );
    assert_eq!(
        context
            .eval(
                "(function(wrapper){ return wrapper() === wrapper; })(function named(){ \
                 return named ||= 1; })"
            )
            .unwrap(),
        Value::Bool(true)
    );
}
