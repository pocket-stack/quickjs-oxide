use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "Program var without initializer",
        "var empty;typeof empty+'|'+(empty===undefined)+'|'+typeof globalThis.empty+'|'+delete empty",
    ),
    (
        "duplicate Program var initializers run in source order",
        "globalThis.varLog='';var repeated=(varLog+='a',1);var repeated=(varLog+='b',repeated+1);varLog+'|'+repeated",
    ),
    (
        "Program var has no TDZ",
        "var selfValue=selfValue;typeof selfValue",
    ),
    (
        "Program var initializer performs NamedEvaluation",
        "var named=function(){};named.name",
    ),
    (
        "Program var flat array binding publishes global cells",
        "var [first,,third=3,...rest]=[1,2,undefined,4,5];first+'|'+third+'|'+rest.join(',')+'|'+globalThis.first+'|'+globalThis.third+'|'+globalThis.rest.length",
    ),
    (
        "Program var nested array binding supports defaults and rest patterns",
        "var [[first]=[40],...[second,third]]=[undefined,1,2];first+'|'+second+'|'+third+'|'+globalThis.first",
    ),
    (
        "Program var preserves an earlier script completion",
        "9;var completed=1",
    ),
    ("sloppy Program var accepts contextual let", "var let=1;let"),
    (
        "unreached blocks and switch clauses still hoist Program vars",
        "if(false){var blocked=1}switch(0){case 1:var switched=2}typeof blocked+'|'+typeof switched",
    ),
    (
        "classic for Program var uses one shared global cell",
        "var readLoop=function(){return loopValue};for(var loopValue=0;loopValue<3;loopValue++);loopValue+'|'+readLoop()",
    ),
    (
        "nested block lexical shadows a Program var",
        "var outerValue=1;{let outerValue=2;Function.blockValue=outerValue}outerValue+'|'+Function.blockValue",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[(
    "strict Program var initializer writes a fixed global",
    "'use strict';\nvar NaN=1;",
)];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "Program var followed by Program lexical",
        "var clash;\nlet clash;",
    ),
    (
        "Program lexical followed by Program var",
        "let clash;\nvar clash;",
    ),
    (
        "strict Program var eval binding",
        "'use strict';\nvar eval=1;",
    ),
    (
        "strict Program var arguments binding",
        "'use strict';\nvar arguments=1;",
    ),
];

struct BoundaryCase {
    description: &'static str,
    source: &'static str,
    rust_message: &'static str,
}

const BOUNDARY_CASES: &[BoundaryCase] = &[BoundaryCase {
    description: "Program var nested object destructuring",
    source: "var [{value}]=[{value:1}];value",
    rust_message: "object destructuring bindings are not implemented yet",
}];

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
    return 'data|' + show(value.value) + '|' + value.writable + '|' +
           value.enumerable + '|' + value.configurable;
  return 'accessor|' + typeof value.get + '|' + typeof value.set + '|' +
         value.enumerable + '|' + value.configurable;
}
function line(name, value) {
  print(name + '=' + value);
}

line('new-eval', observe('var __qjo_var_new'));
line('new-state', descriptor('__qjo_var_new') + '|' +
     observe('delete __qjo_var_new'));

Object.defineProperty(globalThis, '__qjo_var_config', {
  value: 10, writable: true, enumerable: false, configurable: true
});
line('config-no-init', observe('var __qjo_var_config') + '|' +
     descriptor('__qjo_var_config'));
line('config-init', observe('var __qjo_var_config=20') + '|' +
     descriptor('__qjo_var_config') + '|' + observe('__qjo_var_config'));
line('config-delete', observe('delete __qjo_var_config') + '|' +
     descriptor('__qjo_var_config'));

Object.defineProperty(globalThis, '__qjo_var_fixed', {
  value: 7, writable: false, enumerable: false, configurable: false
});
line('fixed-sloppy', observe('var __qjo_var_fixed=8') + '|' +
     descriptor('__qjo_var_fixed'));
line('fixed-strict', observe("'use strict';var __qjo_var_fixed=9") + '|' +
     descriptor('__qjo_var_fixed'));

