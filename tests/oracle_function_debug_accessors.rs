use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DebugInfoMode, DescriptorField,
    JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, Value, WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function hex(value) {
    var out = "";
    for (var i = 0; i < value.length; i++)
        out += value.charCodeAt(i).toString(16).padStart(4, "0");
    return out;
}
function isConstructor(value) {
    try {
        Reflect.construct(function () {}, [], value);
        return true;
    } catch (_) {
        return false;
    }
}
var fp = Function.prototype;
var implementedNames = {
    length: true,
    name: true,
    caller: true,
    arguments: true,
    call: true,
    apply: true,
    bind: true,
    toString: true,
    fileName: true,
    lineNumber: true,
    columnNumber: true,
};
var implementedKeys = Reflect.ownKeys(fp).filter(function (key) {
    return key === Symbol.hasInstance ||
           (typeof key === "string" && implementedNames[key] === true);
});
print("fp-keys=" + implementedKeys.map(String).join(","));

var debugNames = ["fileName", "lineNumber", "columnNumber"];
var getters = debugNames.map(function (name) {
    var descriptor = Object.getOwnPropertyDescriptor(fp, name);
    var getter = descriptor.get;
    var lengthDescriptor = Object.getOwnPropertyDescriptor(getter, "length");
    var nameDescriptor = Object.getOwnPropertyDescriptor(getter, "name");
    print("getter-meta=" + name + ":" + typeof getter + ":" + String(descriptor.set) +
          ":" + Number(descriptor.enumerable) + "," + Number(descriptor.configurable) +
          "|length:" + getter.length + ":" + Number(lengthDescriptor.writable) + "," +
          Number(lengthDescriptor.enumerable) + "," + Number(lengthDescriptor.configurable) +
          "|name:" + getter.name + ":" + Number(nameDescriptor.writable) + "," +
          Number(nameDescriptor.enumerable) + "," + Number(nameDescriptor.configurable) +
          "|keys:" + Reflect.ownKeys(getter).map(String).join(",") +
          "|proto:" + (Object.getPrototypeOf(getter) === fp) +
          "|constructor:" + isConstructor(getter) +
          "|source:" + hex(fp.toString.call(getter)));
    return getter;
});
print("getter-identities=" +
      (getters[0] !== getters[1] && getters[1] !== getters[2] && getters[0] !== getters[2]));

var primary = eval("\n  (function named(){})");
print("primary=" + String(getters[0].call(primary)) + "|" +
      String(getters[1].call(primary)) + "|" + String(getters[2].call(primary)));
print("primary-direct=" + String(primary.fileName) + "|" +
      String(primary.lineNumber) + "|" + String(primary.columnNumber));
print("primary-source=" + hex(fp.toString.call(primary)));
Object.defineProperty(primary, "name", { value: "renamed" });
print("renamed=" + String(primary.fileName) + "|" + String(primary.lineNumber) + "|" +
      String(primary.columnNumber) + "|" + hex(fp.toString.call(primary)));

var anonymous = eval("\n  (function(){})");
print("anonymous=" + String(anonymous.fileName) + "|" +
      String(anonymous.lineNumber) + "|" + String(anonymous.columnNumber) + "|" +
      hex(fp.toString.call(anonymous)));
print("command-line-file=" + String(hex.fileName));

var tabbed = eval("\t(function tabbed(){})");
var carriage = eval("/*x*/\r(function carriage(){})");
var crlf = eval("/*x*/\r\n(function crlf(){})");
var unicode = eval("/*😀*/\t(function unicode(){})");
var lineSeparator = eval("/*x*/\u2028(function lineSeparator(){})");
print("positions=" +
      "tab:" + tabbed.lineNumber + ":" + tabbed.columnNumber +
      "|cr:" + carriage.lineNumber + ":" + carriage.columnNumber +
      "|crlf:" + crlf.lineNumber + ":" + crlf.columnNumber +
      "|unicode:" + unicode.lineNumber + ":" + unicode.columnNumber +
      "|u2028:" + lineSeparator.lineNumber + ":" + lineSeparator.columnNumber);

var outer = eval("\n(function outer(){ return function inner(){}; })");
var inner = outer();
print("nested=" + outer.fileName + ":" + outer.lineNumber + ":" + outer.columnNumber +
      "|" + inner.fileName + ":" + inner.lineNumber + ":" + inner.columnNumber);

var bound = primary.bind(null);
var ordinary = {};
var receivers = [undefined, null, 1, ordinary, fp, bound, getters[0]];
print("invalid-receivers=" + getters.map(function (getter) {
    return receivers.map(function (receiver) {
        return String(getter.call(receiver, 1, 2));
    }).join(",");
}).join("|"));
"#;

