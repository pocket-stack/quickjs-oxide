use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField, JsBigInt, JsString,
    ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value,
    WellKnownSymbol,
};

// The global String constructor and the full prototype table are intentionally
// outside this milestone. The surface observation therefore filters pinned
// QuickJS to the implemented keys. Every observed JavaScript string, including
// conversion logs and errors, crosses stdout as UTF-16 code-unit hex.
const ORACLE_PROBE: &str = r#"
function hex(value) {
    value = String(value);
    var out = "";
    for (var i = 0; i < value.length; i++)
        out += ("0000" + value.charCodeAt(i).toString(16)).slice(-4);
    return out;
}
function render(value) {
    if (value === undefined) return "u";
    if (typeof value === "number" && Number.isNaN(value)) return "nan";
    if (typeof value === "string") return "s:" + hex(value);
    return String(value);
}
function flags(object, key) {
    var d = Object.getOwnPropertyDescriptor(object, key);
    return (d.writable ? "1" : "0") + (d.enumerable ? "1" : "0") +
           (d.configurable ? "1" : "0");
}
function signature(fn) {
    return hex(fn.name) + ":" + fn.length + ":" +
           Reflect.ownKeys(fn).map(hex).join(",") + ":" + constructable(fn);
}
function constructable(fn) {
    try { Reflect.construct(function () {}, [], fn); return true; }
    catch (_) { return false; }
}
function observe(thunk) {
    try { return "ok:" + render(thunk()); }
    catch (error) { return "throw:" + hex(error.name) + ":" + hex(error.message); }
}

var names = ["length", "at", "charCodeAt", "charAt", "concat", "codePointAt",
             "isWellFormed", "toWellFormed", "toString", "valueOf"];
var methods = names.slice(1, 8);
var filtered = Reflect.ownKeys(String.prototype).filter(function (key) {
    return names.indexOf(key) >= 0;
});
print("surface=" + filtered.map(hex).join(",") + "|" +
      names.slice(0, 8).map(function (name) { return flags(String.prototype, name); }).join("|"));
print("metadata=" + methods.map(function (name) {
    return signature(String.prototype[name]);
}).join("|"));

var source = "A\ud83d\ude00\ud800Z";
var indices = [undefined, NaN, -0, 0, 1, 2, 3, 4, 5, -1, -2, -5, -6,
               1.9, -1.9, Infinity, -Infinity, 2147483648, -2147483649];
print("at=" + indices.map(function (index) { return render(String.prototype.at.call(source, index)); }).join("|"));
print("charAt=" + indices.map(function (index) { return render(String.prototype.charAt.call(source, index)); }).join("|"));
print("charCodeAt=" + indices.map(function (index) { return render(String.prototype.charCodeAt.call(source, index)); }).join("|"));
print("codePointAt=" + indices.map(function (index) { return render(String.prototype.codePointAt.call(source, index)); }).join("|"));

var conversionLog = "";
var receiver = {};
Object.defineProperty(receiver, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { conversionLog += "r:" + hint + ","; return source; }
});
var indexObject = {};
Object.defineProperty(indexObject, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { conversionLog += "i:" + hint + ","; return 1.9; }
});
var ordered = String.prototype.at.call(receiver, indexObject);
print("order=" + hex(conversionLog) + "|" + render(ordered));

var genericWrapper = Object.prototype.valueOf.call("xy");
print("generic=" + [render(String.prototype.at.call("x")),
      render(String.prototype.at.call(123, 1)),
      render(String.prototype.charAt.call(true, 0)),
      render(String.prototype.codePointAt.call(1n, 0)),
      render(String.prototype.at.call(genericWrapper, -1))].join("|"));

