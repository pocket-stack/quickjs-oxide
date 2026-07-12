use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsString, ObjectRef, OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "Program function is callable before its declaration",
        "typeof basicFunction+'|'+basicFunction();function basicFunction(){return 7}",
    ),
    (
        "strict Program function is callable before its declaration",
        "'use strict';typeof strictFunction+'|'+strictFunction();function strictFunction(){return 8}",
    ),
    (
        "last duplicate Program function wins before authored code",
        "duplicateFunction();function duplicateFunction(){return 1}function duplicateFunction(){return 2}",
    ),
    (
        "strict duplicate Program functions remain valid",
        "'use strict';duplicateStrict();function duplicateStrict(){return 1}function duplicateStrict(){return 2}",
    ),
    (
        "Program var initializer runs after the last function hoist",
        "var firstMixed=mixedFunction();function mixedFunction(){return 1}var middleMixed=mixedFunction();var mixedFunction=function(){return 3};function mixedFunction(){return 2}firstMixed+'|'+middleMixed+'|'+mixedFunction()",
    ),
    (
        "Program var without initializer preserves the function hoist",
        "typeof varFunction+'|'+varFunction();var varFunction;function varFunction(){return 4}",
    ),
    (
        "Program function preserves an earlier script completion",
        "9;function completionFunction(){return 1}",
    ),
    (
        "Program function metadata and self reference use its declared name",
        "function metadataFunction(first,second){return metadataFunction===globalThis.metadataFunction}metadataFunction.name+'|'+metadataFunction.length+'|'+metadataFunction()+'|'+(metadataFunction.prototype.constructor===metadataFunction)",
    ),
    (
        "Program declaration name is mutable rather than a private function-name binding",
        "'use strict';function mutableDeclaration(){mutableDeclaration=3;return mutableDeclaration}mutableDeclaration()+'|'+globalThis.mutableDeclaration",
    ),
    (
        "Program function captures a later lexical binding",
        "function readLaterLexical(){return laterLexical}let laterLexical=5;readLaterLexical()",
    ),
    (
        "lexical-first quirk exposes the hoisted function before let initialization",
        "Function.savedLexicalFirst=lexicalFirst;let lexicalFirst=1;function lexicalFirst(){return 2}Function.savedLexicalFirst()+'|'+lexicalFirst+'|'+typeof globalThis.lexicalFirst",
    ),
    (
        "const-first quirk exposes the hoisted function before const initialization",
        "Function.savedConstFirst=constFirst;const constFirst=1;function constFirst(){return 2}Function.savedConstFirst()+'|'+constFirst",
    ),
    (
        "const self initializer reads the raw-hoisted function",
        "const constSelf=constSelf;function constSelf(){return 6}constSelf()",
    ),
    (
        "last duplicate function hoists into a preceding lexical cell",
        "Function.savedDuplicateLexical=duplicateLexical;let duplicateLexical=1;function duplicateLexical(){return 2}function duplicateLexical(){return 3}Function.savedDuplicateLexical()+'|'+duplicateLexical",
    ),
    (
        "Program function prologue preserves authored branch targets",
        "if(false){missingFunctionBranch}else{Function.functionBranch=1}function branchFunction(){return 4}Function.functionBranch+'|'+branchFunction()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "fixed global blocks Program function declaration",
        "function NaN(){return 1}",
    ),
    (
        "hoisted Program function body fault",
        "hoistedFault();\nfunction hoistedFault(){\nmissingInHoistedFunction;\n}",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "Program function followed by let",
        "function clash(){}\nlet clash;",
    ),
    (
        "Program function followed by const",
        "function clash(){}\nconst clash=1;",
    ),
    (
        "strict Program function named eval",
        "'use strict';\nfunction eval(){}",
    ),
    (
        "strict Program function named arguments",
        "'use strict';\nfunction arguments(){}",
    ),
];

const UNSUPPORTED_BOUNDARY_CASES: &[(&str, &str)] = &[
    (
        "async Program function declaration",
        "async function unsupportedAsync(){}",
    ),
    (
        "generator Program function declaration",
        "function* unsupportedGenerator(){}",
    ),
];