#[test]
fn function_debug_accessors_and_strip_modes_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP function debug accessor differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (mode, oracle_flag) in [
        (DebugInfoMode::Full, None),
        (DebugInfoMode::StripSource, Some("--strip-source")),
        (DebugInfoMode::StripDebug, Some("-s")),
    ] {
        assert_eq!(
            rust_observations(mode),
            oracle_observations(&oracle, oracle_flag),
            "debug mode {mode:?} differed from pinned QuickJS"
        );
    }
}

fn rust_observations(mode: DebugInfoMode) -> Vec<String> {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(mode);
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let implemented = [
        "length",
        "name",
        "caller",
        "arguments",
        "call",
        "apply",
        "bind",
        "toString",
        "fileName",
        "lineNumber",
        "columnNumber",
        "Symbol(Symbol.hasInstance)",
    ];
    let has_instance = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let key_names = runtime
        .own_property_keys(&function_prototype)
        .unwrap()
        .into_iter()
        .filter_map(|key| {
            let name = if key == has_instance {
                "Symbol(Symbol.hasInstance)".to_owned()
            } else {
                runtime
                    .property_key_to_js_string(&key)
                    .unwrap()
                    .to_utf8_lossy()
            };
            implemented.contains(&name.as_str()).then_some(name)
        })
        .collect::<Vec<_>>();
    let mut output = vec![format!("fp-keys={}", key_names.join(","))];

    let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
    let mut getters = Vec::new();
    for name in ["fileName", "lineNumber", "columnNumber"] {
        let key = runtime.intern_property_key(name).unwrap();
        let CompleteOrdinaryPropertyDescriptor::Accessor {
            get: Some(getter),
            set: None,
            enumerable,
            configurable,
        } = runtime
            .get_own_property(&function_prototype, &key)
            .unwrap()
            .unwrap()
        else {
            panic!("{name} was not a getter-only accessor");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: length,
            writable: length_writable,
            enumerable: length_enumerable,
            configurable: length_configurable,
        } = own_descriptor(&runtime, getter.as_object(), "length")
        else {
            panic!("getter length was not data");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(getter_name),
            writable: name_writable,
            enumerable: name_enumerable,
            configurable: name_configurable,
        } = own_descriptor(&runtime, getter.as_object(), "name")
        else {
            panic!("getter name was not string data");
        };
        let getter_keys = runtime
            .own_property_keys(getter.as_object())
            .unwrap()
            .iter()
            .map(|key| {
                runtime
                    .property_key_to_js_string(key)
                    .unwrap()
                    .to_utf8_lossy()
            })
            .collect::<Vec<_>>()
            .join(",");
        let Value::String(getter_source) = context
            .call(&to_string, Value::Object(getter.as_object().clone()), &[])
            .unwrap()
        else {
            panic!("getter toString was not a string");
        };
        output.push(format!(
            "getter-meta={name}:function:undefined:{},{}|length:{}:{},{},{}|name:{}:{},{},{}|keys:{getter_keys}|proto:{}|constructor:{}|source:{}",
            bit(enumerable),
            bit(configurable),
            plain_value(length),
            bit(length_writable),
            bit(length_enumerable),
            bit(length_configurable),
            getter_name.to_utf8_lossy(),
            bit(name_writable),
            bit(name_enumerable),
            bit(name_configurable),
            runtime.get_prototype_of(getter.as_object()).unwrap()
                == Some(function_prototype.clone()),
            runtime.is_constructor(getter.as_object()).unwrap(),
            hex(&getter_source),
        ));
        getters.push((key, getter));
    }
    output.push(format!(
        "getter-identities={}",
        getters[0].1.as_object() != getters[1].1.as_object()
            && getters[1].1.as_object() != getters[2].1.as_object()
            && getters[0].1.as_object() != getters[2].1.as_object()
    ));

    let primary = eval_function(&mut context, "\n  (function named(){})");
    output.push(format!(
        "primary={}|{}|{}",
        call_text(&mut context, &getters[0].1, Value::Object(primary.clone())),
        call_text(&mut context, &getters[1].1, Value::Object(primary.clone())),
        call_text(&mut context, &getters[2].1, Value::Object(primary.clone())),
    ));
    output.push(format!(
        "primary-direct={}|{}|{}",
        property_text(&mut context, &primary, &getters[0].0),
        property_text(&mut context, &primary, &getters[1].0),
        property_text(&mut context, &primary, &getters[2].0),
    ));
    output.push(format!(
        "primary-source={}",
        function_source_hex(&mut context, &to_string, &primary)
    ));
    let name_key = runtime.intern_property_key("name").unwrap();
    assert!(
        runtime
            .define_own_property(
                &primary,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from("renamed"))),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    output.push(format!(
        "renamed={}|{}|{}|{}",
        property_text(&mut context, &primary, &getters[0].0),
        property_text(&mut context, &primary, &getters[1].0),
        property_text(&mut context, &primary, &getters[2].0),
        function_source_hex(&mut context, &to_string, &primary),
    ));

    let anonymous = eval_function(&mut context, "\n  (function(){})");
    output.push(format!(
        "anonymous={}|{}|{}|{}",
        property_text(&mut context, &anonymous, &getters[0].0),
        property_text(&mut context, &anonymous, &getters[1].0),
        property_text(&mut context, &anonymous, &getters[2].0),
        function_source_hex(&mut context, &to_string, &anonymous),
    ));
    let command_line =
        eval_function_with_filename(&mut context, "(function commandLine(){})", "<cmdline>");
    output.push(format!(
        "command-line-file={}",
        property_text(&mut context, &command_line, &getters[0].0)
    ));

    let positions = [
        ("tab", "\t(function tabbed(){})"),
        ("cr", "/*x*/\r(function carriage(){})"),
        ("crlf", "/*x*/\r\n(function crlf(){})"),
        ("unicode", "/*😀*/\t(function unicode(){})"),
        ("u2028", "/*x*/\u{2028}(function lineSeparator(){})"),
    ]
    .map(|(label, source)| {
        let function = eval_function(&mut context, source);
        format!(
            "{label}:{}:{}",
            property_text(&mut context, &function, &getters[1].0),
            property_text(&mut context, &function, &getters[2].0),
        )
    });
    output.push(format!("positions={}", positions.join("|")));

    let outer = eval_function(
        &mut context,
        "\n(function outer(){ return function inner(){}; })",
    );
    let outer_callable = runtime.as_callable(&outer).unwrap().unwrap();
    let Value::Object(inner) = context
        .call(&outer_callable, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("outer function did not return its inner function");
    };
    output.push(format!(
        "nested={}:{}:{}|{}:{}:{}",
        property_text(&mut context, &outer, &getters[0].0),
        property_text(&mut context, &outer, &getters[1].0),
        property_text(&mut context, &outer, &getters[2].0),
        property_text(&mut context, &inner, &getters[0].0),
        property_text(&mut context, &inner, &getters[1].0),
        property_text(&mut context, &inner, &getters[2].0),
    ));

    let bind = property_callable(&runtime, &mut context, &function_prototype, "bind");
    let Value::Object(bound) = context
        .call(&bind, Value::Object(primary), &[Value::Null])
        .unwrap()
    else {
        panic!("bind did not return an object");
    };
    let ordinary = context.new_object().unwrap();
    let invalid_receivers = [
        Value::Undefined,
        Value::Null,
        Value::Int(1),
        Value::Object(ordinary),
        Value::Object(function_prototype),
        Value::Object(bound),
        Value::Object(getters[0].1.as_object().clone()),
    ];
    let invalid = getters
        .iter()
        .map(|(_, getter)| {
            invalid_receivers
                .iter()
                .cloned()
                .map(|receiver| {
                    let value = context
                        .call(getter, receiver, &[Value::Int(1), Value::Int(2)])
                        .unwrap();
                    plain_value(value)
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("|");
    output.push(format!("invalid-receivers={invalid}"));
    output
}

fn eval_function(context: &mut Context, source: &str) -> ObjectRef {
    eval_function_with_filename(context, source, "<input>")
}

fn eval_function_with_filename(context: &mut Context, source: &str, filename: &str) -> ObjectRef {
    let Value::Object(function) = context.eval_with_filename(source, filename).unwrap() else {
        panic!("source did not evaluate to a function: {source:?}");
    };
    function
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
    runtime.as_callable(&value).unwrap().unwrap()
}

fn own_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
) -> CompleteOrdinaryPropertyDescriptor {
    let key = runtime.intern_property_key(name).unwrap();
    runtime.get_own_property(object, &key).unwrap().unwrap()
}

fn call_text(context: &mut Context, getter: &CallableRef, receiver: Value) -> String {
    plain_value(
        context
            .call(getter, receiver, &[Value::Int(1), Value::Int(2)])
            .unwrap(),
    )
}

fn property_text(context: &mut Context, object: &ObjectRef, key: &PropertyKey) -> String {
    plain_value(context.get_property(object, key).unwrap())
}

fn function_source_hex(
    context: &mut Context,
    to_string: &CallableRef,
    function: &ObjectRef,
) -> String {
    let Value::String(source) = context
        .call(to_string, Value::Object(function.clone()), &[])
        .unwrap()
    else {
        panic!("Function.prototype.toString did not return a string");
    };
    hex(&source)
}

fn plain_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) | Value::Object(_) => panic!("unexpected observation value"),
    }
}

fn hex(value: &JsString) -> String {
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect()
}

const fn bit(value: bool) -> u8 {
    value as u8
}

fn oracle_observations(oracle: &OsStr, flag: Option<&str>) -> Vec<String> {
    let mut command = Command::new(oracle);
    if let Some(flag) = flag {
        command.arg(flag);
    }
    let output = command.args(["-e", ORACLE_PROBE]).output().unwrap();
    assert!(
        output.status.success(),
        "QuickJS function debug accessor oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}
