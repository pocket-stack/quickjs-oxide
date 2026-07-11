use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function show(value) {
    if (value === undefined) return "undefined:undefined";
    if (value === null) return "object:null";
    if (typeof value === "symbol") return "symbol:<symbol>";
    if (typeof value === "object" || typeof value === "function")
        return typeof value + ":<object>";
    return typeof value + ":" + String(value);
}
function observe(thunk) {
    try {
        return show(thunk());
    } catch (error) {
        if (error === membershipSentinel) return "throw:sentinel";
        if (error !== null && typeof error === "object")
            return "throw:" + error.name + "|" + error.message;
        return "throw:" + typeof error + "|" + String(error);
    }
}
function identity(value) { return value; }

var membershipLog = "";
var membershipSentinel = {};
var membershipProto = {};
Object.defineProperty(membershipProto, "inherited", {
    value: 1, writable: true, enumerable: true, configurable: true
});
Object.defineProperty(membershipProto, "accessor", {
    get: function () { membershipLog += "G"; return 1; },
    enumerable: true, configurable: true
});
var membershipTarget = Object.create(membershipProto);
for (var membershipName of ["own", "1", "2", "true", "string"])
    Object.defineProperty(membershipTarget, membershipName, {
        value: 1, writable: true, enumerable: true, configurable: true
    });
var membershipSymbol = Symbol("membership");
membershipTarget[membershipSymbol] = 1;
var membershipKey = {
    [Symbol.toPrimitive]: function (hint) {
        membershipLog += "k(" + hint + ")";
        return "own";
    }
};
var membershipThrowKey = {
    [Symbol.toPrimitive]: function (hint) {
        membershipLog += "k(" + hint + ")";
        throw membershipSentinel;
    }
};
function membershipLeft() { membershipLog += "l"; return membershipKey; }
function membershipThrowLeft() { membershipLog += "l"; return membershipThrowKey; }
function membershipRight() { membershipLog += "r"; return membershipTarget; }
function membershipPrimitiveRight() { membershipLog += "r"; return 1; }

function MembershipCtor() {}
var membershipCtor = MembershipCtor;
var membershipInstance = new membershipCtor();
var membershipOther = {};
var membershipNeedle = {};
var membershipBound = membershipCtor.bind(null);
var membershipBoundInstance = new membershipBound();

function MembershipCustomCtor() {}
var membershipCustomCtor = MembershipCustomCtor;
Object.defineProperty(membershipCustomCtor, Symbol.hasInstance, {
    value: function (candidate) {
        membershipLog += this === membershipCustomCtor ? "ct" : "cw";
        return candidate === membershipNeedle;
    },
    configurable: true
});
var membershipCustomBound = membershipCustomCtor.bind(null);

var membershipInheritedHasInstanceProto = {};
Object.defineProperty(membershipInheritedHasInstanceProto, Symbol.hasInstance, {
    value: function (candidate) {
        membershipLog += this === membershipInheritedHasInstanceTarget ? "it" : "iw";
        return candidate === membershipNeedle;
    },
    configurable: true
});
var membershipInheritedHasInstanceTarget =
    Object.create(membershipInheritedHasInstanceProto);

var membershipAccessorMethod = function (candidate) {
    membershipLog += this === membershipAccessorTarget ? "ct" : "cw";
    return candidate === membershipNeedle;
};
var membershipAccessorTarget = {};
Object.defineProperty(membershipAccessorTarget, Symbol.hasInstance, {
    get: function () { membershipLog += "g"; return membershipAccessorMethod; },
    configurable: true
});
function membershipInstanceLeft() { membershipLog += "l"; return membershipNeedle; }
function membershipInstanceRight() { membershipLog += "r"; return membershipAccessorTarget; }

function MembershipNullHasInstance() {}
var membershipNullHasInstance = MembershipNullHasInstance;
Object.defineProperty(membershipNullHasInstance, Symbol.hasInstance, {
    value: null, configurable: true
});
var membershipNullInstance = new membershipNullHasInstance();

var membershipPrototypeMatch = {};
var membershipPrototypeCandidate = Object.create(membershipPrototypeMatch);
Object.defineProperty(Function.prototype, "prototype", {
    get: function () { membershipLog += "p"; return membershipPrototypeMatch; },
    configurable: true
});

var membershipPrototypeThrowTarget = Function.prototype.call;
Object.defineProperty(membershipPrototypeThrowTarget, "prototype", {
    get: function () { membershipLog += "P"; throw membershipSentinel; },
    configurable: true
});

