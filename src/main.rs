use std::io::Write as _;
use std::process::ExitCode;

use quickjs_oxide::lexer::quickjs_detect_module_bytes;
use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    Context, DebugInfoMode, DescriptorField, JsString, ModuleImportAttributes,
    ModuleImportMetaProperty, ModuleLoadResult, ModuleLoader, ModuleLoaderError,
    OrdinaryPropertyDescriptor, PromiseState, QUICKJS_COMPAT_VERSION, Runtime, RuntimeError, Value,
};

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
        let source = std::fs::read(&filename)
            .map_err(|_| ModuleLoaderError::new(format!("module filename '{filename}'")))?;
        if import_type_is(attributes, "json5") {
            return Ok(ModuleLoadResult::Json5Bytes(source));
        }
        if filename.ends_with(".json") || import_type_is(attributes, "json") {
            return Ok(ModuleLoadResult::JsonBytes(source));
        }
        let url = canonical_file_url(&filename).map_err(ModuleLoaderError::new)?;
        Ok(ModuleLoadResult::SourceBytesWithImportMeta {
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
            EvaluationSource::Utf8(&source),
            "<cmdline>",
            source_goal,
            main_module_path,
            &args[index..],
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
    match std::fs::read(file) {
        Ok(source) => {
            let source_goal = match source_goal {
                SourceGoal::Auto if is_module_file(file, &source) => SourceGoal::Module,
                SourceGoal::Auto => SourceGoal::Script,
                source_goal => source_goal,
            };
            evaluate(
                EvaluationSource::Bytes(&source),
                file,
                source_goal,
                Some(file),
                &args[index..],
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

#[derive(Clone, Copy)]
enum EvaluationSource<'a> {
    Utf8(&'a str),
    Bytes(&'a [u8]),
}

fn evaluate(
    source: EvaluationSource<'_>,
    filename: &str,
    source_goal: SourceGoal,
    main_module_path: Option<&str>,
    script_args: &[String],
    debug_info: DebugInfoMode,
    print_result: bool,
) -> ExitCode {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(debug_info);
    // Upstream qjs installs its filesystem loader for every process, including
    // Script-goal `-e`, so dynamic import has the same host boundary everywhere.
    let _module_loader = runtime.set_module_loader(FileModuleLoader);
    let mut context = runtime.new_context();
    let script_args = match script_args
        .iter()
        .map(|argument| JsString::try_from_utf8(argument).map_err(RuntimeError::from))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(script_args) => script_args,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    if let Err(error) = context.install_qjs_helpers_with_script_args(&script_args) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    let evaluation = match source_goal {
        SourceGoal::Script => match source {
            EvaluationSource::Utf8(source) => context.eval_with_filename(source, filename),
            EvaluationSource::Bytes(source) => context.eval_bytes_with_filename(source, filename),
        }
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
                        report_exception(format_pending_exception(&runtime, &mut context));
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
            report_exception(format_exception(&runtime, &exception));
            ExitCode::from(1)
        }
        Err(EvaluationError::Host(error)) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
        Err(EvaluationError::Runtime(RuntimeError::Exception)) => {
            report_exception(format_pending_exception(&runtime, &mut context));
            ExitCode::from(1)
        }
        Err(EvaluationError::Runtime(error)) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn is_module_file(filename: &str, source: &[u8]) -> bool {
    filename.ends_with(".mjs") || quickjs_detect_module_bytes(source)
}

fn evaluate_module(
    runtime: &Runtime,
    context: &mut Context,
    source: EvaluationSource<'_>,
    filename: &str,
    main_module_path: Option<&str>,
) -> Result<Value, EvaluationError> {
    let module = match source {
        EvaluationSource::Utf8(source) => context.compile_module_with_filename(source, filename),
        EvaluationSource::Bytes(source) => {
            context.compile_module_bytes_with_filename(source, filename)
        }
    }?;
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

fn format_pending_exception(runtime: &Runtime, context: &mut Context) -> Option<Vec<u8>> {
    let exception = context.take_exception().ok().flatten()?;
    format_exception(runtime, &exception)
}

fn format_exception(runtime: &Runtime, exception: &Value) -> Option<Vec<u8>> {
    runtime.qjs_print_value_bytes(exception).ok()
}

fn report_exception(exception: Option<Vec<u8>>) {
    let Some(exception) = exception else {
        eprintln!("JavaScript exception");
        return;
    };
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    let _ = stderr.write_all(&exception);
    let _ = stderr.write_all(b"\n");
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
            Some(b"Error".to_vec())
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
            Some(b"PrototypeName: PrototypeMessage\nprototype stack".to_vec())
        );
    }
}
