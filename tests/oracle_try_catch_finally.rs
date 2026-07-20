use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const CATCH_CASES: &[(&str, &str)] = &[
    ("basic primitive catch", "try{throw 7}catch(e){e}"),
    ("optional catch binding", "try{throw 8}catch{9}"),
    ("no-throw path skips catch", "try{10}catch(e){11}"),
    (
        "primitive throw preserves type and value",
        "try{throw 'primitive'}catch(e){typeof e+'|'+e}",
    ),
    (
        "object throw preserves identity",
        "try{throw Function}catch(e){typeof e+'|'+(e===Function)}",
    ),
    (
        "VM native error is catchable",
        "try{null.x}catch(e){e.name+'|'+e.message}",
    ),
    (
        "native getter error is catchable",
        "try{Function.prototype.caller}catch(e){e.name+'|'+e.message}",
    ),
    (
        "callee throw reaches caller catch",
        "(function(){try{(function thrownCallee(){throw 12})()}catch(e){return e}})()",
    ),
    (
        "constructor throw reaches caller catch",
        "(function(){var C=function ThrownConstructor(){throw 13};try{new C()}catch(e){return e}})()",
    ),
    (
        "nearest catch wins",
        "(function(){var s='';try{try{throw 1}catch(e){s+='i'+e}}catch(e){s+='o'+e}return s})()",
    ),
    (
        "rethrow reaches the outer catch",
        "(function(){try{try{throw 1}catch(e){throw e}}catch(e){return e}})()",
    ),
    (
        "new throw from catch reaches the outer catch",
        "(function(){try{try{throw 1}catch(e){throw 3}}catch(e){return e}})()",
    ),
];

const CATCH_SCOPE_CASES: &[(&str, &str)] = &[
    (
        "catch binding permits same-name var",
        "(function(){try{throw 4}catch(e){var e;return e}})()",
    ),
    (
        "closure captures catch binding",
        "(function(){var f;try{throw 4}catch(e){f=function(){return e}}return f()})()",
    ),
    (
        "re-entered catch receives a fresh captured cell",
        "(function(){var f,g,i=0;while(i<2){try{throw ++i}catch(e){if(i===1)f=function(){return e};else g=function(){return e}}}return f()+'|'+g()+'|'+(f()!==g())})()",
    ),
];

const FINALLY_CASES: &[(&str, &str)] = &[
    (
        "normal try executes finally",
        "(function(){var s='';try{s+='t'}finally{s+='f'}return s})()",
    ),
    (
        "caught throw executes finally",
        "(function(){var s='';try{s+='t';throw 'c'}catch(e){s+=e}finally{s+='f'}return s})()",
    ),
    (
        "uncaught inner throw executes finally before outer catch",
        "(function(){var s='';try{try{throw 1}finally{s+='f'}}catch(e){return s+'|'+e}})()",
    ),
    (
        "return value crosses finally",
        "(function(){var s='';var r=(function(){try{return 1}finally{s+='f'}})();return r+'|'+s})()",
    ),
    (
        "catch inside finally preserves the outer gosub return address",
        "(function(){var s='';var r=(function(){try{return 1}finally{try{throw 2}catch(e){s+=e}}})();return r+'|'+s})()",
    ),
    (
        "finally return overrides pending return",
        "(function(){try{return 1}finally{return 2}})()",
    ),
    (
        "finally throw overrides pending throw",
        "(function(){try{try{throw 1}finally{throw 2}}catch(e){return e}})()",
    ),
    (
        "throw from catch still executes finally",
        "(function(){var s='';try{try{throw 1}catch(e){s+='c';throw 2}finally{s+='f'}}catch(e){return s+'|'+e}})()",
    ),
    (
        "unlabelled break crosses one finally",
        "(function(){var s='';while(true){try{s+='t';break}finally{s+='f'}}return s})()",
    ),
    (
        "unlabelled continue crosses one finally",
        "(function(){var s='',i=0;while(i<2){i++;try{continue}finally{s+=i}}return s})()",
    ),
    (
        "unlabelled break crosses nested finally clauses",
        "(function(){var s='';while(true){try{try{s+='t';break}finally{s+='i'}}finally{s+='o'}}return s})()",
    ),
    (
        "unlabelled continue crosses nested finally clauses",
        "(function(){var s='',i=0;while(i<2){i++;try{try{if(i<2)continue;s+='b'}finally{s+='i'}}finally{s+='o'}}return s})()",
    ),
    (
        "labelled break crosses one finally",
        "(function(){var s='';outer:while(true){try{s+='t';break outer}finally{s+='f'}}return s})()",
    ),
    (
        "labelled continue crosses one finally",
        "(function(){var s='',i=0;outer:while(i<2){i++;try{continue outer}finally{s+=i}}return s})()",
    ),
    (
        "labelled break crosses nested finally clauses",
        "(function(){var s='';outer:while(true){try{try{s+='t';break outer}finally{s+='i'}}finally{s+='o'}}return s})()",
    ),
    (
        "labelled continue crosses nested finally clauses",
        "(function(){var s='',i=0;outer:while(i<2){i++;try{try{if(i<2)continue outer;s+='b'}finally{s+='i'}}finally{s+='o'}}return s})()",
    ),
    (
        "nested throw catch and finally order",
        "(function(){var s='';try{try{s+='t';throw 'a'}finally{s+='f'}}catch(e){s+=e}finally{s+='g'}return s})()",
    ),
    (
        "switch discriminant remains below catch and finally state",
        "(function(v){var s='';switch(v){case 1:try{s+='t';throw 'x'}catch(e){s+=e}finally{s+='f'}case 2:s+='2';break;default:s+='d'}return s})(1)",
    ),
];

