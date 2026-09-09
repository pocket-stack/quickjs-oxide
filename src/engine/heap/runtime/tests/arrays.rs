use super::*;

#[test]
fn array_class_roots_length_layout_values_and_realm_prototype() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_prototype = first.array_prototype().unwrap();
    let second_prototype = second.array_prototype().unwrap();
    assert_ne!(first_prototype, second_prototype);
    assert!(runtime.is_array_object(&first_prototype).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&first_prototype).unwrap(),
        Some(first.object_prototype().unwrap())
    );

    let foreign_realm_value = second.new_object().unwrap();
    let array = first
        .new_array_from_values(vec![Value::Int(10), Value::Object(foreign_realm_value)])
        .unwrap();
    assert!(runtime.is_array_object(&array).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(first_prototype)
    );

    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        first.get_own_property(&array, &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: true,
            enumerable: false,
            configurable: false,
        })
    );
    let keys = runtime
        .own_property_keys(&array)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .0
                .state
                .borrow()
                .atoms
                .to_string(key.atom())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["0", "1", "length"]);
    let zero = runtime.intern_property_key("0").unwrap();
    assert_eq!(first.get_property(&array, &zero).unwrap(), Value::Int(10));
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(array.object_id()).unwrap();
        let ObjectPayload::Array { dense: Some(dense) } = &object.payload else {
            panic!("fresh Array did not retain its physical dense payload");
        };
        assert_eq!(dense.len(), 2);
        let shape = state.heap.shape(object.shape).unwrap();
        assert_eq!(shape.entries().len(), 1);
        assert!(
            state
                .atoms
                .array_index(shape.entries()[0].atom)
                .unwrap()
                .is_none()
        );
    }

    let other_runtime = Runtime::new();
    let mut other_context = other_runtime.new_context();
    let wrong_runtime_value = other_context.new_object().unwrap();
    assert!(matches!(
        first.new_array_from_values(vec![Value::Object(wrong_runtime_value)]),
        Err(RuntimeError::WrongRuntime("Array element"))
    ));
}

#[test]
fn array_dense_tail_delete_and_interior_delete_match_quickjs_conversion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array = context
        .new_array_from_values(vec![Value::Int(10), Value::Int(20), Value::Int(30)])
        .unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    let one = runtime.intern_property_key("1").unwrap();
    let two = runtime.intern_property_key("2").unwrap();

    assert!(runtime.delete_property(&array, &two).unwrap());
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(3)
    );
    assert!(!runtime.has_own_property(&array, &two).unwrap());
    assert_eq!(runtime.array_fast_len(&array).unwrap(), Some(2));

    assert!(context.set_property(&array, &two, Value::Int(31)).unwrap());
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(3)
    );
    assert_eq!(runtime.array_fast_len(&array).unwrap(), Some(3));

    assert!(runtime.delete_property(&array, &zero).unwrap());
    assert_eq!(runtime.array_fast_len(&array).unwrap(), None);
    assert!(!runtime.has_own_property(&array, &zero).unwrap());
    assert_eq!(context.get_property(&array, &one).unwrap(), Value::Int(20));
    assert_eq!(context.get_property(&array, &two).unwrap(), Value::Int(31));
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(3)
    );
}