var concatLog = "";
var fallback = {};
var concatReceiver = {
    toString: function () { concatLog += "rt,"; return fallback; },
    valueOf: function () { concatLog += "rv,"; return "R"; }
};
var argA = { toString: function () { concatLog += "at,"; return "A"; } };
var argB = {
    toString: function () { concatLog += "bt,"; return fallback; },
    valueOf: function () { concatLog += "bv,"; return "B"; }
};
var argC = { toString: function () { concatLog += "c,"; throw new RangeError("C"); } };
var argD = { toString: function () { concatLog += "d,"; return "D"; } };
var noArgs = String.prototype.concat.call("R");
var concatenated = String.prototype.concat.call(concatReceiver, argA, argB);
var successLog = concatLog;
concatLog = "";
var thrown = observe(function () {
    return String.prototype.concat.call(concatReceiver, argA, argC, argD);
});
print("concat=" + [render(noArgs), render(concatenated), hex(successLog),
      thrown, hex(concatLog)].join("|"));

var wellInputs = ["", "abc", "\ud83d\ude00", "\ud800", "\udc00",
                  "\udc00\ud800", "A\ud800B\udc00C", "\ud800\udc00\ud800\udc00"];
var well = wellInputs.map(function (value) {
    return String.prototype.isWellFormed.call(value) + ":" +
           hex(String.prototype.toWellFormed.call(value));
}).join("|");
var wellLog = "";
var wellReceiver = { toString: function () { wellLog += "r,"; return "\ud800X"; } };
var ignored = { toString: function () { wellLog += "arg,"; throw 1; } };
var wellBoolean = String.prototype.isWellFormed.call(wellReceiver, ignored);
var wellString = String.prototype.toWellFormed.call(wellReceiver, ignored);
print("well=" + well + "|ignored:" + wellBoolean + ":" + hex(wellString) + ":" + hex(wellLog));

print("errors=" + [
    observe(function () { return String.prototype.at.call(null, 0); }),
    observe(function () { return String.prototype.at.call(undefined, 0); }),
    observe(function () { return String.prototype.at.call(Symbol("receiver"), 0); }),
    observe(function () { return String.prototype.at.call("x", 1n); }),
    observe(function () { return String.prototype.at.call("x", Symbol("index")); }),
    observe(function () { return String.prototype.concat.call("x", Symbol("argument")); })
].join("|"));
"#;

