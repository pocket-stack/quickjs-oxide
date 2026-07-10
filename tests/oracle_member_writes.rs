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

let log = "";
let seenThis = false;
let seenValue;
const proto = {};
const target = Object.create(proto);
Object.defineProperty(proto, "member", {
    set(value) { log += "s"; seenThis = this === target; seenValue = value; },
    configurable: true
});
const key = {
    [Symbol.toPrimitive](hint) { log += "k(" + hint + ")"; return "member"; }
};
Object.defineProperty(proto, "compound", {
    get() { log += "g"; return 2; },
    set(value) { log += "s"; seenThis = this === target; seenValue = value; },
    configurable: true
});
const compoundKey = {
    [Symbol.toPrimitive](hint) { log += "k(" + hint + ")"; return "compound"; }
};
function baseExpr() { log += "b"; return target; }
function keyExpr() { log += "q"; return key; }
function compoundKeyExpr() { log += "q"; return compoundKey; }
function rhsExpr() { log += "r"; return 9; }

log = ""; seenThis = false; seenValue = undefined;
const setResult = baseExpr()[keyExpr()] = rhsExpr();
print("ordered-set=" + [show(setResult), log, show(seenThis), show(seenValue),
      show(Object.hasOwn(target, "member")), show(target.member)].join("|"));

log = "";
const deleteResult = delete baseExpr()[keyExpr()];
print("ordered-delete=" + [show(deleteResult), log,
      show(Object.hasOwn(target, "member")), show(target.member)].join("|"));

log = ""; seenThis = false; seenValue = undefined;
const compoundResult = baseExpr()[compoundKeyExpr()] += rhsExpr();
print("ordered-compound=" + [show(compoundResult), log, show(seenThis), show(seenValue),
      show(Object.hasOwn(target, "compound"))].join("|"));
log = "";
print("null-compound=" + observe(() => null[compoundKeyExpr()] += rhsExpr()) + "|" + log);

const compoundSymbol = Symbol("compound-key");
const otherCompoundSymbol = Symbol("compound-key");
target[compoundSymbol] = 3;
const symbolKeyObject = {
    [Symbol.toPrimitive](hint) { log += "k(" + hint + ")"; return compoundSymbol; }
};
log = "";
const objectSymbolCompound = target[symbolKeyObject] += 4;
print("object-symbol-compound=" + [show(objectSymbolCompound), log,
      show(target[compoundSymbol]), show(target[otherCompoundSymbol])].join("|"));

target.arithmetic = 20;
print("arithmetic=" + [show(target.arithmetic += 2), show(target.arithmetic -= 4),
      show(target.arithmetic *= 3), show(target.arithmetic /= 2),
      show(target.arithmetic %= 5)].join("|"));

log = "";
print("null-set=" + observe(() => null[keyExpr()] = rhsExpr()) + "|" + log);
log = "";
print("null-delete=" + observe(() => delete null[keyExpr()]) + "|" + log);

const originalPrototype = Function.prototype;
const sloppyReadonly = Function.prototype = 1;
print("readonly-sloppy=" + [show(sloppyReadonly),
      show(Function.prototype === originalPrototype), show(delete Function.prototype)].join("|"));
print("readonly-strict-set=" + observe(() => (function(){ "use strict"; return Function.prototype = 1; })()));
print("readonly-strict-delete=" + observe(() => (function(){ "use strict"; return delete Function.prototype; })()));
print("native-setter=" + observe(() => Function.prototype.caller = 1));

Object.defineProperty(target, "getterOnly", {
    get() { return 1; }, configurable: true
});
const getterOnlySloppy = target.getterOnly = 2;
print("getter-only=" + [show(getterOnlySloppy), show(target.getterOnly),
      observe(() => (function(){ "use strict"; return target.getterOnly = 3; })()),
      show(target.getterOnly)].join("|"));

target.named = function() {};
print("property-name=" + show(target.named.name));

const symbol = Symbol("same");
const otherSymbol = Symbol("same");
target[symbol] = 1;
target[otherSymbol] = 2;
const symbolResult = target[symbol] = 5;
const symbolCompound = target[symbol] += 2;
print("symbol=" + [show(symbolResult), show(symbolCompound),
      show(target[symbol]), show(target[otherSymbol])].join("|"));

