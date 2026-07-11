use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value,
    WellKnownSymbol,
};

// Deliberate partial-surface probe: key order is compared only after filtering
// upstream to the three String keys implemented by this milestone. Every
// observed JavaScript string is emitted as UTF-16 code-unit hex so lone
// surrogates and error text survive the stdout boundary losslessly.
const ORACLE_PROBE: &str = r#"
function hex(value) {
    value = String(value);
    var out = "";
    for (var i = 0; i < value.length; i++)
        out += ("0000" + value.charCodeAt(i).toString(16)).slice(-4);
    return out;
}
function flags(object, key) {
    var d = Object.getOwnPropertyDescriptor(object, key);
    return (d.writable ? "1" : "0") + (d.enumerable ? "1" : "0") +
           (d.configurable ? "1" : "0");
}
function signature(fn) {
    return hex(fn.name) + ":" + fn.length + ":" +
           Reflect.ownKeys(fn).map(hex).join(",");
}
function constructable(fn) {
    try { Reflect.construct(function () {}, [], fn); return true; }
    catch (_) { return false; }
}
function observe(thunk) {
    try { return "ok:" + hex(thunk()); }
    catch (error) { return "throw:" + hex(error.name) + ":" + hex(error.message); }
}

var source = "A\ud800B";
var ts = String.prototype.toString;
var vo = String.prototype.valueOf;
var filtered = Reflect.ownKeys(String.prototype).filter(function (key) {
    return key === "length" || key === "toString" || key === "valueOf";
});
print("surface=" + filtered.map(hex).join(",") + "|" +
      [flags(String.prototype, "length"), flags(String.prototype, "toString"),
       flags(String.prototype, "valueOf")].join("|"));
print("metadata=" + signature(ts) + "|" + signature(vo) + "|" +
      (ts === vo) + "|" + constructable(ts) + "|" + constructable(vo));

var wrapper = Object.prototype.valueOf.call(source);
var spoof = Object.create(String.prototype);
var changed = Object.prototype.valueOf.call(source);
Object.setPrototypeOf(changed, Object.prototype);
print("brand=" + [hex(ts.call(source)), hex(vo.call(wrapper)),
      hex(ts.call(String.prototype)), observe(function () { return ts.call(spoof); }),
      hex(vo.call(changed)), hex(Object.prototype.toString.call(changed))].join("|"));

var ordinary = Object.prototype.valueOf.call(source);
ordinary.toString = function () { return "override"; };
ordinary.valueOf = function () { return 7; };
print("override=" + [hex(ts.call(ordinary)), hex(vo.call(ordinary)),
      hex(decodeURI(ordinary)), ordinary + 1, +ordinary].join("|"));

var strictGetThis;
Object.defineProperty(String.prototype, "__strictGet", {
    configurable: true,
    get: function () { "use strict"; strictGetThis = this; return this; }
});
var getResult = source.__strictGet;
print("get=" + [hex(getResult), strictGetThis === source].join("|"));

var strictSetThis, strictSetValue;
Object.defineProperty(String.prototype, "__strictSet", {
    configurable: true,
    set: function (value) { "use strict"; strictSetThis = this; strictSetValue = value; }
});
var setResult = source.__strictSet = 7;
print("set=" + [setResult, strictSetThis === source, strictSetValue,
      observe(function () { "use strict"; return source.__missing = 1; }),
      observe(function () { "use strict"; return source.length = 1; })].join("|"));

var boxA = Object.prototype.valueOf.call(source);
var boxB = Object.prototype.valueOf.call(source);
print("object=" + [hex(Object.prototype.toString.call(source)),
      hex(Object.prototype.toLocaleString.call(source)), boxA !== boxB,
      Object.getPrototypeOf(boxA) === String.prototype, hex(vo.call(boxA)),
      Reflect.ownKeys(boxA).map(hex).join(","), flags(boxA, "length")].join("|"));

Object.defineProperty(String.prototype, Symbol.toStringTag, {
    configurable: true, value: "CustomString"
});
var customTag = Object.prototype.toString.call(source);
Object.defineProperty(String.prototype, Symbol.toStringTag, { value: 1 });
var nonStringTag = Object.prototype.toString.call(source);
delete String.prototype[Symbol.toStringTag];
var deletedTag = Object.prototype.toString.call(source);
print("tags=" + [hex(customTag), hex(nonStringTag), hex(deletedTag)].join("|"));

