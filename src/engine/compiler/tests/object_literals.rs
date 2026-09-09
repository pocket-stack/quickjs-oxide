use super::*;

#[test]
fn object_literals_lower_quickjs_data_proto_computed_and_spread_paths() {
    let fixed = compile_unlinked_script("({a:1,if:2,'x':3,0x10:4,a:5})").unwrap();
    assert!(
        fixed
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Object))
    );
    assert_eq!(
        fixed
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::DefineField(_)))
            .count(),
        5
    );

    let shorthand = compile_unlinked_script("var value=1;({value})").unwrap();
    assert!(
        shorthand
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
    );

    let computed = compile_unlinked_script("({[1]:function(){}})").unwrap();
    assert!(computed.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(_),
            Instruction::SetNameComputed,
            Instruction::DefineArrayEl,
            Instruction::Drop
        ]
    )));
    assert!(
        computed
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ToPropKey))
    );

    let proto = compile_unlinked_script("({__proto__:null})").unwrap();
    assert!(
        proto
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetProto))
    );
    assert!(
        !proto
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
    );

    let spread = compile_unlinked_script("({a:1,...value,b:2})").unwrap();
    assert!(
        spread
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CopyDataProperties))
    );
}

#[test]
fn object_literal_methods_lower_fixed_and_computed_non_constructors() {
    let script = compile_unlinked_script("({a(value,){return value},['b'](){return this}})")
        .expect("concise methods compile");
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethod {
            kind: DefineMethodKind::Method,
            enumerable: true,
            ..
        }
    )));
    assert!(script.code().iter().any(|instruction| matches!(
        instruction,
        Instruction::DefineMethodComputed {
            kind: DefineMethodKind::Method,
            enumerable: true,
        }
    )));

    let methods = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0].metadata().argument_count, 1);
    for method in methods {
        assert!(!method.metadata().has_prototype);
        assert_eq!(method.metadata().constructor_kind, ConstructorKind::None);
    }
}

#[test]
fn object_literal_accessors_lower_fixed_and_computed_zero_one_arity_functions() {
    let script = compile_unlinked_script(
        "({get fixed(){return this},set fixed(value,){this.value=value},get ['computed'](){return arguments.length},set [1](value){return typeof new.target}})",
    )
    .expect("object literal accessors compile");

    let mut fixed_kinds = Vec::new();
    let mut computed_kinds = Vec::new();
    for instruction in script.code() {
        match instruction {
            Instruction::DefineMethod {
                kind, enumerable, ..
            } => {
                assert!(*enumerable);
                fixed_kinds.push(*kind);
            }
            Instruction::DefineMethodComputed { kind, enumerable } => {
                assert!(*enumerable);
                computed_kinds.push(*kind);
            }
            _ => {}
        }
    }
    assert_eq!(
        fixed_kinds,
        [DefineMethodKind::Getter, DefineMethodKind::Setter]
    );
    assert_eq!(
        computed_kinds,
        [DefineMethodKind::Getter, DefineMethodKind::Setter]
    );
    assert_eq!(
        script
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::ToPropKey))
            .count(),
        2
    );

    let accessors = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(accessors.len(), 4);
    assert_eq!(
        accessors
            .iter()
            .map(|accessor| accessor.metadata().argument_count)
            .collect::<Vec<_>>(),
        [0, 1, 0, 1]
    );
    for accessor in accessors {
        assert!(!accessor.metadata().has_prototype);
        assert_eq!(accessor.metadata().constructor_kind, ConstructorKind::None);
    }
}

