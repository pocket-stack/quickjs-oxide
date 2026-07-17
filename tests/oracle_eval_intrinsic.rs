use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ErrorKind, JsString, Runtime, RuntimeError, Value};

// This probe deliberately stops before String source execution. It freezes the
// realm-local %eval% shell that can be implemented without silently treating a
// syntactic direct eval as an indirect Context::eval call.
const ORACLE_PROBE: &str = r#"
(function () {
    function flags(descriptor) {
        return (descriptor.writable ? "1" : "0") +
               (descriptor.enumerable ? "1" : "0") +
               (descriptor.configurable ? "1" : "0");
    }
    function isConstructor(value) {
        try {
            Reflect.construct(function () {}, [], value);
            return true;
        } catch (_) {
            return false;
        }
    }

    var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "eval");
    var lengthDescriptor = Object.getOwnPropertyDescriptor(eval, "length");
    var nameDescriptor = Object.getOwnPropertyDescriptor(eval, "name");
    var observations = [
        "metadata=" + [
            typeof eval,
            eval.name,
            eval.length,
            Object.getPrototypeOf(eval) === Function.prototype,
            Object.prototype.hasOwnProperty.call(eval, "prototype"),
            eval.prototype === undefined,
            isConstructor(eval),
            Object.getOwnPropertyNames(eval).join(","),
            globalDescriptor.value === eval,
            flags(globalDescriptor),
            flags(lengthDescriptor),
            flags(nameDescriptor)
        ].join("|")
    ];

    var constructError = "none";
    try {
        new eval();
    } catch (error) {
        constructError = error.name;
    }
    observations.push("construct=" + constructError);

    var marker = {};
    var alias = eval;
    observations.push("calls=" + [
        eval(marker) === marker,
        (eval)(marker) === marker,
        (0, eval)(marker) === marker,
        alias(marker) === marker,
        eval() === undefined
    ].join("|"));

    var coercions = 0;
    function poison() {
        coercions++;
        throw 99;
    }
    var ordinary = {};
    ordinary[Symbol.toPrimitive] = poison;
    ordinary.toString = poison;
    ordinary.valueOf = poison;
    var boxedString = new String("40 + 2");
    boxedString[Symbol.toPrimitive] = poison;
    boxedString.toString = poison;
    boxedString.valueOf = poison;
    var symbol = Symbol("eval source");
    observations.push("identity=" + [
        eval(ordinary) === ordinary,
        eval(boxedString) === boxedString,
        eval(symbol) === symbol,
        coercions
    ].join("|"));

    var held = eval;
    var deleted = delete globalThis.eval;
    var absent = typeof globalThis.eval === "undefined";
    var heldAfterDelete = held(ordinary) === ordinary;
    globalThis.eval = function replacement() { return 17; };
    var replacementVisible = globalThis.eval(ordinary) === 17;
    var heldAfterReplacement = held(ordinary) === ordinary;
    observations.push("mutation=" + [
        deleted,
        absent,
        heldAfterDelete,
        replacementVisible,
        heldAfterReplacement,
        coercions
    ].join("|"));

    return observations.join("\n");
})()
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    "metadata=function|eval|1|true|false|true|false|length,name|true|101|001|001",
    "construct=TypeError",
    "calls=true|true|true|true|true",
    "identity=true|true|true|0",
    "mutation=true|true|true|true|true|0",
];

#[test]
fn eval_shell_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(rust, EXPECTED_OBSERVATIONS, "host-side eval shell drifted");

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval intrinsic differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "eval intrinsic shell differed from pinned QuickJS"
    );
}

#[test]
fn foreign_realm_eval_callable_preserves_non_string_identity() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let eval = global_eval(&runtime, &mut defining);

    assert_eq!(
        caller.call(&eval, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );

    let ordinary = caller.new_object().unwrap();
    let caller_global = caller.global_object().unwrap();
    let Value::Object(returned) = caller
        .call(
            &eval,
            Value::Object(caller_global),
            &[Value::Object(ordinary.clone())],
        )
        .unwrap()
    else {
        panic!("foreign eval did not return the ordinary object");
    };
    assert_eq!(returned, ordinary);

    let Value::Object(boxed_string) = caller.eval("new String('40 + 2')").unwrap() else {
        panic!("String construction did not return an object");
    };
    let Value::Object(returned) = caller
        .call(&eval, Value::Null, &[Value::Object(boxed_string.clone())])
        .unwrap()
    else {
        panic!("foreign eval did not return the String object");
    };
    assert_eq!(returned, boxed_string);

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("foreign eval").unwrap()))
        .unwrap();
    let Value::Symbol(returned) = caller
        .call(&eval, Value::Bool(true), &[Value::Symbol(symbol.clone())])
        .unwrap()
    else {
        panic!("foreign eval did not return the Symbol");
    };
    assert_eq!(returned, symbol);
}

#[test]
fn primitive_string_eval_stays_a_typed_uncatchable_frontier() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let eval = global_eval(&runtime, &mut context);

    assert_unsupported(
        context.call(
            &eval,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("40 + 2").unwrap())],
        ),
        "host Context::call",
    );
    assert!(!context.has_exception());
    assert_eq!(context.take_exception().unwrap(), None);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_unsupported(
        context.eval(
            r#"(function () {
                try {
                    return eval("40 + 2");
                } catch (error) {
                    return "caught:" + error;
                }
            })()"#,
        ),
        "source-level try/catch",
    );
    assert!(
        !context.has_exception(),
        "Unsupported eval source execution leaked into the JavaScript exception slot"
    );
    assert_eq!(context.take_exception().unwrap(), None);
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(value) = context.eval(ORACLE_PROBE).unwrap() else {
        panic!("eval oracle probe did not return a String");
    };
    value.to_utf8_lossy().lines().map(str::to_owned).collect()
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let wrapper = "print(std.evalScript(scriptArgs[0]));";
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, ORACLE_PROBE])
        .output()
        .expect("run pinned QuickJS eval intrinsic oracle");
    assert!(
        output.status.success(),
        "pinned QuickJS eval intrinsic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("pinned QuickJS eval intrinsic oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn global_eval(runtime: &Runtime, context: &mut Context) -> CallableRef {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("eval").unwrap();
    let Value::Object(function) = context.get_property(&global, &key).unwrap() else {
        panic!("global eval was not an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .expect("global eval was not callable")
}

fn assert_unsupported(result: Result<Value, RuntimeError>, boundary: &str) {
    let Err(RuntimeError::Engine(error)) = result else {
        panic!("primitive String eval did not stay an engine error at {boundary}: {result:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{boundary}");
    assert_eq!(
        error.message(),
        "eval source execution is not implemented yet",
        "{boundary}"
    );
}