const COMPLETION_AND_QUIRK_CASES: &[(&str, &str)] = &[
    (
        "normal finally does not replace Script completion",
        "try{1}finally{2}",
    ),
    (
        "normal finally does not replace catch completion",
        "try{throw 1}catch(e){3}finally{2}",
    ),
    (
        "caught throw preserves QuickJS intervening captured-cell reuse",
        "(function(){var f,g,i=0;while(i<2){i++;try{{let x=i;if(i===1)f=function(){return x};else g=function(){return x};throw 0}}catch(e){}}return f()+'|'+g()+'|'+(f===g)})()",
    ),
];

const ABRUPT_FINALLY_CELL_REUSE_CASES: &[(&str, &str)] = &[
    (
        "return overridden by finally continue reuses the captured try lexical",
        "(function(){var f,g,i=0;while(i<2){try{let x=++i;if(i===1)f=function(){return x};else g=function(){return x};return 9}finally{continue}}return f()+'|'+g()})()",
    ),
    (
        "return overridden by finally break reuses the captured try lexical after outer-loop re-entry",
        "(function(){var f,g,i=0;while(i<2){while(true){try{let x=++i;if(i===1)f=function(){return x};else g=function(){return x};return 9}finally{break}}}return f()+'|'+g()})()",
    ),
    (
        "throw overridden by finally continue reuses the captured try lexical",
        "(function(){var f,g,i=0;while(i<2){try{let x=++i;if(i===1)f=function(){return x};else g=function(){return x};throw 9}finally{continue}}return f()+'|'+g()})()",
    ),
    (
        "throw overridden by finally break reuses the captured try lexical after outer-loop re-entry",
        "(function(){var f,g,i=0;while(i<2){while(true){try{let x=++i;if(i===1)f=function(){return x};else g=function(){return x};throw 9}finally{break}}}return f()+'|'+g()})()",
    ),
    (
        "nested finally throw then continue overrides return and re-enters the captured lexical",
        "(function(){var f,g,i=0;while(i<2){try{try{let x=++i;if(i===1)f=function(){return x};else g=function(){return x};return 9}finally{throw 8}}finally{continue}}return f()+'|'+g()})()",
    ),
    (
        "nested finally return then continue overrides throw and re-enters the captured lexical",
        "(function(){var f,g,i=0;while(i<2){try{try{let x=++i;if(i===1)f=function(){return x};else g=function(){return x};throw 9}finally{return 8}}finally{continue}}return f()+'|'+g()})()",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "caught native fault rethrown without replacing its origin",
        "(function outer(){\n  try {\n    (function inner(){ null.rethrowFault; })();\n  } catch (error) {\n    throw error;\n  }\n})()",
    ),
    (
        "fault raised from a catch body",
        "(function outer(){\n  try {\n    throw 1;\n  } catch (error) {\n    (function catchInner(){ null.catchFault; })();\n  }\n})()",
    ),
    (
        "finally fault overrides the try fault",
        "(function outer(){\n  try {\n    null.tryFault;\n  } finally {\n    null.finallyFault;\n  }\n})()",
    ),
    (
        "caught Error object keeps its eager callee stack",
        "(function outer(){\n  try {\n    (function inner(){ throw new Error('boom'); })();\n  } catch (error) {\n    throw error;\n  }\n})()",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    ("try requires catch or finally", "try{}"),
    ("empty catch parameter is invalid", "try{}catch(){}"),
    ("reserved catch parameter is invalid", "try{}catch(if){}"),
    (
        "catch parameter requires a right parenthesis",
        "try{}catch(e {}",
    ),
    ("finally requires a block", "try{}finally 1"),
    ("catch cannot appear without try", "catch(e){}"),
    ("finally cannot appear without try", "finally{}"),
    (
        "catch binding conflicts with same-block let",
        "try{throw 4}catch(e){let e}",
    ),
    (
        "catch binding conflicts with same-block const",
        "try{throw 4}catch(e){const e=1}",
    ),
    (
        "strict catch binding rejects eval",
        "'use strict';try{}catch(eval){}",
    ),
    (
        "catch cannot follow a completed finally",
        "try{}finally{}catch(e){}",
    ),
];

#[test]
fn catch_dispatch_values_match_pinned_quickjs() {
    compare_value_cases("catch dispatch", CATCH_CASES);
}

#[test]
fn catch_binding_scope_values_match_pinned_quickjs() {
    compare_value_cases("catch binding scope", CATCH_SCOPE_CASES);
}

#[test]
fn finally_abrupt_completion_values_match_pinned_quickjs() {
    compare_value_cases("finally abrupt completion", FINALLY_CASES);
}

#[test]
fn try_completion_and_caught_throw_quirk_match_pinned_quickjs() {
    compare_value_cases(
        "try completion and caught-throw quirk",
        COMPLETION_AND_QUIRK_CASES,
    );
}

#[test]
fn abrupt_finally_override_cell_reuse_matches_pinned_quickjs() {
    compare_value_cases(
        "abrupt finally override captured-cell reuse",
        ABRUPT_FINALLY_CELL_REUSE_CASES,
    );
}

#[test]
fn try_catch_finally_full_strip_source_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP try/catch/finally stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["--strip-source"], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn try_catch_finally_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP try/catch/finally parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn try_catch_cross_realm_regression() {
    // Pinned with the QuickJS C API compile-in-A/execute-in-B path. The Rust
    // API exposes that operation directly; QJS_ORACLE keeps this test grouped
    // with the same checksum-pinned release as the process differentials.
    let Some(_oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP try/catch cross-realm oracle: set QJS_ORACLE to upstream qjs");
        return;
    };

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let caught_error = defining.compile("try{null.realmFault}catch(e){e}").unwrap();
    let Value::Object(caught_error) = caller.execute(&caught_error).unwrap() else {
        panic!("cross-realm catch did not return its native Error object");
    };
    let Value::Object(type_error_a) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(type_error_b) = caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&caught_error).unwrap(),
        Some(type_error_a)
    );
    assert_ne!(
        runtime.get_prototype_of(&caught_error).unwrap(),
        Some(type_error_b)
    );

    let caught_object = defining.compile("try{throw Function}catch(e){e}").unwrap();
    let caught_object = caller.execute(&caught_object).unwrap();
    let caller_function = caller.eval("Function").unwrap();
    let defining_function = defining.eval("Function").unwrap();
    assert_eq!(caught_object, caller_function);
    assert_ne!(caught_object, defining_function);

    let caught_closure = defining
        .compile("try{throw 5}catch(e){(function(){return e})}")
        .unwrap();
    let Value::Object(caught_closure) = caller.execute(&caught_closure).unwrap() else {
        panic!("cross-realm catch did not return its captured closure");
    };
    let Value::Object(function_prototype_a) = defining.eval("Function.prototype").unwrap() else {
        panic!("defining Function.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&caught_closure).unwrap(),
        Some(function_prototype_a)
    );
    let caught_closure = runtime.as_callable(&caught_closure).unwrap().unwrap();
    assert_eq!(
        caller.call(&caught_closure, Value::Undefined, &[]).unwrap(),
        Value::Int(5)
    );
}

fn compare_value_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group} differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "{group} drifted for {description}: {source:?}",
        );
    }
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

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
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