#[test]
fn object_literal_super_references_authenticate_home_object_and_quickjs_stack_forms() {
    let script = compile_unlinked_script(
        "({get read(){return super.value},set write(value){super.value=value},call(key){return super[key]()},update(){return super.value++},remove(){return delete super.value},iterate(values){for(super.value of values){}}})",
    )
    .expect("direct object-method super properties compile");
    let methods = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 6);
    assert!(
        methods
            .iter()
            .all(|method| method.metadata().needs_home_object)
    );
    assert!(methods.iter().all(|method| {
        method.code().windows(4).any(|window| {
            matches!(
                window,
                [
                    Instruction::PushHomeObject,
                    Instruction::PutLocal(_),
                    Instruction::PushThis,
                    Instruction::PutLocal(_),
                ]
            )
        })
    }));
    assert!(methods.iter().all(|method| {
        method.code().windows(3).any(|window| {
            matches!(
                window,
                [
                    Instruction::GetLocal(_),
                    Instruction::GetLocal(_),
                    Instruction::GetSuper
                ]
            )
        })
    }));
    assert!(
        methods[0]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValue))
    );
    assert!(
        methods[1]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutSuperValue))
    );
    assert!(
        methods[2]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValueForCall))
    );
    assert!(
        methods[3]
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::Perm5, Instruction::PutSuperValue]))
    );
    assert!(
        methods[4]
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowDeleteSuper))
    );
    assert!(
        methods[5]
            .code()
            .windows(2)
            .any(|window| matches!(window, [Instruction::Rot4Left, Instruction::PutSuperValue]))
    );
}

#[test]
fn object_literal_arrow_super_relays_lexical_this_and_home_object() {
    let script = compile_unlinked_script(
        "({method(){return()=>()=>{super['value'];super.call();super.value=1;super.value+=1;super.value||=2;++super.value;super.value++;delete super.value}}})",
    )
    .expect("nested arrows inherit ObjectLiteral super properties");
    let method = script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("script lost its object method");
    let relay = method
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("method lost its first arrow");
    let inner = relay
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("first arrow lost its nested arrow");

    assert!(method.metadata().needs_home_object);
    assert!(method.metadata().super_allowed);
    assert!(!method.metadata().super_call_allowed);
    assert!(!relay.metadata().needs_home_object);
    assert!(relay.metadata().super_allowed);
    assert!(!relay.metadata().super_call_allowed);
    assert!(!inner.metadata().needs_home_object);
    assert!(inner.metadata().super_allowed);
    assert!(!inner.metadata().super_call_allowed);
    assert_eq!(method.metadata().local_count, 2);
    assert_eq!(relay.metadata().local_count, 0);
    assert_eq!(inner.metadata().local_count, 0);

    let [
        Instruction::PushHomeObject,
        Instruction::PutLocal(home_object),
        Instruction::PushThis,
        Instruction::PutLocal(this_value),
        ..,
    ] = method.code()
    else {
        panic!("method pseudo-binding prologue did not match QuickJS order");
    };
    assert_ne!(home_object, this_value);
    assert_eq!(relay.closure_variables().len(), 2);
    assert!(relay.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentLocal(*this_value)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert!(relay.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentLocal(*home_object)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert_eq!(inner.closure_variables().len(), 2);
    assert!(inner.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentClosure(0)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert!(inner.closure_variables().iter().any(|descriptor| {
        descriptor.source == ClosureSource::ParentClosure(1)
            && descriptor.kind == ClosureVariableKind::Normal
            && descriptor.name == ClosureVariableName::None
    }));
    assert!(
        inner
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetSuperValueForCall))
    );
    assert!(
        inner
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PutSuperValue))
    );
    assert!(
        inner
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowDeleteSuper))
    );

    assert_eq!(
        evaluate_in_context(
            r#"
                (function () {
                    var log = "";
                    var proto = {
                        method(addend) { log += "c"; return this.count + addend; },
                        get value() { log += "g"; return this.count; },
                        set value(input) { log += "s"; this.count = input; }
                    };
                    var home = {
                        __proto__: proto,
                        count: 38,
                        make(key) {
                            return () => () => {
                                var read = super[key];
                                var call = super.method(2);
                                var assigned = super.value = 39;
                                var compound = super.value += 1;
                                var logical = super.value ||= 99;
                                var post = super.value++;
                                var pre = ++super.value;
                                var deleted;
                                try { delete super.value; }
                                catch (error) { deleted = error.name; }
                                return [
                                    read, call, assigned, compound, logical,
                                    post, pre, this.count, deleted, log
                                ].join("|");
                            };
                        }
                    };
                    var relay = home.make("value").call({ count: 100 });
                    return relay.call({ count: 200 });
                })()
            "#,
        ),
        Value::String(JsString::from_static(
            "38|40|39|40|40|40|42|42|ReferenceError|gcsgsggsgs"
        ))
    );

    let error =
        compile_unlinked_script("({method(){return()=>function ordinary(){return super.value}}})")
            .expect_err("ordinary functions must truncate inherited super capability");
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "'super' is only valid in a method");
}

