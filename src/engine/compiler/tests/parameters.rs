use super::*;

#[test]
fn class_constructor_guard_precedes_observable_parameter_initialization() {
    for source in [
        "class C { constructor(a, b = 1, c) {} }",
        "class C { constructor(a, b, c, d = 1, e) {} }",
        "class C { constructor(a, b, ...rest) {} }",
        "class C { constructor([a]) {} }",
    ] {
        let script = compile_unlinked_script(source).unwrap();
        let constructor = script
            .constants()
            .iter()
            .filter_map(|constant| constant.as_child())
            .find(|function| {
                function.metadata().constructor_kind == ConstructorKind::Base
                    && !function.metadata().has_prototype
            })
            .expect("class constructor child");
        validate_parameter_bytecode_layout(
            constructor.metadata(),
            constructor.code(),
            &vec![false; usize::from(constructor.metadata().local_count)],
            constructor.parameter_environment(),
        )
        .unwrap_or_else(|error| panic!("invalid constructor ABI for {source}: {error}"));
        let guard = constructor
            .code()
            .iter()
            .position(|instruction| matches!(instruction, Instruction::CheckCtor))
            .expect("class constructor guard");
        let first_parameter_operation = constructor
            .code()
            .iter()
            .position(|instruction| {
                matches!(instruction, Instruction::GetArg(_) | Instruction::Rest(_))
            })
            .expect("class constructor parameter operation");
        assert!(
            guard < first_parameter_operation,
            "class guard followed parameter initialization for {source}"
        );
    }

    let rest = compile_unlinked_script("class C { constructor(a, b, ...rest) {} }").unwrap();
    let rest = rest
        .constants()
        .iter()
        .filter_map(|constant| constant.as_child())
        .find(|function| !function.metadata().has_prototype)
        .expect("rest class constructor child");
    assert!(rest.code().windows(7).any(|window| matches!(
        window,
        [
            Instruction::CheckCtor,
            Instruction::PushThis,
            Instruction::PushActiveFunction,
            Instruction::CallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Rest(2),
            Instruction::PutArg(2),
        ]
    )));

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var effects=0;
                class Defaulted { constructor(a,b,c=(effects+=1),d){} }
                class Pattern { constructor([a=(effects+=10)]=[]){} }
                class Rest {
                    constructor(a,b,...rest){this.value=a+b+rest[0]+rest[1]}
                }
                var errors=[];
                try { Defaulted(1,2); } catch (error) { errors.push(error.name); }
                try { Pattern(); } catch (error) { errors.push(error.name); }
                try { Rest(1,2,3,4); } catch (error) { errors.push(error.name); }
                return errors.join(',')+'|'+effects+'|'+new Rest(10,10,20,2).value;
            })()"#,
        ),
        Value::String(JsString::from_static("TypeError,TypeError,TypeError|0|42"))
    );
}

#[test]
fn class_name_scopes_in_parameter_initializers_are_not_body_lexicals() {
    let original = "(function({ k = class i { [_ => i]() {} } } = {}) { var j=0; })()";
    assert_eq!(evaluate_in_context(original), Value::Undefined);
    assert_eq!(
        evaluate_in_context(&format!("'use strict';{original}")),
        Value::Undefined
    );

    assert_eq!(
        evaluate_in_context(
            "(function({k=class Inner { method(){return Inner} }}){return new k().method()===k})({})",
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(k=class Inner { method(){return Inner} }){return new k().method()===k})()",
        ),
        Value::Bool(true)
    );

    let script = compile_unlinked_script(
        "(function({k=class Inner { method(){return Inner} }}){return k})({})",
    )
    .unwrap();
    let function = script.constants()[0]
        .as_child()
        .expect("parameter function child");
    assert!(function.local_definitions().iter().any(|definition| {
        definition.is_lexical && definition.is_const && definition.is_parameter_initializer
    }));
}