var tagThis;
Object.defineProperty(String.prototype, Symbol.toStringTag, {
    configurable: true,
    get: function () { "use strict"; tagThis = this; throw new RangeError("tag"); }
});
var tagThrow = observe(function () { return Object.prototype.toString.call(source); });
delete String.prototype[Symbol.toStringTag];
print("tag-throw=" + [tagThrow, hex(vo.call(tagThis)),
      Object.getPrototypeOf(tagThis) === String.prototype].join("|"));
"#;

#[test]
fn string_conversion_core_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String conversion-core differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_observations(),
        oracle_observations(&oracle),
        "String conversion-core behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let prototype = context.string_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let ts = property_callable(&runtime, &mut context, &prototype, "toString");
    let vo = property_callable(&runtime, &mut context, &prototype, "valueOf");
    let ots = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let ols = property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let ovo = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let source = JsString::from_utf16([0x41, 0xd800, 0x42]);
    let mut out = Vec::with_capacity(9);

    let implemented = own_key_names(&runtime, &prototype)
        .into_iter()
        .filter(|key| matches!(key.as_str(), "length" | "toString" | "valueOf"))
        .map(|key| hex(&JsString::from(key.as_str())))
        .collect::<Vec<_>>()
        .join(",");
    out.push(format!(
        "surface={implemented}|{}|{}|{}",
        data_flags(&runtime, &prototype, "length"),
        data_flags(&runtime, &prototype, "toString"),
        data_flags(&runtime, &prototype, "valueOf")
    ));
    out.push(format!(
        "metadata={}|{}|{}|{}|{}",
        signature(&runtime, &mut context, &ts),
        signature(&runtime, &mut context, &vo),
        ts == vo,
        runtime.is_constructor(ts.as_object()).unwrap(),
        runtime.is_constructor(vo.as_object()).unwrap()
    ));

    let wrapper = box_string(&mut context, &ovo, source.clone());
    let spoof = context.new_object().unwrap();
    assert!(runtime.set_prototype_of(&spoof, Some(&prototype)).unwrap());
    let changed = box_string(&mut context, &ovo, source.clone());
    assert!(
        runtime
            .set_prototype_of(&changed, Some(&object_prototype))
            .unwrap()
    );
    out.push(format!(
        "brand={}|{}|{}|{}|{}|{}",
        call_string(&mut context, &ts, Value::String(source.clone())),
        call_string(&mut context, &vo, Value::Object(wrapper)),
        call_string(&mut context, &ts, Value::Object(prototype.clone())),
        observe_call(&runtime, &mut context, &ts, Value::Object(spoof)),
        call_string(&mut context, &vo, Value::Object(changed.clone())),
        call_string(&mut context, &ots, Value::Object(changed))
    ));

    let ordinary = box_string(&mut context, &ovo, source.clone());
    define_data(
        &runtime,
        &ordinary,
        "toString",
        Value::Object(
            eval_callable(&runtime, &mut context, "(function(){return 'override';})")
                .as_object()
                .clone(),
        ),
    );
    define_data(
        &runtime,
        &ordinary,
        "valueOf",
        Value::Object(
            eval_callable(&runtime, &mut context, "(function(){return 7;})")
                .as_object()
                .clone(),
        ),
    );
    define_data(
        &runtime,
        &global,
        "conversionWrapper",
        Value::Object(ordinary.clone()),
    );
    out.push(format!(
        "override={}|{}|{}|{}|{}",
        call_string(&mut context, &ts, Value::Object(ordinary.clone())),
        call_string(&mut context, &vo, Value::Object(ordinary)),
        eval_value(&mut context, "decodeURI(conversionWrapper)"),
        eval_value(&mut context, "conversionWrapper + 1"),
        eval_value(&mut context, "+conversionWrapper")
    ));

    define_data(&runtime, &global, "strictGetThis", Value::Undefined);
    let strict_get = eval_callable(
        &runtime,
        &mut context,
        r#"(function(){"use strict";strictGetThis=this;return this;})"#,
    );
    define_accessor(&runtime, &prototype, "__strictGet", Some(strict_get), None);
    let get_result = context.eval("'A\\ud800B'.__strictGet").unwrap();
    let get_this = global_value(&runtime, &mut context, &global, "strictGetThis");
    out.push(format!(
        "get={}|{}",
        encoded_value(get_result),
        get_this == Value::String(source.clone())
    ));

    define_data(&runtime, &global, "strictSetThis", Value::Undefined);
    define_data(&runtime, &global, "strictSetValue", Value::Undefined);
    let strict_set = eval_callable(
        &runtime,
        &mut context,
        r#"(function(value){"use strict";strictSetThis=this;strictSetValue=value;})"#,
    );
    define_accessor(&runtime, &prototype, "__strictSet", None, Some(strict_set));
    let set_result = context.eval("'A\\ud800B'.__strictSet = 7").unwrap();
    let set_this = global_value(&runtime, &mut context, &global, "strictSetThis");
    let set_value = global_value(&runtime, &mut context, &global, "strictSetValue");
    out.push(format!(
        "set={}|{}|{}|{}|{}",
        encoded_value(set_result),
        set_this == Value::String(source.clone()),
        encoded_value(set_value),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function(){"use strict";return ('A\ud800B').__missing=1;})()"#,
        ),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function(){"use strict";return ('A\ud800B').length=1;})()"#,
        )
    ));

    let box_a = box_string(&mut context, &ovo, source.clone());
    let box_b = box_string(&mut context, &ovo, source.clone());
    out.push(format!(
        "object={}|{}|{}|{}|{}|{}|{}",
        call_string(&mut context, &ots, Value::String(source.clone())),
        call_string(&mut context, &ols, Value::String(source.clone())),
        box_a != box_b,
        runtime.get_prototype_of(&box_a).unwrap() == Some(prototype.clone()),
        call_string(&mut context, &vo, Value::Object(box_a.clone())),
        own_key_names(&runtime, &box_a)
            .into_iter()
            .map(|key| hex(&JsString::from(key.as_str())))
            .collect::<Vec<_>>()
            .join(","),
        data_flags(&runtime, &box_a, "length")
    ));

    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    define_data_key(
        &runtime,
        &prototype,
        &tag,
        Value::String(JsString::from("CustomString")),
    );
    let custom = call_string(&mut context, &ots, Value::String(source.clone()));
    define_data_key(&runtime, &prototype, &tag, Value::Int(1));
    let non_string = call_string(&mut context, &ots, Value::String(source.clone()));
    assert!(runtime.delete_property(&prototype, &tag).unwrap());
    let deleted = call_string(&mut context, &ots, Value::String(source.clone()));
    out.push(format!("tags={custom}|{non_string}|{deleted}"));

    define_data(&runtime, &global, "tagThis", Value::Undefined);
    let tag_getter = eval_callable(
        &runtime,
        &mut context,
        r#"(function(){"use strict";tagThis=this;throw new RangeError('tag');})"#,
    );
    define_accessor_key(&runtime, &prototype, &tag, Some(tag_getter), None);
    let tag_throw = observe_call(&runtime, &mut context, &ots, Value::String(source.clone()));
    let Value::Object(tag_this) = global_value(&runtime, &mut context, &global, "tagThis") else {
        panic!("@@toStringTag getter did not capture the temporary String wrapper");
    };
    assert!(runtime.delete_property(&prototype, &tag).unwrap());
    out.push(format!(
        "tag-throw={tag_throw}|{}|{}",
        call_string(&mut context, &vo, Value::Object(tag_this.clone())),
        runtime.get_prototype_of(&tag_this).unwrap() == Some(prototype)
    ));

    out
}