var membershipInvalidTarget = {};
var membershipBadMethodTarget = {};
Object.defineProperty(membershipBadMethodTarget, Symbol.hasInstance, {
    value: 1, configurable: true
});
var membershipGetterThrowTarget = {};
Object.defineProperty(membershipGetterThrowTarget, Symbol.hasInstance, {
    get: function () { membershipLog += "g"; throw membershipSentinel; },
    configurable: true
});
var membershipMethodThrowTarget = {};
Object.defineProperty(membershipMethodThrowTarget, Symbol.hasInstance, {
    value: function () { membershipLog += "c"; throw membershipSentinel; },
    configurable: true
});
function MembershipBadPrototype() {}
var membershipBadPrototype = MembershipBadPrototype;
membershipBadPrototype.prototype = 1;

print("precedence=" + [
    show(1 + 1 in membershipTarget),
    show(0 | 1 in membershipTarget),
    show("own" in membershipTarget === true),
    show(false == "missing" in membershipTarget),
    show("own" in membershipTarget ? 7 : 9),
    show(membershipInstance instanceof membershipCtor === true),
    show(membershipInstance instanceof membershipCtor in membershipTarget),
    show(false && "x" in null),
    show(typeof "own" in membershipTarget)
].join("|"));

print("no-in-valid=" + [
    show((function () {
        for (var value = ("own" in membershipTarget); false; );
        return value;
    })()),
    show((function () {
        for (var value = true ? "own" in membershipTarget : false; false; );
        return value;
    })()),
    show((function () {
        for (var value = identity("own" in membershipTarget); false; );
        return value;
    })()),
    show((function () {
        for (var value = false ? false : ("own" in membershipTarget); false; );
        return value;
    })()),
    show((function () {
        for (var value = false; false; );
        return "own" in membershipTarget;
    })()),
    show((function () {
        for (var value = membershipInstance instanceof membershipCtor; false; );
        return value;
    })())
].join("|"));

print("in-basic=" + [
    show("own" in membershipTarget),
    show("inherited" in membershipTarget),
    show("missing" in membershipTarget),
    show(1 in membershipTarget),
    show(membershipSymbol in membershipTarget),
    show(Symbol.hasInstance in Function.prototype)
].join("|"));

membershipLog = "";
var membershipObservation = observe(function () {
    return "accessor" in membershipTarget;
});
print("in-accessor=" + membershipObservation + "|log:" + membershipLog);

membershipLog = "";
membershipObservation = observe(function () {
    return membershipLeft() in membershipRight();
});
print("in-order=" + membershipObservation + "|log:" + membershipLog);

membershipLog = "";
membershipObservation = observe(function () {
    return membershipLeft() in membershipPrimitiveRight();
});
print("in-invalid-rhs-order=" + membershipObservation + "|log:" + membershipLog);

membershipLog = "";
membershipObservation = observe(function () {
    return membershipThrowLeft() in membershipRight();
});
print("in-key-throw=" + membershipObservation + "|log:" + membershipLog);

print("in-invalid=" + [
    observe(function () { return "x" in null; }),
    observe(function () { return "x" in undefined; }),
    observe(function () { return "x" in 1; }),
    observe(function () { return "x" in Symbol("rhs"); })
].join("|"));

print("instance-basic=" + [
    show(membershipInstance instanceof membershipCtor),
    show(membershipOther instanceof membershipCtor),
    show(1 instanceof membershipCtor),
    show(membershipBoundInstance instanceof membershipCtor),
    show(membershipBoundInstance instanceof membershipBound)
].join("|"));

membershipLog = "";
print("instance-custom=" +
      show(membershipNeedle instanceof membershipCustomCtor) + "|" +
      show(membershipOther instanceof membershipCustomCtor) +
      "|log:" + membershipLog);

membershipLog = "";
print("instance-inherited-custom=" +
      show(membershipNeedle instanceof membershipInheritedHasInstanceTarget) +
      "|log:" + membershipLog);

membershipLog = "";
print("instance-accessor-custom=" +
      show(membershipNeedle instanceof membershipAccessorTarget) +
      "|log:" + membershipLog);

membershipLog = "";
print("instance-order=" +
      show(membershipInstanceLeft() instanceof membershipInstanceRight()) +
      "|log:" + membershipLog);

print("instance-null-fallback=" +
      show(membershipNullInstance instanceof membershipNullHasInstance));

membershipLog = "";
print("instance-bound-custom=" +
      show(membershipNeedle instanceof membershipCustomBound) +
      "|log:" + membershipLog);

membershipLog = "";
print("instance-prototype-accessor=" +
      show(membershipPrototypeCandidate instanceof Function.prototype) +
      "|log:" + membershipLog);

membershipLog = "";
print("instance-primitive-short=" +
      show(1 instanceof membershipPrototypeThrowTarget) +
      "|log:" + membershipLog);

print("instance-invalid=" + [
    observe(function () { return membershipNeedle instanceof 1; }),
    observe(function () { return membershipNeedle instanceof membershipInvalidTarget; }),
    observe(function () { return membershipNeedle instanceof membershipBadMethodTarget; }),
    observe(function () { return membershipNeedle instanceof membershipBadPrototype; })
].join("|"));