#[test]
fn body_eval_after_default_parameters_keeps_parameter_bindings_visible() {
    assert_eq!(
        evaluate_in_context("(function(first, second = 1) { return eval('first + second'); })(41)",),
        Value::Int(42)
    );
}

#[test]
fn implicit_arguments_binding_is_lazy_and_precedes_body_hoists() {
    for source in [
        "(function() { return 1; })",
        "(function() { return delete arguments; })",
        "(function(arguments) { return arguments; })",
    ] {
        let script = compile_unlinked_script(source).unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert!(
            !function
                .code()
                .iter()
                .any(|op| matches!(op, Instruction::Arguments(_))),
            "unexpected arguments object for {source}"
        );
    }

    for (source, kind) in [
        ("(function() { return arguments; })", ArgumentsKind::Mapped),
        (
            "(function() { 'use strict'; return arguments; })",
            ArgumentsKind::Unmapped,
        ),
        (
            "(function() { var arguments; return typeof arguments; })",
            ArgumentsKind::Mapped,
        ),
    ] {
        let script = compile_unlinked_script(source).unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert!(matches!(
            function.code(),
            [Instruction::Arguments(actual), Instruction::PutLocal(0), ..]
                if *actual == kind
        ));
        assert_eq!(function.local_definitions().len(), 1, "{source}");
    }

    let script = compile_unlinked_script(
        "(function arguments() { function arguments() {} return arguments; })",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().function_name_local, None);
    assert!(matches!(
        function.code(),
        [
            Instruction::Arguments(ArgumentsKind::Mapped),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(0),
            ..
        ]
    ));

    let script = compile_unlinked_script(
        "(function outer() { return function inner() { return arguments; }; })",
    )
    .unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let inner = outer.constants()[0].as_child().unwrap();
    assert!(
        !outer
            .code()
            .iter()
            .any(|op| matches!(op, Instruction::Arguments(_)))
    );
    assert!(matches!(
        inner.code(),
        [
            Instruction::Arguments(ArgumentsKind::Mapped),
            Instruction::PutLocal(0),
            ..
        ]
    ));

    assert_eq!(
        evaluate_in_context("(function(arguments) { var arguments; return arguments; })(7)"),
        Value::Int(7)
    );
}

#[test]
fn parameter_binding_patterns_publish_quickjs_anonymous_argument_abi() {
    let script =
        compile_unlinked_script("(function(left,[a,b],{c},...[rest]){return left+a+b+c+rest})")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 3);
    assert_eq!(function.metadata().defined_argument_count, 4);
    assert_eq!(function.metadata().rest_parameter, None);
    assert_eq!(function.metadata().rest_pattern_start, Some(3));
    assert_eq!(function.metadata().parameter_environment_local_count, 0);
    assert_eq!(function.metadata().pattern_argument_count, 2);
    let marker = usize::try_from(
        function
            .metadata()
            .parameter_pattern_end
            .expect("pattern initialization marker"),
    )
    .unwrap();
    assert!(matches!(
        function.code().get(marker),
        Some(Instruction::Nop)
    ));
    assert_eq!(
        function
            .argument_definitions()
            .iter()
            .map(|definition| definition.name.is_some())
            .collect::<Vec<_>>(),
        vec![true, false, false]
    );
    assert_eq!(
        function
            .code()
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| match instruction {
                Instruction::GetArg(argument) if pc < marker => Some(*argument),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(
        function.code()[..marker]
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Rest(3)))
    );
    assert!(!function.code()[marker + 1..].iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::GetArg(1)
                | Instruction::PutArg(1)
                | Instruction::SetArg(1)
                | Instruction::GetArg(2)
                | Instruction::PutArg(2)
                | Instruction::SetArg(2)
        )
    }));

    let rest_only = compile_unlinked_script("(function(...[a,b]){})").unwrap();
    let rest_only = rest_only.constants()[0].as_child().unwrap();
    assert_eq!(rest_only.metadata().argument_count, 0);
    assert_eq!(rest_only.metadata().defined_argument_count, 1);
    assert_eq!(rest_only.metadata().rest_pattern_start, Some(0));
    assert_eq!(rest_only.metadata().pattern_argument_count, 0);
    assert_eq!(rest_only.argument_definitions(), []);
}