#[test]
fn string_utf16_prefix_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String UTF-16 prefix differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let rust = rust_observations();
    let upstream = oracle_observations(&oracle);
    assert_eq!(rust.len(), 11, "Rust probe breadth changed unexpectedly");
    assert_eq!(
        upstream.len(),
        11,
        "QuickJS probe breadth changed unexpectedly"
    );
    assert_eq!(
        rust, upstream,
        "String UTF-16 prefix behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let prototype = context.string_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let at = property_callable(&runtime, &mut context, &prototype, "at");
    let char_at = property_callable(&runtime, &mut context, &prototype, "charAt");
    let char_code_at = property_callable(&runtime, &mut context, &prototype, "charCodeAt");
    let concat = property_callable(&runtime, &mut context, &prototype, "concat");
    let code_point_at = property_callable(&runtime, &mut context, &prototype, "codePointAt");
    let is_well_formed = property_callable(&runtime, &mut context, &prototype, "isWellFormed");
    let to_well_formed = property_callable(&runtime, &mut context, &prototype, "toWellFormed");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let method_table = [
        ("at", &at),
        ("charCodeAt", &char_code_at),
        ("charAt", &char_at),
        ("concat", &concat),
        ("codePointAt", &code_point_at),
        ("isWellFormed", &is_well_formed),
        ("toWellFormed", &to_well_formed),
    ];
    let implemented = [
        "length",
        "at",
        "charCodeAt",
        "charAt",
        "concat",
        "codePointAt",
        "isWellFormed",
        "toWellFormed",
        "toString",
        "valueOf",
    ];
    let filtered = own_key_names(&runtime, &prototype)
        .into_iter()
        .filter(|key| implemented.contains(&key.as_str()))
        .map(|key| hex(&JsString::from(key.as_str())))
        .collect::<Vec<_>>()
        .join(",");
    let flags = implemented[..8]
        .iter()
        .map(|name| data_flags(&runtime, &prototype, name))
        .collect::<Vec<_>>()
        .join("|");
    let mut out = vec![format!("surface={filtered}|{flags}")];
    out.push(format!(
        "metadata={}",
        method_table
            .iter()
            .map(|(_, method)| signature(&runtime, &mut context, method))
            .collect::<Vec<_>>()
            .join("|")
    ));

    let source = JsString::from_utf16([0x41, 0xd83d, 0xde00, 0xd800, 0x5a]);
    let indices = [
        Value::Undefined,
        Value::Float(f64::NAN),
        Value::Float(-0.0),
        Value::Int(0),
        Value::Int(1),
        Value::Int(2),
        Value::Int(3),
        Value::Int(4),
        Value::Int(5),
        Value::Int(-1),
        Value::Int(-2),
        Value::Int(-5),
        Value::Int(-6),
        Value::Float(1.9),
        Value::Float(-1.9),
        Value::Float(f64::INFINITY),
        Value::Float(f64::NEG_INFINITY),
        Value::Float(2_147_483_648.0),
        Value::Float(-2_147_483_649.0),
    ];
    for (name, method) in [
        ("at", &at),
        ("charAt", &char_at),
        ("charCodeAt", &char_code_at),
        ("codePointAt", &code_point_at),
    ] {
        let values = indices
            .iter()
            .cloned()
            .map(|index| {
                render(
                    context
                        .call(method, Value::String(source.clone()), &[index])
                        .unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        out.push(format!("{name}={values}"));
    }

    define_data(
        &runtime,
        &global,
        "conversionLog",
        Value::String(JsString::from("")),
    );
    let receiver = context.new_object().unwrap();
    let receiver_conversion = eval_callable(
        &runtime,
        &mut context,
        r#"(function(hint){conversionLog=conversionLog+'r:'+hint+',';return 'A\ud83d\ude00\ud800Z';})"#,
    );
    define_to_primitive(&runtime, &receiver, receiver_conversion);
    let index_object = context.new_object().unwrap();
    let index_conversion = eval_callable(
        &runtime,
        &mut context,
        r#"(function(hint){conversionLog=conversionLog+'i:'+hint+',';return 1.9;})"#,
    );
    define_to_primitive(&runtime, &index_object, index_conversion);
    let ordered = context
        .call(&at, Value::Object(receiver), &[Value::Object(index_object)])
        .unwrap();
    let conversion_log = expect_string(global_value(
        &runtime,
        &mut context,
        &global,
        "conversionLog",
    ));
    out.push(format!(
        "order={}|{}",
        hex(&conversion_log),
        render(ordered)
    ));

    let Value::Object(generic_wrapper) = context
        .call(&object_value_of, Value::String(JsString::from("xy")), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box String");
    };
    let missing_index = context
        .call(&at, Value::String(JsString::from("x")), &[])
        .unwrap();
    let generic_number = context
        .call(&at, Value::Int(123), &[Value::Int(1)])
        .unwrap();
    let generic_boolean = context
        .call(&char_at, Value::Bool(true), &[Value::Int(0)])
        .unwrap();
    let generic_bigint = context
        .call(
            &code_point_at,
            Value::BigInt(JsBigInt::one()),
            &[Value::Int(0)],
        )
        .unwrap();
    let generic_wrapper = context
        .call(&at, Value::Object(generic_wrapper), &[Value::Int(-1)])
        .unwrap();
    out.push(format!(
        "generic={}|{}|{}|{}|{}",
        render(missing_index),
        render(generic_number),
        render(generic_boolean),
        render(generic_bigint),
        render(generic_wrapper)
    ));

    define_data(
        &runtime,
        &global,
        "concatLog",
        Value::String(JsString::from("")),
    );
    let fallback = context.new_object().unwrap();
    define_data(&runtime, &global, "concatFallback", Value::Object(fallback));
    let concat_receiver = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &concat_receiver,
        "toString",
        "(function(){concatLog=concatLog+'rt,';return concatFallback;})",
    );
    define_method(
        &runtime,
        &mut context,
        &concat_receiver,
        "valueOf",
        "(function(){concatLog=concatLog+'rv,';return 'R';})",
    );
    let arg_a = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &arg_a,
        "toString",
        "(function(){concatLog=concatLog+'at,';return 'A';})",
    );
    let arg_b = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &arg_b,
        "toString",
        "(function(){concatLog=concatLog+'bt,';return concatFallback;})",
    );
    define_method(
        &runtime,
        &mut context,
        &arg_b,
        "valueOf",
        "(function(){concatLog=concatLog+'bv,';return 'B';})",
    );
    let arg_c = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &arg_c,
        "toString",
        "(function(){concatLog=concatLog+'c,';throw new RangeError('C');})",
    );
    let arg_d = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &arg_d,
        "toString",
        "(function(){concatLog=concatLog+'d,';return 'D';})",
    );
    let no_args = context
        .call(&concat, Value::String(JsString::from("R")), &[])
        .unwrap();
    let concatenated = context
        .call(
            &concat,
            Value::Object(concat_receiver.clone()),
            &[Value::Object(arg_a.clone()), Value::Object(arg_b)],
        )
        .unwrap();
    let success_log = expect_string(global_value(&runtime, &mut context, &global, "concatLog"));
    define_data(
        &runtime,
        &global,
        "concatLog",
        Value::String(JsString::from("")),
    );
    let thrown = observe_call_args(
        &runtime,
        &mut context,
        &concat,
        Value::Object(concat_receiver),
        &[
            Value::Object(arg_a),
            Value::Object(arg_c),
            Value::Object(arg_d),
        ],
    );
    let throw_log = expect_string(global_value(&runtime, &mut context, &global, "concatLog"));
    out.push(format!(
        "concat={}|{}|{}|{}|{}",
        render(no_args),
        render(concatenated),
        hex(&success_log),
        thrown,
        hex(&throw_log)
    ));

    let well_inputs = [
        JsString::from(""),
        JsString::from("abc"),
        JsString::from_utf16([0xd83d, 0xde00]),
        JsString::from_utf16([0xd800]),
        JsString::from_utf16([0xdc00]),
        JsString::from_utf16([0xdc00, 0xd800]),
        JsString::from_utf16([0x41, 0xd800, 0x42, 0xdc00, 0x43]),
        JsString::from_utf16([0xd800, 0xdc00, 0xd800, 0xdc00]),
    ];
    let mut well = Vec::with_capacity(well_inputs.len());
    for input in well_inputs {
        let Value::Bool(valid) = context
            .call(&is_well_formed, Value::String(input.clone()), &[])
            .unwrap()
        else {
            panic!("isWellFormed did not return Boolean");
        };
        let output = call_string(&mut context, &to_well_formed, Value::String(input), &[]);
        well.push(format!("{valid}:{}", hex(&output)));
    }
    define_data(
        &runtime,
        &global,
        "wellLog",
        Value::String(JsString::from("")),
    );
    let well_receiver = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &well_receiver,
        "toString",
        r#"(function(){wellLog=wellLog+'r,';return '\ud800X';})"#,
    );
    let ignored = context.new_object().unwrap();
    define_method(
        &runtime,
        &mut context,
        &ignored,
        "toString",
        "(function(){wellLog=wellLog+'arg,';throw 1;})",
    );
    let Value::Bool(well_boolean) = context
        .call(
            &is_well_formed,
            Value::Object(well_receiver.clone()),
            &[Value::Object(ignored.clone())],
        )
        .unwrap()
    else {
        panic!("isWellFormed did not return Boolean");
    };
    let well_string = call_string(
        &mut context,
        &to_well_formed,
        Value::Object(well_receiver),
        &[Value::Object(ignored)],
    );
    let well_log = expect_string(global_value(&runtime, &mut context, &global, "wellLog"));
    out.push(format!(
        "well={}|ignored:{well_boolean}:{}:{}",
        well.join("|"),
        hex(&well_string),
        hex(&well_log)
    ));

    let receiver_symbol = runtime
        .new_symbol(Some(JsString::from("receiver")))
        .unwrap();
    let index_symbol = runtime.new_symbol(Some(JsString::from("index"))).unwrap();
    let argument_symbol = runtime
        .new_symbol(Some(JsString::from("argument")))
        .unwrap();
    out.push(format!(
        "errors={}|{}|{}|{}|{}|{}",
        observe_call_args(&runtime, &mut context, &at, Value::Null, &[Value::Int(0)],),
        observe_call_args(
            &runtime,
            &mut context,
            &at,
            Value::Undefined,
            &[Value::Int(0)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &at,
            Value::Symbol(receiver_symbol),
            &[Value::Int(0)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &at,
            Value::String(JsString::from("x")),
            &[Value::BigInt(JsBigInt::one())],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &at,
            Value::String(JsString::from("x")),
            &[Value::Symbol(index_symbol)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &concat,
            Value::String(JsString::from("x")),
            &[Value::Symbol(argument_symbol)],
        )
    ));

    out
}

#[test]
fn string_utf16_prefix_cross_realm_and_error_realms_match_quickjs() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_prototype = first.string_prototype().unwrap();
    let at = property_callable(&runtime, &mut first, &first_prototype, "at");
    assert_eq!(
        second
            .call(
                &at,
                Value::String(JsString::from_utf16([0xd83d, 0xde00])),
                &[Value::Int(0)],
            )
            .unwrap(),
        Value::String(JsString::from_utf16([0xd83d]))
    );

    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    assert_eq!(
        second.call(
            &at,
            Value::String(JsString::from("x")),
            &[Value::BigInt(JsBigInt::one())],
        ),
        Err(RuntimeError::Exception)
    );
    let framework_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
        Some(first_type_error)
    );

    let second_range_error = intrinsic_prototype(&runtime, &mut second, "RangeError");
    let throwing_index = second.new_object().unwrap();
    let user_throw = eval_callable(
        &runtime,
        &mut second,
        "(function(){throw new RangeError('user');})",
    );
    define_to_primitive(&runtime, &throwing_index, user_throw);
    assert_eq!(
        first.call(
            &at,
            Value::String(JsString::from("x")),
            &[Value::Object(throwing_index)],
        ),
        Err(RuntimeError::Exception)
    );
    let user_error = take_exception_object(&mut first);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(second_range_error)
    );
}