#[test]
fn object_literal_direct_eval_super_relays_authenticated_home_object() {
    let script = compile_unlinked_script(
        "({method(){return eval('super.value')},get read(){return eval('super.value')},set write(value){eval('super.value=value')},arrow(){return()=>eval('super.value')}})",
    )
    .expect("ObjectLiteral methods with direct eval SuperProperty compile");
    let methods = script
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 4);
    assert!(!script.metadata().super_allowed);
    assert!(!script.metadata().super_call_allowed);
    assert!(methods.iter().all(|method| method.metadata().super_allowed));
    assert!(
        methods
            .iter()
            .all(|method| !method.metadata().super_call_allowed)
    );
    assert!(
        methods
            .iter()
            .all(|method| method.metadata().needs_home_object)
    );
    for method in &methods {
        let home_entries = method
            .code()
            .iter()
            .enumerate()
            .filter_map(|(index, instruction)| {
                matches!(instruction, Instruction::PushHomeObject).then_some(index)
            })
            .collect::<Vec<_>>();
        let this_entries = method
            .code()
            .iter()
            .enumerate()
            .filter_map(|(index, instruction)| {
                matches!(instruction, Instruction::PushThis).then_some(index)
            })
            .collect::<Vec<_>>();
        let ([home_entry], [this_entry]) = (home_entries.as_slice(), this_entries.as_slice())
        else {
            panic!("method did not have unique HomeObject/this entry operations");
        };
        assert!(home_entry < this_entry);
        let Instruction::PutLocal(home_local) = method.code()[home_entry + 1] else {
            panic!("HomeObject entry did not initialize its local");
        };
        let Instruction::PutLocal(this_local) = method.code()[this_entry + 1] else {
            panic!("this entry did not initialize its local");
        };
        assert_ne!(home_local, this_local);
    }

    for direct_eval_owner in &methods[..3] {
        let environment = &direct_eval_owner.eval_environments()[0];
        assert!(environment.super_allowed);
        assert!(!environment.super_call_allowed);
        let function_root = environment
            .scopes
            .iter()
            .find(|scope| scope.kind == EvalScopeKind::FunctionRoot)
            .expect("method direct eval lost its function root");
        for expected in [THIS_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME] {
            assert!(
                function_root
                    .bindings
                    .iter()
                    .any(|binding| binding.name == JsString::from_static(expected)),
                "method direct eval lost {expected}",
            );
        }
    }
    let arrow = methods[3]
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("method lost its eval-owning arrow");
    assert!(arrow.metadata().super_allowed);
    assert!(!arrow.metadata().super_call_allowed);
    let named_parent_local = |expected: &'static str| {
        arrow
            .closure_variables()
            .iter()
            .find(|descriptor| {
                let ClosureVariableName::Constant(name) = descriptor.name else {
                    return false;
                };
                matches!(descriptor.source, ClosureSource::ParentLocal(_))
                    && matches!(
                        arrow.constants()[name as usize].as_primitive(),
                        Some(crate::engine::value::PrimitiveValue::String(name)) if name == &JsString::from_static(expected)
                    )
            })
            .expect("eval-visible arrow lost its named pseudo-binding relay")
    };
    let this_relay = named_parent_local(THIS_LOCAL_NAME);
    let home_relay = named_parent_local(HOME_OBJECT_LOCAL_NAME);
    assert_ne!(this_relay.source, home_relay.source);
    let arrow_environment = &arrow.eval_environments()[0];
    assert!(arrow_environment.super_allowed);
    assert!(!arrow_environment.super_call_allowed);
    let arrow_owner = arrow_environment
        .scopes
        .iter()
        .find(|scope| {
            scope.kind == EvalScopeKind::FunctionRoot
                && scope
                    .bindings
                    .iter()
                    .any(|binding| binding.name == JsString::from_static(HOME_OBJECT_LOCAL_NAME))
        })
        .expect("arrow direct eval lost its method owner");
    for expected in [THIS_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME] {
        assert!(
            arrow_owner
                .bindings
                .iter()
                .any(|binding| binding.name == JsString::from_static(expected)),
            "arrow direct eval lost {expected}",
        );
    }

    let cutoff_script = compile_unlinked_script(
        "({method(){super.value;return function ordinary(){return eval('super.value')}}})",
    )
    .expect("ordinary direct-eval cutoff compiles before runtime String parsing");
    let cutoff_method = cutoff_script
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("cutoff script lost its method");
    let cutoff_ordinary = cutoff_method
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("cutoff method lost its ordinary function");
    assert!(cutoff_method.metadata().super_allowed);
    assert!(!cutoff_ordinary.metadata().super_allowed);
    assert!(!cutoff_ordinary.metadata().super_call_allowed);
    assert!(!cutoff_ordinary.eval_environments()[0].super_allowed);
    assert!(!cutoff_ordinary.eval_environments()[0].super_call_allowed);

    assert_eq!(
        evaluate_in_context(
            r#"
                (function () {
                    var proto = {
                        get value() { return this.base + 2; },
                        set value(input) { this.base = input - 2; },
                        call() { return this.base + 2; }
                    };
                    var home = {
                        __proto__: proto,
                        base: 40,
                        method() { return eval("super.value"); },
                        get read() { return eval("super.value"); },
                        set write(value) { eval("super.value = value"); },
                        arrow() { return (() => eval("super.call()"))(); },
                        escaped() { return eval("() => super.value"); },
                        nested() { return eval("eval('super.value')"); },
                        cutoff() {
                            super.value;
                            return function ordinary() { return eval("super.value"); };
                        },
                        indirect() { return (0, eval)("super.value"); },
                        superCall() { return eval("super(sideEffect = true)"); }
                    };
                    var initial = home.method();
                    var getter = home.read;
                    home.write = 44;
                    var setter = home.base;
                    var arrow = home.arrow();
                    var receiver = { base: 40 };
                    var escaped = home.escaped.call(receiver)();
                    var nested = home.nested.call(receiver);
                    var cutoff;
                    try { home.cutoff()(); } catch (error) { cutoff = error.name; }
                    var globalEval;
                    try { eval("super.value"); } catch (error) { globalEval = error.name; }
                    var indirect;
                    try { home.indirect(); } catch (error) { indirect = error.name; }
                    var sideEffect = false;
                    var superCall;
                    try { home.superCall(); } catch (error) { superCall = error.name; }
                    return [
                        initial, getter, setter, arrow, escaped, nested,
                        cutoff, globalEval, indirect, superCall, sideEffect
                    ].join("|");
                })()
            "#,
        ),
        Value::String(JsString::from_static(
            "42|42|42|44|42|42|SyntaxError|SyntaxError|SyntaxError|SyntaxError|false"
        ))
    );
}

