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
    get() { log += "g"; return oldValue; },
    set(value) { log += "s"; seenThis = this === target; seenValue = value; },
    configurable: true
});
let oldValue = 2;
let identifierOld = 2;
let identifierGetThis = false;
let identifierSetThis = false;
Object.defineProperty(globalThis, "identifierAccessor", {
    get() { log += "G"; identifierGetThis = this === globalThis; return identifierOld; },
    set(value) { log += "S"; identifierSetThis = this === globalThis; seenValue = value; },
    configurable: true
});
Object.defineProperty(globalThis, "identifierReadonly", {
    value: 2, writable: false, configurable: true
});
Object.defineProperty(globalThis, "identifierThrowing", {
    get() { log += "G"; throw new Error("identifier getter"); },
    configurable: true
});
const compoundKey = {
    [Symbol.toPrimitive](hint) { log += "k(" + hint + ")"; return "compound"; }
};
function baseExpr() { log += "b"; return target; }
function keyExpr() { log += "q"; return key; }
function compoundKeyExpr() { log += "q"; return compoundKey; }
function rhsExpr() { log += "r"; return 9; }
function throwingRhs() { log += "r"; throw new Error("rhs"); }
const throwingKey = {};
Object.defineProperty(throwingKey, Symbol.toPrimitive, {
    value: function() { log += "k!"; throw new Error("key"); }
});
function throwingKeyExpr() { log += "q"; return throwingKey; }

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
oldValue = 14; log = ""; seenThis = false; seenValue = undefined;
const bitwiseCompoundResult = baseExpr()[compoundKeyExpr()] &= rhsExpr();
print("ordered-bitwise-compound=" + [show(bitwiseCompoundResult), log,
      show(seenThis), show(seenValue), show(Object.hasOwn(target, "compound"))].join("|"));
log = "";
print("null-compound=" + observe(() => null[compoundKeyExpr()] += rhsExpr()) + "|" + log);
log = "";
print("null-bitwise-compound=" + observe(() => null[compoundKeyExpr()] &= rhsExpr()) + "|" + log);

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
const binaryNullishSymbol = compoundSymbol ?? otherCompoundSymbol;
print("binary-nullish-symbol=" + [show(binaryNullishSymbol === compoundSymbol),
      show(binaryNullishSymbol === otherCompoundSymbol)].join("|"));

target[compoundSymbol] = 0;
log = "";
const objectSymbolLogical = target[symbolKeyObject] ||= 8;
print("object-symbol-logical=" + [show(objectSymbolLogical), log,
      show(target[compoundSymbol]), show(target[otherCompoundSymbol])].join("|"));
log = "";
const objectSymbolLogicalShort = target[symbolKeyObject] ||= 10;
print("object-symbol-logical-short=" + [show(objectSymbolLogicalShort), log,
      show(target[compoundSymbol]), show(target[otherCompoundSymbol])].join("|"));

target.arithmetic = 20;
print("arithmetic=" + [show(target.arithmetic += 2), show(target.arithmetic -= 4),
      show(target.arithmetic *= 3), show(target.arithmetic /= 2),
      show(target.arithmetic %= 5)].join("|"));
target.bitwise = 14;
print("bitwise=" + [show(target.bitwise &= 11), show(target.bitwise ^= 3),
      show(target.bitwise |= 4)].join("|"));

