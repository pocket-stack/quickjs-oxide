use std::fmt::Write as _;
use std::process::ExitCode;

use quickjs_oxide::lexer::quickjs_detect_module;
use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    Context, DebugInfoMode, DescriptorField, JsString, ModuleImportAttributes,
    ModuleImportMetaProperty, ModuleLoadResult, ModuleLoader, ModuleLoaderError,
    OrdinaryPropertyDescriptor, PromiseState, PropertyKey, QUICKJS_COMPAT_VERSION, Runtime,
    RuntimeError, Value,
};

const QUICKJS_PRINT_MAX_STRING_LENGTH: usize = 1_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SourceGoal {
    #[default]
    Auto,
    Script,
    Module,
}

enum EvaluationError {
    Host(String),
    Runtime(RuntimeError),
    Rejected(Value),
}

impl From<RuntimeError> for EvaluationError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

#[derive(Debug)]
struct FileModuleLoader;

impl ModuleLoader for FileModuleLoader {
    fn check_attributes(
        &self,
        attributes: &[quickjs_oxide::ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        for attribute in attributes {
            if !attribute.key.utf16_units().eq("type".encode_utf16()) {
                return Err(ModuleLoaderError::new(format!(
                    "import attribute '{}' is not supported",
                    attribute.key.to_utf8_lossy()
                )));
            }
        }
        Ok(())
    }

    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let units = normalized_name.utf16_units().collect::<Vec<_>>();
        let filename = String::from_utf16(&units)
            .map_err(|_| ModuleLoaderError::new("module filename is not valid Unicode"))?;
        let source = std::fs::read_to_string(&filename)
            .map_err(|_| ModuleLoaderError::new(format!("module filename '{filename}'")))?;
        if import_type_is(attributes, "json5") {
            return Ok(ModuleLoadResult::Json5Text(source));
        }
        if filename.ends_with(".json") || import_type_is(attributes, "json") {
            return Ok(ModuleLoadResult::JsonText(source));
        }
        let url = canonical_file_url(&filename).map_err(ModuleLoaderError::new)?;
        Ok(ModuleLoadResult::SourceTextWithImportMeta {
            source,
            properties: module_import_meta_properties(&url, false)
                .map_err(|error| ModuleLoaderError::new(error.to_string()))?,
        })
    }
}

fn import_type_is(attributes: &ModuleImportAttributes, expected: &str) -> bool {
    attributes.effective().is_some_and(|attributes| {
        attributes.iter().any(|attribute| {
            attribute.key.utf16_units().eq("type".encode_utf16())
                && attribute.value.utf16_units().eq(expected.encode_utf16())
        })
    })
}

fn canonical_file_url(filename: &str) -> Result<String, String> {
    if filename.contains(':') {
        return Ok(filename.to_owned());
    }
    #[cfg(windows)]
    {
        return Ok(format!("file://{filename}"));
    }
    #[cfg(not(windows))]
    let canonical = std::fs::canonicalize(filename).map_err(|_| "realpath failure".to_owned())?;
    #[cfg(not(windows))]
    let canonical = canonical
        .to_str()
        .ok_or_else(|| "module filename is not valid Unicode".to_owned())?;
    #[cfg(not(windows))]
    Ok(format!("file://{canonical}"))
}

