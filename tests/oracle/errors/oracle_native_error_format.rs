use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    DescriptorField, JsString, OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

#[derive(Debug, Eq, PartialEq)]
struct Observation {
    name: Vec<u16>,
    message: Vec<u16>,
}

#[test]
fn native_error_format_uses_the_quickjs_255_byte_payload_boundary() {
    let cases = native_error_name_cases();
    let actual = rust_observations(&cases);
    let suffix = " is not a constructor".encode_utf16().collect::<Vec<_>>();

    let expected_messages = [
        [vec![u16::from(b'A'); 234], suffix.clone()].concat(),
        [
            vec![u16::from(b'A'); 235],
            suffix.iter().copied().take(20).collect(),
        ]
        .concat(),
        [vec![u16::from(b'A'); 253], vec![0x00e9]].concat(),
        [vec![u16::from(b'A'); 254], vec![0xfffd]].concat(),
        [vec![u16::from(b'A'); 252], vec![0xfffd]].concat(),
        [vec![u16::from(b'A'); 251], vec![0xd83d, 0xde42]].concat(),
        [vec![u16::from(b'A'); 253], vec![0xfffd]].concat(),
        [vec![u16::from(b'A'); 252], vec![0xd800]].concat(),
        [vec![u16::from(b'A')], suffix].concat(),
        "not a constructor".encode_utf16().collect(),
    ];
    assert_eq!(actual.len(), expected_messages.len());
    for (index, (actual, expected_message)) in actual.iter().zip(expected_messages).enumerate() {
        assert_eq!(actual.name, "TypeError".encode_utf16().collect::<Vec<_>>());
        assert_eq!(
            actual.message, expected_message,
            "native error case {index}"
        );
    }
}

#[test]
fn native_error_format_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP native-error formatter differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let cases = native_error_name_cases();
    assert_eq!(
        rust_observations(&cases),
        oracle_observations(&oracle, &cases),
        "native Error formatting differed from pinned QuickJS"
    );
}

#[test]
fn explicit_error_constructor_message_bypasses_the_native_throw_buffer() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let error_key = runtime.intern_property_key("Error").unwrap();
    let Value::Object(error_constructor) = context.get_property(&global, &error_key).unwrap()
    else {
        panic!("global Error was not an object");
    };
    let error_constructor = runtime
        .as_callable(&error_constructor)
        .unwrap()
        .expect("global Error was not callable");
    let long_message = JsString::try_from_utf8(&"M".repeat(300)).unwrap();
    let Value::Object(error) = context
        .call(
            &error_constructor,
            Value::Undefined,
            &[Value::String(long_message.clone())],
        )
        .unwrap()
    else {
        panic!("Error constructor did not return an object");
    };
    let message_key = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message_key).unwrap(),
        Value::String(long_message)
    );
}

fn native_error_name_cases() -> Vec<Vec<u16>> {
    vec![
        vec![u16::from(b'A'); 234],
        vec![u16::from(b'A'); 235],
        [vec![u16::from(b'A'); 253], vec![0x00e9]].concat(),
        [vec![u16::from(b'A'); 254], vec![0x00e9]].concat(),
        [vec![u16::from(b'A'); 252], vec![0xd83d, 0xde42]].concat(),
        [vec![u16::from(b'A'); 251], vec![0xd83d, 0xde42]].concat(),
        [vec![u16::from(b'A'); 253], vec![0xd800]].concat(),
        [vec![u16::from(b'A'); 252], vec![0xd800]].concat(),
        vec![u16::from(b'A'), 0x0000, u16::from(b'B')],
    ]
}

fn rust_observations(cases: &[Vec<u16>]) -> Vec<Observation> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let number_key = runtime.intern_property_key("Number").unwrap();
    let Value::Object(number) = context.get_property(&global, &number_key).unwrap() else {
        panic!("global Number was not an object");
    };
    let is_finite_key = runtime.intern_property_key("isFinite").unwrap();
    let Value::Object(is_finite) = context.get_property(&number, &is_finite_key).unwrap() else {
        panic!("Number.isFinite was not an object");
    };
    let name_key = runtime.intern_property_key("name").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    let mut names = cases
        .iter()
        .map(|units| JsString::try_from_utf16(units.iter().copied()).unwrap())
        .collect::<Vec<_>>();
    names.push(
        JsString::try_from_utf8(&"A".repeat(8193))
            .unwrap()
            .try_concat(&JsString::try_from_utf8(&"B".repeat(513)).unwrap())
            .unwrap(),
    );

    names
        .into_iter()
        .map(|name| {
            assert!(
                context
                    .define_own_property(
                        &is_finite,
                        &name_key,
                        &OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(Value::String(name)),
                            writable: DescriptorField::Present(false),
                            enumerable: DescriptorField::Present(false),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .unwrap()
            );
            assert_eq!(
                context.eval("\nnew Number.isFinite"),
                Err(RuntimeError::Exception)
            );
            let Some(Value::Object(error)) = context.take_exception().unwrap() else {
                panic!("native constructor failure did not install an Error object");
            };
            let Value::String(name) = context.get_property(&error, &name_key).unwrap() else {
                panic!("native Error name was not a String");
            };
            let Value::String(message) = context.get_property(&error, &message_key).unwrap() else {
                panic!("native Error message was not a String");
            };
            Observation {
                name: name.utf16_units().collect(),
                message: message.utf16_units().collect(),
            }
        })
        .collect()
}

fn oracle_observations(oracle: &OsStr, cases: &[Vec<u16>]) -> Vec<Observation> {
    let cases = cases
        .iter()
        .map(|units| {
            format!(
                "String.fromCharCode({})",
                units
                    .iter()
                    .map(|unit| format!("0x{unit:04x}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r#"
function units(value) {{
    var out = [];
    for (var i = 0; i < value.length; i++)
        out.push(("0000" + value.charCodeAt(i).toString(16)).slice(-4));
    return out.join(",");
}}
var cases = [{cases}];
cases.push("A".repeat(8193) + "B".repeat(513));
for (var i = 0; i < cases.length; i++) {{
    Object.defineProperty(Number.isFinite, "name", {{
        value: cases[i], configurable: true
    }});
    try {{
        new Number.isFinite;
    }} catch (error) {{
        print(units(error.name) + "|" + units(error.message));
    }}
}}
"#
    );
    let output = Command::new(oracle)
        .args(["--std", "-e", &source])
        .output()
        .expect("run QuickJS native-error formatter oracle");
    assert!(
        output.status.success(),
        "QuickJS native-error formatter oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS native-error formatter oracle emitted non-UTF-8 observations")
        .lines()
        .map(|line| {
            let (name, message) = line
                .split_once('|')
                .expect("malformed QuickJS native-error observation");
            Observation {
                name: parse_units(name),
                message: parse_units(message),
            }
        })
        .collect()
}

fn parse_units(value: &str) -> Vec<u16> {
    if value.is_empty() {
        Vec::new()
    } else {
        value
            .split(',')
            .map(|unit| u16::from_str_radix(unit, 16).expect("malformed UTF-16 hex unit"))
            .collect()
    }
}
