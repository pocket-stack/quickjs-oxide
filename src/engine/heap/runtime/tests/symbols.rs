use super::*;

#[test]
fn symbol_intrinsic_graph_registry_brand_and_wrapper_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = global_callable(&runtime, &mut context, "Symbol");
    let prototype = context.symbol_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Ordinary
    ));
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        [
            "length",
            "name",
            "for",
            "keyFor",
            "toPrimitive",
            "iterator",
            "match",
            "matchAll",
            "replace",
            "search",
            "split",
            "toStringTag",
            "isConcatSpreadable",
            "hasInstance",
            "species",
            "unscopables",
            "asyncIterator",
            "prototype",
        ]
    );
    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "toString",
            "valueOf",
            "description",
            "constructor",
            "Symbol.toPrimitive",
            "Symbol.toStringTag",
        ]
    );

    let to_primitive_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    assert!(matches!(
        runtime
            .get_own_property(&prototype, &to_primitive_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(_),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    let description_key = runtime.intern_property_key("description").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(description_getter),
        set: None,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&prototype, &description_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Symbol.prototype.description was not a getter-only accessor");
    };
    assert_eq!(
        runtime
            .get_prototype_of(description_getter.as_object())
            .unwrap(),
        Some(context.function_prototype().unwrap())
    );

    let Value::Symbol(no_description) = context.call(&constructor, Value::Undefined, &[]).unwrap()
    else {
        panic!("Symbol() did not return a Symbol primitive");
    };
    let Value::Symbol(empty_description) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static(""))],
        )
        .unwrap()
    else {
        panic!("Symbol(\"\") did not return a Symbol primitive");
    };
    assert_ne!(no_description, empty_description);
    assert_eq!(runtime.symbol_description(&no_description).unwrap(), None);
    assert_eq!(
        runtime.symbol_description(&empty_description).unwrap(),
        Some(JsString::from_static(""))
    );
    let ignored_argument = context.new_object().unwrap();
    assert!(matches!(
        context.construct(&constructor, &[Value::Object(ignored_argument)]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("Symbol is not a constructor")
    );

    let symbol_for = property_callable(&runtime, &mut context, constructor.as_object(), "for");
    let key_for = property_callable(&runtime, &mut context, constructor.as_object(), "keyFor");
    let registry_key = Value::String(JsString::from_static("registry"));
    let first_registered = context
        .call(
            &symbol_for,
            Value::Undefined,
            std::slice::from_ref(&registry_key),
        )
        .unwrap();
    let second_registered = context
        .call(
            &symbol_for,
            Value::Null,
            std::slice::from_ref(&registry_key),
        )
        .unwrap();
    assert_eq!(first_registered, second_registered);
    assert_eq!(
        context
            .call(
                &key_for,
                Value::Undefined,
                std::slice::from_ref(&first_registered),
            )
            .unwrap(),
        registry_key
    );
    assert_eq!(
        context
            .call(
                &key_for,
                Value::Undefined,
                &[Value::Symbol(no_description.clone())],
            )
            .unwrap(),
        Value::Undefined
    );

    for (name, symbol) in [
        ("toPrimitive", WellKnownSymbol::ToPrimitive),
        ("iterator", WellKnownSymbol::Iterator),
        ("match", WellKnownSymbol::Match),
        ("matchAll", WellKnownSymbol::MatchAll),
        ("replace", WellKnownSymbol::Replace),
        ("search", WellKnownSymbol::Search),
        ("split", WellKnownSymbol::Split),
        ("toStringTag", WellKnownSymbol::ToStringTag),
        ("isConcatSpreadable", WellKnownSymbol::IsConcatSpreadable),
        ("hasInstance", WellKnownSymbol::HasInstance),
        ("species", WellKnownSymbol::Species),
        ("unscopables", WellKnownSymbol::Unscopables),
        ("asyncIterator", WellKnownSymbol::AsyncIterator),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime
                .get_own_property(constructor.as_object(), &key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Symbol(value),
                writable: false,
                enumerable: false,
                configurable: false,
            }) if value == runtime.well_known_symbol(symbol)
        ));
    }

    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    assert_eq!(
        context
            .call(&to_string, Value::Symbol(empty_description.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("Symbol()"))
    );
    assert_eq!(
        context
            .call(&value_of, Value::Symbol(no_description.clone()), &[],)
            .unwrap(),
        Value::Symbol(no_description.clone())
    );

    let object_prototype = context.object_prototype().unwrap();
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let Value::Object(wrapper) = context
        .call(&object_value_of, Value::Symbol(no_description.clone()), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box a Symbol primitive");
    };
    assert_eq!(runtime.get_prototype_of(&wrapper).unwrap(), Some(prototype));
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom))
            if atom == &no_description.atom()
    ));
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper), &[])
            .unwrap(),
        Value::Symbol(no_description)
    );
    assert_eq!(
        context.eval("Symbol('source').toString()").unwrap(),
        Value::String(JsString::from_static("Symbol(source)"))
    );
    assert_eq!(
        context.eval("Symbol('source').description").unwrap(),
        Value::String(JsString::from_static("source"))
    );
    assert_eq!(
        context.eval("Symbol().description").unwrap(),
        Value::Undefined
    );
}

#[test]
fn symbol_wrapper_owns_its_atom_and_realm_until_final_collection() {
    let runtime = Runtime::new();
    let (atom, wrapper) = {
        let mut context = runtime.new_context();
        let object_prototype = context.object_prototype().unwrap();
        let object_value_of =
            property_callable(&runtime, &mut context, &object_prototype, "valueOf");
        let symbol = runtime
            .new_symbol(Some(JsString::from_static("wrapper-only")))
            .unwrap();
        let atom = symbol.atom();
        let Value::Object(wrapper) = context
            .call(&object_value_of, Value::Symbol(symbol), &[])
            .unwrap()
        else {
            panic!("Object.prototype.valueOf did not return a Symbol wrapper");
        };
        (atom, wrapper)
    };

    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "the live wrapper must retain its Symbol prototype realm graph"
    );
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .atoms
            .resolve(atom)
            .unwrap()
            .ref_count,
        Some(1),
        "the wrapper must own the local symbol atom after the primitive root drops"
    );

    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
    assert!(
        runtime.0.state.borrow().atoms.resolve(atom).is_err(),
        "the final wrapper release must return its symbol atom ownership"
    );
}

#[test]
fn symbols_are_runtime_owned_and_distinct_from_registry_entries() {
    let runtime = Runtime::new();
    let name = JsString::from_static("Symbol.iterator");
    let unique = runtime.well_known_symbol(WellKnownSymbol::Iterator);
    let repeated = runtime.well_known_symbol(WellKnownSymbol::Iterator);
    let registry = runtime.symbol_for(&name).unwrap();
    assert_eq!(unique, repeated);
    assert_ne!(unique, registry);
    assert_ne!(PropertyKey::from(&unique), PropertyKey::from(&registry));
    assert_eq!(runtime.symbol_key_for(&unique).unwrap(), None);
    assert_eq!(runtime.symbol_key_for(&registry).unwrap(), Some(name));
}
