use super::*;

#[test]
fn function_debug_accessors_match_quickjs_descriptors_realms_and_receivers() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let function_prototype = first.function_prototype().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let specs = [
        (
            "fileName",
            "get fileName",
            NativeFunctionId::FunctionPrototypeFileName,
            NativeCProto::Getter,
        ),
        (
            "lineNumber",
            "get lineNumber",
            NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Line),
            NativeCProto::GetterMagic,
        ),
        (
            "columnNumber",
            "get columnNumber",
            NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Column),
            NativeCProto::GetterMagic,
        ),
    ];
    let mut getters = Vec::new();
    for (property_name, getter_name, target, cproto) in specs {
        let key = runtime.intern_property_key(property_name).unwrap();
        let CompleteOrdinaryPropertyDescriptor::Accessor {
            get: Some(getter),
            set: None,
            enumerable: false,
            configurable: true,
        } = runtime
            .get_own_property(&function_prototype, &key)
            .unwrap()
            .unwrap()
        else {
            panic!("{property_name} was not the expected getter-only accessor");
        };
        assert_eq!(
            runtime.get_prototype_of(getter.as_object()).unwrap(),
            Some(function_prototype.clone())
        );
        assert_eq!(runtime.callable_realm(&getter).unwrap(), first.realm);
        assert!(!runtime.is_constructor(getter.as_object()).unwrap());
        assert_eq!(
            runtime
                .get_own_property(getter.as_object(), &prototype_key)
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime
                .get_own_property(getter.as_object(), &length_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(0),
                writable: false,
                enumerable: false,
                configurable: true,
            })
        ));
        assert!(matches!(
            runtime
                .get_own_property(getter.as_object(), &name_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::try_from_utf8(getter_name).unwrap()
        ));
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(getter.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("debug accessor getter was not a native function");
        };
        assert_eq!(data.target, target);
        assert_eq!(data.target.descriptor().cproto, cproto);
        drop(state);
        getters.push((key, getter));
    }
    assert_ne!(getters[0].1.as_object(), getters[1].1.as_object());
    assert_ne!(getters[1].1.as_object(), getters[2].1.as_object());

    let source = "\n  (function named(){})";
    let Value::Object(function) = second.eval_with_filename(source, "receiver.js").unwrap() else {
        panic!("debug receiver source did not return a function");
    };
    for (index, expected) in [
        Value::String(JsString::from_static("receiver.js")),
        Value::Int(2),
        Value::Int(4),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            first.get_property(&function, &getters[index].0).unwrap(),
            expected
        );
        assert_eq!(
            first
                .call(
                    &getters[index].1,
                    Value::Object(function.clone()),
                    &[Value::Int(1), Value::Int(2)],
                )
                .unwrap(),
            expected,
            "getter ABI must ignore arguments and inspect the receiver"
        );
    }

    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind_object) = first.get_property(&function_prototype, &bind_key).unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(bound) = first
        .call(&bind, Value::Object(function.clone()), &[Value::Null])
        .unwrap()
    else {
        panic!("bind did not return an object");
    };
    let ordinary = first.new_object().unwrap();
    for (_, getter) in &getters {
        for receiver in [
            Value::Undefined,
            Value::Null,
            Value::Int(1),
            Value::Object(ordinary.clone()),
            Value::Object(function_prototype.clone()),
            Value::Object(bound.clone()),
            Value::Object(getter.as_object().clone()),
        ] {
            assert_eq!(first.call(getter, receiver, &[]).unwrap(), Value::Undefined);
        }
    }

    // If an embedder enables the constructor bit, QuickJS's getter cproto
    // receives newTarget as its receiver.
    let target = runtime.as_callable(&function).unwrap().unwrap();
    runtime
        .set_constructor_bit(getters[0].1.as_object(), true)
        .unwrap();
    assert_eq!(
        first
            .construct_with_new_target(&getters[0].1, &target, &[])
            .unwrap(),
        Value::String(JsString::from_static("receiver.js"))
    );
    runtime
        .set_constructor_bit(getters[0].1.as_object(), false)
        .unwrap();
}

