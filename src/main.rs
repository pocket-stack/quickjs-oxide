use std::fmt::Write as _;
use std::process::ExitCode;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    Context, JsString, PropertyKey, QUICKJS_COMPAT_VERSION, Runtime, RuntimeError, Value,
};

const QUICKJS_PRINT_MAX_STRING_LENGTH: usize = 1_000;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version" | "-v") => {
            println!(
                "quickjs-oxide {} (QuickJS {} compatibility target)",
                env!("CARGO_PKG_VERSION"),
                QUICKJS_COMPAT_VERSION
            );
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") | None => {
            println!("usage: qjs [options] [file [args]]");
            println!("  -e, --eval EXPR   evaluate EXPR");
            println!("  -v, --version     show version and compatibility target");
            ExitCode::SUCCESS
        }
        Some("-q" | "--quit") => {
            let runtime = Runtime::new();
            let _context = runtime.new_context();
            ExitCode::SUCCESS
        }
        Some("-e" | "--eval") => {
            let Some(source) = args.next() else {
                eprintln!("qjs: -e requires an expression");
                return ExitCode::from(2);
            };
            evaluate(&source, "<cmdline>")
        }
        Some(option) if option.starts_with('-') => {
            eprintln!("qjs: unknown option: {option}");
            ExitCode::from(2)
        }
        Some(file) => match std::fs::read_to_string(file) {
            Ok(source) => evaluate(&source, file),
            Err(error) => {
                eprintln!("qjs: could not read '{file}': {error}");
                ExitCode::from(1)
            }
        },
    }
}