#[test]
fn string_utf16_prefix_method_retains_then_releases_its_realm_graph() {
    let runtime = Runtime::new();
    let method = {
        let mut context = runtime.new_context();
        let prototype = context.string_prototype().unwrap();
        property_callable(&runtime, &mut context, &prototype, "at")
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(method);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn define_to_primitive(runtime: &Runtime, object: &ObjectRef, callable: CallableRef) {
    define_data_key(
        runtime,
        object,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(callable.as_object().clone()),
    );
}

fn define_method(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
    source: &str,
) {
    let callable = eval_callable(runtime, context, source);
    define_data(
        runtime,
        object,
        name,
        Value::Object(callable.as_object().clone()),
    );
}

fn call_string(
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> JsString {
    let Value::String(value) = context.call(callable, this_value, arguments).unwrap() else {
        panic!("String observation did not return a String");
    };
    value
}

fn signature(runtime: &Runtime, context: &mut Context, callable: &CallableRef) -> String {
    let name = expect_string(
        context
            .get_property(
                callable.as_object(),
                &runtime.intern_property_key("name").unwrap(),
            )
            .unwrap(),
    );
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
        "{}:{length}:{}:{}",
        hex(&name),
        own_key_names(runtime, callable.as_object())
            .into_iter()
            .map(|key| hex(&JsString::from(key.as_str())))
            .collect::<Vec<_>>()
            .join(","),
        runtime.is_constructor(callable.as_object()).unwrap()
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

fn observe_call_args(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> String {
    match context.call(callable, this_value, arguments) {
        Ok(value) => format!("ok:{}", render(value)),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("call failed outside JavaScript completion: {error}"),
    }
}

fn render(value: Value) -> String {
    match value {
        Value::Undefined => "u".to_owned(),
        Value::String(value) => format!("s:{}", hex(&value)),
        Value::Float(value) if value.is_nan() => "nan".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Null => "null".to_owned(),
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
    expect_string(
        context
            .get_property(error, &runtime.intern_property_key(name).unwrap())
            .unwrap(),
    )
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

fn expect_string(value: Value) -> JsString {
    let Value::String(value) = value else {
        panic!("value was not a String");
    };
    value
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
        .expect("run QuickJS String UTF-16 prefix oracle");
    assert!(
        output.status.success(),
        "QuickJS String UTF-16 prefix oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS String UTF-16 prefix oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