#[test]
fn parameter_binding_patterns_execute_across_sync_function_forms() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var out=[];
                function declaration([a],{b}){return a+b}
                out.push(declaration([40],{b:2}));
                out.push((function([a]){return a+2})([40]));
                out.push((([a])=>a+2)([40]));
                out.push(({base:40,method([a]){return this.base+a}}).method([2]));
                out.push(Function('[a]','return a+2')([40]));
                var assigned;
                var setter={set value([a]){assigned=a}};
                setter.value=[42];
                out.push(assigned);
                return out.join('|');
            })()"#,
        ),
        Value::String(JsString::from_static("42|42|42|42|42|42"))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function([a,,[b,...tail]],{x:y,[String('z')]:z,...rest}){
                return a+'|'+b+'|'+tail.join(',')+'|'+y+'|'+z+'|'+rest.extra;
            })([1,0,[2,3,4]],{x:5,z:6,extra:7})"#,
        ),
        Value::String(JsString::from_static("1|2|3,4|5|6|7"))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(...[a,b]){
                return a+b+'|'+arguments.length+'|'+(function(...[]){}).length+'|'+
                    (function(...{}){}).length+'|'+(function(...[[]]){}).length+'|'+
                    (function(...[,]){}).length+'|'+(function(...[x]){}).length+'|'+
                    (function(...{x}){}).length+'|'+(function(...[]){var x}).length+'|'+
                    (function(...[]){return arguments}).length+'|'+
                    (function(...[]){return this}).length+'|'+
                    (function(...[]){return new.target}).length+'|'+
                    (function(...[]){return function(){return this}}).length+'|'+
                    (function(...[]){return ()=>this}).length+'|'+
                    (function named(...[]){return named}).length+'|'+
                    (function(x,...[]){}).length+'|'+(function(x,...[y]){}).length+'|'+
                    (function mixed([x],...rest){return x+rest[0]+'|'+mixed.length})([40],2);
            })(40,2)"#,
        ),
        Value::String(JsString::from_static(
            "42|2|0|0|0|0|1|1|1|1|1|1|0|1|1|2|2|42|1"
        ))
    );
}

#[test]
fn parameter_binding_patterns_follow_quickjs_scope_arguments_and_hoist_order() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var results=[];
                results.push((function([a]){
                    arguments[0]=[9];
                    a=7;
                    return a+'|'+arguments[0][0];
                })([1]));
                results.push((function([a]){var a;return a})([42]));
                results.push((function([a]){
                    function a(){return 42}
                    return typeof a+'|'+a();
                })([1]));
                var key='outer';
                results.push((function({[String(key)]:value}){
                    var key='body';
                    return value;
                })({undefined:42,outer:1}));
                var lexical='outer';
                results.push((function({[lexical]:value}){
                    let lexical='body';
                    return value;
                })({outer:42}));
                results.push((function([a]){return eval('a')})([42]));
                results.push((function({[eval('"key"')]:value}){
                    return value;
                })({key:42}));
                results.push((function({[arguments]:arguments}){
                    return arguments;
                })({undefined:1,"[object Arguments]":42}));
                results.push((function({[(()=>eval('typeof key'))()]:value}){
                    var key='body';
                    return value;
                })({undefined:42}));
                results.push((function(){
                    return (([a])=>a+arguments[1])([40]);
                })(0,2));
                return results.join(';');
            })()"#,
        ),
        Value::String(JsString::from_static(
            "7|9;42;function|42;42;42;42;42;42;42;42"
        ))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var saved;
                function capture(fn){saved=fn;return 'undefined'}
                return (function({[capture(()=>key)]:value}){
                    var key='body';
                    return value+'|'+saved();
                })({undefined:42});
            })()"#,
        ),
        Value::String(JsString::from_static("42|body"))
    );
}