fn module_import_meta_properties(
    url: &str,
    is_main: bool,
) -> Result<Vec<ModuleImportMetaProperty>, quickjs_oxide::JsStringError> {
    Ok(vec![
        ModuleImportMetaProperty::new(
            JsString::try_from_utf8("url")?,
            Value::String(JsString::try_from_utf8(url)?),
        ),
        ModuleImportMetaProperty::new(JsString::try_from_utf8("main")?, Value::Bool(is_main)),
    ])
}

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut debug_info = DebugInfoMode::Full;
    let mut expression = None;
    let mut print_result = false;
    let mut quit = false;
    let mut source_goal = SourceGoal::Auto;
    let mut index = 0;
    while index < args.len() && args[index].starts_with('-') && args[index] != "-" {
        let option = args[index].clone();
        index += 1;
        match option.as_str() {
            "--" => break,
            "--strip-source" => debug_info = DebugInfoMode::StripSource,
            "--print-result" => print_result = true,
            "--module" => source_goal = SourceGoal::Module,
            "--script" => source_goal = SourceGoal::Script,
            "--version" => {
                println!(
                    "quickjs-oxide {} (QuickJS {} compatibility target)",
                    env!("CARGO_PKG_VERSION"),
                    QUICKJS_COMPAT_VERSION
                );
                return ExitCode::SUCCESS;
            }
            "--help" => {
                println!("usage: qjs [options] [file [args]]");
                println!("  -e, --eval EXPR   evaluate EXPR");
                println!("  -m, --module      load as an ES module");
                println!("      --script      load as a script");
                println!("  -s                strip all debug information");
                println!("      --strip-source strip only function source text");
                println!("      --print-result print the script completion value");
                println!("  -v, --version     show version and compatibility target");
                return ExitCode::SUCCESS;
            }
            "--quit" => quit = true,
            "--eval" => {
                let Some(source) = args.get(index) else {
                    eprintln!("qjs: -e requires an expression");
                    return ExitCode::from(2);
                };
                expression = Some(source.clone());
                index += 1;
            }
            short if short.starts_with('-') && !short.starts_with("--") => {
                for (offset, short_option) in short[1..].char_indices() {
                    match short_option {
                        's' => debug_info = DebugInfoMode::StripDebug,
                        'm' => source_goal = SourceGoal::Module,
                        'q' => quit = true,
                        'v' => {
                            println!(
                                "quickjs-oxide {} (QuickJS {} compatibility target)",
                                env!("CARGO_PKG_VERSION"),
                                QUICKJS_COMPAT_VERSION
                            );
                            return ExitCode::SUCCESS;
                        }
                        'h' => {
                            println!("usage: qjs [options] [file [args]]");
                            println!("  -e, --eval EXPR   evaluate EXPR");
                            println!("  -m, --module      load as an ES module");
                            println!("      --script      load as a script");
                            println!("  -s                strip all debug information");
                            println!("      --strip-source strip only function source text");
                            println!("  -v, --version     show version and compatibility target");
                            return ExitCode::SUCCESS;
                        }
                        'e' => {
                            let source_offset = 1 + offset + short_option.len_utf8();
                            if source_offset < short.len() {
                                expression = Some(short[source_offset..].to_owned());
                            } else {
                                let Some(source) = args.get(index) else {
                                    eprintln!("qjs: -e requires an expression");
                                    return ExitCode::from(2);
                                };
                                expression = Some(source.clone());
                                index += 1;
                            }
                            break;
                        }
                        _ => {
                            eprintln!("qjs: unknown option: -{short_option}");
                            return ExitCode::from(2);
                        }
                    }
                }
            }
            _ => {
                eprintln!("qjs: unknown option: {option}");
                return ExitCode::from(2);
            }
        }
    }

    if quit {
        let runtime = Runtime::new();
        runtime.set_debug_info_mode(debug_info);
        let _context = runtime.new_context();
        return ExitCode::SUCCESS;
    }
    if let Some(source) = expression {
        let source_goal = match source_goal {
            SourceGoal::Module => SourceGoal::Module,
            SourceGoal::Auto | SourceGoal::Script => SourceGoal::Script,
        };
        // On Unix, pinned qjs ignores js_module_set_import_meta's failed
        // realpath("<cmdline>") and leaves import.meta empty. Its Windows path
        // has no realpath call and initializes file://<cmdline> normally.
        #[cfg(windows)]
        let main_module_path = (source_goal == SourceGoal::Module).then_some("<cmdline>");
        #[cfg(not(windows))]
        let main_module_path = None;
        return evaluate(
            &source,
            "<cmdline>",
            source_goal,
            main_module_path,
            debug_info,
            print_result,
        );
    }
    let Some(file) = args.get(index) else {
        println!("usage: qjs [options] [file [args]]");
        println!("  -e, --eval EXPR   evaluate EXPR");
        println!("  -v, --version     show version and compatibility target");
        return ExitCode::SUCCESS;
    };
    match std::fs::read_to_string(file) {
        Ok(source) => {
            let source_goal = match source_goal {
                SourceGoal::Auto if is_module_file(file, &source) => SourceGoal::Module,
                SourceGoal::Auto => SourceGoal::Script,
                source_goal => source_goal,
            };
            evaluate(
                &source,
                file,
                source_goal,
                Some(file),
                debug_info,
                print_result,
            )
        }
        Err(error) => {
            eprintln!("{file}: {error}");
            ExitCode::from(1)
        }
    }
}

