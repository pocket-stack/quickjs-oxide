use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value,
};

const ORACLE_PROBE: &str = r#"
function bits(d) {
    return Number(d.writable) + "," + Number(d.enumerable) + "," + Number(d.configurable);
}
var ep = Error.prototype, tp = TypeError.prototype;
print("error-keys=" + Reflect.ownKeys(Error).join(","));
print("type-error-keys=" + Reflect.ownKeys(TypeError).join(","));
print("error-prototype-keys=" + Reflect.ownKeys(ep).join(","));
print("type-error-prototype-keys=" + Reflect.ownKeys(tp).join(","));
print("constructor-links=" + bits(Object.getOwnPropertyDescriptor(Error, "prototype")) + "|" + bits(Object.getOwnPropertyDescriptor(ep, "constructor")));
print("chains=" + (Object.getPrototypeOf(Error) === Function.prototype) + "|" + (Object.getPrototypeOf(TypeError) === Error) + "|" + (Object.getPrototypeOf(ep) === Object.prototype) + "|" + (Object.getPrototypeOf(tp) === ep));
print("metadata=" + Error.length + "|" + TypeError.length + "|" + Error.name + "|" + TypeError.name);
var empty = Error(), numbered = Error(42), typed = new TypeError("x");
var caused = Error(undefined, { cause: undefined });
print("instances=" + (Object.getPrototypeOf(empty) === ep) + "|" + (Object.getPrototypeOf(typed) === tp) + "|" + Object.hasOwn(empty, "message") + "|" + Error.isError(empty));
print("messages=" + Object.hasOwn(empty, "message") + "|" + numbered.message + "|" + Object.hasOwn(caused, "cause") + "|" + String(caused.cause));
print("cause-bits=" + bits(Object.getOwnPropertyDescriptor(caused, "cause")));
print("to-string=" + ep.toString.call(empty) + "|" + ep.toString.call(numbered) + "|" + ep.toString.call(typed) + "|" + ep.toString.call({ name: "", message: "m" }));
print("is-error=" + Error.isError(empty) + "|" + Error.isError(ep) + "|" + Error.isError(Object.create(ep)));
print("method-bits=" + bits(Object.getOwnPropertyDescriptor(Error, "isError")) + "|" + bits(Object.getOwnPropertyDescriptor(ep, "toString")));
print("object-prefix=" + Reflect.ownKeys(Object.prototype).join(",") + "|" + bits(Object.getOwnPropertyDescriptor(Object.prototype, "toString")) + "|" + bits(Object.getOwnPropertyDescriptor(Object.prototype, "valueOf")));
print("object-message=" + Error({}).message + "|" + Error({ [Symbol.toStringTag]: "X" }).message);
print("object-tags=" + Object.prototype.toString.call(null) + "|" + Object.prototype.toString.call(undefined) + "|" + Object.prototype.toString.call({}) + "|" + Object.prototype.toString.call(function(){}) + "|" + Object.prototype.toString.call(Error()));
try { Error(Symbol()); } catch (e) { print("symbol-message=" + e.name + "|" + e.message); }
"#;