Function.varGets = 0;
Function.varSets = 0;
Function.varSeen = 'none';
Object.defineProperty(globalThis, '__qjo_var_accessor', {
  get: function () { Function.varGets++; return 4; },
  set: function (value) { Function.varSets++; Function.varSeen = value; },
  enumerable: false,
  configurable: true
});
line('accessor-no-init', observe('var __qjo_var_accessor') + '|' +
     Function.varGets + '|' + Function.varSets + '|' + Function.varSeen + '|' +
     descriptor('__qjo_var_accessor'));
line('accessor-init', observe('var __qjo_var_accessor=12') + '|' +
     Function.varGets + '|' + Function.varSets + '|' + Function.varSeen + '|' +
     descriptor('__qjo_var_accessor'));

std.evalScript('globalThis.__qjo_var_atomic_marker=0;let __qjo_var_existing_lex=9');
line('atomic-failure', observe(
  '__qjo_var_atomic_marker=1;var __qjo_var_fresh_before=(__qjo_var_atomic_marker=2),__qjo_var_existing_lex=(__qjo_var_atomic_marker=3),__qjo_var_fresh_after=(__qjo_var_atomic_marker=4)'
));
line('atomic-state', observe(
  "__qjo_var_atomic_marker+'|'+typeof __qjo_var_fresh_before+'|'+typeof __qjo_var_fresh_after+'|'+__qjo_var_existing_lex"
) + '|' + descriptor('__qjo_var_fresh_before') + '|' +
     descriptor('__qjo_var_fresh_after'));

globalThis.__qjo_var_sealed_marker = 0;
globalThis.__qjo_var_sealed_existing = 5;
Object.preventExtensions(globalThis);
line('sealed-failure', observe(
  '__qjo_var_sealed_marker=1;var __qjo_var_sealed_existing=6,__qjo_var_sealed_missing=7'
));
line('sealed-state', observe(
  "__qjo_var_sealed_marker+'|'+__qjo_var_sealed_existing+'|'+typeof __qjo_var_sealed_missing"
) + '|' + descriptor('__qjo_var_sealed_missing'));
line('sealed-existing', observe('var __qjo_var_sealed_existing=8') + '|' +
     descriptor('__qjo_var_sealed_existing'));
})();
"#;

#[test]
fn program_var_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program var differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = observe_rust_eval(&runtime, &mut context, source, description);
        let quickjs = observe_oracle_sequence(&oracle, &[source], description);
        assert_eq!(rust, quickjs, "Program var drifted for {description}");
    }
}

#[test]
fn program_var_cross_eval_state_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program var state differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let sequences: &[(&str, &[&str])] = &[
        (
            "repeated Program var preserves and updates one global cell",
            &[
                "var crossValue=1;Function.crossRead=function(){return crossValue};undefined",
                "var crossValue",
                "crossValue+'|'+Function.crossRead()",
                "var crossValue=2;crossValue+'|'+Function.crossRead()",
                "let crossValue=3",
            ],
        ),
        (
            "hidden unresolved VarRef reconnects to a later Program var",
            &[
                "Function.hiddenVarRead=function(){return hiddenVar};undefined",
                "var hiddenVar=4",
                "Function.hiddenVarRead()+'|'+hiddenVar+'|'+typeof globalThis.hiddenVar",
            ],
        ),
        (
            "configurable global property survives var and reconnects after delete",
            &[
                "globalThis.reconnectVar=1;Function.reconnectVarRead=function(){return reconnectVar};undefined",
                "var reconnectVar=2",
                "Function.reconnectVarRead()+'|'+reconnectVar+'|'+globalThis.reconnectVar",
                "delete reconnectVar",
                "Function.reconnectVarRead()",
                "reconnectVar=3",
                "Function.reconnectVarRead()+'|'+reconnectVar+'|'+globalThis.reconnectVar",
            ],
        ),
        (
            "var preserves a configurable property so a later lexical may split",
            &[
                "globalThis.splitAfterVar=1",
                "var splitAfterVar=2",
                "let splitAfterVar=3",
                "splitAfterVar+'|'+globalThis.splitAfterVar",
                "var splitAfterVar=4",
                "splitAfterVar+'|'+globalThis.splitAfterVar",
            ],
        ),
        (
            "Program var preflight conflict is atomic and source ordered",
            &[
                "globalThis.varMarker=0;let existingVarConflict=9",
                "varMarker=1;var freshVarBefore=(varMarker=2),existingVarConflict=(varMarker=3),freshVarAfter=(varMarker=4)",
                "varMarker+'|'+typeof freshVarBefore+'|'+typeof freshVarAfter+'|'+existingVarConflict",
                "var freshVarBefore=5,freshVarAfter=6;freshVarBefore+'|'+freshVarAfter",
            ],
        ),
        (
            "abrupt initializer leaves every Program var instantiated",
            &[
                "globalThis.initializerLog='';Function.readFirstVar=function(){return firstVar};Function.readSecondVar=function(){return secondVar};Function.readThirdVar=function(){return thirdVar};var firstVar=(initializerLog+='a',1),secondVar=(initializerLog+='b',(function(){throw 17})()),thirdVar=(initializerLog+='c',3)",
                "initializerLog+'|'+Function.readFirstVar()+'|'+typeof Function.readSecondVar()+'|'+typeof Function.readThirdVar()",
                "var secondVar=2,thirdVar=3;firstVar+'|'+secondVar+'|'+thirdVar",
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
        assert_eq!(rust, quickjs, "Program var state drifted for {description}");
    }
}

#[test]
fn program_var_global_property_and_preflight_matrix_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program var property differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        rust_property_observations(),
        oracle_property_observations(&oracle),
        "Program var global-property or preflight behavior drifted"
    );
}

