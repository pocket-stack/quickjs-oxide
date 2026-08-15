use super::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
enum RawModuleApi {
    Default,
    Filename,
    Options,
}

fn assert_script_true(context: &mut Context, source: &str) {
    assert_eq!(context.eval(source).unwrap(), Value::Bool(true));
}

fn error_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(error, &key).unwrap()
}

#[test]
fn raw_module_context_apis_preserve_authored_bytes_and_execute() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let cases: &[(RawModuleApi, &[u8])] = &[
        (
            RawModuleApi::Default,
            b"globalThis.__rawModuleUnits = '\xed\xa0\x80'; globalThis.__rawModuleTotal = 40;",
        ),
        (
            RawModuleApi::Filename,
            b"/*\x80*/ globalThis.__rawModuleTotal += 1;",
        ),
        (
            RawModuleApi::Options,
            b"/*\xff*/ globalThis.__rawModuleTotal += 1;",
        ),
    ];

    for &(api, source) in cases {
        let module = match api {
            RawModuleApi::Default => context.compile_module_bytes(source),
            RawModuleApi::Filename => {
                context.compile_module_bytes_with_filename(source, "raw-filename.mjs")
            }
            RawModuleApi::Options => context
                .compile_module_bytes_with_options(source, &CompileOptions::new("raw-options.mjs")),
        }
        .unwrap_or_else(|error| panic!("{api:?} raw module compilation failed: {error}"));
        context
            .execute_module(&module)
            .unwrap_or_else(|error| panic!("{api:?} raw module execution failed: {error}"));
    }

    assert_script_true(
        &mut context,
        "__rawModuleTotal === 42 && __rawModuleUnits.length === 1 && __rawModuleUnits.charCodeAt(0) === 0xd800",
    );
}

#[test]
fn raw_module_syntax_error_uses_byte_exact_filename_line_and_column() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_bytes_with_filename(b"/*\x80*/@", "raw-parse.mjs"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("raw module SyntaxError was not an object");
    };
    assert_eq!(
        error_property(&runtime, &mut context, &error, "name"),
        Value::String(JsString::from_static("SyntaxError"))
    );
    assert_eq!(
        error_property(&runtime, &mut context, &error, "fileName"),
        Value::String(JsString::from_static("raw-parse.mjs"))
    );
    assert_eq!(
        error_property(&runtime, &mut context, &error, "lineNumber"),
        Value::Int(1)
    );
    assert_eq!(
        error_property(&runtime, &mut context, &error, "columnNumber"),
        Value::Int(5)
    );
}

#[derive(Debug)]
struct RawBytesModuleLoader {
    modules: HashMap<String, ModuleLoadResult>,
}

impl ModuleLoader for RawBytesModuleLoader {
    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = String::from_utf16(
            &normalized_name.utf16_units().collect::<Vec<_>>(),
        )
        .map_err(|_| ModuleLoaderError::new("raw fixture module name is not valid UTF-16"))?;
        self.modules
            .get(&normalized_name)
            .cloned()
            .ok_or_else(|| ModuleLoaderError::new("raw fixture module is missing"))
    }
}

#[test]
fn loader_accepts_raw_static_and_dynamic_sources_with_import_meta() {
    let runtime = Runtime::new();
    let loader = RawBytesModuleLoader {
        modules: HashMap::from([
            (
                "pkg/static.js".to_owned(),
                ModuleLoadResult::SourceBytes(b"/*\x80*/ export const value = 40;".to_vec()),
            ),
            (
                "pkg/dynamic.js".to_owned(),
                ModuleLoadResult::SourceBytesWithImportMeta {
                    source: b"/*\xff*/ export const value = import.meta.answer;".to_vec(),
                    properties: vec![ModuleImportMetaProperty::new(
                        JsString::from_static("answer"),
                        Value::Int(2),
                    )],
                },
            ),
        ]),
    };
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let entry = context
        .compile_module_bytes_with_filename(
            b"import { value } from './static.js'; globalThis.__rawStatic = value; import('./dynamic.js').then(function (module) { globalThis.__rawDynamic = module.value; });",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&entry).unwrap();
    assert_script_true(&mut context, "__rawStatic === 40");
    let mut jobs = 0;
    while runtime.execute_pending_job().unwrap() {
        jobs += 1;
        assert!(jobs <= 128, "raw dynamic-import jobs did not quiesce");
    }
    assert!(jobs > 0, "raw dynamic import did not enqueue work");
    assert_script_true(&mut context, "__rawDynamic === 2");
}

#[test]
fn raw_loader_syntax_error_uses_dependency_name_and_byte_column() {
    let runtime = Runtime::new();
    let loader = RawBytesModuleLoader {
        modules: HashMap::from([(
            "pkg/bad.js".to_owned(),
            ModuleLoadResult::SourceBytes(b"/*\x80*/\nexport const bad = @;".to_vec()),
        )]),
    };
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './bad.js';", "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("raw dependency SyntaxError was not an object");
    };
    assert_eq!(
        error_property(&runtime, &mut context, &error, "fileName"),
        Value::String(JsString::from_static("pkg/bad.js"))
    );
    assert_eq!(
        error_property(&runtime, &mut context, &error, "lineNumber"),
        Value::Int(2)
    );
    assert_eq!(
        error_property(&runtime, &mut context, &error, "columnNumber"),
        Value::Int(20)
    );
}