membershipLog = "";
var getterThrow = observe(function () {
    return membershipNeedle instanceof membershipGetterThrowTarget;
});
print("instance-getter-throw=" + getterThrow + "|log:" + membershipLog);

membershipLog = "";
var methodThrow = observe(function () {
    return membershipNeedle instanceof membershipMethodThrowTarget;
});
print("instance-method-throw=" + methodThrow + "|log:" + membershipLog);

membershipLog = "";
var prototypeThrow = observe(function () {
    return membershipNeedle instanceof membershipPrototypeThrowTarget;
});
print("instance-prototype-throw=" + prototypeThrow + "|log:" + membershipLog);
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    "precedence=boolean:true|number:1|boolean:true|boolean:true|number:7|boolean:true|boolean:true|boolean:false|boolean:true",
    "no-in-valid=boolean:true|boolean:true|boolean:true|boolean:true|boolean:true|boolean:true",
    "in-basic=boolean:true|boolean:true|boolean:false|boolean:true|boolean:true|boolean:true",
    "in-accessor=boolean:true|log:",
    "in-order=boolean:true|log:lrk(string)",
    "in-invalid-rhs-order=throw:TypeError|invalid 'in' operand|log:lr",
    "in-key-throw=throw:sentinel|log:lrk(string)",
    "in-invalid=throw:TypeError|invalid 'in' operand|throw:TypeError|invalid 'in' operand|throw:TypeError|invalid 'in' operand|throw:TypeError|invalid 'in' operand",
    "instance-basic=boolean:true|boolean:false|boolean:false|boolean:true|boolean:true",
    "instance-custom=boolean:true|boolean:false|log:ctct",
    "instance-inherited-custom=boolean:true|log:it",
    "instance-accessor-custom=boolean:true|log:gct",
    "instance-order=boolean:true|log:lrgct",
    "instance-null-fallback=boolean:true",
    "instance-bound-custom=boolean:true|log:ct",
    "instance-prototype-accessor=boolean:true|log:p",
    "instance-primitive-short=boolean:false|log:",
    "instance-invalid=throw:TypeError|invalid 'instanceof' right operand|throw:TypeError|invalid 'instanceof' right operand|throw:TypeError|not a function|throw:TypeError|operand 'prototype' property is not an object",
    "instance-getter-throw=throw:sentinel|log:g",
    "instance-method-throw=throw:sentinel|log:c",
    "instance-prototype-throw=throw:sentinel|log:P",
];

const NO_IN_ERROR_CASES: &[(&str, &str)] = &[
    (
        "unparenthesized in in a classic-for var initializer",
        "(function(){ for (var value = 'own' in membershipTarget; false; ); })()",
    ),
    (
        "conditional alternate inherits the classic-for NoIn mode",
        "(function(){ for (var value = false ? false : 'own' in membershipTarget; false; ); })()",
    ),
    (
        "assignment right hand side inherits the classic-for NoIn mode",
        "(function(){ var other; for (var value = other = 'own' in membershipTarget; false; ); })()",
    ),
    (
        "expression initializer inherits the classic-for NoIn mode",
        "(function(){ var value; for (value = 'own' in membershipTarget; false; ); })()",
    ),
];

const PARSER_ERROR_CASES: &[(&str, &str)] = &[
    (
        "numeric adjacency before instanceof",
        "1instanceof Function",
    ),
    ("missing instanceof right operand", "1 instanceof"),
    ("missing in right operand", "'x' in"),
    ("escaped in is not an operator", "1 \\u0069n Function"),
    (
        "escaped instanceof is not an operator",
        "1 \\u0069nstanceof Function",
    ),
];

#[test]
fn pinned_quickjs_relational_membership_contract_is_stable() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP pinned relational-membership contract: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        oracle_observations(&oracle),
        EXPECTED_OBSERVATIONS,
        "the pinned QuickJS in/instanceof contract drifted"
    );
}

#[test]
fn source_relational_membership_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP relational-membership differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        rust_observations(),
        oracle_observations(&oracle),
        "source in/instanceof behavior differed from pinned QuickJS"
    );
}

#[test]
fn source_relational_membership_no_in_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP relational-membership NoIn differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in NO_IN_ERROR_CASES {
        assert_eq!(
            rust_error_observation(source),
            oracle_error_observation(&oracle, source),
            "NoIn diagnostic mismatch for {description}: {source:?}"
        );
    }
    for &(description, source) in PARSER_ERROR_CASES {
        assert_eq!(
            rust_error_observation(source),
            oracle_error_observation(&oracle, source),
            "membership parser diagnostic mismatch for {description}: {source:?}"
        );
    }
}