#[test]
fn parameter_binding_pattern_assignment_prescan_matches_quickjs_token_rule() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var key=0;return (function({[(key+=1)]:value}){return value})({1:42})})()",
        ),
        Value::Int(42)
    );

    for source in [
        "(function([a=1]){})",
        "(function([a],b=1){})",
        "(function(a=1,[b]){})",
        "(function({[(key=1)]:value}){})",
        "(function(...[a=1]){})",
        "(([a=1])=>a)",
        "({method({value=1}){}})",
    ] {
        let script = compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("parameter expression {source:?}: {error}"));
        let function = script.constants()[0].as_child().unwrap();
        assert!(
            function.parameter_environment().is_some(),
            "standalone '=' did not publish a Parameter Environment for {source}"
        );
    }
}

#[test]
fn parameter_assignment_prescan_reserves_every_bound_name_before_initializers() {
    let script = compile_unlinked_script(
        "(function(left,[a,{b:[c=class Nested{},...d]},...e],{[(()=>`key,${1}`)()]:f=/,/.test(',')?class Named{}:1,g},right=class Right{}){})",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().parameter_environment_local_count, 8);

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary([x=class Ordinary{},y=40],z=2){return y+z}
                function* generator([x=class Generator{},y=40],z=2){return y+z}
                var arrow=([x=class Arrow{},y=40],z=2)=>y+z;
                var object={
                    method([x=class ObjectMethod{},y=40],z=2){return y+z},
                    *generator([x=class ObjectGenerator{},y=40],z=2){return y+z}
                };
                class Holder {
                    method([x=class ClassMethod{},y=40],z=2){return y+z}
                    *generator([x=class ClassGenerator{},y=40],z=2){return y+z}
                }
                var holder=new Holder;
                return [
                    ordinary([undefined]),
                    generator([undefined]).next().value,
                    arrow([undefined]),
                    object.method([undefined]),
                    object.generator([undefined]).next().value,
                    holder.method([undefined]),
                    holder.generator([undefined]).next().value
                ].join('|');
            })()"#,
        ),
        Value::String(JsString::from_static("42|42|42|42|42|42|42"))
    );
}

#[test]
fn parameter_assignment_prescan_retains_quickjs_bits_at_the_depth_bound() {
    let source = format!(
        "(a={}class Nested{{}}{},b=1)",
        "(".repeat(260),
        ")".repeat(260)
    );
    let mut lexer = Lexer::new(&source);
    let first = lexer.next_token().unwrap();
    let source_span = first.span;
    let parser = Parser {
        lexer,
        tokens: vec![first],
        cursor: 0,
        current_function: 0,
        in_mode: InMode::Allow,
        functions: vec![
            FunctionIr::new(
                None,
                FunctionKind::Script,
                FunctionSourceInfo {
                    span: source_span,
                    definition: SourceOffset::try_from_usize(0).unwrap(),
                    range: None,
                },
                FunctionIrOptions {
                    function_name: Some("<scan-test>".to_owned()),
                    private_name_binding: false,
                    class_constructor: false,
                    derived_class_constructor: false,
                    parameters: Vec::new(),
                    defined_argument_count: 0,
                    has_simple_parameter_list: true,
                    rest_parameter: None,
                    strict: false,
                    super_capabilities: SuperCapabilities::NONE,
                },
            )
            .unwrap(),
        ],
        module: None,
        module_declaration_export: ModuleDeclarationExport::None,
        module_declaration_export_target: None,
        anonymous_function_definition: None,
        pending_unsupported: None,
    };

    assert_eq!(parser.parenthesized_parameter_has_assignment(), Some(true));
    assert_eq!(
        parser.parenthesized_parameter_scan(),
        Some(ParenthesizedParameterScan {
            has_assignment: true,
            bound_name_count: Some(2),
        })
    );
}