#[test]
fn array_indices_grow_length_and_obey_readonly_and_extensible_state() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array = context.new_array().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let five = runtime.intern_property_key("5").unwrap();
    assert!(
        context
            .define_own_property(
                &array,
                &five,
                &data_descriptor(Value::Int(50), true, true, true)
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(6)
    );

    let non_index = runtime.intern_property_key("4294967295").unwrap();
    assert!(
        context
            .define_own_property(
                &array,
                &non_index,
                &data_descriptor(Value::Int(99), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(6)
    );

    assert!(
        context
            .define_own_property(
                &array,
                &length,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let six = runtime.intern_property_key("6").unwrap();
    assert!(
        !context
            .define_own_property(
                &array,
                &six,
                &data_descriptor(Value::Int(60), true, true, true)
            )
            .unwrap()
    );
    assert!(!context.set_property(&array, &six, Value::Int(60)).unwrap());
    assert!(context.set_property(&array, &five, Value::Int(51)).unwrap());
    assert_eq!(context.get_property(&array, &five).unwrap(), Value::Int(51));

    let extensible = context.new_array().unwrap();
    runtime.prevent_extensions(&extensible).unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    assert!(
        !context
            .define_own_property(
                &extensible,
                &zero,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&extensible, &length).unwrap(),
        Value::Int(0)
    );
}

#[test]
fn array_length_shrink_deletes_descending_and_rolls_back_at_fixed_index() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array = context
        .new_array_from_values((0..5).map(Value::Int).collect())
        .unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let fixed = runtime.intern_property_key("3").unwrap();
    assert!(
        context
            .define_own_property(
                &array,
                &fixed,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    assert!(
        !context
            .define_own_property(
                &array,
                &length,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.get_own_property(&array, &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(4),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );
    let four = runtime.intern_property_key("4").unwrap();
    assert!(!runtime.has_own_property(&array, &four).unwrap());
    assert!(runtime.has_own_property(&array, &fixed).unwrap());
    let two = runtime.intern_property_key("2").unwrap();
    assert!(runtime.has_own_property(&array, &two).unwrap());
    assert!(!context.set_property(&array, &four, Value::Int(44)).unwrap());
    assert!(context.set_property(&array, &two, Value::Int(22)).unwrap());
}

#[test]
fn array_assignment_rejections_keep_quickjs_length_diagnostics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let length = runtime.intern_property_key("length").unwrap();

    let read_only = context.new_array().unwrap();
    assert!(
        context
            .define_own_property(
                &read_only,
                &length,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let read_only_key = runtime.intern_property_key("readOnlyArray").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &read_only_key,
                &data_descriptor(Value::Object(read_only), true, true, true),
            )
            .unwrap()
    );
    for source in [
        "(function(){'use strict';readOnlyArray[0]=1})()",
        "(function(){'use strict';readOnlyArray.length=0})()",
    ] {
        assert_eq!(context.eval(source), Err(RuntimeError::Exception));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from_static("'length' is read-only")
        );
    }

    let fixed = context.new_array_from_values(vec![Value::Int(1)]).unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    assert!(
        context
            .define_own_property(
                &fixed,
                &zero,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let fixed_key = runtime.intern_property_key("fixedArray").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &fixed_key,
                &data_descriptor(Value::Object(fixed), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("(function(){'use strict';fixedArray.length=0})()"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not configurable")
    );
}

#[test]
fn array_join_separator_overflow_still_gets_nullish_slots_and_later_throw_wins() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(source) = context
        .eval(
            r#"(function(){
                var source=Object();globalThis.joinOverflowLog="";source.length=4;
                source[0]="a";
                source.__defineGetter__("1",function(){joinOverflowLog+="1";return null});
                source.__defineGetter__("2",function(){joinOverflowLog+="2";return undefined});
                source.__defineGetter__("3",function(){joinOverflowLog+="3";throw 77});
                return source;
            })()"#,
        )
        .unwrap()
    else {
        panic!("Array.join overflow fixture was not an object");
    };
    let completion = runtime
        .call_array_prototype_join_with_string_limit(
            context.realm,
            ArrayJoinKind::Join,
            crate::engine::vm::call::NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &crate::engine::vm::call::NativeArguments {
                actual_arg_count: 1,
                readable: vec![Value::String(JsString::from_static("xx"))],
            },
            2,
        )
        .unwrap();
    assert!(matches!(completion, Completion::Throw(Value::Int(77))));
    assert_eq!(
        context.eval("joinOverflowLog").unwrap(),
        Value::String(JsString::from_static("123"))
    );
}

#[test]
fn array_locale_separator_overflow_invokes_method_but_skips_result_to_string() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(source) = context
        .eval(
            r#"(function(){
                var source=Object(),element=Object();globalThis.localeOverflowLog="";
                source.length=2;source[0]="aa";source[1]=element;
                element.toLocaleString=function(){
                    var result=Object();localeOverflowLog+="M";
                    result.toString=function(){localeOverflowLog+="C";return "result"};
                    return result;
                };
                return source;
            })()"#,
        )
        .unwrap()
    else {
        panic!("Array.toLocaleString overflow fixture was not an object");
    };
    let error = runtime
        .call_array_prototype_join_with_string_limit(
            context.realm,
            ArrayJoinKind::ToLocaleString,
            crate::engine::vm::call::NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &crate::engine::vm::call::NativeArguments {
                actual_arg_count: 0,
                readable: Vec::new(),
            },
            2,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::Engine(ref error)
            if error.kind() == ErrorKind::JsInternal
                && error.message() == "string too long"
    ));
    assert_eq!(
        context.eval("localeOverflowLog").unwrap(),
        Value::String(JsString::from_static("M"))
    );
}

#[test]
fn array_locale_method_throw_replaces_pending_separator_overflow() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(source) = context
        .eval(
            r#"(function(){
                var source=Object(),element=Object();source.length=2;
                source[0]="aa";source[1]=element;
                element.toLocaleString=function(){throw 88};
                return source;
            })()"#,
        )
        .unwrap()
    else {
        panic!("Array.toLocaleString throwing overflow fixture was not an object");
    };
    let completion = runtime
        .call_array_prototype_join_with_string_limit(
            context.realm,
            ArrayJoinKind::ToLocaleString,
            crate::engine::vm::call::NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &crate::engine::vm::call::NativeArguments {
                actual_arg_count: 0,
                readable: Vec::new(),
            },
            2,
        )
        .unwrap();
    assert!(matches!(completion, Completion::Throw(Value::Int(88))));
}