const ORACLE_PROPERTY_PROBE: &str = r#"
(function () {
function show(value) {
  return typeof value + '|' + String(value);
}
function observe(source) {
  try {
    return 'return|' + show(std.evalScript(source));
  } catch (error) {
    if (error !== null && typeof error === 'object')
      return 'throw|object|' + error.name + '|' + error.message;
    return 'throw|' + show(error);
  }
}
function descriptor(name) {
  var value = Object.getOwnPropertyDescriptor(globalThis, name);
  if (value === undefined)
    return 'missing';
  if ('value' in value)
    return 'data|' + typeof value.value + '|' + value.writable + '|' +
           value.enumerable + '|' + value.configurable;
  return 'accessor|' + typeof value.get + '|' + typeof value.set + '|' +
         value.enumerable + '|' + value.configurable;
}
function line(name, value) {
  print(name + '=' + value);
}

line('new-eval', observe(
  "typeof __qjo_new_function+'|'+__qjo_new_function();function __qjo_new_function(){return 1}"
));
line('new-state', descriptor('__qjo_new_function') + '|' +
     observe('__qjo_new_function.name'));

line('autoinit', observe(
  'parseInt();function parseInt(){return 19}'
) + '|' + descriptor('parseInt'));

Object.defineProperty(globalThis, '__qjo_config_function', {
  value: 3, writable: false, enumerable: false, configurable: true
});
line('config-data', observe(
  '__qjo_config_function();function __qjo_config_function(){return 10}'
) + '|' + descriptor('__qjo_config_function'));

Function.functionGets = 0;
Function.functionSets = 0;
Object.defineProperty(globalThis, '__qjo_accessor_function', {
  get: function () { Function.functionGets++; return 3; },
  set: function () { Function.functionSets++; },
  enumerable: false,
  configurable: true
});
line('config-accessor', observe(
  '__qjo_accessor_function();function __qjo_accessor_function(){return 11}'
) + '|' + Function.functionGets + '|' + Function.functionSets + '|' +
     descriptor('__qjo_accessor_function'));

Object.defineProperty(globalThis, '__qjo_fixed_valid_function', {
  value: 3, writable: true, enumerable: true, configurable: false
});
line('fixed-valid', observe(
  '__qjo_fixed_valid_function();function __qjo_fixed_valid_function(){return 12}'
) + '|' + descriptor('__qjo_fixed_valid_function'));

Object.defineProperty(globalThis, '__qjo_fixed_readonly_function', {
  value: 3, writable: false, enumerable: true, configurable: false
});
line('fixed-readonly', observe(
  'function __qjo_fixed_readonly_function(){return 13}'
) + '|' + descriptor('__qjo_fixed_readonly_function'));

Object.defineProperty(globalThis, '__qjo_fixed_hidden_function', {
  value: 3, writable: true, enumerable: false, configurable: false
});
line('fixed-hidden', observe(
  'function __qjo_fixed_hidden_function(){return 14}'
) + '|' + descriptor('__qjo_fixed_hidden_function'));

Function.fixedAccessorGets = 0;
Function.fixedAccessorSets = 0;
Object.defineProperty(globalThis, '__qjo_fixed_accessor_function', {
  get: function () { Function.fixedAccessorGets++; return 3; },
  set: function () { Function.fixedAccessorSets++; },
  enumerable: true,
  configurable: false
});
line('fixed-accessor', observe(
  'function __qjo_fixed_accessor_function(){return 15}'
) + '|' + Function.fixedAccessorGets + '|' + Function.fixedAccessorSets + '|' +
     descriptor('__qjo_fixed_accessor_function'));

std.evalScript('let __qjo_function_priority=1');
Object.defineProperty(globalThis, '__qjo_function_priority', {
  value: 3, writable: false, enumerable: true, configurable: false
});
line('type-before-lexical', observe(
  'function __qjo_function_priority(){return 18}'
) + '|' + descriptor('__qjo_function_priority'));

globalThis.__qjo_function_marker = 0;
Object.defineProperty(globalThis, '__qjo_blocked_function', {
  value: 3, writable: false, enumerable: true, configurable: false
});
line('atomic-failure', observe(
  '__qjo_function_marker=1;function __qjo_fresh_function(){return 1}function __qjo_blocked_function(){return 2}'
));
line('atomic-state', observe(
  "__qjo_function_marker+'|'+typeof __qjo_fresh_function"
) + '|' + descriptor('__qjo_fresh_function') + '|' +
     descriptor('__qjo_blocked_function'));

Object.defineProperty(globalThis, '__qjo_sealed_existing_function', {
  value: 3, writable: false, enumerable: false, configurable: true
});
globalThis.__qjo_sealed_function_marker = 0;
Object.preventExtensions(globalThis);
line('sealed-existing', observe(
  '__qjo_sealed_existing_function();function __qjo_sealed_existing_function(){return 16}'
) + '|' + descriptor('__qjo_sealed_existing_function'));
line('sealed-missing', observe(
  '__qjo_sealed_function_marker=1;function __qjo_sealed_missing_function(){return 17}'
));
line('sealed-state', observe(
  "__qjo_sealed_function_marker+'|'+typeof __qjo_sealed_missing_function"
) + '|' + descriptor('__qjo_sealed_missing_function'));
})();
"#;