#[test]
fn error_intrinsic_slice_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Error oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    let cause = runtime.intern_property_key("cause").unwrap();
    let is_error_key = runtime.intern_property_key("isError").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();

    let error_prototype = object_data_value(&runtime, error.as_object(), &prototype);
    let type_error_prototype = object_data_value(&runtime, type_error.as_object(), &prototype);
    let error_keys = own_key_names(&runtime, error.as_object());
    let type_error_keys = own_key_names(&runtime, type_error.as_object());
    let error_prototype_keys = own_key_names(&runtime, &error_prototype);
    let type_error_prototype_keys = own_key_names(&runtime, &type_error_prototype);
    let error_prototype_bits = descriptor_bits(
        runtime
            .get_own_property(error.as_object(), &prototype)
            .unwrap()
            .unwrap(),
    );
    let error_constructor_link_bits = descriptor_bits(
        runtime
            .get_own_property(&error_prototype, &constructor)
            .unwrap()
            .unwrap(),
    );
    let chains = [
        runtime.get_prototype_of(error.as_object()).unwrap()
            == Some(context.function_prototype().unwrap()),
        runtime.get_prototype_of(type_error.as_object()).unwrap()
            == Some(error.as_object().clone()),
        runtime.get_prototype_of(&error_prototype).unwrap()
            == Some(context.object_prototype().unwrap()),
        runtime.get_prototype_of(&type_error_prototype).unwrap() == Some(error_prototype.clone()),
    ];
    let metadata = [
        primitive_data_value(&runtime, error.as_object(), &length),
        primitive_data_value(&runtime, type_error.as_object(), &length),
        primitive_data_value(&runtime, error.as_object(), &name),
        primitive_data_value(&runtime, type_error.as_object(), &name),
    ];

    let Value::Object(empty) = context.call(&error, Value::Undefined, &[]).unwrap() else {
        panic!("Error() did not return an object");
    };
    let Value::Object(numbered) = context
        .call(&error, Value::Undefined, &[Value::Int(42)])
        .unwrap()
    else {
        panic!("Error(42) did not return an object");
    };
    let Value::Object(typed) = context
        .construct(
            &type_error,
            &[Value::String(JsString::try_from_utf8("x").unwrap())],
        )
        .unwrap()
    else {
        panic!("new TypeError did not return an object");
    };
    let options = context.new_object().unwrap();
    assert!(
        context
            .define_own_property(
                &options,
                &cause,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Undefined),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(caused) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(options)],
        )
        .unwrap()
    else {
        panic!("Error(undefined, options) did not return an object");
    };

    let is_error_object = context
        .get_property(error.as_object(), &is_error_key)
        .unwrap();
    let Value::Object(is_error_object) = is_error_object else {
        panic!("Error.isError was not an object");
    };
    let is_error = runtime.as_callable(&is_error_object).unwrap().unwrap();
    let to_string_object = context
        .get_property(&error_prototype, &to_string_key)
        .unwrap();
    let Value::Object(to_string_object) = to_string_object else {
        panic!("Error.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

    let custom = context.new_object().unwrap();
    define_data(
        &mut context,
        &custom,
        &name,
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    define_data(
        &mut context,
        &custom,
        &message,
        Value::String(JsString::try_from_utf8("m").unwrap()),
    );
    let spoof = context.new_object().unwrap();
    assert!(
        runtime
            .set_prototype_of(&spoof, Some(&error_prototype))
            .unwrap()
    );

    let empty_is_error = call_bool(&mut context, &is_error, &[Value::Object(empty.clone())]);
    let prototype_is_error = call_bool(
        &mut context,
        &is_error,
        &[Value::Object(error_prototype.clone())],
    );
    let spoof_is_error = call_bool(&mut context, &is_error, &[Value::Object(spoof)]);
    let cause_descriptor = runtime.get_own_property(&caused, &cause).unwrap().unwrap();

    let symbol = runtime.new_symbol(None).unwrap();
    assert!(matches!(
        context.call(&error, Value::Undefined, &[Value::Symbol(symbol)]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(symbol_exception) = context.take_exception().unwrap().unwrap() else {
        panic!("Error(Symbol()) did not throw an Error object");
    };
    let symbol_name = property_string(&mut context, &symbol_exception, &name);
    let symbol_message = property_string(&mut context, &symbol_exception, &message);

    let object_prototype = context.object_prototype().unwrap();
    let object_prefix = own_key_names(&runtime, &object_prototype);
    let object_to_string_descriptor = runtime
        .get_own_property(&object_prototype, &to_string_key)
        .unwrap()
        .unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let object_value_of_descriptor = runtime
        .get_own_property(&object_prototype, &value_of_key)
        .unwrap()
        .unwrap();
    let Value::Object(object_to_string_object) = context
        .get_property(&object_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Object.prototype.toString was not an object");
    };
    let object_to_string = runtime
        .as_callable(&object_to_string_object)
        .unwrap()
        .unwrap();
    let plain_message_input = context.new_object().unwrap();
    let Value::Object(plain_message_error) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Object(plain_message_input)],
        )
        .unwrap()
    else {
        panic!("Error(plain object) did not return an object");
    };
    let tagged_message_input = context.new_object().unwrap();
    let to_string_tag =
        PropertyKey::from(runtime.well_known_symbol(quickjs_oxide::WellKnownSymbol::ToStringTag));
    define_data(
        &mut context,
        &tagged_message_input,
        &to_string_tag,
        Value::String(JsString::try_from_utf8("X").unwrap()),
    );
    let Value::Object(tagged_message_error) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Object(tagged_message_input)],
        )
        .unwrap()
    else {
        panic!("Error(tagged object) did not return an object");
    };
    let tag_object = context.new_object().unwrap();
    let tag_function = context.eval("(0, function(){})").unwrap();
    let tag_error = context.call(&error, Value::Undefined, &[]).unwrap();

    vec![
        format!("error-keys={}", error_keys.join(",")),
        format!("type-error-keys={}", type_error_keys.join(",")),
        format!("error-prototype-keys={}", error_prototype_keys.join(",")),
        format!(
            "type-error-prototype-keys={}",
            type_error_prototype_keys.join(",")
        ),
        format!("constructor-links={error_prototype_bits}|{error_constructor_link_bits}"),
        format!(
            "chains={}|{}|{}|{}",
            chains[0], chains[1], chains[2], chains[3]
        ),
        format!(
            "metadata={}|{}|{}|{}",
            metadata[0], metadata[1], metadata[2], metadata[3]
        ),
        format!(
            "instances={}|{}|{}|{}",
            runtime.get_prototype_of(&empty).unwrap() == Some(error_prototype.clone()),
            runtime.get_prototype_of(&typed).unwrap() == Some(type_error_prototype),
            runtime.has_own_property(&empty, &message).unwrap(),
            empty_is_error
        ),
        format!(
            "messages={}|{}|{}|{}",
            runtime.has_own_property(&empty, &message).unwrap(),
            property_string(&mut context, &numbered, &message),
            runtime.has_own_property(&caused, &cause).unwrap(),
            value_text(context.get_property(&caused, &cause).unwrap())
        ),
        format!("cause-bits={}", descriptor_bits(cause_descriptor)),
        format!(
            "to-string={}|{}|{}|{}",
            call_string(&mut context, &to_string, Value::Object(empty)),
            call_string(&mut context, &to_string, Value::Object(numbered)),
            call_string(&mut context, &to_string, Value::Object(typed)),
            call_string(&mut context, &to_string, Value::Object(custom))
        ),
        format!("is-error={empty_is_error}|{prototype_is_error}|{spoof_is_error}"),
        format!(
            "method-bits={}|{}",
            descriptor_bits(
                runtime
                    .get_own_property(error.as_object(), &is_error_key)
                    .unwrap()
                    .unwrap()
            ),
            descriptor_bits(
                runtime
                    .get_own_property(&error_prototype, &to_string_key)
                    .unwrap()
                    .unwrap()
            )
        ),
        format!(
            "object-prefix={}|{}|{}",
            object_prefix.join(","),
            descriptor_bits(object_to_string_descriptor),
            descriptor_bits(object_value_of_descriptor)
        ),
        format!(
            "object-message={}|{}",
            property_string(&mut context, &plain_message_error, &message),
            property_string(&mut context, &tagged_message_error, &message)
        ),
        format!(
            "object-tags={}|{}|{}|{}|{}",
            call_string(&mut context, &object_to_string, Value::Null),
            call_string(&mut context, &object_to_string, Value::Undefined),
            call_string(&mut context, &object_to_string, Value::Object(tag_object)),
            call_string(&mut context, &object_to_string, tag_function),
            call_string(&mut context, &object_to_string, tag_error)
        ),
        format!("symbol-message={symbol_name}|{symbol_message}"),
    ]
}

fn global_callable(
    runtime: &Runtime,
    context: &mut quickjs_oxide::Context,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(object) = context
        .get_property(&context.global_object().unwrap(), &key)
        .unwrap()
    else {
        panic!("global {name} was not an object");
    };
    runtime.as_callable(&object).unwrap().unwrap()
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn object_data_value(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey) -> ObjectRef {
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(value),
        ..
    } = runtime.get_own_property(object, key).unwrap().unwrap()
    else {
        panic!("property was not an object-valued data property");
    };
    value
}

fn primitive_data_value(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey) -> String {
    let CompleteOrdinaryPropertyDescriptor::Data { value, .. } =
        runtime.get_own_property(object, key).unwrap().unwrap()
    else {
        panic!("property was not a data property");
    };
    value_text(value)
}

fn descriptor_bits(descriptor: CompleteOrdinaryPropertyDescriptor) -> String {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            writable,
            enumerable,
            configurable,
            ..
        } => bits(writable, enumerable, configurable),
        CompleteOrdinaryPropertyDescriptor::Accessor {
            enumerable,
            configurable,
            ..
        } => format!(
            "accessor,{},{}",
            u8::from(enumerable),
            u8::from(configurable)
        ),
    }
}

fn bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "{},{},{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn define_data(
    context: &mut quickjs_oxide::Context,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
) {
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

fn call_bool(
    context: &mut quickjs_oxide::Context,
    callable: &CallableRef,
    arguments: &[Value],
) -> bool {
    let Value::Bool(value) = context.call(callable, Value::Undefined, arguments).unwrap() else {
        panic!("boolean native call did not return a boolean");
    };
    value
}

fn call_string(
    context: &mut quickjs_oxide::Context,
    callable: &CallableRef,
    this_value: Value,
) -> String {
    let Value::String(value) = context.call(callable, this_value, &[]).unwrap() else {
        panic!("string native call did not return a string");
    };
    value.to_utf8_lossy()
}

fn property_string(
    context: &mut quickjs_oxide::Context,
    object: &ObjectRef,
    key: &PropertyKey,
) -> String {
    let Value::String(value) = context.get_property(object, key).unwrap() else {
        panic!("property was not a string");
    };
    value.to_utf8_lossy()
}

fn value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) => "symbol".to_owned(),
        Value::Object(_) => "object".to_owned(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Error oracle");
    assert!(
        output.status.success(),
        "QuickJS Error oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Error oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