#[test]
fn array_length_uses_quickjs_double_conversion_and_caller_realm_errors() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut caller = runtime.new_context();
    let array = first.new_array_from_values(vec![Value::Int(1)]).unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let value = caller
        .eval("(function(){function V(){this.count=0}V.prototype.valueOf=function(){this.count++;return this.count};return new V})()")
        .unwrap();
    assert_eq!(
        caller.define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value.clone()),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut caller),
        JsString::from_static("invalid array length")
    );
    let Value::Object(value) = value else {
        panic!("length conversion fixture was not an object");
    };
    let count = runtime.intern_property_key("count").unwrap();
    assert_eq!(caller.get_property(&value, &count).unwrap(), Value::Int(2));
    assert_eq!(caller.get_property(&array, &length).unwrap(), Value::Int(1));

    assert_eq!(
        caller.define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Float(1.5)),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("invalid Array length did not materialize a RangeError");
    };
    let expected = runtime
        .0
        .state
        .borrow()
        .heap
        .context(caller.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Range.index()]
    .unwrap();
    assert_eq!(
        runtime
            .get_prototype_of(&error)
            .unwrap()
            .unwrap()
            .object_id(),
        expected
    );

    let throwing = caller
        .eval("(function(){function V(){}V.prototype.valueOf=function(){throw 77};return new V})()")
        .unwrap();
    assert_eq!(
        caller.define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(throwing),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(caller.take_exception().unwrap(), Some(Value::Int(77)));
}