#[test]
fn parameter_expression_binding_patterns_publish_the_quickjs_argument_scope_abi() {
    let script = compile_unlinked_script(
        "(function(left,[a,b=left],{c},right=a+b+c){return left+a+b+c+right})",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    let layout = function
        .parameter_environment()
        .expect("standalone '=' creates the parentless argument scope");

    assert_eq!(function.metadata().argument_count, 4);
    assert_eq!(function.metadata().defined_argument_count, 3);
    assert_eq!(function.metadata().parameter_environment_local_count, 5);
    assert_eq!(function.metadata().pattern_argument_count, 2);
    assert_eq!(
        function
            .argument_definitions()
            .iter()
            .map(|definition| definition.name.is_some())
            .collect::<Vec<_>>(),
        vec![true, false, false, true]
    );
    assert_eq!(
        layout
            .argument_cells
            .iter()
            .map(|cell| (cell.argument, cell.parameter_local))
            .collect::<Vec<_>>(),
        vec![(0, 0), (3, 4)]
    );
    assert_eq!(
        layout
            .pattern_copies
            .iter()
            .map(|copy| (copy.parameter_local, copy.body_local))
            .collect::<Vec<_>>(),
        vec![(1, 5), (2, 6), (3, 7)]
    );
    assert_eq!(
        layout.default_sources.as_ref(),
        [ParameterDefaultSource::Argument(3)]
    );
    assert_eq!(
        function.metadata().parameter_pattern_end,
        Some(layout.initialization_end)
    );
}

#[test]
fn parameter_expression_binding_patterns_match_quickjs_scope_copy_and_length_quirks() {
    assert_eq!(
        evaluate_in_context(
            "(function(){var key='outer';return (function({[String(key)]:value},[x=1]){var key='body';return value})({outer:42,undefined:1},[])})()",
        ),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("(function([a],b=(a=5)){return a})([1])"),
        Value::Int(5)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var saved;var body=(function([a],f=(saved=()=>a)){var read=()=>a;a=2;return read})([1]);return body()+'|'+saved()})()",
        ),
        Value::String(JsString::from_static("2|1"))
    );
    assert_eq!(
        evaluate_in_context(
            "[(function([a=1],b){}).length,(function([a]=[1],b){}).length,(function(a,[b=1],c){}).length,(function(a,[b]=[1],c){}).length,(function(a,...[b=1]){}).length,(function(a=1,...[b]){}).length].join('|')",
        ),
        Value::String(JsString::from_static("2|0|3|1|2|0"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var source={};return (function({}=source){var source=null;return 42})()})()",
        ),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){return (function(...[a]=[99]){return String(a)} )()+'|'+(function(...[a]=[99]){return a})(42)+'|'+(function(...[a]=[99]){}).length})()",
        ),
        Value::String(JsString::from_static("undefined|42|0"))
    );
    assert_eq!(
        evaluate_in_context(
            "(function(){var out;({set value([a]=[42]){out=a}}).value=undefined;return out})()",
        ),
        Value::Int(42)
    );
}

