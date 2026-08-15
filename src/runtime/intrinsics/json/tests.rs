use super::*;

#[test]
fn json_native_cproto_matches_pinned_function_table() {
    for kind in [
        JsonNativeKind::IsRawJson,
        JsonNativeKind::Parse,
        JsonNativeKind::RawJson,
        JsonNativeKind::Stringify,
    ] {
        let descriptor = NativeFunctionId::Json(kind).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::Generic, "{kind:?}");
        assert!(!descriptor.cproto.default_is_constructor(), "{kind:?}");
    }
}

#[test]
fn global_json_is_realm_aware_lazy_and_reserves_the_pinned_table_order() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let key = runtime.intern_property_key("JSON").unwrap();

    for (global, realm) in [(&first_global, first.realm), (&second_global, second.realm)] {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(global.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot),
            Some(PropertySlot::AutoInit(AutoInitProperty::Json {
                realm: defining_realm,
            })) if *defining_realm == realm
        ));
    }

    let Value::Object(json) = first.get_property(&first_global, &key).unwrap() else {
        panic!("JSON did not materialize to an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&json).unwrap(),
        Some(first.object_prototype().unwrap()),
    );
    let expected = [
        (JsonNativeKind::IsRawJson, "isRawJSON", 1),
        (JsonNativeKind::Parse, "parse", 2),
        (JsonNativeKind::RawJson, "rawJSON", 1),
        (JsonNativeKind::Stringify, "stringify", 3),
    ];
    for (kind, name, length) in expected {
        let method = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(json.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot = usize::try_from(shape.find(method.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::Json(target),
                name: target_name,
                length: target_length,
                min_readable_args,
            })) if *realm == first.realm
                && *target == kind
                && *target_name == name
                && *target_length == length
                && *min_readable_args == length
        ));
    }
}

#[test]
fn deleting_lazy_global_json_releases_its_realm_edge() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("JSON").unwrap();
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();

    assert!(runtime.delete_property(&global, &key).unwrap());
    assert!(!runtime.has_own_property(&global, &key).unwrap());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm)
            .unwrap(),
        before - 1,
    );
}

