use super::*;

#[test]
fn direct_function_body_declarations_hoist_into_argument_and_local_bindings() {
    for (source, expected) in [
        (
            "(function(){return before();function before(){return 1}})()",
            Value::Int(1),
        ),
        (
            "(function(parameter){function local(){return later}function parameter(){return 10}function local(){return later+1}let later=2;return parameter()+local()})(0)",
            Value::Int(13),
        ),
        (
            "(function(){function arguments(){return 4}return arguments()})()",
            Value::Int(4),
        ),
        (
            "(function(){var arguments;function arguments(){return 6}return arguments()})()",
            Value::Int(6),
        ),
        (
            "(function(){function arguments(){return 7}var arguments;return arguments()})()",
            Value::Int(7),
        ),
        (
            "(function(){var arguments=function(){return 9};function arguments(){return 8}return arguments()})()",
            Value::Int(9),
        ),
        (
            "(function named(){function named(){return 5}return named()})()",
            Value::Int(5),
        ),
        (
            "(function(){'use strict';function mutable(){mutable=6;return mutable}return mutable()})()",
            Value::Int(6),
        ),
        (
            "(function(){if(false)return 0;function branch(){return 2}var count=0;while(count<1)count++;return branch()+count})()",
            Value::Int(3),
        ),
    ] {
        assert_eq!(evaluate_in_context(source), expected, "{source}");
    }

    let script = compile_unlinked_script(
        "(function(first,second){var local;function local(){}function second(){}function first(){}function local(){return 1}})",
    )
    .unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    assert!(matches!(outer.code()[0], Instruction::FClosure(2)));
    assert!(matches!(outer.code()[1], Instruction::PutArg(0)));
    assert!(matches!(outer.code()[2], Instruction::FClosure(1)));
    assert!(matches!(outer.code()[3], Instruction::PutArg(1)));
    assert!(matches!(outer.code()[4], Instruction::FClosure(3)));
    assert!(matches!(outer.code()[5], Instruction::PutLocal(0)));
    for child in outer
        .constants()
        .iter()
        .filter_map(|value| value.as_child())
    {
        assert_eq!(child.metadata().function_name_local, None);
    }
}

#[test]
fn direct_function_body_declaration_conflicts_match_quickjs_order() {
    for (source, message) in [
        (
            "(function(){function conflict(){};let conflict})",
            "invalid redefinition of a variable",
        ),
        (
            "(function(){let conflict;function conflict(){}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(conflict){function conflict(){};let conflict})",
            "invalid redefinition of parameter name",
        ),
        (
            "(function(){'use strict';function eval(){}})",
            "invalid function name in strict code",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }
}

#[test]
fn scoped_function_declarations_keep_entry_and_annex_closures_distinct() {
    for (source, expected) in [
        (
            "(function(){var inside;{function f(){return f}inside=f}return (inside!==f)+'|'+(inside()===inside)+'|'+(f()===inside)})()",
            "true|true|true",
        ),
        (
            "(function(){var inside;{function f(){return 1}function f(){return 2}inside=f}return inside()+'|'+f()+'|'+(inside===f)})()",
            "2|1|false",
        ),
        (
            "(function(){'use strict';var inside;{function f(){return 3}inside=f}return typeof f+'|'+inside()})()",
            "undefined|3",
        ),
        (
            "(function(parameter){var inside;{function parameter(){return 4}inside=parameter}return parameter+'|'+inside()})(1)",
            "1|4",
        ),
        (
            "(function(){var g;{f=8;function f(){}g=f}return g+'|'+typeof f+'|'+f.name})()",
            "8|function|f",
        ),
        (
            "(function(){var a,b,i=0;while(i<2){let x=i;function f(){return x}if(i===0)a=f;else b=f;i++}return (a!==b)+'|'+a()+'|'+b()})()",
            "true|0|1",
        ),
        (
            "(function(){var trace=typeof f;switch(0){case (trace+='|'+typeof f,0):function f(){return 5}}return trace+'|'+f()})()",
            "undefined|function|5",
        ),
        (
            "(function(){var original=function self(){{function self(){return 6}}return self};var replacement=original();return (replacement!==original)+'|'+replacement()})()",
            "true|6",
        ),
    ] {
        assert_eq!(
            evaluate_in_context(source),
            Value::String(JsString::from_static(expected)),
            "{source}"
        );
    }

    let script = compile_unlinked_script(
        "(function(){{function duplicate(){return 1}function duplicate(){return 2}}return duplicate()})",
    )
    .unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let closures = outer
        .code()
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::FClosure(constant) => Some(*constant),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(closures, [1, 0, 0, 1]);
    assert!(outer.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(0),
            Instruction::Dup,
            Instruction::PutLocal(_),
            Instruction::Drop,
        ]
    )));
    assert!(!outer.code().windows(3).any(|window| matches!(
        window,
        [
            Instruction::FClosure(1),
            Instruction::Dup,
            Instruction::PutLocal(_),
        ]
    )));

    let arguments_script =
        compile_unlinked_script("(function(){{function arguments(){return 3}}return 1})").unwrap();
    let arguments_outer = arguments_script.constants()[0].as_child().unwrap();
    assert_eq!(arguments_outer.local_definitions().len(), 1);
    assert_eq!(
        arguments_outer.local_definitions()[0].name.as_ref(),
        Some(&JsString::from_static("arguments"))
    );
    assert!(arguments_outer.local_definitions()[0].is_lexical);
    assert!(
        !arguments_outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Dup)),
        "implicit arguments name incorrectly created an Annex root write"
    );

    let shadow_script = compile_unlinked_script(
        "(function(){let shadow=12;{function shadow(){return 13}}return shadow})",
    )
    .unwrap();
    let shadow_outer = shadow_script.constants()[0].as_child().unwrap();
    assert_eq!(shadow_outer.local_definitions().len(), 2);
    assert!(
        shadow_outer
            .local_definitions()
            .iter()
            .all(|definition| definition.is_lexical)
    );
    assert!(
        !shadow_outer
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Dup)),
        "prior enclosing lexical incorrectly allowed an Annex root write"
    );
}