#[test]
fn program_function_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = observe_rust_eval(&runtime, &mut context, source, description);
        let quickjs = observe_oracle_sequence(&oracle, &[source], description);
        assert_eq!(rust, quickjs, "Program function drifted for {description}");
    }
}

#[test]
fn program_function_cross_eval_state_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function state differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let sequences: &[(&str, &[&str])] = &[
        (
            "later Program function declaration replaces the global function identity",
            &[
                "function crossFunction(){return 1}Function.oldCrossFunction=crossFunction;undefined",
                "crossFunction()",
                "function crossFunction(){return 2}crossFunction()",
                "Function.oldCrossFunction()+'|'+crossFunction()+'|'+(Function.oldCrossFunction===crossFunction)",
            ],
        ),
        (
            "configurable property becomes a fixed Program function binding",
            &[
                "globalThis.configFunction=1",
                "function configFunction(){return 3}configFunction()",
                "delete configFunction",
                "let configFunction=4",
                "configFunction()",
            ],
        ),
        (
            "existing global lexical blocks a later Program function",
            &[
                "let lexicalFunctionConflict=1",
                "function lexicalFunctionConflict(){return 2}",
                "lexicalFunctionConflict+'|'+typeof globalThis.lexicalFunctionConflict",
            ],
        ),
        (
            "Program var and function share one global cell across evals",
            &[
                "var varFunctionConflict=1",
                "function varFunctionConflict(){return 2}varFunctionConflict()",
                "var varFunctionConflict",
                "varFunctionConflict()",
                "var varFunctionConflict=4",
                "varFunctionConflict",
            ],
        ),
        (
            "Program function closure retains a later lexical binding",
            &[
                "function persistentLexicalRead(){return persistentLexical}let persistentLexical=5;persistentLexicalRead()",
                "persistentLexicalRead()+'|'+persistentLexical",
            ],
        ),
        (
            "failed lexical initializer preserves its raw-hoisted function",
            &[
                "let failedLexicalFunction=missingLexicalInitializer;function failedLexicalFunction(){return 9}",
                "typeof failedLexicalFunction+'|'+failedLexicalFunction()+'|'+typeof globalThis.failedLexicalFunction",
            ],
        ),
    ];

    for &(description, sources) in sequences {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = sources
            .iter()
            .map(|source| observe_rust_eval(&runtime, &mut context, source, description))
            .collect::<Vec<_>>()
            .join("\n");
        let quickjs = observe_oracle_sequence(&oracle, sources, description);
        assert_eq!(
            rust, quickjs,
            "Program function state drifted for {description}"
        );
    }
}