oldValue = 2; log = ""; seenThis = false; seenValue = undefined;
const logicalAndSet = baseExpr()[compoundKeyExpr()] &&= rhsExpr();
print("logical-and-set=" + [show(logicalAndSet), log, show(seenThis), show(seenValue)].join("|"));
oldValue = 0; log = ""; seenThis = false; seenValue = undefined;
const logicalAndSkip = baseExpr()[compoundKeyExpr()] &&= rhsExpr();
print("logical-and-skip=" + [show(logicalAndSkip), log, show(seenThis), show(seenValue)].join("|"));
oldValue = 0; log = ""; seenThis = false; seenValue = undefined;
const logicalOrSet = baseExpr()[compoundKeyExpr()] ||= rhsExpr();
print("logical-or-set=" + [show(logicalOrSet), log, show(seenThis), show(seenValue)].join("|"));
oldValue = 2; log = ""; seenThis = false; seenValue = undefined;
const logicalOrSkip = baseExpr()[compoundKeyExpr()] ||= rhsExpr();
print("logical-or-skip=" + [show(logicalOrSkip), log, show(seenThis), show(seenValue)].join("|"));
oldValue = null; log = ""; seenThis = false; seenValue = undefined;
const logicalNullishSet = baseExpr()[compoundKeyExpr()] ??= rhsExpr();
print("logical-nullish-set=" + [show(logicalNullishSet), log, show(seenThis), show(seenValue)].join("|"));
oldValue = false; log = ""; seenThis = false; seenValue = undefined;
const logicalNullishSkip = baseExpr()[compoundKeyExpr()] ??= rhsExpr();
print("logical-nullish-skip=" + [show(logicalNullishSkip), log, show(seenThis), show(seenValue)].join("|"));

const coercionBomb = {};
Object.defineProperty(coercionBomb, Symbol.toPrimitive, {
    value: function() { log += "p"; throw new Error("coerced"); }
});
log = "";
const binaryNullishObject = coercionBomb ?? null;
print("binary-nullish-object=" + [show(binaryNullishObject === coercionBomb), log].join("|"));
oldValue = coercionBomb; log = "";
const logicalObjectAnd = baseExpr()[compoundKeyExpr()] &&= rhsExpr();
print("logical-object-and=" + [show(logicalObjectAnd), log].join("|"));
oldValue = coercionBomb; log = "";
const logicalObjectOr = baseExpr()[compoundKeyExpr()] ||= rhsExpr();
print("logical-object-or=" + [show(logicalObjectOr === coercionBomb), log].join("|"));
oldValue = coercionBomb; log = "";
const logicalObjectNullish = baseExpr()[compoundKeyExpr()] ??= rhsExpr();
print("logical-object-nullish=" + [show(logicalObjectNullish === coercionBomb), log].join("|"));
oldValue = 1; log = "";
print("logical-rhs-throw=" + observe(() =>
      baseExpr()[compoundKeyExpr()] &&= throwingRhs()) + "|" + log);
log = "";
print("logical-key-throw=" + observe(() =>
      baseExpr()[throwingKeyExpr()] ||= rhsExpr()) + "|" + log);
log = "";
print("logical-getter-throw=" + observe(() =>
      Function.prototype.caller &&= rhsExpr()) + "|" + log);

identifierOld = 2; log = ""; identifierGetThis = false;
identifierSetThis = false; seenValue = undefined;
const identifierArithmetic = identifierAccessor += rhsExpr();
print("identifier-arithmetic=" + [show(identifierArithmetic), log,
      show(identifierGetThis), show(identifierSetThis), show(seenValue)].join("|"));
identifierOld = 14; log = ""; identifierGetThis = false;
identifierSetThis = false; seenValue = undefined;
const identifierBitwise = identifierAccessor &= rhsExpr();
print("identifier-bitwise=" + [show(identifierBitwise), log,
      show(identifierGetThis), show(identifierSetThis), show(seenValue)].join("|"));
identifierOld = 0; log = ""; identifierGetThis = false;
identifierSetThis = false; seenValue = undefined;
const identifierLogicalSet = identifierAccessor ||= rhsExpr();
print("identifier-logical-set=" + [show(identifierLogicalSet), log,
      show(identifierGetThis), show(identifierSetThis), show(seenValue)].join("|"));
identifierOld = 2; log = ""; identifierGetThis = false;
identifierSetThis = false; seenValue = undefined;
const identifierLogicalShort = identifierAccessor ||= rhsExpr();
print("identifier-logical-short=" + [show(identifierLogicalShort), log,
      show(identifierGetThis), show(identifierSetThis), show(seenValue)].join("|"));