#[test]
fn parameter_expression_binding_patterns_compose_identifier_rest_across_surfaces() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary([a=1],...rest){return a+'|'+rest.join(',')}
                var arrow=([a=1],...rest)=>a+'|'+rest.join(',');
                var object={method([a=1],...rest){return a+'|'+rest.join(',')}};
                return ordinary([],2,3)+';'+arrow([],2,3)+';'+object.method([],2,3);
            })()"#,
        ),
        Value::String(JsString::from_static("1|2,3;1|2,3;1|2,3"))
    );
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary([a]=[1],...rest){return a+'|'+rest.join(',')}
                var arrow=([a]=[1],...rest)=>a+'|'+rest.join(',');
                var object={method([a]=[1],...rest){return a+'|'+rest.join(',')}};
                return ordinary(undefined,2,3)+';'+arrow(undefined,2,3)+';'+
                    object.method(undefined,2,3);
            })()"#,
        ),
        Value::String(JsString::from_static("1|2,3;1|2,3;1|2,3"))
    );
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function later([a],b=1,...rest){return a+'|'+b+'|'+rest.join(',')}
                function earlier(a=0,[b],...rest){return a+'|'+b+'|'+rest.join(',')}
                function empty({},b=1,...rest){return b+'|'+rest.join(',')}
                return later([40],undefined,2,3)+';'+earlier(undefined,[40],2,3)+';'+
                    empty({},undefined,2,3);
            })()"#,
        ),
        Value::String(JsString::from_static("40|1|2,3;0|40|2,3;1|2,3"))
    );
}

#[test]
fn parameter_expression_binding_arguments_and_duplicate_order_match_quickjs() {
    let script =
        compile_unlinked_script("(function({arguments=1}){var arguments;return arguments})")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert!(function.parameter_environment().is_some());
    assert!(
        function
            .code()
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::Arguments(_)))
    );
    assert_eq!(
        evaluate_in_context("(function({arguments=1}){var arguments;return arguments})({})"),
        Value::Int(1)
    );

    for source in [
        "function f(a,a=1){'use strict'}",
        "({m(a,a=1){'use strict'}})",
        "(a,a=1)=>{'use strict'}",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(
            error.message(),
            "duplicate parameter names not allowed in this context",
            "{source}"
        );
    }
}

#[test]
fn parameter_expression_binding_cells_and_body_copies_survive_gc_independently() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            "var initializerRead,bodyRead;bodyRead=(function([a],f=(initializerRead=()=>a)){var read=()=>a;a=2;return read})([1])",
        )
        .unwrap();
    runtime.run_gc().unwrap();
    assert_eq!(
        context.eval("bodyRead()+'|'+initializerRead()").unwrap(),
        Value::String(JsString::from_static("2|1"))
    );
}

#[test]
fn identifier_rest_parameters_publish_quickjs_length_and_entry_order() {
    let script = compile_unlinked_script(
        "(function(left,...rest){ function rest(){ return 42; } return arguments; })",
    )
    .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 2);
    assert_eq!(function.metadata().defined_argument_count, 1);
    assert!(matches!(
        function.code(),
        [
            Instruction::Arguments(ArgumentsKind::Unmapped),
            Instruction::PutLocal(0),
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::FClosure(0),
            Instruction::PutArg(1),
            ..
        ]
    ));

    let script = compile_unlinked_script("({method(left,...rest){return rest}})").unwrap();
    let method = script.constants()[0].as_child().unwrap();
    assert_eq!(method.metadata().argument_count, 2);
    assert_eq!(method.metadata().defined_argument_count, 1);
    assert!(matches!(
        method.code(),
        [Instruction::Rest(1), Instruction::PutArg(1), ..]
    ));

    let script = compile_unlinked_script("(left,...rest)=>rest").unwrap();
    let arrow = script.constants()[0].as_child().unwrap();
    assert_eq!(arrow.metadata().argument_count, 2);
    assert_eq!(arrow.metadata().defined_argument_count, 1);
    assert!(matches!(
        arrow.code(),
        [Instruction::Rest(1), Instruction::PutArg(1), ..]
    ));
}

#[test]
fn identifier_rest_with_direct_eval_uses_the_extended_entry_order() {
    let script =
        compile_unlinked_script("function f(...rest){ return eval('this.marker') + rest[0] }")
            .unwrap();
    let function = script.constants()[0].as_child().unwrap();
    let eval_variable_object = function.metadata().eval_variable_object_local.unwrap();
    let variable_environment = function
        .code()
        .windows(2)
        .position(|window| {
            matches!(
                window,
                [
                    Instruction::VariableEnvironment,
                    Instruction::PutLocal(target),
                ] if *target == eval_variable_object
            )
        })
        .unwrap();
    let rest = function
        .code()
        .windows(2)
        .position(|window| matches!(window, [Instruction::Rest(0), Instruction::PutArg(0)]))
        .unwrap();
    assert!(variable_environment < rest);

    assert_eq!(
        evaluate_in_context(
            "(function(...rest){ return eval('this.marker') + rest[0] }).call({marker:40},2)",
        ),
        Value::Int(42)
    );
}