fn evaluate(source: &str, filename: &str) -> ExitCode {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval_with_filename(source, filename) {
        Ok(_) => ExitCode::SUCCESS,
        Err(RuntimeError::Exception) => {
            match format_pending_exception(&runtime, &mut context) {
                Some(exception) => eprintln!("{exception}"),
                None => eprintln!("JavaScript exception"),
            }
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn format_pending_exception(runtime: &Runtime, context: &mut Context) -> Option<String> {
    let exception = context.take_exception().ok().flatten()?;
    format_exception(runtime, &exception)
}

fn format_exception(runtime: &Runtime, exception: &Value) -> Option<String> {
    if let Value::Object(object) = &exception {
        if runtime.is_error_object(object).ok()? {
            let name = runtime.intern_property_key("name").ok()?;
            let message = runtime.intern_property_key("message").ok()?;
            let name = runtime
                .raw_string_property_for_diagnostics(object, &name)
                .ok()?
                .map_or_else(|| "Error".to_owned(), |name| diagnostic_c_string(&name));
            let message = runtime
                .raw_string_property_for_diagnostics(object, &message)
                .ok()?
                .map(|message| diagnostic_c_string(&message));
            let header = match message {
                Some(message) if !message.is_empty() => format!("{name}: {message}"),
                Some(_) | None => name,
            };
            let stack = runtime.intern_property_key("stack").ok()?;
            if let Some(stack) = runtime
                .raw_string_property_for_diagnostics(object, &stack)
                .ok()?
            {
                let stack = diagnostic_c_string(&stack);
                return Some(format!(
                    "{header}\n{}",
                    stack.strip_suffix('\n').unwrap_or(&stack)
                ));
            }
            return Some(header);
        }
    }
    format_thrown_value(runtime, exception)
}

fn format_thrown_value(runtime: &Runtime, value: &Value) -> Option<String> {
    Some(match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if *value == 0.0 && value.is_sign_negative() => "-0".to_owned(),
        Value::Float(value) => number_to_string(*value),
        Value::BigInt(value) => format!("{value}n"),
        Value::String(value) => quote_js_string(value, Some(QUICKJS_PRINT_MAX_STRING_LENGTH)),
        Value::Symbol(symbol) => {
            let key = PropertyKey::from(symbol);
            let description = runtime.property_key_to_js_string(&key).ok()?;
            let description = if is_ascii_identifier(&description) {
                description.to_utf8_lossy()
            } else {
                quote_js_string(&description, None)
            };
            format!("Symbol({description})")
        }
        // Full side-effect-free object traversal and class-specific rendering
        // will move behind a runtime diagnostic API as more object classes are
        // implemented. Error objects use the exact QuickJS path above.
        Value::Object(_) => "[object Object]".to_owned(),
    })
}

fn diagnostic_c_string(value: &JsString) -> String {
    JsString::from_utf16(value.utf16_units().take_while(|unit| *unit != 0)).to_utf8_lossy()
}

fn quote_js_string(value: &JsString, max_length: Option<usize>) -> String {
    let units = value.utf16_units().collect::<Vec<_>>();
    let limit = max_length.unwrap_or(units.len()).min(units.len());
    let mut output = String::with_capacity(limit.saturating_add(2));
    output.push('"');

    let mut index = 0;
    while index < limit {
        let unit = units[index];
        index += 1;
        match unit {
            0x0009 => output.push_str("\\t"),
            0x000d => output.push_str("\\r"),
            0x000a => output.push_str("\\n"),
            0x0008 => output.push_str("\\b"),
            0x000c => output.push_str("\\f"),
            0x005c => output.push_str("\\\\"),
            0x0022 => output.push_str("\\\""),
            0x0020..=0x007e => {
                output.push(char::from_u32(u32::from(unit)).expect("ASCII is valid"))
            }
            0x0000..=0x001f | 0x007f..=0x009f => push_unicode_escape(&mut output, unit),
            0xd800..=0xdbff if index < limit && (0xdc00..=0xdfff).contains(&units[index]) => {
                let low = units[index];
                index += 1;
                let scalar =
                    0x1_0000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
                output.push(char::from_u32(scalar).expect("surrogate pair is a valid scalar"));
            }
            0xd800..=0xdfff => push_unicode_escape(&mut output, unit),
            _ => output.push(
                char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16 unit is a scalar"),
            ),
        }
    }

    output.push('"');
    if units.len() > limit {
        let remaining = units.len() - limit;
        let plural = if remaining > 1 { "s" } else { "" };
        write!(output, "... {remaining} more character{plural}")
            .expect("writing to a String cannot fail");
    }
    output
}

fn push_unicode_escape(output: &mut String, unit: u16) {
    write!(output, "\\u{unit:04x}").expect("writing to a String cannot fail");
}

fn is_ascii_identifier(value: &JsString) -> bool {
    let mut units = value.utf16_units();
    let Some(first) = units.next() else {
        return false;
    };
    is_ascii_identifier_start(first)
        && units.all(|unit| is_ascii_identifier_start(unit) || (0x0030..=0x0039).contains(&unit))
}

const fn is_ascii_identifier_start(unit: u16) -> bool {
    matches!(unit, 0x0061..=0x007a | 0x0041..=0x005a | 0x005f | 0x0024)
}

#[cfg(test)]
mod tests {
    use quickjs_oxide::{
        AccessorValue, DescriptorField, JsString, OrdinaryPropertyDescriptor, Runtime, Value,
    };

    use super::format_exception;

    fn data_descriptor(value: Value) -> OrdinaryPropertyDescriptor {
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(false),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        }
    }

    fn accessor_descriptor(getter: quickjs_oxide::CallableRef) -> OrdinaryPropertyDescriptor {
        OrdinaryPropertyDescriptor {
            get: DescriptorField::Present(AccessorValue::Callable(getter)),
            set: DescriptorField::Present(AccessorValue::Undefined),
            enumerable: DescriptorField::Present(false),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        }
    }

    #[test]
    fn error_dump_uses_raw_shadowing_and_never_executes_getters() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(error) = context.eval("new Error(\"boom\")").unwrap() else {
            panic!("Error constructor did not return an object");
        };
        let prototype = runtime
            .get_prototype_of(&error)
            .unwrap()
            .expect("Error has a prototype");
        let Value::Object(getter_object) = context
            .eval("(function diagnosticGetter(){ throw \"getter ran\"; })")
            .unwrap()
        else {
            panic!("getter expression did not return an object");
        };
        let getter = runtime
            .as_callable(&getter_object)
            .unwrap()
            .expect("getter is callable");

        for (name, prototype_value) in [
            ("name", "PrototypeName"),
            ("message", "PrototypeMessage"),
            ("stack", "prototype stack\n"),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            let _ = context.get_property(&prototype, &key).unwrap();
            assert!(
                runtime
                    .define_own_property(
                        &prototype,
                        &key,
                        &data_descriptor(Value::String(JsString::from(prototype_value))),
                    )
                    .unwrap()
            );
            assert!(
                runtime
                    .define_own_property(&error, &key, &accessor_descriptor(getter.clone()))
                    .unwrap()
            );
        }

        assert!(!context.has_exception());
        assert_eq!(
            format_exception(&runtime, &Value::Object(error)),
            Some("Error".to_owned())
        );
        assert!(!context.has_exception(), "diagnostic getter was executed");
    }

    #[test]
    fn error_dump_reads_exactly_one_raw_prototype_level() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(error) = context.eval("new Error()").unwrap() else {
            panic!("Error constructor did not return an object");
        };
        let prototype = runtime
            .get_prototype_of(&error)
            .unwrap()
            .expect("Error has a prototype");

        for (name, value) in [
            ("name", "PrototypeName"),
            ("message", "PrototypeMessage"),
            ("stack", "prototype stack\n"),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            let _ = context.get_property(&prototype, &key).unwrap();
            assert!(
                runtime
                    .define_own_property(
                        &prototype,
                        &key,
                        &data_descriptor(Value::String(JsString::from(value))),
                    )
                    .unwrap()
            );
        }
        let stack = runtime.intern_property_key("stack").unwrap();
        assert!(runtime.delete_property(&error, &stack).unwrap());

        assert_eq!(
            format_exception(&runtime, &Value::Object(error)),
            Some("PrototypeName: PrototypeMessage\nprototype stack".to_owned())
        );
    }
}