#[test]
fn runtime_debug_info_mode_matches_quickjs_strip_source_and_strip_debug() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let keys = ["fileName", "lineNumber", "columnNumber"]
        .map(|name| runtime.intern_property_key(name).unwrap());
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let Value::Object(to_string_object) = context
        .get_property(&function_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();
    let expression = "\n  (function stripped() {})";

    let Value::Object(full) = context.eval_with_filename(expression, "full.js").unwrap() else {
        panic!("full debug compile did not return a function");
    };
    assert_eq!(runtime.debug_info_mode(), DebugInfoMode::Full);
    assert_eq!(
        context
            .call(&to_string, Value::Object(full.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("function stripped() {}"))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let Value::Object(source_stripped) = context
        .eval_with_filename(expression, "source-stripped.js")
        .unwrap()
    else {
        panic!("source-stripped compile did not return a function");
    };
    for (key, expected) in keys.iter().zip([
        Value::String(JsString::from_static("source-stripped.js")),
        Value::Int(2),
        Value::Int(4),
    ]) {
        assert_eq!(
            context.get_property(&source_stripped, key).unwrap(),
            expected
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function stripped() {\n    [native code]\n}"
        ))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let Value::Object(debug_stripped) = context
        .eval_with_filename(expression, "debug-stripped.js")
        .unwrap()
    else {
        panic!("debug-stripped compile did not return a function");
    };
    for key in &keys {
        assert_eq!(
            context.get_property(&debug_stripped, key).unwrap(),
            Value::Undefined
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(debug_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function stripped() {\n    [native code]\n}"
        ))
    );

    // Changing the runtime policy never mutates already-published bytecode.
    assert_eq!(
        context.call(&to_string, Value::Object(full), &[]).unwrap(),
        Value::String(JsString::from_static("function stripped() {}"))
    );
}

#[test]
fn function_debug_position_distinguishes_missing_debug_from_missing_pc_table() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let keys = ["fileName", "lineNumber", "columnNumber"]
        .map(|name| runtime.intern_property_key(name).unwrap());

    let with_debug = runtime
        .publish_unlinked_function(
            context.realm,
            debug_draft(UnlinkedFunctionDebug {
                filename: JsString::from_static("no-pc-table.js"),
                pc2line: None,
                source: None,
            }),
        )
        .unwrap();
    let with_debug = runtime
        .new_bytecode_closure(context.realm, &with_debug)
        .unwrap();
    for (key, expected) in keys.iter().zip([
        Value::String(JsString::from_static("no-pc-table.js")),
        Value::Int(0),
        Value::Int(0),
    ]) {
        assert_eq!(
            context.get_property(with_debug.as_object(), key).unwrap(),
            expected
        );
    }

    let without_debug = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let without_debug = runtime
        .new_bytecode_closure(context.realm, &without_debug)
        .unwrap();
    for key in &keys {
        assert_eq!(
            context
                .get_property(without_debug.as_object(), key)
                .unwrap(),
            Value::Undefined
        );
    }
}

#[test]
fn publication_accepts_arbitrary_debug_source_bytes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source =
        b"function f(){/*\x80Q*/\0\xef\xbb\xbf\xed\xa0\xbd\xed\xb8\x80\xed\xa0\x80\xff}".to_vec();
    let function = runtime
        .publish_unlinked_function(
            context.realm,
            debug_draft(UnlinkedFunctionDebug {
                filename: JsString::from_static("raw-source.js"),
                pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
                source: Some(source.clone().into_boxed_slice()),
            }),
        )
        .unwrap();

    assert_eq!(
        runtime.test_function_debug_source(&function).unwrap(),
        Some(source.clone())
    );

    let callable = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
    let actual = context
        .call(&to_string, Value::Object(callable.as_object().clone()), &[])
        .unwrap();
    let expected = JsString::try_from_bytes(&source).unwrap();
    assert_eq!(actual, Value::String(expected.clone()));
    let units = expected.utf16_units().collect::<Vec<_>>();
    assert!(units.contains(&0xfffd));
    assert!(!units.contains(&u16::from(b'Q')));
    assert!(units.contains(&0));
    assert!(units.contains(&0xd800));
}