#[test]
fn identifier_rest_parameters_execute_across_sync_function_forms() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary(left,...rest){
                    return left+'|'+rest.length+'|'+rest[0]+'|'+rest[1]+'|'+Array.isArray(rest);
                }
                var arrow=(left,...rest)=>left+'|'+rest.length+'|'+rest[0]+'|'+rest[1];
                var object={base:40,method(left,...rest){
                    return this.base+'|'+left+'|'+rest.length+'|'+rest[0]+'|'+rest[1];
                }};
                var dynamic=Function('left','...rest','return left+rest[0]+rest[1]');
                return ordinary(40,1,2)+';'+arrow(40,1,2)+';'+object.method(1,1,2)+';'+
                    dynamic(40,1,1)+';'+ordinary.length+'|'+arrow.length+'|'+
                    object.method.length+'|'+dynamic.length;
            })()"#,
        ),
        Value::String(JsString::from_static(
            "40|2|1|2|true;40|2|1|2;40|1|2|1|2;42;1|1|1|1"
        ))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(left,...rest){
                arguments[0]=7;
                arguments[1]=8;
                var first=left+'|'+rest[0];
                left=9;
                rest[0]=10;
                return first+'|'+left+'|'+rest[0]+'|'+arguments[0]+'|'+arguments[1];
            })(1,2)"#,
        ),
        Value::String(JsString::from_static("1|2|9|10|7|8"))
    );

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var retained=(function(...rest){var rest;return Array.isArray(rest)+'|'+rest.length})();
                var hoisted=(function(...rest){function rest(){return 42}return typeof rest+'|'+rest()})();
                var capture=(function(...rest){return function(){return rest[0]+rest[1]}})(40,2);
                return retained+';'+hoisted+';'+capture();
            })()"#,
        ),
        Value::String(JsString::from_static("true|0;function|42;42"))
    );
}

#[test]
fn identifier_default_parameters_execute_across_sync_function_forms() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                function ordinary(a=40,b=a+2){return b}
                var arrow=(a=40,b=a+2)=>b;
                var object={base:40,method(a=this.base,b=a+2){return b}};
                var dynamic=Function('a=40','b=a+2','return b');
                return ordinary()+'|'+arrow()+'|'+object.method()+'|'+dynamic()+'|'+
                    ordinary.length+'|'+arrow.length+'|'+object.method.length+'|'+dynamic.length;
            })()"#,
        ),
        Value::String(JsString::from_static("42|42|42|42|0|0|0|0"))
    );
}

#[test]
fn identifier_default_before_rest_uses_the_parameter_environment_abi() {
    let script = compile_unlinked_script("function f(a=40,...rest){return a+rest.length}").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().argument_count, 2);
    assert_eq!(function.metadata().defined_argument_count, 0);
    assert_eq!(function.metadata().rest_parameter, Some(1));
    assert_eq!(function.metadata().parameter_environment_local_count, 2);
    assert!(matches!(
        function.code(),
        [
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            ..
        ]
    ));
    assert!(function.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::Rest(1),
            Instruction::Dup,
            Instruction::PutArg(1),
            Instruction::InitializeLocal(1),
        ]
    )));

    assert_eq!(
        evaluate_in_context("(function(a=40,...rest){return a+rest.length})(undefined,1,1)"),
        Value::Int(42)
    );
}

