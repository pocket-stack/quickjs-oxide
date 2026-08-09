use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

const PROBE: &str = r#"
function show(value) {
    if (value === undefined) return "undefined:undefined";
    if (value === null) return "object:null";
    return typeof value + ":" + String(value);
}
function observe(thunk) {
    try { return show(thunk()); }
    catch (error) { return "throw:" + error.name + ":" + error.message; }
}
function observeSource(source) {
    try { return show((0, eval)(source)); }
    catch (error) { return "throw:" + error.name + ":" + error.message; }
}

print("identifier-string=" + show((function(){
    var value = "41";
    var old = value++;
    return typeof old + ":" + old + "|" + typeof value + ":" + value;
})()));
print("identifier-bigint=" + show((function(){
    var value = 41n;
    var post = value--;
    var prefix = --value;
    return post * 10000n + prefix * 100n + value;
})()));

let log = "";
let coercible = {};
Object.defineProperty(coercible, Symbol.toPrimitive, {
    value(hint) { log += "v(" + hint + ")"; return 7; }
});
log = "";
const objectOld = coercible++;
print("identifier-object-post=" + [show(objectOld), show(coercible), log].join("|"));

let bigintCoercible = {};
Object.defineProperty(bigintCoercible, Symbol.toPrimitive, {
    value(hint) { log += "B(" + hint + ")"; return 7n; }
});
log = "";
const bigintObjectOld = bigintCoercible++;
print("identifier-object-bigint-post=" +
      [show(bigintObjectOld), show(bigintCoercible), log].join("|"));

let symbolValue = Symbol("update");
print("identifier-symbol=" + observe(() => symbolValue++) + "|" + typeof symbolValue);

let seenValue;
const computedOld = {};
Object.defineProperty(computedOld, Symbol.toPrimitive, {
    value(hint) { log += "v(" + hint + ")"; return 7; }
});
const target = {};
Object.defineProperty(target, "member", {
    get() { log += "g"; return computedOld; },
    set(value) { log += "s"; seenValue = value; },
    configurable: true
});
const key = {};
Object.defineProperty(key, Symbol.toPrimitive, {
    value(hint) { log += "k(" + hint + ")"; return "member"; }
});
function baseExpr() { log += "b"; return target; }
function keyExpr() { log += "q"; return key; }

log = ""; seenValue = undefined;
const computedPost = baseExpr()[keyExpr()]++;
print("computed-post=" + [show(computedPost), log, show(seenValue)].join("|"));
log = ""; seenValue = undefined;
const computedPrefix = ++baseExpr()[keyExpr()];
print("computed-prefix=" + [show(computedPrefix), log, show(seenValue)].join("|"));

const readonlyOld = {};
Object.defineProperty(readonlyOld, Symbol.toPrimitive, {
    value(hint) { log += "r(" + hint + ")"; return 4; }
});
const readonlyTarget = {};
Object.defineProperty(readonlyTarget, "value", {
    value: readonlyOld, writable: false, configurable: true
});
log = "";
const readonlySloppy = readonlyTarget.value++;
print("readonly-member-sloppy=" + [show(readonlySloppy),
      show(readonlyTarget.value === readonlyOld), log].join("|"));
log = "";
print("readonly-member-strict=" + observe(() =>
      (function(){ "use strict"; return ++readonlyTarget.value; })()) + "|" + log);

Object.defineProperty(globalThis, "readonlyIdentifier", {
    value: "4", writable: false, configurable: true
});
const readonlyIdentifierOld = readonlyIdentifier++;
print("readonly-identifier-sloppy=" +
      [show(readonlyIdentifierOld), show(readonlyIdentifier)].join("|"));
print("readonly-identifier-strict=" + observe(() =>
      (function(){ "use strict"; return ++readonlyIdentifier; })()));

print("missing-post=" + observe(() => updateMissing++));
log = "";
print("null-computed=" + observe(() => null[keyExpr()]++) + "|" + log);

print("asi-lf=" + show((function(){
    var x = 1, y = 4;
    var result = x
    ++y;
    return result * 100 + x * 10 + y;
})()));
print("power-updates=" + show((function(){
    var x = 2;
    return (++x ** 2) * 100 + (x++ ** 2) * 10 + x;
})()));
print("power-unary-error=" +
      observeSource("(function(){ var x = 2; -x++ ** 2; })()"));
print("invalid-update=" +
      observeSource("(function(){ var x = 2; ++(x ** 2); })()"));
print("strict-arguments=" +
      observeSource("\"use strict\"; arguments++"));
"#;

