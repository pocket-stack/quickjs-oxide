use super::*;

#[test]
fn global_uri_codecs_match_quickjs_graph_and_utf16_kernel() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let names = [
        "decodeURI",
        "decodeURIComponent",
        "encodeURI",
        "encodeURIComponent",
        "escape",
        "unescape",
    ];
    for name in names {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(runtime.is_auto_init_own_property(&global, &key).unwrap());
        let callable = global_callable(&runtime, &mut context, name);
        assert_eq!(
            own_key_names(&runtime, callable.as_object()),
            ["length", "name"]
        );
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "length"),
            Value::Int(1)
        );
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "name"),
            Value::String(JsString::try_from_utf8(name).unwrap())
        );
        assert!(matches!(
            runtime.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));
    }

    for (name, input, expected) in [
        ("decodeURI", "%2f%20", "%2f "),
        ("decodeURIComponent", "%2f%20", "/ "),
        ("encodeURI", ";/ ?", ";/%20?"),
        ("encodeURIComponent", ";/ ?", "%3B%2F%20%3F"),
        ("unescape", "%E9%u0100", "éĀ"),
    ] {
        let callable = global_callable(&runtime, &mut context, name);
        assert_eq!(
            context
                .call(
                    &callable,
                    Value::Null,
                    &[
                        Value::String(JsString::try_from_utf8(input).unwrap()),
                        Value::Int(99),
                    ],
                )
                .unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap())
        );
    }
    let escape = global_callable(&runtime, &mut context, "escape");
    assert_eq!(
        context
            .call(
                &escape,
                Value::Undefined,
                &[Value::String(
                    JsString::try_from_utf16([0x00e9, 0xd83d, 0xde00]).unwrap(),
                )],
            )
            .unwrap(),
        Value::String(JsString::from_static("%E9%uD83D%uDE00"))
    );

    for (name, input, message) in [
        ("decodeURI", "%", "expecting hex digit"),
        ("decodeURIComponent", "%E0%A0", "expecting %"),
    ] {
        let callable = global_callable(&runtime, &mut context, name);
        assert_eq!(
            context.call(
                &callable,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8(input).unwrap())],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::try_from_utf8(message).unwrap()
        );
    }
    let encode = global_callable(&runtime, &mut context, "encodeURI");
    assert_eq!(
        context.call(
            &encode,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf16([0xdc00]).unwrap(),)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("invalid character")
    );
}

#[test]
fn global_primitive_constants_match_quickjs_frozen_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let supported_global_prefix = own_key_names(&runtime, &global)
        .into_iter()
        .filter(|name| {
            matches!(
                name.as_str(),
                "parseInt"
                    | "parseFloat"
                    | "isNaN"
                    | "isFinite"
                    | "decodeURI"
                    | "decodeURIComponent"
                    | "encodeURI"
                    | "encodeURIComponent"
                    | "escape"
                    | "unescape"
                    | "Infinity"
                    | "NaN"
                    | "undefined"
                    | "Number"
                    | "Boolean"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        supported_global_prefix,
        [
            "parseInt",
            "parseFloat",
            "isNaN",
            "isFinite",
            "decodeURI",
            "decodeURIComponent",
            "encodeURI",
            "encodeURIComponent",
            "escape",
            "unescape",
            "Infinity",
            "NaN",
            "undefined",
            "Number",
            "Boolean",
        ]
    );
    for (name, expected) in [
        ("undefined", Value::Undefined),
        ("NaN", Value::Float(f64::NAN)),
        ("Infinity", Value::Float(f64::INFINITY)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let Some(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }) = runtime.get_own_property(&global, &key).unwrap()
        else {
            panic!("global {name} was not an own data property");
        };
        assert!(value.same_value(&expected), "global {name}");
        assert!(!writable, "global {name}");
        assert!(!enumerable, "global {name}");
        assert!(!configurable, "global {name}");
        assert!(!runtime.delete_property(&global, &key).unwrap());
        assert_eq!(
            context.eval(&format!("delete {name}")).unwrap(),
            Value::Bool(false)
        );
    }
}

#[test]
fn global_to_string_tag_matches_quickjs_descriptor_and_class_tag() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(matches!(
        runtime.get_own_property(&global, &tag).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("global")
    ));
    assert_eq!(
        runtime.own_property_keys(&global).unwrap().last(),
        Some(&tag)
    );

    let object_prototype = context.object_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    assert_eq!(
        context
            .call(&object_to_string, Value::Object(global.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("[object global]"))
    );
    assert!(runtime.delete_property(&global, &tag).unwrap());
    assert_eq!(
        context
            .call(&object_to_string, Value::Object(global), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object Object]"))
    );
}

#[test]
fn global_this_matches_quickjs_identity_descriptor_and_mutation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("globalThis").unwrap();
    assert!(matches!(
        runtime.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == global
    ));
    assert_eq!(
        context
            .eval("globalThis === globalThis.globalThis")
            .unwrap(),
        Value::Bool(true)
    );

    assert!(context.set_property(&global, &key, Value::Int(17)).unwrap());
    assert_eq!(context.eval("globalThis").unwrap(), Value::Int(17));
    assert!(matches!(
        runtime.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(17),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    assert!(runtime.delete_property(&global, &key).unwrap());
    assert_eq!(
        context.eval("typeof globalThis").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert!(
        context
            .set_property(&global, &key, Value::Object(global.clone()))
            .unwrap()
    );
    assert!(matches!(
        runtime.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: true,
            configurable: true,
        }) if value == global
    ));
}