#[test]
fn program_function_global_property_matrix_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function property differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        rust_property_observations(),
        oracle_property_observations(&oracle),
        "Program function global-property behavior drifted"
    );
}

#[test]
fn program_function_full_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn program_function_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn unsupported_program_function_boundaries_remain_explicit() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function boundaries: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in UNSUPPORTED_BOUNDARY_CASES {
        let quickjs = observe_oracle_sequence(&oracle, &[source], description);
        assert!(
            quickjs.starts_with("return|"),
            "pinned QuickJS unexpectedly rejected {description}: {quickjs}"
        );
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = observe_rust_eval(&runtime, &mut context, source, description);
        assert!(
            rust.starts_with("throw|object|SyntaxError|"),
            "unsupported {description} was not rejected explicitly: {rust}"
        );
    }
}

#[test]
fn program_function_cross_realm_matches_pinned_c_api() {
    let Some(_oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program function cross-realm oracle: set QJS_ORACLE to upstream qjs");
        return;
    };

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining.eval("globalThis.realmTag='A'").unwrap();
    caller.eval("globalThis.realmTag='B'").unwrap();

    let bytecode = defining
        .compile("function crossRealmFunction(){return realmTag}crossRealmFunction")
        .unwrap();
    assert_eq!(
        defining.eval("typeof crossRealmFunction").unwrap(),
        Value::String(JsString::try_from_utf8("undefined").unwrap())
    );
    assert_eq!(
        caller.eval("typeof crossRealmFunction").unwrap(),
        Value::String(JsString::try_from_utf8("undefined").unwrap())
    );

    let Value::Object(function_a) = defining.execute(&bytecode).unwrap() else {
        panic!("defining execution did not return its Program function");
    };
    let Value::Object(function_b) = caller.execute(&bytecode).unwrap() else {
        panic!("caller execution did not return its Program function");
    };
    let Value::Object(function_prototype_a) = defining.eval("Function.prototype").unwrap() else {
        panic!("defining Function.prototype was not an object");
    };
    let Value::Object(function_prototype_b) = caller.eval("Function.prototype").unwrap() else {
        panic!("caller Function.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&function_a).unwrap(),
        Some(function_prototype_a.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&function_b).unwrap(),
        Some(function_prototype_a.clone())
    );
    assert_ne!(
        runtime.get_prototype_of(&function_b).unwrap(),
        Some(function_prototype_b)
    );
    let callable_a = runtime.as_callable(&function_a).unwrap().unwrap();
    let callable_b = runtime.as_callable(&function_b).unwrap().unwrap();
    assert_eq!(
        defining.call(&callable_a, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::try_from_utf8("A").unwrap())
    );
    assert_eq!(
        caller.call(&callable_b, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::try_from_utf8("B").unwrap())
    );
    assert_eq!(
        defining.eval("crossRealmFunction()").unwrap(),
        Value::String(JsString::try_from_utf8("A").unwrap())
    );
    assert_eq!(
        caller.eval("crossRealmFunction()").unwrap(),
        Value::String(JsString::try_from_utf8("B").unwrap())
    );

    define_global_data(
        &runtime,
        &mut caller,
        "blockedRealmFunction",
        Value::Int(1),
        false,
        true,
        false,
    );
    let blocked = defining
        .compile("Function.preflightSide=1;function blockedRealmFunction(){}")
        .unwrap();
    assert_eq!(
        caller.execute(&blocked),
        Err(RuntimeError::Exception),
        "caller preflight unexpectedly accepted a fixed property"
    );
    let error = take_object_exception(&mut caller, "cross-realm function preflight");
    let Value::Object(caller_type_prototype) = caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_prototype)
    );
    assert_eq!(
        defining.eval("typeof Function.preflightSide").unwrap(),
        Value::String(JsString::try_from_utf8("undefined").unwrap())
    );
    assert_eq!(
        caller.eval("typeof Function.preflightSide").unwrap(),
        Value::String(JsString::try_from_utf8("undefined").unwrap())
    );

    let body_error = defining
        .compile("function realmBodyError(){missingRealmBody}realmBodyError()")
        .unwrap();
    assert_eq!(
        caller.execute(&body_error),
        Err(RuntimeError::Exception),
        "cross-realm function body unexpectedly succeeded"
    );
    let error = take_object_exception(&mut caller, "cross-realm function body");
    let Value::Object(defining_reference_prototype) =
        defining.eval("ReferenceError.prototype").unwrap()
    else {
        panic!("defining ReferenceError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_reference_prototype)
    );

    defining
        .eval("Function.aFunctionGets=0;Function.aFunctionSets=0")
        .unwrap();
    caller
        .eval("Function.bFunctionGets=0;Function.bFunctionSets=0")
        .unwrap();
    define_global_accessor(
        &runtime,
        &mut defining,
        "accessRealmFunction",
        "(function(){Function.aFunctionGets++;return 'Aaccess'})",
        "(function(){Function.aFunctionSets++})",
        false,
        true,
    );
    define_global_accessor(
        &runtime,
        &mut caller,
        "accessRealmFunction",
        "(function(){Function.bFunctionGets++;return 'Baccess'})",
        "(function(){Function.bFunctionSets++})",
        false,
        true,
    );
    let accessor = defining
        .compile("function accessRealmFunction(){return 33}accessRealmFunction")
        .unwrap();
    let Value::Object(access_function) = caller.execute(&accessor).unwrap() else {
        panic!("accessor replacement did not return a function");
    };
    assert_eq!(
        runtime.get_prototype_of(&access_function).unwrap(),
        Some(function_prototype_a)
    );
    let access_function = runtime.as_callable(&access_function).unwrap().unwrap();
    assert_eq!(
        caller
            .call(&access_function, Value::Undefined, &[])
            .unwrap(),
        Value::Int(33)
    );
    assert_eq!(
        defining
            .eval("accessRealmFunction+'|'+Function.aFunctionGets")
            .unwrap(),
        Value::String(JsString::try_from_utf8("Aaccess|1").unwrap())
    );
    assert_eq!(
        caller
            .eval("accessRealmFunction()+'|'+Function.bFunctionGets+'|'+Function.bFunctionSets")
            .unwrap(),
        Value::String(JsString::try_from_utf8("33|0|0").unwrap())
    );
    assert_eq!(
        descriptor_text(&runtime, &mut caller, "accessRealmFunction"),
        "data|function|true|true|false"
    );
}