let getterCalls = 0;
Object.defineProperty(target, "deletableGetter", {
    get() { getterCalls++; return 1; }, configurable: true
});
print("delete-getter=" + [show(delete target.deletableGetter), show(getterCalls),
      show(Object.hasOwn(target, "deletableGetter"))].join("|"));

const sealed = {};
Object.preventExtensions(sealed);
print("nonextensible=" + [show(sealed.x = 4), show(Object.hasOwn(sealed, "x")),
      observe(() => (function(){ "use strict"; return sealed.x = 5; })())].join("|"));

print("primitive-set=" + [show((1).x = 4),
      observe(() => (function(){ "use strict"; return (1).x = 5; })()),
      show("abc".length = 4),
      observe(() => (function(){ "use strict"; return "abc".length = 5; })())].join("|"));
print("primitive-delete=" + [show(delete "abc"[0]), show(delete "abc"[9]),
      observe(() => (function(){ "use strict"; return delete "abc"[0]; })()),
      show(delete (1).x)].join("|"));
"#;

#[test]
fn source_member_assignment_and_delete_match_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP member-write differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    define_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    define_global(&runtime, &mut context, "seenThis", Value::Bool(false));
    define_global(&runtime, &mut context, "seenValue", Value::Undefined);

    let proto = context.new_object().unwrap();
    let target = context.new_object_with_prototype(Some(&proto)).unwrap();
    define_global(
        &runtime,
        &mut context,
        "target",
        Value::Object(target.clone()),
    );
    let setter = function(
        &runtime,
        &mut context,
        "(function(value){ log = log + 's'; seenThis = this === target; seenValue = value; })",
    );
    let member = runtime.intern_property_key("member").unwrap();
    define_accessor(
        &mut context,
        &proto,
        &member,
        AccessorValue::Undefined,
        AccessorValue::Callable(setter),
        true,
    );

    let compound_getter = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'g'; return 2; })",
    );
    let compound_setter = function(
        &runtime,
        &mut context,
        "(function(value){ log = log + 's'; seenThis = this === target; seenValue = value; })",
    );
    let compound = runtime.intern_property_key("compound").unwrap();
    define_accessor(
        &mut context,
        &proto,
        &compound,
        AccessorValue::Callable(compound_getter),
        AccessorValue::Callable(compound_setter),
        true,
    );

    let key = context.new_object().unwrap();
    let converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'k(' + hint + ')'; return 'member'; })",
    );
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    define_data(
        &mut context,
        &key,
        &to_primitive,
        Value::Object(converter.as_object().clone()),
        true,
        true,
    );
    define_global(&runtime, &mut context, "key", Value::Object(key));
    let compound_key = context.new_object().unwrap();
    let compound_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'k(' + hint + ')'; return 'compound'; })",
    );
    define_data(
        &mut context,
        &compound_key,
        &to_primitive,
        Value::Object(compound_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "compoundKey",
        Value::Object(compound_key),
    );
    let compound_symbol = runtime
        .new_symbol(Some(JsString::from("compound-key")))
        .unwrap();
    let other_compound_symbol = runtime
        .new_symbol(Some(JsString::from("compound-key")))
        .unwrap();
    define_global(
        &runtime,
        &mut context,
        "compoundSymbol",
        Value::Symbol(compound_symbol),
    );
    define_global(
        &runtime,
        &mut context,
        "otherCompoundSymbol",
        Value::Symbol(other_compound_symbol),
    );
    let symbol_key_object = context.new_object().unwrap();
    let symbol_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ log = log + 'k(' + hint + ')'; return compoundSymbol; })",
    );
    define_data(
        &mut context,
        &symbol_key_object,
        &to_primitive,
        Value::Object(symbol_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "symbolKeyObject",
        Value::Object(symbol_key_object),
    );
    for (name, source) in [
        (
            "baseExpr",
            "(function(){ log = log + 'b'; return target; })",
        ),
        ("keyExpr", "(function(){ log = log + 'q'; return key; })"),
        (
            "compoundKeyExpr",
            "(function(){ log = log + 'q'; return compoundKey; })",
        ),
        ("rhsExpr", "(function(){ log = log + 'r'; return 9; })"),
    ] {
        let value = function(&runtime, &mut context, source);
        define_global(
            &runtime,
            &mut context,
            name,
            Value::Object(value.as_object().clone()),
        );
    }

    let mut output = Vec::new();
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    set_global(&runtime, &mut context, "seenThis", Value::Bool(false));
    set_global(&runtime, &mut context, "seenValue", Value::Undefined);
    let set_result = context.eval("baseExpr()[keyExpr()] = rhsExpr()").unwrap();
    output.push(format!(
        "ordered-set={}|{}|{}|{}|{}|{}",
        show(set_result),
        string_global(&runtime, &mut context, "log"),
        show(global_value(&runtime, &mut context, "seenThis")),
        show(global_value(&runtime, &mut context, "seenValue")),
        show(Value::Bool(
            runtime.has_own_property(&target, &member).unwrap()
        )),
        show(context.get_property(&target, &member).unwrap()),
    ));

    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let delete_result = context.eval("delete baseExpr()[keyExpr()]").unwrap();
    output.push(format!(
        "ordered-delete={}|{}|{}|{}",
        show(delete_result),
        string_global(&runtime, &mut context, "log"),
        show(Value::Bool(
            runtime.has_own_property(&target, &member).unwrap()
        )),
        show(context.get_property(&target, &member).unwrap()),
    ));

    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    set_global(&runtime, &mut context, "seenThis", Value::Bool(false));
    set_global(&runtime, &mut context, "seenValue", Value::Undefined);
    let compound_result = context
        .eval("baseExpr()[compoundKeyExpr()] += rhsExpr()")
        .unwrap();
    output.push(format!(
        "ordered-compound={}|{}|{}|{}|{}",
        show(compound_result),
        string_global(&runtime, &mut context, "log"),
        show(global_value(&runtime, &mut context, "seenThis")),
        show(global_value(&runtime, &mut context, "seenValue")),
        show(Value::Bool(
            runtime.has_own_property(&target, &compound).unwrap()
        )),
    ));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let null_compound = observe(
        &runtime,
        &mut context,
        "null[compoundKeyExpr()] += rhsExpr()",
    );
    output.push(format!(
        "null-compound={null_compound}|{}",
        string_global(&runtime, &mut context, "log")
    ));

    context.eval("target[compoundSymbol] = 3").unwrap();
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let object_symbol_compound = context.eval("target[symbolKeyObject] += 4").unwrap();
    runtime.run_gc().unwrap();
    output.push(format!(
        "object-symbol-compound={}|{}|{}|{}",
        show(object_symbol_compound),
        string_global(&runtime, &mut context, "log"),
        show(context.eval("target[compoundSymbol]").unwrap()),
        show(context.eval("target[otherCompoundSymbol]").unwrap()),
    ));

    let arithmetic = runtime.intern_property_key("arithmetic").unwrap();
    define_data(
        &mut context,
        &target,
        &arithmetic,
        Value::Int(20),
        true,
        true,
    );
    let mut arithmetic_values = Vec::new();
    for source in [
        "target.arithmetic += 2",
        "target.arithmetic -= 4",
        "target.arithmetic *= 3",
        "target.arithmetic /= 2",
        "target.arithmetic %= 5",
    ] {
        arithmetic_values.push(show(context.eval(source).unwrap()));
    }
    output.push(format!("arithmetic={}", arithmetic_values.join("|")));

    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let null_set = observe(&runtime, &mut context, "null[keyExpr()] = rhsExpr()");
    output.push(format!(
        "null-set={null_set}|{}",
        string_global(&runtime, &mut context, "log")
    ));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let null_delete = observe(&runtime, &mut context, "delete null[keyExpr()]");
    output.push(format!(
        "null-delete={null_delete}|{}",
        string_global(&runtime, &mut context, "log")
    ));

    let original = context.eval("Function.prototype").unwrap();
    let sloppy = context.eval("Function.prototype = 1").unwrap();
    let unchanged = context.eval("Function.prototype").unwrap() == original;
    let deleted = context.eval("delete Function.prototype").unwrap();
    output.push(format!(
        "readonly-sloppy={}|{}|{}",
        show(sloppy),
        show(Value::Bool(unchanged)),
        show(deleted)
    ));
    output.push(format!(
        "readonly-strict-set={}",
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return Function.prototype = 1; })()"
        )
    ));
    output.push(format!(
        "readonly-strict-delete={}",
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return delete Function.prototype; })()"
        )
    ));
    output.push(format!(
        "native-setter={}",
        observe(&runtime, &mut context, "Function.prototype.caller = 1")
    ));

    let getter = function(&runtime, &mut context, "(function(){ return 1; })");
    let getter_only = runtime.intern_property_key("getterOnly").unwrap();
    define_accessor(
        &mut context,
        &target,
        &getter_only,
        AccessorValue::Callable(getter),
        AccessorValue::Undefined,
        true,
    );
    let getter_sloppy = context.eval("target.getterOnly = 2").unwrap();
    let getter_before = context.eval("target.getterOnly").unwrap();
    let getter_strict = observe(
        &runtime,
        &mut context,
        "(function(){ 'use strict'; return target.getterOnly = 3; })()",
    );
    let getter_after = context.eval("target.getterOnly").unwrap();
    output.push(format!(
        "getter-only={}|{}|{}|{}",
        show(getter_sloppy),
        show(getter_before),
        getter_strict,
        show(getter_after)
    ));

    output.push(format!(
        "property-name={}",
        show(
            context
                .eval("target.named = function(){}, target.named.name")
                .unwrap()
        )
    ));

    let symbol = runtime.new_symbol(Some(JsString::from("same"))).unwrap();
    let other_symbol = runtime.new_symbol(Some(JsString::from("same"))).unwrap();
    define_global(
        &runtime,
        &mut context,
        "symbol",
        Value::Symbol(symbol.clone()),
    );
    define_global(
        &runtime,
        &mut context,
        "otherSymbol",
        Value::Symbol(other_symbol),
    );
    context.eval("target[symbol] = 1").unwrap();
    context.eval("target[otherSymbol] = 2").unwrap();
    let symbol_result = context.eval("target[symbol] = 5").unwrap();
    let symbol_compound = context.eval("target[symbol] += 2").unwrap();
    output.push(format!(
        "symbol={}|{}|{}|{}",
        show(symbol_result),
        show(symbol_compound),
        show(context.eval("target[symbol]").unwrap()),
        show(context.eval("target[otherSymbol]").unwrap())
    ));

    define_global(&runtime, &mut context, "getterCalls", Value::Int(0));
    let delete_getter = function(
        &runtime,
        &mut context,
        "(function(){ getterCalls = getterCalls + 1; return 1; })",
    );
    let deletable = runtime.intern_property_key("deletableGetter").unwrap();
    define_accessor(
        &mut context,
        &target,
        &deletable,
        AccessorValue::Callable(delete_getter),
        AccessorValue::Undefined,
        true,
    );
    let delete_getter_result = context.eval("delete target.deletableGetter").unwrap();
    output.push(format!(
        "delete-getter={}|{}|{}",
        show(delete_getter_result),
        show(global_value(&runtime, &mut context, "getterCalls")),
        show(Value::Bool(
            runtime.has_own_property(&target, &deletable).unwrap()
        ))
    ));

    let sealed = context.new_object().unwrap();
    runtime.prevent_extensions(&sealed).unwrap();
    define_global(
        &runtime,
        &mut context,
        "sealed",
        Value::Object(sealed.clone()),
    );
    let sealed_sloppy = context.eval("sealed.x = 4").unwrap();
    let x = runtime.intern_property_key("x").unwrap();
    let sealed_strict = observe(
        &runtime,
        &mut context,
        "(function(){ 'use strict'; return sealed.x = 5; })()",
    );
    output.push(format!(
        "nonextensible={}|{}|{}",
        show(sealed_sloppy),
        show(Value::Bool(runtime.has_own_property(&sealed, &x).unwrap())),
        sealed_strict
    ));

    output.push(format!(
        "primitive-set={}|{}|{}|{}",
        show(context.eval("(1).x = 4").unwrap()),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return (1).x = 5; })()"
        ),
        show(context.eval("'abc'.length = 4").unwrap()),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return 'abc'.length = 5; })()"
        )
    ));
    output.push(format!(
        "primitive-delete={}|{}|{}|{}",
        show(context.eval("delete 'abc'[0]").unwrap()),
        show(context.eval("delete 'abc'[9]").unwrap()),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return delete 'abc'[0]; })()"
        ),
        show(context.eval("delete (1).x").unwrap())
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
                panic!("member-write probe threw a non-object: {source}");
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
        Err(error) => panic!("member-write probe hit engine error for {source:?}: {error}"),
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
        .unwrap_or_else(|error| panic!("could not run QuickJS member-write oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS member-write oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS emitted non-UTF-8 stdout: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}
