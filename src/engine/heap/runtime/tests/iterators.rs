use super::*;

#[test]
fn iterator_to_string_tag_accessor_matches_quickjs_metadata_and_setter_semantics() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let iterator_prototype = first.iterator_prototype().unwrap();
    let function_prototype = first.function_prototype().unwrap();
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(getter),
        set: Some(setter),
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&iterator_prototype, &tag)
        .unwrap()
        .unwrap()
    else {
        panic!("Iterator.prototype @@toStringTag was not the QuickJS accessor pair");
    };

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    for (callable, target, cproto, expected_name, expected_length) in [
        (
            &getter,
            NativeFunctionId::IteratorPrototypeToStringTagGetter,
            NativeCProto::Getter,
            "get [Symbol.toStringTag]",
            0,
        ),
        (
            &setter,
            NativeFunctionId::IteratorPrototypeToStringTagSetter,
            NativeCProto::Setter,
            "set [Symbol.toStringTag]",
            1,
        ),
    ] {
        assert_eq!(runtime.callable_realm(callable).unwrap(), first.realm);
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(function_prototype.clone())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            runtime
                .get_own_property(callable.as_object(), &prototype)
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime
                .get_own_property(callable.as_object(), &name)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::try_from_utf8(expected_name).unwrap()
        ));
        assert!(matches!(
            runtime
                .get_own_property(callable.as_object(), &length)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == expected_length
        ));
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(callable.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("Iterator tag accessor was not a native function");
        };
        assert_eq!(data.target, target);
        assert_eq!(data.target.descriptor().cproto, cproto);
    }

    for receiver in [
        Value::Null,
        Value::Int(1),
        Value::Object(first.new_object().unwrap()),
    ] {
        assert_eq!(
            second.call(&getter, receiver, &[]).unwrap(),
            Value::String(JsString::from_static("Iterator"))
        );
    }

    let iterator_global = runtime.intern_property_key("Iterator").unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(iterator_constructor),
        writable: true,
        enumerable: false,
        configurable: true,
    }) = runtime
        .get_own_property(&first.global_object().unwrap(), &iterator_global)
        .unwrap()
    else {
        panic!("global Iterator did not match the QuickJS property descriptor");
    };
    assert!(
        runtime
            .as_callable(&iterator_constructor)
            .unwrap()
            .is_some()
    );
    assert!(runtime.is_constructor(&iterator_constructor).unwrap());
    assert_eq!(
        runtime
            .get_own_property(&iterator_constructor, &prototype)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(iterator_prototype.clone()),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );

    let inherited = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(!runtime.has_own_property(&inherited, &tag).unwrap());
    assert!(
        first
            .set_property(
                &inherited,
                &tag,
                Value::String(JsString::from_static("Custom")),
            )
            .unwrap()
    );
    assert_eq!(
        runtime.get_own_property(&inherited, &tag).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(JsString::from_static("Custom")),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );

    let existing = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(
        runtime
            .define_own_property(
                &existing,
                &tag,
                &data_descriptor(
                    Value::String(JsString::from_static("old")),
                    true,
                    false,
                    false,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        first
            .call(
                &setter,
                Value::Object(existing.clone()),
                &[Value::String(JsString::from_static("new"))],
            )
            .unwrap(),
        Value::Undefined
    );
    assert_eq!(
        runtime.get_own_property(&existing, &tag).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(JsString::from_static("new")),
            writable: true,
            enumerable: false,
            configurable: false,
        })
    );

    let seen = runtime.intern_property_key("iteratorTagSeen").unwrap();
    let first_global = first.global_object().unwrap();
    assert!(
        first
            .define_own_property(
                &first_global,
                &seen,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let recording_setter = eval_callable(
        &runtime,
        &mut first,
        "(function(value) { iteratorTagSeen = value; })",
    );
    let own_accessor = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(
        runtime
            .define_own_property(
                &own_accessor,
                &tag,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(recording_setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        first
            .call(
                &setter,
                Value::Object(own_accessor),
                &[Value::String(JsString::from_static("seen"))],
            )
            .unwrap(),
        Value::Undefined
    );
    assert_eq!(
        first.get_property(&first_global, &seen).unwrap(),
        Value::String(JsString::from_static("seen"))
    );

    let readonly = runtime.new_object(Some(&iterator_prototype)).unwrap();
    assert!(
        runtime
            .define_own_property(
                &readonly,
                &tag,
                &data_descriptor(
                    Value::String(JsString::from_static("fixed")),
                    false,
                    false,
                    false,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        first.call(
            &setter,
            Value::Object(readonly),
            &[Value::String(JsString::from_static("no"))],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut first),
        JsString::from_static("'Symbol.toStringTag' is read-only")
    );

    assert_eq!(
        first.set_property(
            &iterator_prototype,
            &tag,
            Value::String(JsString::from_static("no")),
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut first),
        JsString::from_static("Cannot assign to read only property")
    );

    assert_eq!(
        second.call(
            &setter,
            Value::Int(1),
            &[Value::String(JsString::from_static("no"))],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(type_error) = second.take_exception().unwrap().unwrap() else {
        panic!("primitive Iterator tag receiver did not throw an Error object");
    };
    let type_error_constructor = global_callable(&runtime, &mut first, "TypeError");
    let Value::Object(type_error_prototype) = first
        .get_property(type_error_constructor.as_object(), &prototype)
        .unwrap()
    else {
        panic!("TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(type_error_prototype),
        "native accessor errors must use the accessor's defining realm"
    );
}

#[test]
fn string_iterator_inherits_iterator_tag_after_own_tag_is_deleted() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let string_prototype = context.string_prototype().unwrap();
    let string_iterator_prototype = context.string_iterator_prototype().unwrap();
    let iterator_prototype = context.iterator_prototype().unwrap();
    let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert_eq!(
        own_key_names(&runtime, &iterator_prototype),
        [
            "drop",
            "filter",
            "flatMap",
            "map",
            "take",
            "every",
            "find",
            "forEach",
            "some",
            "reduce",
            "toArray",
            "constructor",
            "Symbol.iterator",
            "Symbol.toStringTag",
        ]
    );

    let Value::Object(method) = context.get_property(&string_prototype, &iterator).unwrap() else {
        panic!("String.prototype @@iterator was not an object");
    };
    let method = runtime.as_callable(&method).unwrap().unwrap();
    let Value::Object(string_iterator) = context
        .call(&method, Value::String(JsString::from_static("x")), &[])
        .unwrap()
    else {
        panic!("String.prototype @@iterator did not return an object");
    };
    let object_prototype = context.object_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    assert_eq!(
        context
            .call(
                &object_to_string,
                Value::Object(string_iterator.clone()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::from_static("[object String Iterator]"))
    );

    assert!(
        runtime
            .delete_property(&string_iterator_prototype, &tag)
            .unwrap()
    );
    assert_eq!(
        context.get_property(&string_iterator, &tag).unwrap(),
        Value::String(JsString::from_static("Iterator"))
    );
    assert_eq!(
        context
            .call(&object_to_string, Value::Object(string_iterator), &[],)
            .unwrap(),
        Value::String(JsString::from_static("[object Iterator]"))
    );
}

#[test]
fn iterator_from_wraps_a_string_when_its_iterator_method_is_missing() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval(
            r#"
            (function () {
                var originalIterator = String.prototype[Symbol.iterator];
                var originalNext = String.prototype.next;
                var originalReturn = String.prototype.return;
                try {
                    String.prototype[Symbol.iterator] = null;
                    var missingNext = Iterator.from("abc");
                    var missingNextThrows = false;
                    try {
                        missingNext.next();
                    } catch (error) {
                        missingNextThrows = error instanceof TypeError;
                    }

                    var nextThis;
                    var returnThis;
                    var returnArgc = -1;
                    var returnResult = { done: true, value: 42 };
                    String.prototype.next = function () {
                        "use strict";
                        nextThis = this;
                        return { done: false, value: 7 };
                    };
                    String.prototype.return = function () {
                        "use strict";
                        returnThis = this;
                        returnArgc = arguments.length;
                        return returnResult;
                    };
                    delete String.prototype[Symbol.iterator];

                    var wrapped = Iterator.from("abc");
                    var nextResult = wrapped.next();
                    var actualReturn = wrapped.return("ignored");
                    return missingNextThrows
                        && nextResult.done === false
                        && nextResult.value === 7
                        && nextThis === "abc"
                        && returnThis === "abc"
                        && returnArgc === 0
                        && actualReturn === returnResult;
                } finally {
                    String.prototype[Symbol.iterator] = originalIterator;
                    if (originalNext === undefined) {
                        delete String.prototype.next;
                    } else {
                        String.prototype.next = originalNext;
                    }
                    if (originalReturn === undefined) {
                        delete String.prototype.return;
                    } else {
                        String.prototype.return = originalReturn;
                    }
                }
            })()
            "#,
        )
        .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn iterator_close_skips_only_result_brand_check_for_pending_exception() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let primitive_return = eval_callable(&runtime, &mut context, "(function(){ return 1; })");
    let return_key = runtime.intern_property_key("return").unwrap();
    let iterator = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &iterator,
                &return_key,
                &data_descriptor(
                    Value::Object(primitive_return.as_object().clone()),
                    true,
                    true,
                    true,
                ),
            )
            .unwrap()
    );
    let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
    assert!(matches!(
        VmHost::iterator_close(&mut host, Value::Object(iterator.clone()), true).unwrap(),
        IteratorCloseOutcome::Closed
    ));
    assert!(matches!(
        VmHost::iterator_close(&mut host, Value::Object(iterator), false).unwrap(),
        IteratorCloseOutcome::Throw(Value::Object(_))
    ));

    let non_callable = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &non_callable,
                &return_key,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        VmHost::iterator_close(&mut host, Value::Object(non_callable), true).unwrap(),
        IteratorCloseOutcome::Throw(Value::Object(_))
    ));
}

#[test]
fn native_iterator_next_wraps_public_calls_but_for_of_consumes_raw_outcomes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let iterator_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let string_prototype = context.string_prototype().unwrap();
    let Value::Object(iterator_method) = context
        .get_property(&string_prototype, &iterator_key)
        .unwrap()
    else {
        panic!("String.prototype @@iterator was not an object");
    };
    let iterator_method = runtime.as_callable(&iterator_method).unwrap().unwrap();
    let Value::Object(iterator) = context
        .call(
            &iterator_method,
            Value::String(JsString::from_static("A")),
            &[],
        )
        .unwrap()
    else {
        panic!("String.prototype @@iterator did not return an object");
    };
    let next_key = runtime.intern_property_key("next").unwrap();
    let Value::Object(next) = context.get_property(&iterator, &next_key).unwrap() else {
        panic!("String Iterator next was not an object");
    };
    let next = runtime.as_callable(&next).unwrap().unwrap();
    {
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(next.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("String Iterator next was not a direct native function");
        };
        assert_eq!(data.target, NativeFunctionId::StringIteratorNext);
        assert_eq!(data.target.descriptor().cproto, NativeCProto::IteratorNext);
    }

    let before_public = runtime.0.state.borrow().iterator_result_allocations;
    let Value::Object(result) = context.call(&next, Value::Object(iterator), &[]).unwrap() else {
        panic!("ordinary String Iterator next call did not return an object");
    };
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_public + 1
    );
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    let value_key = runtime.intern_property_key("value").unwrap();
    let done_key = runtime.intern_property_key("done").unwrap();
    assert_eq!(
        runtime.get_own_property(&result, &value_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(JsString::from_static("A")),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );
    assert_eq!(
        runtime.get_own_property(&result, &done_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Bool(false),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );

    let before_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval("(function(){var s='';for(var value of 'ab')s+=value;return s})()")
            .unwrap(),
        Value::String(JsString::from_static("ab"))
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_raw,
        "direct native ForOfNext must not allocate iterator-result wrappers"
    );

    let before_array_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval("(function(){var sum=0;for(var value of [20,22])sum+=value;return sum})()")
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_array_raw,
        "direct Array Iterator next must use the raw native ABI"
    );

    let before_concat_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval(
                "(function(){var sum=0;for(var value of Iterator.concat([20],[22]))sum+=value;return sum})()",
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_concat_raw,
        "direct Iterator Concat next and its inner built-ins must use the raw native ABI"
    );

    let before_bound = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        context
            .eval(
                "(function(){var iterator='z'[Symbol.iterator]();iterator.next=iterator.next.bind(iterator);function Iterable(){};Iterable.prototype[Symbol.iterator]=function(){return iterator};var s='';for(var value of new Iterable)s+=value;return s})()",
            )
            .unwrap(),
        Value::String(JsString::from_static("z"))
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_bound + 2,
        "a bound iterator-next method must retain generic call wrapping"
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn native_iterator_next_raw_dispatch_keeps_the_outer_operation_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let iterator_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let next_key = runtime.intern_property_key("next").unwrap();
    let string_prototype = defining.string_prototype().unwrap();
    let Value::Object(iterator_method) = defining
        .get_property(&string_prototype, &iterator_key)
        .unwrap()
    else {
        panic!("defining String.prototype @@iterator was not an object");
    };
    let iterator_method = runtime.as_callable(&iterator_method).unwrap().unwrap();
    let Value::Object(foreign_iterator) = caller
        .call(
            &iterator_method,
            Value::String(JsString::from_static("xy")),
            &[],
        )
        .unwrap()
    else {
        panic!("cross-realm String iterator creation did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&foreign_iterator).unwrap(),
        Some(defining.string_iterator_prototype().unwrap())
    );
    let Value::Object(foreign_next) = defining.get_property(&foreign_iterator, &next_key).unwrap()
    else {
        panic!("cross-realm String Iterator next was not an object");
    };

    let foreign_iterator_key = runtime.intern_property_key("foreignIterator").unwrap();
    let foreign_next_key = runtime.intern_property_key("foreignNext").unwrap();
    let caller_global = caller.global_object().unwrap();
    for (key, value) in [
        (
            &foreign_iterator_key,
            Value::Object(foreign_iterator.clone()),
        ),
        (&foreign_next_key, Value::Object(foreign_next.clone())),
    ] {
        assert!(
            caller
                .define_own_property(
                    &caller_global,
                    key,
                    &data_descriptor(value, true, true, true),
                )
                .unwrap()
        );
    }

    let before_raw = runtime.0.state.borrow().iterator_result_allocations;
    assert_eq!(
        caller
            .eval("(function(){var s='';for(var value of foreignIterator)s+=value;return s})()",)
            .unwrap(),
        Value::String(JsString::from_static("xy"))
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_raw
    );

    let Value::Object(error) = caller
        .eval(
            "(function(){function Invalid(){};Invalid.prototype[Symbol.iterator]=function(){return this};Invalid.prototype.next=foreignNext;try{for(var value of new Invalid)value}catch(error){return error}})()",
        )
        .unwrap()
    else {
        panic!("wrong-brand raw iterator-next call did not return its caught Error");
    };
    let type_error = global_callable(&runtime, &mut caller, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(type_error_prototype) = caller
        .get_property(type_error.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(type_error_prototype),
        "QuickJS's direct native iterator-next path keeps the outer operation realm"
    );
    assert_eq!(
        runtime.0.state.borrow().iterator_result_allocations,
        before_raw
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}