#[test]
fn json_module_parser_returns_the_strict_json_value() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = JsString::from_static("{\"answer\":42}");
    let filename = JsString::from_static("answer.json");
    let NativeConversion::Value(Value::Object(value)) = runtime
        .parse_json_module_text(context.realm, &source, &filename)
        .unwrap()
    else {
        panic!("strict JSON module text did not return its object value");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        context.get_property(&value, &answer).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn quickjs_extended_json_module_parser_is_host_selected_and_keeps_strict_json_strict() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = JsString::try_from_utf8(
        "/* leading */\n{\n\
         // comment\n\
         bare: 'single quoted',\n\
         verticalEscape: 'a\\vb',\n\
         continued: 'left\\\nright',\n\
         trailing: [1, 2,],\n\
         \u{000c}\u{000b}plus: +.5,\n\
         leadingDot: .25,\n\
         hexadecimal: 0x2a,\n\
         octal: 0o52,\n\
         binary: 0b101010,\n\
         notANumber: NaN,\n\
         positiveInfinity: +Infinity,\n\
         negativeInfinity: -Infinity,\n\
        }",
    )
    .unwrap();
    let filename = JsString::from_static("fixtures/value.data");
    let NativeConversion::Value(Value::Object(value)) = runtime
        .parse_json5_module_text(context.realm, &source, &filename)
        .unwrap()
    else {
        panic!("QuickJS extended JSON did not return its object value");
    };
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("__json5Value").unwrap();
    assert!(
        context
            .set_property(&global, &key, Value::Object(value))
            .unwrap()
    );
    assert_eq!(
        context
            .eval(
                "const value = __json5Value;\n\
                 value.bare === 'single quoted' &&\n\
                 value.verticalEscape.length === 3 &&\n\
                 value.verticalEscape.charCodeAt(1) === 11 &&\n\
                 value.continued === 'leftright' &&\n\
                 value.trailing.join(',') === '1,2' &&\n\
                 value.plus === 0.5 && value.leadingDot === 0.25 &&\n\
                 value.hexadecimal === 42 && value.octal === 42 &&\n\
                 value.binary === 42 && Number.isNaN(value.notANumber) &&\n\
                 value.positiveInfinity === Infinity &&\n\
                 value.negativeInfinity === -Infinity",
            )
            .unwrap(),
        Value::Bool(true)
    );

    let NativeConversion::Throw(Value::Object(error)) = runtime
        .parse_json_module_text(context.realm, &source, &filename)
        .unwrap()
    else {
        panic!("strict JSON unexpectedly accepted QuickJS extended JSON");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("unexpected token: '/'"))
    );

    let line_separator = JsString::try_from_utf8("// comment\u{2028}{answer: 42}").unwrap();
    let NativeConversion::Value(Value::Object(value)) = runtime
        .parse_json5_module_text(context.realm, &line_separator, &filename)
        .unwrap()
    else {
        panic!("extended JSON line comment did not consume its Unicode terminator");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        context.get_property(&value, &answer).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn json_module_parser_reports_pinned_quickjs_token_locations() {
    let cases = [
        ("{\n  notJson: 0\n}\n", "expecting property name", 2, 3),
        (r#""a\q""#, "Bad escaped character", 1, 4),
        ("\"a\nb\"", "Bad control character in string literal", 1, 3),
        ("01", "Unexpected number", 1, 1),
        ("1.", "Unterminated fractional number", 1, 3),
        ("1e+", "Exponent part is missing a number", 1, 4),
        ("{\"a\":\0}", "unexpected token: ''", 1, 6),
        ("-é", "Unexpected token '�'", 1, 2),
        ("true false", "unexpected data at the end", 1, 6),
        ("{} \"unterminated", "Unexpected end of JSON input", 1, 4),
        ("\"😀\" x", "unexpected data at the end", 1, 5),
    ];

    for (source, expected_message, expected_line, expected_column) in cases {
        assert_json_module_syntax_location(
            source,
            expected_message,
            expected_line,
            expected_column,
        );
    }
}

#[test]
fn json_parse_prepends_pinned_input_location_to_the_active_backtrace() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval_with_filename(
            r#"
                (function authored() {
                    try {
                        JSON.parse('\n"  \\x"');
                    } catch (error) {
                        return [
                            error.name,
                            error.message,
                            error.fileName,
                            error.lineNumber,
                            error.columnNumber,
                            error.stack
                        ];
                    }
                })()
            "#,
            "json-callsite.js",
        )
        .unwrap();
    let Value::Object(result) = result else {
        panic!("JSON.parse diagnostic probe did not return its result array");
    };

    for (index, expected) in [
        Value::String(JsString::from_static("SyntaxError")),
        Value::String(JsString::from_static("Bad escaped character")),
        Value::String(JsString::from_static("<input>")),
        Value::Int(2),
        Value::Int(5),
    ]
    .into_iter()
    .enumerate()
    {
        let key = runtime.intern_property_key(&index.to_string()).unwrap();
        assert_eq!(context.get_property(&result, &key).unwrap(), expected);
    }

    let stack_key = runtime.intern_property_key("5").unwrap();
    let Value::String(stack) = context.get_property(&result, &stack_key).unwrap() else {
        panic!("JSON.parse SyntaxError stack was not a string");
    };
    let stack = stack.to_string();
    assert!(
        stack.starts_with("    at <input>:2:5\n    at parse (native)\n"),
        "JSON.parse stack lost its pinned synthetic source frame: {stack:?}",
    );
    assert!(
        stack.contains("    at authored (json-callsite.js:"),
        "JSON.parse stack lost its authored caller: {stack:?}",
    );
}

#[test]
fn quickjs_extended_json_module_parser_reports_pinned_negative_boundaries() {
    let cases = [
        ("{é: 1}", "unexpected character", 1, 2),
        ("{aé: 1}", "unexpected character", 1, 3),
        ("{a:\0}", "unexpected token: ''", 1, 4),
        ("{\"value\": 1.}", "Unterminated fractional number", 1, 13),
        (
            "{'value':'left\\\rright'}\n",
            "Bad escaped character",
            1,
            16,
        ),
        (
            "{'value':'left\\\r\nright'}\n",
            "Bad escaped character",
            1,
            16,
        ),
        ("0x", "Unexpected token '", 1, 3),
        ("+0o", "Unexpected token '", 1, 4),
        ("-0b", "Unexpected token '", 1, 4),
        ("{a:0xé}", "Unexpected token '�'", 1, 6),
        ("{a:+é}", "Unexpected token '�'", 1, 5),
        ("[1 2.]", "Unterminated fractional number", 1, 6),
    ];

    for (source, expected_message, expected_line, expected_column) in cases {
        assert_json5_module_syntax_location(
            source,
            expected_message,
            expected_line,
            expected_column,
        );
    }
}

fn assert_json_module_syntax_location(
    source: &str,
    expected_message: &str,
    expected_line: i32,
    expected_column: i32,
) {
    assert_json_module_syntax_location_with_mode(
        source,
        expected_message,
        expected_line,
        expected_column,
        false,
    );
}

fn assert_json5_module_syntax_location(
    source: &str,
    expected_message: &str,
    expected_line: i32,
    expected_column: i32,
) {
    assert_json_module_syntax_location_with_mode(
        source,
        expected_message,
        expected_line,
        expected_column,
        true,
    );
}

fn assert_json_module_syntax_location_with_mode(
    source: &str,
    expected_message: &str,
    expected_line: i32,
    expected_column: i32,
    extended: bool,
) {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = JsString::try_from_utf8(source).unwrap();
    let filename = JsString::from_static("fixtures/value.json");
    let parsed = if extended {
        runtime.parse_json5_module_text(context.realm, &source, &filename)
    } else {
        runtime.parse_json_module_text(context.realm, &source, &filename)
    };
    let NativeConversion::Throw(Value::Object(error)) = parsed.unwrap() else {
        panic!("invalid JSON module text did not throw a SyntaxError");
    };

    for (name, expected) in [
        (
            "message",
            Value::String(JsString::try_from_utf8(expected_message).unwrap()),
        ),
        ("fileName", Value::String(filename.clone())),
        ("lineNumber", Value::Int(expected_line)),
        ("columnNumber", Value::Int(expected_column)),
        (
            "stack",
            Value::String(
                JsString::try_from_utf8(&format!(
                    "    at fixtures/value.json:{expected_line}:{expected_column}\n"
                ))
                .unwrap(),
            ),
        ),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(
            context.get_property(&error, &key).unwrap(),
            expected,
            "{name} differed for {source:?}",
        );
    }
}