#[test]
fn identifier_default_parameter_environment_matches_quickjs_tdz_and_body_split() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(function(a=b,b=2){return a})()",
        "(function(a=a){return a})()",
        "((a=b,b=2)=>a)()",
        "({method(a=a){return a}}).method()",
    ] {
        let (name, _) = evaluate_error(&runtime, &mut context, source);
        assert_eq!(name, JsString::from_static("ReferenceError"), "{source}");
    }

    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var read;
                function f(a=1,b=(read=function(){return a},0)){
                    a=2;
                    return a+'|'+read();
                }
                return f();
            })()"#,
        ),
        // Pinned QuickJS keeps body reads on the raw argument slot while the
        // initializer closure retains the lexical parameter cell.
        Value::String(JsString::from_static("2|1"))
    );
}

#[test]
fn identifier_default_parameter_uses_private_function_name_before_body_shadowing() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var read=(function f(a=f){var f;return typeof a+'|'+(a===undefined)+'|'+typeof f})();
                var closure=(function f(a=()=>f){var f=1;return typeof a()+'|'+(a()===f)})();
                var write=(function f(a=(f=1)){var f;return typeof f+'|'+(f===1)})();
                return read+';'+closure+';'+write;
            })()"#,
        ),
        Value::String(JsString::from_static(
            "function|false|undefined;function|false;undefined|false"
        ))
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (name, message) = evaluate_error(
        &runtime,
        &mut context,
        "(function(){'use strict';return (function f(a=(f=1)){var f;return typeof f})()})()",
    );
    assert_eq!(name, JsString::from_static("TypeError"));
    assert_eq!(message, JsString::from_static("'f' is read-only"));
}

#[test]
fn identifier_default_parameters_run_before_body_hoists_and_use_unmapped_arguments() {
    assert_eq!(
        evaluate_in_context(
            r#"(function(){
                var outer=40;
                function f(a=outer,b=arguments[0]){
                    var outer=1;
                    function a(){return 42}
                    arguments[0]=7;
                    return a()+'|'+b+'|'+arguments.length;
                }
                return f(undefined);
            })()"#,
        ),
        Value::String(JsString::from_static("42|undefined|1"))
    );
}

#[test]
fn identifier_rest_array_is_allocated_in_the_callee_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let Value::Object(function) = defining
        .eval("(function(...rest){return Object.getPrototypeOf(rest)===Array.prototype})")
        .unwrap()
    else {
        panic!("rest source did not produce a function");
    };
    let function = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller
            .call(
                &function,
                Value::Undefined,
                &[Value::Int(40), Value::Int(2)]
            )
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn identifier_rest_parameter_early_errors_match_quickjs_policy() {
    for source in [
        "function f(...rest,next){}",
        "function f(...rest,){}",
        "function f(...rest=[]){}",
        "function f(value,...value,){}",
        "(...rest,next)=>0",
        "(...rest,)=>0",
        "(...rest=[])=>0",
        "(value,...value,)=>0",
        "({method(value,...value,){}})",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "expecting ')'", "{source}");
    }

    for source in [
        "function f(value,value,...rest){}",
        "(value,value,...rest)=>0",
        "({method(value,value,...rest){}})",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "duplicate argument names not allowed in this context",
            "{source}"
        );
    }

    for source in [
        "function f(...rest){'use strict';}",
        "function f(value,...value){'use strict';}",
        "(...rest)=>{'use strict';}",
        "(value,...value)=>{'use strict';}",
        "({method(value,...value){'use strict';}})",
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "\"use strict\" not allowed in function with default or destructuring parameter",
            "{source}"
        );
    }

    for source in ["({get value(...rest){}})", "({set value(...rest){}})"] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid number of arguments for getter or setter",
            "{source}"
        );
    }

    compile_unlinked_script("function f(value,value){}")
        .expect("a sloppy ordinary simple parameter list may contain duplicates");
    compile_unlinked_script("'use strict';function f(...rest){}")
        .expect("inherited strictness does not make a rest parameter directive invalid");
}