#[test]
fn program_var_full_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program var stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn program_var_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program var parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn selected_unsupported_program_var_boundaries_stay_explicit() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program var boundaries: set QJS_ORACLE to upstream qjs");
        return;
    };

    for case in BOUNDARY_CASES {
        let quickjs = run_cli(&oracle, &[], case.source, case.description);
        assert!(
            quickjs.status.success(),
            "pinned QuickJS rejected {}: {}",
            case.description,
            String::from_utf8_lossy(&quickjs.stderr),
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval(case.source),
            Err(RuntimeError::Exception),
            "Rust unexpectedly accepted {}",
            case.description,
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!(
                "Rust boundary did not throw an Error for {}",
                case.description
            );
        };
        assert_eq!(
            error_string_property(&runtime, &mut context, &error, "name", case.description),
            "SyntaxError"
        );
        assert_eq!(
            error_string_property(&runtime, &mut context, &error, "message", case.description),
            case.rust_message
        );
    }
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
            "var __qjo_var_new",
            "new Program var"
        )
    ));
    output.push(format!(
        "new-state={}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_var_new"),
        observe_rust_eval(
            &runtime,
            &mut context,
            "delete __qjo_var_new",
            "delete new Program var"
        )
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_var_config",
        Value::Int(10),
        true,
        false,
        true,
    );
    let config_no_init = observe_rust_eval(
        &runtime,
        &mut context,
        "var __qjo_var_config",
        "existing configurable Program var",
    );
    output.push(format!(
        "config-no-init={config_no_init}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_var_config")
    ));
    let config_init = observe_rust_eval(
        &runtime,
        &mut context,
        "var __qjo_var_config=20",
        "initialize configurable Program var",
    );
    let config_descriptor = descriptor_text(&runtime, &mut context, "__qjo_var_config");
    let config_value = observe_rust_eval(
        &runtime,
        &mut context,
        "__qjo_var_config",
        "read configurable Program var",
    );
    output.push(format!(
        "config-init={config_init}|{config_descriptor}|{config_value}"
    ));
    let config_delete = observe_rust_eval(
        &runtime,
        &mut context,
        "delete __qjo_var_config",
        "delete configurable Program var",
    );
    output.push(format!(
        "config-delete={config_delete}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_var_config")
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_var_fixed",
        Value::Int(7),
        false,
        false,
        false,
    );
    let fixed_sloppy = observe_rust_eval(
        &runtime,
        &mut context,
        "var __qjo_var_fixed=8",
        "sloppy fixed Program var initializer",
    );
    output.push(format!(
        "fixed-sloppy={fixed_sloppy}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_var_fixed")
    ));
    let fixed_strict = observe_rust_eval(
        &runtime,
        &mut context,
        "'use strict';var __qjo_var_fixed=9",
        "strict fixed Program var initializer",
    );
    output.push(format!(
        "fixed-strict={fixed_strict}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_var_fixed")
    ));

    context
        .eval("Function.varGets=0;Function.varSets=0;Function.varSeen='none'")
        .unwrap();
    define_global_accessor(&runtime, &mut context, "__qjo_var_accessor");
    let accessor_no_init = observe_rust_eval(
        &runtime,
        &mut context,
        "var __qjo_var_accessor",
        "accessor Program var without initializer",
    );
    output.push(format!(
        "accessor-no-init={accessor_no_init}|{}|{}|{}|{}",
        eval_text(&mut context, "Function.varGets"),
        eval_text(&mut context, "Function.varSets"),
        eval_text(&mut context, "Function.varSeen"),
        descriptor_text(&runtime, &mut context, "__qjo_var_accessor"),
    ));
    let accessor_init = observe_rust_eval(
        &runtime,
        &mut context,
        "var __qjo_var_accessor=12",
        "accessor Program var initializer",
    );
    output.push(format!(
        "accessor-init={accessor_init}|{}|{}|{}|{}",
        eval_text(&mut context, "Function.varGets"),
        eval_text(&mut context, "Function.varSets"),
        eval_text(&mut context, "Function.varSeen"),
        descriptor_text(&runtime, &mut context, "__qjo_var_accessor"),
    ));

    context
        .eval("globalThis.__qjo_var_atomic_marker=0;let __qjo_var_existing_lex=9")
        .unwrap();
    output.push(format!(
        "atomic-failure={}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_var_atomic_marker=1;var __qjo_var_fresh_before=(__qjo_var_atomic_marker=2),__qjo_var_existing_lex=(__qjo_var_atomic_marker=3),__qjo_var_fresh_after=(__qjo_var_atomic_marker=4)",
            "atomic Program var preflight",
        )
    ));
    output.push(format!(
        "atomic-state={}|{}|{}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_var_atomic_marker+'|'+typeof __qjo_var_fresh_before+'|'+typeof __qjo_var_fresh_after+'|'+__qjo_var_existing_lex",
            "state after Program var preflight",
        ),
        descriptor_text(&runtime, &mut context, "__qjo_var_fresh_before"),
        descriptor_text(&runtime, &mut context, "__qjo_var_fresh_after"),
    ));

    context
        .eval("globalThis.__qjo_var_sealed_marker=0;globalThis.__qjo_var_sealed_existing=5")
        .unwrap();
    let global = context.global_object().unwrap();
    runtime.prevent_extensions(&global).unwrap();
    output.push(format!(
        "sealed-failure={}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_var_sealed_marker=1;var __qjo_var_sealed_existing=6,__qjo_var_sealed_missing=7",
            "non-extensible Program var preflight",
        )
    ));
    output.push(format!(
        "sealed-state={}|{}",
        observe_rust_eval(
            &runtime,
            &mut context,
            "__qjo_var_sealed_marker+'|'+__qjo_var_sealed_existing+'|'+typeof __qjo_var_sealed_missing",
            "state after non-extensible Program var preflight",
        ),
        descriptor_text(&runtime, &mut context, "__qjo_var_sealed_missing"),
    ));
    let sealed_existing = observe_rust_eval(
        &runtime,
        &mut context,
        "var __qjo_var_sealed_existing=8",
        "existing Program var on non-extensible global",
    );
    output.push(format!(
        "sealed-existing={sealed_existing}|{}",
        descriptor_text(&runtime, &mut context, "__qjo_var_sealed_existing")
    ));

    output
}

fn oracle_property_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", ORACLE_PROPERTY_PROBE])
        .output()
        .expect("run QuickJS Program-var property oracle");
    assert!(
        output.status.success(),
        "QuickJS Program-var property oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Program-var property output was not UTF-8")
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

fn define_global_accessor(runtime: &Runtime, context: &mut Context, name: &str) {
    let getter = function(
        runtime,
        context,
        "(function(){Function.varGets++;return 4})",
    );
    let setter = function(
        runtime,
        context,
        "(function(value){Function.varSets++;Function.varSeen=value})",
    );
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
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
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
            "data|{}|{}|{writable}|{enumerable}|{configurable}",
            value_type(&value),
            primitive_value_text(value),
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
    match value {
        None => "undefined",
        Some(_) => "function",
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
            value_type(&value),
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
                    value_type(&value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Object(_) => "object",
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
