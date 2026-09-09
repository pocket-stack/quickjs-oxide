use super::*;

#[test]
fn program_lexicals_lower_to_source_ordered_global_declarations() {
    let source = "let first=1,second=function(){return later};const later=2;first+second()";
    let script = compile_unlinked_script(source).unwrap();
    assert_eq!(script.local_definitions().len(), 1);
    assert_eq!(script.closure_variables().len(), 3);

    let declaration_names = script
        .closure_variables()
        .iter()
        .map(|descriptor| {
            assert_eq!(descriptor.source, ClosureSource::GlobalDeclaration);
            assert!(descriptor.is_lexical);
            let ClosureVariableName::Constant(index) = descriptor.name else {
                panic!("global declaration lost its semantic name");
            };
            let crate::engine::value::PrimitiveValue::String(name) = script.constants()
                [index as usize]
                .as_primitive()
                .expect("global declaration name is not primitive")
            else {
                panic!("global declaration name is not a string");
            };
            name.to_utf8_lossy()
        })
        .collect::<Vec<_>>();
    assert_eq!(declaration_names, ["first", "second", "later"]);
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::PutVarInit(_)))
            .count(),
        3
    );

    let child = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("global lexical initializer lost its child function");
    assert_eq!(child.closure_variables().len(), 1);
    assert_eq!(
        child.closure_variables()[0].source,
        ClosureSource::ParentGlobal(2)
    );
    assert!(child.closure_variables()[0].is_lexical);
    assert!(child.closure_variables()[0].is_const);

    let stripped =
        compile_unlinked_script_with_filename(source, "<global-strip>", DebugInfoMode::StripDebug)
            .unwrap();
    assert!(stripped.closure_variables().iter().all(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && matches!(descriptor.name, ClosureVariableName::Constant(_))
    }));
    let stripped_child = stripped
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("stripped global lexical lost its child function");
    assert!(matches!(
        stripped_child.closure_variables()[0].name,
        ClosureVariableName::Constant(_)
    ));
}

#[test]
fn program_vars_keep_every_source_ordered_global_declaration() {
    let source = "var first;{var first;var second=function(){return later}}if(false)var later=3;for(var loop=0;false;){}";
    let script = compile_unlinked_script(source).unwrap();
    assert_eq!(script.local_definitions().len(), 1);

    let declaration_names = script
        .closure_variables()
        .iter()
        .map(|descriptor| {
            assert_eq!(descriptor.source, ClosureSource::GlobalDeclaration);
            assert!(!descriptor.is_lexical);
            assert!(!descriptor.is_const);
            assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
            let ClosureVariableName::Constant(index) = descriptor.name else {
                panic!("global var declaration lost its semantic name");
            };
            let crate::engine::value::PrimitiveValue::String(name) = script.constants()
                [index as usize]
                .as_primitive()
                .expect("global var declaration name is not primitive")
            else {
                panic!("global var declaration name is not a string");
            };
            name.to_utf8_lossy()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        declaration_names,
        ["first", "first", "second", "later", "loop"]
    );
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::PutVar(_)))
            .count(),
        3
    );
    assert!(
        !script
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutVarInit(_)))
    );

    let child = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("global var initializer lost its child function");
    assert_eq!(child.closure_variables().len(), 1);
    assert_eq!(
        child.closure_variables()[0].source,
        ClosureSource::ParentGlobal(3)
    );
    assert!(!child.closure_variables()[0].is_lexical);

    let stripped = compile_unlinked_script_with_filename(
        source,
        "<global-var-strip>",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(stripped.closure_variables().iter().all(|descriptor| {
        descriptor.source == ClosureSource::GlobalDeclaration
            && matches!(descriptor.name, ClosureVariableName::Constant(_))
    }));
}

#[test]
fn program_functions_keep_descriptors_but_hoist_into_the_first_name_slot() {
    let script = compile_unlinked_script(
        "let mixed;function mixed(){return mixed}function repeated(){return 1}function repeated(){return 2}repeated",
    )
    .unwrap();
    assert_eq!(script.closure_variables().len(), 4);
    assert!(script.closure_variables()[0].is_lexical);
    assert_eq!(
        script.closure_variables()[1].kind,
        ClosureVariableKind::GlobalFunction
    );
    assert_eq!(
        script.closure_variables()[2].kind,
        ClosureVariableKind::GlobalFunction
    );
    assert_eq!(
        script.closure_variables()[3].kind,
        ClosureVariableKind::GlobalFunction
    );
    assert!(matches!(script.code()[0], Instruction::FClosure(0)));
    assert!(matches!(script.code()[1], Instruction::PutVarInit(0)));
    assert!(matches!(script.code()[2], Instruction::FClosure(1)));
    assert!(matches!(script.code()[3], Instruction::PutVarInit(2)));
    assert!(matches!(script.code()[4], Instruction::FClosure(2)));
    assert!(matches!(script.code()[5], Instruction::PutVarInit(2)));

    let children = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].metadata().function_name_local, None);
    assert_eq!(
        children[0].closure_variables()[0].source,
        ClosureSource::ParentGlobal(0)
    );
    assert!(children[0].closure_variables()[0].is_lexical);
    for child in &children[1..] {
        assert_eq!(child.metadata().function_name_local, None);
    }
}