#[test]
fn object_literal_grammar_is_fail_closed_at_remaining_method_frontiers() {
    for source in [
        "({})",
        "({a:1,})",
        "({a})",
        "({let})",
        "({['x']:2})",
        "({...null})",
        "({__proto__:null,a:1})",
        "({a(){}})",
        "({a(...rest){return rest}})",
        "({a(value=1){}})",
        "({a({value}){}})",
        "({get(){}})",
        "({set(value){}})",
        "({[1](){}})",
        "({get a(){}})",
        "({set a(value){}})",
        "({set a(value=1){}})",
        "({set a({value}){}})",
        "({get ['a'](){}})",
        "({set [1](value,){}})",
        "({get get(){}})",
        "({set set(value){}})",
        "({*a(){}})",
        "({async a(){}})",
        "({async *a(){yield 1}})",
        "({get\nlineBreak(){},set\nlineBreak(value){}})",
        "({g\\u0065t(){},s\\u0065t(value){}})",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid Object literal {source:?}: {error}"));
    }

    for source in [
        "({get a(value){}})",
        "({get a(value=1){}})",
        "({get a({value}){}})",
        "({get a(...rest){}})",
        "({set a(){}})",
        "({set a(...rest){}})",
        "({set a(left,right){}})",
        "({set a(left=1,right){}})",
        "({set a({left},right){}})",
        "({set a([left],right){}})",
        "({set a(value,value){}})",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "invalid number of arguments for getter or setter",
            "accessor arity did not match QuickJS for {source}"
        );
    }
    for source in ["({get a(1){}})", "({set a(1,2){}})"] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "missing formal parameter",
            "malformed parameters must fail before the accessor arity check for {source}"
        );
    }
    for source in [
        "({get a(){'use strict';public=42}})",
        "'use strict';({set a(value){public=42}})",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "unexpected token in expression: 'public'",
            "strict future-reserved references must remain SyntaxError for {source}"
        );
    }
    assert_eq!(
        compile_unlinked_script("({a(value,value){}})")
            .unwrap_err()
            .message(),
        "duplicate argument names not allowed in this context"
    );
    assert_eq!(
        compile_unlinked_script("({__proto__:null,__proto__:{}})")
            .unwrap_err()
            .message(),
        "duplicate __proto__ property name"
    );
    assert_eq!(
        compile_unlinked_script("({get=1})").unwrap_err().message(),
        "expecting '}'"
    );
    assert_eq!(
        compile_unlinked_script("({#private:1})")
            .unwrap_err()
            .message(),
        "invalid property name"
    );

    for (source, column) in [
        ("({#private: 1})", 3),
        ("({#private() {}})", 3),
        ("({*#private() {}})", 4),
        ("({get #private() {}})", 7),
        ("({set #private(value) {}})", 7),
        ("({async #private() {}})", 9),
        ("({async *#private() {}})", 10),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), "invalid property name", "{source:?}");
        let start = error
            .span()
            .expect("object-literal error lost its span")
            .start;
        assert_eq!((start.line, start.column), (1, column), "{source:?}");
    }
}

#[test]
fn object_literal_runtime_preserves_descriptors_proto_names_and_pinned_spread() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var x=3;var o={2:'two',a:1,x,a:4};var d=Object.getOwnPropertyDescriptor(o,'a');return o[2]+'|'+o.x+'|'+o.a+'|'+d.writable+'|'+d.enumerable+'|'+d.configurable})()"
        ),
        Value::String(JsString::from_static("two|3|4|true|true|true"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var p={marker:7};var a={__proto__:p};var b={__proto__:1};var c={['__proto__']:9};return a.marker+'|'+Object.hasOwn(a,'__proto__')+'|'+(Object.getPrototypeOf(b)===Object.prototype)+'|'+c.__proto__})()"
        ),
        Value::String(JsString::from_static("7|false|true|9"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var s=Symbol('key');var a={[s]:function(){},plain:function(){}};return a[s].name+'|'+a.plain.name+'|'+Object.hasOwn({...\"ab\"},'0')})()"
        ),
        Value::String(JsString::from_static("[key]|plain|false"))
    );
}