#[test]
fn array_slots_and_realm_roots_survive_gc_then_collect() {
    let runtime = Runtime::new();
    let baseline_atoms = runtime.test_atom_count();
    let (array, zero, one) = {
        let mut context = runtime.new_context();
        let element = context.new_object().unwrap();
        let symbol = runtime
            .new_symbol(Some(JsString::from_static("dense")))
            .unwrap();
        let array = context
            .new_array_from_values(vec![Value::Object(element.clone()), Value::Symbol(symbol)])
            .unwrap();
        let zero = runtime.intern_property_key("0").unwrap();
        let one = runtime.intern_property_key("1").unwrap();
        assert!(
            context
                .define_own_property(
                    &array,
                    &zero,
                    &OrdinaryPropertyDescriptor {
                        enumerable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(runtime.array_fast_len(&array).unwrap(), None);
        (array, zero, one)
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    assert!(matches!(
        get_property(&runtime, &array, &zero).unwrap(),
        Value::Object(_)
    ));
    assert!(matches!(
        get_property(&runtime, &array, &one).unwrap(),
        Value::Symbol(_)
    ));
    drop(array);
    drop(zero);
    drop(one);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn array_dense_self_cycle_is_visible_to_cycle_collection() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        let array = context.new_array().unwrap();
        let zero = runtime.intern_property_key("0").unwrap();
        assert!(
            context
                .set_property(&array, &zero, Value::Object(array.clone()))
                .unwrap()
        );
    }
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn array_of_uses_set_for_a_custom_result_length() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context.new_object().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let setter = eval_callable(
        &runtime,
        &mut context,
        "(function(value){ this.seen = value; })",
    );
    assert!(
        context
            .define_own_property(
                &result,
                &length,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let global = context.global_object().unwrap();
    let result_key = runtime.intern_property_key("arrayOfResult").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &result_key,
                &data_descriptor(Value::Object(result.clone()), true, true, true),
            )
            .unwrap()
    );
    let constructor = eval_callable(
        &runtime,
        &mut context,
        "(function CustomArray(){ return arrayOfResult; })",
    );
    let array = global_callable(&runtime, &mut context, "Array");
    let of = property_callable(&runtime, &mut context, array.as_object(), "of");
    assert_eq!(
        context
            .call(
                &of,
                Value::Object(constructor.as_object().clone()),
                &[Value::Int(11), Value::Int(22)],
            )
            .unwrap(),
        Value::Object(result.clone())
    );

    for (name, expected) in [("0", 11), ("1", 22), ("seen", 2)] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(
            context.get_property(&result, &key).unwrap(),
            Value::Int(expected)
        );
    }
    assert!(matches!(
        context.get_own_property(&result, &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor { .. })
    ));
}

#[test]
fn array_of_create_data_property_reports_the_quickjs_rejection() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let array = global_callable(&runtime, &mut context, "Array");
    let of = property_callable(&runtime, &mut context, array.as_object(), "of");

    let blocked = context.new_object().unwrap();
    runtime.prevent_extensions(&blocked).unwrap();
    let blocked_key = runtime.intern_property_key("blockedArrayResult").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &blocked_key,
                &data_descriptor(Value::Object(blocked), true, true, true),
            )
            .unwrap()
    );
    let blocked_constructor = eval_callable(
        &runtime,
        &mut context,
        "(function BlockedArray(){ return blockedArrayResult; })",
    );
    assert_eq!(
        context.call(
            &of,
            Value::Object(blocked_constructor.as_object().clone()),
            &[Value::Int(1)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("object is not extensible")
    );

    let frozen = context.new_array().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(
        context
            .define_own_property(
                &frozen,
                &length,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let frozen_key = runtime.intern_property_key("frozenArrayResult").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &frozen_key,
                &data_descriptor(Value::Object(frozen), true, true, true),
            )
            .unwrap()
    );
    let frozen_constructor = eval_callable(
        &runtime,
        &mut context,
        "(function FrozenArray(){ return frozenArrayResult; })",
    );
    assert_eq!(
        context.call(
            &of,
            Value::Object(frozen_constructor.as_object().clone()),
            &[Value::Int(1)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("'length' is read-only")
    );
}

#[test]
fn array_constructor_sets_through_inherited_indices_like_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.array_prototype().unwrap();
    let zero = runtime.intern_property_key("0").unwrap();
    let setter = eval_callable(
        &runtime,
        &mut context,
        "(function(value){ this.hit = value; })",
    );
    assert!(
        context
            .define_own_property(
                &prototype,
                &zero,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let constructor = global_callable(&runtime, &mut context, "Array");
    let Value::Object(array) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("x"))],
        )
        .unwrap()
    else {
        panic!("Array call did not return an object");
    };
    let hit = runtime.intern_property_key("hit").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        context.get_property(&array, &hit).unwrap(),
        Value::String(JsString::from_static("x"))
    );
    assert!(!runtime.has_own_property(&array, &zero).unwrap());
    assert_eq!(
        context.get_property(&array, &length).unwrap(),
        Value::Int(0)
    );

    let one = runtime.intern_property_key("1").unwrap();
    assert!(
        context
            .define_own_property(
                &prototype,
                &one,
                &data_descriptor(Value::Int(1), false, false, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[Value::Int(10), Value::Int(20)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("'1' is read-only")
    );
}