#[test]
fn update_expressions_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP update-expression differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut output = Vec::new();

    output.push(format!(
        "identifier-string={}",
        show(
            context
                .eval(
                    "(function(){ var value = '41'; var old = value++; return typeof old + ':' + old + '|' + typeof value + ':' + value; })()",
                )
                .unwrap(),
        )
    ));
    output.push(format!(
        "identifier-bigint={}",
        show(
            context
                .eval(
                    "(function(){ var value = 41n; var post = value--; var prefix = --value; return post * 10000n + prefix * 100n + value; })()",
                )
                .unwrap(),
        )
    ));

    define_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));

    let coercible = context.new_object().unwrap();
    let coercible_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'v(' + hint + ')'; return 7; })",
    );
    define_data(
        &mut context,
        &coercible,
        &to_primitive,
        Value::Object(coercible_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "coercible",
        Value::Object(coercible),
    );
    clear_log(&runtime, &mut context);
    let object_old = context.eval("coercible++").unwrap();
    output.push(format!(
        "identifier-object-post={}|{}|{}",
        show(object_old),
        show(global_value(&runtime, &mut context, "coercible")),
        string_global(&runtime, &mut context, "log"),
    ));

    let bigint_coercible = context.new_object().unwrap();
    let bigint_coercible_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'B(' + hint + ')'; return 7n; })",
    );
    define_data(
        &mut context,
        &bigint_coercible,
        &to_primitive,
        Value::Object(bigint_coercible_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "bigintCoercible",
        Value::Object(bigint_coercible),
    );
    clear_log(&runtime, &mut context);
    let bigint_object_old = context.eval("bigintCoercible++").unwrap();
    output.push(format!(
        "identifier-object-bigint-post={}|{}|{}",
        show(bigint_object_old),
        show(global_value(&runtime, &mut context, "bigintCoercible")),
        string_global(&runtime, &mut context, "log"),
    ));

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("update").unwrap()))
        .unwrap();
    define_global(&runtime, &mut context, "symbolValue", Value::Symbol(symbol));
    output.push(format!(
        "identifier-symbol={}|symbol",
        observe(&runtime, &mut context, "symbolValue++")
    ));

    define_global(&runtime, &mut context, "seenValue", Value::Undefined);
    let computed_old = context.new_object().unwrap();
    let computed_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'v(' + hint + ')'; return 7; })",
    );
    define_data(
        &mut context,
        &computed_old,
        &to_primitive,
        Value::Object(computed_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "computedOld",
        Value::Object(computed_old),
    );

    let target = context.new_object().unwrap();
    let getter = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'g'; return computedOld; })",
    );
    let setter = function(
        &runtime,
        &mut context,
        "(function(value){ log = log + 's'; seenValue = value; })",
    );
    let member = runtime.intern_property_key("member").unwrap();
    define_accessor(
        &mut context,
        &target,
        &member,
        AccessorValue::Callable(getter),
        AccessorValue::Callable(setter),
        true,
    );
    define_global(&runtime, &mut context, "target", Value::Object(target));

    let key = context.new_object().unwrap();
    let key_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'k(' + hint + ')'; return 'member'; })",
    );
    define_data(
        &mut context,
        &key,
        &to_primitive,
        Value::Object(key_converter.as_object().clone()),
        true,
        true,
    );
    define_global(&runtime, &mut context, "key", Value::Object(key));
    let base_expr = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'b'; return target; })",
    );
    define_global(
        &runtime,
        &mut context,
        "baseExpr",
        Value::Object(base_expr.as_object().clone()),
    );
    let key_expr = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'q'; return key; })",
    );
    define_global(
        &runtime,
        &mut context,
        "keyExpr",
        Value::Object(key_expr.as_object().clone()),
    );

    for (label, source) in [
        ("computed-post", "baseExpr()[keyExpr()]++"),
        ("computed-prefix", "++baseExpr()[keyExpr()]"),
    ] {
        clear_log(&runtime, &mut context);
        set_global(&runtime, &mut context, "seenValue", Value::Undefined);
        let value = context.eval(source).unwrap();
        output.push(format!(
            "{label}={}|{}|{}",
            show(value),
            string_global(&runtime, &mut context, "log"),
            show(global_value(&runtime, &mut context, "seenValue")),
        ));
    }

    let readonly_old = context.new_object().unwrap();
    let readonly_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'r(' + hint + ')'; return 4; })",
    );
    define_data(
        &mut context,
        &readonly_old,
        &to_primitive,
        Value::Object(readonly_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "readonlyOld",
        Value::Object(readonly_old.clone()),
    );
    let readonly_target = context.new_object().unwrap();
    let readonly_value = runtime.intern_property_key("value").unwrap();
    define_data(
        &mut context,
        &readonly_target,
        &readonly_value,
        Value::Object(readonly_old),
        false,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "readonlyTarget",
        Value::Object(readonly_target),
    );
    clear_log(&runtime, &mut context);
    let readonly_sloppy = context.eval("readonlyTarget.value++").unwrap();
    output.push(format!(
        "readonly-member-sloppy={}|{}|{}",
        show(readonly_sloppy),
        show(
            context
                .eval("readonlyTarget.value === readonlyOld")
                .unwrap()
        ),
        string_global(&runtime, &mut context, "log"),
    ));
    clear_log(&runtime, &mut context);
    let readonly_strict = observe(
        &runtime,
        &mut context,
        "(function(){ 'use strict'; return ++readonlyTarget.value; })()",
    );
    output.push(format!(
        "readonly-member-strict={readonly_strict}|{}",
        string_global(&runtime, &mut context, "log"),
    ));

    let readonly_identifier = runtime.intern_property_key("readonlyIdentifier").unwrap();
    let global = context.global_object().unwrap();
    define_data(
        &mut context,
        &global,
        &readonly_identifier,
        Value::String(JsString::try_from_utf8("4").unwrap()),
        false,
        true,
    );
    let readonly_identifier_old = context.eval("readonlyIdentifier++").unwrap();
    output.push(format!(
        "readonly-identifier-sloppy={}|{}",
        show(readonly_identifier_old),
        show(global_value(&runtime, &mut context, "readonlyIdentifier")),
    ));
    output.push(format!(
        "readonly-identifier-strict={}",
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return ++readonlyIdentifier; })()",
        )
    ));

    output.push(format!(
        "missing-post={}",
        observe(&runtime, &mut context, "updateMissing++")
    ));
    clear_log(&runtime, &mut context);
    let null_computed = observe(&runtime, &mut context, "null[keyExpr()]++");
    output.push(format!(
        "null-computed={null_computed}|{}",
        string_global(&runtime, &mut context, "log"),
    ));

    output.push(format!(
        "asi-lf={}",
        show(
            context
                .eval(
                    "(function(){ var x = 1, y = 4; var result = x\n++y; return result * 100 + x * 10 + y; })()",
                )
                .unwrap(),
        )
    ));
    output.push(format!(
        "power-updates={}",
        show(
            context
                .eval(
                    "(function(){ var x = 2; return (++x ** 2) * 100 + (x++ ** 2) * 10 + x; })()",
                )
                .unwrap(),
        )
    ));
    output.push(format!(
        "power-unary-error={}",
        observe(
            &runtime,
            &mut context,
            "(function(){ var x = 2; -x++ ** 2; })()"
        )
    ));
    output.push(format!(
        "invalid-update={}",
        observe(
            &runtime,
            &mut context,
            "(function(){ var x = 2; ++(x ** 2); })()"
        )
    ));
    output.push(format!(
        "strict-arguments={}",
        observe(&runtime, &mut context, "'use strict'; arguments++")
    ));

    output
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function probe did not return an object: {source}");
    };
    runtime.as_callable(&object).unwrap().unwrap()
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    define_data(context, &global, &key, value, true, true);
}