fn rust_property_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut output = Vec::new();

    output.push(format!(
        "new-eval={}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "typeof __qjo_new_function+'|'+__qjo_new_function();function __qjo_new_function(){return 1}",
            "new Program function",
        )
    ));
    output.push(format!(
        "new-state={}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_new_function"),
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_new_function.name",
            "new Program function name",
        )
    ));

    let autoinit = observe_rust_eval(
        &runtime,
        &mut context,
        "parseInt();function parseInt(){return 19}",
        "AutoInit Program function replacement",
    );
    output.push(format!(
        "autoinit={autoinit}|{}",
        descriptor_text(&runtime, &mut context, "parseInt")
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_config_function",
        Value::Int(3),
        false,
        false,
        true,
    );
    let config_data = observe_rust_eval(
        &runtime,
        &mut context,
        "__qjo_config_function();function __qjo_config_function(){return 10}",
        "configurable data Program function",
    );
    output.push(format!(
        "config-data={config_data}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_config_function")
    ));

    context
        .eval("Function.functionGets=0;Function.functionSets=0")
        .unwrap();
    define_global_accessor(
        &runtime,
        &mut context,
        "__qjo_accessor_function",
        "(function(){Function.functionGets++;return 3})",
        "(function(){Function.functionSets++})",
        false,
        true,
    );
    let config_accessor = observe_rust_eval(
        &runtime,
        &mut context,
        "__qjo_accessor_function();function __qjo_accessor_function(){return 11}",
        "configurable accessor Program function",
    );
    output.push(format!(
        "config-accessor={config_accessor}|{}|{}|{}",
        eval_text(&mut context, "Function.functionGets"),
        eval_text(&mut context, "Function.functionSets"),
        descriptor_text(&runtime, &mut context, "__qjo_accessor_function"),
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_fixed_valid_function",
        Value::Int(3),
        true,
        true,
        false,
    );
    let fixed_valid = observe_rust_eval(
        &runtime,
        &mut context,
        "__qjo_fixed_valid_function();function __qjo_fixed_valid_function(){return 12}",
        "fixed valid Program function",
    );
    output.push(format!(
        "fixed-valid={fixed_valid}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_fixed_valid_function")
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_fixed_readonly_function",
        Value::Int(3),
        false,
        true,
        false,
    );
    let fixed_readonly = observe_rust_eval(
        &runtime,
        &mut context,
        "function __qjo_fixed_readonly_function(){return 13}",
        "fixed readonly Program function",
    );
    output.push(format!(
        "fixed-readonly={fixed_readonly}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_fixed_readonly_function")
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_fixed_hidden_function",
        Value::Int(3),
        true,
        false,
        false,
    );
    let fixed_hidden = observe_rust_eval(
        &runtime,
        &mut context,
        "function __qjo_fixed_hidden_function(){return 14}",
        "fixed non-enumerable Program function",
    );
    output.push(format!(
        "fixed-hidden={fixed_hidden}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_fixed_hidden_function")
    ));

    context
        .eval("Function.fixedAccessorGets=0;Function.fixedAccessorSets=0")
        .unwrap();
    define_global_accessor(
        &runtime,
        &mut context,
        "__qjo_fixed_accessor_function",
        "(function(){Function.fixedAccessorGets++;return 3})",
        "(function(){Function.fixedAccessorSets++})",
        true,
        false,
    );
    let fixed_accessor = observe_rust_eval(
        &runtime,
        &mut context,
        "function __qjo_fixed_accessor_function(){return 15}",
        "fixed accessor Program function",
    );
    output.push(format!(
        "fixed-accessor={fixed_accessor}|{}|{}|{}",
        eval_text(&mut context, "Function.fixedAccessorGets"),
        eval_text(&mut context, "Function.fixedAccessorSets"),
        descriptor_text(&runtime, &mut context, "__qjo_fixed_accessor_function"),
    ));

    assert_eq!(
        observe_rust_eval(
            &runtime,
            &mut context,
            "let __qjo_function_priority=1",
            "function preflight priority lexical setup",
        ),
        "return|undefined|undefined"
    );
    define_global_data(
        &runtime,
        &mut context,
        "__qjo_function_priority",
        Value::Int(3),
        false,
        true,
        false,
    );
    let priority = observe_rust_eval(
        &runtime,
        &mut context,
        "function __qjo_function_priority(){return 18}",
        "function TypeError-before-lexical preflight priority",
    );
    output.push(format!(
        "type-before-lexical={priority}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_function_priority")
    ));

    context.eval("globalThis.__qjo_function_marker=0").unwrap();
    define_global_data(
        &runtime,
        &mut context,
        "__qjo_blocked_function",
        Value::Int(3),
        false,
        true,
        false,
    );
    output.push(format!(
        "atomic-failure={}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_function_marker=1;function __qjo_fresh_function(){return 1}function __qjo_blocked_function(){return 2}",
            "atomic Program function preflight",
        )
    ));
    output.push(format!(
        "atomic-state={}|{}|{}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_function_marker+'|'+typeof __qjo_fresh_function",
            "state after Program function preflight",
        ),
        descriptor_text(&runtime, &mut context, "__qjo_fresh_function"),
        descriptor_text(&runtime, &mut context, "__qjo_blocked_function"),
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_sealed_existing_function",
        Value::Int(3),
        false,
        false,
        true,
    );
    context
        .eval("globalThis.__qjo_sealed_function_marker=0")
        .unwrap();
    let global = context.global_object().unwrap();
    runtime.prevent_extensions(&global).unwrap();
    let sealed_existing = observe_rust_eval(
        &runtime,
        &mut context,
        "__qjo_sealed_existing_function();function __qjo_sealed_existing_function(){return 16}",
        "existing Program function on non-extensible global",
    );
    output.push(format!(
        "sealed-existing={sealed_existing}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_sealed_existing_function")
    ));
    output.push(format!(
        "sealed-missing={}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_sealed_function_marker=1;function __qjo_sealed_missing_function(){return 17}",
            "missing Program function on non-extensible global",
        )
    ));
    output.push(format!(
        "sealed-state={}|{}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_sealed_function_marker+'|'+typeof __qjo_sealed_missing_function",
            "state after non-extensible Program function preflight",
        ),
        descriptor_text(&runtime, &mut context, "__qjo_sealed_missing_function"),
    ));

    output
}

