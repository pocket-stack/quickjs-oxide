use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    Value, WellKnownSymbol,
};

// Deliberate current-slice boundary: the oracle can use Object/Reflect and
// literals to describe the result compactly, while the Rust side constructs
// the same objects and descriptors through the public host API. Cross-realm,
// custom-newTarget and class/Proxy paths belong to runtime unit tests until
// those source-language surfaces exist in quickjs-oxide.
const ORACLE_PROBE: &str = r#"
function flags(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function observe(thunk) {
    try { return String(thunk()); }
    catch (error) { return "throw:" + error.name + ":" + error.message; }
}

var bombHit = false;
var bomb = {};
Object.defineProperty(bomb, Symbol.toPrimitive, {
    value: function() { bombHit = true; throw "coerced"; },
    configurable: true
});

print("call=" + [
    Boolean(), Boolean(undefined), Boolean(null), Boolean(false), Boolean(true),
    Boolean(0), Boolean(-0), Boolean(NaN), Boolean(""), Boolean("0"),
    Boolean(0n), Boolean(1n), Boolean(Symbol("boolean")), Boolean(bomb), bombHit
].join("|"));

var boxedFalse = new Boolean(false);
var boxedTrue = new Boolean(true);
var boxedBomb = new Boolean(bomb);
print("new=" + [
    typeof boxedFalse,
    Object.getPrototypeOf(boxedFalse) === Boolean.prototype,
    Object.prototype.toString.call(boxedFalse),
    boxedFalse.valueOf(),
    Reflect.ownKeys(boxedFalse).length,
    boxedTrue.valueOf(), boxedBomb.valueOf(), bombHit
].join("|"));

print("coercion=" + [
    Boolean(boxedFalse), +boxedFalse, boxedFalse + 1,
    boxedFalse == false, boxedFalse === false,
    boxedFalse.valueOf(), boxedFalse.toString(),
    Object.prototype.valueOf.call(boxedFalse) === boxedFalse,
    +boxedTrue
].join("|"));

print("methods=" + [
    false.valueOf(), true.toString(),
    Boolean.prototype.valueOf(), Boolean.prototype.toString(),
    Object.prototype.toString.call(Boolean.prototype),
    Object.getPrototypeOf(Boolean.prototype) === Object.prototype
].join("|"));

print("ctor-keys=" + Reflect.ownKeys(Boolean).map(String).join(","));
print("proto-keys=" + Reflect.ownKeys(Boolean.prototype).map(String).join(","));
print("descriptors=" + [
    flags(globalThis, "Boolean"),
    flags(Boolean, "length"), flags(Boolean, "name"), flags(Boolean, "prototype"),
    flags(Boolean.prototype, "toString"), flags(Boolean.prototype, "valueOf"),
    flags(Boolean.prototype, "constructor")
].join("|"));
print("graph=" + [
    typeof Boolean,
    Object.getPrototypeOf(Boolean) === Function.prototype,
    Boolean.prototype.constructor === Boolean,
    Object.isExtensible(Boolean.prototype)
].join("|"));

print("brands=" + [
    observe(function() { return Boolean.prototype.valueOf.call(0); }),
    observe(function() { return Boolean.prototype.toString.call("false"); }),
    observe(function() { var detached = Boolean.prototype.valueOf; return detached(); })
].join("|"));

var objectBoxA = Object.prototype.valueOf.call(false);
var objectBoxB = Object.prototype.valueOf.call(false);
print("object-links=" + [
    Object.prototype.toString.call(false),
    Object.prototype.toString.call(true),
    Object.getPrototypeOf(objectBoxA) === Boolean.prototype,
    Boolean.prototype.valueOf.call(objectBoxA),
    objectBoxA === objectBoxB,
    Object.prototype.toLocaleString.call(false),
    Object.prototype.toLocaleString.call(true)
].join("|"));

var strictGetThis;
var sloppyGetThis;
var strictSetThis;
var strictSetValue;
var sloppySetThis;
var sloppySetValue;
Object.defineProperty(Boolean.prototype, "__strictGet", {
    configurable: true,
    get: function() { "use strict"; strictGetThis = this; return this; }
});
Object.defineProperty(Boolean.prototype, "__sloppyGet", {
    configurable: true,
    get: function() { sloppyGetThis = this; return this.valueOf(); }
});
Object.defineProperty(Boolean.prototype, "__strictSet", {
    configurable: true,
    set: function(value) { "use strict"; strictSetThis = this; strictSetValue = value; }
});
Object.defineProperty(Boolean.prototype, "__sloppySet", {
    configurable: true,
    set: function(value) { sloppySetThis = this; sloppySetValue = value; }
});
var strictGetResult = (false).__strictGet;
var sloppyGetResult = (false).__sloppyGet;
var strictSetResult = (false).__strictSet = 7;
var sloppySetResult = (false).__sloppySet = 8;
print("accessors=" + [
    strictGetResult, strictGetThis === false,
    sloppyGetResult, typeof sloppyGetThis,
    Object.getPrototypeOf(sloppyGetThis) === Boolean.prototype, sloppyGetThis.valueOf(),
    strictSetResult, strictSetThis === false, strictSetValue,
    sloppySetResult, typeof sloppySetThis,
    Object.getPrototypeOf(sloppySetThis) === Boolean.prototype,
    sloppySetThis.valueOf(), sloppySetValue
].join("|"));

var getterHit = false;
var deleteHit = false;
Object.defineProperty(Boolean.prototype, "__rw", {
    value: 1, writable: true, configurable: true
});
Object.defineProperty(Boolean.prototype, "__ro", {
    value: 1, writable: false, configurable: true
});
Object.defineProperty(Boolean.prototype, "__getterOnly", {
    configurable: true,
    get: function() { getterHit = true; return 1; }
});
Object.defineProperty(Boolean.prototype, "__delete", {
    configurable: true,
    get: function() { deleteHit = true; return 1; }
});
var sloppyRw = (false).__rw = 2;
var sloppyRo = (false).__ro = 2;
var sloppyGetterOnly = (false).__getterOnly = 2;
var deleted = delete (false).__delete;
print("writes=" + [
    sloppyRw, (false).__rw,
    observe(function() { "use strict"; return (false).__rw = 3; }),
    sloppyRo, (false).__ro,
    observe(function() { "use strict"; return (false).__ro = 3; }),
    sloppyGetterOnly,
    observe(function() { "use strict"; return (false).__getterOnly = 3; }),
    getterHit, deleted, deleteHit,
    Object.prototype.hasOwnProperty.call(Boolean.prototype, "__delete"),
    (function() { "use strict"; return delete (false).__delete; })()
].join("|"));

var tagThis;
Object.defineProperty(Boolean.prototype, Symbol.toStringTag, {
    configurable: true,
    get: function() { "use strict"; tagThis = this; return "CustomBoolean"; }
});
var localeGetterThis;
var localeCallThis;
function localeMethod() { "use strict"; localeCallThis = this; return this; }
Object.defineProperty(Boolean.prototype, "toString", {
    configurable: true,
    get: function() { "use strict"; localeGetterThis = this; return localeMethod; }
});
var customTag = Object.prototype.toString.call(false);
var customLocale = Object.prototype.toLocaleString.call(false);
print("custom-object-methods=" + [
    customTag, typeof tagThis, tagThis instanceof Boolean, tagThis.valueOf(),
    customLocale, localeGetterThis === false, localeCallThis === false
].join("|"));
"#;

#[test]
fn boolean_intrinsic_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Boolean intrinsic differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        rust_observations(),
        oracle_observations(&oracle),
        "Boolean intrinsic behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let boolean_prototype = context.boolean_prototype().unwrap();
    let boolean = property_callable(&runtime, &mut context, &global, "Boolean");
    let boolean_object = boolean.as_object().clone();
    let boolean_to_string =
        property_callable(&runtime, &mut context, &boolean_prototype, "toString");
    let boolean_value_of = property_callable(&runtime, &mut context, &boolean_prototype, "valueOf");
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");

    let bomb_hit = define_global(&runtime, &global, "bombHit", Value::Bool(false));
    let bomb = context.new_object().unwrap();
    let bomb_coercion = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { bombHit = true; throw "coerced"; })"#,
    );
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    define_data_key(
        &runtime,
        &bomb,
        &to_primitive,
        Value::Object(bomb_coercion.as_object().clone()),
        true,
        false,
        true,
    );

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("boolean").unwrap()))
        .unwrap();
    let call_values = [
        context.call(&boolean, Value::Undefined, &[]).unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Undefined])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Null])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Bool(false)])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Bool(true)])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Int(0)])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Float(-0.0)])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Float(f64::NAN)])
            .unwrap(),
        context
            .call(
                &boolean,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8("").unwrap())],
            )
            .unwrap(),
        context
            .call(
                &boolean,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8("0").unwrap())],
            )
            .unwrap(),
        context
            .call(
                &boolean,
                Value::Undefined,
                &[Value::BigInt(JsBigInt::zero())],
            )
            .unwrap(),
        context
            .call(
                &boolean,
                Value::Undefined,
                &[Value::BigInt(JsBigInt::one())],
            )
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Symbol(symbol)])
            .unwrap(),
        context
            .call(&boolean, Value::Undefined, &[Value::Object(bomb.clone())])
            .unwrap(),
        context.get_property(&global, &bomb_hit).unwrap(),
    ];
    let mut observations = vec![format!("call={}", join_values(&call_values))];

    let boxed_false = expect_object(
        context.construct(&boolean, &[Value::Bool(false)]).unwrap(),
        "new Boolean(false)",
    );
    let boxed_true = expect_object(
        context.construct(&boolean, &[Value::Bool(true)]).unwrap(),
        "new Boolean(true)",
    );
    let boxed_bomb = expect_object(
        context.construct(&boolean, &[Value::Object(bomb)]).unwrap(),
        "new Boolean(bomb)",
    );
    observations.push(format!(
        "new=object|{}|{}|{}|{}|{}|{}|{}",
        runtime
            .get_prototype_of(&boxed_false)
            .unwrap()
            .is_some_and(|prototype| prototype == boolean_prototype),
        plain_value(
            context
                .call(&object_to_string, Value::Object(boxed_false.clone()), &[],)
                .unwrap()
        ),
        plain_value(unbox_boolean(
            &mut context,
            &boolean_value_of,
            Value::Object(boxed_false.clone()),
        )),
        runtime.own_property_keys(&boxed_false).unwrap().len(),
        plain_value(unbox_boolean(
            &mut context,
            &boolean_value_of,
            Value::Object(boxed_true.clone()),
        )),
        plain_value(unbox_boolean(
            &mut context,
            &boolean_value_of,
            Value::Object(boxed_bomb),
        )),
        plain_value(context.get_property(&global, &bomb_hit).unwrap()),
    ));

    define_global(
        &runtime,
        &global,
        "boxedFalse",
        Value::Object(boxed_false.clone()),
    );
    define_global(
        &runtime,
        &global,
        "boxedTrue",
        Value::Object(boxed_true.clone()),
    );
    let coercion_sources = [
        "Boolean(boxedFalse)",
        "+boxedFalse",
        "boxedFalse + 1",
        "boxedFalse == false",
        "boxedFalse === false",
        "boxedFalse.valueOf()",
        "boxedFalse.toString()",
    ];
    let mut coercion = coercion_sources
        .into_iter()
        .map(|source| plain_value(context.eval(source).unwrap()))
        .collect::<Vec<_>>();
    coercion.push(
        matches!(
            context
                .call(
                    &object_value_of,
                    Value::Object(boxed_false.clone()),
                    &[],
                )
                .unwrap(),
            Value::Object(object) if object == boxed_false
        )
        .to_string(),
    );
    coercion.push(plain_value(context.eval("+boxedTrue").unwrap()));
    observations.push(format!("coercion={}", coercion.join("|")));

    let method_values = [
        context.eval("false.valueOf()").unwrap(),
        context.eval("true.toString()").unwrap(),
        context
            .call(
                &boolean_value_of,
                Value::Object(boolean_prototype.clone()),
                &[],
            )
            .unwrap(),
        context
            .call(
                &boolean_to_string,
                Value::Object(boolean_prototype.clone()),
                &[],
            )
            .unwrap(),
        context
            .call(
                &object_to_string,
                Value::Object(boolean_prototype.clone()),
                &[],
            )
            .unwrap(),
        Value::Bool(
            runtime
                .get_prototype_of(&boolean_prototype)
                .unwrap()
                .is_some_and(|prototype| prototype == object_prototype),
        ),
    ];
    observations.push(format!("methods={}", join_values(&method_values)));

    observations.push(format!(
        "ctor-keys={}",
        own_key_names(&runtime, &boolean_object).join(",")
    ));
    observations.push(format!(
        "proto-keys={}",
        own_key_names(&runtime, &boolean_prototype).join(",")
    ));
    observations.push(format!(
        "descriptors={}",
        [
            data_flags(&runtime, &global, "Boolean"),
            data_flags(&runtime, &boolean_object, "length"),
            data_flags(&runtime, &boolean_object, "name"),
            data_flags(&runtime, &boolean_object, "prototype"),
            data_flags(&runtime, &boolean_prototype, "toString"),
            data_flags(&runtime, &boolean_prototype, "valueOf"),
            data_flags(&runtime, &boolean_prototype, "constructor"),
        ]
        .join("|")
    ));
    observations.push(format!(
        "graph=function|{}|{}|{}",
        runtime
            .get_prototype_of(&boolean_object)
            .unwrap()
            .is_some_and(|prototype| prototype == function_prototype),
        matches!(
            context
                .get_property(
                    &boolean_prototype,
                    &runtime.intern_property_key("constructor").unwrap(),
                )
                .unwrap(),
            Value::Object(object) if object == boolean_object
        ),
        runtime.is_extensible(&boolean_prototype).unwrap(),
    ));

    observations.push(format!(
        "brands={}|{}|{}",
        observe_call(&runtime, &mut context, &boolean_value_of, Value::Int(0),),
        observe_call(
            &runtime,
            &mut context,
            &boolean_to_string,
            Value::String(JsString::try_from_utf8("false").unwrap()),
        ),
        observe_call(&runtime, &mut context, &boolean_value_of, Value::Undefined,),
    ));

    let object_box_a = expect_object(
        context
            .call(&object_value_of, Value::Bool(false), &[])
            .unwrap(),
        "Object.prototype.valueOf.call(false)",
    );
    let object_box_b = expect_object(
        context
            .call(&object_value_of, Value::Bool(false), &[])
            .unwrap(),
        "second Object.prototype.valueOf.call(false)",
    );
    let object_links = [
        plain_value(
            context
                .call(&object_to_string, Value::Bool(false), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(&object_to_string, Value::Bool(true), &[])
                .unwrap(),
        ),
        runtime
            .get_prototype_of(&object_box_a)
            .unwrap()
            .is_some_and(|prototype| prototype == boolean_prototype)
            .to_string(),
        plain_value(unbox_boolean(
            &mut context,
            &boolean_value_of,
            Value::Object(object_box_a.clone()),
        )),
        (object_box_a == object_box_b).to_string(),
        plain_value(
            context
                .call(&object_to_locale_string, Value::Bool(false), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(&object_to_locale_string, Value::Bool(true), &[])
                .unwrap(),
        ),
    ];
    observations.push(format!("object-links={}", object_links.join("|")));

    for name in [
        "strictGetThis",
        "sloppyGetThis",
        "strictSetThis",
        "strictSetValue",
        "sloppySetThis",
        "sloppySetValue",
    ] {
        define_global(&runtime, &global, name, Value::Undefined);
    }
    let strict_get = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; strictGetThis = this; return this; })"#,
    );
    let sloppy_get = eval_callable(
        &runtime,
        &mut context,
        "(function() { sloppyGetThis = this; return this.valueOf(); })",
    );
    let strict_set = eval_callable(
        &runtime,
        &mut context,
        r#"(function(value) { "use strict"; strictSetThis = this; strictSetValue = value; })"#,
    );
    let sloppy_set = eval_callable(
        &runtime,
        &mut context,
        "(function(value) { sloppySetThis = this; sloppySetValue = value; })",
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "__strictGet",
        Some(strict_get),
        None,
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "__sloppyGet",
        Some(sloppy_get),
        None,
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "__strictSet",
        None,
        Some(strict_set),
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "__sloppySet",
        None,
        Some(sloppy_set),
    );
    let strict_get_result = context.eval("(false).__strictGet").unwrap();
    let sloppy_get_result = context.eval("(false).__sloppyGet").unwrap();
    let strict_set_result = context.eval("(false).__strictSet = 7").unwrap();
    let sloppy_set_result = context.eval("(false).__sloppySet = 8").unwrap();
    let strict_get_this = global_value(&runtime, &mut context, &global, "strictGetThis");
    let sloppy_get_this = global_value(&runtime, &mut context, &global, "sloppyGetThis");
    let strict_set_this = global_value(&runtime, &mut context, &global, "strictSetThis");
    let sloppy_set_this = global_value(&runtime, &mut context, &global, "sloppySetThis");
    let accessor_values = [
        plain_value(strict_get_result),
        matches!(strict_get_this, Value::Bool(false)).to_string(),
        plain_value(sloppy_get_result),
        "object".to_owned(),
        object_has_prototype(&runtime, &sloppy_get_this, &boolean_prototype).to_string(),
        plain_value(unbox_boolean(
            &mut context,
            &boolean_value_of,
            sloppy_get_this,
        )),
        plain_value(strict_set_result),
        matches!(strict_set_this, Value::Bool(false)).to_string(),
        plain_value(global_value(
            &runtime,
            &mut context,
            &global,
            "strictSetValue",
        )),
        plain_value(sloppy_set_result),
        "object".to_owned(),
        object_has_prototype(&runtime, &sloppy_set_this, &boolean_prototype).to_string(),
        plain_value(unbox_boolean(
            &mut context,
            &boolean_value_of,
            sloppy_set_this,
        )),
        plain_value(global_value(
            &runtime,
            &mut context,
            &global,
            "sloppySetValue",
        )),
    ];
    observations.push(format!("accessors={}", accessor_values.join("|")));

    define_global(&runtime, &global, "getterHit", Value::Bool(false));
    define_global(&runtime, &global, "deleteHit", Value::Bool(false));
    define_data(
        &runtime,
        &boolean_prototype,
        "__rw",
        Value::Int(1),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &boolean_prototype,
        "__ro",
        Value::Int(1),
        false,
        false,
        true,
    );
    let getter_only = eval_callable(
        &runtime,
        &mut context,
        "(function() { getterHit = true; return 1; })",
    );
    let delete_getter = eval_callable(
        &runtime,
        &mut context,
        "(function() { deleteHit = true; return 1; })",
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "__getterOnly",
        Some(getter_only),
        None,
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "__delete",
        Some(delete_getter),
        None,
    );
    let sloppy_rw = context.eval("(false).__rw = 2").unwrap();
    let rw_after = context.eval("(false).__rw").unwrap();
    let strict_rw = observe_eval(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; return (false).__rw = 3; })()"#,
    );
    let sloppy_ro = context.eval("(false).__ro = 2").unwrap();
    let ro_after = context.eval("(false).__ro").unwrap();
    let strict_ro = observe_eval(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; return (false).__ro = 3; })()"#,
    );
    let sloppy_getter_only = context.eval("(false).__getterOnly = 2").unwrap();
    let strict_getter_only = observe_eval(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; return (false).__getterOnly = 3; })()"#,
    );
    let deleted = context.eval("delete (false).__delete").unwrap();
    let delete_key = runtime.intern_property_key("__delete").unwrap();
    let writes = [
        plain_value(sloppy_rw),
        plain_value(rw_after),
        strict_rw,
        plain_value(sloppy_ro),
        plain_value(ro_after),
        strict_ro,
        plain_value(sloppy_getter_only),
        strict_getter_only,
        plain_value(global_value(&runtime, &mut context, &global, "getterHit")),
        plain_value(deleted),
        plain_value(global_value(&runtime, &mut context, &global, "deleteHit")),
        runtime
            .has_own_property(&boolean_prototype, &delete_key)
            .unwrap()
            .to_string(),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function() { "use strict"; return delete (false).__delete; })()"#,
        ),
    ];
    observations.push(format!("writes={}", writes.join("|")));

    define_global(&runtime, &global, "tagThis", Value::Undefined);
    define_global(&runtime, &global, "localeGetterThis", Value::Undefined);
    define_global(&runtime, &global, "localeCallThis", Value::Undefined);
    let tag_getter = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; tagThis = this; return "CustomBoolean"; })"#,
    );
    define_accessor_key(
        &runtime,
        &boolean_prototype,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag)),
        Some(tag_getter),
        None,
    );
    let locale_method = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; localeCallThis = this; return this; })"#,
    );
    define_global(
        &runtime,
        &global,
        "localeMethod",
        Value::Object(locale_method.as_object().clone()),
    );
    let locale_getter = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; localeGetterThis = this; return localeMethod; })"#,
    );
    define_accessor(
        &runtime,
        &boolean_prototype,
        "toString",
        Some(locale_getter),
        None,
    );
    let custom_tag = context
        .call(&object_to_string, Value::Bool(false), &[])
        .unwrap();
    let tag_this = global_value(&runtime, &mut context, &global, "tagThis");
    let custom_locale = context
        .call(&object_to_locale_string, Value::Bool(false), &[])
        .unwrap();
    let custom = [
        plain_value(custom_tag),
        "object".to_owned(),
        object_has_prototype(&runtime, &tag_this, &boolean_prototype).to_string(),
        plain_value(unbox_boolean(&mut context, &boolean_value_of, tag_this)),
        plain_value(custom_locale),
        matches!(
            global_value(&runtime, &mut context, &global, "localeGetterThis"),
            Value::Bool(false)
        )
        .to_string(),
        matches!(
            global_value(&runtime, &mut context, &global, "localeCallThis"),
            Value::Bool(false)
        )
        .to_string(),
    ];
    observations.push(format!("custom-object-methods={}", custom.join("|")));

    observations
}