#[test]
fn string_conversion_core_cross_realm_and_error_realm_match_quickjs() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_string = first.string_prototype().unwrap();
    let second_string = second.string_prototype().unwrap();
    let first_object = first.object_prototype().unwrap();
    let first_vo = property_callable(&runtime, &mut first, &first_string, "valueOf");
    let second_vo = property_callable(&runtime, &mut second, &second_string, "valueOf");
    let first_ovo = property_callable(&runtime, &mut first, &first_object, "valueOf");
    let first_ots = property_callable(&runtime, &mut first, &first_object, "toString");
    let source = JsString::from_utf16([0x41, 0xd800, 0x42]);

    let wrapper = box_string(&mut second, &first_ovo, source.clone());
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(first_string.clone())
    );
    assert_eq!(
        second
            .call(&second_vo, Value::Object(wrapper), &[])
            .unwrap(),
        Value::String(source)
    );

    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    define_data_key(
        &runtime,
        &first_string,
        &tag,
        Value::String(JsString::from("FirstString")),
    );
    define_data_key(
        &runtime,
        &second_string,
        &tag,
        Value::String(JsString::from("SecondString")),
    );
    assert_eq!(
        second
            .call(&first_ots, Value::String(JsString::from("x")), &[])
            .unwrap(),
        Value::String(JsString::from("[object FirstString]"))
    );

    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let spoof = second.new_object().unwrap();
    assert_eq!(
        second.call(&first_vo, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    );
    let error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(first_type_error)
    );
}