fn oracle_property_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", ORACLE_PROPERTY_PROBE])
        .output()
        .expect("run QuickJS Program-function property oracle");
    assert!(
        output.status.success(),
        "QuickJS Program-function property oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Program-function property output was not UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn define_global_data(
    runtime: &Runtime,
    context: &mut Context,
    name: &str,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(writable),
                    enumerable: DescriptorField::Present(enumerable),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_global_accessor(
    runtime: &Runtime,
    context: &mut Context,
    name: &str,
    getter_source: &str,
    setter_source: &str,
    enumerable: bool,
    configurable: bool,
) {
    let getter = function(runtime, context, getter_source);
    let setter = function(runtime, context, setter_source);
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    enumerable: DescriptorField::Present(enumerable),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function setup did not produce an object: {source}");
    };
    runtime.as_callable(&object).unwrap().unwrap()
}

fn take_object_exception(context: &mut Context, description: &str) -> ObjectRef {
    let Value::Object(error) = context
        .take_exception()
        .unwrap_or_else(|failure| panic!("take {description} exception: {failure}"))
        .unwrap_or_else(|| panic!("{description} exception was missing"))
    else {
        panic!("{description} did not throw an object");
    };
    error
}

fn descriptor_text(runtime: &Runtime, context: &mut Context, name: &str) -> String {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    let Some(descriptor) = context.get_own_property(&global, &key).unwrap() else {
        return "missing".to_owned();
    };
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => format!(
            "data|{}|{writable}|{enumerable}|{configurable}",
            value_type(runtime, &value),
        ),
        CompleteOrdinaryPropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => format!(
            "accessor|{}|{}|{enumerable}|{configurable}",
            accessor_type(&get),
            accessor_type(&set),
        ),
    }
}