#[test]
fn publication_rejects_malformed_debug_pc_order_range_and_position() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    let malformed = [
        UnlinkedFunctionDebug {
            filename: JsString::from_static("range.js"),
            pc2line: Some(Pc2LineTable::new(
                LineColumn::new(0, 0),
                vec![Pc2LineEntry {
                    pc: 2,
                    position: LineColumn::new(0, 0),
                }],
            )),
            source: None,
        },
        UnlinkedFunctionDebug {
            filename: JsString::from_static("order.js"),
            pc2line: Some(Pc2LineTable::new(
                LineColumn::new(0, 0),
                vec![
                    Pc2LineEntry {
                        pc: 1,
                        position: LineColumn::new(0, 1),
                    },
                    Pc2LineEntry {
                        pc: 0,
                        position: LineColumn::new(0, 0),
                    },
                ],
            )),
            source: None,
        },
        UnlinkedFunctionDebug {
            filename: JsString::from_static("position.js"),
            pc2line: Some(Pc2LineTable::new(LineColumn::new(u32::MAX, 0), Vec::new())),
            source: None,
        },
    ];

    for debug in malformed {
        assert!(
            runtime
                .publish_unlinked_function(context.realm, debug_draft(debug))
                .is_err()
        );
        assert_eq!(
            runtime.heap_counts().function_bytecode_nodes,
            baseline,
            "malformed debug metadata changed the heap"
        );
    }
}

#[test]
fn publication_keeps_duplicate_last_and_unreachable_pc_metadata() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let duplicate = debug_draft(UnlinkedFunctionDebug {
        filename: JsString::from_static("duplicate.js"),
        pc2line: Some(Pc2LineTable::new(
            LineColumn::new(0, 0),
            vec![
                Pc2LineEntry {
                    pc: 0,
                    position: LineColumn::new(1, 1),
                },
                Pc2LineEntry {
                    pc: 0,
                    position: LineColumn::new(2, 2),
                },
            ],
        )),
        source: None,
    });
    let duplicate = runtime
        .publish_unlinked_function(context.realm, duplicate)
        .unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&duplicate, Some(0))
            .unwrap(),
        Some((JsString::from_static("duplicate.js"), LineColumn::new(2, 2)))
    );

    let unreachable = UnlinkedFunction::fixture(
        vec![
            Instruction::Goto(2),
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_debug(UnlinkedFunctionDebug {
        filename: JsString::from_static("unreachable.js"),
        pc2line: Some(Pc2LineTable::new(
            LineColumn::new(0, 0),
            vec![Pc2LineEntry {
                pc: 1,
                position: LineColumn::new(9, 4),
            }],
        )),
        source: None,
    });
    let unreachable = runtime
        .publish_unlinked_function(context.realm, unreachable)
        .unwrap();
    assert_eq!(
        runtime
            .test_function_debug_location(&unreachable, Some(1))
            .unwrap(),
        Some((
            JsString::from_static("unreachable.js"),
            LineColumn::new(9, 4)
        ))
    );
}

#[test]
fn publication_rollback_releases_the_new_debug_filename_atom() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let stale_realm = context.realm;
    drop(context);
    runtime.run_gc().unwrap();
    let baseline_atoms = runtime.test_atom_count();
    let baseline_bytecode = runtime.heap_counts().function_bytecode_nodes;

    let function = debug_draft(UnlinkedFunctionDebug {
        filename: JsString::from_static("rollback-debug-filename.js"),
        pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
        source: None,
    });
    assert!(
        runtime
            .publish_unlinked_function(stale_realm, function)
            .is_err()
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline_bytecode
    );
}