#[test]
fn program_function_then_lexical_remains_a_source_ordered_syntax_error() {
    let error = compile_unlinked_script("function clash(){};let clash").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "invalid redefinition of global identifier");
    assert!(compile_unlinked_script("let clash;function clash(){}").is_ok());
}

#[test]
fn program_vars_instantiate_persist_and_preserve_existing_properties() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("var value;var value=2;if(false){var dormant=3}for(var loop=0;loop<2;loop++){};value+'|'+typeof dormant+'|'+loop")
            .unwrap(),
        Value::String(JsString::from_static("2|undefined|2"))
    );
    let global = context.global_object().unwrap();
    for (name, value) in [
        ("value", Value::Int(2)),
        ("dormant", Value::Undefined),
        ("loop", Value::Int(2)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(
            context.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable: true,
                enumerable: true,
                configurable: false,
            })
        );
    }
    assert_eq!(context.eval("var value;value").unwrap(), Value::Int(2));
    assert_eq!(
        context
            .eval("var captured=1,read=function(){return captured};captured=4;read()")
            .unwrap(),
        Value::Int(4)
    );
    assert_eq!(context.eval("delete value").unwrap(), Value::Bool(false));

    assert_eq!(context.eval("hostValue=7").unwrap(), Value::Int(7));
    let host = runtime.intern_property_key("hostValue").unwrap();
    assert_eq!(
        context.eval("var hostValue;hostValue").unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        context.get_own_property(&global, &host).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(7),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    );
    assert_eq!(
        context
            .eval("Function.hostRead=function(){return hostValue};var hostValue=8;delete hostValue")
            .unwrap(),
        Value::Bool(true)
    );
    assert!(matches!(
        context.eval("Function.hostRead()"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.eval("var hostValue;hostValue").unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context.eval("Function.hostRead()").unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context.get_own_property(&global, &host).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: true,
            configurable: false,
        })
    );

    let fixed = runtime.intern_property_key("fixedVar").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &fixed,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(false),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("var fixedVar;fixedVar").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("var fixedVar=2;fixedVar").unwrap(),
        Value::Int(1)
    );
    assert!(matches!(
        context.eval("'use strict';var fixedVar=2"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        context.get_own_property(&global, &fixed).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(1),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    );

    assert_eq!(
        context.eval("Function.varSetterHits=0").unwrap(),
        Value::Int(0)
    );
    let Value::Object(setter) = context
        .eval("(function(value){Function.varSetterHits=value})")
        .unwrap()
    else {
        panic!("global var accessor probe did not create a setter");
    };
    let setter = runtime.as_callable(&setter).unwrap().unwrap();
    let accessor = runtime.intern_property_key("accessorVar").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &accessor,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Undefined),
                    set: DescriptorField::Present(AccessorValue::Callable(setter.clone())),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .eval("var accessorVar;Function.varSetterHits")
            .unwrap(),
        Value::Int(0),
        "a var without initializer must not invoke an existing setter"
    );
    assert_eq!(
        context
            .eval("var accessorVar=9;Function.varSetterHits")
            .unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        context.get_own_property(&global, &accessor).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor {
            get: None,
            set: Some(setter),
            enumerable: false,
            configurable: true,
        })
    );

    let mut inherited = runtime.new_context();
    let inherited_key = runtime.intern_property_key("inheritedVar").unwrap();
    let prototype = inherited.object_prototype().unwrap();
    assert!(
        inherited
            .define_own_property(
                &prototype,
                &inherited_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(5)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        inherited.eval("var inheritedVar;inheritedVar").unwrap(),
        Value::Undefined
    );
    let inherited_global = inherited.global_object().unwrap();
    assert_eq!(
        inherited
            .get_own_property(&inherited_global, &inherited_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: true,
            configurable: false,
        })
    );

    let mut auto_init = runtime.new_context();
    assert_eq!(
        auto_init.eval("var Number;typeof Number").unwrap(),
        Value::String(JsString::from_static("function"))
    );
    assert_eq!(
        auto_init.eval("var Number=9;Number").unwrap(),
        Value::Int(9)
    );
    let number = runtime.intern_property_key("Number").unwrap();
    let auto_init_global = auto_init.global_object().unwrap();
    assert_eq!(
        auto_init
            .get_own_property(&auto_init_global, &number)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(9),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    );
}