#[test]
fn membership_errors_follow_opcode_conversion_and_method_defining_realms() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let second_type_error = intrinsic_prototype(&runtime, &mut second, "TypeError");
    assert_ne!(first_type_error, second_type_error);

    for source in [
        "(function(){ return 'x' in 1; })",
        "(function(){ return 1 instanceof 2; })",
    ] {
        let probe = function(&runtime, &mut first, source);
        assert_eq!(
            second.call(&probe, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        );
        let error = take_exception_object(&mut second);
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(first_type_error.clone()),
            "opcode framework error did not use its bytecode realm for {source:?}"
        );
    }

    let has_instance = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let custom_target = second.new_object().unwrap();
    let custom_method = function(
        &runtime,
        &mut first,
        "(function(){ throw new TypeError('custom hasInstance realm'); })",
    );
    define_data_key(
        &mut second,
        &custom_target,
        &has_instance,
        Value::Object(custom_method.as_object().clone()),
    );
    define_global(
        &runtime,
        &mut second,
        "realmCustomTarget",
        Value::Object(custom_target),
    );
    let custom_probe = function(
        &runtime,
        &mut second,
        "(function(){ return Function instanceof realmCustomTarget; })",
    );
    assert_eq!(
        second.call(&custom_probe, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let custom_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&custom_error).unwrap(),
        Some(first_type_error.clone())
    );

    let bad_target = function(&runtime, &mut first, "(function RealmBadPrototype(){})");
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        first
            .set_property(bad_target.as_object(), &prototype, Value::Int(1))
            .unwrap()
    );
    define_global(
        &runtime,
        &mut second,
        "realmBadPrototype",
        Value::Object(bad_target.as_object().clone()),
    );
    let native_probe = function(
        &runtime,
        &mut second,
        "(function(){ return Function instanceof realmBadPrototype; })",
    );
    assert_eq!(
        second.call(&native_probe, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let native_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(first_type_error.clone()),
        "standard hasInstance error did not use the native method realm"
    );

    let null_target = function(&runtime, &mut first, "(function RealmNullHasInstance(){})");
    define_data_key(
        &mut first,
        null_target.as_object(),
        &has_instance,
        Value::Null,
    );
    assert!(
        first
            .set_property(null_target.as_object(), &prototype, Value::Int(1))
            .unwrap()
    );
    define_global(
        &runtime,
        &mut second,
        "realmNullHasInstance",
        Value::Object(null_target.as_object().clone()),
    );
    let null_probe = function(
        &runtime,
        &mut second,
        "(function(){ return Function instanceof realmNullHasInstance; })",
    );
    assert_eq!(
        second.call(&null_probe, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let null_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&null_error).unwrap(),
        Some(second_type_error.clone()),
        "nullish hasInstance fallback did not use the opcode realm"
    );

    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    let throwing_key = second.new_object().unwrap();
    let throwing_conversion = function(
        &runtime,
        &mut first,
        "(function(){ throw new TypeError('key conversion realm'); })",
    );
    define_data_key(
        &mut second,
        &throwing_key,
        &to_primitive,
        Value::Object(throwing_conversion.as_object().clone()),
    );
    define_global(
        &runtime,
        &mut second,
        "realmThrowingKey",
        Value::Object(throwing_key),
    );
    let target = second.new_object().unwrap();
    define_global(
        &runtime,
        &mut second,
        "realmMembershipTarget",
        Value::Object(target),
    );
    let throwing_key_probe = function(
        &runtime,
        &mut second,
        "(function(){ return realmThrowingKey in realmMembershipTarget; })",
    );
    assert_eq!(
        second.call(&throwing_key_probe, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let conversion_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&conversion_error).unwrap(),
        Some(first_type_error)
    );

    let invalid_key = second.new_object().unwrap();
    let invalid_conversion = function(&runtime, &mut first, "(function(){ return Function; })");
    define_data_key(
        &mut second,
        &invalid_key,
        &to_primitive,
        Value::Object(invalid_conversion.as_object().clone()),
    );
    define_global(
        &runtime,
        &mut second,
        "realmInvalidKey",
        Value::Object(invalid_key),
    );
    let invalid_key_probe = function(
        &runtime,
        &mut second,
        "(function(){ return realmInvalidKey in realmMembershipTarget; })",
    );
    assert_eq!(
        second.call(&invalid_key_probe, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let invalid_conversion_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&invalid_conversion_error).unwrap(),
        Some(second_type_error),
        "post-call ToPrimitive framework error did not return to the opcode realm"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let sentinel = context.new_object().unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipSentinel",
        Value::Object(sentinel.clone()),
    );
    define_global(&runtime, &mut context, "membershipLog", string(""));

    let proto = context.new_object().unwrap();
    define_data_name(&runtime, &mut context, &proto, "inherited", Value::Int(1));
    let accessor = function(
        &runtime,
        &mut context,
        "(function(){ membershipLog = membershipLog + 'G'; return 1; })",
    );
    define_accessor_name(
        &runtime,
        &mut context,
        &proto,
        "accessor",
        AccessorValue::Callable(accessor),
    );
    let target = context.new_object_with_prototype(Some(&proto)).unwrap();
    for name in ["own", "1", "2", "true", "string"] {
        define_data_name(&runtime, &mut context, &target, name, Value::Int(1));
    }
    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("membership").unwrap()))
        .unwrap();
    define_data_key(
        &mut context,
        &target,
        &PropertyKey::from(symbol.clone()),
        Value::Int(1),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipSymbol",
        Value::Symbol(symbol),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipTarget",
        Value::Object(target.clone()),
    );

    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    let key = context.new_object().unwrap();
    let key_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ membershipLog = membershipLog + 'k(' + hint + ')'; return 'own'; })",
    );
    define_data_key(
        &mut context,
        &key,
        &to_primitive,
        Value::Object(key_converter.as_object().clone()),
    );
    define_global(&runtime, &mut context, "membershipKey", Value::Object(key));

    let throw_key = context.new_object().unwrap();
    let throw_key_converter = function(
        &runtime,
        &mut context,
        "(function(hint){ membershipLog = membershipLog + 'k(' + hint + ')'; throw membershipSentinel; })",
    );
    define_data_key(
        &mut context,
        &throw_key,
        &to_primitive,
        Value::Object(throw_key_converter.as_object().clone()),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipThrowKey",
        Value::Object(throw_key),
    );

    for (name, source) in [
        ("identity", "(function(value){ return value; })"),
        (
            "membershipLeft",
            "(function(){ membershipLog = membershipLog + 'l'; return membershipKey; })",
        ),
        (
            "membershipThrowLeft",
            "(function(){ membershipLog = membershipLog + 'l'; return membershipThrowKey; })",
        ),
        (
            "membershipRight",
            "(function(){ membershipLog = membershipLog + 'r'; return membershipTarget; })",
        ),
        (
            "membershipPrimitiveRight",
            "(function(){ membershipLog = membershipLog + 'r'; return 1; })",
        ),
    ] {
        let callable = function(&runtime, &mut context, source);
        define_global(
            &runtime,
            &mut context,
            name,
            Value::Object(callable.as_object().clone()),
        );
    }

    let ctor = function(&runtime, &mut context, "(function MembershipCtor(){})");
    define_global(
        &runtime,
        &mut context,
        "membershipCtor",
        Value::Object(ctor.as_object().clone()),
    );
    let instance = expect_object(context.eval("new membershipCtor()").unwrap());
    define_global(
        &runtime,
        &mut context,
        "membershipInstance",
        Value::Object(instance),
    );
    let other = context.new_object().unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipOther",
        Value::Object(other.clone()),
    );
    let needle = context.new_object().unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipNeedle",
        Value::Object(needle.clone()),
    );

    let bound = expect_object(context.eval("membershipCtor.bind(null)").unwrap());
    define_global(
        &runtime,
        &mut context,
        "membershipBound",
        Value::Object(bound),
    );
    let bound_instance = expect_object(context.eval("new membershipBound()").unwrap());
    define_global(
        &runtime,
        &mut context,
        "membershipBoundInstance",
        Value::Object(bound_instance),
    );

    let custom_ctor = function(
        &runtime,
        &mut context,
        "(function MembershipCustomCtor(){})",
    );
    define_global(
        &runtime,
        &mut context,
        "membershipCustomCtor",
        Value::Object(custom_ctor.as_object().clone()),
    );
    let custom_method = function(
        &runtime,
        &mut context,
        "(function(candidate){ membershipLog = membershipLog + (this === membershipCustomCtor ? 'ct' : 'cw'); return candidate === membershipNeedle; })",
    );
    let has_instance = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    define_data_key(
        &mut context,
        custom_ctor.as_object(),
        &has_instance,
        Value::Object(custom_method.as_object().clone()),
    );
    let custom_bound = expect_object(context.eval("membershipCustomCtor.bind(null)").unwrap());
    define_global(
        &runtime,
        &mut context,
        "membershipCustomBound",
        Value::Object(custom_bound),
    );

    let inherited_has_instance_proto = context.new_object().unwrap();
    let inherited_has_instance_target = context
        .new_object_with_prototype(Some(&inherited_has_instance_proto))
        .unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipInheritedHasInstanceTarget",
        Value::Object(inherited_has_instance_target.clone()),
    );
    let inherited_method = function(
        &runtime,
        &mut context,
        "(function(candidate){ membershipLog = membershipLog + (this === membershipInheritedHasInstanceTarget ? 'it' : 'iw'); return candidate === membershipNeedle; })",
    );
    define_data_key(
        &mut context,
        &inherited_has_instance_proto,
        &has_instance,
        Value::Object(inherited_method.as_object().clone()),
    );

    let accessor_target = context.new_object().unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipAccessorTarget",
        Value::Object(accessor_target.clone()),
    );
    let accessor_method = function(
        &runtime,
        &mut context,
        "(function(candidate){ membershipLog = membershipLog + (this === membershipAccessorTarget ? 'ct' : 'cw'); return candidate === membershipNeedle; })",
    );
    define_global(
        &runtime,
        &mut context,
        "membershipAccessorMethod",
        Value::Object(accessor_method.as_object().clone()),
    );
    let accessor_getter = function(
        &runtime,
        &mut context,
        "(function(){ membershipLog = membershipLog + 'g'; return membershipAccessorMethod; })",
    );
    define_accessor_key(
        &mut context,
        &accessor_target,
        &has_instance,
        AccessorValue::Callable(accessor_getter),
    );
    for (name, source) in [
        (
            "membershipInstanceLeft",
            "(function(){ membershipLog = membershipLog + 'l'; return membershipNeedle; })",
        ),
        (
            "membershipInstanceRight",
            "(function(){ membershipLog = membershipLog + 'r'; return membershipAccessorTarget; })",
        ),
    ] {
        let callable = function(&runtime, &mut context, source);
        define_global(
            &runtime,
            &mut context,
            name,
            Value::Object(callable.as_object().clone()),
        );
    }

    let null_has_instance = function(
        &runtime,
        &mut context,
        "(function MembershipNullHasInstance(){})",
    );
    define_data_key(
        &mut context,
        null_has_instance.as_object(),
        &has_instance,
        Value::Null,
    );
    define_global(
        &runtime,
        &mut context,
        "membershipNullHasInstance",
        Value::Object(null_has_instance.as_object().clone()),
    );
    let null_instance = expect_object(context.eval("new membershipNullHasInstance()").unwrap());
    define_global(
        &runtime,
        &mut context,
        "membershipNullInstance",
        Value::Object(null_instance),
    );

    let prototype_match = context.new_object().unwrap();
    let prototype_candidate = context
        .new_object_with_prototype(Some(&prototype_match))
        .unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipPrototypeMatch",
        Value::Object(prototype_match),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipPrototypeCandidate",
        Value::Object(prototype_candidate),
    );
    let function_prototype = context.function_prototype().unwrap();
    let prototype_getter = function(
        &runtime,
        &mut context,
        "(function(){ membershipLog = membershipLog + 'p'; return membershipPrototypeMatch; })",
    );
    define_accessor_name(
        &runtime,
        &mut context,
        &function_prototype,
        "prototype",
        AccessorValue::Callable(prototype_getter),
    );

    let call = property_callable(&runtime, &mut context, &function_prototype, "call");
    let prototype_throw_getter = function(
        &runtime,
        &mut context,
        "(function(){ membershipLog = membershipLog + 'P'; throw membershipSentinel; })",
    );
    define_accessor_name(
        &runtime,
        &mut context,
        call.as_object(),
        "prototype",
        AccessorValue::Callable(prototype_throw_getter),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipPrototypeThrowTarget",
        Value::Object(call.as_object().clone()),
    );

    let invalid_target = context.new_object().unwrap();
    define_global(
        &runtime,
        &mut context,
        "membershipInvalidTarget",
        Value::Object(invalid_target),
    );
    let bad_method_target = context.new_object().unwrap();
    define_data_key(
        &mut context,
        &bad_method_target,
        &has_instance,
        Value::Int(1),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipBadMethodTarget",
        Value::Object(bad_method_target),
    );

    let getter_throw_target = context.new_object().unwrap();
    let getter_throw = function(
        &runtime,
        &mut context,
        "(function(){ membershipLog = membershipLog + 'g'; throw membershipSentinel; })",
    );
    define_accessor_key(
        &mut context,
        &getter_throw_target,
        &has_instance,
        AccessorValue::Callable(getter_throw),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipGetterThrowTarget",
        Value::Object(getter_throw_target),
    );

    let method_throw_target = context.new_object().unwrap();
    let method_throw = function(
        &runtime,
        &mut context,
        "(function(){ membershipLog = membershipLog + 'c'; throw membershipSentinel; })",
    );
    define_data_key(
        &mut context,
        &method_throw_target,
        &has_instance,
        Value::Object(method_throw.as_object().clone()),
    );
    define_global(
        &runtime,
        &mut context,
        "membershipMethodThrowTarget",
        Value::Object(method_throw_target),
    );

    let bad_prototype = function(
        &runtime,
        &mut context,
        "(function MembershipBadPrototype(){})",
    );
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        context
            .set_property(bad_prototype.as_object(), &prototype_key, Value::Int(1))
            .unwrap()
    );
    define_global(
        &runtime,
        &mut context,
        "membershipBadPrototype",
        Value::Object(bad_prototype.as_object().clone()),
    );

    let mut output = Vec::new();
    output.push(format!(
        "precedence={}",
        [
            "1 + 1 in membershipTarget",
            "0 | 1 in membershipTarget",
            "'own' in membershipTarget === true",
            "false == 'missing' in membershipTarget",
            "'own' in membershipTarget ? 7 : 9",
            "membershipInstance instanceof membershipCtor === true",
            "membershipInstance instanceof membershipCtor in membershipTarget",
            "false && 'x' in null",
            "typeof 'own' in membershipTarget",
        ]
        .map(|source| observe(&runtime, &mut context, &sentinel, source))
        .join("|")
    ));
    output.push(format!(
        "no-in-valid={}",
        [
            "(function(){ for (var value = ('own' in membershipTarget); false; ); return value; })()",
            "(function(){ for (var value = true ? 'own' in membershipTarget : false; false; ); return value; })()",
            "(function(){ for (var value = identity('own' in membershipTarget); false; ); return value; })()",
            "(function(){ for (var value = false ? false : ('own' in membershipTarget); false; ); return value; })()",
            "(function(){ for (var value = false; false; ); return 'own' in membershipTarget; })()",
            "(function(){ for (var value = membershipInstance instanceof membershipCtor; false; ); return value; })()",
        ]
        .map(|source| observe(&runtime, &mut context, &sentinel, source))
        .join("|")
    ));
    output.push(format!(
        "in-basic={}",
        [
            "'own' in membershipTarget",
            "'inherited' in membershipTarget",
            "'missing' in membershipTarget",
            "1 in membershipTarget",
            "membershipSymbol in membershipTarget",
            "Symbol.hasInstance in Function.prototype",
        ]
        .map(|source| observe(&runtime, &mut context, &sentinel, source))
        .join("|")
    ));

    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "in-accessor",
        "'accessor' in membershipTarget",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "in-order",
        "membershipLeft() in membershipRight()",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "in-invalid-rhs-order",
        "membershipLeft() in membershipPrimitiveRight()",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "in-key-throw",
        "membershipThrowLeft() in membershipRight()",
    ));
    output.push(format!(
        "in-invalid={}",
        [
            "'x' in null",
            "'x' in undefined",
            "'x' in 1",
            "'x' in Symbol('rhs')",
        ]
        .map(|source| observe(&runtime, &mut context, &sentinel, source))
        .join("|")
    ));

    output.push(format!(
        "instance-basic={}",
        [
            "membershipInstance instanceof membershipCtor",
            "membershipOther instanceof membershipCtor",
            "1 instanceof membershipCtor",
            "membershipBoundInstance instanceof membershipCtor",
            "membershipBoundInstance instanceof membershipBound",
        ]
        .map(|source| observe(&runtime, &mut context, &sentinel, source))
        .join("|")
    ));
    set_log(&runtime, &mut context, "");
    let custom_true = observe(
        &runtime,
        &mut context,
        &sentinel,
        "membershipNeedle instanceof membershipCustomCtor",
    );
    let custom_false = observe(
        &runtime,
        &mut context,
        &sentinel,
        "membershipOther instanceof membershipCustomCtor",
    );
    output.push(format!(
        "instance-custom={custom_true}|{custom_false}|log:{}",
        string_global(&runtime, &mut context, "membershipLog")
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-inherited-custom",
        "membershipNeedle instanceof membershipInheritedHasInstanceTarget",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-accessor-custom",
        "membershipNeedle instanceof membershipAccessorTarget",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-order",
        "membershipInstanceLeft() instanceof membershipInstanceRight()",
    ));
    output.push(format!(
        "instance-null-fallback={}",
        observe(
            &runtime,
            &mut context,
            &sentinel,
            "membershipNullInstance instanceof membershipNullHasInstance"
        )
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-bound-custom",
        "membershipNeedle instanceof membershipCustomBound",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-prototype-accessor",
        "membershipPrototypeCandidate instanceof Function.prototype",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-primitive-short",
        "1 instanceof membershipPrototypeThrowTarget",
    ));
    output.push(format!(
        "instance-invalid={}",
        [
            "membershipNeedle instanceof 1",
            "membershipNeedle instanceof membershipInvalidTarget",
            "membershipNeedle instanceof membershipBadMethodTarget",
            "membershipNeedle instanceof membershipBadPrototype",
        ]
        .map(|source| observe(&runtime, &mut context, &sentinel, source))
        .join("|")
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-getter-throw",
        "membershipNeedle instanceof membershipGetterThrowTarget",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-method-throw",
        "membershipNeedle instanceof membershipMethodThrowTarget",
    ));
    output.push(observe_with_log(
        &runtime,
        &mut context,
        &sentinel,
        "instance-prototype-throw",
        "membershipNeedle instanceof membershipPrototypeThrowTarget",
    ));

    output
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function probe did not return an object: {source}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("function probe was not callable: {source}"))
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("callable property {name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("property {name} was not callable"))
}