fn accessor_type(value: &Option<CallableRef>) -> &'static str {
    if value.is_some() {
        "function"
    } else {
        "undefined"
    }
}

fn eval_text(context: &mut Context, source: &str) -> String {
    primitive_value_text(context.eval(source).unwrap())
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Object(object) => {
            if runtime.as_callable(object).unwrap().is_some() {
                "function"
            } else {
                "object"
            }
        }
        Value::Symbol(_) => "symbol",
    }
}

fn primitive_value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}

fn observe_oracle_sequence(oracle: &OsStr, sources: &[&str], description: &str) -> String {
    let wrapper = r#"
(function () {
for (var index = 0; index < scriptArgs.length; index++) {
  try {
    var value = std.evalScript(scriptArgs[index]);
    print('return|' + typeof value + '|' + String(value));
  } catch (error) {
    if (error !== null && typeof error === 'object')
      print('throw|object|' + error.name + '|' + error.message);
    else
      print('throw|' + typeof error + '|' + String(error));
  }
}
})();
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper])
        .args(sources)
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS sequence failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn compare_cli(oracle: &OsStr, options: &[&str], source: &str, description: &str) {
    let rust = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        options,
        source,
        description,
    );
    let quickjs = run_cli(oracle, options, source, description);
    assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(rust.stdout, quickjs.stdout, "{description}");
    assert_eq!(rust.stderr, quickjs.stderr, "{description}");
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
    };
    value.to_utf8_lossy()
}

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