fn evaluate(
    source: &str,
    filename: &str,
    source_goal: SourceGoal,
    main_module_path: Option<&str>,
    debug_info: DebugInfoMode,
    print_result: bool,
) -> ExitCode {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(debug_info);
    // Upstream qjs installs its filesystem loader for every process, including
    // Script-goal `-e`, so dynamic import has the same host boundary everywhere.
    let _module_loader = runtime.set_module_loader(FileModuleLoader);
    let mut context = runtime.new_context();
    if let Err(error) = context.install_qjs_print() {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    let evaluation = match source_goal {
        SourceGoal::Script => context
            .eval_with_filename(source, filename)
            .map_err(EvaluationError::Runtime),
        SourceGoal::Module => {
            evaluate_module(&runtime, &mut context, source, filename, main_module_path)
        }
        SourceGoal::Auto => unreachable!("the source goal is resolved before evaluation"),
    };
    match evaluation {
        Ok(value) => {
            loop {
                match runtime.execute_pending_job() {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(RuntimeError::Exception) => {
                        match format_pending_exception(&runtime, &mut context) {
                            Some(exception) => eprintln!("{exception}"),
                            None => eprintln!("JavaScript exception"),
                        }
                        return ExitCode::from(1);
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        return ExitCode::from(1);
                    }
                }
            }
            if print_result {
                println!("{}", completion_text(value));
            }
            ExitCode::SUCCESS
        }
        Err(EvaluationError::Rejected(exception)) => {
            match format_exception(&runtime, &exception) {
                Some(exception) => eprintln!("{exception}"),
                None => eprintln!("JavaScript exception"),
            }
            ExitCode::from(1)
        }
        Err(EvaluationError::Host(error)) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
        Err(EvaluationError::Runtime(RuntimeError::Exception)) => {
            match format_pending_exception(&runtime, &mut context) {
                Some(exception) => eprintln!("{exception}"),
                None => eprintln!("JavaScript exception"),
            }
            ExitCode::from(1)
        }
        Err(EvaluationError::Runtime(error)) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn is_module_file(filename: &str, source: &str) -> bool {
    filename.ends_with(".mjs") || quickjs_detect_module(source)
}

fn evaluate_module(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    filename: &str,
    main_module_path: Option<&str>,
) -> Result<Value, EvaluationError> {
    let module = context.compile_module_with_filename(source, filename)?;
    if let Some(main_module_path) = main_module_path {
        let url = canonical_file_url(main_module_path).map_err(EvaluationError::Host)?;
        let import_meta = context.get_module_import_meta(&module)?;
        for property in module_import_meta_properties(&url, true).map_err(RuntimeError::from)? {
            let key = runtime
                .intern_property_key_js_string(property.key())
                .map_err(RuntimeError::from)?;
            let defined = context.define_own_property(
                &import_meta,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(property.value().clone()),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !defined {
                return Err(EvaluationError::Runtime(RuntimeError::Invariant(
                    "fresh import.meta property definition was rejected",
                )));
            }
        }
    }
    let Value::Object(promise) = context.execute_module(&module)? else {
        return Err(EvaluationError::Runtime(RuntimeError::Invariant(
            "module evaluation did not return a Promise",
        )));
    };

    loop {
        let snapshot = runtime
            .promise_snapshot(&promise)?
            .ok_or(RuntimeError::Invariant(
                "module evaluation returned a non-Promise object",
            ))?;
        match snapshot.state() {
            PromiseState::Fulfilled => return Ok(snapshot.result().clone()),
            PromiseState::Rejected => {
                return Err(EvaluationError::Rejected(snapshot.result().clone()));
            }
            PromiseState::Pending => {
                if !runtime.execute_pending_job()? {
                    std::thread::yield_now();
                }
            }
        }
    }
}

fn completion_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Symbol(_) => "Symbol()".to_owned(),
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
    char::decode_utf16(value.utf16_units().take_while(|unit| *unit != 0))
        .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
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
                        &data_descriptor(Value::String(
                            JsString::try_from_utf8(prototype_value).unwrap(),
                        )),
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
                        &data_descriptor(Value::String(JsString::try_from_utf8(value).unwrap())),
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