identifierOld = undefined; log = ""; seenValue = undefined;
const identifierNamed = identifierAccessor ??= function(){};
print("identifier-name=" + [show(identifierNamed.name),
      show(identifierNamed === seenValue), log].join("|"));
identifierOld = undefined; log = ""; seenValue = undefined;
const identifierParenNamed = (identifierAccessor) ??= function(){};
print("identifier-paren-name=" + [show(identifierParenNamed.name),
      show(identifierParenNamed === seenValue), log].join("|"));
print("identifier-readonly=" + [show(identifierReadonly += 3), show(identifierReadonly),
      observe(() => (function(){ "use strict"; return identifierReadonly += 3; })()),
      observe(() => (function(){ "use strict"; return identifierReadonly ||= 9; })()),
      observe(() => (function(){ "use strict"; return identifierReadonly &&= 9; })())].join("|"));
print("identifier-bitwise-readonly=" + [show(identifierReadonly &= 3),
      show(identifierReadonly),
      observe(() => (function(){ "use strict"; return identifierReadonly |= 1; })())].join("|"));
log = "";
print("identifier-missing=" + observe(() => identifierMissing += rhsExpr()) + "|" + log);
log = "";
print("identifier-bitwise-missing=" + observe(() => identifierMissingBits &= rhsExpr()) + "|" + log);
log = "";
print("identifier-getter-throw=" + observe(() => identifierThrowing ||= rhsExpr()) + "|" + log);

log = "";
print("null-logical=" + observe(() => null[compoundKeyExpr()] ||= rhsExpr()) + "|" + log);

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
log = "";
print("readonly-logical-skip=" + observe(() =>
      (function(){ "use strict"; return Function.prototype ||= rhsExpr(); })() === Function.prototype) + "|" + log);
log = "";
print("readonly-logical-set=" + observe(() =>
      (function(){ "use strict"; return Function.prototype &&= rhsExpr(); })()) + "|" + log);