fn expect_object(value: Value) -> ObjectRef {
    let Value::Object(value) = value else {
        panic!("expected object, got {value:?}");
    };
    value
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("expected an Error object exception");
    };
    error
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let global = context.global_object().unwrap();
    let Value::Object(constructor) = get_property(runtime, context, &global, name) else {
        panic!("{name} constructor was not an object");
    };
    let Value::Object(prototype) = get_property(runtime, context, &constructor, "prototype") else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    define_data_name(runtime, context, &global, name, value);
}

fn define_data_name(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
    value: Value,
) {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(context, object, &key, value);
}

fn define_data_key(context: &mut Context, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_accessor_name(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
    get: AccessorValue,
) {
    let key = runtime.intern_property_key(name).unwrap();
    define_accessor_key(context, object, &key, get);
}

fn define_accessor_key(
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
    get: AccessorValue,
) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(get),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn observe_with_log(
    runtime: &Runtime,
    context: &mut Context,
    sentinel: &ObjectRef,
    label: &str,
    source: &str,
) -> String {
    set_log(runtime, context, "");
    let observation = observe(runtime, context, sentinel, source);
    format!(
        "{label}={observation}|log:{}",
        string_global(runtime, context, "membershipLog")
    )
}

fn observe(runtime: &Runtime, context: &mut Context, sentinel: &ObjectRef, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => show(value),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap()
                .expect("exception completion had no value");
            if exception == Value::Object(sentinel.clone()) {
                return "throw:sentinel".to_owned();
            }
            match exception {
                Value::Object(error) if runtime.is_error_object(&error).unwrap() => {
                    let Value::String(name) = get_property(runtime, context, &error, "name") else {
                        panic!("Error.name was not a string for {source:?}");
                    };
                    let Value::String(message) = get_property(runtime, context, &error, "message")
                    else {
                        panic!("Error.message was not a string for {source:?}");
                    };
                    format!("throw:{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
                }
                Value::String(value) => format!("throw:string|{}", value.to_utf8_lossy()),
                Value::Int(value) => format!("throw:number|{value}"),
                Value::Float(value) => format!("throw:number|{value}"),
                Value::Bool(value) => format!("throw:boolean|{value}"),
                Value::Undefined => "throw:undefined|undefined".to_owned(),
                Value::Null => "throw:object|null".to_owned(),
                Value::BigInt(value) => format!("throw:bigint|{value}"),
                Value::Symbol(_) => "throw:symbol|<symbol>".to_owned(),
                Value::Object(_) => "throw:object|<object>".to_owned(),
            }
        }
        Err(error) => panic!("probe {source:?} failed with an engine error: {error}"),
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

fn string(value: &str) -> Value {
    Value::String(JsString::try_from_utf8(value).unwrap())
}

fn set_log(runtime: &Runtime, context: &mut Context, value: &str) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("membershipLog").unwrap();
    assert!(context.set_property(&global, &key, string(value)).unwrap());
}

fn get_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(object, &key).unwrap()
}