fn set_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(context.set_property(&global, &key, value).unwrap());
}

fn global_value(runtime: &Runtime, context: &mut Context, name: &str) -> Value {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(&global, &key).unwrap()
}

fn string_global(runtime: &Runtime, context: &mut Context, name: &str) -> String {
    let Value::String(value) = global_value(runtime, context, name) else {
        panic!("global {name} was not a string");
    };
    value.to_utf8_lossy()
}

fn clear_log(runtime: &Runtime, context: &mut Context) {
    set_global(
        runtime,
        context,
        "log",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
}

fn define_data(
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
    writable: bool,
    configurable: bool,
) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(writable),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_accessor(
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
    get: AccessorValue,
    set: AccessorValue,
    configurable: bool,
) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(get),
                    set: DescriptorField::Present(set),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn observe(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => show(value),
        Err(RuntimeError::Exception) => {
            let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
                panic!("update-expression probe threw a non-object: {source}");
            };
            let name = runtime.intern_property_key("name").unwrap();
            let message = runtime.intern_property_key("message").unwrap();
            let Value::String(name) = context.get_property(&error, &name).unwrap() else {
                panic!("exception name was not a string: {source}");
            };
            let Value::String(message) = context.get_property(&error, &message).unwrap() else {
                panic!("exception message was not a string: {source}");
            };
            format!("throw:{}:{}", name.to_utf8_lossy(), message.to_utf8_lossy())
        }
        Err(error) => panic!("update-expression probe hit engine error for {source:?}: {error}"),
    }
}

fn show(value: Value) -> String {
    match value {
        Value::Undefined => "undefined:undefined".to_owned(),
        Value::Null => "object:null".to_owned(),
        Value::Bool(value) => format!("boolean:{value}"),
        Value::Int(value) => format!("number:{value}"),
        Value::Float(value) => format!("number:{value}"),
        Value::BigInt(value) => format!("bigint:{value}"),
        Value::String(value) => format!("string:{}", value.to_utf8_lossy()),
        Value::Symbol(_) => "symbol:<symbol>".to_owned(),
        Value::Object(_) => "object:<object>".to_owned(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", PROBE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS update-expression oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS update-expression oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS emitted non-UTF-8 stdout: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}