#[test]
fn program_var_preflight_conflicts_and_parser_scope_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("let existingLexical=1").unwrap(),
        Value::Undefined
    );
    assert!(matches!(
        context.eval("var existingLexical"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global var/lexical conflict did not throw an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("redeclaration of 'existingLexical'"))
    );
    assert_eq!(
        context.eval("globalThis.varMarker=0").unwrap(),
        Value::Int(0)
    );
    assert!(matches!(
        context.eval("varMarker=1;var freshBefore=(varMarker=2),existingLexical=(varMarker=3),freshAfter=(varMarker=4)"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();
    assert_eq!(context.eval("varMarker").unwrap(), Value::Int(0));
    assert_eq!(
        context
            .eval("typeof freshBefore+'|'+typeof freshAfter")
            .unwrap(),
        Value::String(JsString::from_static("undefined|undefined"))
    );

    let mut sealed = runtime.new_context();
    assert_eq!(
        sealed.eval("let sealedLexical=1").unwrap(),
        Value::Undefined
    );
    let sealed_global = sealed.global_object().unwrap();
    runtime.prevent_extensions(&sealed_global).unwrap();
    assert!(matches!(
        sealed.eval("var sealedLexical"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = sealed.take_exception().unwrap().unwrap() else {
        panic!("sealed global var declaration did not throw an Error object");
    };
    assert_eq!(
        sealed.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        sealed.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(
            "cannot define variable 'sealedLexical'"
        ))
    );

    let mut atomic = runtime.new_context();
    assert_eq!(
        atomic
            .eval("globalThis.atomicExisting=5;globalThis.atomicMarker=0")
            .unwrap(),
        Value::Int(0)
    );
    let atomic_global = atomic.global_object().unwrap();
    runtime.prevent_extensions(&atomic_global).unwrap();
    assert!(matches!(
        atomic.eval("atomicMarker=1;var atomicExisting=6,atomicMissing=7"),
        Err(RuntimeError::Exception)
    ));
    atomic.take_exception().unwrap().unwrap();
    assert_eq!(atomic.eval("atomicMarker").unwrap(), Value::Int(0));
    assert_eq!(atomic.eval("atomicExisting").unwrap(), Value::Int(5));
    assert_eq!(
        atomic.eval("typeof atomicMissing").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );

    for (source, expected) in [
        (
            "let conflict;var conflict",
            "invalid redefinition of lexical identifier",
        ),
        (
            "var conflict;let conflict",
            "invalid redefinition of global identifier",
        ),
        (
            "{var conflict;var conflict;let conflict}",
            "invalid redefinition of global identifier",
        ),
        (
            "{let conflict;var conflict}",
            "invalid redefinition of lexical identifier",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            expected,
            "{source}"
        );
    }
    for source in [
        "var allowed;{var allowed;let allowed}",
        "{var sibling}{var sibling;let sibling}",
        "{let shadow}var shadow",
    ] {
        compile_unlinked_script(source).unwrap();
    }
}

#[test]
fn program_var_cross_realm_instantiation_and_fallback_match_quickjs() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let fresh = defining.compile("var crossVar=41;crossVar+1").unwrap();
    assert_eq!(caller.execute(&fresh).unwrap(), Value::Int(42));
    assert_eq!(
        defining.eval("typeof crossVar").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(caller.eval("crossVar").unwrap(), Value::Int(41));

    assert_eq!(
        defining.eval("crossData='A'").unwrap(),
        Value::String(JsString::from_static("A"))
    );
    assert_eq!(
        caller.eval("crossData='B'").unwrap(),
        Value::String(JsString::from_static("B"))
    );
    let data = defining
        .compile("var crossData='written';crossData")
        .unwrap();
    assert_eq!(
        caller.execute(&data).unwrap(),
        Value::String(JsString::from_static("written"))
    );
    assert_eq!(
        defining.eval("crossData").unwrap(),
        Value::String(JsString::from_static("A"))
    );
    assert_eq!(
        caller.eval("crossData").unwrap(),
        Value::String(JsString::from_static("written"))
    );

    assert_eq!(
        defining.eval("Function.aSeen='none'").unwrap(),
        Value::String(JsString::from_static("none"))
    );
    assert_eq!(
        caller.eval("Function.bSeen='none'").unwrap(),
        Value::String(JsString::from_static("none"))
    );
    let Value::Object(a_getter) = defining.eval("(function(){return 'Aget'})").unwrap() else {
        panic!("defining realm accessor getter was not callable");
    };
    let Value::Object(a_setter) = defining
        .eval("(function(value){Function.aSeen=value})")
        .unwrap()
    else {
        panic!("defining realm accessor setter was not callable");
    };
    let Value::Object(b_getter) = caller.eval("(function(){return 'Bget'})").unwrap() else {
        panic!("caller realm accessor getter was not callable");
    };
    let Value::Object(b_setter) = caller
        .eval("(function(value){Function.bSeen=value})")
        .unwrap()
    else {
        panic!("caller realm accessor setter was not callable");
    };
    let a_getter = runtime.as_callable(&a_getter).unwrap().unwrap();
    let a_setter = runtime.as_callable(&a_setter).unwrap().unwrap();
    let b_getter = runtime.as_callable(&b_getter).unwrap().unwrap();
    let b_setter = runtime.as_callable(&b_setter).unwrap().unwrap();
    let accessor = runtime.intern_property_key("crossAccessor").unwrap();
    for (context, getter, setter) in [
        (&mut defining, a_getter, a_setter),
        (&mut caller, b_getter, b_setter),
    ] {
        let global = context.global_object().unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &accessor,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    let accessor_script = defining
        .compile("var crossAccessor='written';crossAccessor")
        .unwrap();
    assert_eq!(
        caller.execute(&accessor_script).unwrap(),
        Value::String(JsString::from_static("Aget"))
    );
    assert_eq!(
        defining.eval("crossAccessor+'|'+Function.aSeen").unwrap(),
        Value::String(JsString::from_static("Aget|written"))
    );
    assert_eq!(
        caller.eval("crossAccessor+'|'+Function.bSeen").unwrap(),
        Value::String(JsString::from_static("Bget|none"))
    );

    let readonly = runtime.intern_property_key("crossReadonly").unwrap();
    let defining_global = defining.global_object().unwrap();
    assert!(
        defining
            .define_own_property(
                &defining_global,
                &readonly,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    writable: DescriptorField::Present(false),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let caller_global = caller.global_object().unwrap();
    assert!(
        caller
            .define_own_property(
                &caller_global,
                &readonly,
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
    let Value::Object(defining_type_prototype) = defining.eval("TypeError.prototype").unwrap()
    else {
        panic!("defining TypeError.prototype was not an object");
    };
    let readonly_script = defining
        .compile("'use strict';var crossReadonly=2")
        .unwrap();
    assert!(matches!(
        caller.execute(&readonly_script),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm var initializer did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_prototype)
    );

    let mut syntax_caller = runtime.new_context();
    syntax_caller.eval("let crossConflict=1").unwrap();
    let Value::Object(syntax_prototype) = syntax_caller.eval("SyntaxError.prototype").unwrap()
    else {
        panic!("caller SyntaxError.prototype was not an object");
    };
    let conflict = defining.compile("var crossConflict").unwrap();
    assert!(matches!(
        syntax_caller.execute(&conflict),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = syntax_caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm var conflict did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(syntax_prototype)
    );

    let mut type_caller = runtime.new_context();
    let Value::Object(type_prototype) = type_caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };
    let type_global = type_caller.global_object().unwrap();
    runtime.prevent_extensions(&type_global).unwrap();
    let missing = defining.compile("var missingCrossVar").unwrap();
    assert!(matches!(
        type_caller.execute(&missing),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = type_caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm non-extensible var did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(type_prototype)
    );
}

#[test]
fn program_var_function_cell_cycle_is_collectable_after_context_drop() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval("var cycle=function(){return cycle};cycle()===cycle")
                .unwrap(),
            Value::Bool(true)
        );
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 1);
        assert!(counts.var_ref_nodes > 0);
        assert!(counts.function_bytecode_nodes > 0);
    }

    assert_eq!(runtime.heap_counts().context_nodes, 1);
    runtime.run_gc().unwrap();
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.var_ref_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
    assert_eq!(counts.live, 0);
}

#[test]
fn program_function_declaration_cycle_is_collectable_after_context_drop() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval("function declarationCycle(){return declarationCycle};declarationCycle()===declarationCycle")
                .unwrap(),
            Value::Bool(true)
        );
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 1);
        assert!(counts.var_ref_nodes > 0);
        assert!(counts.function_bytecode_nodes > 0);
    }

    assert_eq!(runtime.heap_counts().context_nodes, 1);
    runtime.run_gc().unwrap();
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.var_ref_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
    assert_eq!(counts.live, 0);
}

#[test]
fn program_global_lexicals_persist_shadow_and_reject_redeclaration() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("let mutable=1,named=function(){};const fixed=3;mutable+'|'+named.name+'|'+fixed")
            .unwrap(),
        Value::String(JsString::from_static("1|named|3"))
    );
    assert_eq!(context.eval("mutable+=2").unwrap(), Value::Int(3));
    assert_eq!(
        context.eval("typeof globalThis.mutable").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(context.eval("delete mutable").unwrap(), Value::Bool(false));

    let global = context.global_object().unwrap();
    let lexical_environment = context.global_var_object().unwrap();
    for (binding_name, expected_value, writable) in [
        ("mutable", Value::Int(3), true),
        ("fixed", Value::Int(3), false),
    ] {
        let key = runtime.intern_property_key(binding_name).unwrap();
        assert!(context.get_own_property(&global, &key).unwrap().is_none());
        assert_eq!(
            context
                .get_own_property(&lexical_environment, &key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: expected_value,
                writable,
                enumerable: true,
                configurable: true,
            })
        );
    }

    assert!(matches!(
        context.eval("fixed=4"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global const write did not throw an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'fixed' is read-only"))
    );

    assert!(matches!(
        context.eval("let mutable=9"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global lexical redeclaration did not throw an Error object");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("redeclaration of 'mutable'"))
    );

    assert_eq!(context.eval("shadowedGlobal=1").unwrap(), Value::Int(1));
    assert_eq!(
        context.eval("let shadowedGlobal=2;shadowedGlobal").unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context.eval("globalThis.shadowedGlobal").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("delete globalThis.shadowedGlobal").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(context.eval("shadowedGlobal").unwrap(), Value::Int(2));

    let mut sealed = runtime.new_context();
    let sealed_global = sealed.global_object().unwrap();
    runtime.prevent_extensions(&sealed_global).unwrap();
    assert_eq!(
        sealed
            .eval("let sealedLexical=6;const sealedConst=7;sealedLexical+sealedConst")
            .unwrap(),
        Value::Int(13)
    );
}

#[test]
fn program_global_lexical_preflight_and_failed_initializers_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();

    let delayed = context
        .compile("let delayedGlobal=7;delayedGlobal")
        .unwrap();
    assert_eq!(
        context.eval("typeof delayedGlobal").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(context.execute(&delayed).unwrap(), Value::Int(7));
    assert!(matches!(
        context.execute(&delayed),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    let atomic = context
        .compile("let untouched=function(){return Infinity},NaN=1,Infinity=2")
        .unwrap();
    assert!(matches!(
        context.execute(&atomic),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("global declaration preflight did not throw an Error object");
    };
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("redeclaration of 'NaN'"))
    );
    assert_eq!(
        context.eval("typeof untouched").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(
        context.eval("let untouched=4;untouched").unwrap(),
        Value::Int(4)
    );

    assert!(matches!(
        context.eval(
            "Function.saved=function(){return captured};let captured=(function(){throw 17})()"
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));
    assert!(matches!(
        context.eval("Function.saved()"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("declaring-script capture did not preserve the global lexical TDZ");
    };
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("captured is not initialized"))
    );
    assert!(matches!(
        context.eval("captured"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("later eval did not materialize a missing-global ReferenceError");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("ReferenceError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'captured' is not defined"))
    );
    assert_eq!(
        context.eval("typeof captured").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(context.eval("delete captured").unwrap(), Value::Bool(false));
    assert!(matches!(
        context.eval("let captured=1"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap().unwrap();

    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let caller_syntax_prototype = caller.eval("SyntaxError.prototype").unwrap();
    let conflict = defining.compile("let NaN=1").unwrap();
    assert!(matches!(
        caller.execute(&conflict),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm declaration conflict did not throw an Error object");
    };
    let Value::Object(caller_syntax_prototype) = caller_syntax_prototype else {
        panic!("SyntaxError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_syntax_prototype)
    );

    let cross_realm = defining
        .compile("let crossRealmBinding=41;crossRealmBinding+1")
        .unwrap();
    assert_eq!(caller.execute(&cross_realm).unwrap(), Value::Int(42));
    assert_eq!(
        defining.eval("typeof crossRealmBinding").unwrap(),
        Value::String(JsString::from_static("undefined"))
    );
    assert_eq!(caller.eval("crossRealmBinding").unwrap(), Value::Int(41));
}