fn define_global(runtime: &Runtime, global: &ObjectRef, name: &str, value: Value) -> PropertyKey {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(runtime, global, &key, value, true, true, true);
    key
}

fn define_data(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(
        runtime,
        object,
        &key,
        value,
        writable,
        enumerable,
        configurable,
    );
}

fn define_data_key(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(writable),
                    enumerable: DescriptorField::Present(enumerable),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "host data-property definition was rejected"
    );
}

fn define_accessor(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
    get: Option<CallableRef>,
    set: Option<CallableRef>,
) {
    let key = runtime.intern_property_key(name).unwrap();
    define_accessor_key(runtime, object, &key, get, set);
}

fn define_accessor_key(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
    get: Option<CallableRef>,
    set: Option<CallableRef>,
) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(
                        get.map_or(AccessorValue::Undefined, AccessorValue::Callable),
                    ),
                    set: DescriptorField::Present(
                        set.map_or(AccessorValue::Undefined, AccessorValue::Callable),
                    ),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "host accessor-property definition was rejected"
    );
}

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("callable source did not produce an object: {source:?}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("source did not produce a callable: {source:?}"))
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn global_value(runtime: &Runtime, context: &mut Context, global: &ObjectRef, name: &str) -> Value {
    context
        .get_property(global, &runtime.intern_property_key(name).unwrap())
        .unwrap()
}

fn unbox_boolean(context: &mut Context, value_of: &CallableRef, this_value: Value) -> Value {
    context.call(value_of, this_value, &[]).unwrap()
}

fn object_has_prototype(runtime: &Runtime, value: &Value, prototype: &ObjectRef) -> bool {
    let Value::Object(object) = value else {
        return false;
    };
    runtime
        .get_prototype_of(object)
        .unwrap()
        .is_some_and(|actual| actual == *prototype)
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
    };
    object
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn data_flags(runtime: &Runtime, object: &ObjectRef, name: &str) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let descriptor = runtime
        .get_own_property(object, &key)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} descriptor was absent"));
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    } = descriptor
    else {
        panic!("{name} descriptor was not data");
    };
    format!(
        "{}{}{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn observe_eval(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => {
            panic!("Rust source failed outside JavaScript completion: {source:?}: {error}")
        }
    }
}

fn observe_call(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
) -> String {
    match context.call(callable, this_value, &[]) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust call failed outside JavaScript completion: {error}"),
    }
}

fn take_error(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Boolean operation did not throw an Error object");
    };
    let name = error_text(runtime, context, &error, "name");
    let message = error_text(runtime, context, &error, "message");
    format!("throw:{name}:{message}")
}

fn error_text(runtime: &Runtime, context: &mut Context, error: &ObjectRef, name: &str) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context.get_property(error, &key).unwrap() else {
        panic!("Error.{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn join_values(values: &[Value]) -> String {
    values
        .iter()
        .cloned()
        .map(plain_value)
        .collect::<Vec<_>>()
        .join("|")
}

fn plain_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_nan() => "NaN".to_owned(),
        Value::Float(0.0) => "0".to_owned(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) => "Symbol(boolean)".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Boolean intrinsic oracle");
    assert!(
        output.status.success(),
        "QuickJS Boolean intrinsic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Boolean intrinsic oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
