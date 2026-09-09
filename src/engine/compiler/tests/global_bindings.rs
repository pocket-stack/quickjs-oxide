use super::*;

#[test]
fn detached_vm_rejects_runtime_global_execution_explicitly() {
    let error = compile_script("answer").unwrap_err();
    assert!(error.message().contains("global-environment"));

    let error = compile_script("delete answer").unwrap_err();
    assert!(error.message().contains("global-environment"));
}

#[test]
fn runtime_global_get_and_direct_typeof_use_the_bytecode_realm() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let mut caller_context = runtime.new_context();
    let answer = runtime.intern_property_key("answer").unwrap();
    let marker = runtime.intern_property_key("marker").unwrap();
    let descriptor = |value| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        defining_context
            .define_own_property(
                &defining_context.global_object().unwrap(),
                &answer,
                &descriptor(Value::Int(1)),
            )
            .unwrap()
    );
    defining_context
        .create_global_lexical_for_test("answer", false, Some(Value::Int(2)))
        .unwrap();
    assert_eq!(defining_context.eval("answer").unwrap(), Value::Int(2));
    assert_eq!(
        defining_context.eval("typeof answer").unwrap(),
        Value::String(JsString::from_static("number"))
    );
    assert_eq!(
        defining_context.eval("typeof missingGlobal").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(
        defining_context.eval("typeof ((missingGlobal))").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert!(matches!(
        defining_context.eval("typeof (0, missingGlobal)"),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        defining_context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));

    let marker_object = defining_context.new_object().unwrap();
    assert!(
        defining_context
            .define_own_property(
                &defining_context.global_object().unwrap(),
                &marker,
                &descriptor(Value::Object(marker_object.clone())),
            )
            .unwrap()
    );
    let Value::Object(function) = defining_context
        .eval("(0, function(){ return marker; })")
        .unwrap()
    else {
        panic!("global-realm probe did not produce a function");
    };
    let callable = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller_context
            .call(&callable, Value::Undefined, &[])
            .unwrap(),
        Value::Object(marker_object)
    );

    assert!(matches!(
        caller_context.eval("missingGlobal"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("missing global did not materialize a ReferenceError");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert!(matches!(
        caller_context.get_property(&exception, &message).unwrap(),
        Value::String(value) if value == JsString::from_static("'missingGlobal' is not defined")
    ));
}

#[test]
fn global_put_matches_strict_sloppy_readonly_and_setter_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let descriptor = |value, writable| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(writable),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };

    assert_eq!(context.eval("created = 7").unwrap(), Value::Int(7));
    assert_eq!(context.eval("created").unwrap(), Value::Int(7));

    let readonly = runtime.intern_property_key("readonly").unwrap();
    assert!(
        context
            .define_own_property(&global, &readonly, &descriptor(Value::Int(1), false))
            .unwrap()
    );
    assert_eq!(context.eval("readonly = 2").unwrap(), Value::Int(2));
    assert_eq!(context.eval("readonly").unwrap(), Value::Int(1));
    assert!(matches!(
        context.eval("'use strict'; readonly = 2"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict read-only global assignment did not throw an object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'readonly' is read-only"))
    );
    assert_eq!(context.eval("readonly += 2").unwrap(), Value::Int(3));
    assert_eq!(context.eval("readonly").unwrap(), Value::Int(1));
    assert_eq!(
        context.eval("'use strict'; readonly ||= 9").unwrap(),
        Value::Int(1)
    );
    assert!(matches!(
        context.eval("'use strict'; readonly += 2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert!(matches!(
        context.eval("'use strict'; readonly &&= 2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    let inherited = runtime.intern_property_key("inheritedReadOnly").unwrap();
    assert!(
        context
            .define_own_property(
                &context.object_prototype().unwrap(),
                &inherited,
                &descriptor(Value::Int(5), false),
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("inheritedReadOnly = 6").unwrap(),
        Value::Int(6)
    );
    assert_eq!(context.eval("inheritedReadOnly").unwrap(), Value::Int(5));
    assert!(matches!(
        context.eval("'use strict'; inheritedReadOnly = 6"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict inherited read-only assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'inheritedReadOnly' is read-only"))
    );

    let no_setter = runtime.intern_property_key("noSetter").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &no_setter,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(context.eval("noSetter = 8").unwrap(), Value::Int(8));
    assert!(matches!(
        context.eval("'use strict'; noSetter = 8"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict setter-less assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("no setter for property"))
    );
    assert_eq!(context.eval("noSetter ||= 8").unwrap(), Value::Int(8));
    assert!(matches!(
        context.eval("'use strict'; noSetter ||= 8"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("'use strict'; noSetter &&= 8").unwrap(),
        Value::Undefined
    );

    assert!(matches!(
        context.eval("'use strict'; trulyMissing = 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    let truly_missing = runtime.intern_property_key("trulyMissing").unwrap();
    assert!(!runtime.has_own_property(&global, &truly_missing).unwrap());

    let sink = runtime.intern_property_key("sink").unwrap();
    assert!(
        context
            .define_own_property(&global, &sink, &descriptor(Value::Int(0), true))
            .unwrap()
    );
    let Value::Object(setter) = context
        .eval("(function(v) { sink = v; return 99; })")
        .unwrap()
    else {
        panic!("setter source did not produce a function");
    };
    let setter = runtime.as_callable(&setter).unwrap().unwrap();
    let target = runtime.intern_property_key("setterTarget").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &target,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(context.eval("setterTarget = 42").unwrap(), Value::Int(42));
    assert_eq!(context.eval("sink").unwrap(), Value::Int(42));
    assert_eq!(context.eval("setterTarget ||= 17").unwrap(), Value::Int(17));
    assert_eq!(context.eval("sink").unwrap(), Value::Int(17));

    let Value::Object(getter) = context.eval("(function() { return this; })").unwrap() else {
        panic!("getter source did not produce a function");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let getter_target = runtime.intern_property_key("getterTarget").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &getter_target,
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
    assert_eq!(
        context.eval("getterTarget").unwrap(),
        Value::Object(global.clone())
    );
    assert_eq!(
        context.eval("typeof getterTarget").unwrap(),
        Value::String(JsString::from_static("object"))
    );

    let Value::Object(throwing_getter) = context.eval("(function() { throw 17; })").unwrap() else {
        panic!("throwing getter source did not produce a function");
    };
    let throwing_getter = runtime.as_callable(&throwing_getter).unwrap().unwrap();
    let throwing_target = runtime.intern_property_key("throwingGetter").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &throwing_target,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(throwing_getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        context.eval("typeof throwingGetter"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));

    assert_eq!(context.eval("compoundSide = 0").unwrap(), Value::Int(0));
    assert!(matches!(
        context.eval("missingCompound += (compoundSide = 1)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(context.eval("compoundSide").unwrap(), Value::Int(0));
    assert!(matches!(
        context.eval("missingLogical ||= (compoundSide = 2)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(context.eval("compoundSide").unwrap(), Value::Int(0));
}

#[test]
fn global_lexical_tdz_const_shadow_and_initialization_share_the_resolved_cell() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let shadowed = runtime.intern_property_key("shadowed").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &shadowed,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(reader) = context.eval("(function() { return shadowed; })").unwrap() else {
        panic!("lexical reader source did not produce a function");
    };
    let reader = runtime.as_callable(&reader).unwrap().unwrap();

    context
        .create_global_lexical_for_test("shadowed", true, None)
        .unwrap();
    assert_eq!(context.eval("delete shadowed").unwrap(), Value::Bool(false));
    assert_eq!(
        context.get_property(&global, &shadowed).unwrap(),
        Value::Int(1)
    );
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1),
        "a precompiled ordinary global descriptor falls back to the global object while the later lexical is uninitialized"
    );

    context
        .initialize_global_lexical_for_test("shadowed", Value::Int(2))
        .unwrap();
    assert_eq!(
        context.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
    let Value::Object(writer) = context.eval("(function() { shadowed = 3; })").unwrap() else {
        panic!("lexical writer source did not produce a function");
    };
    let writer = runtime.as_callable(&writer).unwrap().unwrap();
    assert!(matches!(
        context.call(&writer, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert_eq!(
        context.get_property(&global, &shadowed).unwrap(),
        Value::Int(1)
    );
    assert_eq!(context.eval("shadowed ||= 9").unwrap(), Value::Int(2));
    assert!(matches!(
        context.eval("shadowed += 3"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed &= 3"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const bitwise compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed <<= 1"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const shift compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed **= 3"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const exponent compound assignment did not throw an object");
    };
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'shadowed' is read-only"))
    );
    assert!(matches!(
        context.eval("shadowed &&= 3"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    context
        .create_global_lexical_for_test("mutableLexical", false, None)
        .unwrap();
    assert_eq!(
        context.eval("typeof mutableLexical").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert!(matches!(
        context.eval("mutableLexical += 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert!(matches!(
        context.eval("mutableLexical |= 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert!(matches!(
        context.eval("mutableLexical **= 2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    context
        .initialize_global_lexical_for_test("mutableLexical", Value::Int(4))
        .unwrap();
    assert_eq!(context.eval("mutableLexical |= 8").unwrap(), Value::Int(12));
    assert_eq!(context.eval("mutableLexical ^= 3").unwrap(), Value::Int(15));
    assert_eq!(context.eval("mutableLexical &= 7").unwrap(), Value::Int(7));
    assert_eq!(context.eval("mutableLexical += 3").unwrap(), Value::Int(10));
    assert_eq!(context.eval("mutableLexical &&= 5").unwrap(), Value::Int(5));
    assert_eq!(context.eval("mutableLexical ??= 9").unwrap(), Value::Int(5));
    assert_eq!(
        context.eval("mutableLexical **= 2").unwrap(),
        Value::Int(25)
    );
    assert_eq!(context.eval("mutableLexical").unwrap(), Value::Int(25));

    context
        .create_global_lexical_for_test("mutableShift", false, None)
        .unwrap();
    assert!(matches!(
        context.eval("mutableShift >>>= 1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    context
        .initialize_global_lexical_for_test("mutableShift", Value::Int(-8))
        .unwrap();
    assert_eq!(context.eval("mutableShift >>= 1").unwrap(), Value::Int(-4));
    assert_eq!(
        context.eval("mutableShift >>>= 1").unwrap(),
        Value::Int(2_147_483_646)
    );
    assert_eq!(context.eval("mutableShift <<= 1").unwrap(), Value::Int(-4));
}

#[test]
fn unresolved_name_compiles_to_one_global_then_parent_global_relays() {
    let script = compile_unlinked_script(
        "(function() { return function() { return function() { return relayName; }; }; })",
    )
    .unwrap();
    let mut function = &script;
    for depth in 0..4 {
        let descriptor = function
            .closure_variables()
            .first()
            .expect("every function on the unresolved-name path needs a closure slot");
        assert_eq!(
            descriptor.source,
            if depth == 0 {
                ClosureSource::Global
            } else {
                ClosureSource::ParentGlobal(0)
            }
        );
        let ClosureVariableName::Constant(name_index) = descriptor.name else {
            panic!("unlinked global relay did not retain a name constant");
        };
        assert!(matches!(
            function.constants()[name_index as usize].as_primitive(),
            Some(crate::engine::value::PrimitiveValue::String(name)) if name == &JsString::from_static("relayName")
        ));
        if depth == 3 {
            assert!(matches!(
                function.code(),
                [Instruction::GetVar(0), Instruction::Return, ..]
            ));
            break;
        }
        function = function
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("global relay path lost its nested child");
    }
}

#[test]
fn late_global_property_delete_reconnect_and_cross_realm_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let Value::Object(reader) = defining
        .eval("(function() { return lateRealmValue; })")
        .unwrap()
    else {
        panic!("late global reader source did not produce a function");
    };
    let reader = runtime.as_callable(&reader).unwrap().unwrap();
    let key = runtime.intern_property_key("lateRealmValue").unwrap();
    let descriptor = |value| OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(value)),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        caller
            .define_own_property(&caller.global_object().unwrap(), &key, &descriptor(9),)
            .unwrap()
    );
    let defining_global = defining.global_object().unwrap();
    assert!(
        defining
            .define_own_property(&defining_global, &key, &descriptor(1))
            .unwrap()
    );
    runtime.run_gc().unwrap();
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1)
    );

    assert!(runtime.delete_property(&defining_global, &key).unwrap());
    runtime.run_gc().unwrap();
    assert!(matches!(
        caller.call(&reader, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller.take_exception().unwrap().unwrap() else {
        panic!("missing defining-realm global did not throw an object");
    };
    let reference_error = runtime.intern_property_key("ReferenceError").unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(reference_error) = defining
        .get_property(&defining_global, &reference_error)
        .unwrap()
    else {
        panic!("defining realm ReferenceError was not an object");
    };
    let Value::Object(reference_error_prototype) =
        defining.get_property(&reference_error, &prototype).unwrap()
    else {
        panic!("defining realm ReferenceError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(reference_error_prototype)
    );
    assert!(
        defining
            .define_own_property(&defining_global, &key, &descriptor(2))
            .unwrap()
    );
    runtime.run_gc().unwrap();
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
}