log = "";
print("readonly-bitwise-set=" + observe(() =>
      (function(){ "use strict"; return Function.prototype &= rhsExpr(); })()) + "|" + log);
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
    define_global(&runtime, &mut context, "oldValue", Value::Int(2));
    define_global(&runtime, &mut context, "identifierOld", Value::Int(2));
    define_global(
        &runtime,
        &mut context,
        "identifierGetThis",
        Value::Bool(false),
    );
    define_global(
        &runtime,
        &mut context,
        "identifierSetThis",
        Value::Bool(false),
    );
    let global = context.global_object().unwrap();
    define_global(
        &runtime,
        &mut context,
        "identifierGlobal",
        Value::Object(global.clone()),
    );
    let identifier_getter = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'G'; identifierGetThis = this === identifierGlobal; return identifierOld; })",
    );
    let identifier_setter = function(
        &runtime,
        &mut context,
        "(function(value){ log = log + 'S'; identifierSetThis = this === identifierGlobal; seenValue = value; })",
    );
    let identifier_accessor = runtime.intern_property_key("identifierAccessor").unwrap();
    define_accessor(
        &mut context,
        &global,
        &identifier_accessor,
        AccessorValue::Callable(identifier_getter),
        AccessorValue::Callable(identifier_setter),
        true,
    );
    let identifier_readonly = runtime.intern_property_key("identifierReadonly").unwrap();
    define_data(
        &mut context,
        &global,
        &identifier_readonly,
        Value::Int(2),
        false,
        true,
    );
    let identifier_throwing_getter = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'G'; throw new Error('identifier getter'); })",
    );
    let identifier_throwing = runtime.intern_property_key("identifierThrowing").unwrap();
    define_accessor(
        &mut context,
        &global,
        &identifier_throwing,
        AccessorValue::Callable(identifier_throwing_getter),
        AccessorValue::Undefined,
        true,
    );

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
        "(function(){ log = log + 'g'; return oldValue; })",
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
    let throwing_key = context.new_object().unwrap();
    let throwing_key_converter = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'k!'; throw new Error('key'); })",
    );
    define_data(
        &mut context,
        &throwing_key,
        &to_primitive,
        Value::Object(throwing_key_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "throwingKey",
        Value::Object(throwing_key),
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
        (
            "throwingRhs",
            "(function(){ log = log + 'r'; throw new Error('rhs'); })",
        ),
        (
            "throwingKeyExpr",
            "(function(){ log = log + 'q'; return throwingKey; })",
        ),
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
    set_global(&runtime, &mut context, "oldValue", Value::Int(14));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    set_global(&runtime, &mut context, "seenThis", Value::Bool(false));
    set_global(&runtime, &mut context, "seenValue", Value::Undefined);
    let bitwise_compound_result = context
        .eval("baseExpr()[compoundKeyExpr()] &= rhsExpr()")
        .unwrap();
    output.push(format!(
        "ordered-bitwise-compound={}|{}|{}|{}|{}",
        show(bitwise_compound_result),
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
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let null_bitwise_compound = observe(
        &runtime,
        &mut context,
        "null[compoundKeyExpr()] &= rhsExpr()",
    );
    output.push(format!(
        "null-bitwise-compound={null_bitwise_compound}|{}",
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
    let binary_nullish_symbol = context
        .eval("compoundSymbol ?? otherCompoundSymbol")
        .unwrap();
    runtime.run_gc().unwrap();
    output.push(format!(
        "binary-nullish-symbol={}|{}",
        show(Value::Bool(
            binary_nullish_symbol == global_value(&runtime, &mut context, "compoundSymbol")
        )),
        show(Value::Bool(
            binary_nullish_symbol == global_value(&runtime, &mut context, "otherCompoundSymbol")
        )),
    ));

    context.eval("target[compoundSymbol] = 0").unwrap();
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let object_symbol_logical = context.eval("target[symbolKeyObject] ||= 8").unwrap();
    runtime.run_gc().unwrap();
    output.push(format!(
        "object-symbol-logical={}|{}|{}|{}",
        show(object_symbol_logical),
        string_global(&runtime, &mut context, "log"),
        show(context.eval("target[compoundSymbol]").unwrap()),
        show(context.eval("target[otherCompoundSymbol]").unwrap()),
    ));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let object_symbol_logical_short = context.eval("target[symbolKeyObject] ||= 10").unwrap();
    runtime.run_gc().unwrap();
    output.push(format!(
        "object-symbol-logical-short={}|{}|{}|{}",
        show(object_symbol_logical_short),
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

    let bitwise = runtime.intern_property_key("bitwise").unwrap();
    define_data(&mut context, &target, &bitwise, Value::Int(14), true, true);
    let bitwise_values = [
        "target.bitwise &= 11",
        "target.bitwise ^= 3",
        "target.bitwise |= 4",
    ]
    .into_iter()
    .map(|source| show(context.eval(source).unwrap()))
    .collect::<Vec<_>>();
    output.push(format!("bitwise={}", bitwise_values.join("|")));

    for (label, old_value, source) in [
        (
            "logical-and-set",
            Value::Int(2),
            "baseExpr()[compoundKeyExpr()] &&= rhsExpr()",
        ),
        (
            "logical-and-skip",
            Value::Int(0),
            "baseExpr()[compoundKeyExpr()] &&= rhsExpr()",
        ),
        (
            "logical-or-set",
            Value::Int(0),
            "baseExpr()[compoundKeyExpr()] ||= rhsExpr()",
        ),
        (
            "logical-or-skip",
            Value::Int(2),
            "baseExpr()[compoundKeyExpr()] ||= rhsExpr()",
        ),
        (
            "logical-nullish-set",
            Value::Null,
            "baseExpr()[compoundKeyExpr()] ??= rhsExpr()",
        ),
        (
            "logical-nullish-skip",
            Value::Bool(false),
            "baseExpr()[compoundKeyExpr()] ??= rhsExpr()",
        ),
    ] {
        set_global(&runtime, &mut context, "oldValue", old_value);
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        set_global(&runtime, &mut context, "seenThis", Value::Bool(false));
        set_global(&runtime, &mut context, "seenValue", Value::Undefined);
        let result = context.eval(source).unwrap();
        output.push(format!(
            "{label}={}|{}|{}|{}",
            show(result),
            string_global(&runtime, &mut context, "log"),
            show(global_value(&runtime, &mut context, "seenThis")),
            show(global_value(&runtime, &mut context, "seenValue")),
        ));
    }

    let coercion_bomb = context.new_object().unwrap();
    let coercion_bomb_converter = function(
        &runtime,
        &mut context,
        "(function(){ log = log + 'p'; throw new Error('coerced'); })",
    );
    define_data(
        &mut context,
        &coercion_bomb,
        &to_primitive,
        Value::Object(coercion_bomb_converter.as_object().clone()),
        true,
        true,
    );
    define_global(
        &runtime,
        &mut context,
        "coercionBomb",
        Value::Object(coercion_bomb.clone()),
    );
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let binary_nullish_object = context.eval("coercionBomb ?? null").unwrap();
    runtime.run_gc().unwrap();
    output.push(format!(
        "binary-nullish-object={}|{}",
        show(Value::Bool(
            binary_nullish_object == Value::Object(coercion_bomb.clone())
        )),
        string_global(&runtime, &mut context, "log")
    ));
    set_global(
        &runtime,
        &mut context,
        "oldValue",
        Value::Object(coercion_bomb.clone()),
    );
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let logical_object_and = context
        .eval("baseExpr()[compoundKeyExpr()] &&= rhsExpr()")
        .unwrap();
    output.push(format!(
        "logical-object-and={}|{}",
        show(logical_object_and),
        string_global(&runtime, &mut context, "log")
    ));

    for (label, source) in [
        (
            "logical-object-or",
            "baseExpr()[compoundKeyExpr()] ||= rhsExpr()",
        ),
        (
            "logical-object-nullish",
            "baseExpr()[compoundKeyExpr()] ??= rhsExpr()",
        ),
    ] {
        set_global(
            &runtime,
            &mut context,
            "oldValue",
            Value::Object(coercion_bomb.clone()),
        );
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        let result = context.eval(source).unwrap();
        runtime.run_gc().unwrap();
        output.push(format!(
            "{label}={}|{}",
            show(Value::Bool(result == Value::Object(coercion_bomb.clone()))),
            string_global(&runtime, &mut context, "log")
        ));
    }

    set_global(&runtime, &mut context, "oldValue", Value::Int(1));
    for (label, source) in [
        (
            "logical-rhs-throw",
            "baseExpr()[compoundKeyExpr()] &&= throwingRhs()",
        ),
        (
            "logical-key-throw",
            "baseExpr()[throwingKeyExpr()] ||= rhsExpr()",
        ),
        (
            "logical-getter-throw",
            "Function.prototype.caller &&= rhsExpr()",
        ),
    ] {
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        let observation = observe(&runtime, &mut context, source);
        output.push(format!(
            "{label}={observation}|{}",
            string_global(&runtime, &mut context, "log")
        ));
    }

    for (label, old_value, source) in [
        (
            "identifier-arithmetic",
            Value::Int(2),
            "identifierAccessor += rhsExpr()",
        ),
        (
            "identifier-bitwise",
            Value::Int(14),
            "identifierAccessor &= rhsExpr()",
        ),
        (
            "identifier-logical-set",
            Value::Int(0),
            "identifierAccessor ||= rhsExpr()",
        ),
        (
            "identifier-logical-short",
            Value::Int(2),
            "identifierAccessor ||= rhsExpr()",
        ),
    ] {
        set_global(&runtime, &mut context, "identifierOld", old_value);
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        set_global(
            &runtime,
            &mut context,
            "identifierGetThis",
            Value::Bool(false),
        );
        set_global(
            &runtime,
            &mut context,
            "identifierSetThis",
            Value::Bool(false),
        );
        set_global(&runtime, &mut context, "seenValue", Value::Undefined);
        let result = context.eval(source).unwrap();
        output.push(format!(
            "{label}={}|{}|{}|{}|{}",
            show(result),
            string_global(&runtime, &mut context, "log"),
            show(global_value(&runtime, &mut context, "identifierGetThis")),
            show(global_value(&runtime, &mut context, "identifierSetThis")),
            show(global_value(&runtime, &mut context, "seenValue")),
        ));
    }

    let name = runtime.intern_property_key("name").unwrap();
    for (label, source) in [
        ("identifier-name", "identifierAccessor ??= function(){}"),
        (
            "identifier-paren-name",
            "(identifierAccessor) ??= function(){}",
        ),
    ] {
        set_global(&runtime, &mut context, "identifierOld", Value::Undefined);
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        set_global(&runtime, &mut context, "seenValue", Value::Undefined);
        let Value::Object(result) = context.eval(source).unwrap() else {
            panic!("identifier name probe did not produce a function");
        };
        let result_value = Value::Object(result.clone());
        output.push(format!(
            "{label}={}|{}|{}",
            show(context.get_property(&result, &name).unwrap()),
            show(Value::Bool(
                result_value == global_value(&runtime, &mut context, "seenValue")
            )),
            string_global(&runtime, &mut context, "log"),
        ));
    }

    output.push(format!(
        "identifier-readonly={}|{}|{}|{}|{}",
        show(context.eval("identifierReadonly += 3").unwrap()),
        show(context.eval("identifierReadonly").unwrap()),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return identifierReadonly += 3; })()"
        ),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return identifierReadonly ||= 9; })()"
        ),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return identifierReadonly &&= 9; })()"
        ),
    ));
    output.push(format!(
        "identifier-bitwise-readonly={}|{}|{}",
        show(context.eval("identifierReadonly &= 3").unwrap()),
        show(context.eval("identifierReadonly").unwrap()),
        observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return identifierReadonly |= 1; })()"
        ),
    ));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let identifier_missing = observe(&runtime, &mut context, "identifierMissing += rhsExpr()");
    output.push(format!(
        "identifier-missing={identifier_missing}|{}",
        string_global(&runtime, &mut context, "log")
    ));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let identifier_bitwise_missing =
        observe(&runtime, &mut context, "identifierMissingBits &= rhsExpr()");
    output.push(format!(
        "identifier-bitwise-missing={identifier_bitwise_missing}|{}",
        string_global(&runtime, &mut context, "log")
    ));
    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let identifier_getter_throw =
        observe(&runtime, &mut context, "identifierThrowing ||= rhsExpr()");
    output.push(format!(
        "identifier-getter-throw={identifier_getter_throw}|{}",
        string_global(&runtime, &mut context, "log")
    ));

    set_global(
        &runtime,
        &mut context,
        "log",
        Value::String(JsString::from("")),
    );
    let null_logical = observe(
        &runtime,
        &mut context,
        "null[compoundKeyExpr()] ||= rhsExpr()",
    );
    output.push(format!(
        "null-logical={null_logical}|{}",
        string_global(&runtime, &mut context, "log")
    ));

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
        "readonly-logical-skip={}",
        {
            set_global(
                &runtime,
                &mut context,
                "log",
                Value::String(JsString::from("")),
            );
            let observation = observe(
                &runtime,
                &mut context,
                "(function(){ 'use strict'; return Function.prototype ||= rhsExpr(); })() === Function.prototype",
            );
            format!(
                "{observation}|{}",
                string_global(&runtime, &mut context, "log")
            )
        }
    ));
    output.push(format!("readonly-logical-set={}", {
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        let observation = observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return Function.prototype &&= rhsExpr(); })()",
        );
        format!(
            "{observation}|{}",
            string_global(&runtime, &mut context, "log")
        )
    }));
    output.push(format!("readonly-bitwise-set={}", {
        set_global(
            &runtime,
            &mut context,
            "log",
            Value::String(JsString::from("")),
        );
        let observation = observe(
            &runtime,
            &mut context,
            "(function(){ 'use strict'; return Function.prototype &= rhsExpr(); })()",
        );
        format!(
            "{observation}|{}",
            string_global(&runtime, &mut context, "log")
        )
    }));
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
