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
fn atom_error_format_preserves_ascii_fast_path_and_non_ascii_scratch_boundary() {
    let actual = rust_observations(&atom_cases());
    let expected_arguments = [
        vec![u16::from(b'A'); 70],
        vec![u16::from(b'A'); 300],
        vec![u16::from(b'A'); 58],
        vec![u16::from(b'A'); 100],
        vec![u16::from(b'A'); 58],
        [vec![u16::from(b'A'); 57], vec![0x00e9]].concat(),
        vec![u16::from(b'A'); 58],
        [vec![u16::from(b'A'); 54], vec![0xd83d, 0xde42]].concat(),
        [vec![u16::from(b'A'); 55], vec![0xd83d]].concat(),
        vec![u16::from(b'A')],
        vec![0x00e9],
        vec![u16::from(b'A'); 240],
        vec![u16::from(b'A'); 241],
    ];
    let suffix = "' is read-only".encode_utf16().collect::<Vec<_>>();
    assert_eq!(actual.len(), expected_arguments.len());
    for (index, (actual, argument)) in actual.iter().zip(expected_arguments).enumerate() {
        let mut expected = vec![u16::from(b'\'')];
        expected.extend(argument);
        expected.extend(&suffix);
        expected.truncate(255);
        assert_eq!(actual.name, "TypeError".encode_utf16().collect::<Vec<_>>());
        assert_eq!(actual.message, expected, "atom error case {index}");
    }
}

#[test]
fn atom_error_format_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP native atom-error differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let cases = atom_cases();
    assert_eq!(
        rust_observations(&cases),
        oracle_observations(&oracle, &cases),
        "native atom Error formatting differed from pinned QuickJS"
    );
}

fn atom_cases() -> Vec<Vec<u16>> {
    vec![
        vec![u16::from(b'A'); 70],
        vec![u16::from(b'A'); 300],
        [vec![u16::from(b'A'); 300], vec![0x00e9]].concat(),
        [vec![u16::from(b'A'); 100], vec![0]].concat(),
        [vec![u16::from(b'A'); 100], vec![0, 0x00e9]].concat(),
        [vec![u16::from(b'A'); 57], vec![0x00e9]].concat(),
        [vec![u16::from(b'A'); 58], vec![0x00e9]].concat(),
        [vec![u16::from(b'A'); 54], vec![0xd83d, 0xde42]].concat(),
        [vec![u16::from(b'A'); 55], vec![0xd83d, 0xde42]].concat(),
        vec![u16::from(b'A'), 0, u16::from(b'B')],
        vec![0x00e9, 0, u16::from(b'B')],
        vec![u16::from(b'A'); 240],
        vec![u16::from(b'A'); 241],
    ]
}

fn rust_observations(cases: &[Vec<u16>]) -> Vec<Observation> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let function_key = runtime.intern_property_key("Function").unwrap();
    let Value::Object(function) = context.get_property(&global, &function_key).unwrap() else {
        panic!("global Function was not an object");
    };
    let probe_key = runtime
        .intern_property_key("__native_error_atom_key")
        .unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &probe_key,
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
    let name_key = runtime.intern_property_key("name").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();

    cases
        .iter()
        .map(|units| {
            let spelling = JsString::try_from_utf16(units.iter().copied()).unwrap();
            let property_key = runtime.intern_property_key_js_string(&spelling).unwrap();
            assert!(
                context
                    .define_own_property(
                        &function,
                        &property_key,
                        &OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(Value::Int(0)),
                            writable: DescriptorField::Present(false),
                            enumerable: DescriptorField::Present(false),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .unwrap()
            );
            assert!(
                context
                    .set_property(&global, &probe_key, Value::String(spelling))
                    .unwrap()
            );
            assert_eq!(
                context
                    .eval("(function(){'use strict'; Function[__native_error_atom_key] = 1;})()",),
                Err(RuntimeError::Exception)
            );
            let Some(Value::Object(error)) = context.take_exception().unwrap() else {
                panic!("strict read-only write did not install an Error object");
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
    let case_count = cases.len();
    let cases_source = cases
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
    return value.length + ":" + out.join(",");
}}
var cases = [{cases_source}];
for (var i = 0; i < cases.length; i++) {{
    var key = cases[i];
    Object.defineProperty(Function, key, {{
        value: 0, writable: false, configurable: true
    }});
    try {{
        (function() {{ "use strict"; Function[key] = 1; }})();
    }} catch (error) {{
        print(i + "|" + units(error.name) + "|" + units(error.message));
    }}
}}
"#
    );
    let output = Command::new(oracle)
        .args(["--std", "-e", &source])
        .output()
        .expect("run QuickJS native atom-error oracle");
    assert!(
        output.status.success(),
        "QuickJS native atom-error oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = String::from_utf8(output.stdout)
        .expect("QuickJS native atom-error oracle emitted non-UTF-8 observations");
    let rows = rows.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), case_count);
    rows.into_iter()
        .enumerate()
        .map(|(expected_index, row)| {
            let mut fields = row.split('|');
            let index = fields
                .next()
                .expect("missing atom-error row index")
                .parse::<usize>()
                .expect("malformed atom-error row index");
            assert_eq!(index, expected_index);
            let name = parse_units(fields.next().expect("missing atom-error name"));
            let message = parse_units(fields.next().expect("missing atom-error message"));
            assert!(fields.next().is_none(), "extra atom-error row field");
            Observation { name, message }
        })
        .collect()
}

fn parse_units(value: &str) -> Vec<u16> {
    let (length, units) = value.split_once(':').expect("malformed UTF-16 observation");
    let length = length
        .parse::<usize>()
        .expect("malformed UTF-16 observation length");
    let units = if units.is_empty() {
        Vec::new()
    } else {
        units
            .split(',')
            .map(|unit| u16::from_str_radix(unit, 16).expect("malformed UTF-16 hex unit"))
            .collect::<Vec<_>>()
    };
    assert_eq!(units.len(), length);
    units
}