fn string_global(runtime: &Runtime, context: &mut Context, name: &str) -> String {
    let global = context.global_object().unwrap();
    let Value::String(value) = get_property(runtime, context, &global, name) else {
        panic!("global {name} was not a string");
    };
    value.to_utf8_lossy()
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS relational-membership oracle");
    assert!(
        output.status.success(),
        "QuickJS relational-membership oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS relational-membership oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn rust_error_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust parser did not materialize an Error object");
    };
    let Value::String(name) = get_property(&runtime, &mut context, &error, "name") else {
        panic!("Rust Error.name was not a string");
    };
    let Value::String(message) = get_property(&runtime, &mut context, &error, "message") else {
        panic!("Rust Error.message was not a string");
    };
    let Value::Int(line) = get_property(&runtime, &mut context, &error, "lineNumber") else {
        panic!("Rust Error.lineNumber was not an integer");
    };
    let Value::Int(column) = get_property(&runtime, &mut context, &error, "columnNumber") else {
        panic!("Rust Error.columnNumber was not an integer");
    };
    format!(
        "{}|{}|{line}:{column}",
        name.to_utf8_lossy(),
        message.to_utf8_lossy()
    )
}

fn oracle_error_observation(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let mut lines = stderr.lines();
    let first = lines
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    let location = lines
        .find_map(|line| line.trim().strip_prefix("at <cmdline>:"))
        .unwrap_or_else(|| panic!("QuickJS emitted no location for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}|{location}",
        first.trim_start_matches("SyntaxError: ")
    )
}