#[test]
fn string_conversion_core_wrapper_realm_graph_is_collectable() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        let object = context.object_prototype().unwrap();
        let ovo = property_callable(&runtime, &mut context, &object, "valueOf");
        box_string(
            &mut context,
            &ovo,
            JsString::from_utf16([0x41, 0xd800, 0x42]),
        )
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    assert_eq!(own_key_names(&runtime, &wrapper), ["0", "1", "2", "length"]);
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn box_string(context: &mut Context, ovo: &CallableRef, value: JsString) -> ObjectRef {
    let Value::Object(object) = context.call(ovo, Value::String(value), &[]).unwrap() else {
        panic!("Object.prototype.valueOf did not box String");
    };
    object
}

fn call_string(context: &mut Context, callable: &CallableRef, this_value: Value) -> String {
    let Value::String(value) = context.call(callable, this_value, &[]).unwrap() else {
        panic!("String observation did not return a String");
    };
    hex(&value)
}

fn signature(runtime: &Runtime, context: &mut Context, callable: &CallableRef) -> String {
    let Value::String(name) = context
        .get_property(
            callable.as_object(),
            &runtime.intern_property_key("name").unwrap(),
        )
        .unwrap()
    else {
        panic!("function name was not a String");
    };
    let Value::Int(length) = context
        .get_property(
            callable.as_object(),
            &runtime.intern_property_key("length").unwrap(),
        )
        .unwrap()
    else {
        panic!("function length was not an Int32");
    };
    format!(
        "{}:{length}:{}",
        hex(&name),
        own_key_names(runtime, callable.as_object())
            .into_iter()
            .map(|key| hex(&JsString::from(key.as_str())))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(runtime, object, &key, value);
}

fn define_data_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
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
            .unwrap()
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
    let descriptor = runtime
        .get_own_property(object, &runtime.intern_property_key(name).unwrap())
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

fn observe_call(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
) -> String {
    match context.call(callable, this_value, &[]) {
        Ok(value) => format!("ok:{}", encoded_value(value)),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("call failed outside JavaScript completion: {error}"),
    }
}

fn observe_eval(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => format!("ok:{}", encoded_value(value)),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("source failed outside JavaScript completion: {source:?}: {error}"),
    }
}

fn eval_value(context: &mut Context, source: &str) -> String {
    encoded_value(context.eval(source).unwrap())
}

fn encoded_value(value: Value) -> String {
    match value {
        Value::String(value) => hex(&value),
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::Symbol(_) => panic!("unexpected Symbol observation"),
        Value::Object(_) => panic!("unexpected Object observation"),
    }
}

fn take_error(runtime: &Runtime, context: &mut Context) -> String {
    let error = take_exception_object(context);
    let name = error_string(runtime, context, &error, "name");
    let message = error_string(runtime, context, &error, "message");
    format!("throw:{}:{}", hex(&name), hex(&message))
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("operation did not throw an Error object");
    };
    error
}

fn error_string(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
) -> JsString {
    let Value::String(value) = context
        .get_property(error, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("Error.{name} was not a String");
    };
    value
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let global = context.global_object().unwrap();
    let constructor = property_callable(runtime, context, &global, name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn hex(value: &JsString) -> String {
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect()
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS String conversion-core oracle");
    assert!(
        output.status.success(),
        "QuickJS String conversion-core oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS String conversion-core oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
