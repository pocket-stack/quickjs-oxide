use super::*;

#[test]
fn object_prototype_prefix_methods_are_lazy_and_report_core_tags() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let to_locale_string_key = runtime.intern_property_key("toLocaleString").unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let baseline_objects = runtime.heap_counts().object_nodes;
    assert_eq!(
        own_key_names(&runtime, &object_prototype),
        [
            "toString",
            "toLocaleString",
            "valueOf",
            "hasOwnProperty",
            "isPrototypeOf",
            "propertyIsEnumerable",
            "__proto__",
            "__defineGetter__",
            "__defineSetter__",
            "__lookupGetter__",
            "__lookupSetter__",
            "constructor",
        ]
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

    let Value::Object(to_string_object) = context
        .get_property(&object_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Object.prototype.toString was not an object");
    };
    let Value::Object(to_locale_string_object) = context
        .get_property(&object_prototype, &to_locale_string_key)
        .unwrap()
    else {
        panic!("Object.prototype.toLocaleString was not an object");
    };
    let Value::Object(value_of_object) = context
        .get_property(&object_prototype, &value_of_key)
        .unwrap()
    else {
        panic!("Object.prototype.valueOf was not an object");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 3);
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();
    let to_locale_string = runtime
        .as_callable(&to_locale_string_object)
        .unwrap()
        .unwrap();
    let value_of = runtime.as_callable(&value_of_object).unwrap().unwrap();
    let object = context.new_object().unwrap();
    let function = context.eval("(0, function(){})").unwrap();
    let error = global_callable(&runtime, &mut context, "Error");
    let error = context.call(&error, Value::Undefined, &[]).unwrap();
    for (value, expected) in [
        (Value::Null, "[object Null]"),
        (Value::Undefined, "[object Undefined]"),
        (Value::Object(object.clone()), "[object Object]"),
        (function, "[object Function]"),
        (error, "[object Error]"),
    ] {
        assert_eq!(
            context.call(&to_string, value, &[]).unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap())
        );
    }
    assert_eq!(
        context
            .call(&value_of, Value::Object(object.clone()), &[])
            .unwrap(),
        Value::Object(object.clone())
    );
    assert_eq!(
        context
            .call(&to_locale_string, Value::Object(object), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object Object]"))
    );
}

#[test]
fn object_define_properties_filters_lazy_entries_without_materializing_them() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let lazy_keys = [
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "__defineGetter__",
        "__defineSetter__",
        "__lookupGetter__",
        "__lookupSetter__",
    ]
    .map(|name| runtime.intern_property_key(name).unwrap());
    for key in &lazy_keys {
        assert!(
            runtime
                .is_auto_init_own_property(&object_prototype, key)
                .unwrap()
        );
    }

    let object_constructor = global_callable(&runtime, &mut context, "Object");
    let define_properties = property_callable(
        &runtime,
        &mut context,
        object_constructor.as_object(),
        "defineProperties",
    );
    let target = context.new_object().unwrap();
    assert_eq!(
        context
            .call(
                &define_properties,
                Value::Undefined,
                &[
                    Value::Object(target.clone()),
                    Value::Object(object_prototype.clone()),
                ],
            )
            .unwrap(),
        Value::Object(target.clone())
    );
    assert!(runtime.own_property_keys(&target).unwrap().is_empty());
    for key in &lazy_keys {
        assert!(
            runtime
                .is_auto_init_own_property(&object_prototype, key)
                .unwrap()
        );
    }
}

#[test]
fn shape_sharing_and_descriptor_defaults_follow_quickjs_layout() {
    let runtime = Runtime::new();
    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("x").unwrap();

    let empty_shapes = {
        let state = runtime.0.state.borrow();
        (
            state.heap.object(first.object_id()).unwrap().shape,
            state.heap.object(second.object_id()).unwrap().shape,
        )
    };
    assert_eq!(empty_shapes.0, empty_shapes.1);

    let defaulted = OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(7)),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        runtime
            .define_own_property(&first, &key, &defaulted)
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(&second, &key, &defaulted)
            .unwrap()
    );
    assert_eq!(
        runtime.get_own_property(&first, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(7),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );
    let populated_shapes = {
        let state = runtime.0.state.borrow();
        (
            state.heap.object(first.object_id()).unwrap().shape,
            state.heap.object(second.object_id()).unwrap().shape,
        )
    };
    assert_eq!(populated_shapes.0, populated_shapes.1);
    assert_ne!(populated_shapes.0, empty_shapes.0);
}