#[test]
fn scoped_function_conflicts_and_global_annex_order_match_quickjs() {
    for (source, message) in [
        (
            "(function(){{let conflict;function conflict(}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){'use strict';{function duplicate(){}function duplicate(}})",
            "invalid redefinition of lexical identifier",
        ),
        (
            "(function(){{var conflict;function conflict(){}}})",
            "invalid redefinition of a variable",
        ),
        (
            "(function(){{function conflict(){}var conflict;}})",
            "invalid redefinition of lexical identifier",
        ),
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            message,
            "{source}"
        );
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("{function lateGlobalLexical(){}}let lateGlobalLexical;"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Annex B global lexical collision did not throw an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(
            "lateGlobalLexical is not initialized"
        ))
    );

    assert_eq!(
        context
            .eval(
                "let priorGlobalLexical=7;{function priorGlobalLexical(){return 8}}\
                 typeof globalThis.priorGlobalLexical+'|'+priorGlobalLexical"
            )
            .unwrap(),
        Value::String(JsString::from_static("undefined|7"))
    );
}

#[test]
fn annex_b_single_and_labelled_declarations_preserve_scope_shape() {
    let program = compile_unlinked_script(
        "programLabel:function programLabel(){return programLabel};programLabel",
    )
    .unwrap();
    assert_eq!(
        program.local_definitions().len(),
        1,
        "Program labelled functions must not allocate a lexical local"
    );
    assert!(program.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(_),
            Instruction::Dup,
            Instruction::PutVar(_),
            Instruction::PutVar(_),
        ]
    )));

    let body = compile_unlinked_script(
        "(function(){bodyLabel:function bodyLabel(){return 3};return bodyLabel})",
    )
    .unwrap();
    let body = body.constants()[0].as_child().unwrap();
    assert_eq!(body.local_definitions().len(), 2);
    assert!(body.local_definitions()[0].is_lexical);
    assert!(!body.local_definitions()[1].is_lexical);
    assert!(body.code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::FClosure(_),
            Instruction::Dup,
            Instruction::PutLocal(_),
            Instruction::Drop,
        ]
    )));

    let parameter = compile_unlinked_script(
        "(function(parameter){label:function parameter(){return 4};return parameter})",
    )
    .unwrap();
    let parameter = parameter.constants()[0].as_child().unwrap();
    assert_eq!(parameter.local_definitions().len(), 1);
    assert!(parameter.local_definitions()[0].is_lexical);
    assert!(
        !parameter
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Dup)),
        "same-name parameter incorrectly received an Annex root write"
    );

    let duplicate = compile_unlinked_script(
        "(function(){if(true)function duplicate(){return 1}else function duplicate(){return 2};return duplicate})",
    )
    .unwrap();
    let duplicate = duplicate.constants()[0].as_child().unwrap();
    assert_eq!(
        duplicate
            .local_definitions()
            .iter()
            .filter(|definition| definition.is_lexical)
            .count(),
        2
    );
    assert_eq!(
        duplicate
            .code()
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Dup))
            .count(),
        1,
        "only the first same-scope if declaration is Annex-eligible"
    );

    for source in [
        "var prior;label:function prior(){}",
        "function prior(){}label:function prior(){}",
        "let prior;label:function prior(){}",
    ] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().message(),
            "invalid redefinition of global identifier",
            "{source}"
        );
    }
    compile_unlinked_script("{var nested;}label:function nested(){}")
        .expect("a nested first var must not block the Program label exception");
}
