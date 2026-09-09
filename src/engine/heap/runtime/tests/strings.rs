use super::*;

#[test]
fn string_wrapper_exotic_indices_length_define_delete_and_order_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let string_prototype_id = runtime
        .0
        .state
        .borrow()
        .heap
        .context(context.realm)
        .unwrap()
        .primitive_prototypes[PrimitiveKind::String.index()]
    .expect("String exotic-core prototype slot was absent");
    let string_prototype =
        crate::engine::api::ObjectRef::from_borrowed_handle(runtime.clone(), string_prototype_id)
            .unwrap();

    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(string_prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::String(value)) if value.is_empty()
    ));
    assert_eq!(
        own_key_names(&runtime, &string_prototype),
        [
            "length",
            "at",
            "charCodeAt",
            "charAt",
            "concat",
            "codePointAt",
            "isWellFormed",
            "toWellFormed",
            "indexOf",
            "lastIndexOf",
            "includes",
            "endsWith",
            "startsWith",
            "match",
            "matchAll",
            "search",
            "split",
            "substring",
            "substr",
            "slice",
            "repeat",
            "replace",
            "replaceAll",
            "padEnd",
            "padStart",
            "trim",
            "trimEnd",
            "trimRight",
            "trimStart",
            "trimLeft",
            "toString",
            "valueOf",
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
            "constructor",
            "normalize",
            "localeCompare",
            "Symbol.iterator",
        ],
        "implemented-key filtered order, not the complete String prototype table"
    );
    let length_key = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        runtime
            .get_own_property(&string_prototype, &length_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(0),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    );

    let payload = JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xd800]).unwrap();
    let wrapper = runtime
        .new_string_object(&string_prototype, payload.clone(), false)
        .unwrap();
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::String(value)) if value == &payload
    ));
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(string_prototype.clone())
    );
    assert_eq!(
        own_key_names(&runtime, &wrapper),
        ["0", "1", "2", "3", "length"]
    );
    assert_eq!(
        runtime.get_own_property(&wrapper, &length_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(4),
            writable: false,
            enumerable: false,
            configurable: false,
        })
    );

    for (index, unit) in [0x41, 0xd83d, 0xde00, 0xd800].into_iter().enumerate() {
        let key = runtime.intern_property_key(&index.to_string()).unwrap();
        let expected = Value::String(JsString::try_from_utf16([unit]).unwrap());
        assert_eq!(
            runtime.get_own_property(&wrapper, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: expected.clone(),
                writable: false,
                enumerable: true,
                configurable: false,
            })
        );
        assert!(runtime.has_own_property(&wrapper, &key).unwrap());
        assert!(
            runtime
                .define_own_property(
                    &wrapper,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(expected),
                        writable: DescriptorField::Present(false),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(!runtime.delete_property(&wrapper, &key).unwrap());
    }

    let zero = runtime.intern_property_key("0").unwrap();
    for descriptor in [
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::String(JsString::from_static("X"))),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            writable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            enumerable: DescriptorField::Present(false),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            get: DescriptorField::Present(AccessorValue::Undefined),
            ..OrdinaryPropertyDescriptor::new()
        },
    ] {
        assert!(
            !runtime
                .define_own_property(&wrapper, &zero, &descriptor)
                .unwrap()
        );
    }
    assert!(!runtime.delete_property(&wrapper, &length_key).unwrap());

    let eight = runtime.intern_property_key("8").unwrap();
    let foo = runtime.intern_property_key("foo").unwrap();
    let leading_zero = runtime.intern_property_key("01").unwrap();
    let symbol = PropertyKey::from(
        runtime
            .new_symbol(Some(JsString::from_static("tail")))
            .unwrap(),
    );
    for key in [&foo, &leading_zero, &eight, &symbol] {
        assert!(
            runtime
                .define_own_property(
                    &wrapper,
                    key,
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
    }
    assert_eq!(
        own_key_names(&runtime, &wrapper),
        ["0", "1", "2", "3", "8", "length", "foo", "01", "tail"]
    );

    runtime.prevent_extensions(&wrapper).unwrap();
    assert!(
        runtime
            .define_own_property(&wrapper, &zero, &OrdinaryPropertyDescriptor::new(),)
            .unwrap()
    );
    let nine = runtime.intern_property_key("9").unwrap();
    assert!(
        !runtime
            .define_own_property(
                &wrapper,
                &nine,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let sloppy = eval_callable(&runtime, &mut context, "(function(){ return this; })");
    let Value::Object(escaped) = context
        .call(&sloppy, Value::String(payload.clone()), &[])
        .unwrap()
    else {
        panic!("sloppy String this did not escape as a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&escaped).unwrap(),
        Some(string_prototype)
    );
    assert_eq!(
        own_key_names(&runtime, &escaped),
        ["0", "1", "2", "3", "length"]
    );
}

#[test]
fn string_method_slice_matches_quickjs_table_and_code_unit_rules() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();

    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "length",
            "at",
            "charCodeAt",
            "charAt",
            "concat",
            "codePointAt",
            "isWellFormed",
            "toWellFormed",
            "indexOf",
            "lastIndexOf",
            "includes",
            "endsWith",
            "startsWith",
            "match",
            "matchAll",
            "search",
            "split",
            "substring",
            "substr",
            "slice",
            "repeat",
            "replace",
            "replaceAll",
            "padEnd",
            "padStart",
            "trim",
            "trimEnd",
            "trimRight",
            "trimStart",
            "trimLeft",
            "toString",
            "valueOf",
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
            "constructor",
            "normalize",
            "localeCompare",
            "Symbol.iterator",
        ],
        "implemented entries must retain their pinned QuickJS table order"
    );

    let methods = [
        ("at", "at", 1, NativeCProto::GenericMagic, 1),
        ("charCodeAt", "charCodeAt", 1, NativeCProto::Generic, 1),
        ("charAt", "charAt", 1, NativeCProto::GenericMagic, 1),
        ("concat", "concat", 1, NativeCProto::Generic, 0),
        ("codePointAt", "codePointAt", 1, NativeCProto::Generic, 1),
        ("isWellFormed", "isWellFormed", 0, NativeCProto::Generic, 0),
        ("toWellFormed", "toWellFormed", 0, NativeCProto::Generic, 0),
        ("substring", "substring", 2, NativeCProto::Generic, 2),
        ("substr", "substr", 2, NativeCProto::Generic, 2),
        ("slice", "slice", 2, NativeCProto::Generic, 2),
        ("padEnd", "padEnd", 1, NativeCProto::GenericMagic, 1),
        ("padStart", "padStart", 1, NativeCProto::GenericMagic, 1),
        ("trim", "trim", 0, NativeCProto::GenericMagic, 0),
        ("trimEnd", "trimEnd", 0, NativeCProto::GenericMagic, 0),
        ("trimRight", "trimEnd", 0, NativeCProto::GenericMagic, 0),
        ("trimStart", "trimStart", 0, NativeCProto::GenericMagic, 0),
        ("trimLeft", "trimStart", 0, NativeCProto::GenericMagic, 0),
        (
            "toLowerCase",
            "toLowerCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        (
            "toUpperCase",
            "toUpperCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        (
            "toLocaleLowerCase",
            "toLocaleLowerCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        (
            "toLocaleUpperCase",
            "toLocaleUpperCase",
            0,
            NativeCProto::GenericMagic,
            0,
        ),
        ("anchor", "anchor", 1, NativeCProto::GenericMagic, 1),
        ("big", "big", 0, NativeCProto::GenericMagic, 0),
        ("blink", "blink", 0, NativeCProto::GenericMagic, 0),
        ("bold", "bold", 0, NativeCProto::GenericMagic, 0),
        ("fixed", "fixed", 0, NativeCProto::GenericMagic, 0),
        ("fontcolor", "fontcolor", 1, NativeCProto::GenericMagic, 1),
        ("fontsize", "fontsize", 1, NativeCProto::GenericMagic, 1),
        ("italics", "italics", 0, NativeCProto::GenericMagic, 0),
        ("link", "link", 1, NativeCProto::GenericMagic, 1),
        ("small", "small", 0, NativeCProto::GenericMagic, 0),
        ("strike", "strike", 0, NativeCProto::GenericMagic, 0),
        ("sub", "sub", 0, NativeCProto::GenericMagic, 0),
        ("sup", "sup", 0, NativeCProto::GenericMagic, 0),
    ];
    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    for (name, function_name, length, cproto, min_readable_args) in methods {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&prototype, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(_),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));
        let method = property_callable(&runtime, &mut context, &prototype, name);
        assert!(!runtime.is_constructor(method.as_object()).unwrap());
        assert!(matches!(
            runtime
                .get_own_property(method.as_object(), &length_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == length
        ));
        assert!(matches!(
            runtime
                .get_own_property(method.as_object(), &name_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::try_from_utf8(function_name).unwrap()
        ));
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(method.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("String method was not a native function: {name}");
        };
        assert_eq!(data.target.descriptor().cproto, cproto);
        assert_eq!(data.min_readable_args, min_readable_args);
    }

    let payload = JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xd800, 0x5a]).unwrap();
    let at = property_callable(&runtime, &mut context, &prototype, "at");
    assert_eq!(
        context
            .call(&at, Value::String(payload.clone()), &[Value::Int(-1)])
            .unwrap(),
        Value::String(JsString::from_static("Z"))
    );
    assert_eq!(
        context
            .call(&at, Value::String(payload.clone()), &[Value::Int(1)])
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xd83d]).unwrap())
    );
    assert_eq!(
        context
            .call(&at, Value::String(payload.clone()), &[Value::Int(5)])
            .unwrap(),
        Value::Undefined
    );

    let char_at = property_callable(&runtime, &mut context, &prototype, "charAt");
    assert_eq!(
        context
            .call(&char_at, Value::String(payload.clone()), &[Value::Int(-1)],)
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    let char_code_at = property_callable(&runtime, &mut context, &prototype, "charCodeAt");
    assert_eq!(
        context
            .call(
                &char_code_at,
                Value::String(payload.clone()),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::Int(0xde00)
    );
    assert!(matches!(
        context
            .call(
                &char_code_at,
                Value::String(payload.clone()),
                &[Value::Int(5)],
            )
            .unwrap(),
        Value::Float(value) if value.is_nan()
    ));

    let code_point_at = property_callable(&runtime, &mut context, &prototype, "codePointAt");
    assert_eq!(
        context
            .call(
                &code_point_at,
                Value::String(payload.clone()),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::Int(0x1f600)
    );
    assert_eq!(
        context
            .call(
                &code_point_at,
                Value::String(payload.clone()),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::Int(0xde00)
    );

    let concat = property_callable(&runtime, &mut context, &prototype, "concat");
    assert_eq!(
        context
            .call(
                &concat,
                Value::String(JsString::from_static("R")),
                &[
                    Value::Undefined,
                    Value::Null,
                    Value::Bool(true),
                    Value::BigInt(JsBigInt::one()),
                ],
            )
            .unwrap(),
        Value::String(JsString::from_static("Rundefinednulltrue1"))
    );
    let mut near_limit = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
    for _ in 0..16 {
        near_limit = near_limit.try_concat(&near_limit).unwrap();
    }
    assert_eq!(near_limit.len(), 536_936_448);
    assert!(matches!(
        near_limit.try_concat(&near_limit),
        Err(crate::engine::value::JsStringError::TooLong)
    ));

    let is_well_formed = property_callable(&runtime, &mut context, &prototype, "isWellFormed");
    let to_well_formed = property_callable(&runtime, &mut context, &prototype, "toWellFormed");
    assert_eq!(
        context
            .call(
                &is_well_formed,
                Value::String(payload.clone()),
                &[Value::Symbol(runtime.new_symbol(None).unwrap())],
            )
            .unwrap(),
        Value::Bool(false),
        "well-formed methods ignore all actual arguments"
    );
    assert_eq!(
        context
            .call(&to_well_formed, Value::String(payload), &[])
            .unwrap(),
        Value::String(JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xfffd, 0x5a]).unwrap(),)
    );

    assert_eq!(
        context.call(&at, Value::Null, &[Value::Int(0)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("null or undefined are forbidden")
    );
    assert_eq!(
        context.eval("'abc'.at(-1)").unwrap(),
        Value::String(JsString::from_static("c"))
    );
    assert_eq!(context.eval("'abc'.charCodeAt(1)").unwrap(), Value::Int(98));
}

#[test]
fn string_rope_vm_and_native_concat_overflow_use_the_defining_realms() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_string = first.string_prototype().unwrap();
    let concat = property_callable(&runtime, &mut first, &first_string, "concat");
    let internal_error = global_callable(&runtime, &mut first, "InternalError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(first_internal_error_prototype) = first
        .get_property(internal_error.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("InternalError.prototype was not an object");
    };

    let mut near_limit = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
    for _ in 0..16 {
        near_limit = near_limit.try_concat(&near_limit).unwrap();
    }
    assert_eq!(near_limit.len(), 536_936_448);
    assert!(!near_limit.is_flat());

    let global = first.global_object().unwrap();
    let near_key = runtime.intern_property_key("nearLimitString").unwrap();
    assert!(
        runtime
            .define_own_property(
                &global,
                &near_key,
                &data_descriptor(Value::String(near_limit.clone()), true, false, true),
            )
            .unwrap()
    );
    assert_eq!(
        first.eval("nearLimitString + nearLimitString"),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(vm_error)) = first.take_exception().unwrap() else {
        panic!("VM String overflow did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&vm_error).unwrap(),
        Some(first_internal_error_prototype.clone())
    );
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        first.get_property(&vm_error, &message).unwrap(),
        Value::String(JsString::from_static("string too long"))
    );

    assert_eq!(
        second.call(
            &concat,
            Value::String(JsString::from_static("")),
            &[Value::String(near_limit.clone()), Value::String(near_limit),],
        ),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(native_error)) = second.take_exception().unwrap() else {
        panic!("native String.concat overflow did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(first_internal_error_prototype)
    );
    assert_eq!(
        second.get_property(&native_error, &message).unwrap(),
        Value::String(JsString::from_static("string too long"))
    );
}

#[test]
fn string_conversion_core_brand_lookup_object_routes_and_overrides_match_quickjs_slice() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");

    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "length",
            "at",
            "charCodeAt",
            "charAt",
            "concat",
            "codePointAt",
            "isWellFormed",
            "toWellFormed",
            "indexOf",
            "lastIndexOf",
            "includes",
            "endsWith",
            "startsWith",
            "match",
            "matchAll",
            "search",
            "split",
            "substring",
            "substr",
            "slice",
            "repeat",
            "replace",
            "replaceAll",
            "padEnd",
            "padStart",
            "trim",
            "trimEnd",
            "trimRight",
            "trimStart",
            "trimLeft",
            "toString",
            "valueOf",
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
            "constructor",
            "normalize",
            "localeCompare",
            "Symbol.iterator",
        ],
        "implemented-key filtered order, not the complete String prototype table"
    );
    for name in ["toString", "valueOf"] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&prototype, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(_),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));
    }

    let payload = JsString::try_from_utf16([0x41, 0xd800, 0x42]).unwrap();
    let wrapper = runtime
        .new_string_object(&prototype, payload.clone(), false)
        .unwrap();
    for method in [&to_string, &value_of] {
        assert_eq!(
            context
                .call(method, Value::String(payload.clone()), &[Value::Int(99)])
                .unwrap(),
            Value::String(payload.clone())
        );
        assert_eq!(
            context
                .call(method, Value::Object(wrapper.clone()), &[])
                .unwrap(),
            Value::String(payload.clone())
        );
        assert_eq!(
            context
                .call(method, Value::Object(prototype.clone()), &[])
                .unwrap(),
            Value::String(JsString::from_static(""))
        );
    }

    let spoof = runtime.new_object(Some(&prototype)).unwrap();
    assert_eq!(
        context.call(&to_string, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("String brand failure did not publish an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("not a string"))
    );

    assert!(
        runtime
            .set_prototype_of(&wrapper, Some(&object_prototype))
            .unwrap()
    );
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper.clone()), &[])
            .unwrap(),
        Value::String(payload.clone())
    );

    let conversion_wrapper = runtime
        .new_string_object(&prototype, payload.clone(), false)
        .unwrap();
    let override_to_string =
        eval_callable(&runtime, &mut context, "(function(){ return 'override'; })");
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    assert!(
        runtime
            .define_own_property(
                &conversion_wrapper,
                &to_string_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(
                        override_to_string.as_object().clone(),
                    )),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        runtime
            .to_primitive(
                context.realm,
                Value::Object(conversion_wrapper.clone()),
                ToPrimitiveHint::String,
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("override")))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(conversion_wrapper), &[])
            .unwrap(),
        Value::String(payload.clone()),
        "saved brand method must ignore ordinary conversion overrides"
    );

    assert_eq!(
        context.eval("'source'.toString()").unwrap(),
        Value::String(JsString::from_static("source"))
    );
    assert_eq!(
        context.eval("'source'.valueOf()").unwrap(),
        Value::String(JsString::from_static("source"))
    );

    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    assert_eq!(
        context
            .call(&object_to_string, Value::String(payload.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object String]"))
    );
    assert_eq!(
        context
            .call(
                &object_to_locale_string,
                Value::String(payload.clone()),
                &[],
            )
            .unwrap(),
        Value::String(payload.clone())
    );
    let Value::Object(first_box) = context
        .call(&object_value_of, Value::String(payload.clone()), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box String");
    };
    let Value::Object(second_box) = context
        .call(&object_value_of, Value::String(payload), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box String");
    };
    assert_ne!(first_box, second_box);
    assert_eq!(
        runtime.get_prototype_of(&first_box).unwrap(),
        Some(prototype.clone())
    );

    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &tag,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from_static("Custom"))),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(
                &object_to_string,
                Value::String(JsString::from_static("x")),
                &[]
            )
            .unwrap(),
        Value::String(JsString::from_static("[object Custom]"))
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &tag,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(
                &object_to_string,
                Value::String(JsString::from_static("x")),
                &[]
            )
            .unwrap(),
        Value::String(JsString::from_static("[object String]"))
    );
    assert!(runtime.delete_property(&prototype, &tag).unwrap());
}