#[test]
fn own_keys_preserve_quickjs_category_order_and_utf16_identity() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let symbol_a = runtime
        .new_symbol(Some(JsString::from_static("a")))
        .unwrap();
    let symbol_b = runtime
        .new_symbol(Some(JsString::from_static("b")))
        .unwrap();
    let symbol_key_a = PropertyKey::from(&symbol_a);
    let symbol_key_b = PropertyKey::from(&symbol_b);

    for (key, value) in [
        (runtime.intern_property_key("beta").unwrap(), 1),
        (runtime.intern_property_key("4294967295").unwrap(), 2),
        (runtime.intern_property_key("2147483648").unwrap(), 3),
        (runtime.intern_property_key("01").unwrap(), 4),
        (runtime.intern_property_key("4294967294").unwrap(), 5),
        (runtime.intern_property_key("0").unwrap(), 6),
        (runtime.intern_property_key("-0").unwrap(), 7),
    ] {
        assert!(set_property(&runtime, &object, &key, Value::Int(value)).unwrap());
    }
    assert!(set_property(&runtime, &object, &symbol_key_a, Value::Int(8)).unwrap());
    assert!(
        set_property(
            &runtime,
            &object,
            &runtime.intern_property_key("2").unwrap(),
            Value::Int(9)
        )
        .unwrap()
    );
    assert!(set_property(&runtime, &object, &symbol_key_b, Value::Int(10)).unwrap());

    let expected = [
        runtime.intern_property_key("0").unwrap(),
        runtime.intern_property_key("2").unwrap(),
        runtime.intern_property_key("2147483648").unwrap(),
        runtime.intern_property_key("4294967294").unwrap(),
        runtime.intern_property_key("beta").unwrap(),
        runtime.intern_property_key("4294967295").unwrap(),
        runtime.intern_property_key("01").unwrap(),
        runtime.intern_property_key("-0").unwrap(),
        symbol_key_a.clone(),
        symbol_key_b.clone(),
    ];
    assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);

    let surrogate = runtime
        .intern_property_key_js_string(&JsString::try_from_utf16([0xd800]).unwrap())
        .unwrap();
    let replacement = runtime
        .intern_property_key_js_string(&JsString::try_from_utf16([0xfffd]).unwrap())
        .unwrap();
    assert_ne!(surrogate, replacement);
    assert!(set_property(&runtime, &object, &surrogate, Value::Int(11)).unwrap());
    assert!(set_property(&runtime, &object, &replacement, Value::Int(12)).unwrap());
    assert_eq!(
        runtime.property_key_to_js_string(&surrogate).unwrap(),
        JsString::try_from_utf16([0xd800]).unwrap()
    );
}

#[test]
fn delete_readd_and_frozen_same_value_rules_match_oracle() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let a = runtime.intern_property_key("a").unwrap();
    let b = runtime.intern_property_key("b").unwrap();
    let c = runtime.intern_property_key("c").unwrap();
    for key in [&a, &b, &c] {
        assert!(set_property(&runtime, &object, key, Value::Int(1)).unwrap());
    }
    assert!(runtime.delete_property(&object, &a).unwrap());
    assert!(set_property(&runtime, &object, &a, Value::Int(2)).unwrap());
    assert_eq!(
        runtime.own_property_keys(&object).unwrap(),
        vec![b.clone(), c.clone(), a.clone()]
    );

    let nan = runtime.intern_property_key("nan").unwrap();
    let zero = runtime.intern_property_key("zero").unwrap();
    assert!(
        runtime
            .define_own_property(
                &object,
                &nan,
                &data_descriptor(Value::Float(f64::NAN), false, true, false)
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &zero,
                &data_descriptor(Value::Int(0), false, true, false)
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &nan,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Float(f64::NAN)),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert!(
        !runtime
            .define_own_property(
                &object,
                &nan,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(0)),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert!(
        !runtime
            .define_own_property(
                &object,
                &zero,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Float(-0.0)),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
}

#[test]
fn inherited_set_and_prototype_constraints_match_ordinary_semantics() {
    let runtime = Runtime::new();
    let parent = runtime.new_object(None).unwrap();
    let writable = runtime.intern_property_key("writable").unwrap();
    let readonly = runtime.intern_property_key("readonly").unwrap();
    assert!(
        runtime
            .define_own_property(
                &parent,
                &writable,
                &data_descriptor(Value::Int(1), true, true, true)
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &parent,
                &readonly,
                &data_descriptor(Value::Int(1), false, true, true)
            )
            .unwrap()
    );
    let child = runtime.new_object(Some(&parent)).unwrap();
    assert!(set_property(&runtime, &child, &writable, Value::Int(2)).unwrap());
    assert!(!set_property(&runtime, &child, &readonly, Value::Int(2)).unwrap());
    assert_eq!(
        get_property(&runtime, &child, &writable).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        get_property(&runtime, &child, &readonly).unwrap(),
        Value::Int(1)
    );

    let receiver = runtime.new_object(None).unwrap();
    assert!(
        set_property_with_receiver(
            &runtime,
            &parent,
            &writable,
            Value::Int(3),
            Value::Object(receiver.clone()),
        )
        .unwrap()
    );
    assert_eq!(
        get_property(&runtime, &parent, &writable).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        get_property(&runtime, &receiver, &writable).unwrap(),
        Value::Int(3)
    );

    let mut context = runtime.new_context();
    let Value::Object(receiver_setter) = context.eval("(function(value) {})").unwrap() else {
        panic!("receiver setter probe did not produce a function");
    };
    let receiver_setter = runtime.as_callable(&receiver_setter).unwrap().unwrap();
    let accessor_receiver = runtime.new_object(None).unwrap();
    assert!(
        runtime
            .define_own_property(
                &accessor_receiver,
                &writable,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(receiver_setter)),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(
        !set_property_with_receiver(
            &runtime,
            &parent,
            &writable,
            Value::Int(4),
            Value::Object(accessor_receiver),
        )
        .unwrap()
    );

    let fixed = runtime.new_object(None).unwrap();
    runtime.prevent_extensions(&fixed).unwrap();
    assert!(runtime.set_prototype_of(&fixed, None).unwrap());
    assert!(!runtime.set_prototype_of(&fixed, Some(&parent)).unwrap());

    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    assert!(runtime.set_prototype_of(&first, Some(&second)).unwrap());
    assert!(!runtime.set_prototype_of(&second, Some(&first)).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&first).unwrap(),
        Some(second.clone())
    );
    assert_eq!(runtime.get_prototype_of(&second).unwrap(), None);
}
