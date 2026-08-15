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

#[derive(Clone, Copy, Debug)]
enum RawJsonMode {
    Strict,
    Extended,
}

impl RawJsonMode {
    fn load_result(self, source: &[u8]) -> ModuleLoadResult {
        match self {
            Self::Strict => ModuleLoadResult::JsonBytes(source.to_vec()),
            Self::Extended => ModuleLoadResult::Json5Bytes(source.to_vec()),
        }
    }

    const fn import_type(self) -> &'static str {
        match self {
            Self::Strict => "json",
            Self::Extended => "json5",
        }
    }
}

#[test]
fn raw_json_static_loading_keeps_attributes_cache_and_import_meta_paths() {
    let runtime = Runtime::new();
    let (loader, _, loads) = JsonModuleLoader::new([
        (
            "pkg/value.json",
            ModuleLoadResult::JsonBytes(
                b"{\"delta\":2,\"wtf\":\"\xed\xa0\x80\",\"cesu\":\"\xed\xa0\xbd\xed\xb8\x80\",\"bom\":\"\xef\xbb\xbf\"}"
                    .to_vec(),
            ),
        ),
        (
            "pkg/meta.js",
            ModuleLoadResult::SourceBytesWithImportMeta {
                source: b"/*\xff*/ import payload from './value.json' with { type: 'json' }; export const answer = import.meta.base + payload.delta;"
                    .to_vec(),
                properties: vec![ModuleImportMetaProperty::new(
                    JsString::from_static("base"),
                    Value::Int(40),
                )],
            },
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let entry = context
        .compile_module_with_filename(
            r#"
            import first from "./value.json" with { type: "json" };
            import second from "../pkg/value.json" with { type: "json" };
            import { answer } from "./meta.js";
            globalThis.__rawJsonStatic =
                first === second && answer === 42 &&
                first.wtf.length === 1 && first.wtf.charCodeAt(0) === 0xd800 &&
                first.cesu.length === 2 && first.cesu.charCodeAt(0) === 0xd83d &&
                first.cesu.charCodeAt(1) === 0xde00 &&
                first.bom.length === 1 && first.bom.charCodeAt(0) === 0xfeff;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&entry).unwrap();
    assert_script_true(&mut context, "__rawJsonStatic === true");
    assert_eq!(
        loads.borrow().as_slice(),
        [
            RecordedAttributeLoad {
                name: "pkg/value.json".to_owned(),
                attributes: Some(vec![("type".to_owned(), "json".to_owned())]),
            },
            RecordedAttributeLoad {
                name: "pkg/meta.js".to_owned(),
                attributes: None,
            },
        ]
    );
}

#[test]
fn dynamic_raw_json5_loading_skips_malformed_comment_bytes() {
    let runtime = Runtime::new();
    let (loader, _, loads) = JsonModuleLoader::new([(
        "pkg/value.data",
        ModuleLoadResult::Json5Bytes(b"/*\x80*/ {answer: 0x2a, marker:'\xed\xa0\x80',}".to_vec()),
    )]);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let promise = eval_dynamic_import(
        &mut context,
        "import('./value.data', { with: { type: 'json5' } })",
        "pkg/entry.js",
    );

    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(drain_jobs(&runtime) > 0);
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    let Value::Object(namespace) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("dynamic raw JSON5 import did not fulfill with a module namespace");
    };
    let default = runtime.intern_property_key("default").unwrap();
    let Value::Object(value) = context.get_property(&namespace, &default).unwrap() else {
        panic!("dynamic raw JSON5 module did not expose its default object");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        context.get_property(&value, &answer).unwrap(),
        Value::Int(42)
    );
    let marker = runtime.intern_property_key("marker").unwrap();
    assert_eq!(
        context.get_property(&value, &marker).unwrap(),
        Value::String(JsString::try_from_utf16([0xd800]).unwrap())
    );
    assert_eq!(
        loads.borrow().as_slice(),
        [RecordedAttributeLoad {
            name: "pkg/value.data".to_owned(),
            attributes: Some(vec![("type".to_owned(), "json5".to_owned())]),
        }]
    );
}

#[test]
fn raw_json_errors_match_pinned_quickjs_byte_diagnostics() {
    struct Case {
        label: &'static str,
        mode: RawJsonMode,
        source: &'static [u8],
        message: &'static str,
        column: i32,
    }

    let cases = [
        Case {
            label: "malformed direct string byte",
            mode: RawJsonMode::Strict,
            source: b"{\"x\":\"\x80\"}",
            message: "Bad UTF-8 sequence",
            column: 7,
        },
        Case {
            label: "malformed escaped byte",
            mode: RawJsonMode::Strict,
            source: b"{\"x\":\"\\\x80\"}",
            message: "Bad escaped character",
            column: 8,
        },
        Case {
            label: "malformed token byte",
            mode: RawJsonMode::Strict,
            source: b"{\"x\":\x80}",
            message: "unexpected character",
            column: 6,
        },
        Case {
            label: "malformed byte after strict number sign",
            mode: RawJsonMode::Strict,
            source: b"-\x80",
            message: "Unexpected token '\u{fffd}",
            column: 2,
        },
        Case {
            label: "embedded NUL after strict number sign",
            mode: RawJsonMode::Strict,
            source: b"-\0",
            message: "Unexpected token '",
            column: 2,
        },
        Case {
            label: "malformed byte after extended number sign",
            mode: RawJsonMode::Extended,
            source: b"+\xff",
            message: "Unexpected token '\u{fffd}'",
            column: 2,
        },
        Case {
            label: "malformed byte after extended radix prefix",
            mode: RawJsonMode::Extended,
            source: b"0x\x80",
            message: "Unexpected token '\u{fffd}",
            column: 3,
        },
        Case {
            label: "embedded NUL after extended radix prefix",
            mode: RawJsonMode::Extended,
            source: b"0x\0",
            message: "Unexpected token '",
            column: 3,
        },
        Case {
            label: "NUL string byte",
            mode: RawJsonMode::Strict,
            source: b"{\"x\":\"\0\"}",
            message: "Bad control character in string literal",
            column: 7,
        },
        Case {
            label: "NUL token byte",
            mode: RawJsonMode::Strict,
            source: b"{\"x\":\0}",
            message: "unexpected token: ''",
            column: 6,
        },
        Case {
            label: "leading BOM",
            mode: RawJsonMode::Strict,
            source: b"\xef\xbb\xbf{\"x\":1}",
            message: "unexpected character",
            column: 1,
        },
        Case {
            label: "CESU-8 pair trailing token",
            mode: RawJsonMode::Strict,
            source: b"\"\xed\xa0\xbd\xed\xb8\x80\" @",
            message: "unexpected data at the end",
            column: 6,
        },
        Case {
            label: "canonical UTF-8 scalar trailing token",
            mode: RawJsonMode::Strict,
            source: b"\"\xf0\x9f\x98\x80\" @",
            message: "unexpected data at the end",
            column: 5,
        },
        Case {
            label: "malformed JSON5 comment then token",
            mode: RawJsonMode::Extended,
            source: b"/*\x80*/@",
            message: "unexpected token: '@'",
            column: 5,
        },
    ];

    for case in cases {
        let runtime = Runtime::new();
        let (loader, _, loads) =
            JsonModuleLoader::new([("pkg/value.data", case.mode.load_result(case.source))]);
        let _registration = runtime.set_module_loader(loader);
        let mut context = runtime.new_context();
        let entry = format!(
            "import value from './value.data' with {{ type: '{}' }}; globalThis.__rawJsonErrorBody = value;",
            case.mode.import_type(),
        );

        assert!(
            matches!(
                context.compile_module_with_filename(&entry, "pkg/entry.js"),
                Err(RuntimeError::Exception)
            ),
            "{} unexpectedly compiled",
            case.label,
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("{} did not throw a SyntaxError object", case.label);
        };
        for (name, expected) in [
            ("name", Value::String(JsString::from_static("SyntaxError"))),
            (
                "message",
                Value::String(JsString::try_from_utf8(case.message).unwrap()),
            ),
            (
                "fileName",
                Value::String(JsString::from_static("pkg/value.data")),
            ),
            ("lineNumber", Value::Int(1)),
            ("columnNumber", Value::Int(case.column)),
        ] {
            assert_eq!(
                error_property(&runtime, &mut context, &error, name),
                expected,
                "{} {name} differed",
                case.label,
            );
        }
        assert_eq!(
            loads.borrow().as_slice(),
            [RecordedAttributeLoad {
                name: "pkg/value.data".to_owned(),
                attributes: Some(vec![(
                    "type".to_owned(),
                    case.mode.import_type().to_owned(),
                )]),
            }],
            "{} load record differed",
            case.label,
        );
    }
}
